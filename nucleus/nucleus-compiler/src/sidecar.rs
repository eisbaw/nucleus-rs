//! Codegen-contract sidecar: per-`DataId` types, const values,
//! symbolic loop bounds (TASK-0160), and per-`KernelId` signatures
//! (TASK-0169) for **EventList-only** backends.
//!
//! ## Why this exists
//!
//! The per-worker `EventList` ([`crate::event::Event`]) is, after
//! TASK-0156 (value bindings) and TASK-0159 ([`Event::Loop`](crate::event::Event::Loop)
//! structure), a faithful *control + value* contract: a backend can
//! reconstruct every kernel call and every rolled loop from it
//! alone. But the pthreads-sync backend also needs three things the
//! EventList deliberately does **not** carry, because they are not
//! per-event facts — they are *whole-program schedule-pass metadata*,
//! exactly like the `name_kernels` / `name_data` / `name_workers`
//! tables a backend already receives alongside the EventList:
//!
//! 1. **Per-`DataId` [`ResolvedType`]** — the backend pre-allocates
//!    `let mut c = vec![0; 256];` (length = product of dims) and
//!    types worker slots `Arc<Slot<Vec<i32>>>` (element type from
//!    the [`ScalarType`]) and casts scalar kernel args. None of that
//!    is recoverable from `Event::Fire` / `DataSlice` (which carry a
//!    `DataId` + index `IrExpr`s only, no shape, no element type).
//!
//! 2. **Const name → value** — so a backend renders a loop or array
//!    bound that mentions a `const N = 256` without re-reading the
//!    AlgoIR.
//!
//! 3. **The *unevaluated* loop-bound expression** for each loop
//!    variable. `build_acfg` folds `for y : 1 .. H-1` to a concrete
//!    `Range<i64>` (`1..15`) — it calls `eval_const(lo)/eval_const(hi)`
//!    and stores the `i64`s into [`crate::acfg::ACFGNode::Repeat`]
//!    (a non-const or overflowing bound is a typed
//!    `BuildAcfgError::NonConstLoopBound` / `OverflowingLoopBound`,
//!    not a panic — TASK-0398). `Event::Loop` mirrors that
//!    concrete `Range<i64>`, so the source form `H - 1` is destroyed
//!    before either the ACFG or the EventList. The AlgoIR-walking
//!    pthreads-sync backend emits `for y in (1_i64)..((16_i64 -
//!    1_i64))` from the *source* `IrStmt::For` bounds; an
//!    EventList-only backend cannot reproduce `(16_i64 - 1_i64)`
//!    from a folded `15`. This sidecar captures the **unevaluated**
//!    `lo`/`hi` [`IrExpr`]s additively at the `build_acfg` boundary,
//!    keyed by the *same* [`IterVar`] that `Event::Loop` carries, so
//!    a backend pairs them up and renders the bound verbatim.
//!
//! 4. **Per-`KernelId` signature** — the declared param/return
//!    [`ResolvedType`]s of every kernel (TASK-0169). The shared
//!    `backend_common::render::fire::render_fire_arg` helper decides
//!    a scalar argument cast — an iter-var-derived scalar expression
//!    fed to a scalar kernel param renders as `(expr) as usize` — by
//!    reading `algo.kernels[callee].params[i]` and testing
//!    `ResolvedType::is_scalar`. `Event::Fire` carries the
//!    [`KernelId`] and the argument *values* (`FireBinding`,
//!    TASK-0156) but not the callee's declared param types. Without
//!    this table an EventList-only backend (TASK-0124) cannot
//!    reproduce that cast/dispatch decision. The table is keyed by
//!    the *same* [`KernelId`] `Event::Fire` carries, so a backend
//!    joins `Event::Fire.kernel` -> [`kernel_sigs`](NameSidecar::kernel_sigs)
//!    with no name round-trip — exactly the `name_data` ->
//!    `data_types` join, mirrored for kernels. With this in place the
//!    `(EventList, name tables, NameSidecar)` contract is **fully
//!    AlgoIR-free** for pthreads-sync codegen (the last known gap).
//!
//! ## What this sidecar deliberately does NOT do
//!
//! - It does **not** stop `eval_const` folding and it does **not**
//!   make `ACFGNode::Repeat.range` symbolic. The analysis Net
//!   (`acfg_to_petri`) and the boundedness/deadlock passes consume
//!   the *unrolled, concrete* iteration counts; making the range
//!   symbolic would ripple into them (the high-regression refactor
//!   TASK-0142/TASK-0159 deliberately avoided). The Net keeps the
//!   concrete range; this sidecar additively carries the source form
//!   *in parallel*. They are decoupled by construction.
//!
//! - It does **not** switch the pthreads-sync backend off the AlgoIR
//!   walk. That is TASK-0124. This module makes the contract
//!   *sufficient* and proves it (see `tests/petri_to_events.rs`
//!   `sidecar_*` tests, mirroring TASK-0156's
//!   `eventlist_alone_reconstructs_stencil_kernel_call`). The actual
//!   `emit()` signature change + AlgoIR removal is TASK-0124's work.
//!
//! ## Determinism
//!
//! Every table is a [`BTreeMap`] keyed by an opaque id ([`DataId`] /
//! [`IterVar`] / [`KernelId`]) or a `String` (const name). Iteration is sorted;
//! [`build_sidecar`] is a pure function of `(LinkedIR, ACFG name
//! tables)`. The same inputs yield a byte-identical sidecar, and the
//! serde wire form (feature-gated, like `event`/`contract`) is
//! stable for golden tests.

use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::acfg::NotifyMode;
use crate::algo::{CombineOp, IrExpr, Purity, ResolvedType, ScalarType};
use crate::event::{DataId, IterVar, KernelId, SeqTag, WorkerId};
use crate::sched::TransportMode;
use crate::link::LinkedIR;

// The unified per-`SeqTag` transfer-facts value (TASK-0455.08). Split into
// a child module to keep this file under the 1000-LoC mega-file fence
// (`just check-mega-files`), same precedent as `collectors` /
// `cumulative_tests`; re-exported so it stays
// `nucleus_compiler::sidecar::XferFacts`.
mod xfer_facts;
pub use xfer_facts::XferFacts;

/// The unevaluated source bounds of a `for` loop, captured before
/// `build_acfg` folds them to a concrete `Range<i64>`.
///
/// `lo` / `hi` are the AlgoIR [`IrExpr`]s verbatim — e.g. for
/// `for y : 1 .. H-1` (`H` a const = 16) `lo = IntLit(1)`,
/// `hi = BinOp(Sub, Ident("H"), IntLit(1))`. The backend renders
/// these with [`NameSidecar::consts`] in scope to produce the source
/// form (`(16_i64 - 1_i64)`); the analysis Net independently keeps
/// the folded `1..15`. The two never need to agree — they serve
/// different consumers (codegen vs boundedness/deadlock).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LoopBound {
    /// Unevaluated lower bound expression (the `lo` in `lo..hi`).
    pub lo: IrExpr,
    /// Unevaluated upper bound expression (the `hi` in `lo..hi`).
    pub hi: IrExpr,
}

/// The codegen-contract sidecar (TASK-0160).
///
/// Travels alongside the per-worker `EventList` + the ACFG `name_*`
/// tables. A backend consuming ONLY `(EventList, name tables,
/// NameSidecar)` — never the AlgoIR — has everything it needs to:
///
/// - size a pre-init allocation: `vec![<zero>; product(dims)]` from
///   [`data_types`](Self::data_types)`[did].dims`;
/// - pick the Rust element type and worker slot type
///   (`Vec<i32>` / `Arc<Slot<Vec<i32>>>`) and scalar-arg casts from
///   the [`ScalarType`];
/// - render an array/loop bound that mentions a const from
///   [`consts`](Self::consts);
/// - re-emit a rolled `for v in lo..hi` with the *source-form* bound
///   from [`loop_bounds`](Self::loop_bounds), paired to
///   `Event::Loop { iter_var, .. }` by the shared [`IterVar`];
/// - decide a scalar-argument cast (`(expr) as usize`) / whole-array
///   vs scalar dispatch for a kernel call from
///   [`kernel_sigs`](Self::kernel_sigs), paired to `Event::Fire {
///   kernel, .. }` by the shared [`KernelId`] — the last fact the
///   pthreads-sync backend still reads from `algo.kernels`.
///
/// With [`kernel_sigs`](Self::kernel_sigs) in place this contract is
/// **fully AlgoIR-free** for pthreads-sync codegen.
///
/// All maps are deterministic ([`BTreeMap`]); see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NameSidecar {
    /// Per data symbol: its [`ResolvedType`] (scalar + dims). The
    /// key is the same [`DataId`] the EventList's `DataSlice.data`
    /// and `Event::Alloc.data` carry, and the same one
    /// `ACFG::name_data` maps names to. Length-of-allocation =
    /// product of `dims`; scalar (`dims == []`) data is a single
    /// element.
    pub data_types: BTreeMap<DataId, ResolvedType>,

    /// Const name → `(ScalarType, value)`. `value` is the
    /// already-evaluated `i64` (the const evaluator is
    /// integer-only); `ScalarType` lets the backend pick the literal
    /// suffix / cast. This is the table the backend uses to render a
    /// bound expression that references a const.
    pub consts: BTreeMap<String, ConstValue>,

    /// Per loop variable: the *unevaluated* source bounds. Keyed by
    /// the [`IterVar`] that `Event::Loop { iter_var, .. }` and
    /// `ACFG::name_iter_vars` share, so a backend walking an
    /// `Event::Loop` looks the symbolic bound up here instead of
    /// using the folded `range`.
    pub loop_bounds: BTreeMap<IterVar, LoopBound>,

    /// Per kernel: its declared param + return [`ResolvedType`]s
    /// (TASK-0169). The key is the *same* [`KernelId`] that
    /// `Event::Fire { kernel, .. }` carries and that
    /// `ACFG::name_kernels` maps names to. A backend rendering a
    /// `Fire`'s argument list joins `Event::Fire.kernel` here,
    /// indexes `params[i]` for the i-th argument, and applies the
    /// scalar-cast / aggregate-dispatch rule
    /// (`KernelSig::params[i].is_scalar()`) exactly as the
    /// AlgoIR-walking `backend_common::render::fire::render_fire_arg`
    /// does against `algo.kernels[callee].params[i]` today — without
    /// the AlgoIR.
    /// This is the last `algo.kernels` read pthreads-sync codegen
    /// has; with this table the contract is fully AlgoIR-free.
    pub kernel_sigs: BTreeMap<KernelId, KernelSig>,

    /// Per-worker loop-range override for loops carrying a
    /// `partition=workers` schedule directive (TASK-0212). Mirrors
    /// [`crate::acfg::ACFG::partition_worker_ranges`] verbatim — the
    /// codegen contract surface for that ACFG sidecar. A backend
    /// rendering an `Event::Loop` for a worker whose `iter_var` has
    /// an entry here MUST prefer the concrete per-worker range over
    /// the symbolic [`loop_bounds`](Self::loop_bounds) entry (the symbolic bound names
    /// the SOURCE range, not the partitioned per-worker slice).
    /// Workers not listed in the inner map (e.g. host) and iter vars
    /// without an outer entry fall back to [`loop_bounds`](Self::loop_bounds) /
    /// `Event::Loop.range` exactly as before TASK-0212.
    ///
    /// Why a separate sidecar field and not e.g. overloading the
    /// existing `loop_bounds` with a per-worker variant: `loop_bounds`
    /// carries SYMBOLIC bounds (e.g. `B - 1`) for source loops so the
    /// backend can render the bound expression verbatim with consts
    /// in scope. Per-worker partition bounds are CONCRETE literals
    /// (`0..4`, `4..8`, …) the partition pass computes from the
    /// source range and the worker count; mixing concrete and
    /// symbolic into one map would lose that distinction. A
    /// dedicated map makes the precedence rule (concrete overrides
    /// symbolic for partitioned vars) explicit at the consumer site.
    ///
    /// Determinism: nested `BTreeMap`s keyed by id; iteration is in
    /// numeric order. serde-default so an old wire payload (no field)
    /// deserialises as empty (no overrides, pre-TASK-0212 behaviour).
    #[cfg_attr(feature = "serde", serde(default))]
    pub partition_worker_ranges: BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,

    /// Per-`SeqTag` **unified transfer facts** — the single
    /// backend-facing surface for every per-transfer-edge codegen fact
    /// the schedule's `transfer DATA : ...` directive carries through to
    /// a matched Push/Wait pair (TASK-0455.08). Replaces the former
    /// parallel `transfer_buffer_for_seq` (TASK-0233) and
    /// `transfer_transport_for_seq` (TASK-0438.02) maps — each fact used
    /// to ride its OWN `BTreeMap<SeqTag, _>` with its own collector,
    /// serde-default, and forward-clone, and `notify` had no sidecar
    /// surface at all. Unifying them under one [`XferFacts`] value
    /// populated by ONE collector removes the divergence hazard the
    /// `KernelSig` comment below warns about (N parallel maps that can
    /// silently fall out of step).
    ///
    /// Every fact lives in `ACFG::XferPlaceholder::policy` upstream
    /// (`buffer`, `transport`, `notify`), but the backend receives only
    /// `NameSidecar` per the EventList contract (TASK-0124), so the facts
    /// ride along the sidecar. `pipeline_depth` is the one exception: it
    /// is NOT a `policy` field — it lives on `ACFG::pipeline_depth_for_seq`
    /// (the Petri/initial-marking source of truth, consumed only by the
    /// middle-end) and is MIRRORED into [`XferFacts::pipeline_depth`] here
    /// as a read-only backend-facing copy. See that field's docs for why
    /// the ACFG stays the single source.
    ///
    /// One entry per matched Push/Wait pair (the seq is unique per pair;
    /// both endpoints share it — `passes::transfer_inject` guarantees that
    /// invariant). Sync transfers also appear here (their `buffer`
    /// defaults to 1, `transport` to PIO, `notify` to `Default` per
    /// [`crate::acfg::TransferPolicy::default`]).
    ///
    /// Empty for any algorithm that produces no cross-worker transfers
    /// (single-worker or same-worker-only schedules). The `Event::Push`
    /// / `Event::Wait` variants carry `seq: SeqTag` so a codegen consumer
    /// joins this map with the event's seq via the accessor helpers
    /// ([`xfer_facts`](Self::xfer_facts_for), [`xfer_buffer`](Self::xfer_buffer),
    /// [`xfer_transport`](Self::xfer_transport), [`xfer_notify`](Self::xfer_notify)).
    ///
    /// Determinism: `BTreeMap` keyed by SeqTag; iteration is in numeric
    /// order. serde-default so an older wire payload (no field)
    /// deserialises as empty.
    #[cfg_attr(feature = "serde", serde(default))]
    pub xfer_facts: BTreeMap<SeqTag, XferFacts>,

    /// Per-(KernelId, IterVar) halo width inferred from the kernel's
    /// access pattern (TASK-0260, Stage 1). Mirrors
    /// [`crate::acfg::ACFG::halo_widths`] verbatim — the codegen
    /// contract surface for that ACFG sidecar.
    ///
    /// A backend (Stage 2 — TASK-0263 — `transfer_inject` extension)
    /// joins this map with the [`IterTile::bounds`](crate::event::IterTile)
    /// it walks during projection to extend per-tile transfer ranges
    /// by the halo width on each side of each axis. STAGE 1 lands the
    /// fact; no current backend / pass observes it yet, so emitted
    /// code is byte-identical to pre-TASK-0260.
    ///
    /// Empty for algorithms whose kernels do not exercise affine
    /// `iter_var + b` reads (every example today ships this way:
    /// Stage 1 is observationally inert). serde-default so an older
    /// wire payload (no field) deserialises as empty — same backward-
    /// compat contract as `xfer_facts` (TASK-0233/TASK-0455.08) and
    /// `partition_worker_ranges` (TASK-0212).
    ///
    /// ### Shape
    ///
    /// `BTreeMap<KernelId, BTreeMap<IterVar, u64>>` — nested maps,
    /// both keyed by `serde(transparent)` `u64` newtypes so the
    /// codegen-contract JSON wire form round-trips. See
    /// [`crate::acfg::ACFG::halo_widths`] docs for the rationale (a
    /// tuple-keyed flat map cannot be a JSON map key).
    ///
    /// Determinism: nested `BTreeMap`s, iteration in numeric order.
    #[cfg_attr(feature = "serde", serde(default))]
    pub halo_widths: BTreeMap<KernelId, BTreeMap<IterVar, u64>>,

    /// Per-(IterVar, DataId, axis) delay-line slot inferred from
    /// `reuse`-tagged loop bodies (TASK-0261, Stage 1). Mirrors
    /// [`crate::acfg::ACFG::reuse_widths`] verbatim — the codegen
    /// contract surface for that ACFG sidecar.
    ///
    /// A backend (Stage 2 — TASK-0265 — backend walker / Plan
    /// delay-line emit) joins this map with the iv-iteration it is
    /// projecting and rewrites each `grid[iv + b]` read inside the
    /// loop body into a circular-buffer lookup. STAGE 1 lands the
    /// fact; no current backend / pass observes it yet, so emitted
    /// code is byte-identical to pre-TASK-0261.
    ///
    /// Empty for algorithms whose schedules carry no `reuse` loop
    /// directives, and (silently) for reuse loops whose body's
    /// iv-bearing offsets are degenerate (length 1 — a no-op delay
    /// line). serde-default so an older wire payload (no field)
    /// deserialises as empty — same backward-compat contract as
    /// `halo_widths` (TASK-0260), `xfer_facts`
    /// (TASK-0233/TASK-0455.08), and `partition_worker_ranges` (TASK-0212).
    ///
    /// ### Shape
    ///
    /// `BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64 /* axis */,
    /// ReuseSlot>>>` — triple-nested maps; the deep nest is
    /// load-bearing for serde-JSON (tuple keys are not JSON map
    /// keys). See [`crate::acfg::ACFG::reuse_widths`] docs for the
    /// rationale.
    ///
    /// Determinism: nested `BTreeMap`s, iteration in numeric order.
    #[cfg_attr(feature = "serde", serde(default))]
    pub reuse_widths: BTreeMap<
        IterVar,
        BTreeMap<DataId, BTreeMap<u64, crate::passes::reuse_inference::ReuseSlot>>,
    >,

    /// Per-outer-`IterVar` pairing of the two iter-vars a
    /// `partition=blocks2d` directive partitions (TASK-0264 cycle
    /// 113, AC#1). Mirrors
    /// [`crate::acfg::ACFG::partition_pairs`] verbatim — the codegen
    /// contract surface for that ACFG sidecar.
    ///
    /// A downstream consumer (TASK-0289 halo-strip Push/Wait
    /// synthesis) reads this to disambiguate paired-by-one-blocks2d-
    /// directive iter-vars from two-independent-rows-directives on
    /// unrelated loops. Empty for algorithms whose schedule carries
    /// no `partition=blocks2d` directives (every shipped example
    /// today is in this set; the cycle-79 mp-tcp-* examples + 02-13
    /// all use partition=workers / partition=rows or no partition).
    ///
    /// serde-default so an older wire payload (no field) deserialises
    /// as empty — same additive contract as
    /// [`Self::partition_worker_ranges`] (TASK-0212).
    ///
    /// Determinism: `BTreeMap` keyed by `IterVar` (a `u64` newtype);
    /// iteration is in numeric order.
    #[cfg_attr(feature = "serde", serde(default))]
    pub partition_pairs: BTreeMap<IterVar, IterVar>,

    /// Per-outer-`IterVar` `(rows, cols)` grid shape inferred by
    /// `partition_blocks2d`'s `decompose_grid(num_workers)` call
    /// (TASK-0264 cycle 113, AC#2). Mirrors
    /// [`crate::acfg::ACFG::grid_shape_for_outer_iv`] verbatim — the
    /// codegen contract surface for that ACFG sidecar.
    ///
    /// A downstream consumer (TASK-0289) inverts WorkerId →
    /// (row, col) via `(i / cols, i % cols)` where `i =
    /// bset_position(worker)` and `(rows, cols) = sidecar
    /// .grid_shape_for_outer_iv[outer_iv]`. Empty for algorithms
    /// whose schedule carries no `partition=blocks2d` directives.
    ///
    /// serde-default so an older wire payload (no field) deserialises
    /// as empty.
    ///
    /// Determinism: `BTreeMap` keyed by `IterVar` (a `u64` newtype);
    /// iteration is in numeric order.
    #[cfg_attr(feature = "serde", serde(default))]
    pub grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)>,

    /// DataIds whose algorithm-IR shape is a **cumulative
    /// cross-iteration** array (TASK-0341.02.02.01.03, cycle 213): the
    /// data symbol is the LHS of a `<--` inside a `for` loop AND reads
    /// itself on the RHS at a DIFFERENT index expression along that
    /// loop's iteration axis (e.g. 16-jacobi's `field[t][y][x] <--
    /// jacobi5_or_seed(field[(t+ITERS)%(ITERS+1)][y-1][x], ...)` —
    /// `field` self-read at dim 0 index `(t+ITERS)%(ITERS+1)` != the
    /// LHS write index `t`).
    ///
    /// Consumed by
    /// [`crate::sidecar`]-aware
    /// `backend_common::multi_worker_walker::collect_accumulate_waits`
    /// to EXCLUDE such arrays from the overlapping-write accumulator
    /// (`wrapping_add` fan-in) classification. For a cumulative array
    /// every worker holds the FULL shared history after each exchange;
    /// summing whole local arrays double-counts that history across N
    /// workers (the empirically-observed xN on 16-jacobi). The correct
    /// combine for a cumulative partition-band exchange is COPY
    /// (banded slice-paste, the `wait_slice` N-D path), NOT accumulate.
    ///
    /// Contrast a DISJOINT single-pass accumulator
    /// (08-histogram's `histogram[b] <-- bin_inc(histogram[b], ...)`):
    /// the self-read uses the SAME index `b` as the LHS write (no
    /// cross-iteration index shift), so it is NOT cumulative and stays
    /// accumulate.
    ///
    /// serde-default so an older wire payload (no field) deserialises
    /// as empty.
    #[cfg_attr(feature = "serde", serde(default))]
    pub cumulative_data: std::collections::BTreeSet<DataId>,

    /// Data symbols in DECLARATION order, as [`DataId`]s
    /// (TASK-0049.10.06). The codegen-contract surface for
    /// `AlgoIR::data_decl_order`: the order the `data ...` decls appear
    /// in the source, mapped through `ACFG::name_data` to the canonical
    /// [`DataId`] the EventList carries.
    ///
    /// Why this exists despite `data_types` already holding every
    /// `DataId`: `data_types` is a `BTreeMap` keyed by `DataId`, and
    /// `DataId` is assigned ALPHABETICALLY (`acfg::build` enumerates the
    /// name-keyed `BTreeMap`), so iterating `data_types` yields
    /// alphabetical, NOT declaration, order. The embedded multi-MCU
    /// backend's global `input.bin` / `reference.bin` byte layout must
    /// match the reference generator's HAND-WRITTEN declaration-order
    /// block layout; this vector is the only sound ordering handle for
    /// that (see `embedded_pattern::multimcu::compute_input_offsets`).
    ///
    /// Deterministic: a `Vec` built in `AlgoIR::data_decl_order` order,
    /// no HashMap iteration. serde-default so an old wire payload (no
    /// field) deserialises as empty (backends that don't need a global
    /// byte layout simply never read it).
    #[cfg_attr(feature = "serde", serde(default))]
    pub data_decl_order: Vec<DataId>,

    /// Per-accumulator-`DataId` overlapping-write combine identity
    /// (TASK-0343.01.01). For each data symbol that is an
    /// algorithm-level accumulator (the LHS-appears-in-RHS `acc[..] <--
    /// k(acc[..], ...)` shape), this maps its [`DataId`] to the
    /// [`CombineOp`] declared on the OWNING kernel `k` (the callee on
    /// the RHS `Call` of its `<--` Dataflow).
    ///
    /// Consumed by
    /// `backend_common::multi_worker_walker::wait::render_accumulate_assign`
    /// to choose the host element-wise combine emit for the
    /// overlapping-write fan-in: `sum` → `name[_k].wrapping_add(_tmp[_k])`,
    /// `or` → `name[_k] | _tmp[_k]`, `xor` → `name[_k] ^ _tmp[_k]`.
    /// Pre-TASK-0343.01.01 that combine was hardcoded to `wrapping_add`.
    ///
    /// An accumulator data symbol ABSENT from this map declared NO
    /// combine identity on its owning kernel; the driver gate
    /// (`check_accumulator_consistency`) and the render path both fail
    /// LOUD on that case (no silent assume-sum fallback — TASK-0343.01.01
    /// AC#4 soundness reject).
    ///
    /// Built at sidecar time by resolving each accumulator data's
    /// Dataflow-RHS callee kernel's `combine` attribute. Empty for every
    /// algorithm whose accumulator kernels declare no `combine`
    /// (i.e. for a single-worker schedule there is no fan-in, so the
    /// map being empty is inert — the accumulate fan-in only arises
    /// under a distributed whole-array-replicate partition).
    ///
    /// serde-default so an older wire payload (no field) deserialises as
    /// empty (pre-TASK-0343.01.01: no combine declared anywhere).
    ///
    /// Determinism: `BTreeMap` keyed by `DataId`; iteration in numeric
    /// order.
    #[cfg_attr(feature = "serde", serde(default))]
    pub combine_for_data: BTreeMap<DataId, CombineOp>,
}

/// A resolved kernel signature as the codegen contract needs it: the
/// positional parameter [`ResolvedType`]s, the optional return type,
/// and the kernel's [`Purity`]. Mirrors the fields of
/// [`crate::algo::ResolvedKernel`] the backend actually consumes (the
/// `name` is resolved via the `name_kernels` table; `name_span` is
/// informational only). Kept as a dedicated struct — rather than
/// embedding `ResolvedKernel` — so the serde surface stays minimal (it
/// reuses the feature-gated [`ResolvedType`] derive from TASK-0160 plus
/// the [`Purity`] derive from TASK-0049.10.01, and adds none to the
/// AlgoIR), exactly the [`ConstValue`] / `ResolvedConst` precedent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct KernelSig {
    // DIVERGENCE HAZARD (TASK-0169 review): this is a structural
    // *copy* of `ResolvedKernel` fields, not a projection of the type
    // itself (deliberate — embedding `ResolvedKernel` would drag its
    // `name_span` / hand-written `PartialEq` in). If `ResolvedKernel`
    // ever grows another codegen-relevant field (e.g. a variadic/ABI
    // tag), mirror it here AND in `build_sidecar`'s kernel-sig
    // section, or the EventList-only backend (TASK-0124) will silently
    // diverge with no compile error. `purity` was the first such
    // divergence the hazard actually caught: it was omitted as
    // "irrelevant to argument rendering" but the embedded backend
    // (TASK-0049.10.01) needs it to distinguish an effectful indexed-
    // output peripheral-read kernel (`mic_in[frame] <-- fe_capture()`)
    // from a pure indexed compute — structurally identical Fire
    // shapes that must lower differently. It is now mirrored below.
    /// Positional parameter types, in declaration order. The i-th
    /// entry types the i-th `Event::Fire` argument; `is_scalar()`
    /// drives the `(expr) as usize` scalar-arg cast vs the
    /// whole-array dispatch in
    /// `backend_common::render::fire::render_fire_arg`.
    pub params: Vec<ResolvedType>,
    /// Return type: `None` for a unit (`()`) return, `Some(t)` for a
    /// typed return.
    pub ret: Option<ResolvedType>,
    /// Kernel purity ([`Purity::Pure`] / [`Purity::Effectful`]),
    /// mirrored verbatim from [`crate::algo::ResolvedKernel::purity`].
    /// Codegen-relevant for the embedded-pattern backend: an effectful
    /// indexed-output zero-input kernel lowers to a per-frame shim
    /// region-read into the indexed slice rather than the pure
    /// `kernels::<callee>(..)` stub call (TASK-0049.10.01).
    pub purity: Purity,
    /// Overlapping-write accumulator combine identity (TASK-0343.01.01),
    /// mirrored verbatim from [`crate::algo::ResolvedKernel::combine`].
    /// `Some(_)` iff the kernel decl declared `combine = <op>`; `None`
    /// otherwise. This is the per-kernel fact; the consumer-facing,
    /// per-accumulator-data resolution lives in
    /// [`NameSidecar::combine_for_data`].
    ///
    /// serde-default so an older wire payload (no field) deserialises as
    /// `None` (pre-TASK-0343.01.01: no kernel declares a combine).
    #[cfg_attr(feature = "serde", serde(default))]
    pub combine: Option<CombineOp>,
}

/// A resolved const as the codegen contract needs it: its evaluated
/// value plus its declared scalar type (for literal-suffix / cast
/// selection). Mirrors the fields of [`crate::algo::ResolvedConst`]
/// the backend actually consumes (the `name` is the map key).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ConstValue {
    /// Declared scalar type of the const.
    pub ty: ScalarType,
    /// Evaluated integer value (const evaluator is integer-only).
    pub value: i64,
}

/// A typed error from [`build_sidecar`].
///
/// ## Why this is an error, not a `panic!` (TASK-0170)
///
/// `build_sidecar` is *not* a "cannot happen for link-valid IR"
/// internal-invariant guard like the `name<->id desync` `unwrap_or_else`
/// panics elsewhere in this file (those genuinely cannot fire — the
/// names were enumerated *from* the same `linked.algo` maps a moment
/// earlier). [`SameNameLoopBoundConflict`](Self::SameNameLoopBoundConflict)
/// is *reachable from a valid Nuc program*: two sequential sibling
/// loops reusing one variable name with different bounds — e.g.
/// `for i : 0..N { c[i] <-- f(...) }  for i : 0..M { d[i] <-- f(...) }`
/// (distinct data so single-assignment holds) — parse, lower, link,
/// and `build_acfg` *accept it* (lowering only rejects a loop var
/// shadowing a declared const/data/kernel; PRD §6.2.3 lets loop
/// variables share one namespace and shadow at their loop). But
/// `ACFG::name_iter_vars` assigns *one* [`IterVar`] per *name*
/// (`acfg.rs` enumerates the unique loop-var names), so both loops
/// collapse onto one `Event::Loop.iter_var` / one `loop_bounds`
/// entry and the shared key cannot represent two different bounds.
///
/// Per fail-fast discipline this is surfaced as a clean typed error
/// (driver prints `nucleus: error: ...`), not a process-aborting
/// `panic!`, exactly like
/// [`crate::passes::block_transform::BlockTransformError`]. The
/// *proper* long-term fix — giving such loops distinct `IterVar`
/// identity so the program *compiles* — is the deeper ACFG/Event
/// redesign tracked as **TASK-0171** (depends on this task), out of
/// TASK-0170's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarError {
    /// Two loops in the program share a variable `name` (hence one
    /// [`IterVar`]) but declare *different* source bounds. A shared
    /// `Event::Loop.iter_var` / `loop_bounds` entry cannot represent
    /// both, so an EventList-only backend (TASK-0124) would otherwise
    /// silently lose one loop's bounds. Carries the offending name and
    /// both bound pairs verbatim so the diagnostic is actionable
    /// without re-reading the source.
    SameNameLoopBoundConflict {
        /// The reused loop-variable name (e.g. `i`).
        var: String,
        /// The bounds kept from the first occurrence in source order.
        first: LoopBound,
        /// The conflicting bounds of a later same-named loop.
        second: LoopBound,
    },
}

impl std::fmt::Display for SidecarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SidecarError::SameNameLoopBoundConflict { var, first, second } => write!(
                f,
                "loop variable `{var}` is reused by two loops with \
                 DIFFERENT bounds ({:?}..{:?} vs {:?}..{:?}); loop \
                 variables share one namespace (PRD §6.2.3) so both \
                 loops map to a single iteration-variable identity, \
                 which cannot carry two distinct bound pairs. Rename \
                 one loop's variable, or give them matching bounds. \
                 (Distinct-identity support for this case is tracked as future work.)",
                first.lo, first.hi, second.lo, second.hi
            ),
        }
    }
}

impl std::error::Error for SidecarError {}

impl NameSidecar {
    /// Number of elements a backend must allocate for `data`
    /// (`vec![<zero>; N]`): the product of its dimensions, or `1`
    /// for a scalar (`dims == []`). `None` if `data` is not in the
    /// table (caller passed a `DataId` from a different ACFG — a
    /// programming error worth surfacing, not defaulting).
    ///
    /// Note an empty `dims` yields `1` (the empty product), which is
    /// correct: a scalar is one element. A *dimension* of `0` would
    /// yield `0`; that is faithfully reported (a zero-length array is
    /// a real, if degenerate, shape — do not paper over it).
    pub fn alloc_len(&self, data: DataId) -> Option<usize> {
        self.data_types
            .get(&data)
            .map(|t| t.dims.iter().product::<usize>())
    }

    /// The [`ResolvedType`] of `data`, or `None` if absent.
    pub fn data_type(&self, data: DataId) -> Option<&ResolvedType> {
        self.data_types.get(&data)
    }

    /// The [`KernelSig`] of `kernel`, or `None` if absent (caller
    /// passed a [`KernelId`] from a different ACFG — a programming
    /// error worth surfacing, not defaulting). Join key is the same
    /// [`KernelId`] `Event::Fire { kernel, .. }` carries.
    pub fn kernel_sig(&self, kernel: KernelId) -> Option<&KernelSig> {
        self.kernel_sigs.get(&kernel)
    }

    /// The [`XferFacts`] for transfer `seq`, or `None` if absent (no
    /// cross-worker Push/Wait pair carries that seq). Join key is the
    /// same [`SeqTag`] `Event::Push { seq, .. }` / `Event::Wait { seq,
    /// .. }` carry. An absent entry is a real contract gap for a seq
    /// that DOES appear in the EventList (the backend should fail loud,
    /// as `event_plan` and the pthreads-async ring sizing do) — it is
    /// NOT a default-able miss.
    pub fn xfer_facts_for(&self, seq: SeqTag) -> Option<&XferFacts> {
        self.xfer_facts.get(&seq)
    }

    /// The buffer capacity for transfer `seq`, or `None` if the seq has
    /// no [`XferFacts`] entry. Thin convenience over
    /// [`xfer_facts_for`](Self::xfer_facts_for) for the channel-sizing
    /// consumers (`event_plan` and the pthreads-async ring sizing) that
    /// previously read the standalone `transfer_buffer_for_seq` map.
    /// (`tcp_plan` is sync/buffer=1 and never did a sizing lookup.)
    pub fn xfer_buffer(&self, seq: SeqTag) -> Option<u64> {
        self.xfer_facts.get(&seq).map(|f| f.buffer)
    }

    /// The transport-path hint for transfer `seq`, defaulting to
    /// [`TransportMode::Pio`] when the seq has no [`XferFacts`] entry —
    /// preserving the embedded backend's pre-TASK-0438.02 "absent ⇒ PIO"
    /// contract that the 02-split-add / 14-hearing-aid byte-exact gate
    /// depends on.
    pub fn xfer_transport(&self, seq: SeqTag) -> TransportMode {
        self.xfer_facts
            .get(&seq)
            .map(|f| f.transport)
            .unwrap_or(TransportMode::Pio)
    }

    /// The notification mode for transfer `seq`, defaulting to
    /// [`NotifyMode::Default`] (schedule stated no preference / no entry)
    /// — the per-seq `notify` surface threaded by TASK-0455.08; its
    /// honouring consumer is TASK-0455.02.
    pub fn xfer_notify(&self, seq: SeqTag) -> NotifyMode {
        self.xfer_facts
            .get(&seq)
            .map(|f| f.notify)
            .unwrap_or(NotifyMode::Default)
    }
}

/// Build the codegen-contract sidecar from a linked program and the
/// ACFG name tables it produced.
///
/// Pure and additive: it reads `linked.algo` (consts, data, kernels,
/// source `for` statements) and the `acfg`'s deterministic name maps,
/// and builds four `BTreeMap`s. It does **not** mutate the ACFG, touch
/// `eval_const`, or change `ACFGNode::Repeat`. Call it once after
/// `build_acfg` (the name tables it keys against are fixed there;
/// `block_transform` only *adds* synthetic tile iter-vars, which
/// have no source `for` and so simply have no `loop_bounds` entry —
/// a backend uses the concrete `Event::Loop.range` for those, which
/// is correct since a synthesised tile loop has no source form).
///
/// ### Keying invariant
///
/// `data_types` is keyed by the *same* [`DataId`] `build_acfg`
/// assigned (`acfg.name_data`), `loop_bounds` by the same
/// [`IterVar`] (`acfg.name_iter_vars`), and `kernel_sigs` by the
/// same [`KernelId`] (`acfg.name_kernels`). This is what lets a
/// backend join the sidecar to `Event::Alloc.data` /
/// `DataSlice.data`, `Event::Loop.iter_var`, and `Event::Fire.kernel`
/// with no name round-trip.
///
/// ### Errors
///
/// Returns [`SidecarError::SameNameLoopBoundConflict`] if the program
/// has two loops that reuse one variable name with *different* bounds
/// (a valid-Nuc input that the shared-`IterVar` model cannot
/// represent; TASK-0170). This is a clean typed error, never a
/// `panic!` — the EventList-only backend (TASK-0124) must surface it
/// via the driver, not abort. The internal name<->id desync guards
/// remain `panic!`s: those are unreachable for link-valid IR.
pub fn build_sidecar(
    linked: &LinkedIR,
    acfg: &crate::acfg::ACFG,
) -> Result<NameSidecar, SidecarError> {
    // (a) Per-DataId ResolvedType. Invert via the ACFG's name_data
    //     so the key is the canonical DataId the EventList uses, not
    //     an ad-hoc re-enumeration (single source of truth for
    //     name<->id is the ACFG).
    let mut data_types: BTreeMap<DataId, ResolvedType> = BTreeMap::new();
    for (name, did) in &acfg.name_data {
        // Every name in acfg.name_data was enumerated FROM
        // linked.algo.data (build_acfg), so the lookup is total. A
        // miss is a compiler-internal invariant break, not user
        // input — fail loud with context rather than silently drop a
        // symbol the backend will then fail to size.
        let rd = linked.algo.data.get(name).unwrap_or_else(|| {
            panic!(
                "sidecar: data symbol `{name}` is in ACFG name_data but \
                 not in linked.algo.data — name<->id table desync"
            )
        });
        data_types.insert(*did, rd.ty.clone());
    }

    // (b) Const name -> (ScalarType, value). Copied verbatim from
    //     the already-resolved AlgoIR consts (integer-evaluated at
    //     lower time).
    let consts: BTreeMap<String, ConstValue> = linked
        .algo
        .consts
        .iter()
        .map(|(name, rc)| {
            (
                name.clone(),
                ConstValue {
                    ty: rc.ty.clone(),
                    value: rc.value,
                },
            )
        })
        .collect();

    // (c) Symbolic loop bounds. Walk the SOURCE statements (where
    //     the unevaluated lo/hi still exist) and key by the IterVar
    //     build_acfg assigned to that loop-var name. This is the
    //     additive capture of exactly the expression eval_const
    //     destroys at acfg.rs ~694-697 — captured here in parallel,
    //     WITHOUT stopping the fold.
    let mut loop_bounds: BTreeMap<IterVar, LoopBound> = BTreeMap::new();
    collect_loop_bounds(&linked.algo.stmts, &acfg.name_iter_vars, &mut loop_bounds)?;

    // (d) Per-KernelId signature. Invert acfg.name_kernels so the key
    //     is the canonical KernelId Event::Fire carries — exactly the
    //     name_data -> data_types inversion in (a), mirrored for
    //     kernels. We copy params + ret + purity (the codegen-relevant
    //     fields of ResolvedKernel); `name` is the map's resolution key
    //     and `name_span` is informational only. `purity` is mirrored
    //     because the embedded-pattern backend distinguishes an
    //     effectful indexed-output peripheral read from a pure indexed
    //     compute on it (TASK-0049.10.01).
    let mut kernel_sigs: BTreeMap<KernelId, KernelSig> = BTreeMap::new();
    for (name, kid) in &acfg.name_kernels {
        // Every name in acfg.name_kernels was enumerated FROM
        // linked.algo.kernels (build_acfg), so the lookup is total. A
        // miss is a compiler-internal invariant break, not user
        // input — fail loud with context rather than silently drop a
        // kernel the backend will then fail to type-cast args for.
        let rk = linked.algo.kernels.get(name).unwrap_or_else(|| {
            panic!(
                "sidecar: kernel `{name}` is in ACFG name_kernels but \
                 not in linked.algo.kernels — name<->id table desync"
            )
        });
        kernel_sigs.insert(
            *kid,
            KernelSig {
                params: rk.params.clone(),
                ret: rk.ret.clone(),
                purity: rk.purity,
                // TASK-0343.01.01: mirror the accumulator combine
                // identity so the codegen contract can resolve it
                // per-data (see combine_for_data below).
                combine: rk.combine,
            },
        );
    }

    // (e) Per-worker partition ranges (TASK-0212). Forwarded verbatim
    //     from the ACFG sidecar `partition_worker_ranges`, which the
    //     `passes::partition_workers` pass populated by consuming
    //     `loop X : partition=workers` schedule directives. Empty for
    //     ACFGs whose schedule carries no `partition=workers`
    //     directive — backends then see the pre-TASK-0212 source-range
    //     `Event::Loop.range` projection and the symbolic
    //     `loop_bounds` entry as before. A `.clone()` because the
    //     ACFG and the sidecar are independent owners of their
    //     respective copies (the ACFG flows through the rest of the
    //     middle-end; the sidecar flows out to codegen).
    let partition_worker_ranges = acfg.partition_worker_ranges.clone();

    // (f) Per-SeqTag UNIFIED transfer facts (TASK-0455.08; subsumes the
    //     former parallel transfer_buffer_for_seq (TASK-0233) +
    //     transfer_transport_for_seq (TASK-0438.02) maps). ONE walk over
    //     the post-pass ACFG tree (after transfer_inject populated the
    //     XferPlaceholder nodes) keys policy.{buffer,transport,notify}
    //     by seq; the pipeline-depth mirror is then layered on from the
    //     ACFG's pipeline_depth_for_seq (the Petri/initial-marking source
    //     of truth — NOT a policy field, see XferFacts::pipeline_depth).
    //     A Push and its matching Wait share one seq + one policy, so the
    //     second insertion under a given key is idempotent across the two
    //     endpoints — same invariant the per-fact collectors relied on.
    let mut xfer_facts: BTreeMap<SeqTag, XferFacts> = BTreeMap::new();
    collect_xfer_facts(&acfg.root, &acfg.pipeline_depth_for_seq, &mut xfer_facts);

    // (g) Per-(KernelId, IterVar) halo widths (TASK-0260 Stage 1).
    //     Forwarded verbatim from the ACFG sidecar `halo_widths`, which
    //     the `passes::halo_inference` pass populated by walking
    //     `linked.algo.stmts` for `kernel(grid[iv + b], ...)`-shaped
    //     access patterns. Empty for ACFGs whose kernels don't exercise
    //     affine `iv + b` reads. A `.clone()` for the same independent-
    //     ownership reason `partition_worker_ranges` clones above.
    let halo_widths = acfg.halo_widths.clone();

    // (h) Per-(IterVar, DataId, axis) reuse widths (TASK-0261 Stage 1).
    //     Forwarded verbatim from the ACFG sidecar `reuse_widths`,
    //     which the `passes::reuse_inference` pass populated by walking
    //     every `reuse`-tagged loop's body for affine `iv + b` DataRef
    //     accesses + asserting contiguous offset sets. Empty for ACFGs
    //     whose schedules carry no `reuse` directives. A `.clone()` for
    //     the same independent-ownership reason `partition_worker_ranges`
    //     clones above.
    let reuse_widths = acfg.reuse_widths.clone();

    // (i) Per-outer-IterVar partition pairing (TASK-0264 cycle 113,
    //     AC#1). Forwarded verbatim from the ACFG sidecar
    //     `partition_pairs`, populated by `passes::partition_blocks2d`
    //     to record the (outer_iv → inner_iv) coupling of every
    //     blocks2d directive. Empty for ACFGs whose schedule carries
    //     no `partition=blocks2d` directives — backends pre-TASK-0289
    //     simply do not read it.
    let partition_pairs = acfg.partition_pairs.clone();

    // (j) Per-outer-IterVar `(rows, cols)` grid shape (TASK-0264
    //     cycle 113, AC#2). Forwarded verbatim from the ACFG sidecar
    //     `grid_shape_for_outer_iv`, populated by
    //     `passes::partition_blocks2d` from the decompose_grid result.
    //     Empty for ACFGs whose schedule carries no
    //     `partition=blocks2d` directives.
    let grid_shape_for_outer_iv = acfg.grid_shape_for_outer_iv.clone();

    // (k) Cumulative cross-iteration data symbols (TASK-0341.02.02.01.03,
    //     cycle 213). Walk the source statements: a data symbol is
    //     cumulative iff it is the LHS of a `<--` nested inside a `for`
    //     loop AND reads itself on the RHS at an index expression that
    //     DIFFERS from the LHS index along that loop's iteration axis.
    //     Resolve names -> DataId via acfg.name_data. Non-empty only for
    //     algorithms with a cross-iteration self-read: 16-jacobi (`field`)
    //     AND 11-game-of-life (`grid`) both match the shape. game-of-life
    //     ships no `partition=` schedule, so its cumulative classification
    //     is inert downstream (the band-rewrite / hoist passes are
    //     partition-guarded no-ops) — but the SET is not "16-jacobi only".
    //     Pinned by `cumulative_tests::{jacobi_field,game_of_life_grid}_*`.
    //
    //     A cumulative name is always a declared data symbol (it is the
    //     LHS of a `<--` dataflow, which link-valid IR requires to be in
    //     `linked.algo.data`, hence in `acfg.name_data` — build_acfg
    //     enumerated name_data FROM that same data table). A name present
    //     here but absent from `acfg.name_data` is therefore a
    //     compiler-internal name<->id desync, NOT user input. Resolve it
    //     fail-loud with context — identical in kind to the (a)/(d)/(j)
    //     `unwrap_or_else(panic!)` guards — never silently drop it. A
    //     dropped cumulative symbol would skip the COPY-not-accumulate
    //     exclusion (the xN-double-count protection that is
    //     value-correctness-load-bearing; TASK-0459). Pinned loud by the
    //     `sidecar_build_desync` integration test. (Block (l)
    //     combine_for_data is the one name<->id site that legitimately
    //     `filter_map`s instead — its accumulator walk can name a symbol
    //     the partition pass elided, and the consuming gate re-derives +
    //     reports the miss; that justification does NOT apply to a
    //     declared cumulative LHS.)
    let mut cumulative_names: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    collect_cumulative_data_names(&linked.algo.stmts, &[], &mut cumulative_names);
    let cumulative_data: std::collections::BTreeSet<DataId> = cumulative_names
        .iter()
        .map(|name| {
            *acfg.name_data.get(name).unwrap_or_else(|| {
                panic!(
                    "sidecar: cumulative data symbol `{name}` is classified \
                     from linked.algo.stmts but is not in ACFG name_data — \
                     cumulative<->id table desync (dropping it would skip the \
                     COPY-not-accumulate exclusion; TASK-0459)"
                )
            })
        })
        .collect();

    // (m) Data-declaration order as DataIds (TASK-0049.10.06; formerly
    //     mislabeled as a second "(j)" — TASK-0459 notes and commit
    //     ca270c8 cite this block as the data_decl_order guard). Map
    //     each name in `linked.algo.data_decl_order` (already in source
    //     declaration order, built by `lower_data`) through
    //     `acfg.name_data` to its canonical DataId. Mirrors the (a)
    //     name_data -> data_types inversion, but iterating the
    //     ORDER-bearing Vec instead of the alphabetical map so the
    //     resulting Vec<DataId> preserves declaration order. A name
    //     present here but absent from `acfg.name_data` is a
    //     compiler-internal desync (build_acfg enumerated name_data FROM
    //     the same linked.algo.data the order vec was built alongside) —
    //     fail loud with context, exactly like the (a)/(d) blocks, never
    //     silently drop a symbol the backend's byte layout depends on.
    let data_decl_order: Vec<DataId> = linked
        .algo
        .data_decl_order
        .iter()
        .map(|name| {
            *acfg.name_data.get(name).unwrap_or_else(|| {
                panic!(
                    "sidecar: data symbol `{name}` is in \
                     linked.algo.data_decl_order but not in ACFG \
                     name_data — decl-order<->id table desync"
                )
            })
        })
        .collect();

    // (l) Per-accumulator-DataId combine identity (TASK-0343.01.01).
    //     For each algorithm-level accumulator (`acc[..] <-- k(acc[..],
    //     ...)` LHS-appears-in-RHS), resolve the OWNING kernel `k` (the
    //     top-level RHS `Call` callee) and copy its declared `combine`
    //     attribute keyed by the accumulator's DataId. A kernel that
    //     does NOT declare `combine` produces NO entry — the gate +
    //     render path treat an absent entry as a fail-loud soundness
    //     reject (no silent assume-sum). Resolve names -> DataId via
    //     acfg.name_data; an accumulator whose name is missing there is
    //     a desync (same fail-loud rationale as block (a)/(d)) but is
    //     filtered (filter_map) rather than panicked because the
    //     accumulator-shape walk runs over source stmts that may name a
    //     symbol the partition pass elided — the consuming gate
    //     re-derives the accumulator set from the SAME walk and reports
    //     the missing-combine reject there with full context.
    let combine_for_data: BTreeMap<DataId, CombineOp> =
        collect_combine_for_accumulators(&linked.algo, &acfg.name_data);

    Ok(NameSidecar {
        data_types,
        consts,
        loop_bounds,
        kernel_sigs,
        partition_worker_ranges,
        xfer_facts,
        halo_widths,
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
        cumulative_data,
        data_decl_order,
        combine_for_data,
    })
}

// ACFG-walk + statement-walk collector helpers (combine-for-accumulators,
// cumulative-data, transfer-buffer/transport, loop-bounds). Split into a
// child module (TASK-0343.01.01) to keep this file under the 1000-LoC
// mega-file fence; re-exported into the `sidecar` namespace so
// `build_sidecar` above and the `cumulative_tests` child module
// (`super::collect_cumulative_data_names`) reach them unchanged.
mod collectors;
pub(crate) use collectors::collect_cumulative_data_names;
use collectors::{collect_combine_for_accumulators, collect_loop_bounds, collect_xfer_facts};

// The cumulative-array discriminator's pin tests live in the
// `cumulative_tests` child module (split out under
// `sidecar/cumulative_tests.rs` per TASK-0383 to keep this file under
// the 1000-LoC mega-file fence). As a child of `sidecar` it reaches
// `collect_cumulative_data_names` via `use super::...`.
#[cfg(test)]
mod cumulative_tests;
