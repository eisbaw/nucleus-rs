//! Codegen-contract sidecar: per-`DataId` types, const values,
//! symbolic loop bounds (TASK-0160), and per-`KernelId` signatures
//! (TASK-0169) for **EventList-only** backends.
//!
//! ## Why this exists
//!
//! The per-worker `EventList` ([`crate::event::Event`]) is, after
//! TASK-0156 (value bindings) and TASK-0159 ([`Event::Loop`]
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
//!    (and *panics* on a non-const bound). `Event::Loop` mirrors that
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
//!    [`ResolvedType`]s of every kernel (TASK-0169). The
//!    pthreads-sync backend's `render_call_arg` decides a scalar
//!    argument cast — an iter-var-derived scalar expression fed to a
//!    scalar kernel param renders as `(expr) as usize` — by reading
//!    `algo.kernels[callee].params[i]` and testing
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

use crate::algo::{IrExpr, ResolvedType, ScalarType};
use crate::event::{DataId, IterVar, KernelId, SeqTag, WorkerId};
use crate::link::LinkedIR;

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
    /// AlgoIR-walking `render_call_arg` does against
    /// `algo.kernels[callee].params[i]` today — without the AlgoIR.
    /// This is the last `algo.kernels` read pthreads-sync codegen
    /// has; with this table the contract is fully AlgoIR-free.
    pub kernel_sigs: BTreeMap<KernelId, KernelSig>,

    /// Per-worker loop-range override for loops carrying a
    /// `partition=workers` schedule directive (TASK-0212). Mirrors
    /// [`crate::acfg::ACFG::partition_worker_ranges`] verbatim — the
    /// codegen contract surface for that ACFG sidecar. A backend
    /// rendering an `Event::Loop` for a worker whose `iter_var` has
    /// an entry here MUST prefer the concrete per-worker range over
    /// the symbolic [`loop_bounds`] entry (the symbolic bound names
    /// the SOURCE range, not the partitioned per-worker slice).
    /// Workers not listed in the inner map (e.g. host) and iter vars
    /// without an outer entry fall back to [`loop_bounds`] /
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

    /// Per-SeqTag transfer buffer size — the `transfer DATA : buffer=N`
    /// value the schedule's directive carries through to the matched
    /// Push/Wait pair (TASK-0233). Pthreads-async multi-worker codegen
    /// (TASK-0228 Wave B) needs this to size the per-(DataId,SeqTag)
    /// `Arc<Ring<T>>` instances — the value lives in
    /// `ACFG::XferPlaceholder::policy.buffer` upstream, but the backend
    /// receives only NameSidecar per the EventList contract (TASK-0124),
    /// so it has to ride along the sidecar.
    ///
    /// One entry per matched Push/Wait pair (the seq is unique per
    /// pair; both endpoints share it — `passes::transfer_inject`
    /// guarantees that invariant). Sync transfers also appear here
    /// (their `buffer` defaults to 1 per `TransferPolicy::default`);
    /// async transfers carry the schedule's chosen `buffer=N`.
    ///
    /// Empty for any algorithm that produces no cross-worker
    /// transfers (single-worker or same-worker-only schedules). The
    /// `Event::Push` / `Event::Wait` variants carry `seq: SeqTag` so
    /// a codegen consumer joins this map with the event's seq to
    /// size the ring at runtime.
    ///
    /// Determinism: `BTreeMap` keyed by SeqTag; iteration is in numeric
    /// order. serde-default so an older wire payload (no field)
    /// deserialises as empty.
    #[cfg_attr(feature = "serde", serde(default))]
    pub transfer_buffer_for_seq: BTreeMap<SeqTag, u64>,

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
    /// compat contract as `transfer_buffer_for_seq` (TASK-0233) and
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
}

/// A resolved kernel signature as the codegen contract needs it: the
/// positional parameter [`ResolvedType`]s and the optional return
/// type. Mirrors the fields of [`crate::algo::ResolvedKernel`] the
/// backend actually consumes for argument rendering (the `name` is
/// resolved via the `name_kernels` table; `purity` is irrelevant to
/// codegen). Kept as a dedicated struct — rather than embedding
/// `ResolvedKernel` — so the serde surface stays minimal (it reuses
/// the feature-gated [`ResolvedType`] derive from TASK-0160 and adds
/// none to the AlgoIR), exactly the [`ConstValue`] / `ResolvedConst`
/// precedent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct KernelSig {
    // DIVERGENCE HAZARD (TASK-0169 review): this is a structural
    // *copy* of two `ResolvedKernel` fields, not a projection of the
    // type itself (deliberate — embedding `ResolvedKernel` would drag
    // `Purity`/serde in). If `ResolvedKernel` ever grows another
    // codegen-relevant field (e.g. a variadic/ABI tag), mirror it
    // here AND in `build_sidecar`'s kernel-sig section, or the
    // EventList-only backend (TASK-0124) will silently diverge with
    // no compile error.
    /// Positional parameter types, in declaration order. The i-th
    /// entry types the i-th `Event::Fire` argument; `is_scalar()`
    /// drives the `(expr) as usize` scalar-arg cast vs the
    /// whole-array dispatch in `render_call_arg`.
    pub params: Vec<ResolvedType>,
    /// Return type: `None` for a unit (`()`) return, `Some(t)` for a
    /// typed return.
    pub ret: Option<ResolvedType>,
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
                 (Distinct-identity support for this case is TASK-0171.)",
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
    //     kernels. We copy only params + ret (the codegen-relevant
    //     fields of ResolvedKernel); `purity` is irrelevant to
    //     argument rendering and `name` is the map's resolution key.
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

    // (f) Per-SeqTag transfer buffer (TASK-0233). Walk the post-pass
    //     ACFG tree (after transfer_inject populated the XferPlaceholder
    //     nodes) and key the policy.buffer values by seq. A Push and
    //     its matching Wait share one seq and one buffer value — that
    //     pair-level invariant is established by transfer_inject and
    //     means the map entry is idempotent across the two endpoints.
    let mut transfer_buffer_for_seq: BTreeMap<SeqTag, u64> = BTreeMap::new();
    collect_transfer_buffers(&acfg.root, &mut transfer_buffer_for_seq);

    // (g) Per-(KernelId, IterVar) halo widths (TASK-0260 Stage 1).
    //     Forwarded verbatim from the ACFG sidecar `halo_widths`, which
    //     the `passes::halo_inference` pass populated by walking
    //     `linked.algo.stmts` for `kernel(grid[iv + b], ...)`-shaped
    //     access patterns. Empty for ACFGs whose kernels don't exercise
    //     affine `iv + b` reads. A `.clone()` for the same independent-
    //     ownership reason `partition_worker_ranges` clones above.
    let halo_widths = acfg.halo_widths.clone();

    Ok(NameSidecar {
        data_types,
        consts,
        loop_bounds,
        kernel_sigs,
        partition_worker_ranges,
        transfer_buffer_for_seq,
        halo_widths,
    })
}

/// Walk an ACFG subtree, populating `out` with `(seq -> policy.buffer)`
/// from every `XferPlaceholder` encountered. Mirrors the existing
/// `acfg`-walk pattern (no allocation; in-place fold over the tree).
///
/// Push and Wait endpoints of the same pair share one seq + one
/// policy.buffer, so the second insertion under a given key is
/// idempotent. We accept the redundant write rather than branching
/// on role — simpler + no behavior difference.
fn collect_transfer_buffers(node: &crate::acfg::ACFGNode, out: &mut BTreeMap<SeqTag, u64>) {
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
fn collect_loop_bounds(
    stmts: &[crate::algo::IrStmt],
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeMap<IterVar, LoopBound>,
) -> Result<(), SidecarError> {
    use crate::algo::IrStmt;
    for s in stmts {
        if let IrStmt::For { var, lo, hi, body } = s {
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
