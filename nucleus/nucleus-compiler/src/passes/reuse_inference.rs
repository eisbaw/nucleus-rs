//! Reuse loop-option inference — TASK-0261 Stage 1.
//!
//! For each `for V : reuse;` loop, scan the loop's body for kernel-arg
//! [`IrExpr::DataRef`] indices and recover the set of constant offsets
//! `{b1, b2, ...}` along the iter-var V for each `(DataId, axis)`
//! reachable inside the loop. When that set forms a CONTIGUOUS range
//! `[min..=max]`, the reuse pattern is APPLICABLE: a future codegen
//! consumer (Stage 2 = TASK-0265) can emit a delay-line / circular
//! buffer of length `max - min + 1` that holds the recently-read /
//! recently-computed slice values so each one is loaded exactly ONCE.
//!
//! ## Why this pass exists
//!
//! PRD §6.3.3 line 513: "`reuse` — Identify and reuse loop-carried
//! slices (the 2013 gap)." Without this pass, a `loop V : reuse;`
//! directive is **parsed** ([`crate::sched::parser`]) and **resolved**
//! ([`crate::sched::lower`]: `ResolvedLoopOption::Reuse`) but no
//! downstream consumer reads it. The 2013 gap (Halide's reuse hint
//! syntax with no general-stencil reuse codegen) is still open. This
//! pass is Stage 1: it lands the inference + sidecar persistence;
//! Stage 2 (TASK-0265) is where the backend walker emits the
//! delay-line code.
//!
//! ## Stage decomposition (TASK-0261 family)
//!
//! - **Stage 1** (this pass): walk every `reuse`-tagged loop in
//!   `linked.algo.stmts`, infer the per-(iv, data, axis) offset-set
//!   contiguous-range, persist into the ACFG sidecar
//!   ([`crate::acfg::ACFG::reuse_widths`]). Pure + deterministic. No
//!   backend behaviour change. Same observational-inertness as
//!   TASK-0260 Stage 1 (`halo_inference`).
//! - **Stage 2** (TASK-0265): backend walker (multi_worker_walker +
//!   each per-backend Plan) consumes `reuse_widths` at the
//!   `Event::Loop` emit site and synthesises the circular-buffer
//!   bookkeeping (load-once, read-many) for each `(DataId, axis)` slot
//!   the sidecar names. Coordinates with TASK-0263 (halo Stage 2) —
//!   both consume the same emit site, but they are ORTHOGONAL feature
//!   toggles (halo widens transfer tiles; reuse rewrites read patterns
//!   inside a tile).
//!
//! ## Affine-stride detector
//!
//! Identical contract to [`crate::passes::halo_inference`] — both
//! passes share [`crate::passes::common::affine_decompose`] for the
//! `iv + b` recognition. PRD §13 ("reuse / halo on data-dependent
//! strides is rejected at compile time") is enforced here for the
//! reuse side; halo enforces it for the halo side.
//!
//! Accepted index shapes (per [`crate::passes::common`] module docs):
//!
//! - `iv + b` / `b + iv` / `iv - b` (coefficient `+1`, offset = `b`).
//! - `iv` alone → offset `0`.
//! - Pure constant → no iv-contribution → ignored for this iv.
//!
//! REJECTED shapes:
//!
//! - **Strided / negated iv** (coeff != 1):
//!   [`ReuseInferenceError::StridedAccessNotSupported`]. Strided
//!   reuse is well-defined but Stage 1 is conservative; deferred.
//! - **Multiple iter-vars in one index** (`grid[y + x]`):
//!   [`ReuseInferenceError::MultipleIterVarsInIndex`].
//! - **DataRef / Call inside the index** (`grid[lookup[i]]`,
//!   `grid[f(i)]`): [`ReuseInferenceError::DataDependentStride`].
//! - **Non-affine arithmetic mentioning iv** (e.g. `grid[(i + N) % M]`,
//!   `grid[i * i]`): [`ReuseInferenceError::NonAffineIndex`].
//! - **Non-contiguous offset set** (e.g. `{-3, 0, +5}`):
//!   [`ReuseInferenceError::NonContiguousOffsets`]. The reuse savings
//!   on a sparse window are negligible (most slots would never be
//!   reused); we reject rather than emit a half-useful slot. The user
//!   restructures the algorithm or drops the `reuse` hint.
//!
//! ## Strict vs advisory entry points
//!
//! Mirrors [`crate::passes::halo_inference`]:
//!
//! - [`apply_reuse_inference`] is the strict variant — first error
//!   returned as `Err`.
//! - [`apply_reuse_inference_advisory`] is the lenient variant — every
//!   error collected, partial sidecar committed for unaffected loops.
//!
//! Stage 1 driver policy: lenient. No downstream pass yet reads
//! `reuse_widths` (Stage 2 / TASK-0265 is the consumer), so a typed
//! rejection here is purely advisory until that consumer wiring lands.
//! The driver emits a `nuc_trace!` for each advisory error (visible
//! under `NUC_TRACE=1`). When Stage 2 lands, the driver will either
//! switch to the strict variant or check the errors against the
//! `partition` policy before swallowing them (same forward path as
//! halo Stage 2 / TASK-0263).
//!
//! ## Honest limitations (first cut)
//!
//! - **Coefficient must be +1.** Same as halo — `-iv`, `2*iv`,
//!   `iv * 2 + 1` are rejected. Strided reuse is feasible but adds
//!   a (stride, offsets) shape the Stage 2 emit site doesn't yet need.
//! - **Single iter-var per index.** A multi-axis index `grid[i + j]`
//!   has no single offset along i; rejected.
//! - **No DataRef / Call inside the index.** PRD §13.
//! - **Contiguous-offsets-only.** Non-contiguous sets are rejected.
//! - **One reuse slot per `(IterVar, DataId, axis)`.** Two different
//!   axes of the same data referenced affinely under the same loop iv
//!   produce two entries in the inner-axis map; they coexist. Same data
//!   referenced with reuse on TWO different enclosing loop ivs (nested
//!   reuse) — each outer `IterVar` key gets its own entry, independent.
//! - **`reuse` on a loop body with NO iv-bearing DataRefs.** The pass
//!   records no entry. A degenerate `reuse` on a body that only reads
//!   `grid[i]` (one offset, length 1) is ALSO not recorded — a length-1
//!   delay-line is a no-op. The contract is "no entry ⇔ no reuse code
//!   emit needed".
//!
//! ## Sidecar contract
//!
//! Writes [`crate::acfg::ACFG::reuse_widths`] (mirrored verbatim onto
//! [`crate::sidecar::NameSidecar::reuse_widths`]):
//!
//! ```text
//! BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64 /* axis */, ReuseSlot>>>
//! ```
//!
//! where [`ReuseSlot`] = `{ min_offset: i64, length: u64 }`.
//!
//! Why this shape (vs the brief's `BTreeMap<IterVar, BTreeMap<DataName,
//! ReuseSlot>>`):
//!
//! - Keying by `DataId` mirrors halo's `KernelId` key choice (both
//!   `u64` newtypes) — consistent with the sidecar's id-not-name
//!   convention. Backends reverse-lookup via `name_data` if a name is
//!   needed for diagnostics.
//! - The axis is part of the inner key, not the slot value, because a
//!   single (iv, data) pair CAN touch multiple axes when the same data
//!   appears in DataRefs of different shape (one DataRef on axis 0,
//!   another on axis 1 in the same body). The inner-most map makes the
//!   per-axis lookup explicit.
//! - All four levels are `BTreeMap`s of `serde(transparent)` `u64`
//!   newtypes / a `u64` axis index; the JSON wire form round-trips
//!   without the tuple-key trap (TASK-0233 precedent; halo's nested
//!   shape rationale carries forward verbatim).
//!
//! The field is `#[serde(default)]` so an old wire payload (no field)
//! deserialises as an empty map — same additive contract as
//! `halo_widths` (TASK-0260), `transfer_buffer_for_seq` (TASK-0233),
//! and `partition_worker_ranges` (TASK-0212).
//!
//! ## Independence from partition policy
//!
//! `reuse_widths` is independent of partition. A reuse hint applies to
//! each per-tile loop body whether the loop is partitioned across
//! workers or not — the delay-line lives within ONE worker's tile.
//! Partitioning just bounds which iv-range each worker covers; the
//! reuse codegen is unaffected.
//!
//! ## Determinism
//!
//! - `BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>>` —
//!   three nested ordered maps; iteration is in numeric order at every
//!   level.
//! - `linked.sched.loops` and `linked.algo.stmts` are themselves
//!   deterministic (BTreeMap by var name, Vec in source order).
//! - Offset-set aggregation uses `BTreeSet<i64>`.
//! - No `HashMap` / `HashSet` on any path that affects sidecar bytes.

use std::collections::{BTreeMap, BTreeSet};

use crate::acfg::ACFG;
use crate::algo::{IndexedRef, IrExpr, IrStmt};
use crate::event::{DataId, IterVar};
use crate::link::LinkedIR;
use crate::passes::common::affine_decompose;
use crate::sched::{ResolvedLoopOption, SchedIR};

/// Type alias for the reuse-widths sidecar map.
///
/// `IterVar -> DataId -> axis_idx -> ReuseSlot`. Hand-typed in every
/// function signature would re-trip the `clippy::type_complexity` lint
/// (the type has three nested generics + a non-builtin leaf); the
/// alias is a single point of definition that mirrors
/// [`crate::acfg::ACFG::reuse_widths`] and
/// [`crate::sidecar::NameSidecar::reuse_widths`].
pub(crate) type ReuseWidthsMap = BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>>;

// --------------------------------------------------------------------
// Sidecar payload
// --------------------------------------------------------------------

/// One reuse-slot inference result: a delay line of `length`
/// elements indexed by an offset in `[min_offset .. min_offset + length)`
/// from the current `iv` value.
///
/// Populated by [`apply_reuse_inference`] (TASK-0261 Stage 1).
/// Consumed by Stage 2 (TASK-0265): the backend walker emits a
/// circular buffer of `length` slots and rewrites every `grid[iv + b]`
/// read inside the loop body to `buf[(iv + b - min_offset) % length]`.
///
/// The field is `#[serde(default)]` on the enclosing map so an older
/// wire payload (no slot) deserialises as an absent entry — additive
/// shape preserves backward compatibility (TASK-0233 precedent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReuseSlot {
    /// The minimum offset observed across all `iv + b` reads of this
    /// `(iv, data, axis)` triple. Signed because indices below `iv`
    /// (`iv - 1` etc.) are common.
    pub min_offset: i64,
    /// The number of distinct offsets in the slot, i.e.
    /// `max_offset - min_offset + 1`. Always `> 1` — the degenerate
    /// length-1 case (the only offset is `0`, a single `grid[iv]` read)
    /// is dropped by [`apply_reuse_inference`] because a 1-slot delay
    /// line is a no-op.
    pub length: u64,
}

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Typed errors produced by [`apply_reuse_inference`].
///
/// Every variant names a single semantic violation and carries the
/// payload a diagnostic message needs. Mirrors the shape of
/// [`crate::passes::halo_inference::HaloInferenceError`] — the two
/// passes share the affine-stride detector and so share the rejection
/// taxonomy (with `NonContiguousOffsets` unique to reuse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReuseInferenceError {
    /// A DataRef index inside a `reuse`-tagged loop body is
    /// data-dependent (contains a DataRef or Call). PRD §13 rejects
    /// data-dependent strides at compile time.
    DataDependentStride {
        /// Iter-var of the enclosing `reuse` loop.
        iter_var: String,
        /// Data symbol name being indexed.
        ref_name: String,
        /// 0-based axis position inside the [`IndexedRef`].
        ax_idx: usize,
    },
    /// A DataRef index inside a `reuse`-tagged loop body is affine but
    /// the iter-var coefficient is not `+1` (e.g. `2 * iv + 1`,
    /// `iv * 3`, `-iv`). Strided reuse is well-defined but out-of-scope
    /// for Stage 1.
    StridedAccessNotSupported {
        iter_var: String,
        ref_name: String,
        ax_idx: usize,
        /// The recovered stride coefficient (e.g. `2` for `2*iv`, `-1`
        /// for `-iv`).
        coefficient: i64,
    },
    /// A DataRef index inside a `reuse`-tagged loop body references
    /// TWO OR MORE enclosing iter-vars (e.g. `grid[y + x]`). The reuse
    /// pattern is a per-iv property; multi-iv indices have no single
    /// affine `(coeff, offset)` decomposition.
    MultipleIterVarsInIndex {
        iter_var: String,
        ref_name: String,
        ax_idx: usize,
        /// All iter-var names found in the index, deterministic order.
        iter_vars: Vec<String>,
    },
    /// A DataRef index inside a `reuse`-tagged loop body mentions the
    /// iv but is not in the recognised affine shape (e.g. `(iv + N) %
    /// M` — Mod is non-linear; or `iv * iv`). Distinct from
    /// [`Self::DataDependentStride`] because no DataRef / Call is
    /// involved — pure arithmetic we cannot classify.
    NonAffineIndex {
        iter_var: String,
        ref_name: String,
        ax_idx: usize,
    },
    /// The set of offsets recovered for a `(iv, data, axis)` triple
    /// does NOT form a contiguous integer range. Example:
    /// `{-3, 0, +5}` has length-9 hull but only 3 actually-used slots —
    /// a delay line would mostly hold dead entries. The user must
    /// either drop the `reuse` hint on this loop or restructure the
    /// access pattern to be contiguous.
    NonContiguousOffsets {
        /// Iter-var of the enclosing `reuse` loop.
        iter_var: String,
        /// Data symbol name (resolved from `name_data`).
        ref_name: String,
        /// 0-based axis position inside the [`IndexedRef`].
        ax_idx: u64,
        /// All offsets observed, sorted ascending — for diagnostic
        /// clarity ("you wrote {-3, 0, +5} but [-3..=+5] would need 9
        /// slots").
        offsets: Vec<i64>,
    },
    /// A `reuse` directive names an iter-var that the linker did not
    /// install into `ACFG::name_iter_vars`. Cannot happen for
    /// link-valid IR (the link step inserts every `for var` into the
    /// table); the variant exists so an inconsistently-constructed
    /// `(LinkedIR, ACFG)` pair fails closed with a typed error rather
    /// than panicking — same invariant-guard pattern as
    /// [`crate::passes::halo_inference::HaloInferenceError::UnknownIterVarInScope`].
    UnknownLoopVar { var: String },
    /// A DataRef inside a `reuse`-tagged loop body names a data symbol
    /// the ACFG `name_data` table does not contain. Cannot happen for
    /// link-valid IR (the link step rejects unknown data names);
    /// defensive parity with the partition-rows / halo invariant guards.
    UnknownDataInRef { ref_name: String },
}

impl std::fmt::Display for ReuseInferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReuseInferenceError::DataDependentStride {
                iter_var,
                ref_name,
                ax_idx,
            } => write!(
                f,
                "reuse loop `{iter_var}`: index of `{ref_name}` at axis {ax_idx} is \
                 data-dependent (a DataRef or Call inside the index expression). PRD §13: \
                 reuse rejects data-dependent strides at compile time. Replace with an \
                 affine `iv + b` index (b constant) or drop the `reuse` hint."
            ),
            ReuseInferenceError::StridedAccessNotSupported {
                iter_var,
                ref_name,
                ax_idx,
                coefficient,
            } => write!(
                f,
                "reuse loop `{iter_var}`: index of `{ref_name}` at axis {ax_idx} is strided \
                 (recovered coefficient {coefficient}). TASK-0261 first cut accepts only \
                 coefficient +1 (i.e. `iv + b`); strided / reversed reads are deferred."
            ),
            ReuseInferenceError::MultipleIterVarsInIndex {
                iter_var,
                ref_name,
                ax_idx,
                iter_vars,
            } => write!(
                f,
                "reuse loop `{iter_var}`: index of `{ref_name}` at axis {ax_idx} references \
                 multiple enclosing iter-vars ({}); reuse inference needs the offset `b` in \
                 `iv + b` to be a compile-time constant in this one iv.",
                iter_vars.join(", ")
            ),
            ReuseInferenceError::NonAffineIndex {
                iter_var,
                ref_name,
                ax_idx,
            } => write!(
                f,
                "reuse loop `{iter_var}`: index of `{ref_name}` at axis {ax_idx} is non-affine \
                 (e.g. `iv % M`, `iv * iv`). Reuse inference accepts only `iv + b` (b \
                 constant) — see PRD §13 / TASK-0261 module docs."
            ),
            ReuseInferenceError::NonContiguousOffsets {
                iter_var,
                ref_name,
                ax_idx,
                offsets,
            } => {
                let span = match (offsets.first(), offsets.last()) {
                    (Some(&lo), Some(&hi)) => {
                        format!("[{lo}..={hi}] would need {} slots", hi - lo + 1)
                    }
                    _ => "(empty)".to_string(),
                };
                write!(
                    f,
                    "reuse loop `{iter_var}`: offsets of `{ref_name}` at axis {ax_idx} are \
                     non-contiguous ({offsets:?}); {span} — reuse savings on a sparse window \
                     are negligible. Drop the `reuse` hint or restructure the access pattern."
                )
            }
            ReuseInferenceError::UnknownLoopVar { var } => write!(
                f,
                "reuse inference: loop directive names `{var}` but the ACFG has no such \
                 iter-var id (link-pass invariant violation)"
            ),
            ReuseInferenceError::UnknownDataInRef { ref_name } => write!(
                f,
                "reuse inference: DataRef `{ref_name}` is not in `ACFG::name_data` \
                 (link-pass invariant violation)"
            ),
        }
    }
}

impl std::error::Error for ReuseInferenceError {}

// --------------------------------------------------------------------
// Entry points
// --------------------------------------------------------------------

/// Walk `linked.algo.stmts`, find every loop whose schedule directive
/// carries [`ResolvedLoopOption::Reuse`], and populate
/// [`crate::acfg::ACFG::reuse_widths`] with the per-(iv, data, axis)
/// delay-line slot recovered from the body's DataRef indices.
///
/// Strict variant: returns `Err` on the first
/// [`ReuseInferenceError`] raised. Used by tests + direct callers
/// that want fail-fast semantics.
///
/// Pure: input ACFG is consumed, a new one with the sidecar populated
/// is returned. The tree itself is forwarded unchanged.
///
/// On any error, no partial sidecar is committed — the function
/// validates every reuse loop up front before mutating the sidecar.
pub fn apply_reuse_inference(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, ReuseInferenceError> {
    let (widths, errors) = infer_reuse_widths(linked, &acfg);
    if let Some(e) = errors.into_iter().next() {
        return Err(e);
    }
    Ok(commit_reuse_widths(acfg, widths))
}

/// Lenient variant of [`apply_reuse_inference`]: walks every
/// `reuse`-tagged loop, records every recognisable (iv, data, axis)
/// slot, and returns every typed error it would have raised so the
/// caller decides whether each shape is fatal.
///
/// ## Stage 1 driver policy (TASK-0261)
///
/// The driver consumes the lenient variant in Stage 1: no downstream
/// pass yet reads `reuse_widths`, so a non-affine index inside a
/// `reuse`-tagged loop is only advisory until Stage 2 (TASK-0265,
/// backend walker delay-line emit) makes the consumer concrete. The
/// driver emits a `nuc_trace!` line per returned error (visible under
/// `NUC_TRACE=1`); the e2e baseline stays byte-identical because no
/// cell's emitted bytes change.
///
/// Stage 2 will either (a) move the driver back to the strict variant
/// once the consumer wiring is in place, or (b) check the errors vec
/// against whether a `reuse` directive is actually present on a given
/// loop and only treat them as fatal when reuse is asked for.
pub fn apply_reuse_inference_advisory(
    linked: &LinkedIR,
    acfg: ACFG,
) -> (ACFG, Vec<ReuseInferenceError>) {
    let (widths, errors) = infer_reuse_widths(linked, &acfg);
    let acfg = commit_reuse_widths(acfg, widths);
    (acfg, errors)
}

// --------------------------------------------------------------------
// Inference core
// --------------------------------------------------------------------

/// Core inference. For each `reuse`-tagged iv, walks `linked.algo.stmts`
/// to find the matching `for` loop and inspects its body for DataRef
/// indices contributing to a delay-line slot for THAT iv only.
///
/// We do ONE walk per reuse iv (rather than one walk threading multiple
/// accumulators) so that nested reuse loops are handled by independent
/// passes — each pass sees only DataRefs that mention its target iv.
/// This is the simplest correct shape: the walks are independent, the
/// per-iv accumulators don't entangle, and the per-iv source-order of
/// errors is preserved (a single fused walk would have to interleave
/// errors from multiple ivs in a non-obvious order).
///
/// The walker is COLLECTING (does not short-circuit on the first
/// error) so the lenient variant can record every recognisable fact
/// even when some indices are non-affine. The strict variant short-
/// circuits on the first error at the entry point.
fn infer_reuse_widths(
    linked: &LinkedIR,
    acfg: &ACFG,
) -> (ReuseWidthsMap, Vec<ReuseInferenceError>) {
    let mut widths: ReuseWidthsMap = BTreeMap::new();
    let mut errors: Vec<ReuseInferenceError> = Vec::new();

    let reuse_var_names: BTreeSet<&str> = collect_reuse_var_names(&linked.sched);
    if reuse_var_names.is_empty() {
        // No reuse directives → fast-exit.
        return (widths, errors);
    }

    let ctx = WalkCtx {
        name_data: &acfg.name_data,
        name_iter_vars: &acfg.name_iter_vars,
        consts: &linked.algo.consts,
    };

    // One walk per reuse iv. The walk descends `linked.algo.stmts`
    // looking for a `for var == iv_name` and classifies every DataRef
    // in that subtree against that single iv.
    //
    // Iteration order is BTreeSet (alphabetical on the string), so
    // identical inputs produce identical error orderings.
    for iv_name in &reuse_var_names {
        // Defensive: iv_name must be in name_iter_vars.
        let iv_id = match ctx.name_iter_vars.get(*iv_name) {
            Some(id) => *id,
            None => {
                errors.push(ReuseInferenceError::UnknownLoopVar {
                    var: (*iv_name).to_string(),
                });
                continue;
            }
        };
        let mut accum: ReuseAccum = BTreeMap::new();
        let scope: Vec<String> = Vec::new();
        walk_for_iv(
            &linked.algo.stmts,
            &scope,
            iv_name,
            /*inside=*/ false,
            &ctx,
            &mut accum,
            &mut errors,
        );
        finalise_accum(iv_name, iv_id, accum, &ctx, &mut widths, &mut errors);
    }

    (widths, errors)
}

/// Read-only joining context threaded through the recursive walker.
struct WalkCtx<'a> {
    name_data: &'a BTreeMap<String, DataId>,
    name_iter_vars: &'a BTreeMap<String, IterVar>,
    consts: &'a BTreeMap<String, crate::algo::ResolvedConst>,
}

/// Per-iv accumulator: maps `(DataId, axis_idx) -> set of offsets`.
type ReuseAccum = BTreeMap<DataId, BTreeMap<u64, BTreeSet<i64>>>;

/// Walk `stmts` looking for the `for` loop with var name == `iv_name`,
/// and once entered, classify every DataRef in its subtree.
///
/// `inside` tracks whether we are currently inside (i.e. lexically
/// under) the target reuse `for` loop. While outside, the walker
/// recurses but does NOT classify DataRefs (they don't apply to this
/// iv's slot). Once inside, every DataRef encountered (including those
/// in nested non-reuse OR reuse loops — nesting doesn't matter for this
/// per-iv walk) is classified.
fn walk_for_iv(
    stmts: &[IrStmt],
    scope: &[String],
    iv_name: &str,
    inside: bool,
    ctx: &WalkCtx<'_>,
    accum: &mut ReuseAccum,
    errors: &mut Vec<ReuseInferenceError>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs: _, rhs } => {
                if inside {
                    visit_expr_for_data_refs(rhs, iv_name, scope, ctx, accum, errors);
                }
            }
            IrStmt::Effect { callee: _, args } => {
                if inside {
                    for a in args {
                        visit_expr_for_data_refs(a, iv_name, scope, ctx, accum, errors);
                    }
                }
            }
            IrStmt::For {
                var,
                lo: _,
                hi: _,
                body,
            } => {
                let mut next_scope = scope.to_vec();
                next_scope.push(var.clone());
                let now_inside = inside || var == iv_name;
                walk_for_iv(body, &next_scope, iv_name, now_inside, ctx, accum, errors);
            }
        }
    }
}

/// Recursively walk an expression looking for `IrExpr::DataRef` nodes.
/// Each DataRef's per-axis indices are classified against the active
/// reuse loop's iv name; affine-recognised offsets accumulate into the
/// per-(DataId, axis) offset SET.
fn visit_expr_for_data_refs(
    e: &IrExpr,
    iv_name: &str,
    scope: &[String],
    ctx: &WalkCtx<'_>,
    accum: &mut ReuseAccum,
    errors: &mut Vec<ReuseInferenceError>,
) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            // Resolve data name → id.
            let did = match ctx.name_data.get(name) {
                Some(d) => *d,
                None => {
                    errors.push(ReuseInferenceError::UnknownDataInRef {
                        ref_name: name.clone(),
                    });
                    return;
                }
            };
            for (ax_idx, idx_expr) in indices.iter().enumerate() {
                classify_index(
                    idx_expr, iv_name, scope, name, ax_idx, did, ctx, accum, errors,
                );
            }
        }
        IrExpr::Call { callee: _, args } => {
            for a in args {
                visit_expr_for_data_refs(a, iv_name, scope, ctx, accum, errors);
            }
        }
        IrExpr::Neg(inner) => visit_expr_for_data_refs(inner, iv_name, scope, ctx, accum, errors),
        IrExpr::BinOp(_, lhs, rhs) => {
            visit_expr_for_data_refs(lhs, iv_name, scope, ctx, accum, errors);
            visit_expr_for_data_refs(rhs, iv_name, scope, ctx, accum, errors);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}

/// Classify one index expression against the active reuse iv. Mirrors
/// the halo-inference classifier but writes into a per-axis offset SET
/// (a `BTreeSet<i64>`) instead of a per-iv max-of-|b|; non-iv indices
/// are silently skipped (the index does not contribute to a reuse
/// slot on THIS iv even though it might contribute to another).
#[allow(clippy::too_many_arguments)]
fn classify_index(
    e: &IrExpr,
    iv_name: &str,
    scope: &[String],
    ref_name: &str,
    ax_idx: usize,
    did: DataId,
    ctx: &WalkCtx<'_>,
    accum: &mut ReuseAccum,
    errors: &mut Vec<ReuseInferenceError>,
) {
    // Reject early on DataRef / Call inside the index.
    if expr_contains_dataref_or_call(e) {
        errors.push(ReuseInferenceError::DataDependentStride {
            iter_var: iv_name.to_string(),
            ref_name: ref_name.to_string(),
            ax_idx,
        });
        return;
    }

    // Which enclosing iter-vars does the expression mention?
    let mut ivs_used: BTreeSet<String> = BTreeSet::new();
    collect_iter_var_refs(e, scope, &mut ivs_used);

    if !ivs_used.contains(iv_name) {
        // The index doesn't touch our reuse iv. Two sub-cases:
        //   (a) It's a pure constant (or mentions a sibling iv only).
        //       No contribution to OUR slot. Silently skip.
        //   (b) It's a non-affine arithmetic of OTHER ivs — also
        //       irrelevant to this reuse iv's slot.
        // The other iv's reuse loop (if any) would have classified
        // this same index from ITS active scope. No double-counting
        // because each reuse iv has its own accumulator.
        return;
    }

    if ivs_used.len() > 1 {
        errors.push(ReuseInferenceError::MultipleIterVarsInIndex {
            iter_var: iv_name.to_string(),
            ref_name: ref_name.to_string(),
            ax_idx,
            iter_vars: ivs_used.into_iter().collect(),
        });
        return;
    }

    // ivs_used = {iv_name} — single iv match. Decompose.
    let (coeff, offset) = match affine_decompose(e, iv_name, ctx.consts) {
        Some(pair) => pair,
        None => {
            errors.push(ReuseInferenceError::NonAffineIndex {
                iter_var: iv_name.to_string(),
                ref_name: ref_name.to_string(),
                ax_idx,
            });
            return;
        }
    };

    if coeff != 1 {
        errors.push(ReuseInferenceError::StridedAccessNotSupported {
            iter_var: iv_name.to_string(),
            ref_name: ref_name.to_string(),
            ax_idx,
            coefficient: coeff,
        });
        return;
    }

    // Record the offset.
    accum
        .entry(did)
        .or_default()
        .entry(ax_idx as u64)
        .or_default()
        .insert(offset);
}

/// At loop-body exit, convert each `(DataId, axis) -> offset set` into
/// either a [`ReuseSlot`] (when the set is contiguous and length > 1)
/// or a [`ReuseInferenceError::NonContiguousOffsets`] (when sparse).
/// Length-1 entries (the degenerate `grid[iv]`-only case) are dropped
/// silently — the slot would be a no-op.
fn finalise_accum(
    iv_name: &str,
    iv_id: IterVar,
    accum: ReuseAccum,
    ctx: &WalkCtx<'_>,
    widths: &mut ReuseWidthsMap,
    errors: &mut Vec<ReuseInferenceError>,
) {
    for (did, per_axis) in accum {
        // Reverse-lookup the data name for diagnostics.
        let data_name =
            data_name_for_id(ctx.name_data, did).unwrap_or_else(|| format!("<DataId({})>", did.0));
        for (ax_idx, offsets) in per_axis {
            if offsets.is_empty() {
                continue;
            }
            // BTreeSet iterates in sorted order — first/last are min/max.
            let lo = *offsets.iter().next().expect("non-empty set");
            let hi = *offsets.iter().next_back().expect("non-empty set");
            let hull = (hi - lo) as i128 + 1;
            let used = offsets.len() as i128;
            if hull != used {
                errors.push(ReuseInferenceError::NonContiguousOffsets {
                    iter_var: iv_name.to_string(),
                    ref_name: data_name.clone(),
                    ax_idx,
                    offsets: offsets.iter().copied().collect(),
                });
                continue;
            }
            let length = hull as u64;
            if length <= 1 {
                // Degenerate: a single offset (== 0) ⇒ no reuse to do.
                // Drop the entry; contract says "no entry ⇔ no codegen".
                continue;
            }
            widths
                .entry(iv_id)
                .or_default()
                .entry(did)
                .or_default()
                .insert(
                    ax_idx,
                    ReuseSlot {
                        min_offset: lo,
                        length,
                    },
                );
        }
    }
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

fn collect_reuse_var_names(sched: &SchedIR) -> BTreeSet<&str> {
    let mut out: BTreeSet<&str> = BTreeSet::new();
    for (var, directive) in &sched.loops {
        if directive
            .options
            .iter()
            .any(|opt| matches!(opt, ResolvedLoopOption::Reuse))
        {
            out.insert(var.as_str());
        }
    }
    out
}

/// Walk an expression and union all Ident-leaf names that are present
/// in the enclosing iter-var `scope`. Mirrors
/// [`crate::passes::halo_inference`]'s private helper of the same name
/// — kept duplicated to avoid widening the shared `passes::common`
/// surface for a one-call helper.
fn collect_iter_var_refs(e: &IrExpr, scope: &[String], out: &mut BTreeSet<String>) {
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

/// Reverse-lookup a data symbol name from a `DataId`. Used only for
/// diagnostics (error variants carry a printable name). Returns the
/// FIRST matching key (the `BTreeMap` is one-to-one by construction,
/// so "first" = "only").
fn data_name_for_id(name_data: &BTreeMap<String, DataId>, did: DataId) -> Option<String> {
    name_data
        .iter()
        .find_map(|(n, d)| (*d == did).then(|| n.clone()))
}

// --------------------------------------------------------------------
// Commit
// --------------------------------------------------------------------

/// Destructure-and-rebuild commit. Pre-validation in the caller means
/// no partial-commit hazard; same shape as
/// [`crate::passes::halo_inference`]'s commit helper.
fn commit_reuse_widths(acfg: ACFG, reuse: ReuseWidthsMap) -> ACFG {
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths,
        reuse_widths: _existing,
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
        halo_widths,
        reuse_widths: reuse,
    }
}

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
    use crate::sched::{
        ResolvedLoopDirective, ResolvedPlaceTarget, ResolvedPlacement, ResolvedWorker, SchedIR,
    };

    // ---- Helpers (mirror halo_inference's fixture builder) ----

    fn t_scalar(ty: ScalarType) -> ResolvedType {
        ResolvedType {
            scalar: ty,
            dims: vec![],
        }
    }
    fn t_arr(ty: ScalarType, dims: Vec<usize>) -> ResolvedType {
        ResolvedType { scalar: ty, dims }
    }

    /// Build a tiny LinkedIR with the named iv carrying a `reuse`
    /// directive. Caller supplies the body statements + data dims.
    ///
    /// IMPORTANT: the body must wrap the dataflow in `for {var} : ...`.
    /// We do NOT wrap automatically — the test caller controls the
    /// nest shape so positive / no-loop / nested fixtures are
    /// expressible verbatim.
    fn build_linked_with_reuse(
        body_stmts: Vec<IrStmt>,
        iv_with_reuse: &str,
        grid_dims: Vec<usize>,
        extra_data: Vec<(&str, Vec<usize>)>,
    ) -> LinkedIR {
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
        for (name, dims) in extra_data {
            data.insert(
                name.to_string(),
                ResolvedData {
                    name: name.to_string(),
                    ty: t_arr(ScalarType::I32, dims),
                },
            );
        }
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
            stmts: body_stmts,
        };

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
        let mut loops: BTreeMap<String, ResolvedLoopDirective> = BTreeMap::new();
        loops.insert(
            iv_with_reuse.to_string(),
            ResolvedLoopDirective {
                var: iv_with_reuse.to_string(),
                options: vec![ResolvedLoopOption::Reuse],
                var_span: None,
            },
        );

        let sched = SchedIR {
            algo_path: String::new(),
            worker_classes: BTreeMap::new(),
            memory_regions: BTreeMap::new(),
            workers,
            places,
            place_data: BTreeMap::new(),
            loops,
            transfers: BTreeMap::new(),
            checks: BTreeMap::new(),
        };

        link(algo, sched).expect("link must succeed for reuse test fixtures")
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
    fn ir_mod(l: IrExpr, r: IrExpr) -> IrExpr {
        IrExpr::BinOp(IrBinOp::Mod, Box::new(l), Box::new(r))
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

    fn build_acfg_and_apply(linked: &LinkedIR) -> Result<ACFG, ReuseInferenceError> {
        let acfg = crate::acfg::build_acfg(linked).expect("acfg build");
        apply_reuse_inference(linked, acfg)
    }

    // ---- Positive tests ----

    #[test]
    fn positive_3point_stencil_records_length_3_slot() {
        // for i : 1..15 { out[i] = K(grid[i-1], grid[i], grid[i+1]) }
        // schedule: loop i : reuse;
        // Expected: reuse_widths[i_iv][grid][0] = ReuseSlot{min=-1, length=3}
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_sub(ir_id("i"), ir_int(1))]),
                        data_ref("grid", vec![ir_id("i")]),
                        data_ref("grid", vec![ir_add(ir_id("i"), ir_int(1))]),
                    ],
                ),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![]);
        let acfg = build_acfg_and_apply(&linked).expect("reuse inference succeeds");
        let i_iv = *acfg.name_iter_vars.get("i").unwrap();
        let grid_id = *acfg.name_data.get("grid").unwrap();
        let slot = acfg
            .reuse_widths
            .get(&i_iv)
            .and_then(|m| m.get(&grid_id))
            .and_then(|m| m.get(&0))
            .copied();
        assert_eq!(
            slot,
            Some(ReuseSlot {
                min_offset: -1,
                length: 3
            })
        );
        // Only one (iv, data, axis) entry.
        assert_eq!(acfg.reuse_widths.len(), 1);
    }

    #[test]
    fn positive_separable_filter_two_data_symbols() {
        // for i : 1..15 { out[i] = K(grid[i-1], grid[i+1], src[i-2], src[i+2]) }
        // schedule: loop i : reuse;
        // Expected: grid has slot {min=-1, length=3}; src has {min=-2, length=5}.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(2),
            hi: ir_int(14),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_sub(ir_id("i"), ir_int(1))]),
                        data_ref("grid", vec![ir_id("i")]),
                        data_ref("grid", vec![ir_add(ir_id("i"), ir_int(1))]),
                        data_ref("src", vec![ir_sub(ir_id("i"), ir_int(2))]),
                        data_ref("src", vec![ir_sub(ir_id("i"), ir_int(1))]),
                        data_ref("src", vec![ir_id("i")]),
                        data_ref("src", vec![ir_add(ir_id("i"), ir_int(1))]),
                        data_ref("src", vec![ir_add(ir_id("i"), ir_int(2))]),
                    ],
                ),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![("src", vec![16])]);
        let acfg = build_acfg_and_apply(&linked).expect("reuse inference succeeds");
        let i_iv = *acfg.name_iter_vars.get("i").unwrap();
        let grid_id = *acfg.name_data.get("grid").unwrap();
        let src_id = *acfg.name_data.get("src").unwrap();
        assert_eq!(
            acfg.reuse_widths
                .get(&i_iv)
                .and_then(|m| m.get(&grid_id))
                .and_then(|m| m.get(&0))
                .copied(),
            Some(ReuseSlot {
                min_offset: -1,
                length: 3
            })
        );
        assert_eq!(
            acfg.reuse_widths
                .get(&i_iv)
                .and_then(|m| m.get(&src_id))
                .and_then(|m| m.get(&0))
                .copied(),
            Some(ReuseSlot {
                min_offset: -2,
                length: 5
            })
        );
    }

    #[test]
    fn degenerate_only_bare_iv_records_no_entry() {
        // for i : 0..16 { out[i] = K(grid[i]); }  loop i : reuse;
        // Only offset = 0; length-1 delay line is a no-op. Drop.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(16),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call("K", vec![data_ref("grid", vec![ir_id("i")])]),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![]);
        let acfg = build_acfg_and_apply(&linked).expect("reuse inference succeeds");
        assert!(
            acfg.reuse_widths.is_empty(),
            "length-1 (degenerate) reuse slot must not be recorded; got {:?}",
            acfg.reuse_widths
        );
    }

    #[test]
    fn no_reuse_directive_no_inference_run() {
        // No `reuse` in the schedule → no entries at all.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_sub(ir_id("i"), ir_int(1))]),
                        data_ref("grid", vec![ir_add(ir_id("i"), ir_int(1))]),
                    ],
                ),
            }],
        }];
        // Build LinkedIR with NO reuse directive on i.
        let mut data = BTreeMap::new();
        data.insert(
            "grid".to_string(),
            ResolvedData {
                name: "grid".to_string(),
                ty: t_arr(ScalarType::I32, vec![16]),
            },
        );
        data.insert(
            "out".to_string(),
            ResolvedData {
                name: "out".to_string(),
                ty: t_arr(ScalarType::I32, vec![16]),
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
            stmts: body,
        };
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
        let linked = link(algo, sched).expect("link must succeed");
        let acfg = build_acfg_and_apply(&linked).expect("reuse inference succeeds");
        assert!(
            acfg.reuse_widths.is_empty(),
            "no reuse directive → no entries; got {:?}",
            acfg.reuse_widths
        );
    }

    // ---- Negative tests ----

    #[test]
    fn negative_non_contiguous_offsets_rejected() {
        // for i : reuse; body reads grid[i-3], grid[i], grid[i+5]
        // {-3, 0, +5} — hull length 9, used 3, non-contiguous.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(3),
            hi: ir_int(11),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_sub(ir_id("i"), ir_int(3))]),
                        data_ref("grid", vec![ir_id("i")]),
                        data_ref("grid", vec![ir_add(ir_id("i"), ir_int(5))]),
                    ],
                ),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![]);
        let err = build_acfg_and_apply(&linked).unwrap_err();
        match err {
            ReuseInferenceError::NonContiguousOffsets {
                iter_var,
                ref_name,
                ax_idx,
                offsets,
            } => {
                assert_eq!(iter_var, "i");
                assert_eq!(ref_name, "grid");
                assert_eq!(ax_idx, 0);
                assert_eq!(offsets, vec![-3, 0, 5]);
            }
            other => panic!("expected NonContiguousOffsets, got {other:?}"),
        }
    }

    #[test]
    fn negative_data_dependent_stride() {
        // grid[lookup[i]] — DataRef inside index.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(16),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref("grid", vec![data_ref("lookup", vec![ir_id("i")])])],
                ),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![("lookup", vec![16])]);
        let err = build_acfg_and_apply(&linked).unwrap_err();
        match err {
            ReuseInferenceError::DataDependentStride {
                iter_var,
                ref_name,
                ax_idx,
            } => {
                assert_eq!(iter_var, "i");
                assert_eq!(ref_name, "grid");
                assert_eq!(ax_idx, 0);
            }
            other => panic!("expected DataDependentStride, got {other:?}"),
        }
    }

    #[test]
    fn negative_non_affine_mod_index() {
        // grid[i % 8] — Mod is non-affine in i.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(16),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref("grid", vec![ir_mod(ir_id("i"), ir_int(8))])],
                ),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![]);
        let err = build_acfg_and_apply(&linked).unwrap_err();
        match err {
            ReuseInferenceError::NonAffineIndex {
                iter_var,
                ref_name,
                ax_idx,
            } => {
                assert_eq!(iter_var, "i");
                assert_eq!(ref_name, "grid");
                assert_eq!(ax_idx, 0);
            }
            other => panic!("expected NonAffineIndex, got {other:?}"),
        }
    }

    #[test]
    fn negative_strided_coefficient_two() {
        // grid[2*i + 1] — coefficient 2.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(8),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref(
                        "grid",
                        vec![ir_add(ir_mul(ir_int(2), ir_id("i")), ir_int(1))],
                    )],
                ),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![]);
        let err = build_acfg_and_apply(&linked).unwrap_err();
        match err {
            ReuseInferenceError::StridedAccessNotSupported { coefficient, .. } => {
                assert_eq!(coefficient, 2);
            }
            other => panic!("expected StridedAccessNotSupported, got {other:?}"),
        }
    }

    #[test]
    fn negative_multiple_iter_vars_in_index() {
        // for i : reuse; { for j { out[i][j] = K(grid[i + j]); } }
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            body: vec![IrStmt::For {
                var: "j".to_string(),
                lo: ir_int(0),
                hi: ir_int(4),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("i"), ir_id("j")]),
                    rhs: ir_call(
                        "K",
                        vec![data_ref("grid", vec![ir_add(ir_id("i"), ir_id("j"))])],
                    ),
                }],
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16, 16], vec![]);
        let err = build_acfg_and_apply(&linked).unwrap_err();
        match err {
            ReuseInferenceError::MultipleIterVarsInIndex { iter_vars, .. } => {
                assert_eq!(iter_vars, vec!["i".to_string(), "j".to_string()]);
            }
            other => panic!("expected MultipleIterVarsInIndex, got {other:?}"),
        }
    }

    // ---- Advisory + determinism ----

    #[test]
    fn advisory_collects_all_errors_strict_short_circuits() {
        // Two errors in one body: a non-affine Mod index AND a strided
        // coefficient. Strict returns the FIRST; advisory returns BOTH.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(16),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref("grid", vec![ir_mod(ir_id("i"), ir_int(8))]),
                        data_ref("grid", vec![ir_mul(ir_int(3), ir_id("i"))]),
                    ],
                ),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![]);

        // Strict: first error only.
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let strict_err = apply_reuse_inference(&linked, acfg).unwrap_err();
        // Source order: the Mod DataRef is the first arg → NonAffineIndex first.
        match strict_err {
            ReuseInferenceError::NonAffineIndex { .. } => {}
            other => panic!("strict should surface NonAffineIndex first, got {other:?}"),
        }

        // Advisory: both errors collected.
        let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
        let (_, errs) = apply_reuse_inference_advisory(&linked, acfg);
        assert_eq!(errs.len(), 2, "advisory must collect all errors: {errs:?}");
        assert!(matches!(
            errs[0],
            ReuseInferenceError::NonAffineIndex { .. }
        ));
        assert!(matches!(
            errs[1],
            ReuseInferenceError::StridedAccessNotSupported { coefficient: 3, .. }
        ));
    }

    #[test]
    fn determinism_same_input_yields_same_map() {
        // Use a CONTIGUOUS offset set {-1, 0, +1} so the inference
        // succeeds; the determinism property is about the resulting
        // map shape being bit-identical across two runs, not about
        // exercising the rejection paths (those have their own tests).
        let body_factory = || {
            vec![IrStmt::For {
                var: "i".to_string(),
                lo: ir_int(1),
                hi: ir_int(15),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("i")]),
                    rhs: ir_call(
                        "K",
                        vec![
                            data_ref("grid", vec![ir_sub(ir_id("i"), ir_int(1))]),
                            data_ref("grid", vec![ir_id("i")]),
                            data_ref("grid", vec![ir_add(ir_id("i"), ir_int(1))]),
                        ],
                    ),
                }],
            }]
        };
        let linked1 = build_linked_with_reuse(body_factory(), "i", vec![16], vec![]);
        let linked2 = build_linked_with_reuse(body_factory(), "i", vec![16], vec![]);
        let a1 = build_acfg_and_apply(&linked1).expect("first run");
        let a2 = build_acfg_and_apply(&linked2).expect("second run");
        assert_eq!(a1.reuse_widths, a2.reuse_widths);
        // Sanity: non-empty (the test wouldn't catch much if both maps
        // were empty due to a regression that silently skipped the
        // walk).
        assert!(
            !a1.reuse_widths.is_empty(),
            "determinism test must record at least one slot"
        );
    }

    #[test]
    fn nested_call_inside_kernel_arg_recurses() {
        // for i : reuse; { out[i] = K(inner(grid[i-1], grid[i+1])); }
        // The OUTER K's arg list contains a nested Call. Reuse must
        // still inspect the inner call's DataRef args.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![ir_call(
                        "inner",
                        vec![
                            data_ref("grid", vec![ir_sub(ir_id("i"), ir_int(1))]),
                            data_ref("grid", vec![ir_id("i")]),
                            data_ref("grid", vec![ir_add(ir_id("i"), ir_int(1))]),
                        ],
                    )],
                ),
            }],
        }];
        // Need to add the `inner` kernel + placement.
        let mut linked = build_linked_with_reuse(body, "i", vec![16], vec![]);
        linked.algo.kernels.insert(
            "inner".to_string(),
            ResolvedKernel {
                name: "inner".to_string(),
                params: vec![
                    t_scalar(ScalarType::I32),
                    t_scalar(ScalarType::I32),
                    t_scalar(ScalarType::I32),
                ],
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
        let linked = link(linked.algo, linked.sched).expect("re-link with inner kernel");
        let acfg = build_acfg_and_apply(&linked).expect("reuse inference succeeds");
        let i_iv = *acfg.name_iter_vars.get("i").unwrap();
        let grid_id = *acfg.name_data.get("grid").unwrap();
        let slot = acfg
            .reuse_widths
            .get(&i_iv)
            .and_then(|m| m.get(&grid_id))
            .and_then(|m| m.get(&0))
            .copied();
        assert_eq!(
            slot,
            Some(ReuseSlot {
                min_offset: -1,
                length: 3
            })
        );
    }

    #[test]
    fn nested_reuse_outer_inner_independent_accumulators() {
        // for i : reuse {
        //   for j : reuse {
        //     out[i][j] = K(grid[i-1], grid[i+1], src[j-2], src[j+2]);
        //   }
        // }
        // Two reuse loops, each with its own accumulator:
        //  - i sees grid offsets {-1, +1}; j is a separate iv (no
        //    contribution to i's accum because j is not in i's expr).
        //    Wait — `grid[i-1]` mentions i (good). `src[j-2]` mentions
        //    j only (not i). So i's accum on grid sees {-1, +1}, on
        //    src sees nothing. And j's accum on src sees {-2, +2}, on
        //    grid sees nothing.
        //  - Both produce a slot length 3 (i, grid) and length 5
        //    (j, src).
        // BUT both grid slots are non-contiguous ({-1, +1} skips 0 →
        // hull 3, used 2 → non-contiguous). So we expect NonContiguousOffsets
        // errors, NOT slots. The OK contiguous form needs the bare iv read too.
        //
        // Build the CONTIGUOUS form: include grid[i] and src[j] each.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(2),
            hi: ir_int(14),
            body: vec![IrStmt::For {
                var: "j".to_string(),
                lo: ir_int(2),
                hi: ir_int(14),
                body: vec![IrStmt::Dataflow {
                    lhs: lhs("out", vec![ir_id("i"), ir_id("j")]),
                    rhs: ir_call(
                        "K",
                        vec![
                            data_ref("grid", vec![ir_sub(ir_id("i"), ir_int(1))]),
                            data_ref("grid", vec![ir_id("i")]),
                            data_ref("grid", vec![ir_add(ir_id("i"), ir_int(1))]),
                            data_ref("src", vec![ir_sub(ir_id("j"), ir_int(2))]),
                            data_ref("src", vec![ir_sub(ir_id("j"), ir_int(1))]),
                            data_ref("src", vec![ir_id("j")]),
                            data_ref("src", vec![ir_add(ir_id("j"), ir_int(1))]),
                            data_ref("src", vec![ir_add(ir_id("j"), ir_int(2))]),
                        ],
                    ),
                }],
            }],
        }];
        // Override: schedule has BOTH i and j with reuse.
        let mut linked = build_linked_with_reuse(body, "i", vec![16, 16], vec![("src", vec![16])]);
        linked.sched.loops.insert(
            "j".to_string(),
            ResolvedLoopDirective {
                var: "j".to_string(),
                options: vec![ResolvedLoopOption::Reuse],
                var_span: None,
            },
        );
        let linked = link(linked.algo, linked.sched).expect("re-link with reuse on j");
        let acfg = build_acfg_and_apply(&linked).expect("reuse inference succeeds");
        let i_iv = *acfg.name_iter_vars.get("i").unwrap();
        let j_iv = *acfg.name_iter_vars.get("j").unwrap();
        let grid_id = *acfg.name_data.get("grid").unwrap();
        let src_id = *acfg.name_data.get("src").unwrap();
        // Outer i sees grid offsets {-1, 0, +1} → length 3.
        assert_eq!(
            acfg.reuse_widths
                .get(&i_iv)
                .and_then(|m| m.get(&grid_id))
                .and_then(|m| m.get(&0))
                .copied(),
            Some(ReuseSlot {
                min_offset: -1,
                length: 3
            })
        );
        // i's accum has no entry for src (src indices don't mention i).
        assert!(acfg
            .reuse_widths
            .get(&i_iv)
            .and_then(|m| m.get(&src_id))
            .is_none());
        // Inner j sees src offsets {-2..=+2} → length 5.
        assert_eq!(
            acfg.reuse_widths
                .get(&j_iv)
                .and_then(|m| m.get(&src_id))
                .and_then(|m| m.get(&0))
                .copied(),
            Some(ReuseSlot {
                min_offset: -2,
                length: 5
            })
        );
        // j's accum has no entry for grid (grid indices don't mention j).
        assert!(acfg
            .reuse_widths
            .get(&j_iv)
            .and_then(|m| m.get(&grid_id))
            .is_none());
    }

    #[test]
    fn no_dataref_body_no_entries() {
        // for i : reuse; { out[i] = K(7); }  — no DataRef at all.
        let body = vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(16),
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call("K", vec![ir_int(7)]),
            }],
        }];
        let linked = build_linked_with_reuse(body, "i", vec![16], vec![]);
        let acfg = build_acfg_and_apply(&linked).expect("reuse inference succeeds");
        assert!(acfg.reuse_widths.is_empty());
    }
}
