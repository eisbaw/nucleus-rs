//! Halo region inference from kernel access patterns — TASK-0260 Stage 1.
//!
//! For each kernel invocation inside a `for` nest, scan its argument
//! [`IrExpr::DataRef`] indices to recover the per-axis halo width N: the
//! maximum `|b|` across all `iter_var + b` index reads, where `iter_var`
//! is one of the enclosing loop variables and `b` is a constant integer
//! offset.
//!
//! Example: `blur3(grid[y-1, x], grid[y, x], grid[y+1, x])` inside
//! `for y { for x { ... } }` reads at y-offsets `{-1, 0, +1}`, so
//! `halo_widths[blur3_id][y_iv] = 1`. The x-axis indices are all `x`
//! (offset 0), so `halo_widths[blur3_id][x_iv] = 0` (the contract
//! permits either an explicit 0-width entry or omission; the
//! implementation chose the explicit form — see `no_halo_bare_iv`).
//!
//! ## Why this pass exists
//!
//! PRD §6.3.3 + §9 (rows 5, 6): a distributed schedule on a stencil-like
//! kernel produces WRONG output at partition boundaries unless each
//! worker reads (or has shipped to it) a halo strip of the neighbour's
//! tile. The schedule does NOT state halo size — the compiler infers it
//! from the algorithm's kernel access pattern. This pass is the
//! INFERENCE step; downstream wiring (transfer_inject extending per-tile
//! transfer ranges by the halo width, and per-tile boundary kernel
//! semantics) is filed as a follow-up (TASK-0263).
//!
//! ## Stage decomposition (TASK-0260 family)
//!
//! - **Stage 1** (this pass): walk AlgoIR, infer halo widths per
//!   `(KernelId, IterVar)`, persist into the sidecar. Pure +
//!   deterministic. No backend behaviour change.
//! - **Stage 2** (TASK-0263): `transfer_inject` reads `halo_widths` from
//!   the sidecar and extends per-tile transfer ranges by the halo width
//!   on each side of each axis. This is the cycle that unblocks the
//!   bit-identical stencil-distributed e2e cell (closing AC#5/AC#6 of
//!   TASK-0258/0259 + AC#2/AC#4 of TASK-0043).
//! - **Stage 3** (TASK-0264): block-pair metadata recovery for
//!   halo-strip Push/Wait synthesis under `partition=blocks2d`. Addresses
//!   the TASK-0259 architect forward-carry directly (per-worker
//!   (row, col) inverse + grid shape).
//!
//! ## Affine-stride detector
//!
//! An index expression `e` is considered halo-relevant if and only if it
//! matches **one** of these patterns:
//!
//! 1. A pure constant (no enclosing iter-var Ident anywhere in `e`) —
//!    contributes 0 to every iter-var's halo. We simply skip it.
//! 2. `iv + b` (or `iv - b`, or `b + iv`) where `iv` is the textual name
//!    of an enclosing iter-var and `b` const-folds to an integer.
//!    Contributes `|b|` to `(kernel, iv)`'s halo.
//! 3. `iv` by itself (just the iter-var Ident). Equivalent to `iv + 0`;
//!    contributes 0. The implementation DOES write an explicit
//!    `halo_widths[K][iv] = 0` entry rather than omitting it (so a
//!    Stage-2 consumer can distinguish "inspected, no halo needed"
//!    from "kernel never inspected this axis"); the contract permits
//!    either form — consumers MUST treat absence and 0 identically.
//! 4. `-iv` (the negation of an iter-var Ident). `iv * -1 + 0` is
//!    coefficient `-1`, which DOES qualify as `a == 1` in magnitude — but
//!    reverses iteration order. For the FIRST CUT we reject this with
//!    [`HaloInferenceError::StridedAccessNotSupported`] because the
//!    iteration-order reversal is a strictly larger semantic question than
//!    halo (a flipped read pattern interacts with the partition pass'
//!    band assignment). Filed as a known limitation; documented below.
//!
//! Any other shape is REJECTED with a typed error. PRD §13 explicitly
//! lists "reuse / halo on data-dependent strides" as rejected at compile
//! time; this pass is the enforcement site for that rule on the halo
//! side. The mirror constraint applies equally to TASK-0261 (reuse
//! codegen) — both share the same affine-only prerequisite (forward-carry
//! lesson recorded against TASK-0261).
//!
//! ## Strict vs advisory entry points
//!
//! Two entry points exist:
//!
//! - [`apply_halo_inference`] is the **strict** variant: returns
//!   `Err(HaloInferenceError)` on the first non-affine / strided /
//!   data-dependent index it sees. Used by tests + direct callers
//!   that want fail-fast semantics. This is what the negative-test
//!   gate exercises.
//! - [`apply_halo_inference_advisory`] is the **lenient** variant:
//!   walks the algorithm to completion, records every affine fact it
//!   CAN classify, returns the populated ACFG + the vector of typed
//!   errors that the strict variant would have raised. The driver
//!   consumes this in Stage 1.
//!
//! Why the driver is lenient in Stage 1: no downstream pass yet reads
//! `halo_widths`, so a rejection here is purely advisory until Stage 2
//! (TASK-0263, transfer_inject extension) makes the consumer concrete.
//! Real-world reachable case: example 11 (`11-game-of-life`) reads
//! `grid[(t + ITERS) % (ITERS + 1)]` — a compile-time-constant `Mod`
//! wrap that the affine detector cannot fold (Mod is rejected by the
//! detector). The strict variant would reject; the lenient variant
//! records nothing and lets compilation proceed (the schedule does not
//! partition, so no halo is needed). When Stage 2 lands, the driver
//! will switch to the strict variant (or check the errors vec against
//! the partition policy before swallowing them).
//!
//! ## Honest limitations (first cut)
//!
//! - **Coefficient must be +1.** `2*iv + 1`, `iv * 2`, and `-iv` are
//!   rejected as [`StridedAccessNotSupported`]. Strided reads have
//!   well-defined halo semantics (`|b|` is still the offset, but the
//!   distributed transfer pattern differs from the contiguous-strip case
//!   stencils need) — out of scope for Stage 1.
//! - **Single iter-var per index.** `grid[y + x][x]` (or any index whose
//!   tree contains TWO different enclosing iter-var Idents) is rejected
//!   as [`MultipleIterVarsInIndex`] because the offset `b` would not be a
//!   compile-time constant.
//! - **No DataRef inside an index.** `grid[lookup[y]]` (an index that
//!   itself reads data) is rejected as [`DataDependentStride`]. PRD §13.
//! - **No Call inside an index.** `grid[f(y)]` is rejected as
//!   [`DataDependentStride`] (a runtime call cannot be folded; same harm
//!   class as a DataRef).
//! - **Kernel call must sit inside a `for` nest.** A kernel call at
//!   top-level scope (no enclosing loop) has no IterVar to key halo
//!   against — no entries are recorded for such calls, and that is
//!   correct (a halo on a non-iterated kernel is meaningless).
//! - **Pure-constant indices.** A kernel arg `grid[3]` (no iter-var in
//!   sight) is acceptable and contributes nothing. The kernel reads a
//!   fixed location; no halo is needed.
//! - **Same kernel called twice.** If the algorithm calls the same
//!   kernel in two different `for` nests, both call sites contribute to
//!   the SAME `(KernelId, IterVar)` halo entry (union via max). This is
//!   correct: the codegen-emit pass renders one kernel definition per
//!   KernelId, so a halo that covers all call sites is the only sound
//!   choice.
//!
//! ## Sidecar contract
//!
//! Writes [`crate::sidecar::NameSidecar::halo_widths`]:
//! `BTreeMap<KernelId, BTreeMap<IterVar, u64>>`. The field is `#[serde(default)]`
//! so an old wire payload (no field) deserialises as an empty map (same
//! contract precedent as `transfer_buffer_for_seq` from TASK-0233 and
//! `partition_worker_ranges` from TASK-0212).
//!
//! ## Independence from partition policy
//!
//! `halo_widths` is keyed by `KernelId -> IterVar -> u64` (nested
//! `BTreeMap`) and is **independent of the partition policy**. Halo
//! widths apply whether the loop is partitioned or not; partitioning
//! just MAKES the halo crossings visible. The Stage 3 follow-up
//! (TASK-0264) couples halo to partition shape — that is where
//! block-pair recovery + worker neighbour resolution lands.
//!
//! ## Why post-ACFG pass (Option A) and not link extension (Option B)
//!
//! Option A (this pass) mirrors `partition_rows` / `partition_blocks2d`:
//! reads `linked.algo.stmts` for the source IrExpr trees and
//! `acfg.name_iter_vars` / `acfg.name_kernels` for the join keys.
//! Option B (link-step extension) would need to duplicate iter-var name
//! collection and offer no benefit since:
//!
//! - The consumer (`transfer_inject`) runs BEFORE the partition passes
//!   today but does NOT YET read halo (Stage 2 wires that). For Stage 1
//!   the sidecar carries the result irrespective of pipeline order.
//! - The pass + sidecar surface is the same shape as the partition
//!   sibling passes — a future reader of the code follows one idiom, not
//!   two.
//!
//! ## Determinism
//!
//! - Nested `BTreeMap<KernelId, BTreeMap<IterVar, u64>>`, both `u64`
//!   newtypes; iteration is in numeric order.
//! - Iteration over `linked.algo.stmts` is the source-order vector walk.
//! - No `HashMap`/`HashSet` on any path that affects emitted bytes.
//! - Same input ⇒ byte-identical sidecar.

use std::collections::BTreeMap;

use crate::acfg::ACFG;
use crate::algo::{IndexedRef, IrBinOp, IrExpr, IrStmt, ResolvedConst};
use crate::event::{IterVar, KernelId};
use crate::link::LinkedIR;

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Errors produced by [`apply_halo_inference`].
///
/// Every variant names a single semantic violation and carries the
/// payload a diagnostic message needs. Mirrors the shape of
/// [`crate::passes::partition_rows::PartitionRowsError`] /
/// [`crate::passes::partition_blocks2d::PartitionBlocks2dError`] —
/// typed, no `panic!` on user-reachable shapes, per PRD §10.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HaloInferenceError {
    /// A kernel call's argument indexes data by a non-affine expression
    /// (e.g. `grid[lookup[iv]]`, `grid[iv * iv]`, or `grid[f(iv)]`).
    /// PRD §13 rejects data-dependent strides at compile time; this is
    /// the enforcement site for the halo side.
    DataDependentStride {
        /// Kernel-callee name at the offending call site.
        kernel: String,
        /// Data symbol name being indexed (the `name` of the offending
        /// [`IndexedRef`]).
        ref_name: String,
        /// 0-based axis index inside the [`IndexedRef`] (the position of
        /// the offending index expression in `indices`).
        ax_idx: usize,
    },
    /// A kernel call's argument indexes data by a stride > 1 in
    /// magnitude (e.g. `grid[2*iv + 1]`, `grid[iv * 3]`, or `-iv` which
    /// has magnitude-1 coefficient but reverses iteration order — see
    /// module docs for the iteration-order rationale). Halo on strided
    /// reads has well-defined semantics but is out-of-scope for Stage 1.
    StridedAccessNotSupported {
        /// Kernel-callee name at the offending call site.
        kernel: String,
        /// Data symbol name being indexed.
        ref_name: String,
        /// 0-based axis index inside the [`IndexedRef`].
        ax_idx: usize,
        /// The recovered stride coefficient (e.g. `2` for `2*iv`, `-1`
        /// for `-iv`). `0` if a stride could not be recovered (the
        /// index was non-linear in the iter-var entirely).
        coefficient: i64,
    },
    /// A kernel call's argument indexes data by an expression that
    /// references TWO OR MORE enclosing iter-vars (e.g. `grid[y + x]`).
    /// Halo on such indices is not a single-axis property; one of the
    /// "iter-vars" would have to play the role of `b` and `b` must be a
    /// compile-time constant.
    MultipleIterVarsInIndex {
        /// Kernel-callee name at the offending call site.
        kernel: String,
        /// Data symbol name being indexed.
        ref_name: String,
        /// 0-based axis index inside the [`IndexedRef`].
        ax_idx: usize,
        /// The iter-var names found in the index, deterministic order.
        iter_vars: Vec<String>,
    },
    /// A kernel call's argument indexes data by an expression that
    /// references an iter-var by name but is otherwise not affine
    /// (e.g. an unrecognised operator combination). Distinct from
    /// [`Self::DataDependentStride`] because no DataRef / Call is involved
    /// — pure arithmetic that we cannot classify. Kept as a separate
    /// variant so the diagnostic is precise.
    NonAffineIndex {
        /// Kernel-callee name at the offending call site.
        kernel: String,
        /// Data symbol name being indexed.
        ref_name: String,
        /// 0-based axis index inside the [`IndexedRef`].
        ax_idx: usize,
    },
    /// A kernel call references a kernel name that the linker did not
    /// resolve. Cannot happen for link-valid IR (the link step rejects
    /// `UnknownIdent` upstream). The variant exists so the pass fails
    /// closed on an inconsistently-constructed `(LinkedIR, ACFG)` pair
    /// rather than panicking — same invariant guard
    /// [`crate::passes::partition_rows::PartitionRowsError::UnknownLoopVar`]
    /// carries.
    UnknownKernelInCall { callee: String },
    /// An iter-var name was collected from the lexical scope during the
    /// kernel-arg walk but is missing from `ACFG::name_iter_vars`. Cannot
    /// happen for link-valid IR (the link step inserts every `for var`
    /// into `name_iter_vars`); the variant exists so the pass fails
    /// closed on an inconsistently-constructed `(LinkedIR, ACFG)` pair
    /// rather than panicking — architect-review F-P1 of cycle 81.
    /// Same invariant-guard pattern as [`Self::UnknownKernelInCall`].
    UnknownIterVarInScope { iter_var: String },
}

impl std::fmt::Display for HaloInferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HaloInferenceError::DataDependentStride {
                kernel,
                ref_name,
                ax_idx,
            } => write!(
                f,
                "kernel call `{kernel}` reads `{ref_name}` with a data-dependent index at axis \
                 {ax_idx} (a DataRef or Call inside the index expression). PRD §13: halo \
                 inference rejects data-dependent strides at compile time. Replace the index \
                 with an affine expression `iv + b` (b constant) or remove the call from a \
                 distributed schedule."
            ),
            HaloInferenceError::StridedAccessNotSupported {
                kernel,
                ref_name,
                ax_idx,
                coefficient,
            } => write!(
                f,
                "kernel call `{kernel}` reads `{ref_name}` with a strided index at axis {ax_idx} \
                 (recovered coefficient {coefficient}). TASK-0260 first cut accepts only \
                 coefficient +1 (i.e. `iv + b`); strided / reversed reads (`a*iv`, `-iv`) are \
                 out-of-scope. Filed as a known limitation."
            ),
            HaloInferenceError::MultipleIterVarsInIndex {
                kernel,
                ref_name,
                ax_idx,
                iter_vars,
            } => write!(
                f,
                "kernel call `{kernel}` reads `{ref_name}` at axis {ax_idx} with an index that \
                 references multiple enclosing iter-vars ({}); halo inference needs the offset \
                 `b` in `iv + b` to be a compile-time constant.",
                iter_vars.join(", ")
            ),
            HaloInferenceError::NonAffineIndex {
                kernel,
                ref_name,
                ax_idx,
            } => write!(
                f,
                "kernel call `{kernel}` reads `{ref_name}` at axis {ax_idx} with a non-affine \
                 index expression. Halo inference accepts only `iv + b` (b constant) — see PRD \
                 §13 / TASK-0260 module docs."
            ),
            HaloInferenceError::UnknownKernelInCall { callee } => write!(
                f,
                "halo inference: kernel call references `{callee}` but the ACFG has no such \
                 kernel id (link-pass invariant violation)"
            ),
            HaloInferenceError::UnknownIterVarInScope { iter_var } => write!(
                f,
                "halo inference: iter-var `{iter_var}` was collected from lexical scope but is \
                 missing from `ACFG::name_iter_vars` (link-pass invariant violation; the link \
                 step is contracted to insert every `for var` into the name table)"
            ),
        }
    }
}

impl std::error::Error for HaloInferenceError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Walk `linked.algo.stmts` and populate
/// [`crate::sidecar::NameSidecar::halo_widths`]-bound facts on the
/// ACFG via a transient sidecar mirror on the ACFG itself.
///
/// NOTE: the sidecar field actually lives on [`crate::sidecar::NameSidecar`]
/// and is populated by [`crate::sidecar::build_sidecar`] from a field
/// added to the ACFG by this pass — same protocol as
/// `partition_worker_ranges` (the ACFG carries the canonical map; the
/// `NameSidecar` clones it through). See [`crate::acfg::ACFG::halo_widths`]
/// for the on-ACFG home.
///
/// Pure: input ACFG is consumed, a new one with the sidecar populated
/// is returned. The tree itself is forwarded unchanged.
///
/// On any error, no partial sidecar is committed — the function
/// validates every kernel call up front before mutating the sidecar.
pub fn apply_halo_inference(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, HaloInferenceError> {
    let (halo, errors) = infer_halo_widths(linked, &acfg);
    if let Some(e) = errors.into_iter().next() {
        return Err(e);
    }
    Ok(commit_halo_widths(acfg, halo))
}

/// Lenient variant of [`apply_halo_inference`]: walks the algorithm,
/// records every halo-relevant `iv + b` pattern it CAN classify, and
/// returns every typed error it would have raised so the caller decides
/// whether each shape is fatal.
///
/// ## Stage 1 driver policy (TASK-0260)
///
/// The driver consumes the lenient variant in Stage 1: no downstream
/// pass yet reads `halo_widths`, so a non-affine index is only advisory
/// until Stage 2 (TASK-0263, transfer_inject extension) makes the
/// consumer concrete. The driver emits a `nuc_trace!` line per
/// returned error (visible under `NUC_TRACE=1`); the e2e baseline
/// stays byte-identical because no cell's emitted bytes change.
///
/// Real-world reachable case the lenient variant unblocks: example 11
/// (`11-game-of-life`) reads
/// `grid[(t + ITERS) % (ITERS + 1)][(i + N - 1) % N]` — a constant
/// modulo wrap, NOT runtime-data-dependent. The first-cut affine
/// detector cannot fold `Mod` and the strict variant would reject;
/// the lenient variant records nothing for that (kernel, iter-var)
/// pair and lets compilation proceed (no halo synthesis needed; the
/// schedule does not partition).
///
/// Stage 2 will either (a) move the driver back to the strict variant
/// once the consumer wiring is in place + every shipped example is
/// affine-only, or (b) check the errors vec against the partition
/// policy and only treat them as fatal when a partition asks for halo
/// on the rejected axis.
pub fn apply_halo_inference_advisory(
    linked: &LinkedIR,
    acfg: ACFG,
) -> (ACFG, Vec<HaloInferenceError>) {
    let (halo, errors) = infer_halo_widths(linked, &acfg);
    let acfg = commit_halo_widths(acfg, halo);
    (acfg, errors)
}

/// Core inference: walk `linked.algo.stmts` and return both the
/// populated halo map AND the typed errors raised along the way. The
/// walker is COLLECTING (does not short-circuit on the first error)
/// so the lenient variant can record every recognisable fact even
/// when some indices are non-affine. The strict variant short-
/// circuits on the first error at the entry point.
fn infer_halo_widths(
    linked: &LinkedIR,
    acfg: &ACFG,
) -> (
    BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    Vec<HaloInferenceError>,
) {
    let ctx = WalkCtx {
        name_kernels: &acfg.name_kernels,
        name_iter_vars: &acfg.name_iter_vars,
        consts: &linked.algo.consts,
    };
    let mut halo: BTreeMap<KernelId, BTreeMap<IterVar, u64>> = BTreeMap::new();
    let mut errors: Vec<HaloInferenceError> = Vec::new();
    let scope: Vec<String> = Vec::new();
    collect_from_stmts(&linked.algo.stmts, &scope, &ctx, &mut halo, &mut errors);
    (halo, errors)
}

/// Destructure-and-rebuild commit. Pre-validation in the caller means
/// no partial-commit hazard; the partition passes share the same
/// commit shape.
fn commit_halo_widths(acfg: ACFG, halo: BTreeMap<KernelId, BTreeMap<IterVar, u64>>) -> ACFG {
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths: _existing,
    } = acfg;

    ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths: halo,
    }
}

// --------------------------------------------------------------------
// Statement walker
// --------------------------------------------------------------------

/// Read-only joining context threaded through the recursive walker.
///
/// Bundles the immutable tables every recursion frame needs (kernel
/// name→id, iter-var name→id, const folding table) into one borrow.
/// This keeps the walker function signatures short (3 args + the
/// mutable output map) — clippy's `too_many_arguments` cap on 7 would
/// fire otherwise, and the bundle makes the intent ("here is the
/// join + folding context I read") explicit at every callsite.
struct WalkCtx<'a> {
    name_kernels: &'a BTreeMap<String, KernelId>,
    name_iter_vars: &'a BTreeMap<String, IterVar>,
    consts: &'a BTreeMap<String, ResolvedConst>,
}

/// Walk a flat list of [`IrStmt`]s with `scope` naming the enclosing
/// iter-vars (outermost-first). Each call shadows / extends `scope`
/// via a clone-and-push (the IR is small; the algorithm is one walk).
fn collect_from_stmts(
    stmts: &[IrStmt],
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloInferenceError>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs: _, rhs } => {
                // RHS may be a Call (one or more kernel invocations
                // possibly nested), an IntLit/Ident/Neg/BinOp expression,
                // or a bare DataRef (identity copy). Halo only applies
                // to kernel calls — walk RHS looking for them.
                visit_expr_for_calls(rhs, scope, ctx, out, errors);
            }
            IrStmt::Effect { callee, args } => {
                process_call(callee, args, scope, ctx, out, errors);
            }
            IrStmt::For {
                var,
                lo: _,
                hi: _,
                body,
            } => {
                // Push the loop var onto the scope before recursing into
                // the body. A loop variable might shadow a const name in
                // principle (PRD §6.2.3 lets loop vars use one namespace
                // and shadow at their loop), but the lowering pass already
                // rejects shadowing of declared const/data/kernel — we do
                // NOT need to model that ambiguity here.
                let mut next_scope = scope.to_vec();
                next_scope.push(var.clone());
                collect_from_stmts(body, &next_scope, ctx, out, errors);
            }
        }
    }
}

/// Recursively visit an expression looking for kernel calls. When a
/// `Call` is hit, dispatch to [`process_call`]; otherwise recurse into
/// sub-expressions. DataRefs inside a top-level RHS expression are NOT
/// the halo-relevant case (the halo question is about kernel args, not
/// about top-level reads), but a nested call's args are.
fn visit_expr_for_calls(
    e: &IrExpr,
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloInferenceError>,
) {
    match e {
        IrExpr::Call { callee, args } => process_call(callee, args, scope, ctx, out, errors),
        IrExpr::Neg(inner) => visit_expr_for_calls(inner, scope, ctx, out, errors),
        IrExpr::BinOp(_, lhs, rhs) => {
            visit_expr_for_calls(lhs, scope, ctx, out, errors);
            visit_expr_for_calls(rhs, scope, ctx, out, errors);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) | IrExpr::DataRef(_) => {}
    }
}

/// Inspect a kernel call's arguments for halo-relevant `DataRef`
/// patterns and update `out`. Recursively visits nested calls.
fn process_call(
    callee: &str,
    args: &[IrExpr],
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloInferenceError>,
) {
    // Some "calls" are effect-statement load/capture kernels that the
    // ACFG name_kernels DOES include (every algorithm kernel is in the
    // name table). A miss is a link-invariant break — push the error
    // and skip this call's args (the kid is the join key; without it
    // the args have nowhere to land).
    let kid = match ctx.name_kernels.get(callee) {
        Some(k) => *k,
        None => {
            errors.push(HaloInferenceError::UnknownKernelInCall {
                callee: callee.to_string(),
            });
            return;
        }
    };

    for arg in args {
        // Walk the arg tree. If we hit a DataRef, classify each of its
        // indices; if we hit a nested Call, recurse into IT to handle its
        // args (the nested call has its own kernel id + its own halo
        // contribution).
        visit_arg(arg, kid, callee, scope, ctx, out, errors);
    }
}

/// Recursively walk a kernel arg expression. DataRefs become halo
/// classifications; nested Calls recurse via [`process_call`]; other
/// sub-expressions are walked through (a `BinOp` could in principle wrap
/// a DataRef e.g. `k(grid[y] + 1)` — we handle that by descending).
fn visit_arg(
    e: &IrExpr,
    kid: KernelId,
    callee: &str,
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloInferenceError>,
) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            for (ax_idx, idx_expr) in indices.iter().enumerate() {
                let site = IndexSite {
                    kid,
                    callee,
                    ref_name: name,
                    ax_idx,
                };
                classify_index(idx_expr, &site, scope, ctx, out, errors);
            }
        }
        IrExpr::Call { callee: c2, args } => process_call(c2, args, scope, ctx, out, errors),
        IrExpr::Neg(inner) => visit_arg(inner, kid, callee, scope, ctx, out, errors),
        IrExpr::BinOp(_, lhs, rhs) => {
            visit_arg(lhs, kid, callee, scope, ctx, out, errors);
            visit_arg(rhs, kid, callee, scope, ctx, out, errors);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}

// --------------------------------------------------------------------
// Affine-stride detector
// --------------------------------------------------------------------

/// Diagnostic site context for one DataRef index inspection. Bundles
/// the (kernel-id, kernel-name, data-symbol, axis) tuple so each
/// `HaloInferenceError` constructor has the payload it needs in one
/// borrow. Keeps `classify_index` under the clippy `too_many_arguments`
/// cap (7).
struct IndexSite<'a> {
    kid: KernelId,
    callee: &'a str,
    ref_name: &'a str,
    ax_idx: usize,
}

/// Classify one index expression against the [`IndexSite`] it sits in.
///
/// The four legal shapes (see module docs):
/// - Pure constant → no entry written.
/// - `iv` alone → no entry written (halo 0).
/// - `iv + b` / `iv - b` / `b + iv` (b folds to integer) → write `|b|`.
/// - Everything else → typed error.
fn classify_index(
    e: &IrExpr,
    site: &IndexSite<'_>,
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloInferenceError>,
) {
    // Reject early: DataRef or Call inside the index = data-dependent.
    if expr_contains_dataref_or_call(e) {
        errors.push(HaloInferenceError::DataDependentStride {
            kernel: site.callee.to_string(),
            ref_name: site.ref_name.to_string(),
            ax_idx: site.ax_idx,
        });
        return;
    }

    // Collect every enclosing-iter-var Ident the expression mentions.
    // If none → pure constant, no halo contribution.
    // If two or more distinct iter-vars → reject as MultipleIterVarsInIndex.
    let mut ivs_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    collect_iter_var_refs(e, scope, &mut ivs_used);

    if ivs_used.is_empty() {
        // Pure-constant index. Fold it to verify it's a valid integer
        // (a non-foldable purely-const-name index would surface as
        // NonAffineIndex — but with no iter-vars in scope it's "halo 0
        // for nothing", so we accept silently).
        return;
    }

    if ivs_used.len() > 1 {
        errors.push(HaloInferenceError::MultipleIterVarsInIndex {
            kernel: site.callee.to_string(),
            ref_name: site.ref_name.to_string(),
            ax_idx: site.ax_idx,
            iter_vars: ivs_used.into_iter().collect(),
        });
        return;
    }

    let iv_name = ivs_used.into_iter().next().expect("len == 1 checked");

    // Recover (coefficient, offset) from the expression. The detector
    // accepts only coefficient +1; -1 and |c| > 1 are rejected.
    let (coeff, offset) = match affine_decompose(e, &iv_name, ctx.consts) {
        Some(pair) => pair,
        None => {
            errors.push(HaloInferenceError::NonAffineIndex {
                kernel: site.callee.to_string(),
                ref_name: site.ref_name.to_string(),
                ax_idx: site.ax_idx,
            });
            return;
        }
    };

    if coeff != 1 {
        errors.push(HaloInferenceError::StridedAccessNotSupported {
            kernel: site.callee.to_string(),
            ref_name: site.ref_name.to_string(),
            ax_idx: site.ax_idx,
            coefficient: coeff,
        });
        return;
    }

    let iv = match ctx.name_iter_vars.get(&iv_name) {
        Some(iv) => *iv,
        None => {
            // Defensive — collect_iter_var_refs only emits names that
            // were pushed onto `scope`, and scope is grown from
            // `IrStmt::For { var, ... }`s the link step inserts into
            // name_iter_vars. The variant exists so an inconsistently-
            // constructed `(LinkedIR, ACFG)` pair fails closed with a
            // typed error rather than panicking (cycle-81 architect
            // review F-P1).
            errors.push(HaloInferenceError::UnknownIterVarInScope {
                iter_var: iv_name,
            });
            return;
        }
    };
    let width = offset.unsigned_abs();
    let per_iv = out.entry(site.kid).or_default();
    let entry = per_iv.entry(iv).or_insert(0);
    if width > *entry {
        *entry = width;
    }
}

/// Walk an expression and union all Ident-leaf names that are present in
/// the enclosing iter-var `scope`. Idents that refer to a const or
/// anything else are ignored at this step (they are treated as part of
/// the "constant" contribution and validated later by [`affine_decompose`]
/// when it tries to const-fold).
fn collect_iter_var_refs(
    e: &IrExpr,
    scope: &[String],
    out: &mut std::collections::BTreeSet<String>,
) {
    match e {
        IrExpr::Ident(name) => {
            if scope.iter().any(|s| s == name) {
                out.insert(name.clone());
            }
        }
        IrExpr::IntLit(_) => {}
        IrExpr::Neg(inner) => collect_iter_var_refs(inner, scope, out),
        IrExpr::BinOp(_, lhs, rhs) => {
            collect_iter_var_refs(lhs, scope, out);
            collect_iter_var_refs(rhs, scope, out);
        }
        // DataRef / Call inside the index would have already been rejected
        // upstream (classify_index calls expr_contains_dataref_or_call
        // before us); the no-op here is reached only on the
        // already-rejected path during unit tests of this helper.
        IrExpr::DataRef(_) | IrExpr::Call { .. } => {}
    }
}

/// Does this expression contain ANY `DataRef` or `Call` node anywhere
/// in its subtree? Used by [`classify_index`] to short-circuit on
/// data-dependent indices before the affine decomposer runs.
fn expr_contains_dataref_or_call(e: &IrExpr) -> bool {
    match e {
        IrExpr::DataRef(_) | IrExpr::Call { .. } => true,
        IrExpr::IntLit(_) | IrExpr::Ident(_) => false,
        IrExpr::Neg(inner) => expr_contains_dataref_or_call(inner),
        IrExpr::BinOp(_, lhs, rhs) => {
            expr_contains_dataref_or_call(lhs) || expr_contains_dataref_or_call(rhs)
        }
    }
}

/// Try to decompose `e` as `coefficient * iv + offset` where `iv` is the
/// given iter-var name and both `coefficient` and `offset` const-fold to
/// integers. Returns `Some((coeff, offset))` on success; `None` if the
/// expression is not affine in `iv` in the recognised shape.
///
/// The shapes accepted are:
/// - `iv` → `(1, 0)`
/// - `-iv` → `(-1, 0)`
/// - `iv + c` / `c + iv` → `(1, c)`
/// - `iv - c` → `(1, -c)`
/// - `c - iv` → `(-1, c)`
/// - `c * iv` / `iv * c` → `(c, 0)`
/// - And one level of composition: `c * iv + d`, `c * iv - d`,
///   `d + c * iv`, `d - c * iv` → `(c [or -c], d [or -d])`.
///
/// Any deeper composition (`(a + b) * iv`, `iv + iv`, etc.) returns
/// `None`. The detector is intentionally conservative — the caller
/// raises a typed error on `None` rather than guessing.
///
/// `consts` lets a bound like `OFFSET - 1` fold when `const OFFSET = 1`.
fn affine_decompose(
    e: &IrExpr,
    iv: &str,
    consts: &BTreeMap<String, ResolvedConst>,
) -> Option<(i64, i64)> {
    // Base cases.
    match e {
        IrExpr::Ident(name) if name == iv => return Some((1, 0)),
        IrExpr::Neg(inner) => {
            // `-iv` → (-1, 0). `-(iv + c)` → (-1, -c). `-(c)` is a
            // constant and is handled in `eval_const_int` below.
            if let Some((c, d)) = affine_decompose(inner, iv, consts) {
                return Some((-c, -d));
            }
            // Pure-constant `-c` reaches here only if `inner` is
            // iv-independent (no Ident == iv). Try const-folding the
            // whole expression as a constant offset.
            if let Some(k) = eval_const_int(e, consts) {
                return Some((0, k));
            }
            return None;
        }
        _ => {}
    }

    // Const-foldable constants → (0, k). Catches IntLit, Ident-of-const,
    // and any composition that doesn't mention iv.
    if !expr_mentions(e, iv) {
        return eval_const_int(e, consts).map(|k| (0, k));
    }

    // BinOp cases.
    if let IrExpr::BinOp(op, lhs, rhs) = e {
        let lhs_aff = if expr_mentions(lhs, iv) {
            affine_decompose(lhs, iv, consts)
        } else {
            eval_const_int(lhs, consts).map(|k| (0_i64, k))
        };
        let rhs_aff = if expr_mentions(rhs, iv) {
            affine_decompose(rhs, iv, consts)
        } else {
            eval_const_int(rhs, consts).map(|k| (0_i64, k))
        };
        let (l, r) = (lhs_aff?, rhs_aff?);
        match op {
            IrBinOp::Add => return Some((l.0.checked_add(r.0)?, l.1.checked_add(r.1)?)),
            IrBinOp::Sub => return Some((l.0.checked_sub(r.0)?, l.1.checked_sub(r.1)?)),
            IrBinOp::Mul => {
                // c * iv / iv * c only — at most ONE side mentions iv.
                // (Both sides mentioning iv ⇒ iv*iv ⇒ not affine.)
                match (l.0, r.0) {
                    (0, _) => {
                        // l is pure constant l.1, r is c*iv + d → scale r.
                        return Some((l.1.checked_mul(r.0)?, l.1.checked_mul(r.1)?));
                    }
                    (_, 0) => {
                        return Some((r.1.checked_mul(l.0)?, r.1.checked_mul(l.1)?));
                    }
                    _ => return None,
                }
            }
            // Div / Mod with iv on either side are not affine in iv.
            IrBinOp::Div | IrBinOp::Mod => return None,
        }
    }

    None
}

/// Does `e` syntactically contain an `Ident(iv)` anywhere in its tree?
fn expr_mentions(e: &IrExpr, iv: &str) -> bool {
    match e {
        IrExpr::Ident(n) => n == iv,
        IrExpr::IntLit(_) => false,
        IrExpr::Neg(inner) => expr_mentions(inner, iv),
        IrExpr::BinOp(_, lhs, rhs) => expr_mentions(lhs, iv) || expr_mentions(rhs, iv),
        IrExpr::DataRef(_) | IrExpr::Call { .. } => false,
    }
}

/// Try to evaluate `e` as an integer constant. Returns `None` if `e`
/// references any non-const identifier (an iter-var or unknown name) or
/// contains a DataRef / Call / overflow / div-by-zero. Mirrors the
/// minimum subset of `algo::lower::eval_const` needed for halo offset
/// recovery — we deliberately keep this local + small so the pass has
/// no upstream coupling beyond the `consts` table.
fn eval_const_int(e: &IrExpr, consts: &BTreeMap<String, ResolvedConst>) -> Option<i64> {
    match e {
        IrExpr::IntLit(v) => Some(*v),
        IrExpr::Ident(name) => consts.get(name).map(|c| c.value),
        IrExpr::Neg(inner) => eval_const_int(inner, consts).and_then(i64::checked_neg),
        IrExpr::BinOp(op, lhs, rhs) => {
            let l = eval_const_int(lhs, consts)?;
            let r = eval_const_int(rhs, consts)?;
            match op {
                IrBinOp::Add => l.checked_add(r),
                IrBinOp::Sub => l.checked_sub(r),
                IrBinOp::Mul => l.checked_mul(r),
                IrBinOp::Div => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_div(r)
                    }
                }
                IrBinOp::Mod => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_rem(r)
                    }
                }
            }
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => None,
    }
}

// --------------------------------------------------------------------
// Unit tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::{
        AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, Purity, ResolvedConst, ResolvedData,
        ResolvedKernel, ResolvedType, ScalarType,
    };
    use crate::link::link;
    use crate::sched::{ResolvedPlaceTarget, ResolvedPlacement, ResolvedWorker, SchedIR};

    // ---- Helpers ----

    fn t_scalar(ty: ScalarType) -> ResolvedType {
        ResolvedType {
            scalar: ty,
            dims: vec![],
        }
    }
    fn t_arr(ty: ScalarType, dims: Vec<usize>) -> ResolvedType {
        ResolvedType { scalar: ty, dims }
    }

    /// Build a tiny LinkedIR for halo-inference tests. The shape is:
    /// - one data symbol `grid` of given dims and one out symbol `out`
    /// - one kernel `K` (pure, params/ret types are irrelevant to halo)
    /// - placement: K on a single worker `w0`
    /// - the body statements are provided by the caller.
    fn build_linked(stmts: Vec<IrStmt>, grid_dims: Vec<usize>) -> LinkedIR {
        let mut data = BTreeMap::new();
        data.insert(
            "grid".to_string(),
            ResolvedData {
                name: "grid".to_string(),
                ty: t_arr(ScalarType::I32, grid_dims.clone()),
            },
        );
        data.insert(
            "out".to_string(),
            ResolvedData {
                name: "out".to_string(),
                ty: t_arr(ScalarType::I32, grid_dims),
            },
        );
        let mut kernels = BTreeMap::new();
        kernels.insert(
            "K".to_string(),
            ResolvedKernel {
                name: "K".to_string(),
                params: vec![t_scalar(ScalarType::I32)],
                ret: Some(t_scalar(ScalarType::I32)),
                purity: Purity::Pure,
                name_span: None,
            },
        );
        let algo = AlgoIR {
            consts: BTreeMap::new(),
            data,
            kernels,
            stmts,
        };

        // Minimal SchedIR: one placement of K on a single worker.
        let mut places: BTreeMap<String, ResolvedPlacement> = BTreeMap::new();
        places.insert(
            "K".to_string(),
            ResolvedPlacement {
                kernel: "K".to_string(),
                target: ResolvedPlaceTarget::One("w0".to_string()),
                kernel_span: None,
            },
        );
        let mut workers: BTreeMap<String, ResolvedWorker> = BTreeMap::new();
        workers.insert(
            "w0".to_string(),
            ResolvedWorker {
                name: "w0".to_string(),
                class: crate::sched::DEFAULT_WORKER_CLASS.to_string(),
            },
        );
        let sched = SchedIR {
            algo_path: String::new(),
            worker_classes: BTreeMap::new(),
            memory_regions: BTreeMap::new(),
            workers,
            places,
            place_data: BTreeMap::new(),
            loops: BTreeMap::new(),
            transfers: BTreeMap::new(),
            checks: BTreeMap::new(),
        };

        link(algo, sched).expect("link must succeed for halo test fixtures")
    }

    fn ir_int(v: i64) -> IrExpr {
        IrExpr::IntLit(v)
    }
    fn ir_id(s: &str) -> IrExpr {
        IrExpr::Ident(s.to_string())
    }
    fn ir_add(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Add, Box::new(l), Box::new(r))
    }
    fn ir_sub(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Sub, Box::new(l), Box::new(r))
    }
    fn ir_mul(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Mul, Box::new(l), Box::new(r))
    }
    fn ir_call(callee: &str, args: Vec<IrExpr>) -> IrExpr {
        IrExpr::Call {
            callee: callee.to_string(),
            args,
        }
    }
    fn data_ref(name: &str, indices: Vec<IrExpr>) -> IrExpr {
        IrExpr::DataRef(IndexedRef {
            name: name.to_string(),
            indices,
        })
    }
    fn lhs(name: &str, indices: Vec<IrExpr>) -> IndexedRef {
        IndexedRef {
            name: name.to_string(),
            indices,
        }
    }

    // ---- Direct affine-decomposer tests (no LinkedIR) ----

    #[test]
    fn affine_decompose_iv_plus_one() {
        let e = ir_add(ir_id("y"), ir_int(1));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, 1)));
    }

    #[test]
    fn affine_decompose_iv_minus_one() {
        let e = ir_sub(ir_id("y"), ir_int(1));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, -1)));
    }

    #[test]
    fn affine_decompose_const_plus_iv() {
        let e = ir_add(ir_int(2), ir_id("y"));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, 2)));
    }

    #[test]
    fn affine_decompose_bare_iv() {
        let e = ir_id("y");
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((1, 0)));
    }

    #[test]
    fn affine_decompose_negated_iv() {
        let e = IrExpr::Neg(Box::new(ir_id("y")));
        // coefficient -1 — recognised but caller rejects.
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((-1, 0)));
    }

    #[test]
    fn affine_decompose_strided_two_iv() {
        let e = ir_mul(ir_int(2), ir_id("y"));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), Some((2, 0)));
    }

    #[test]
    fn affine_decompose_iv_squared_is_none() {
        let e = ir_mul(ir_id("y"), ir_id("y"));
        assert_eq!(affine_decompose(&e, "y", &BTreeMap::new()), None);
    }

    #[test]
    fn affine_decompose_uses_const_table() {
        // const STRIDE = 2; index = y + STRIDE → (1, 2).
        let mut consts = BTreeMap::new();
        consts.insert(
            "STRIDE".to_string(),
            ResolvedConst {
                name: "STRIDE".to_string(),
                ty: ScalarType::I32,
                value: 2,
            },
        );
        let e = ir_add(ir_id("y"), ir_id("STRIDE"));
        assert_eq!(affine_decompose(&e, "y", &consts), Some((1, 2)));
    }

    // ---- Full-pipeline tests via apply_halo_inference ----

    /// Tiny ACFG builder: pulls `linked` through build_acfg. Avoids
    /// the partition/transfer passes — halo inference works on the raw
    /// `build_acfg` output (post-block-transform is fine but not needed
    /// for these synthetic algorithms with no block= directive).
    fn build_acfg_and_apply(linked: &LinkedIR) -> Result<ACFG, HaloInferenceError> {
        let acfg = crate::acfg::build_acfg(linked).expect("acfg build");
        apply_halo_inference(linked, acfg)
    }

    #[test]
    fn positive_3point_stencil_along_y() {
        // for y : 1..15 { out[y] <-- K(grid[y-1], grid[y], grid[y+1]) }
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(1))]),
                        data_ref("grid", vec![ir_id("y")]),
                        data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                    ],
                ),
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
        let k_id = *acfg.name_kernels.get("K").unwrap();
        let y_iv = *acfg.name_iter_vars.get("y").unwrap();
        assert_eq!(
            acfg.halo_widths
                .get(&k_id)
                .and_then(|m| m.get(&y_iv))
                .copied(),
            Some(1)
        );
        // No other keys.
        assert_eq!(acfg.halo_widths.len(), 1);
    }

    #[test]
    fn positive_9point_stencil_two_axes() {
        // for y : 1..15 { for x : 1..15 {
        //   out[y][x] <-- K(grid[y-1][x-1], grid[y][x], grid[y+1][x+1])
        // } }
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            body: vec![IrStmt::For {
                var: "x".to_string(),
                lo: ir_int(1),
                hi: ir_int(15),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("y"), ir_id("x")]),
                    rhs: ir_call(
                        "K",
                        vec![
                            data_ref(
                                "grid",
                                vec![ir_sub(ir_id("y"), ir_int(1)), ir_sub(ir_id("x"), ir_int(1))],
                            ),
                            data_ref("grid", vec![ir_id("y"), ir_id("x")]),
                            data_ref(
                                "grid",
                                vec![ir_add(ir_id("y"), ir_int(1)), ir_add(ir_id("x"), ir_int(1))],
                            ),
                        ],
                    ),
                }],
            }],
        }];
        let linked = build_linked(stmts, vec![16, 16]);
        let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
        let k_id = *acfg.name_kernels.get("K").unwrap();
        let y_iv = *acfg.name_iter_vars.get("y").unwrap();
        let x_iv = *acfg.name_iter_vars.get("x").unwrap();
        assert_eq!(
            acfg.halo_widths
                .get(&k_id)
                .and_then(|m| m.get(&y_iv))
                .copied(),
            Some(1)
        );
        assert_eq!(
            acfg.halo_widths
                .get(&k_id)
                .and_then(|m| m.get(&x_iv))
                .copied(),
            Some(1)
        );
        // Outer map has one entry (the kernel K); inner map has two
        // (one per axis).
        assert_eq!(acfg.halo_widths.len(), 1);
        assert_eq!(acfg.halo_widths.get(&k_id).map(|m| m.len()), Some(2));
    }

    #[test]
    fn positive_mixed_access_widest_wins() {
        // for y : 2..14 {
        //   out[y] <-- K(grid[y-2], grid[y], grid[y+1])
        // }
        // halo on y should be max(2, 0, 1) = 2.
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(2),
            hi: ir_int(14),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(2))]),
                        data_ref("grid", vec![ir_id("y")]),
                        data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                    ],
                ),
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
        let k_id = *acfg.name_kernels.get("K").unwrap();
        let y_iv = *acfg.name_iter_vars.get("y").unwrap();
        assert_eq!(
            acfg.halo_widths
                .get(&k_id)
                .and_then(|m| m.get(&y_iv))
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn no_halo_pure_constant_index() {
        // for y : 0..4 { out[y] <-- K(grid[3]) }
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call("K", vec![data_ref("grid", vec![ir_int(3)])]),
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
        assert!(acfg.halo_widths.is_empty());
    }

    #[test]
    fn no_halo_bare_iv() {
        // for y : 0..4 { out[y] <-- K(grid[y]) } — halo 0.
        //
        // The contract (brief): "the (kernel, iv) entry is either
        // missing OR maps to 0." The implementation chooses to record
        // an explicit 0-width entry on every (kernel, iv) pair the
        // detector inspects — this makes the sidecar's keyset a
        // useful "every iv this kernel touches" index for the Stage 2
        // consumer (TASK-0263). A non-touching iv simply has no entry.
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call("K", vec![data_ref("grid", vec![ir_id("y")])]),
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
        let k_id = *acfg.name_kernels.get("K").unwrap();
        let y_iv = *acfg.name_iter_vars.get("y").unwrap();
        // Width must be 0 (or absent — both satisfy the contract; we
        // emit explicit 0).
        let width = acfg
            .halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied()
            .unwrap_or(0);
        assert_eq!(width, 0);
    }

    #[test]
    fn negative_data_dependent_stride() {
        // for y : 0..4 { out[y] <-- K(grid[lookup[y]]) }
        // — index expression is grid[(lookup[y])] which is a DataRef inside the index. Reject.
        // Add a `lookup` data symbol to the algorithm.
        let mut linked = build_linked(
            vec![IrStmt::For {
                var: "y".to_string(),
                lo: ir_int(0),
                hi: ir_int(4),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("y")]),
                    rhs: ir_call(
                        "K",
                        vec![data_ref("grid", vec![data_ref("lookup", vec![ir_id("y")])])],
                    ),
                }],
            }],
            vec![16],
        );
        linked.algo.data.insert(
            "lookup".to_string(),
            ResolvedData {
                name: "lookup".to_string(),
                ty: t_arr(ScalarType::I32, vec![16]),
            },
        );
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let err = apply_halo_inference(&linked, acfg).unwrap_err();
        match err {
            HaloInferenceError::DataDependentStride {
                kernel,
                ref_name,
                ax_idx,
            } => {
                assert_eq!(kernel, "K");
                assert_eq!(ref_name, "grid");
                assert_eq!(ax_idx, 0);
            }
            other => panic!("expected DataDependentStride, got {other:?}"),
        }
    }

    #[test]
    fn negative_strided_access() {
        // for y : 0..4 { out[y] <-- K(grid[2*y + 1]) } — coefficient 2, reject.
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref(
                        "grid",
                        vec![ir_add(ir_mul(ir_int(2), ir_id("y")), ir_int(1))],
                    )],
                ),
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let err = apply_halo_inference(&linked, acfg).unwrap_err();
        match err {
            HaloInferenceError::StridedAccessNotSupported { coefficient, .. } => {
                assert_eq!(coefficient, 2);
            }
            other => panic!("expected StridedAccessNotSupported, got {other:?}"),
        }
    }

    #[test]
    fn negative_two_iter_vars_in_one_index() {
        // for y : 0..4 { for x : 0..4 {
        //   out[y][x] <-- K(grid[y + x]) — two iter-vars in one index. Reject.
        // } }
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            body: vec![IrStmt::For {
                var: "x".to_string(),
                lo: ir_int(0),
                hi: ir_int(4),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("y"), ir_id("x")]),
                    rhs: ir_call(
                        "K",
                        vec![data_ref("grid", vec![ir_add(ir_id("y"), ir_id("x"))])],
                    ),
                }],
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let err = apply_halo_inference(&linked, acfg).unwrap_err();
        match err {
            HaloInferenceError::MultipleIterVarsInIndex { iter_vars, .. } => {
                assert_eq!(iter_vars, vec!["x".to_string(), "y".to_string()]);
            }
            other => panic!("expected MultipleIterVarsInIndex, got {other:?}"),
        }
    }

    #[test]
    fn negative_negated_iv_rejected() {
        // for y : 0..4 { out[y] <-- K(grid[-y]) } — coefficient -1, reject.
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref("grid", vec![IrExpr::Neg(Box::new(ir_id("y")))])],
                ),
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let err = apply_halo_inference(&linked, acfg).unwrap_err();
        match err {
            HaloInferenceError::StridedAccessNotSupported { coefficient, .. } => {
                assert_eq!(coefficient, -1);
            }
            other => panic!("expected StridedAccessNotSupported, got {other:?}"),
        }
    }

    #[test]
    fn determinism_same_input_yields_same_map() {
        // Run the same input through the pass twice; assert the
        // resulting halo_widths maps are byte-identical (well, value-
        // identical — BTreeMap implements PartialEq).
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(1))]),
                        data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                    ],
                ),
            }],
        }];
        let linked = build_linked(stmts, vec![16]);
        let acfg1 = build_acfg_and_apply(&linked).expect("first run");
        let acfg2 = build_acfg_and_apply(&linked).expect("second run");
        assert_eq!(acfg1.halo_widths, acfg2.halo_widths);
    }

    #[test]
    fn nested_call_inside_kernel_arg_recurses() {
        // for y : 1..15 {
        //   out[y] <-- K(inner(grid[y-1], grid[y+1]))
        // }
        // The OUTER call K has no DataRef args (its arg is a Call).
        // Halo inference must still scan the INNER call's args and
        // record halo against `inner`'s KernelId, not K's.
        let mut linked = build_linked(
            vec![IrStmt::For {
                var: "y".to_string(),
                lo: ir_int(1),
                hi: ir_int(15),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("y")]),
                    rhs: ir_call(
                        "K",
                        vec![ir_call(
                            "inner",
                            vec![
                                data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(1))]),
                                data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                            ],
                        )],
                    ),
                }],
            }],
            vec![16],
        );
        // Add the `inner` kernel + its placement so link succeeds.
        linked.algo.kernels.insert(
            "inner".to_string(),
            ResolvedKernel {
                name: "inner".to_string(),
                params: vec![t_scalar(ScalarType::I32), t_scalar(ScalarType::I32)],
                ret: Some(t_scalar(ScalarType::I32)),
                purity: Purity::Pure,
                name_span: None,
            },
        );
        linked.sched.places.insert(
            "inner".to_string(),
            ResolvedPlacement {
                kernel: "inner".to_string(),
                target: ResolvedPlaceTarget::One("w0".to_string()),
                kernel_span: None,
            },
        );
        // Re-link to refresh kernel_workers / placements.
        let linked = link(linked.algo, linked.sched).expect("re-link");
        let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
        let inner_id = *acfg.name_kernels.get("inner").unwrap();
        let k_id = *acfg.name_kernels.get("K").unwrap();
        let y_iv = *acfg.name_iter_vars.get("y").unwrap();
        // Halo recorded against the INNER kernel (the one whose args
        // touch the DataRefs).
        assert_eq!(
            acfg.halo_widths
                .get(&inner_id)
                .and_then(|m| m.get(&y_iv))
                .copied(),
            Some(1)
        );
        // The outer K has no halo entry.
        assert_eq!(
            acfg.halo_widths
                .get(&k_id)
                .and_then(|m| m.get(&y_iv))
                .copied(),
            None
        );
    }

    #[test]
    fn no_halo_call_outside_loop_scope() {
        // K(grid[3]) at top level — no enclosing for, no halo entries.
        // (The grid[3] is pure-constant anyway, but the absence of a
        // for nest is itself a stricter no-halo signal.)
        let stmts = vec![IrStmt::Effect {
            callee: "K".to_string(),
            args: vec![data_ref("grid", vec![ir_int(3)])],
        }];
        // K's purity must be effectful for an Effect statement.
        let mut linked = build_linked(stmts, vec![16]);
        linked.algo.kernels.get_mut("K").unwrap().purity = Purity::Effectful;
        // re-link to take the updated purity.
        let linked = link(linked.algo, linked.sched).expect("re-link with effectful K");
        let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
        assert!(acfg.halo_widths.is_empty());
    }
}
