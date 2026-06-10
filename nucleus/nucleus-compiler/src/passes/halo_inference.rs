//! Halo region inference from kernel access patterns — TASK-0260 Stage 1.
//!
//! For each kernel invocation inside a `for` nest, scan its argument
//! [`crate::algo::IrExpr::DataRef`] indices to recover the per-axis halo width N: the
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
//!    `per_iv.entry(iv).or_insert(0)` in `halo_inference/walker.rs`,
//!    its home since the TASK-0460 split) would let `== 0`
//!    narrative pins pass vacuously. Judged unlikely; preserving the
//!    contract robustness is worth the accepted vacuous-pass arm. A
//!    future change to require explicit-0 (Option C) would need a
//!    contract-pin test of the new invariant plus consumer-side
//!    `unwrap_or(0)` → explicit `expect(...)` migration across the
//!    test suite.
//!
//!    **TASK-0307 cycle-123 structural sentinel**: the in-module test
//!    `no_halo_bare_iv` (search for `fn no_halo_bare_iv` in
//!    `halo_inference/tests/stencil.rs`, its home since the TASK-0460
//!    split) carries a `copied() == Some(0)` structural assertion
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
//!   walker raises, the fatality predicate is one of three rules
//!   depending on the error variant:
//!
//!   * For `DataDependentStride` (a data-dependent READ index) —
//!     TASK-0373 / TASK-0384 split on the `is_scatter_rmw` flag:
//!       - PURE GATHER (affine LHS, e.g.
//!         `y[i] <-- ...x[col_idx[i][k]]`): advisory. The transfer
//!         layer broadcasts the WHOLE gathered array to every worker
//!         (the array's data-dependent dim is marked OPAQUE in
//!         `transfer_inject::record_access_per_dim`), so any runtime
//!         index loaded is in range.
//!       - SCATTER RMW (data-dependent LHS, e.g.
//!         `histogram[input[i]] <-- inc(histogram[input[i]])`): the
//!         RHS read reaches this pass. ADVISORY iff the scatter target
//!         replicates whole-array (no partitioned iv affinely indexes
//!         it — the INPUT-INDEX partition), in which case each worker
//!         scatters its slice into a private full-width partial and the
//!         host element-wise-sums the partials (TASK-0343 combine,
//!         TASK-0384 admit). FATAL under partition iff a partitioned iv
//!         affinely indexes the scatter target (a BIN partition —
//!         band-partitioned target, replicate-per-worker unsound). See
//!         `scatter_target_replicates_whole_array`.
//!   * For `UnknownKernelInCall` — where the iv set the failing
//!     index actually depends on is not bounded by the lexical iv
//!     walk — the rule consults the enclosing-loop scope: if ANY iv
//!     in that scope carries a
//!     [`crate::sched::ResolvedLoopOption::Partition`] directive,
//!     the error is FATAL (the original cycle-95 (B) rule). This is a
//!     fail-closed diagnostic for inconsistently-constructed IR.
//!   * For all other variants (`NonAffineIndex`,
//!     `StridedAccessNotSupported`, `MultipleIterVarsInIndex`,
//!     `UnknownLoopVar`) — where the failing index expression's iv
//!     set is recoverable lexically via `collect_iter_var_refs` —
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
//!   rejected as [`HaloInferenceError::StridedAccessNotSupported`]. Strided reads have
//!   well-defined halo semantics (`|b|` is still the offset, but the
//!   distributed transfer pattern differs from the contiguous-strip case
//!   stencils need) — out of scope for Stage 1.
//! - **Single iter-var per index.** `grid[y + x][x]` (or any index whose
//!   tree contains TWO different enclosing iter-var Idents) is rejected
//!   as [`HaloInferenceError::MultipleIterVarsInIndex`] because the offset `b` would not be a
//!   compile-time constant.
//! - **No DataRef inside an index.** `grid[lookup[y]]` (an index that
//!   itself reads data) is rejected as [`HaloInferenceError::DataDependentStride`]. PRD §13.
//! - **No Call inside an index.** `grid[f(y)]` is rejected as
//!   [`HaloInferenceError::DataDependentStride`] (a runtime call cannot be folded; same harm
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
//! contract precedent as `xfer_facts` from TASK-0233/TASK-0455.08 and
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
use crate::event::{IterVar, KernelId};
use crate::link::LinkedIR;

// The inference walk + the partition-policy fatality predicate live in
// the carved sibling submodules (TASK-0460). Their parent-facing entries
// are `pub(super)`; the three public entry points below dispatch into
// them.
use self::partition_policy::error_is_fatal_under_partition;
use self::walker::{commit_halo_widths, infer_halo_widths};

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
        /// [`crate::algo::IndexedRef`]).
        ref_name: String,
        /// 0-based axis index inside the [`crate::algo::IndexedRef`] (the position of
        /// the offending index expression in `indices`).
        ax_idx: usize,
        /// TASK-0373: `true` iff this data-dependent READ sits in a
        /// firing whose `<--` LHS is itself a data-dependent WRITE
        /// (a scatter read-modify-write, e.g.
        /// `histogram[input[i]] <-- inc(histogram[input[i]])`). A pure
        /// gather (affine LHS such as `y[i]`) sets this `false`.
        ///
        /// Drives the fatality split in
        /// [`partition_policy::error_is_fatal_under_partition`]: a pure gather READ is
        /// served by whole-array broadcast (advisory); a scatter RMW
        /// READ under partition is ADVISORY when the scatter target
        /// replicates whole-array (the INPUT-INDEX partition —
        /// replicate-per-worker + element-wise-sum combine, TASK-0384),
        /// and FATAL only when a partitioned iv affinely indexes the
        /// scatter target (a BIN partition —
        /// `scatter_target_replicates_whole_array` returns false). This
        /// field does NOT affect [`Display`] (the message is the same
        /// READ diagnostic) and is `false` on the single-worker /
        /// unpartitioned paths where it is never consulted.
        ///
        /// [`Display`]: std::fmt::Display
        is_scatter_rmw: bool,
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
        /// 0-based axis index inside the [`crate::algo::IndexedRef`].
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
        /// 0-based axis index inside the [`crate::algo::IndexedRef`].
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
        /// 0-based axis index inside the [`crate::algo::IndexedRef`].
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
                // The scatter-RMW bit does not change the diagnostic
                // text — the message describes the READ index shape,
                // which is identical for gather and scatter reads.
                is_scatter_rmw: _,
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
/// carries a [`crate::sched::ResolvedLoopOption::Partition`] directive in the
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
/// Per-variant rule split (see `classify_index` for the iv-set
/// population at each push site):
///
/// - [`HaloInferenceError::DataDependentStride`] — index is itself
///   a `DataRef`/`Call`; the iv set is empty (the lexical walker
///   does not descend into data-dependent addresses). TASK-0373
///   splits on the `is_scatter_rmw` flag stamped at the push site:
///     - PURE GATHER (affine LHS, e.g. `y[i] <-- ...x[col_idx[i][k]]`):
///       advisory. The transfer layer serves the read by broadcasting
///       the WHOLE gathered array to every worker (the array's
///       data-dependent dim is marked OPAQUE in
///       `transfer_inject::record_access_per_dim`); any runtime index
///       loaded is in range.
///     - SCATTER RMW (data-dependent LHS, e.g.
///       `histogram[input[i]] <-- inc(histogram[input[i]])`): ADVISORY
///       under partition iff the scatter target replicates whole-array
///       (the INPUT-INDEX partition — TASK-0384 admits it via the
///       TASK-0343 replicate-per-worker + element-wise-sum combine);
///       FATAL iff a partitioned iv affinely indexes the scatter target
///       (a BIN partition, band-partitioned target). See
///       `error_is_fatal_under_partition` /
///       `scatter_target_replicates_whole_array` for the soundness
///       coupling.
/// - All other variants ([`NonAffineIndex`],
///   [`StridedAccessNotSupported`], [`MultipleIterVarsInIndex`],
///   [`UnknownLoopVar`]) — the iv set is populated by
///   `collect_iter_var_refs` at the error-push site. Fatal iff
///   AT LEAST ONE iv in the failing-index set is partitioned;
///   advisory otherwise. The 11-game-of-life regression pin
///   ("no partition + Mod wrap stays advisory") is preserved: the
///   pipelined/naive schedules carry zero `partition=`, so no iv
///   in the failing-index set (= `{t}`) returns `true` from
///   `iv_is_partitioned`.
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
/// Halo-widths map: kernel → iter-var → halo width.
type HaloMap = BTreeMap<KernelId, BTreeMap<IterVar, u64>>;
/// Typed error paired with TWO iv-name vectors captured at the
/// error-push site:
///
/// - The enclosing-loop scope (outermost-first iter-var names) — the
///   pre-TASK-0341.02.02.01 fatality input. Still consulted by TWO
///   arms (see [`partition_policy::error_is_fatal_under_partition`]): the
///   `UnknownKernelInCall` fail-closed diagnostic, AND the
///   `DataDependentStride` *scatter-RMW* sub-case. TASK-0373 narrowed
///   — did NOT retire — scope's use for `DataDependentStride`: a PURE
///   gather (`is_scatter_rmw == false`) is advisory regardless of
///   partition state (the gathered array is whole-array broadcast). A
///   scatter read-modify-write (`is_scatter_rmw == true`) consults
///   `scope` for the "is any enclosing iv partitioned?" half of the
///   test; TASK-0384 then adds the soundness half
///   (`scatter_target_replicates_whole_array`): the scatter is FATAL
///   only when partitioned AND the target is band-partitioned (a BIN
///   partition), and ADVISORY for the INPUT-INDEX partition (the target
///   replicates whole-array). The field is also part of the
///   captured-at-push-site diagnostic record.
/// - The ivs the FAILING INDEX EXPRESSION actually references —
///   collected via [`walker::collect_iter_var_refs`]. Empty when the index
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

// --------------------------------------------------------------------
// Submodules (TASK-0460 content-preserving mega-file split)
// --------------------------------------------------------------------

mod partition_policy;
mod walker;

#[cfg(test)]
mod tests;
