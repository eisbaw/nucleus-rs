//! Sync injection — TASK-0017.
//!
//! Walks the ACFG and inserts [`ACFGNode::Sync`] barriers between
//! regions where workers must rendezvous before progressing.
//!
//! ## Rules implemented (from the task spec)
//!
//! 1. **Sequence boundary.** Between two consecutive children of a
//!    [`ACFGNode::Sequence`] where the first child *writes* on
//!    workers `W1`, the second child *reads* on workers `W2`, and
//!    `W1 != W2`: insert a Sync with participants `W1 ∪ W2`.
//!
//! 2. **Repeat entry.** At the boundary into a [`ACFGNode::Repeat`]
//!    body whose body's workers differ from the prior statement's
//!    writing workers: prepend a Sync to the body's sequence whose
//!    participants are `W_prior ∪ W_body`.
//!
//! 3. **Repeat exit.** If cross-worker writes happen *inside* a
//!    Repeat body (i.e. the body's writing workers span more than
//!    one worker), append a Sync at the body's end whose
//!    participants are exactly those writing workers.
//!
//! 4. **Elision.** A `Sync` whose participant set has fewer than two
//!    elements is meaningless (no rendezvous with self); the pass
//!    never emits such a Sync.
//!
//! ## Idempotence
//!
//! `inject_syncs(inject_syncs(x)) == inject_syncs(x)` for any ACFG
//! `x`. This holds because:
//!
//! - Sync nodes have empty reads/writes, so they never trigger a
//!   new Sync between themselves and neighbours on a re-run.
//! - Each rule checks whether the placeholder it would place is
//!   already there. The pass therefore replaces rather than appends.
//!
//! Both behaviours are tested in `tests/sync_inject.rs`.
//!
//! ## Honest limitations
//!
//! - **Over-syncs (partially fixed by TASK-0218).** The Sequence
//!   rule no longer emits a Sync between two bare
//!   [`ACFGNode::Operation`] nodes when their dataflow already shares
//!   a data symbol that crosses the worker boundary — `transfer_inject`
//!   will emit a Push/Wait pair for that symbol, and the Push/Wait
//!   pair already supplies the rendezvous the barrier would have
//!   added. See [`push_wait_pair_covers`] for the exact condition.
//!   The pre-TASK-0218 over-sync still applies to Sequence boundaries
//!   where prev/curr are nested (Sequence/Repeat) and to Repeat
//!   entry/exit barriers — those are more involved to reason about
//!   safely without consulting the global Push/Wait coverage. PRD §8
//!   places sync only at "control-flow joins where no data crosses";
//!   we still err on the side of safety on the nested shapes.
//! - **No conditionals.** The algorithm grammar has no `if` (PRD
//!   §6.2.4), so [`ACFGNode`] has no `If` variant; this pass has
//!   no `If` arm.
//! - **No optimisation pass.** Adjacent Syncs with identical
//!   participant sets are not merged. The rule set produces at
//!   most one Sync per boundary, so adjacency requires the user to
//!   write back-to-back patterns that wouldn't merge sensibly
//!   anyway. Filed as a follow-up if a real example trips on it.
//!
//! ## Why a separate module under `passes/`
//!
//! The `acfg` module is the type definition. Mixing rewrites into
//! the same file conflates "what an ACFG is" with "what passes do
//! to one". Future passes (transfer injection, Petri-net
//! construction) follow the same pattern: one file per pass under
//! `passes/`.

use std::collections::BTreeSet;

use crate::acfg::{ACFGNode, SyncPlaceholder, ACFG};
use crate::event::{IterVar, SyncTag, WorkerId};

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Inject [`ACFGNode::Sync`] barriers into `acfg` per the rules in
/// the module docs. Consumes the input and returns a new ACFG; the
/// name-table maps are forwarded unchanged.
///
/// Idempotent: `inject_syncs(inject_syncs(x))` is structurally equal
/// to `inject_syncs(x)`.
pub fn inject_syncs(acfg: ACFG) -> ACFG {
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        // TASK-0134: sync_inject does not consult or mutate the
        // pipeline-depth sidecar; forward verbatim. transfer_inject
        // populates it; acfg_to_petri reads it.
        pipeline_depth_for_seq,
        // TASK-0260: sync_inject does not consult or mutate the halo-
        // widths sidecar; forward verbatim. halo_inference populates it;
        // Stage 2 (TASK-0263) makes transfer_inject the consumer.
        halo_widths,
        // TASK-0261: sync_inject does not consult or mutate the reuse-
        // widths sidecar; forward verbatim. reuse_inference populates
        // it; Stage 2 (TASK-0265) makes the backend walker the consumer.
        reuse_widths,
    } = acfg;

    // The set of iter-vars that the partition-{workers,rows,blocks2d}
    // passes (TASK-0212 / TASK-0258 / TASK-0259) have rewritten into
    // per-worker slices. For such a Repeat the workers iterate
    // disjoint sub-ranges while non-participating workers (host) still
    // project the source range; a per-iteration body-entry /
    // body-exit barrier in this shape *deadlocks* because each
    // iteration expects N+1 arrivals (host + N compute workers) but
    // only the host arrives in iterations that no compute worker
    // covers, and 1 worker + host in iterations that one covers. We
    // therefore skip the body-internal entry/exit Sync insertions on
    // partitioned Repeats; the loop-boundary Syncs (the Sequence rule
    // between the prior/next siblings and the Repeat itself) survive
    // unchanged and provide the once-before / once-after barrier
    // semantic that an embarrassingly-parallel partition expects.
    //
    // TASK-0268: the skip MUST also propagate to INNER Repeats nested
    // within a partitioned Repeat. Otherwise an inner `loop x : block=N`
    // (or any non-partitioned inner Repeat) still emits its
    // wrap_repeat_body body-exit barrier, and under floor-with-spillover
    // remainder policy (TASK-0262 — 4 workers × 14 rows ⇒ 4/4/3/3
    // unequal iteration counts) workers with FEWER outer-y iterations
    // exit early and leave inner-x barrier participants short. Workers
    // with MORE outer-y iterations then block forever on the inner-x
    // exit Sync. The propagation flag `inside_partition` is sticky
    // once a partitioned Repeat is entered — every Repeat below it
    // skips wrap_repeat_body too. The same single-assignment argument
    // applies inside-down: the body has no cross-iteration data
    // dependency (PRD §6.2.1), so the only cross-worker rendezvous
    // anywhere inside the partition is the Push/Wait pair around its
    // outermost boundary (which the loop-boundary Sequence rule still
    // emits). Per-iteration body barriers inside the partition were
    // never structurally required — pre-TASK-0268 they coincidentally
    // worked only for equal-iter-count partitions (12-rows × 4-workers
    // shape).
    //
    // Why this is correct: the cross-worker data the entry Sync was
    // guarding is delivered by the Push/Wait pair (TASK-0117 fan-out
    // makes each compute worker receive its slice before its body
    // runs); the exit Sync was guarding the gather back to the host,
    // which is again handled by the Push/Wait gather pair. The body
    // has no cross-iteration data dependency (single-assignment + per-
    // sample dataflow), so per-iteration synchronisation is redundant
    // for the partition=workers/rows/blocks2d shape.
    let partitioned_iter_vars: BTreeSet<IterVar> =
        partition_worker_ranges.keys().copied().collect();

    // `prior_writes` for the outer-most call is empty: there is no
    // statement before the program root. `inside_partition=false` at
    // the root — only set to true on entering a partitioned Repeat.
    let mut new_root = inject_in_node(root, &BTreeSet::new(), &partitioned_iter_vars, false);

    // Assign the stable cross-worker barrier identity (TASK-0172).
    //
    // This is the `Sync` analogue of how `transfer_inject` assigns
    // `XferPlaceholder::seq`: a monotonic counter handed out where the
    // *global* barrier structure is visible (here: the whole injected
    // ACFG). We do it as a deterministic **pre-order** walk of the
    // FINAL tree rather than during injection because the injection
    // walk creates Syncs out of final-tree order (it recurses children
    // before inserting a Sequence-boundary Sync, and `insert(0, ..)`s
    // the Repeat-entry Sync). Pre-order of the final tree is exactly
    // the order `petri_to_events::walk` visits `ACFGNode::Sync`, and
    // hence the order each participant encounters the barrier in its
    // projected `EventList`. For a *uniform*-barrier program (every
    // Sync has the same participant set — the tier-1 shape, e.g.
    // example 02-split's three `{host,w0}` barriers) this yields the
    // same 0,1,2,… numbering the old backend pre-order-index heuristic
    // produced, so generated code stays byte-identical. The tag is
    // assigned ONCE per barrier node and then projected (cloned) into
    // every participant's list by `petri_to_events::emit_sync`, so all
    // participants of one barrier carry the same `SyncTag` — the
    // cross-worker join key, with no global walk required of any
    // backend. The walk is over `Vec` children + `BTreeSet`
    // participants only (no `HashMap`/`HashSet` iteration), so the
    // assignment is reproducible: same program ⇒ same tags.
    let mut next_sync: u64 = 0;
    assign_sync_tags(&mut new_root, &mut next_sync);

    ACFG {
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
    }
}

// --------------------------------------------------------------------
// Recursive walker
// --------------------------------------------------------------------

/// Recurse into a single node. `prior_writes` is the set of writing
/// workers immediately preceding *this* node in its enclosing
/// sequence (empty if `node` is the first child or if `node` is the
/// program root).
///
/// Returns the rewritten node. Only [`ACFGNode::Sequence`] and
/// [`ACFGNode::Repeat`] descend; the other variants are returned
/// unchanged.
fn inject_in_node(
    node: ACFGNode,
    prior_writes: &BTreeSet<WorkerId>,
    partitioned_iter_vars: &BTreeSet<IterVar>,
    inside_partition: bool,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => ACFGNode::Sequence(inject_in_sequence(
            children,
            partitioned_iter_vars,
            inside_partition,
        )),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            // 1) Recurse into the body first so any inner Sequence
            //    rules and any nested Repeat rules are applied before
            //    we look at the body-boundary rules. `prior_writes`
            //    for the body's first statement is empty in the
            //    recursive call: from the body's *own* perspective,
            //    nothing precedes its first statement inside the
            //    Repeat. The boundary into the Repeat is handled by
            //    the wrap step below.
            //
            //    TASK-0268: `inside_partition` is sticky downward —
            //    once we enter a partitioned Repeat, every descendant
            //    Repeat skips wrap_repeat_body too, because the inner
            //    Repeats' body-entry/exit barriers would still
            //    deadlock under floor-with-spillover unequal per-
            //    worker iteration counts driven by the outer partition.
            let is_partitioned = partitioned_iter_vars.contains(&iter_var);
            let inner_inside_partition = inside_partition || is_partitioned;
            let inner = inject_in_node(
                *body,
                &BTreeSet::new(),
                partitioned_iter_vars,
                inner_inside_partition,
            );

            // 2) Apply Repeat entry/exit rules. The result is still a
            //    Sequence (the body of a Repeat is always a Sequence
            //    by construction, see acfg::build_stmt). A partitioned
            //    Repeat OR any Repeat nested under one (TASK-0268)
            //    skips this step entirely — per-iteration body Syncs
            //    would deadlock because host iterates the source range
            //    while each compute worker iterates its slice. The
            //    cross-worker data the boundary Syncs protect is
            //    delivered by the TASK-0117 fan-out Push/Wait pairs
            //    around the partitioned loop instead.
            let wrapped = if is_partitioned || inside_partition {
                inner
            } else {
                wrap_repeat_body(inner, prior_writes)
            };

            ACFGNode::Repeat {
                iter_var,
                range,
                body: Box::new(wrapped),
                // sync_inject only injects barriers into the body; the
                // strip-mine rebinding tag is structural and survives
                // verbatim (TASK-0180).
                block_tag,
            }
        }
        // Leaves: nothing to inject inside.
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => leaf,
    }
}

/// Assign each [`ACFGNode::Sync`]'s stable [`SyncTag`] in a
/// deterministic **pre-order** walk of the final injected tree.
///
/// `next` is the monotonic counter (mirrors
/// `transfer_inject::State::fresh_seq`). Pre-order — visit the node,
/// then recurse children left-to-right, descending into `Repeat`
/// bodies — matches the order `petri_to_events::walk` materialises
/// `Event::Sync`, so each participant's projected `EventList` sees
/// barrier tags in ascending order and, for uniform-barrier programs,
/// in the same 0,1,2,… sequence the old per-worker pre-order-index
/// heuristic produced (keeps generated code byte-identical there).
///
/// Iteration is over `Vec` children only, so the assignment is fully
/// deterministic: the same ACFG always yields the same tags.
fn assign_sync_tags(node: &mut ACFGNode, next: &mut u64) {
    match node {
        ACFGNode::Sync(s) => {
            s.sync = SyncTag(*next);
            *next += 1;
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                assign_sync_tags(c, next);
            }
        }
        ACFGNode::Repeat { body, .. } => assign_sync_tags(body, next),
        ACFGNode::Operation(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Apply the Sequence boundary rule across the children of a
/// Sequence, then recurse into each child (with the correct
/// `prior_writes` argument).
///
/// We process the children left to right, building `out`. For each
/// adjacent pair `(out.last(), child)` we check the rule and push a
/// Sync between them when needed.
fn inject_in_sequence(
    children: Vec<ACFGNode>,
    partitioned_iter_vars: &BTreeSet<IterVar>,
    inside_partition: bool,
) -> Vec<ACFGNode> {
    let mut out: Vec<ACFGNode> = Vec::with_capacity(children.len());

    for child in children {
        // What writes did the previous element produce? Needed both
        // for the Sequence rule and to feed into the recursion (a
        // Repeat needs its prior_writes to apply the entry rule).
        let prior_writes = out.last().map(writing_workers).unwrap_or_default();

        // Recurse FIRST: a Repeat's prior_writes argument must be
        // computed from the just-emitted Sequence neighbour, which
        // could itself be a Sync this pass inserted on a previous
        // iteration. Computing prior_writes after the rule but before
        // recursion is the correct ordering.
        let child = inject_in_node(child, &prior_writes, partitioned_iter_vars, inside_partition);

        // Sequence rule: insert a Sync between `out.last()` and
        // `child` if their worker sets disagree on the write/read
        // axis. Skip if the previous node is already a Sync — the
        // boundary is already barriered (idempotence).
        //
        // TASK-0268: also skip when `inside_partition` is true.
        // Sequence-boundary Syncs inside a partitioned scope fire
        // PER OUTER PARTITION ITERATION; under floor-with-spillover
        // unequal per-worker iter counts (4/4/3/3) workers with
        // fewer iterations exit early and leave the barrier short of
        // participants. The cross-worker data the Sequence boundary
        // would guard is already covered by the Push/Wait pair
        // (TASK-0117 fan-out around the partitioned loop boundary).
        //
        // ENVELOPE LIMIT — TASK-0281: this skip is UNCONDITIONAL
        // inside a partitioned scope; `push_wait_pair_covers` is not
        // consulted. Today the skip is sound because all shipped
        // partitioned schedules satisfy PRD §6.2.1 single-assignment
        // AND every cross-worker dataflow edge crossing the
        // partitioned-loop boundary is already covered by the
        // TASK-0117 fan-out Push/Wait pairs (verified: 03-reduction's
        // distributed reduction lives OUTSIDE the partitioned scope;
        // 05-stencil's blur3 is single-assignment per-output-cell).
        // A future M6+ schedule that nests a cross-partition reducer
        // (an inner Sequence writing to a shared output region placed
        // on a worker set DIFFERENT from the partition's inner-body
        // worker set, NOT covered by Push/Wait) would silently lose
        // synchronisation. Filed as TASK-0281 — if an M6 schedule
        // exercises this shape, replace the unconditional skip with
        // `push_wait_pair_covers || partitioned_scope_covers` (the
        // latter is the deeper participants-aware check option D
        // from cycle-85 analysis).
        if !inside_partition {
            if let Some(prev) = out.last() {
                if !matches!(prev, ACFGNode::Sync(_)) && !matches!(child, ACFGNode::Sync(_)) {
                    let w1 = writing_workers(prev);
                    let w2 = reading_workers(&child);
                    if !w1.is_empty() && !w2.is_empty() && w1 != w2 {
                        let participants: BTreeSet<WorkerId> = w1.union(&w2).copied().collect();
                        if participants.len() >= 2 && !push_wait_pair_covers(prev, &child) {
                            // `sync` is a placeholder here; the real stable
                            // tag is assigned by `assign_sync_tags` in a
                            // deterministic pre-order pass over the final
                            // tree (creation order ≠ final-tree order).
                            out.push(ACFGNode::Sync(SyncPlaceholder {
                                participants,
                                sync: SyncTag(0),
                            }));
                        }
                    }
                }
            }
        }

        out.push(child);
    }

    out
}

/// TASK-0218: would the Push/Wait pair `transfer_inject` will later
/// emit fully cover the rendezvous a Sequence-rule barrier between
/// `prev` and `curr` would impose?
///
/// Returns `true` iff:
///
/// 1. Both `prev` and `curr` are bare [`ACFGNode::Operation`] nodes —
///    NOT Sequence/Repeat. The "Push -> barrier -> Wait" structural
///    shape the task description identifies (the dependency cycle in
///    the analysis net) only holds when the Push lands directly after
///    `prev` and the Wait directly before `curr`. For nested `prev`
///    or `curr`, transfer_inject inserts Push/Wait deeper inside;
///    those don't sit immediately around the barrier and the simple
///    elision argument does not apply.
///
/// 2. There exists at least one data symbol `D` such that
///    `D` is written by `prev` and read by `curr`. transfer_inject
///    emits one Push/Wait pair per cross-worker dataflow edge per
///    (src, dst) worker pair (TASK-0117 fan-out across cartesian
///    product). For bare Operations on disjoint worker sets writing/
///    reading a shared `D`, the cartesian product of producer × consumer
///    workers (minus same-worker pairs) covers exactly the barrier's
///    `W1 ∪ W2` participant set.
///
/// When both hold, the future Push/Wait pair provides the rendezvous
/// the barrier would have provided, and emitting the barrier is
/// strictly over-synchronisation. Concretely: the per-worker control
/// chain orders prev_op -> Push -> ... on the producer, and ... ->
/// Wait -> curr_op on the consumer; the buffer place links Push -> Wait;
/// the chain prev_op -> Push -> Wait -> curr_op is the rendezvous a
/// barrier would have added. The barrier on top creates a structural
/// dependency cycle in the analysis net for pipelined loops (Push
/// blocked on full buffer, Wait blocked on barrier, barrier blocked
/// on Push — see [`crate::passes::acfg_to_petri`] module doc
/// "sync_inject over-syncing forces the path-2 elision").
///
/// Honest scope:
///
/// - Conservative on nested shapes. Repeat exit syncs (writers in
///   body span 2+ workers) and Repeat entry syncs (prior_writes vs
///   body_workers differ) are NOT elided here, even when individual
///   Push/Wait pairs cover all the dataflow. Those barriers
///   participants don't map 1:1 to a single Push/Wait pair the way
///   the bare-Operation case does, and getting the elision condition
///   right for them needs a separate pass that looks at the global
///   Push/Wait coverage (TASK-0218 follow-up if a real example
///   demands it).
///
/// - Same-worker shared symbols don't trigger. The outer `w1 != w2`
///   test already rules out same-worker cases, so this helper only
///   fires when transfer_inject WILL emit a cross-worker Push/Wait.
fn push_wait_pair_covers(prev: &ACFGNode, curr: &ACFGNode) -> bool {
    let (ACFGNode::Operation(prev_op), ACFGNode::Operation(curr_op)) = (prev, curr) else {
        return false;
    };

    // Collect `prev`'s written data symbols and `curr`'s read data
    // symbols, then check for any shared symbol. transfer_inject keys
    // its Push/Wait emission off the same `(producer_workers,
    // consumer_workers, data)` triple via `linked.data_producers` /
    // per-Operation `data_in`; here we don't have `LinkedIR` so we
    // read the dataflow edges directly. A shared symbol with the
    // outer `w1 != w2` test means the worker sets differ AND the
    // dataflow edge crosses workers — exactly what transfer_inject
    // turns into a Push/Wait pair.
    let prev_writes: BTreeSet<crate::event::DataId> = prev_op
        .dataflow
        .edges
        .iter()
        .filter_map(|e| e.data_out)
        .collect();
    if prev_writes.is_empty() {
        return false;
    }
    curr_op
        .dataflow
        .edges
        .iter()
        .flat_map(|e| e.data_in.iter().copied())
        .any(|d| prev_writes.contains(&d))
}

/// Apply the Repeat entry and exit rules to `body`. `body` is always
/// a [`ACFGNode::Sequence`] in a well-formed ACFG (see
/// `acfg::build_stmt`); if it isn't, we wrap it in one so the
/// boundary inserts work uniformly.
///
/// Entry rule: if `prior_writes` (workers writing immediately before
/// the Repeat in its enclosing sequence) is non-empty and differs
/// from the body's workers, prepend a Sync.
///
/// Exit rule: if the body has cross-worker writes (its writing
/// workers span 2+ distinct workers), append a Sync.
///
/// Both rules skip if the corresponding boundary already begins/
/// ends with a Sync (idempotence).
fn wrap_repeat_body(body: ACFGNode, prior_writes: &BTreeSet<WorkerId>) -> ACFGNode {
    let mut seq = match body {
        ACFGNode::Sequence(children) => children,
        // A non-Sequence body is unexpected from build_acfg, but
        // handle it by wrapping into a singleton sequence so the
        // boundary logic doesn't care.
        other => vec![other],
    };

    // ---- Entry rule ----
    let body_workers = workers_in(&seq);
    let already_entry_sync = matches!(seq.first(), Some(ACFGNode::Sync(_)));
    if !already_entry_sync && !prior_writes.is_empty() && !body_workers.is_empty() {
        // "body's workers differ from the prior statement's writing
        // workers": treat as set inequality.
        if &body_workers != prior_writes {
            let participants: BTreeSet<WorkerId> =
                prior_writes.union(&body_workers).copied().collect();
            if participants.len() >= 2 {
                // Placeholder tag; real tag assigned by
                // `assign_sync_tags` (final-tree pre-order).
                seq.insert(
                    0,
                    ACFGNode::Sync(SyncPlaceholder {
                        participants,
                        sync: SyncTag(0),
                    }),
                );
            }
        }
    }

    // ---- Exit rule ----
    let inner_writers = writers_in(&seq);
    let already_exit_sync = matches!(seq.last(), Some(ACFGNode::Sync(_)));
    if !already_exit_sync && inner_writers.len() >= 2 {
        // "cross-worker writes happened inside" -> participants are
        // all writing workers in the body.
        seq.push(ACFGNode::Sync(SyncPlaceholder {
            participants: inner_writers,
            // Placeholder tag; real tag assigned by `assign_sync_tags`.
            sync: SyncTag(0),
        }));
    }

    ACFGNode::Sequence(seq)
}

// --------------------------------------------------------------------
// Read/write worker-set computation
// --------------------------------------------------------------------

/// Workers that this node *writes* (produces data on, or executes
/// effect kernels on). Defined recursively for Repeat/Sequence:
///
/// - [`ACFGNode::Operation`]: the op's worker set if it has any
///   `data_out`, or any effect-kernel firing (which has no data_out
///   but still executes on those workers and may have observable
///   side effects).
/// - [`ACFGNode::Sequence`]: union of children's writes.
/// - [`ACFGNode::Repeat`]: writes of the body.
/// - [`ACFGNode::Sync`] / [`ACFGNode::Xfer`]: empty (these are
///   control/transfer events, not user writes).
///
/// The Sync/Xfer empty case is what makes the pass idempotent.
fn writing_workers(node: &ACFGNode) -> BTreeSet<WorkerId> {
    match node {
        ACFGNode::Operation(op) => {
            // An Operation always represents one firing on
            // `op.workers`. Effect kernels (no data_out) and
            // dataflow kernels both "happen on" their workers, and
            // for the purpose of sync injection we treat both as
            // writes: an effect that the next statement depends on
            // (e.g. `save_output` reading from `output`) still
            // demands the writer reaches a known state first.
            op.workers.clone()
        }
        ACFGNode::Sequence(children) => union_of(children.iter().map(writing_workers)),
        ACFGNode::Repeat { body, .. } => writing_workers(body),
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => BTreeSet::new(),
    }
}

/// Workers that this node *reads* on. Mirror of [`writing_workers`].
///
/// - [`ACFGNode::Operation`]: the op's worker set if any edge has
///   `data_in` (i.e. it reads at least one data symbol). An op with
///   no inputs (e.g. `load_input` with `()` signature) is not a
///   reader.
fn reading_workers(node: &ACFGNode) -> BTreeSet<WorkerId> {
    match node {
        ACFGNode::Operation(op) => {
            if op.dataflow.edges.iter().any(|e| !e.data_in.is_empty()) {
                op.workers.clone()
            } else {
                BTreeSet::new()
            }
        }
        ACFGNode::Sequence(children) => union_of(children.iter().map(reading_workers)),
        ACFGNode::Repeat { body, .. } => reading_workers(body),
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => BTreeSet::new(),
    }
}

/// Union of workers across a list of nodes — `writing_workers`
/// semantics. Used for the Repeat entry rule's "body workers".
fn workers_in(nodes: &[ACFGNode]) -> BTreeSet<WorkerId> {
    union_of(nodes.iter().map(writing_workers))
        .union(&union_of(nodes.iter().map(reading_workers)))
        .copied()
        .collect()
}

/// Writing-worker union over a list of nodes — used for the Repeat
/// exit rule. Distinct from [`workers_in`] because the exit rule
/// specifically tests cross-worker *writes*.
fn writers_in(nodes: &[ACFGNode]) -> BTreeSet<WorkerId> {
    union_of(nodes.iter().map(writing_workers))
}

fn union_of<I>(sets: I) -> BTreeSet<WorkerId>
where
    I: IntoIterator<Item = BTreeSet<WorkerId>>,
{
    let mut out = BTreeSet::new();
    for s in sets {
        out.extend(s);
    }
    out
}

// --------------------------------------------------------------------
// Inspection helpers (used by tests; small enough to keep here)
// --------------------------------------------------------------------

impl ACFGNode {
    /// Count [`ACFGNode::Sync`] nodes in this subtree. Sister to
    /// the existing `count_operations` / `count_repeats` on
    /// [`crate::acfg::ACFGNode`]. Used by `tests/sync_inject.rs`
    /// for structural assertions.
    pub fn count_syncs(&self) -> usize {
        match self {
            ACFGNode::Sync(_) => 1,
            ACFGNode::Repeat { body, .. } => body.count_syncs(),
            ACFGNode::Sequence(children) => children.iter().map(ACFGNode::count_syncs).sum(),
            ACFGNode::Operation(_) | ACFGNode::Xfer(_) => 0,
        }
    }
}

impl ACFG {
    /// Total [`ACFGNode::Sync`] count across the whole ACFG.
    pub fn sync_count(&self) -> usize {
        self.root.count_syncs()
    }
}
