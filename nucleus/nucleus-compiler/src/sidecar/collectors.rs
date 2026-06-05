//! ACFG-walk + statement-walk collector helpers for [`build_sidecar`]
//! (`super::build_sidecar`).
//!
//! Split out of `sidecar.rs` (TASK-0343.01.01) to keep that file under
//! the 1000-LoC mega-file fence (`just check-mega-files`), the same
//! TASK-0383 precedent that carved out `cumulative_tests`. Each helper
//! is a pure fold over the linked algorithm IR or the post-pass ACFG
//! tree, populating one sidecar map; they are re-exported from `super`
//! (`pub(crate) use collectors::*;`) so `build_sidecar` and the
//! `cumulative_tests` child module reach them unchanged.

use std::collections::BTreeMap;

use crate::event::{DataId, IterVar, SeqTag};
use crate::sched::TransportMode;

use super::{LoopBound, SidecarError};
use crate::algo::CombineOp;

/// Build the per-accumulator-`DataId` → [`CombineOp`] map
/// (TASK-0343.01.01). Walks `algo.stmts` (descending into `For`
/// bodies) for the overlapping-write accumulator shape `acc[..] <--
/// k(acc[..], ...)` (LHS name appears among the RHS data references)
/// and, when the RHS is a top-level kernel [`IrExpr::Call`] whose
/// callee kernel declares a `combine = <op>` attribute, records
/// `name_data[acc] -> op`.
///
/// An accumulator whose owning kernel declares NO `combine` produces no
/// entry; the driver gate (`check_accumulator_consistency`) and the
/// render path treat an absent entry as a fail-loud soundness reject
/// (TASK-0343.01.01 AC#4) — there is no silent assume-sum fallback.
///
/// The RHS must be a DIRECT `Call` (the histogram/parity shape). A
/// non-`Call` RHS accumulator (e.g. a bare `acc[i] <-- acc[i]`) has no
/// owning kernel and so contributes no entry — and is likewise a
/// fail-loud reject downstream if it ever reaches the fan-in. Mirrors
/// `collect_accumulator_names` in
/// `backend_common::multi_worker_walker::collect` (same LHS-in-RHS
/// predicate, same canonical [`collect_dataref_names`] read-set walker)
/// so the two stay aligned by construction.
pub(super) fn collect_combine_for_accumulators(
    algo: &crate::algo::AlgoIR,
    name_data: &BTreeMap<String, DataId>,
) -> BTreeMap<DataId, CombineOp> {
    use crate::algo::{collect_dataref_names, IrExpr, IrStmt};
    use std::collections::BTreeSet;

    fn walk(
        stmts: &[crate::algo::IrStmt],
        kernels: &BTreeMap<String, crate::algo::ResolvedKernel>,
        name_data: &BTreeMap<String, DataId>,
        out: &mut BTreeMap<DataId, CombineOp>,
    ) {
        for s in stmts {
            match s {
                IrStmt::Dataflow { lhs, rhs } => {
                    let mut rhs_refs: BTreeSet<String> = BTreeSet::new();
                    collect_dataref_names(rhs, &mut rhs_refs);
                    if !rhs_refs.contains(&lhs.name) {
                        continue; // not an accumulator (no self-read)
                    }
                    // Owning kernel = the top-level RHS Call callee.
                    if let IrExpr::Call { callee, .. } = rhs {
                        if let Some(rk) = kernels.get(callee) {
                            if let (Some(op), Some(did)) =
                                (rk.combine, name_data.get(&lhs.name).copied())
                            {
                                out.insert(did, op);
                            }
                        }
                    }
                }
                IrStmt::For { body, .. } => walk(body, kernels, name_data, out),
                IrStmt::Effect { .. } => {}
            }
        }
    }

    let mut out = BTreeMap::new();
    walk(&algo.stmts, &algo.kernels, name_data, &mut out);
    out
}

/// Walk `stmts` (descending into `IrStmt::For` bodies, carrying the
/// stack of enclosing loop-variable names `enclosing_fors`) and insert
/// into `out` every data symbol that is a **cumulative cross-iteration**
/// array (TASK-0341.02.02.01.03, cycle 213).
///
/// A data symbol `D` is cumulative iff there is an
/// `IrStmt::Dataflow { lhs, rhs }` with `lhs.name == D` nested inside at
/// least one `for` loop, where the RHS contains a self-read
/// `IrExpr::DataRef { name: D, indices }` whose index expression along
/// SOME enclosing-loop axis DIFFERS from the LHS index at the same
/// dimension position. "Differs" is structural `IrExpr` inequality.
///
/// # Discriminator rationale (the histogram vs jacobi distinction)
///
/// - 16-jacobi: `field[t][y][x] <-- jacobi5_or_seed(field[(t+ITERS)%
///   (ITERS+1)][y-1][x], ...)`. The self-read at dim 0 is
///   `(t+ITERS)%(ITERS+1)`, structurally != the LHS dim-0 index `t`
///   ⇒ CUMULATIVE. (`t` is an enclosing-for var, so the index shift is
///   a cross-iteration read.)
/// - 08-histogram: `histogram[b] <-- bin_inc(histogram[b], input[i],
///   b)`. The self-read index `[b]` is IDENTICAL to the LHS index `[b]`
///   ⇒ NOT cumulative (a same-slice read-modify-write disjoint
///   accumulator — stays `wrapping_add` fan-in).
///
/// # Conservatism
///
/// The test requires (a) the dataflow is inside ≥1 `for` AND (b) a
/// self-read index that differs from the LHS at the same dim. A data
/// symbol read at the SAME index everywhere (pure read-modify) is not
/// classified. The differing index must reference an enclosing-for var
/// to be a genuine cross-iteration read, but the simpler structural
/// "index differs" test already separates every shipped schedule
/// correctly and is more robust to const-fold/grammar drift; the
/// enclosing-for guard (a) prevents a top-level non-iterated self-read
/// from being mistaken for cumulative.
pub(crate) fn collect_cumulative_data_names(
    stmts: &[crate::algo::IrStmt],
    enclosing_fors: &[String],
    out: &mut std::collections::BTreeSet<String>,
) {
    use crate::algo::{IndexedRef, IrExpr, IrStmt};

    // Recurse the RHS, collecting every self-read `DataRef` whose name
    // matches `lhs_name`, comparing each index vector against the LHS.
    fn rhs_self_read_differs(rhs: &IrExpr, lhs: &IndexedRef) -> bool {
        match rhs {
            IrExpr::DataRef(r) => {
                if r.name == lhs.name && r.indices != lhs.indices {
                    return true;
                }
                // Descend into index expressions defensively (a future
                // grammar may nest data reads in index position).
                r.indices.iter().any(|ix| rhs_self_read_differs(ix, lhs))
            }
            IrExpr::Call { args, .. } => args.iter().any(|a| rhs_self_read_differs(a, lhs)),
            // A comparison can appear in a (bool-typed) RHS; descend into
            // both operands so a self-read in either is still detected
            // (TASK-0341.02.01.02 / S2).
            IrExpr::BinOp(_, a, b) | IrExpr::Compare(_, a, b) => {
                rhs_self_read_differs(a, lhs) || rhs_self_read_differs(b, lhs)
            }
            IrExpr::Neg(inner) => rhs_self_read_differs(inner, lhs),
            IrExpr::IntLit(_) | IrExpr::Ident(_) => false,
        }
    }

    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                if !enclosing_fors.is_empty() && rhs_self_read_differs(rhs, lhs) {
                    out.insert(lhs.name.clone());
                }
            }
            IrStmt::For { var, body, .. } => {
                let mut nested: Vec<String> = enclosing_fors.to_vec();
                nested.push(var.clone());
                collect_cumulative_data_names(body, &nested, out);
            }
            IrStmt::Effect { .. } => {}
        }
    }
}

/// Walk an ACFG subtree, populating `out` with `(seq -> policy.buffer)`
/// from every `XferPlaceholder` encountered. Mirrors the existing
/// `acfg`-walk pattern (no allocation; in-place fold over the tree).
///
/// Push and Wait endpoints of the same pair share one seq + one
/// policy.buffer, so the second insertion under a given key is
/// idempotent. We accept the redundant write rather than branching
/// on role — simpler + no behavior difference.
pub(super) fn collect_transfer_buffers(node: &crate::acfg::ACFGNode, out: &mut BTreeMap<SeqTag, u64>) {
    use crate::acfg::ACFGNode;
    match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Xfer(x) => {
            out.insert(x.seq, x.policy.buffer);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_transfer_buffers(c, out);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            collect_transfer_buffers(body, out);
        }
    }
}

/// Walk an ACFG subtree, populating `out` with `(seq -> policy.transport)`
/// from every `XferPlaceholder` encountered (TASK-0438.02). Exact mirror
/// of [`collect_transfer_buffers`] — same traversal, same idempotent
/// double-write across the Push/Wait pair (both endpoints share one seq
/// and one `policy.transport`, so the second insertion under a given key
/// is a no-op). We accept the redundant write rather than branching on
/// role — simpler + no behavior difference.
pub(super) fn collect_transfer_transports(
    node: &crate::acfg::ACFGNode,
    out: &mut BTreeMap<SeqTag, TransportMode>,
) {
    use crate::acfg::ACFGNode;
    match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Xfer(x) => {
            out.insert(x.seq, x.policy.transport);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_transfer_transports(c, out);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            collect_transfer_transports(body, out);
        }
    }
}

/// Recursively collect each `for` loop's unevaluated `(lo, hi)`,
/// keyed by the [`IterVar`] `build_acfg` assigned to its variable
/// name. Mirrors `acfg::collect_iter_var_names` (same traversal) so
/// the keys line up one-for-one with `acfg.name_iter_vars`.
///
/// If two loops in the same program share a variable name they share
/// one [`IterVar`] (per `ACFG::name_iter_vars` — loop vars are one
/// namespace, PRD §6.2.3) and therefore one `loop_bounds` entry. We
/// keep the FIRST occurrence in source order; a later same-named loop
/// with *identical* bounds is idempotent (no-op). A later same-named
/// loop with *different* bounds is an ambiguity the shared `IterVar`
/// (and `Event::Loop.iter_var`) cannot represent — TASK-0170 proved
/// this is reachable from a VALID Nuc program (two sequential sibling
/// loops `for i : 0..N`, `for i : 0..M`, writing distinct data so
/// single-assignment holds; lowering only rejects shadowing a
/// *declared* const/data/kernel). Returning a typed
/// [`SidecarError::SameNameLoopBoundConflict`] (fail fast AND
/// verbose: the loop var + both bound exprs) is the honest behaviour
/// — never a `panic!` on valid input. AC#3 (TASK-0170): the
/// EventList-only backend (TASK-0124) consuming this can therefore
/// only ever see a clean driver-surfaced error here, never a process
/// abort. Distinct-identity support so such programs *compile* is the
/// deeper redesign tracked as TASK-0171 (depends on TASK-0170).
///
/// (The `name_iter_vars` lookup below is still an `expect`: every
/// loop var was enumerated *into* `name_iter_vars` by `build_acfg`
/// from these same statements, so a miss is a compiler-internal
/// invariant break, not user input — unreachable for link-valid IR,
/// like the other name<->id desync guards in this file.)
pub(super) fn collect_loop_bounds(
    stmts: &[crate::algo::IrStmt],
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeMap<IterVar, LoopBound>,
) -> Result<(), SidecarError> {
    use crate::algo::IrStmt;
    for s in stmts {
        // `until` (epic S1, TASK-0341.02.01.03) is intentionally ignored:
        // the sidecar loop-bound is the static CAP `lo..hi`. The driver
        // runs run_pre_mediation_passes (build_acfg first) BEFORE
        // build_sidecar, so an `until`-loop is already rejected by the time
        // this runs — this match never observes a `Some(until)` in practice.
        if let IrStmt::For {
            var,
            lo,
            hi,
            until: _,
            body,
        } = s
        {
            let iv = *name_iter_vars.get(var).unwrap_or_else(|| {
                panic!(
                    "sidecar: loop var `{var}` has no IterVar in \
                     acfg.name_iter_vars — name<->id table desync"
                )
            });
            let bound = LoopBound {
                lo: lo.clone(),
                hi: hi.clone(),
            };
            match out.get(&iv) {
                None => {
                    out.insert(iv, bound);
                }
                Some(existing) if *existing != bound => {
                    return Err(SidecarError::SameNameLoopBoundConflict {
                        var: var.clone(),
                        first: existing.clone(),
                        second: bound,
                    });
                }
                Some(_) => { /* same name, same bounds: idempotent */ }
            }
            collect_loop_bounds(body, name_iter_vars, out)?;
        }
    }
    Ok(())
}
