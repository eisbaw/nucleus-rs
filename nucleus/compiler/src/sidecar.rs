//! Codegen-contract sidecar: per-`DataId` types, const values, and
//! symbolic loop bounds for **EventList-only** backends (TASK-0160).
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
//! [`IterVar`]) or a `String` (const name). Iteration is sorted;
//! [`build_sidecar`] is a pure function of `(LinkedIR, ACFG name
//! tables)`. The same inputs yield a byte-identical sidecar, and the
//! serde wire form (feature-gated, like `event`/`contract`) is
//! stable for golden tests.

use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::algo::{IrExpr, ResolvedType, ScalarType};
use crate::event::{DataId, IterVar};
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
///   `Event::Loop { iter_var, .. }` by the shared [`IterVar`].
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
}

/// Build the codegen-contract sidecar from a linked program and the
/// ACFG name tables it produced.
///
/// Pure and additive: it reads `linked.algo` (consts, data, source
/// `for` statements) and the `acfg`'s deterministic name maps, and
/// builds three `BTreeMap`s. It does **not** mutate the ACFG, touch
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
/// assigned (`acfg.name_data`), and `loop_bounds` by the same
/// [`IterVar`] (`acfg.name_iter_vars`). This is what lets a backend
/// join the sidecar to `Event::Alloc.data` / `DataSlice.data` and
/// `Event::Loop.iter_var` with no name round-trip.
pub fn build_sidecar(linked: &LinkedIR, acfg: &crate::acfg::ACFG) -> NameSidecar {
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
    collect_loop_bounds(&linked.algo.stmts, &acfg.name_iter_vars, &mut loop_bounds);

    NameSidecar {
        data_types,
        consts,
        loop_bounds,
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
/// keep the FIRST occurrence in source order and assert the bounds
/// agree if a later same-named loop differs, rather than silently
/// overwriting — a same-named loop with *different* bounds would be
/// an ambiguity the EventList's shared `iter_var` also cannot
/// represent, so surfacing it here (loud, with context) is the
/// honest behaviour. No e2e example exercises this; recorded as a
/// known limitation (see TASK-0160 notes / follow-up).
fn collect_loop_bounds(
    stmts: &[crate::algo::IrStmt],
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeMap<IterVar, LoopBound>,
) {
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
                    panic!(
                        "sidecar: loop var `{var}` reused with DIFFERENT \
                         bounds ({existing:?} vs {bound:?}); a shared \
                         IterVar cannot represent both. Known limitation \
                         (TASK-0160) — no e2e example hits this."
                    );
                }
                Some(_) => { /* same name, same bounds: idempotent */ }
            }
            collect_loop_bounds(body, name_iter_vars, out);
        }
    }
}
