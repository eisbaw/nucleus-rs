use super::*;

// --------------------------------------------------------------------
// Pass B — global Push finaliser (cross-scope)
// --------------------------------------------------------------------
//
// After Pass A every Wait is positioned. Each Wait still needs a
// matching Push after its producer. `splice_pushes_for_waits` already
// did this for in-sequence pairs during the single-pass walk; this
// pass finalises the cross-scope ones it could not see, idempotently
// (keyed on the unique `seq`, with a structural (src,dst,data) guard).
//
// Whole-symbol placement rule, given producer P of D and the Wait W:
//
//   * If P sits inside one or more Repeats that W is NOT inside, the
//     Push goes immediately after the *outermost* such Repeat (D is a
//     loop output, available once the loop completes — the dual of
//     Pass A's loop-input hoist). Example 02: `c` produced by `add`
//     inside `for i`, read by `save_output` after the loop.
//
//   * Otherwise the Push goes immediately after P itself (same scope,
//     or P and W co-resident in the same loop = per-iteration
//     rendezvous).

/// Producer-statement RANK of every data symbol: the depth-first walk
/// position of the FIRST Operation that writes it (TASK-0389).
///
/// This is the SAME ordering `splice_pushes_global` realises for the
/// host's Push events: it pins each Push at its producer Operation's
/// site, so the host's per-channel send order over the projected
/// EventList is exactly this producer-statement order. A lower rank ⇒
/// the data is produced (and therefore pushed) earlier.
///
/// `build_waits_for_op` consumes this to sort the worker's per-channel
/// Wait sequence into the same order — so on a strict-FIFO host->worker
/// channel (`wire::read_msg_expect`) the worker reads in the order the
/// host sent, for ANY declaration order. Before TASK-0389 the worker
/// Wait order followed `data_in` TRAVERSAL order, which only coincided
/// with producer order when the gather index array was declared before
/// its outer array (see `acfg::build::collect_dataref_access_expr`).
///
/// The walk order MUST match `producer_repeat_path`'s (depth-first,
/// Sequence children in order, recursing into Repeat bodies) so the rank
/// agrees with where the Push is actually spliced. Single-assignment
/// (PRD §6.2.1) makes "first writer" == "only writer"; a `BTreeMap`
/// `entry().or_insert` keeps the first occurrence.
pub(super) fn producer_rank_by_data(root: &ACFGNode) -> BTreeMap<DataId, usize> {
    fn walk(node: &ACFGNode, next: &mut usize, out: &mut BTreeMap<DataId, usize>) {
        match node {
            ACFGNode::Operation(op) => {
                if let Some(d) = output_data(op) {
                    out.entry(d).or_insert_with(|| {
                        let r = *next;
                        *next += 1;
                        r
                    });
                }
            }
            ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
            ACFGNode::Repeat { body, .. } => walk(body, next, out),
            ACFGNode::Sequence(children) => {
                for c in children {
                    walk(c, next, out);
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    let mut next = 0usize;
    walk(root, &mut next, &mut out);
    out
}

/// Outer→inner list of Repeat iter-vars enclosing the (unique,
/// single-assignment) producer Operation of `data`.
pub(super) fn producer_repeat_path(
    node: &ACFGNode,
    data: DataId,
    path: &mut Vec<IterVar>,
    found: &mut Option<Vec<IterVar>>,
) {
    if found.is_some() {
        return;
    }
    match node {
        ACFGNode::Operation(op) => {
            if output_data(op) == Some(data) {
                *found = Some(path.clone());
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
        ACFGNode::Repeat { iter_var, body, .. } => {
            path.push(*iter_var);
            producer_repeat_path(body, data, path, found);
            path.pop();
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                producer_repeat_path(c, data, path, found);
            }
        }
    }
}

/// Outer→inner list of Repeat iter-vars enclosing the Wait carrying
/// `seq`.
pub(super) fn wait_repeat_path(
    node: &ACFGNode,
    seq: SeqTag,
    path: &mut Vec<IterVar>,
    found: &mut Option<Vec<IterVar>>,
) {
    if found.is_some() {
        return;
    }
    match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Xfer(x) => {
            if x.role == XferRole::Wait && x.seq == seq {
                *found = Some(path.clone());
            }
        }
        ACFGNode::Repeat { iter_var, body, .. } => {
            path.push(*iter_var);
            wait_repeat_path(body, seq, path, found);
            path.pop();
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                wait_repeat_path(c, seq, path, found);
            }
        }
    }
}

pub(super) fn subtree_produces(node: &ACFGNode, data: DataId) -> bool {
    let mut s = BTreeSet::new();
    produced_data_set(node, &mut s);
    s.contains(&data)
}

/// Insert `push` immediately after the (unique) producer Operation of
/// `push.data` wherever it directly resides — landing it AFTER any Push
/// Xfers already spliced at that same insertion point.
///
/// TASK-0389.01: the "after existing Pushes" detail mirrors
/// `splice_after_repeat`. Single-assignment (PRD §6.2.1) means at most
/// one data is produced per Operation, so today two distinct-data
/// Pushes never share a producer-Op insertion point and the append is a
/// no-op here. It is kept so the non-cut path obeys the SAME
/// rank-ordered-feed → rank-ordered-textual-Push invariant as the cut
/// path, defending a future multi-output Operation shape from silently
/// reintroducing the FIFO reversal.
pub(super) fn splice_after_producer(node: ACFGNode, push: &XferPlaceholder) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            let mut out = Vec::with_capacity(children.len() + 1);
            let mut it = children.into_iter().peekable();
            while let Some(c) = it.next() {
                let is_producer = matches!(&c, ACFGNode::Operation(op)
                    if output_data(op) == Some(push.data));
                let c = splice_after_producer(c, push);
                out.push(c);
                if is_producer {
                    while matches!(it.peek(), Some(ACFGNode::Xfer(x)) if x.role == XferRole::Push) {
                        out.push(it.next().expect("peek confirmed Some"));
                    }
                    out.push(ACFGNode::Xfer(push.clone()));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_producer(*body, push)),
            block_tag,
        },
        leaf => leaf,
    }
}

/// Insert `push` immediately after the Repeat whose iter-var is
/// `cut_iv` and which (transitively) produces `push.data` — landing it
/// AFTER any Push Xfers already spliced at that same insertion point.
///
/// TASK-0389.01: the "after existing Pushes" detail is load-bearing for
/// FIFO correctness when ≥2 loop-OUTPUT data on the SAME (src→dst)
/// channel co-hoist past the SAME enclosing Repeat. `splice_pushes_global`
/// now feeds these Pushes in producer-rank order; appending each new
/// Push at the END of the contiguous run of already-present Pushes that
/// immediately follow the cut Repeat makes the host's textual (= wire
/// send) Push order EQUAL producer-rank order. The naive "immediately
/// after the Repeat" insert reversed co-hoisted Pushes (the most-recent
/// splice landed closest to the Repeat), so the host sent in reverse-rank
/// order while the worker `build_waits_for_op` waits in rank order — a
/// strict-FIFO `read_msg_expect` seq-mismatch panic. Verified by the
/// `task038901_*` unit tests + the e2e `18-multigather/distributed` cell.
pub(super) fn splice_after_repeat(
    node: ACFGNode,
    cut_iv: IterVar,
    push: &XferPlaceholder,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            let mut out: Vec<ACFGNode> = Vec::with_capacity(children.len() + 1);
            let mut it = children.into_iter().peekable();
            while let Some(c) = it.next() {
                let is_cut = matches!(&c, ACFGNode::Repeat { iter_var, .. }
                    if *iter_var == cut_iv && subtree_produces(&c, push.data));
                if is_cut {
                    // Emit the cut Repeat, then any Pushes already
                    // spliced right after it (in their existing order),
                    // then the new Push LAST — so a rank-ordered feed
                    // yields rank-ordered textual Push order.
                    out.push(c);
                    while matches!(it.peek(), Some(ACFGNode::Xfer(x)) if x.role == XferRole::Push) {
                        out.push(it.next().expect("peek confirmed Some"));
                    }
                    out.push(ACFGNode::Xfer(push.clone()));
                } else {
                    out.push(splice_after_repeat(c, cut_iv, push));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_repeat(*body, cut_iv, push)),
            block_tag,
        },
        leaf => leaf,
    }
}

pub(super) fn collect_push_seqs(node: &ACFGNode, seqs: &mut BTreeSet<u64>) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Push => {
            seqs.insert(x.seq.0);
        }
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => collect_push_seqs(body, seqs),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_push_seqs(c, seqs);
            }
        }
    }
}

/// Collect Waits eligible for whole-symbol Push finalisation.
/// TASK-0267: per-Wait classification — every Wait in the tree is
/// eligible; the per-Wait stay-vs-bubble decision in Pass A
/// (`hoist_invariant_waits`) has already placed each Wait at the
/// scope where its `data` becomes producer-relative, so Pass B's
/// producer-path / wait-path cut handles all shapes uniformly.
pub(super) fn collect_waits(node: &ACFGNode, out: &mut Vec<XferPlaceholder>) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Wait => out.push(x.clone()),
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => {
            collect_waits(body, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_waits(c, out);
            }
        }
    }
}

pub(super) fn splice_pushes_global(
    mut root: ACFGNode,
    name_data: &BTreeMap<String, DataId>,
) -> ACFGNode {
    let mut have_seqs: BTreeSet<u64> = BTreeSet::new();
    collect_push_seqs(&root, &mut have_seqs);

    let mut waits: Vec<XferPlaceholder> = Vec::new();
    collect_waits(&root, &mut waits);

    // TASK-0389.01: splice each (src→dst) channel's Pushes in
    // producer-rank order. `collect_waits` returns Waits in DFS order,
    // which is NOT producer-rank order in general — and the splice
    // helpers (`splice_after_repeat` / `splice_after_producer`) now
    // APPEND each new Push after any already-spliced Pushes at the same
    // insertion point. So feeding the Pushes in producer-rank order
    // makes the host's textual (= wire-send) Push order on each channel
    // EQUAL producer-rank order, which is exactly the order
    // `build_waits_for_op` sorts each worker's per-channel Wait
    // sequence into. The two endpoints therefore traverse every
    // strict-FIFO channel in the SAME order → `read_msg_expect` pairs
    // by construction for ANY loop-output nesting (the residual the
    // raw producer-rank Wait sort could not see: ≥2 loop-OUTPUT data on
    // one channel co-hoisting past the same Repeat, where the naive
    // "insert immediately after the Repeat" REVERSED them).
    //
    // The key mirrors `build_waits_for_op`'s `(dst, src, rank, data,
    // seq)` but leads with `rank`: cross-channel relative order is
    // FIFO-irrelevant (independent sockets), and a rank-leading total
    // order keeps same-channel Pushes monotone in rank regardless of
    // how DFS interleaved different channels. `data`/`seq` tiebreak
    // keeps it total and deterministic.
    let push_rank = producer_rank_by_data(&root);
    let rank_of = |d: DataId| push_rank.get(&d).copied().unwrap_or(usize::MAX);
    waits.sort_by(|a, b| {
        rank_of(a.data)
            .cmp(&rank_of(b.data))
            .then(a.dst.cmp(&b.dst))
            .then(a.src.cmp(&b.src))
            .then(a.data.cmp(&b.data))
            .then(a.seq.0.cmp(&b.seq.0))
    });

    for w in waits {
        // Idempotence keyed on `seq` ALONE — deliberately not on
        // (src,dst,data). `seq` is unique per Push/Wait pair (global
        // monotonic counter, see the SeqTag note in `inject_transfers`),
        // so "a Push with this seq exists" is the exact "this transfer
        // is already paired" predicate.
        //
        // We must NOT also skip on (src,dst,data) at this site:
        // legitimate multi-Wait shapes survive the upstream
        // Sequence-scope dedup at `inject_in_sequence` (which now
        // collapses same-(src,dst,data,tile) Waits within a
        // sync_inject barrier epoch — TASK-0335 cycle 158). What
        // reaches here as separate Waits with separate seqs is:
        //   - cross-epoch consumers (separated by a Sync barrier;
        //     each needs its own buffer place because the producer
        //     re-fires per phase), and
        //   - structurally distinct (src,dst,data,tile) tuples (e.g.
        //     different tile slices for partition-aware reads).
        // Either of those genuinely needs its own seq-keyed buffer
        // place in the Petri lowering; suppressing them by
        // (src,dst,data) at this site would leave a buffer place
        // unfilled and deadlock that consumer.
        //
        // Idempotence on re-run is guaranteed by two cooperating
        // mechanisms: Pass A collapses regenerated fresh-seq duplicate
        // Waits against the surviving original (by (src,dst,data) at
        // the destination sequence) BEFORE this pass runs, and the
        // Sequence-scope dedup above prevents new multi-consumer-Op
        // duplicates from being produced in the first place. So only
        // the original seq reaches here and its Push is already in
        // `have_seqs`.
        //
        // TASK-0335 cycle 158 historical note: pre-fix, host-side
        // multi-consumer-Op shapes (e.g. 03-reduction/distributed's
        // two `combine` ops reading `partials`) produced one Wait per
        // (consumer-Op × producer-worker) — for 2 ops × 4 producers,
        // 8 Waits → 8 Pushes here → producer-side splice ordering
        // inverted at the same insertion point → wire FIFO seq
        // mismatch panic on mp-tcp-bufsync. The fix lives at the
        // Sequence-scope Wait dedup site (above), not here: this
        // site's narrow seq-only dedup remains correct, but its
        // input is now upstream-filtered.
        if have_seqs.contains(&w.seq.0) {
            continue;
        }

        // Locate producer and its loop nesting vs the Wait's.
        let mut pp = None;
        producer_repeat_path(&root, w.data, &mut Vec::new(), &mut pp);
        let producer_path = match pp {
            Some(p) => p,
            // DEFENSE-IN-DEPTH behind the root-boundary check in
            // `inject_transfers` (the `escaped_at_root` panic). That
            // check should catch every producerless cross-worker Wait
            // BEFORE this pass runs: a Wait with no producing scope
            // bubbles out of Pass A's hoist and is rejected there.
            // This branch is therefore *expected* unreachable in
            // practice — but it is NOT proven unreachable: the two
            // guards key off different structures (Pass A's
            // whole-symbol-hoist root residue vs a per-`w.data`
            // producer walk of the post-hoist tree), and a future
            // hoist/block-transform change could let a Wait reach here
            // unpaired. Keep it as a loud `panic!` (not `unreachable!`,
            // which would assert a proof we do not have; not
            // `continue`, which would resurrect the silent-drop bug).
            // TASK-0152.
            //
            // Invariant rationale: a cross-worker Wait only exists
            // because `build_waits_for_op` found this symbol in
            // `producers_by_data` (mirrors `linked.data_producers`);
            // therefore a producing Operation MUST exist somewhere in
            // the ACFG (`build_acfg` places the producing kernel).
            // `None` here means the ACFG is malformed — a Wait whose
            // producer kernel was never lowered to an Operation. Fail
            // loud with full context per the `acfg.rs` fail-fast
            // precedent rather than silently leaving an unpaired Wait
            // for a downstream pass to mis-diagnose as a deadlock.
            None => {
                let data_name = name_data
                    .iter()
                    .find_map(|(n, id)| (*id == w.data).then_some(n.as_str()))
                    .unwrap_or("<unknown>");
                panic!(
                    "transfer_inject invariant violation: cross-worker Wait \
                     for data `{data_name}` (id {:?}, seq {:?}, {:?} -> {:?}) \
                     has no producing Operation in the ACFG. A Wait is only \
                     emitted when the schedule records a producer for the \
                     symbol, so this is a malformed ACFG (producer kernel not \
                     lowered), not a partial test input.",
                    w.data, w.seq, w.src, w.dst
                );
            }
        };

        // Single-assignment invariant (PRD §6.2.1, TASK-0153):
        // `producer_repeat_path` takes the FIRST Operation in walk
        // order that writes `w.data`. That is only well-defined if
        // there is exactly one. v2 data is single-assignment, so two
        // writers would be a front-end/lowering bug that would
        // mis-place the Push silently. Assert it in debug builds; no
        // release-build cost.
        debug_assert_eq!(
            count_producers(&root, w.data),
            1,
            "single-assignment violated: data id {:?} has multiple producing \
             Operations; producer_repeat_path would mis-place the Push",
            w.data
        );

        let mut wp = None;
        wait_repeat_path(&root, w.seq, &mut Vec::new(), &mut wp);
        let wait_path = wp.unwrap_or_default();

        let push = XferPlaceholder {
            role: XferRole::Push,
            src: w.src,
            dst: w.dst,
            data: w.data,
            tile: w.tile.clone(),
            seq: w.seq,
            policy: w.policy,
        };

        // The cut: outermost Repeat enclosing the producer that does
        // NOT also enclose the Wait. If present, the producer is a
        // loop output the consumer reads after the loop -> Push goes
        // after that Repeat. Otherwise Push goes right after producer.
        let cut = producer_path
            .iter()
            .find(|iv| !wait_path.contains(iv))
            .copied();

        root = match cut {
            Some(cut_iv) => splice_after_repeat(root, cut_iv, &push),
            None => splice_after_producer(root, &push),
        };

        have_seqs.insert(w.seq.0);
    }

    root
}

// --------------------------------------------------------------------
// Wait construction
// --------------------------------------------------------------------

/// For each cross-worker read in `op`, produce a Wait placeholder.
/// Multiple distinct reads of the same data symbol in one op yield
/// ONE Wait — the consumer needs one rendezvous per (op, data, src)
/// triple, not one per read.
pub(super) fn build_waits_for_op(
    op: &Operation,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
) -> Vec<XferPlaceholder> {
    let consumer_workers: BTreeSet<WorkerId> = op.workers.clone();
    let mut seen: BTreeSet<DataId> = BTreeSet::new();
    let mut out = Vec::new();

    for edge in &op.dataflow.edges {
        for &data_id in &edge.data_in {
            if !seen.insert(data_id) {
                continue;
            }
            let producer_workers = match ctx.producers_by_data.get(&data_id) {
                Some(p) => p,
                None => continue, // No recorded producer (e.g. unread const data).
            };
            if producer_workers == &consumer_workers {
                // Same worker set on both sides.
                //
                // Pre-cycle-147: unconditional `continue; no transfer`
                // (intra-worker dataflow). Cycle-144 AC#2 added a
                // validator that rejected the silent-miscompile arm
                // up-front so the elision here was always safe.
                //
                // Cycle-147 AC#3 lifts the validator's rejection for
                // this same-set case and classifies the elision HERE
                // with the same predicate (`same_set_elision_unsafe_reason`).
                // - SAFE → continue (intra-worker dataflow, as before;
                //   e.g. 13-cnn-inference/batch_parallel reader-iv ==
                //   partition-iv shape).
                // - UNSAFE → fall through to the cartesian-product
                //   fan-out below, which emits one cross-worker
                //   (src, dst) pair per src != dst member of the set.
                //   Each producer w_i pushes its partition slice
                //   (compute_worker = src per `rewrite_partition_tiles`'s
                //   N-to-1 gather rule); each consumer w_j receives
                //   (N-1) cross-pushes covering the other workers'
                //   slices; w_j's own slice is filled by local
                //   production. Combined: full data per worker —
                //   the N-to-N broadcast-of-gather shape the
                //   06-separable-filter/distributed2 schedule needs.
                //   `render_wait_assign`'s `WaitSlice::Rows` path
                //   handles the receiver-side row-band assignment.
                //
                // TASK-0325 cycle-145: this short-circuit is one of
                // TWO same-worker elision sites in this function. The
                // sibling (`if src == dst` inside the cartesian-
                // product fan-out below) elides per-pair when the
                // sets are NOT equal but share at least one worker —
                // the AC#3 extension to that path is NOT in cycle 147
                // scope (no in-tree schedule exercises partial-overlap).
                // The validator still rejects partial-overlap unsafe
                // shapes.
                // TASK-0328 cycle-154: the clause (1) mirror REMOVED in
                // lockstep with the validator-site removal. The
                // cycle-147 P2.1 fold-back kept the emit site in step
                // with the validator's (over-lenient) short-circuit.
                // Cycle-154 architect P1.1 fold-back: this fall-
                // through path is NOT a defensive backstop — it is the
                // primary code path for the silent-miscompile shape.
                // The cycle-147 AC#3 lift returns Ok on
                // `producer_workers == &consumer_workers && unsafe`
                // (the same-set + unsafe combination) precisely so the
                // emit site here can fall through to the cartesian-
                // product fan-out and emit the cross-worker pairs that
                // populate full data on each worker. Pre-fix: clause
                // (1) at the emit site short-circuited the same-set
                // unsafe path → no cross-worker pairs → silent
                // miscompile. Post-fix: emit reaches
                // `same_set_elision_unsafe_reason`; if unsafe, fall
                // through; if safe, continue (elide). Test pin:
                // `task0328_ac2_positive_partition_producer_topfile_consumer`
                // asserts 12 cross-worker pairs for the
                // consumer-at-top-level shape (was 0 pre-fix).
                let prod_access = ctx.producer_writes.get(&data_id);
                let is_safe = match prod_access {
                    None => true, // No producer-side access — pre-AC#2 fall-back path.
                    Some(p) if p.indices.is_empty() => true, // whole-array writer
                    Some(p) => same_set_elision_unsafe_reason(
                        edge,
                        data_id,
                        p,
                        ctx.partition_iter_vars,
                        ctx.name_iter_vars,
                    )
                    .is_none(),
                };
                if is_safe {
                    continue;
                }
                // Unsafe — fall through to the cartesian-product fan-
                // out. AC#3 cross-worker emission.
            }
            let policy = ctx
                .policies_by_data
                .get(&data_id)
                .copied()
                .unwrap_or_default();

            // TASK-0117 fan-out: emit one Wait per (src-worker,
            // dst-worker) member of the cartesian product of the
            // producer and consumer worker entities, skipping any
            // same-worker pair. Each pair gets its own fresh `seq` so
            // the projection's per-worker EventLists carry distinct
            // Pushes/Waits, and the backend can allocate per-pair
            // slots without collision.
            //
            // The pair's `tile` is the enclosing iteration tile with
            // any partitioned axis (TASK-0212) rewritten to the
            // *compute* worker's slice. The "compute worker" for a
            // pair is the worker in the multi-worker side: for a
            // (host -> {w0..w3}) pair to w_i, compute = w_i; for a
            // ({w0..w3} -> host) pair from w_i, compute = w_i. When
            // both sides are size 1, the source range survives
            // unchanged (no partition meaning) — this is exactly the
            // pre-TASK-0117 behaviour for {host} -> {single worker}
            // transfers.
            //
            // Determinism: BTreeSet iterates in sorted WorkerId
            // order; the cartesian-product enumeration is therefore
            // a deterministic function of (producer_workers,
            // consumer_workers). Seq tags are assigned monotonically
            // by `state.fresh_seq()` — same input ⇒ same seqs.
            // Emit one Wait per (src, dst) member of the cartesian
            // product. The per-pair `tile` is initialised here from
            // `enclosing_tile` (the construction-site semantic the
            // pre-TASK-0117 single-pair path used); the final
            // per-worker tile is set by `rewrite_partition_tiles`
            // at the end of `inject_transfers`, after the hoist +
            // splice passes have positioned the placeholders. Doing
            // the partition rewrite as a separate sink keeps the
            // hoist invariant (TASK-0151's loop-invariant tile
            // rewrite) untouched.
            for &src in producer_workers.iter() {
                for &dst in consumer_workers.iter() {
                    if src == dst {
                        // Per-element same-worker elision. Skipping
                        // this pair represents intra-worker dataflow
                        // (worker w reads its own writes — no
                        // cross-worker transfer needed).
                        //
                        // TASK-0325 cycle-145 (partial-overlap case):
                        // when `producer_workers != &consumer_workers`
                        // but they share at least one worker, this
                        // skip elides one transfer per overlap member.
                        // The SILENT-MISCOMPILE arm of this skip
                        // (consumer reads a slice the local producer
                        // does NOT own) is rejected up-front by
                        // `check_no_silent_elision_risk` for the
                        // partial-overlap shape — no in-tree schedule
                        // exercises it.
                        //
                        // TASK-0333 cycle-155 audit confirmed (paired-
                        // lift sweep of the cycle-154 clause-(1)
                        // removal at the same-set arm): this per-element
                        // skip is UNCONDITIONAL — it has no consumer-
                        // scope gate analogous to clause (1), so there
                        // is nothing structurally identical to remove
                        // here. The validator's rejection
                        // (`check_op_no_silent_elision_risk` →
                        // `SameSetSilentElisionRisk`) is the load-
                        // bearing safety net for the partial-overlap
                        // unsafe shape. Regression pin:
                        // `task0333_ac1_partial_overlap_partition_producer_topfile_consumer_rejects`.
                        //
                        // TASK-0324 cycle-147 AC#3 (same-set case):
                        // when the whole-set short-circuit above falls
                        // through (unsafe shape, AC#3 emit), the
                        // cartesian product enumerates (w_i, w_j) for
                        // every pair. This skip then fires for the
                        // N self-pairs `(w_i, w_i)`, leaving only
                        // N*(N-1) cross-pairs emitted. The self-pair
                        // skip is correct: each consumer's own slice
                        // is filled by local production (the same
                        // worker also being the producer), so no
                        // cross-worker transfer is needed for that
                        // axis. The receiver-side `render_wait_assign`
                        // composes local production + (N-1) cross-
                        // pushes into the full data.
                        continue;
                    }
                    out.push(XferPlaceholder {
                        role: XferRole::Wait,
                        src,
                        dst,
                        data: data_id,
                        tile: IterTile::new(enclosing_tile.to_vec()),
                        seq: state.fresh_seq(),
                        policy,
                    });
                }
            }
        }
    }

    // TASK-0389: FIFO host-Push / worker-Wait ordering robustness.
    //
    // On a strict-FIFO channel (mp-tcp-bufsync / mp-tcp-poll via
    // `wire::read_msg_expect(cv, seq)`) the receiver reads the NEXT
    // message on the channel and asserts its seq tag equals the expected
    // one — an EXACT MATCH in FIFO ORDER (NOT a monotonicity check; the
    // tag is purely a pairing sanity key, verified TASK-0389: the wire
    // runtime never compares two seqs for `<`). So the worker's Wait
    // order on each (src -> dst) channel MUST match the order the sender
    // wrote, which is the producer-statement order `splice_pushes_global`
    // realises for the host's Pushes.
    //
    // The Waits above were emitted in `data_in` TRAVERSAL order (the
    // outer `for edge { for data_in }` walk). For a data-dependent gather
    // `x[col_idx[i][k]]`, `collect_dataref_access_expr` recurses
    // index-FIRST, so `col_idx` precedes `x` in `data_in` regardless of
    // declaration order — but the host sends them in producer order. When
    // the gathered array `x` is produced BEFORE its index array
    // `col_idx`, those two orders DIVERGE and `read_msg_expect` panics
    // ("receiver expected 4, wire delivered 8"). See
    // `prog.gather_revdecl.algo.nuc` for the e2e repro.
    //
    // Fix: STABLE-sort the Waits so that, within each (dst, src) channel,
    // they appear in producer-statement rank order. The seqs already
    // travel with their (src, dst, data) pair (the host's Push for each
    // Wait is matched by `seq` at `splice_pushes_global`), so once both
    // endpoints traverse the channel in producer order the tags line up
    // BY CONSTRUCTION — no seq reallocation. The `(dst, src)` lead key
    // groups each FIFO; `producer_rank` orders within it; the
    // `data`/`seq` tiebreak keeps the order total and deterministic.
    // Cross-channel relative order does not affect FIFO correctness
    // (independent sockets) but stays deterministic via the lead key.
    //
    // NO-OP for current declaration orders: when `data_in` traversal
    // already equals producer order (every non-gather program, and
    // `prog.gather.algo.nuc` where `col_idx` is declared before `x`),
    // this stable sort preserves the existing order, so the emitted Wait
    // statements — and the byte output — are unchanged (verified by e2e
    // byte-identity on all pre-existing cells, TASK-0389 AC#5).
    //
    // SCOPE of the producer-rank key (RESOLVED — TASK-0389.01).
    // `producer_rank` is the raw producer-Operation walk position. This
    // sort orders each worker's per-channel Wait sequence by it. The
    // host side is now made to MATCH this exact order:
    // `splice_pushes_global` feeds its Pushes in producer-rank order AND
    // `splice_after_repeat` / `splice_after_producer` APPEND each new
    // Push after any already-spliced Pushes at the same insertion point.
    // So the host's textual (= wire-send) Push order on each (src→dst)
    // channel EQUALS producer-rank order == this Wait order, for ANY
    // loop-output nesting — including the residual TASK-0389 could not
    // yet guarantee: ≥2 loop-OUTPUT data on ONE channel co-hoisting past
    // the SAME Repeat (the `cut` branch). Pre-fix that case REVERSED the
    // co-hoisted Pushes — the host sent reverse-rank while the worker
    // waited rank-order — a strict-FIFO `read_msg_expect` seq mismatch
    // (captured: 18-multigather/distributed on mp-tcp-bufsync, "receiver
    // expected 2, wire delivered 3"). The two endpoints now traverse
    // every channel in the SAME producer-rank order, so the seq tags
    // pair by construction. See `splice_after_repeat`'s docstring and
    // the `task038901_*` unit tests + the 18-multigather e2e cell.
    let rank = |d: DataId| ctx.producer_rank.get(&d).copied().unwrap_or(usize::MAX);
    out.sort_by(|a, b| {
        a.dst
            .cmp(&b.dst)
            .then(a.src.cmp(&b.src))
            .then_with(|| rank(a.data).cmp(&rank(b.data)))
            .then(a.data.cmp(&b.data))
            .then(a.seq.0.cmp(&b.seq.0))
    });

    out
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Convert a [`WorkerEntity`] (`BTreeSet<String>`) to a
/// `BTreeSet<WorkerId>` using the ACFG's name table. Skip names that
/// aren't in the table — that would be a link-pass invariant
/// violation; we don't loudly panic so downstream tests that build
/// synthetic ACFGs can still feed an empty/partial name table.
pub(super) fn entity_to_workerid_set(
    entity: &WorkerEntity,
    name_workers: &BTreeMap<String, WorkerId>,
) -> BTreeSet<WorkerId> {
    entity
        .0
        .iter()
        .filter_map(|name| name_workers.get(name).copied())
        .collect()
}

/// Convert a [`ResolvedTransferDirective`]'s option list into a
/// [`TransferPolicy`]. Multiple options in one directive are walked
/// in order; later options override earlier ones for the same field.
///
/// The defaults match [`TransferPolicy::default`]: synchronous, buffer
/// 1, notify-default.
pub(super) fn policy_from_directive(dir: &ResolvedTransferDirective) -> TransferPolicy {
    let mut p = TransferPolicy::default();
    for opt in &dir.options {
        match opt {
            ResolvedTransferOption::Sync => p.synchronous = true,
            ResolvedTransferOption::Async => p.synchronous = false,
            ResolvedTransferOption::Buffer(n) => p.buffer = *n,
            ResolvedTransferOption::Notify(k) => p.notify = NotifyMode::from(*k),
        }
    }
    p
}

/// Return the data symbol this Operation writes, if any.
/// At M1 a [`crate::acfg::DataflowDag`] holds one edge per firing; we
/// take the first edge's `data_out`. If a future multi-edge DAG lands,
/// this helper widens to a `Vec<DataId>` and callers adjust.
pub(super) fn output_data(op: &Operation) -> Option<DataId> {
    op.dataflow.edges.first().and_then(|e| e.data_out)
}

/// Update the `last_writer` map with `op`'s output, if any.
pub(super) fn update_writer(state: &mut State, op: &Operation) {
    if let Some(d) = output_data(op) {
        state.last_writer.insert(d, op.workers.clone());
    }
}
