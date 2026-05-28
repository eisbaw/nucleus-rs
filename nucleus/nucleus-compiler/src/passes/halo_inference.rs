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
//!
//!    **TASK-0305 cycle-122 project decision (Option B)**: the
//!    `absent ≡ explicit-0` degree of freedom is the LOAD-BEARING
//!    representation contract; downstream tests MAY (and the shipped
//!    `tests/sidecar_halo.rs` `task0299_*` / `task0303_*` narrative
//!    pins DO) consult `halo_widths.get(...).copied().unwrap_or(0)`
//!    and accept either form. Trade-off accepted: a future
//!    silent-skip regression on the per-iv `or_insert(0)` emit site
//!    (the production sink at `classify_index` — search for
//!    `per_iv.entry(iv).or_insert(0)` in this file) would let `== 0`
//!    narrative pins pass vacuously. Judged unlikely; preserving the
//!    contract robustness is worth the accepted vacuous-pass arm. A
//!    future change to require explicit-0 (Option C) would need a
//!    contract-pin test of the new invariant plus consumer-side
//!    `unwrap_or(0)` → explicit `expect(...)` migration across the
//!    test suite.
//!
//!    **TASK-0307 cycle-123 structural sentinel**: the in-module test
//!    `no_halo_bare_iv` (search for `fn no_halo_bare_iv` in this
//!    file) carries a `copied() == Some(0)` structural assertion
//!    that fails LOUD if the production walker at `classify_index` is
//!    silently regressed to skip emission for inspected (kernel, iv)
//!    pairs. This closes the Option B vacuous-pass arm at the
//!    contract boundary WITHOUT coupling the downstream
//!    `task0299_*` / `task0303_*` narrative pins to the explicit-0
//!    representation. Future contract-form changes (e.g. Option C
//!    above) MUST update this sentinel alongside the contract doc.
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
//! ## Strict vs advisory vs partition-policy-aware entry points
//!
//! Three entry points exist:
//!
//! - [`apply_halo_inference`] is the **strict** variant: returns
//!   `Err(HaloInferenceError)` on the first non-affine / strided /
//!   data-dependent index it sees. Used by tests + direct callers
//!   that want fail-fast semantics. This is what the negative-test
//!   gate exercises.
//! - [`apply_halo_inference_advisory`] is the **lenient** variant:
//!   walks the algorithm to completion, records every affine fact it
//!   CAN classify, returns the populated ACFG + the vector of typed
//!   errors that the strict variant would have raised. Retained for
//!   in-pass tests + direct callers that want the full error vector;
//!   NOT called from the driver.
//! - [`apply_halo_inference_partition_aware`] is the **(B')
//!   partition-policy-aware** variant. Originally landed as the
//!   (B) rule under TASK-0275 (cycle 95), refined to (B') under
//!   TASK-0341.02.02.01 (cycle 209). For each typed error the
//!   walker raises, the fatality predicate is one of two rules
//!   depending on the error variant:
//!
//!   * For `DataDependentStride` and `UnknownKernelInCall` —
//!     where the iv set the failing index actually depends on is
//!     not bounded by the lexical iv walk — the rule consults the
//!     enclosing-loop scope: if ANY iv in that scope carries a
//!     [`crate::sched::ResolvedLoopOption::Partition`] directive,
//!     the error is FATAL (the original cycle-95 (B) rule).
//!   * For all other variants (`NonAffineIndex`,
//!     `StridedAccessNotSupported`, `MultipleIterVarsInIndex`,
//!     `UnknownLoopVar`) — where the failing index expression's iv
//!     set is recoverable lexically via [`collect_iter_var_refs`] —
//!     the rule consults THAT iv set: fatal iff at least one of
//!     those ivs is partitioned. This is the cycle-209 refinement.
//!
//!   Otherwise the error is recorded in the advisory vector and
//!   lowering proceeds with whatever halo widths the walker COULD
//!   recover. This is the driver's entry point as of TASK-0275 /
//!   TASK-0341.02.02.01.
//!
//! Why the driver is partition-policy-aware and NOT (A) strict
//! (cf. TASK-0271 reuse precedent which IS (A) strict): the halo
//! consumer in `transfer_inject` (cycle 83, commit cf2f9ac) only
//! extends per-tile transfer ranges when the iv at the kernel-call
//! site is itself partitioned. If the iv is NOT partitioned, a missing
//! halo entry is harmless — the consumer does not fire. The reuse Tier
//! 1 marker by contrast fires for EVERY recognised slot regardless of
//! partition, so (B) degenerates into (A) for reuse and the simpler
//! strict promotion sufficed (TASK-0271 cycle 88).
//!
//! Two real-world reachable cases the (B') policy preserves:
//!
//! - Example 11 (`11-game-of-life`) reads
//!   `grid[(t + ITERS) % (ITERS + 1)]` — a compile-time-constant
//!   `Mod` wrap the affine detector cannot fold. The naive/
//!   pipelined schedules carry ZERO `partition=` directives, so the
//!   failing-index iv set `{t}` and the scope set are both
//!   un-partitioned; under (B') (and the cycle-95 (B)) this stays
//!   advisory and both cells stay PASS. A naive (A) strict mirror
//!   would newly-reject example 11. See TASK-0263 cycle-89
//!   verification block for the full reasoning.
//!
//! - Example 16 (`16-jacobi`) /distributed reads
//!   `field[(t + ITERS) % (ITERS + 1)][y-1][x]` under
//!   `partition=rows` on `y`. The failing axis is axis 0 (iv `t`,
//!   NOT partitioned); the partitioned iv `y` is on axis 1 (`y-1`
//!   on axis 1 IS affine — no error there). The cycle-95 (B) rule
//!   incorrectly classified this fatal because `y` was in the
//!   enclosing scope; the cycle-209 (B') rule correctly classifies
//!   it advisory because the failing-index iv set is `{t}` only,
//!   and `t` is not partitioned. Closes TASK-0341.02.02 AC#3.
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
use crate::algo::{IndexedRef, IrExpr, IrStmt, ResolvedConst};
use crate::event::{IterVar, KernelId};
use crate::link::LinkedIR;
use crate::sched::ResolvedLoopOption;
// Shared affine-stride helpers — see [`crate::passes::common`] module
// docs. The lift in cycle 82 is single-use today but the second
// consumer (TASK-0261 reuse_inference) is landing in the same series
// (cycle-81 review forward-carry, F-P1). Halo inference uses only
// `affine_decompose`; `eval_const_int` / `expr_mentions` are reachable
// through that call and not re-imported here.
use crate::passes::common::affine_decompose;

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
    ///
    /// Variant + field name aligned (TASK-0272 scope A, cycle 95) with
    /// the same shape across 5 sibling passes (partition_workers,
    /// partition_blocks2d, partition_rows, block_transform,
    /// reuse_inference — all carry `UnknownLoopVar { var: String }`).
    UnknownLoopVar { var: String },
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
            HaloInferenceError::UnknownLoopVar { var } => write!(
                f,
                "halo inference: iter-var `{var}` was collected from lexical scope but is \
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
    let (halo, errors_with_scope) = infer_halo_widths(linked, &acfg);
    if let Some((e, _scope, _ivs)) = errors_with_scope.into_iter().next() {
        return Err(e);
    }
    Ok(commit_halo_widths(acfg, halo))
}

/// Lenient variant of [`apply_halo_inference`]: walks the algorithm,
/// records every halo-relevant `iv + b` pattern it CAN classify, and
/// returns every typed error it would have raised so the caller decides
/// whether each shape is fatal.
///
/// ## Driver policy (TASK-0275)
///
/// The driver no longer calls this entry point — it was promoted to
/// the partition-policy-aware [`apply_halo_inference_partition_aware`]
/// once the TASK-0263 transfer_inject halo consumer landed (cycle 83,
/// commit cf2f9ac). A typed error under a partitioned iv now
/// corresponds to a halo the backend would silently fail to ship.
///
/// This advisory entry point is retained for in-pass tests + direct
/// callers that want to inspect the FULL error vector. Do NOT route
/// the driver through this function without re-deciding the policy in
/// TASK-0275's spirit.
pub fn apply_halo_inference_advisory(
    linked: &LinkedIR,
    acfg: ACFG,
) -> (ACFG, Vec<HaloInferenceError>) {
    let (halo, errors_with_scope) = infer_halo_widths(linked, &acfg);
    let acfg = commit_halo_widths(acfg, halo);
    let errors: Vec<HaloInferenceError> = errors_with_scope
        .into_iter()
        .map(|(e, _scope, _ivs)| e)
        .collect();
    (acfg, errors)
}

/// Partition-policy-aware variant — (B') rule as of
/// TASK-0341.02.02.01 cycle 209. Per-error fatality decision based
/// on whether the FAILING INDEX EXPRESSION references an iv that
/// carries a [`ResolvedLoopOption::Partition`] directive in the
/// schedule (refinement of the cycle-95 (B) rule, which consulted
/// the FULL ENCLOSING SCOPE).
///
/// Rationale (see module-level "Strict vs advisory vs
/// partition-policy-aware entry points" for the full picture): the
/// `transfer_inject` halo consumer extends per-tile transfer ranges
/// only when the iv at the kernel-call site is itself partitioned.
/// If the failing index references only non-partitioned ivs (e.g.
/// `field[(t + ITERS) % (ITERS + 1)][y-1][x]` where `y` IS
/// partitioned but axis 0 references only `t`), the missing halo
/// entry is harmless — the consumer does not fire on that axis.
///
/// The cycle-95 (B) rule's predicate `scope.iter().any(...)` was
/// one axis-mapping degree of freedom too coarse: it fired fatal
/// whenever ANY iv in the lexical scope was partitioned, regardless
/// of which iv the failing index referenced. 16-jacobi/distributed
/// (cycle 208) was the first encountered shape where a non-affine
/// wrap on a non-partitioned axis SHARED the enclosing scope with a
/// partitioned iv on a DIFFERENT axis; the (B) rule rejected even
/// though no halo strip on the partitioned axis was at risk.
///
/// Per-variant rule split (see [`classify_index`] for the iv-set
/// population at each push site):
///
/// - [`HaloInferenceError::DataDependentStride`] — index is itself
///   a `DataRef`/`Call`; the iv set is empty (the lexical walker
///   does not descend into data-dependent addresses). Falls back
///   to the conservative SCOPE predicate: if any iv in scope is
///   partitioned, fatal. Rationale: the runtime value of the
///   data-dependent address determines which cell is read, and
///   that is unbounded by the lexical iv set.
/// - All other variants ([`NonAffineIndex`],
///   [`StridedAccessNotSupported`], [`MultipleIterVarsInIndex`],
///   [`UnknownLoopVar`]) — the iv set is populated by
///   [`collect_iter_var_refs`] at the error-push site. Fatal iff
///   AT LEAST ONE iv in the failing-index set is partitioned;
///   advisory otherwise. The 11-game-of-life regression pin
///   ("no partition + Mod wrap stays advisory") is preserved: the
///   pipelined/naive schedules carry zero `partition=`, so no iv
///   in the failing-index set (= `{t}`) returns `true` from
///   [`iv_is_partitioned`].
///
/// Returns `Ok((acfg, advisory_errors))` when no error was
/// classified fatal; the advisory vector lists the typed errors the
/// walker raised that the (B') policy deemed harmless. Returns
/// `Err(e)` on the first error classified fatal — that is the
/// fail-fast contract the driver leans on. The returned ACFG is the
/// committed sidecar (mirrors the advisory variant: partial halo
/// widths from the walker are preserved).
///
/// [`NonAffineIndex`]: HaloInferenceError::NonAffineIndex
/// [`StridedAccessNotSupported`]: HaloInferenceError::StridedAccessNotSupported
/// [`MultipleIterVarsInIndex`]: HaloInferenceError::MultipleIterVarsInIndex
/// [`UnknownLoopVar`]: HaloInferenceError::UnknownLoopVar
pub fn apply_halo_inference_partition_aware(
    linked: &LinkedIR,
    acfg: ACFG,
) -> Result<(ACFG, Vec<HaloInferenceError>), HaloInferenceError> {
    let (halo, errors_with_scope) = infer_halo_widths(linked, &acfg);
    let mut advisory: Vec<HaloInferenceError> = Vec::new();
    for (err, scope, ivs_in_index) in errors_with_scope {
        if error_is_fatal_under_partition(&err, &scope, &ivs_in_index, linked) {
            return Err(err);
        }
        advisory.push(err);
    }
    Ok((commit_halo_widths(acfg, halo), advisory))
}

/// (B') per-variant fatality predicate. Split from the entry-point
/// loop so the per-variant rule is self-documenting + unit-testable
/// without recreating an `infer_halo_widths` walk.
///
/// See [`apply_halo_inference_partition_aware`] for the doc on the
/// per-variant split. The two-rule shape is intentional: `ivs_in_index`
/// is precise but lexical, so the data-dependent variant — whose
/// address is determined at runtime, not by lexically-visible ivs —
/// falls back to the conservative pre-cycle-209 enclosing-scope rule.
fn error_is_fatal_under_partition(
    err: &HaloInferenceError,
    scope: &[String],
    ivs_in_index: &[String],
    linked: &LinkedIR,
) -> bool {
    match err {
        // Data-dependent address: the runtime cell the worker reads
        // is determined by `lookup[..]` or `f(..)`, not by the
        // lexically-visible iv set. The (B') refinement cannot
        // safely narrow this case — keep the (B) conservative
        // enclosing-scope rule.
        HaloInferenceError::DataDependentStride { .. } => {
            scope.iter().any(|iv| iv_is_partitioned(linked, iv))
        }
        // Link-invariant break: the kernel id was missing from
        // `name_kernels`. Never reachable from a link-valid IR.
        // Fall back to scope conservatively — the variant exists
        // primarily for fail-closed diagnostics on inconsistently-
        // constructed `(LinkedIR, ACFG)` pairs.
        HaloInferenceError::UnknownKernelInCall { .. } => {
            scope.iter().any(|iv| iv_is_partitioned(linked, iv))
        }
        // Precise variants: the iv set is populated from the
        // failing index expression at the error-push site. Fatal
        // iff at least one of those ivs is partitioned.
        HaloInferenceError::NonAffineIndex { .. }
        | HaloInferenceError::StridedAccessNotSupported { .. }
        | HaloInferenceError::MultipleIterVarsInIndex { .. }
        | HaloInferenceError::UnknownLoopVar { .. } => {
            ivs_in_index.iter().any(|iv| iv_is_partitioned(linked, iv))
        }
    }
}

/// Does the schedule's `loops` table tag this iv with a
/// `partition=` directive? Returns false for missing iv (no schedule
/// directive at all) and false for ivs whose directive carries only
/// non-partition options (`block=`, `pipeline=`, `reuse`).
///
/// Used by [`apply_halo_inference_partition_aware`] for the per-error
/// fatality decision.
fn iv_is_partitioned(linked: &LinkedIR, iv: &str) -> bool {
    linked
        .sched
        .loops
        .get(iv)
        .map(|d| {
            d.options
                .iter()
                .any(|o| matches!(o, ResolvedLoopOption::Partition(_)))
        })
        .unwrap_or(false)
}

/// Halo-widths map: kernel → iter-var → halo width.
type HaloMap = BTreeMap<KernelId, BTreeMap<IterVar, u64>>;
/// Typed error paired with TWO iv-name vectors captured at the
/// error-push site:
///
/// - The enclosing-loop scope (outermost-first iter-var names) — the
///   pre-TASK-0341.02.02.01 fatality input. Kept for the conservative
///   fallback on [`HaloInferenceError::DataDependentStride`] (where
///   the index expression is itself a `DataRef`/`Call` and an
///   iv-name walk through the data-dependent address would not
///   surface every iv the partition impact depends on).
/// - The ivs the FAILING INDEX EXPRESSION actually references —
///   collected via [`collect_iter_var_refs`]. Empty when the index
///   is a `DataDependentStride` (the walker short-circuits on
///   data-dependent shape before iv collection) OR a pure-constant
///   index that nonetheless trips the const-fold.
///
/// Load-bearing for [`apply_halo_inference_partition_aware`]'s (B')
/// fatality predicate (TASK-0341.02.02.01 cycle 209): the precise
/// iv set lets a non-affine wrap on a non-partitioned axis stay
/// advisory even when a sibling axis IS partitioned (the 16-jacobi/
/// distributed case the (B) rule incorrectly rejected). See the
/// module-level "Strict vs advisory vs partition-policy-aware entry
/// points" doc for the per-variant rule split.
type HaloErrorWithScope = (HaloInferenceError, Vec<String>, Vec<String>);

/// Core inference: walk `linked.algo.stmts` and return both the
/// populated halo map AND the typed errors raised along the way,
/// PAIRED with two iv-name vectors at the error-push site: the
/// enclosing-loop scope (outermost-first) AND the ivs the failing
/// index expression actually references. The walker is COLLECTING
/// (does not short-circuit on the first error) so the lenient +
/// (B') partition-policy-aware variants can both walk a per-error
/// decision over the full error list. The strict variant short-
/// circuits on the first error at the entry point.
///
/// Both iv-vectors are load-bearing for
/// [`apply_halo_inference_partition_aware`]'s per-variant (B')
/// rule split — see that function's docstring for the per-variant
/// rule split between scope-fallback (data-dependent /
/// link-invariant) and ivs-in-index (precise affine variants).
/// [`apply_halo_inference`] and [`apply_halo_inference_advisory`]
/// strip both iv-vectors at their call sites (they only need the
/// typed error).
fn infer_halo_widths(linked: &LinkedIR, acfg: &ACFG) -> (HaloMap, Vec<HaloErrorWithScope>) {
    let ctx = WalkCtx {
        name_kernels: &acfg.name_kernels,
        name_iter_vars: &acfg.name_iter_vars,
        consts: &linked.algo.consts,
    };
    let mut halo: HaloMap = BTreeMap::new();
    let mut errors: Vec<HaloErrorWithScope> = Vec::new();
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
        // TASK-0261: halo_inference does not consult or mutate the
        // reuse-widths sidecar; forward verbatim. reuse_inference runs
        // separately and writes its own field.
        reuse_widths,
        // TASK-0264 cycle 113: halo_inference does not consult or
        // mutate partition_pairs / grid_shape_for_outer_iv (populated
        // by partition_blocks2d, which runs before halo_inference);
        // forward verbatim.
        partition_pairs,
        grid_shape_for_outer_iv,
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
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
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
    errors: &mut Vec<HaloErrorWithScope>,
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
    errors: &mut Vec<HaloErrorWithScope>,
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
    errors: &mut Vec<HaloErrorWithScope>,
) {
    // Some "calls" are effect-statement load/capture kernels that the
    // ACFG name_kernels DOES include (every algorithm kernel is in the
    // name table). A miss is a link-invariant break — push the error
    // and skip this call's args (the kid is the join key; without it
    // the args have nowhere to land).
    let kid = match ctx.name_kernels.get(callee) {
        Some(k) => *k,
        None => {
            // Link-invariant break: empty iv set + scope kept for
            // diagnostic only. The per-error fatality predicate
            // would never reach this variant in practice (every
            // production callsite checks `name_kernels` before
            // halo_inference runs); the (B') rule's conservative
            // fallback applies if it ever did.
            errors.push((
                HaloInferenceError::UnknownKernelInCall {
                    callee: callee.to_string(),
                },
                scope.to_vec(),
                Vec::new(),
            ));
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
    errors: &mut Vec<HaloErrorWithScope>,
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
    errors: &mut Vec<HaloErrorWithScope>,
) {
    // Reject early: DataRef or Call inside the index = data-dependent.
    //
    // The (B') predicate (TASK-0341.02.02.01 cycle 209) uses the iv
    // set the FAILING INDEX EXPRESSION references — but a data-
    // dependent address (e.g. `grid[lookup[y]]`) makes the
    // partition impact unknowable from the lexical iv set alone:
    // the address depends on `lookup[y]`, and the runtime value of
    // `lookup[y]` determines which cell of `grid` the worker reads.
    // We push an empty iv set so the per-error fatality predicate
    // falls back to the conservative enclosing-scope rule for this
    // variant only (see `apply_halo_inference_partition_aware`).
    if expr_contains_dataref_or_call(e) {
        errors.push((
            HaloInferenceError::DataDependentStride {
                kernel: site.callee.to_string(),
                ref_name: site.ref_name.to_string(),
                ax_idx: site.ax_idx,
            },
            scope.to_vec(),
            Vec::new(),
        ));
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

    // The iv set for the failing-index payload is `ivs_used`
    // verbatim — populated lazily as a `Vec<String>` only at each
    // error-push site (and skipped on the success paths).
    if ivs_used.len() > 1 {
        let iv_list: Vec<String> = ivs_used.iter().cloned().collect();
        errors.push((
            HaloInferenceError::MultipleIterVarsInIndex {
                kernel: site.callee.to_string(),
                ref_name: site.ref_name.to_string(),
                ax_idx: site.ax_idx,
                iter_vars: ivs_used.into_iter().collect(),
            },
            scope.to_vec(),
            iv_list,
        ));
        return;
    }

    let iv_name = ivs_used.into_iter().next().expect("len == 1 checked");

    // Recover (coefficient, offset) from the expression. The detector
    // accepts only coefficient +1; -1 and |c| > 1 are rejected.
    let (coeff, offset) = match affine_decompose(e, &iv_name, ctx.consts) {
        Some(pair) => pair,
        None => {
            errors.push((
                HaloInferenceError::NonAffineIndex {
                    kernel: site.callee.to_string(),
                    ref_name: site.ref_name.to_string(),
                    ax_idx: site.ax_idx,
                },
                scope.to_vec(),
                vec![iv_name.clone()],
            ));
            return;
        }
    };

    if coeff != 1 {
        errors.push((
            HaloInferenceError::StridedAccessNotSupported {
                kernel: site.callee.to_string(),
                ref_name: site.ref_name.to_string(),
                ax_idx: site.ax_idx,
                coefficient: coeff,
            },
            scope.to_vec(),
            vec![iv_name.clone()],
        ));
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
            let iv_payload = vec![iv_name.clone()];
            errors.push((
                HaloInferenceError::UnknownLoopVar { var: iv_name },
                scope.to_vec(),
                iv_payload,
            ));
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

// NOTE: `affine_decompose`, `expr_mentions`, and `eval_const_int` were
// moved to [`crate::passes::common`] in cycle 82 (TASK-0261 prerequisite)
// so the reuse-inference pass can share one definition. The semantic
// contract is unchanged; only the call site moved. See the `common`
// module docs for the recognised affine shapes.

// --------------------------------------------------------------------
// Unit tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::{
        AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, Purity, ResolvedData, ResolvedKernel,
        ResolvedType, ScalarType,
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

    // ---- Direct affine-decomposer tests ----
    //
    // Moved to [`crate::passes::common`] tests in cycle 82 (TASK-0261
    // prerequisite). Halo inference still exercises the helper
    // transitively via the full-pipeline tests below; keeping the
    // helper-level coverage in the helper's own module avoids a
    // duplicate test surface that would skew the per-pass test count.

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
        // emit explicit 0). The lenient form (.unwrap_or(0)) documents
        // the contract; the structural form below pins TODAY'S
        // implementation choice as a sentinel.
        let width = acfg
            .halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied()
            .unwrap_or(0);
        assert_eq!(width, 0);

        // TASK-0307 cycle-123 structural sentinel (TASK-0305 cycle-122
        // Option B defence). Pins the implementation choice of recording
        // explicit `Some(0)` for every inspected (kernel, iv) pair at
        // the `classify_index` emit site (search for
        // `per_iv.entry(iv).or_insert(0)` in this file). A future
        // walker regression that silently DROPS entries for bare-iv
        // accesses would make the `== 0` `.unwrap_or(0)` narrative pins
        // in `tests/sidecar_halo.rs` (specifically `task0299_06` and
        // `task0303_07` — both `assert == 0`) pass vacuously: no entry
        // → `.unwrap_or(0)` → 0 ≡ 0. (`task0303_05` is the sibling
        // with the same idiom but `assert == 1` — strict-positive, so
        // contract-form-independent BY CONSTRUCTION, NOT vacuous-pass-
        // prone. Sentinel is moot there.) This single sentinel catches
        // the silent-skip at the contract boundary, without coupling
        // downstream tests to the explicit-0 representation (preserves
        // Option B contract).
        assert_eq!(
            acfg.halo_widths
                .get(&k_id)
                .and_then(|m| m.get(&y_iv))
                .copied(),
            Some(0),
            "structural sentinel: halo_inference must emit an \
             explicit `Some(0)` entry for every inspected (kernel, iv) \
             pair (today's contract-form choice — Option B per \
             TASK-0305). A silent-skip regression here would let the \
             `== 0` `.unwrap_or(0)` narrative pins in \
             tests/sidecar_halo.rs (specifically `task0299_06` and \
             `task0303_07`) pass vacuously."
        );
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

    // ---- TASK-0341.02.02.01 (B') partition-policy-aware regression
    // ---- pins. The cycle-209 refinement narrows the (B) fatality
    // ---- predicate from "any iv in enclosing scope is partitioned"
    // ---- to "the failing index expression itself references a
    // ---- partitioned iv". Pin both edges:
    //
    // - `bprime_modwrap_nonpartitioned_axis_stays_advisory` is the
    //   16-jacobi/distributed shape: a non-affine Mod-wrap on a
    //   non-partitioned axis SHARES the lexical scope with a
    //   partitioned iv on a DIFFERENT axis. Cycle 208 demonstrated
    //   the (B) rule rejected this; cycle 209's (B') rule classifies
    //   it advisory (the precise correctness condition).
    //
    // - `bprime_modwrap_partitioned_axis_stays_fatal` is the
    //   complement: a non-affine Mod-wrap ON the partitioned axis
    //   must STILL fire fatal. Verifies that the (B') refinement
    //   does not silently weaken correctness when the gap really
    //   matters.
    //
    // - `bprime_strided_on_partitioned_iv_stays_fatal` verifies the
    //   per-variant rule applies symmetrically to
    //   `StridedAccessNotSupported`: a stride-2 read on the
    //   partitioned iv still rejects.
    //
    // - `bprime_modwrap_no_partition_at_all_stays_advisory` is the
    //   11-game-of-life regression pin: under naive/pipelined
    //   schedules (no partition directive anywhere) the Mod-wrap
    //   error stays advisory (it did under (B); it must continue
    //   to under (B')).

    use crate::sched::{ResolvedLoopDirective, ResolvedLoopOption};

    /// Construct a tiny ResolvedLoopDirective adding a
    /// `partition=workers` option to the named iv. The exact
    /// PartitionKind is irrelevant to `iv_is_partitioned` (it just
    /// checks for any `Partition(_)`), but `Workers` is the lowest-
    /// dependency variant for synthetic fixtures.
    fn loop_partition_workers(iv: &str) -> ResolvedLoopDirective {
        ResolvedLoopDirective {
            var: iv.to_string(),
            options: vec![ResolvedLoopOption::Partition(
                crate::sched::PartitionKind::Workers,
            )],
            var_span: None,
        }
    }

    fn ir_mod(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Mod, Box::new(l), Box::new(r))
    }

    /// 16-jacobi/distributed shape: `grid[(t + 3) % 4][y]` inside
    /// `for t { for y { ... } }` with `y` partitioned. The failing
    /// index is at axis 0 (the Mod wrap) and references only `t`.
    /// Under (B') this stays advisory; under the pre-cycle-209 (B)
    /// rule it would have been fatal.
    #[test]
    fn bprime_modwrap_nonpartitioned_axis_stays_advisory() {
        // for t : 0..5 { for y : 1..7 { out[t][y] <-- K(grid[(t+3)%4][y]) } }
        let stmts = vec![IrStmt::For {
            var: "t".to_string(),
            lo: ir_int(0),
            hi: ir_int(5),
            body: vec![IrStmt::For {
                var: "y".to_string(),
                lo: ir_int(1),
                hi: ir_int(7),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("t"), ir_id("y")]),
                    rhs: ir_call(
                        "K",
                        vec![data_ref(
                            "grid",
                            vec![ir_mod(ir_add(ir_id("t"), ir_int(3)), ir_int(4)), ir_id("y")],
                        )],
                    ),
                }],
            }],
        }];
        let mut linked = build_linked(stmts, vec![5, 8]);
        // Partition the y axis, NOT the t axis.
        linked
            .sched
            .loops
            .insert("y".to_string(), loop_partition_workers("y"));
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let (acfg, advisory) = apply_halo_inference_partition_aware(&linked, acfg)
            .expect("(B') must classify this advisory — y partitioned but failing axis is t");
        // Exactly one advisory error (the NonAffineIndex on axis 0).
        assert_eq!(
            advisory.len(),
            1,
            "expected one advisory error, got: {advisory:?}"
        );
        assert!(
            matches!(
                &advisory[0],
                HaloInferenceError::NonAffineIndex { ax_idx: 0, .. }
            ),
            "advisory[0] = {:?}",
            advisory[0]
        );
        // The y-axis read at index 1 of `grid` is affine (bare `y`,
        // halo 0) — halo_widths[K][y] should be recorded with width
        // 0. The t-axis carried no halo because the Mod wrap was
        // unfoldable and the iv is non-partitioned.
        let k_id = *acfg.name_kernels.get("K").unwrap();
        let y_iv = *acfg.name_iter_vars.get("y").unwrap();
        assert_eq!(
            acfg.halo_widths
                .get(&k_id)
                .and_then(|m| m.get(&y_iv))
                .copied(),
            Some(0)
        );
    }

    /// Complement of the above: the partitioned iv IS `t` (the axis
    /// the Mod wrap is on). Now (B') must classify fatal — the
    /// partition impact on axis 0 is real, and a halo cannot be
    /// inferred.
    #[test]
    fn bprime_modwrap_partitioned_axis_stays_fatal() {
        let stmts = vec![IrStmt::For {
            var: "t".to_string(),
            lo: ir_int(0),
            hi: ir_int(5),
            body: vec![IrStmt::For {
                var: "y".to_string(),
                lo: ir_int(0),
                hi: ir_int(8),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("t"), ir_id("y")]),
                    rhs: ir_call(
                        "K",
                        vec![data_ref(
                            "grid",
                            vec![ir_mod(ir_add(ir_id("t"), ir_int(3)), ir_int(4)), ir_id("y")],
                        )],
                    ),
                }],
            }],
        }];
        let mut linked = build_linked(stmts, vec![5, 8]);
        // Partition the t axis (the WRAP axis).
        linked
            .sched
            .loops
            .insert("t".to_string(), loop_partition_workers("t"));
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
            "(B') must classify fatal — t is partitioned and the failing index references t",
        );
        assert!(
            matches!(err, HaloInferenceError::NonAffineIndex { ax_idx: 0, .. }),
            "expected NonAffineIndex on axis 0, got: {err:?}"
        );
    }

    /// `StridedAccessNotSupported` on the partitioned iv must still
    /// fire fatal. Symmetric to the Mod-wrap fatal case but on the
    /// strided variant, exercising a different error-push site.
    #[test]
    fn bprime_strided_on_partitioned_iv_stays_fatal() {
        // for y : 0..15 { out[y] <-- K(grid[2*y]) } with y partitioned.
        let stmts = vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(15),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref("grid", vec![ir_mul(ir_int(2), ir_id("y"))])],
                ),
            }],
        }];
        let mut linked = build_linked(stmts, vec![32]);
        linked
            .sched
            .loops
            .insert("y".to_string(), loop_partition_workers("y"));
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
            "(B') must classify fatal — y is partitioned and the failing index references y",
        );
        assert!(
            matches!(err, HaloInferenceError::StridedAccessNotSupported { .. }),
            "expected StridedAccessNotSupported, got: {err:?}"
        );
    }

    /// 11-game-of-life regression pin: Mod-wrap on iv with NO
    /// partition directive anywhere on any iv. Stays advisory under
    /// both (B) and (B'). Confirms the cycle-209 refinement did not
    /// silently regress the canonical preserved case.
    #[test]
    fn bprime_modwrap_no_partition_at_all_stays_advisory() {
        let stmts = vec![IrStmt::For {
            var: "t".to_string(),
            lo: ir_int(0),
            hi: ir_int(5),
            body: vec![IrStmt::For {
                var: "i".to_string(),
                lo: ir_int(0),
                hi: ir_int(32),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("t"), ir_id("i")]),
                    rhs: ir_call(
                        "K",
                        vec![data_ref(
                            "grid",
                            vec![ir_mod(ir_add(ir_id("t"), ir_int(4)), ir_int(5)), ir_id("i")],
                        )],
                    ),
                }],
            }],
        }];
        let linked = build_linked(stmts, vec![5, 32]);
        // No partition directives anywhere.
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let (_acfg, advisory) = apply_halo_inference_partition_aware(&linked, acfg)
            .expect("no partition anywhere ⇒ advisory");
        assert_eq!(
            advisory.len(),
            1,
            "expected one advisory error (the Mod-wrap NonAffineIndex), got: {advisory:?}"
        );
        assert!(matches!(
            advisory[0],
            HaloInferenceError::NonAffineIndex { ax_idx: 0, .. }
        ));
    }
}
