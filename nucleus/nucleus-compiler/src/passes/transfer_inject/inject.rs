use super::*;

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Inject [`ACFGNode::Xfer`] placeholders into `acfg` per the rules in
/// the module docs. The original name-table maps on the [`ACFG`] are
/// forwarded unchanged.
///
/// Pure: same `(linked, acfg)` produces the same output across runs
/// (BTreeMap-based iteration order and a deterministic `SeqTag`
/// counter make this so).
///
/// Idempotent: re-running on the output yields the same tree
/// structurally. See `tests/transfer_inject.rs`.
pub fn inject_transfers(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, TransferInjectError> {
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        // Forward through pre-existing pipeline depths. In the
        // standard driver pipeline this is always empty here (we are
        // the populator), but tests may inject their own values
        // before calling us — preserve them.
        pipeline_depth_for_seq: pre_existing_pipeline_depth,
        // TASK-0263 Stage 2: transfer_inject consumes halo_widths to
        // extend per-tile transfer ranges by the inferred stencil
        // halo widths. The sidecar is read below at the
        // `extend_xfer_tiles_for_halo` step and forwarded verbatim
        // afterwards — it remains the single source of truth (the
        // pass does not rewrite or invalidate halo entries; it only
        // applies them to Xfer tiles).
        halo_widths,
        // TASK-0261: transfer_inject currently DOES NOT read the
        // reuse-widths sidecar — Stage 2 (TASK-0265, backend walker
        // delay-line emit) is the wiring cycle. Forward verbatim.
        reuse_widths,
        // TASK-0264 cycle 113: transfer_inject consumes partition_pairs +
        // grid_shape_for_outer_iv (populated by partition_blocks2d) as
        // of TASK-0289 cycle 114a — see `inject_halo_strip_xfers` at the
        // tail of the finalisation chain below. Forward verbatim into
        // the returned ACFG: the consumer is read-only and never
        // mutates either map.
        partition_pairs,
        grid_shape_for_outer_iv,
    } = acfg;

    // Resolve the link pass's `WorkerEntity` (BTreeSet<String>) to a
    // `BTreeSet<WorkerId>` once, keyed by data symbol name. This is
    // the producer-side worker entity per data symbol.
    let producers_by_data: BTreeMap<DataId, BTreeSet<WorkerId>> = linked
        .data_producers
        .iter()
        .filter_map(|(data_name, entity)| {
            let data_id = name_data.get(data_name).copied()?;
            let workers = entity_to_workerid_set(entity, &name_workers);
            Some((data_id, workers))
        })
        .collect();

    // Resolve the schedule's transfer directives, keyed by DataId.
    // The schedule's `transfers` is keyed by data NAME; we translate
    // once.
    let policies_by_data: BTreeMap<DataId, TransferPolicy> = linked
        .sched
        .transfers
        .iter()
        .filter_map(|(data_name, dir)| {
            let data_id = name_data.get(data_name).copied()?;
            Some((data_id, policy_from_directive(dir)))
        })
        .collect();

    // Resolve `loop VAR : pipeline=D` directives into a per-IterVar
    // map (TASK-0134). The schedule's `loops` is keyed by var NAME;
    // we translate once via `name_iter_vars`. Loops not in
    // `name_iter_vars` (e.g. a `loop x : pipeline=N` referencing a
    // var the algorithm doesn't have) are already a link-step error
    // (LinkErrorKind::UnknownLoop), so we silently skip them here —
    // build_acfg will not have reached us if link rejected.
    let pipeline_depth_for_iter_var: BTreeMap<IterVar, NonZeroU64> = linked
        .sched
        .loops
        .iter()
        .filter_map(|(var_name, dir)| {
            let iv = name_iter_vars.get(var_name).copied()?;
            // PRD §6.3.3: only `pipeline=D` lowers to an initial
            // marking; `block=`, `vectorize=`, `unroll=`, `reuse`,
            // `partition=` all have other effects handled by other
            // passes.
            let depth = dir.options.iter().find_map(|opt| match opt {
                ResolvedLoopOption::Pipeline(d) => NonZeroU64::new(*d),
                _ => None,
            })?;
            Some((iv, depth))
        })
        .collect();

    // TASK-0324 cycle-144 AC#2 + TASK-0325 cycle-145 generalisation +
    // TASK-0324 cycle-147 AC#3 lift: front-loaded validator. Walks every
    // Operation BEFORE the emission recursion. Pre-cycle-147 the
    // validator rejected the same-worker-pair-elision +
    // consumer-reads-outside-local-slice shape with a typed
    // `TransferInjectError::SameSetSilentElisionRisk`. Cycle 147 AC#3
    // adds a carve-out: when the producer and consumer worker sets are
    // IDENTICAL (the same-set case the 06/distributed2 reproducer
    // exercises), the validator allows the unsafe shape through —
    // `build_waits_for_op`'s same-set short-circuit now classifies the
    // elision with the SAME predicate and falls through to the
    // cartesian-product fan-out, emitting cross-worker pairs. The
    // partial-overlap case (TASK-0325 — different sets sharing some
    // workers) still rejects pending AC#3's extension to per-element
    // fan-out elision.
    //
    // Companion comments live at both same-worker short-circuits
    // inside `build_waits_for_op` (the `producer_workers ==
    // &consumer_workers` whole-set short-circuit AND the `if src == dst`
    // per-element skip in the cartesian-product fan-out — grep
    // `if producer_workers == &consumer_workers` and `if src == dst`
    // for the witness anchors).
    let partition_iter_vars: BTreeSet<IterVar> = partition_worker_ranges.keys().copied().collect();
    let mut producer_writes: BTreeMap<DataId, DataAccess> = BTreeMap::new();
    collect_producer_writes(&root, &mut producer_writes);
    // TASK-0389: precompute the producer-statement rank of every data
    // symbol once over the full ACFG (this is invariant under the walk),
    // so `build_waits_for_op` can sort each worker's per-channel Wait
    // sequence into the host's per-channel Push order.
    let producer_rank = producer_rank_by_data(&root);
    check_no_silent_elision_risk(
        &root,
        &producers_by_data,
        &partition_iter_vars,
        &name_iter_vars,
        &name_data,
        &producer_writes,
    )?;

    let ctx = InjectCtx {
        producers_by_data: &producers_by_data,
        policies_by_data: &policies_by_data,
        inner_block_iter_vars: &inner_block_iter_vars,
        partition_iter_vars: &partition_iter_vars,
        name_iter_vars: &name_iter_vars,
        producer_writes: &producer_writes,
        producer_rank: &producer_rank,
    };

    // Counter state for SeqTag generation. A single monotonic counter
    // across the whole ACFG is simplest and meets the "unique per
    // pair" requirement. PRD §8.3 talks about per-(src,dst,data)
    // monotonic numbering — that ordering is also a strict subset of
    // a single global counter (per-triple subsequences are still
    // strictly increasing). The global counter has the property that
    // every Push/Wait pair in the whole program has a distinct seq,
    // which is the strongest reading of "unique".
    let mut state = State {
        next_seq: 0,
        last_writer: BTreeMap::new(),
    };

    let new_root = inject_in_node(root, &ctx, &mut state);

    // Cross-scope finalisation (TASK-0136 / TASK-0139). The single-
    // pass `inject_in_sequence` above only rendezvouses a Push with a
    // Wait when both live in the *same* sequence. When the consumer is
    // inside a plain `for` loop and the producer is in the enclosing
    // sequence (example 02-split: `load_input` on host at top level,
    // `add` on w0 inside `for i`), the Wait is trapped inside the
    // Repeat body and never gets a Push — the net deadlocks.
    //
    // The correct model is *whole-symbol* transfer: a datum that is
    // loop-invariant w.r.t. a Repeat crosses the worker boundary once,
    // not once per iteration. Pass A hoists such Waits out of the loop
    // body; Pass B places the matching Push (after the producer, or
    // after the producer's enclosing loop when the consumer sits
    // outside it — the dual case, e.g. `c` produced inside `for i` and
    // read by `save_output` after the loop).
    //
    // Scope: PER-WAIT (TASK-0267). Pass A's stay-vs-bubble decision is
    // already keyed on whether the Wait's `data` is produced inside
    // the enclosing Repeat's subtree, so a block-governed nest does
    // not need a separate opacity gate: a host-loaded image (producer
    // outside) bubbles up and gets a whole-symbol Push at the
    // producer's scope, while an in-loop produced datum (producer
    // inside the loop body) stays for per-iteration rendezvous. The
    // per-worker partition slice is added later by
    // `rewrite_partition_tiles` (TASK-0117/TASK-0212) and halo widths
    // by `extend_xfer_tiles_for_halo` (TASK-0263), so the Wait carries
    // the right slice regardless of which Repeat it crossed. A future
    // per-block-tile slice (the original TASK-0149/TASK-0158
    // territory) would need a per-tile counterpart to halo extension;
    // no shipped schedule exercises that shape today.
    let new_root = {
        let (hoisted, escaped_at_root) = hoist_invariant_waits(new_root, &[]);
        // A Wait that bubbled all the way out of the tree had no
        // producing scope anywhere (its data is produced by no
        // Operation in the whole ACFG). For a well-formed ACFG this
        // never happens — a cross-worker Wait is only emitted because
        // the schedule records a producer, which `build_acfg` lowers
        // to an Operation. Dropping it silently here would hand a
        // downstream pass an unpaired Wait to mis-diagnose. Fail loud
        // with context instead (TASK-0152).
        if let Some(w) = escaped_at_root.first() {
            let data_name = name_data
                .iter()
                .find_map(|(n, id)| (*id == w.data).then_some(n.as_str()))
                .unwrap_or("<unknown>");
            panic!(
                "transfer_inject invariant violation: cross-worker Wait for \
                 data `{data_name}` (id {:?}, seq {:?}, {:?} -> {:?}) escaped \
                 the whole ACFG with no producing Operation. A Wait is only \
                 emitted when the schedule records a producer for the symbol, \
                 so this is a malformed ACFG (producer kernel not lowered), \
                 not a partial test input.",
                w.data, w.seq, w.src, w.dst
            );
        }
        let spliced = splice_pushes_global(hoisted, &name_data);
        // TASK-0117 per-worker tile finalisation. The hoist + splice
        // passes above may rewrite a Wait/Push tile to the
        // post-hoist enclosing tile (e.g. a loop-invariant `input`
        // Wait at top level lands with `[]`). For a fan-out pair the
        // tile must reflect the COMPUTE worker's partition slice so
        // the backend host-side gather can slice-paste. We do the
        // rewrite as a final ACFG walk keyed by the pair's
        // src/dst against `partition_worker_ranges`; pairs whose
        // neither endpoint is partitioned (the 1:1 host↔single-worker
        // shape from examples 01..07) survive unchanged because the
        // map has no entry for either side.
        // TASK-0301 / TASK-0302: Build the per-data, per-dim iter-var
        // index map by walking every Operation's DataflowDag. Consulted
        // by `rewrite_partition_tiles_inner` to enforce the
        // *contiguous-prefix* invariant on per-Xfer tile bounds: bounds
        // must be in dim order AND cover only a contiguous prefix of
        // the data's dims, lest `wait_slice` (whose convention is
        // `tile.bounds[i].iter_var ↔ data.dim[i]`) silently mis-map a
        // sparse covering. The 1D AXIS-MAPPING discharge (TASK-0301)
        // covered the "iv not in the data's union" case via the
        // per-symbol filter; TASK-0302 generalises the input to a
        // per-dim map so the 2D `b[k][j]` × `partition=blocks2d(i,j)`
        // shape (where j IS in b's union but only at dim 1, with no
        // partitioned iv covering dim 0) also drops safely to a
        // whole-array broadcast rather than slicing the wrong dim.
        let data_dim_iv_map = collect_data_dim_iv_map(&spliced, &name_iter_vars);
        let partitioned =
            rewrite_partition_tiles(spliced, &partition_worker_ranges, &data_dim_iv_map);
        // TASK-0263 Stage 2 halo extension. For each XferPlaceholder
        // whose tile axis carries a non-zero halo entry (the data
        // symbol's consumer kernel's halo widths along that
        // iter-var), extend the axis range by halo on both sides.
        // Clamp against the iter-var's algorithm source range
        // expanded by halo — i.e. the data extent the (unpartitioned)
        // loop would have read. No-op when halo_widths is empty
        // (every pre-Stage-2 example).
        let with_halo = extend_xfer_tiles_for_halo(partitioned, &halo_widths);
        // TASK-0289 cycle 114a halo-strip Push/Wait synthesis. For each
        // `(outer_iv, inner_iv)` pair recorded in `partition_pairs`
        // (populated by partition_blocks2d), synthesise cross-worker
        // halo-strip Push/Wait pairs between N/S/E/W neighbours in the
        // 2D worker grid. Corner pairs (NE/NW/SE/SW) are excluded in
        // this first cut per the TASK-0289 brief. Runs AFTER
        // `rewrite_partition_tiles` and `extend_xfer_tiles_for_halo`
        // so the carefully-crafted halo-strip tiles we emit are not
        // clobbered by either pass — both rewrite tiles in-place by
        // walking every `Xfer`. Short-circuits on empty
        // `partition_pairs` (AC#3 additive-only contract: every shipped
        // schedule today has empty pairs and therefore sees no change
        // in injected XferPlaceholders).
        let with_strips = inject_halo_strip_xfers(
            with_halo,
            &halo_widths,
            &partition_pairs,
            &grid_shape_for_outer_iv,
            &partition_worker_ranges,
            &policies_by_data,
            &data_dim_iv_map,
            &mut state,
        );
        // TASK-0341.02.02.01.{02,03} cycle 213 — cumulative-array
        // partition-band exchange. For a CUMULATIVE cross-iteration
        // array (16-jacobi's `field[t] <-- f(field[t-1], ...)` under
        // `partition=rows(y)`), the w2w + host-gather transfers must:
        //   (1) carry the SENDER's WRITE BAND tile [(t, full), (y,
        //       src-band), (x, full)] — NOT halo-expanded (overlapping
        //       bands would double-write), NOT whole-array (whole-array
        //       accumulate xN-double-counts the shared history); and
        //   (2) the w2w exchange must be HOISTED out of the partition
        //       spatial loops to the enclosing Repeat (for t) body,
        //       send-then-recv, so each worker performs an EQUAL number
        //       of exchanges (the in-`for x` per-(y,x) placement gives
        //       unequal band-size-dependent counts => deadlock).
        // No-op when the algorithm has no cumulative array. Today the
        // cumulative SET is {16-jacobi `field`, 11-game-of-life `grid`}
        // (both have a cross-iteration self-read).
        //
        // CORRECTION (TASK-0366 cycle-214): the original cycle-213 comment
        // here claimed game-of-life "short-circuits to a structural no-op"
        // because `partition_worker_ranges` is empty. That was a comment
        // lie. 11-game-of-life/pipelined DOES emit a cross-worker `grid`
        // transfer (async double-buffer, compute -> host), so
        // `rewrite_cumulative_band_tiles` below DOES walk into `grid`'s
        // Xfer arm and `cumulative_band_bounds` DOES return `None` (no
        // partitioned iv covers any `grid` dim — there is no `partition=`
        // at all). The pass does NOT short-circuit; it falls through and
        // (correctly) keeps the whole-array tile, because with a single
        // compute worker owning all of `grid` there is nothing to xN-
        // double-count. The fail-loud error introduced by TASK-0366
        // therefore guards the `partition_ranges`-NON-EMPTY shape ONLY
        // (a replicated-across-partition-workers cumulative array) — the
        // empty-`partition_ranges` game-of-life case stays whole-array,
        // silent, correct. See `rewrite_cumulative_band_tiles` for the
        // A/B discriminator. (TASK-0341.02.02.01.{01,02,03} co-landed the
        // original cycle-213 machinery.)
        let mut cumulative_names: BTreeSet<String> = BTreeSet::new();
        crate::sidecar::collect_cumulative_data_names(
            &linked.algo.stmts,
            &[],
            &mut cumulative_names,
        );
        let cumulative_data: BTreeSet<DataId> = cumulative_names
            .iter()
            .filter_map(|n| name_data.get(n).copied())
            .collect();
        if cumulative_data.is_empty() {
            with_strips
        } else {
            // Per-DataId dim sizes (i64), resolved via name_data ->
            // linked.algo.data[name].ty.dims. Needed to fill FULL
            // ranges (0..dim) for the non-partitioned axes of the
            // write-band tile.
            let data_dims: BTreeMap<DataId, Vec<i64>> = name_data
                .iter()
                .filter_map(|(n, id)| {
                    linked
                        .algo
                        .data
                        .get(n)
                        .map(|rd| (*id, rd.ty.dims.iter().map(|d| *d as i64).collect()))
                })
                .collect();
            let banded = rewrite_cumulative_band_tiles(
                with_strips,
                &cumulative_data,
                &partition_worker_ranges,
                &data_dim_iv_map,
                &data_dims,
            )?;
            hoist_cumulative_w2w_to_repeat_body(banded, &cumulative_data, &partition_worker_ranges)
        }
    };

    // TASK-0134 — populate the pipeline-depth sidecar AFTER all the
    // hoist/splice/rewrite passes have run. Doing it now (rather than
    // at `fresh_seq` time) is load-bearing: `hoist_invariant_waits`
    // moves Waits OUT of the loop body for loop-invariant whole-symbol
    // transfers, AND rewrites their `tile` to the post-hoist
    // enclosing tile. The seq's pipeline depth must reflect WHERE the
    // Push/Wait pair LANDED, not where it was born — otherwise we'd
    // pre-seed a buffer place with `D` tokens when only one
    // Push/Wait fires for the whole loop and the buffer place would
    // overflow at construction.
    //
    // Walk each `Xfer`'s `tile` (post-hoist enclosing iteration tile)
    // against `pipeline_depth_for_iter_var`; the innermost matching
    // entry wins. If neither endpoint of the pair sits inside a
    // pipelined loop, the seq has no entry (-> `initial_marking = 0`,
    // the default). Pairs whose Push and Wait have differing tiles
    // are kept in sync because: (1) the Wait's tile is the
    // authoritative enclosing context (it is the consumer side; the
    // pair fires per-consumer-tile), and (2) for whole-symbol
    // hoisting both sides are moved together so tiles agree post-
    // splice. We aggregate per seq using "any endpoint inside a
    // pipelined loop" as the trigger, but in practice the two
    // endpoints share the same enclosing tile by construction.
    let mut pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64> = pre_existing_pipeline_depth;
    if !pipeline_depth_for_iter_var.is_empty() {
        annotate_pipeline_depth_for_seq(
            &new_root,
            &pipeline_depth_for_iter_var,
            &mut pipeline_depth_for_seq,
        );
    }

    Ok(ACFG {
        root: new_root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths,
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
    })
}

/// Walk the final ACFG (post-hoist, post-splice, post-rewrite-tiles)
/// and populate `out` with (SeqTag, depth) for every `Xfer`
/// placeholder whose enclosing iteration tile contains an iter-var
/// with a `pipeline=D` directive. **Innermost wins.**
///
/// The "innermost wins" semantic rests on [`IterTile::bounds`]'s
/// documented convention (event.rs ~line 227): "Outer-most iteration
/// variable first." Walking `bounds.iter().rev()` therefore visits
/// innermost-to-outermost, and `find_map` stops at the first
/// (= innermost) pipelined iter-var. All construction sites build
/// `bounds` in nest order — the enclosing-tile stack in
/// `inject_in_sequence`, the fan-out path, and (since TASK-0224) the
/// partition-rewrite path, which walks the enclosing `Repeat` stack
/// directly rather than relying on `IterVar` id ordering.
///
/// Why we use the Xfer's `tile` (not the live walk's enclosing-tile
/// stack): after `hoist_invariant_waits` runs, a Wait may live at a
/// shallower nesting than the Repeat it was born in, but its `tile`
/// is rewritten to that shallower nesting (see the
/// `w.tile = IterTile::new(...)` rewrite in `inject_in_sequence`, now
/// in the `sequence` submodule). The tile is therefore the authoritative
/// "in which loop does this transfer fire" record.
///
/// Determinism: the walk is depth-first source-order; `BTreeMap`
/// insertion preserves stable iteration. If Push and Wait of the
/// same seq disagree on depth (shouldn't happen in well-formed
/// inputs — `splice_pushes_global` places the Push at the same
/// tile-scope as the Wait), **last-visited wins**: source-order
/// traversal visits Push before Wait inside a Sequence, so in
/// practice the Wait's annotation is what lands. In well-formed
/// inputs they agree, so the distinction is academic; documented
/// here so a future invariant change doesn't surprise the reader.
pub(super) fn annotate_pipeline_depth_for_seq(
    node: &ACFGNode,
    pipeline_for_iv: &BTreeMap<IterVar, NonZeroU64>,
    out: &mut BTreeMap<SeqTag, NonZeroU64>,
) {
    match node {
        ACFGNode::Xfer(x) => {
            let depth = x
                .tile
                .bounds
                .iter()
                .rev()
                .find_map(|(iv, _)| pipeline_for_iv.get(iv).copied());
            if let Some(d) = depth {
                out.insert(x.seq, d);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Sequence(children) => {
            for c in children {
                annotate_pipeline_depth_for_seq(c, pipeline_for_iv, out);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            annotate_pipeline_depth_for_seq(body, pipeline_for_iv, out);
        }
    }
}

// --------------------------------------------------------------------
// Recursive walker
// --------------------------------------------------------------------

/// Inject into one ACFGNode. Returns the (possibly rewritten) node.
pub(super) fn inject_in_node(node: ACFGNode, ctx: &InjectCtx<'_>, state: &mut State) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            ACFGNode::Sequence(inject_in_sequence(children, ctx, state, &[], None))
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
            break_cond,
        } => {
            // Build the enclosing-tile contribution from this Repeat.
            // We re-walk the body with the contribution pushed onto a
            // local stack; the helper `inject_in_node_with_tile` does
            // the bookkeeping.
            let outer_tile = vec![(iter_var, range.clone())];
            let new_body = inject_in_node_with_tile(*body, ctx, state, &outer_tile, None);
            ACFGNode::Repeat {
                iter_var,
                range,
                body: Box::new(new_body),
                // transfer_inject only injects Push/Wait/Xfer into the
                // body; the strip-mine rebinding tag is structural and
                // survives verbatim (TASK-0180).
                block_tag,
                // ... likewise the `for..until` halt predicate is carried
                // through unchanged.
                break_cond,
            }
        }
        // Leaves: nothing to inject inside.
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => {
            // Record any side-effect on `last_writer` for non-nested
            // top-level ops in the Sequence walker; here we only see
            // leaves *outside* a Sequence (shouldn't really happen
            // from `build_acfg`, but defensive). Update the writer map
            // if it's an Operation.
            if let ACFGNode::Operation(op) = &leaf {
                update_writer(state, op);
            }
            leaf
        }
    }
}

/// Same as [`inject_in_node`] but with an enclosing-tile context
/// passed in. Used inside `Repeat` bodies so that any
/// `XferPlaceholder` created carries the iteration tile of the
/// enclosing loop(s).
///
/// The optional `hoist_sink` is set when this node sits *inside* a
/// `block`-inner intra-tile loop; instead of emitting Waits in-line
/// at the consumer Operation and Pushes after the producer
/// Operation, the walker forwards them to the sink so the
/// enclosing per-tile body sequence can place them around the inner
/// `Repeat`. TASK-0143.
pub(super) fn inject_in_node_with_tile(
    node: ACFGNode,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
    hoist_sink: Option<&mut HoistSink>,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => ACFGNode::Sequence(inject_in_sequence(
            children,
            ctx,
            state,
            enclosing_tile,
            hoist_sink,
        )),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
            break_cond,
        } => {
            let mut nested = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            // Forward whatever sink we received into the body. The
            // decision of WHERE a sink originates and WHERE it
            // drains lives entirely in `inject_in_sequence` (which
            // creates one when it encounters a `block`-inner Repeat
            // child and parent_sink is None). Repeats themselves
            // are transparent: they neither create sinks nor drop
            // them, regardless of whether the current Repeat is
            // block-inner or not. This is what lets a 2D-blocked
            // schedule hoist through the (outer-j, inner-i) layers
            // all the way up to the outer-i tile body. TASK-0143.
            let new_body = inject_in_node_with_tile(*body, ctx, state, &nested, hoist_sink);
            ACFGNode::Repeat {
                iter_var,
                range,
                body: Box::new(new_body),
                // Transparent reconstruction — preserve the strip-mine
                // rebinding tag verbatim (TASK-0180).
                block_tag,
                // ... and the `for..until` halt predicate, unchanged.
                break_cond,
            }
        }
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => {
            if let ACFGNode::Operation(op) = &leaf {
                update_writer(state, op);
            }
            leaf
        }
    }
}
