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
//! - **Over-syncs**. The Sequence rule does not check whether the
//!   writer's data actually feeds the reader; any pair of stmts on
//!   different worker sets where one writes and the next reads
//!   gets a sync. PRD §8 places sync only at "control-flow joins
//!   where no data crosses"; we err on the side of safety until
//!   transfer injection (TASK-0018) tells us which dataflow edges
//!   are already covered by Push/Wait.
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
use crate::event::WorkerId;

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
    } = acfg;

    // `prior_writes` for the outer-most call is empty: there is no
    // statement before the program root.
    let new_root = inject_in_node(root, &BTreeSet::new());

    ACFG {
        root: new_root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
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
fn inject_in_node(node: ACFGNode, prior_writes: &BTreeSet<WorkerId>) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => ACFGNode::Sequence(inject_in_sequence(children)),
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
            let inner = inject_in_node(*body, &BTreeSet::new());

            // 2) Apply Repeat entry/exit rules. The result is still a
            //    Sequence (the body of a Repeat is always a Sequence
            //    by construction, see acfg::build_stmt).
            let wrapped = wrap_repeat_body(inner, prior_writes);

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

/// Apply the Sequence boundary rule across the children of a
/// Sequence, then recurse into each child (with the correct
/// `prior_writes` argument).
///
/// We process the children left to right, building `out`. For each
/// adjacent pair `(out.last(), child)` we check the rule and push a
/// Sync between them when needed.
fn inject_in_sequence(children: Vec<ACFGNode>) -> Vec<ACFGNode> {
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
        let child = inject_in_node(child, &prior_writes);

        // Sequence rule: insert a Sync between `out.last()` and
        // `child` if their worker sets disagree on the write/read
        // axis. Skip if the previous node is already a Sync — the
        // boundary is already barriered (idempotence).
        if let Some(prev) = out.last() {
            if !matches!(prev, ACFGNode::Sync(_)) && !matches!(child, ACFGNode::Sync(_)) {
                let w1 = writing_workers(prev);
                let w2 = reading_workers(&child);
                if !w1.is_empty() && !w2.is_empty() && w1 != w2 {
                    let participants: BTreeSet<WorkerId> = w1.union(&w2).copied().collect();
                    if participants.len() >= 2 {
                        out.push(ACFGNode::Sync(SyncPlaceholder { participants }));
                    }
                }
            }
        }

        out.push(child);
    }

    out
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
                seq.insert(0, ACFGNode::Sync(SyncPlaceholder { participants }));
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
