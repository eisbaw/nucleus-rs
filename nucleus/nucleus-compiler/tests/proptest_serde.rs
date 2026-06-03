//! Property-based serde round-trip for the codegen-contract boundary
//! types (TASK-0420).
//!
//! ## Scope
//!
//! This file pins ONE invariant for each of the two serde-bearing
//! contract surfaces:
//!
//! 1. [`nucleus_compiler::event::Event`] — the per-worker EventList
//!    contract (PRD §8.3). Property `event_serde_roundtrip`:
//!    `from_str(to_string(e)) == e` over arbitrary `Event` trees.
//! 2. [`nucleus_compiler::NameSidecar`] — the codegen-contract sidecar
//!    (TASK-0160) carrying partition/halo/reuse/buffer/blocks2d/
//!    cumulative metadata. Property `sidecar_serde_roundtrip`:
//!    `from_str(to_string(s)) == s` over arbitrary `NameSidecar`s.
//!
//! These COMPLEMENT (do NOT replace) the ~40 hand-rolled example
//! round-trips in `tests/event.rs` (the `roundtrip(&Event)` helper) and
//! `tests/sidecar_{reuse,halo,partition_blocks2d,buffer}.rs`. Those pin
//! specific wire shapes and specific values; these drive the same serde
//! impls with randomly generated values — arbitrary nesting, all field
//! combinations, degenerate ranges.
//!
//! ## The break-to-update completeness guard (the whole point)
//!
//! The 40 hand-rolled examples do NOT break-to-update: adding a new
//! `Event` variant (or a new serde-bearing `NameSidecar` field) compiles
//! fine against them, so a variant shipped without round-trip safety
//! slips through silently (memory `project-event-sync-synctag` flags a
//! serde required-field contract-version caveat — serde-contract
//! evolution IS a live risk area). This file closes that gap with two
//! EXHAUSTIVE-BY-CONSTRUCTION generators:
//!
//! - [`event_strategy`] is a `prop_oneof!` with exactly one arm PER
//!   `Event` variant, and [`event_variant_completeness_guard`] is a
//!   `match` over `&Event` with NO wildcard arm. Adding a variant to the
//!   `Event` enum forces a non-exhaustive-match COMPILE ERROR here,
//!   which is the prompt to add the matching `prop_oneof!` arm.
//! - [`namesidecar_strategy`] destructures `NameSidecar { .. }` field by
//!   field (no `..`), and [`sidecar_field_completeness_guard`] does the
//!   same. Adding a serde-bearing field to `NameSidecar` forces a
//!   COMPILE ERROR in both, which is the prompt to extend the generator.
//! - The FIELDLESS tag enums fed by `prop_oneof!` `Just(..)` lists
//!   (`SyncKind`, `ScalarType`, `IrBinOp`, `ViolationKind`) get the same
//!   teeth via sibling wildcard-free `*_completeness_guard` matches — a
//!   `prop_oneof!` list is NOT itself exhaustiveness-checked, so the
//!   guard is what forces the break-to-update on a new variant.
//!
//! That break-to-update guard is the entire value over the 40 existing
//! examples; it does NOT close a correctness gap (the examples already
//! cover every CURRENT variant). Honest value: MODERATE, not high.
//!
//! ## Serde fidelity, NOT semantic validity
//!
//! The generators target serde encode/decode fidelity, not "schedules a
//! real compiler pass would emit". Every field is filled with arbitrary
//! in-range values — including combinations no pass produces (a
//! `check_frame: Some(..)` on a strip-mined inner `Loop`; a `Sync` whose
//! `kind` is the only current `Barrier` but whose participant set is
//! empty; inverted/degenerate `Range`s). Arbitrary combinations are a
//! STRONGER round-trip test, and the `Event` / `NameSidecar` types are a
//! data contract, not a validator (`tests/event.rs` module docs make the
//! same point). Round-trip identity is exact: `Event` and `NameSidecar`
//! both derive `PartialEq, Eq`, so `from_str(to_string(x)) == x` is a
//! byte-exact-semantics assertion (`Range<i64>` has no `Hash` — hence
//! `Event`'s hand-rolled `Hash` — but it serde + `PartialEq`s fine, so
//! ranges round-trip).
//!
//! ## Honest-failure path
//!
//! If proptest surfaces a REAL round-trip failure on a seeded value
//! (some generated `Event` / `NameSidecar` does NOT round-trip — a serde
//! attribute dropping a field, a `Range` edge case, a float-equality
//! issue), that is a P1 finding for the surfacing cycle: STOP, do NOT
//! mask it by constraining the generator away from the failing shape.
//! Instead file a precise prerequisite task (with the failing seed + the
//! exact value), leave the property `#[ignore]`d with a comment pointing
//! at that task, and report a BLOCKED/partial outcome. A found
//! round-trip bug is a SUCCESSFUL cycle; masking it is the cardinal sin.
//! (Mirrors `tests/proptest_petri.rs`' honest-failure discipline.)
//!
//! ## Generator honest limits
//!
//! - **Recursion depth is bounded.** `Event::Loop` bodies and
//!   `ArgBinding::Nested` are recursive; both use a `prop_recursive`
//!   with `depth ≤ 2` and a small node budget (mirrors the depth-≤2
//!   approach in `proptest_petri.rs`' widened generator). Deeper nests
//!   are not generated — a serde-recursion bug only reachable at depth
//!   ≥ 3 would be missed (no evidence such a bug class exists; the serde
//!   derive is depth-agnostic). The `tests/event.rs`
//!   `loop_serde_roundtrip_including_nested_loop` example pins one
//!   hand-built depth-2 nest in addition.
//! - **`i64` / `u64` fields draw from the FULL range** (`any::<i64>()` /
//!   `any::<u64>()`), so `i64::MIN`/`MAX` range bounds, inverted ranges
//!   (`start > end`), and empty ranges (`start == end`) are all in the
//!   input space — exactly the serde edge cases worth hitting.
//! - **`ScalarType` includes `F32`/`F64`** in `ResolvedType.scalar`, but
//!   `ScalarType` is a fieldless enum (a tag, no float PAYLOAD), so there
//!   is no NaN-equality hazard: it serialises as a unit variant string.
//!   No `f32`/`f64` VALUE is carried by either contract type.
//! - **`String` fields** (`ArgBinding::Nested.callee`, `IrExpr::Ident`/
//!   `Call.callee`, `CheckFrame.loop_var`, `NameSidecar.consts` keys) draw
//!   from a small ASCII identifier alphabet, not arbitrary UTF-8. JSON
//!   string escaping is exercised by the punctuation in the alphabet
//!   (`_`); full-Unicode escaping fidelity is serde_json's own contract,
//!   out of scope here.
//! - **Map sizes are small** (0..=3 entries per map, 0..=3 dims, etc.)
//!   so each case stays fast; the round-trip is structural, so size adds
//!   coverage with diminishing returns past a few entries.
//!
//! ## Case count
//!
//! [`PROPTEST_CASES`] = 256 (proptest's default). The round-trip body is
//! cheap (one serialise + one deserialise + one `==`), so 256 cases per
//! property keeps the file fast (well under a second). proptest is
//! seeded deterministically; this repo does NOT commit
//! `proptest-regressions/` (none are tracked, none are `.gitignore`d —
//! the convention is no committed seed files), so neither do we.

use std::collections::BTreeMap;
use std::ops::Range;

use proptest::prelude::*;

use nucleus_compiler::algo::{IndexedRef, IrBinOp, IrCmpOp, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    ArgBinding, BlockTag, CheckFrame, DataId, DataSlice, Event, FireBinding, IterTile, IterVar,
    KernelId, Region, SeqTag, SyncKind, SyncTag, ViolationKind, WorkerId,
};
use nucleus_compiler::passes::reuse_inference::ReuseSlot;
use nucleus_compiler::sidecar::{ConstValue, KernelSig, LoopBound, NameSidecar};

/// Case count floor for both round-trip properties. proptest's default
/// is 256; the round-trip body is cheap so 256 is plenty fast. Raising
/// this is safe (the file stays sub-second well past 256) — kept at the
/// default so the floor is self-documenting.
const PROPTEST_CASES: u32 = 256;

// --------------------------------------------------------------------
// Completeness guards (the break-to-update teeth)
//
// These two functions are NEVER called at runtime; their ONLY job is to
// be a wildcard-free `match` / destructure that fails to compile the
// moment a new `Event` variant or a new serde-bearing `NameSidecar`
// field is added. That compile error is the signal to extend the
// matching generator below. Keeping them as standalone fns (rather than
// inline in the strategy) means the guard reads as an explicit,
// documented contract rather than an incidental match.
// --------------------------------------------------------------------

/// Exhaustiveness teeth for [`Event`]. NO wildcard arm — adding a
/// variant to `Event` breaks this match and forces a matching
/// `event_strategy` arm (AC#1).
#[allow(dead_code)]
fn event_variant_completeness_guard(e: &Event) {
    match e {
        Event::Fire { .. } => {}
        Event::Alloc { .. } => {}
        Event::Push { .. } => {}
        Event::Wait { .. } => {}
        Event::Sync { .. } => {}
        Event::Free { .. } => {}
        Event::Loop { .. } => {}
        // INTENTIONALLY no `_ =>` arm. See module docs: a new Event
        // variant must break-to-update the round-trip generator.
    }
}

/// Exhaustiveness teeth for [`NameSidecar`]. Destructures every field by
/// name with NO `..` — adding a serde-bearing field breaks this and
/// forces a matching `namesidecar_strategy` field (AC#2).
#[allow(dead_code)]
fn sidecar_field_completeness_guard(s: &NameSidecar) {
    let NameSidecar {
        data_types: _,
        consts: _,
        loop_bounds: _,
        kernel_sigs: _,
        partition_worker_ranges: _,
        transfer_buffer_for_seq: _,
        halo_widths: _,
        reuse_widths: _,
        partition_pairs: _,
        grid_shape_for_outer_iv: _,
        cumulative_data: _,
        // INTENTIONALLY no `..`. A new serde-bearing field must
        // break-to-update the round-trip generator.
    } = s;
}

// Exhaustiveness teeth for the FIELDLESS tag enums whose generators are
// `prop_oneof!` lists of `Just(..)` (TASK-0420 review fold-back, architect
// P3). A `prop_oneof!` list is NOT exhaustiveness-checked by the compiler,
// so adding a variant to one of these enums would SILENTLY leave its
// generator stale (the new variant simply never generated). These
// wildcard-free `match` guards restore the break-to-update property: a new
// variant fails to compile HERE, which is the prompt to extend the
// matching `prop_oneof!` generator. Mirrors
// `event_variant_completeness_guard`. `#[allow(dead_code)]` — never called;
// exhaustiveness is checked at type-check regardless of liveness.
#[allow(dead_code)]
fn sync_kind_completeness_guard(k: &SyncKind) {
    match k {
        SyncKind::Barrier => {}
        // INTENTIONALLY no `_ =>`. A new SyncKind variant must
        // break-to-update the `sync_kind` generator.
    }
}

#[allow(dead_code)]
fn scalar_type_completeness_guard(t: &ScalarType) {
    match t {
        ScalarType::Usize
        | ScalarType::Isize
        | ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::U64
        | ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::I64
        | ScalarType::F32
        | ScalarType::F64
        | ScalarType::Bool => {}
        // INTENTIONALLY no `_ =>`. A new ScalarType variant must
        // break-to-update the `scalar_type` generator.
    }
}

#[allow(dead_code)]
fn ir_bin_op_completeness_guard(op: &IrBinOp) {
    match op {
        IrBinOp::Add | IrBinOp::Sub | IrBinOp::Mul | IrBinOp::Div | IrBinOp::Mod => {}
        // INTENTIONALLY no `_ =>`. A new IrBinOp variant must
        // break-to-update the `ir_bin_op` generator.
    }
}

#[allow(dead_code)]
fn ir_cmp_op_completeness_guard(op: &IrCmpOp) {
    match op {
        IrCmpOp::Le
        | IrCmpOp::Lt
        | IrCmpOp::Eq
        | IrCmpOp::Ne
        | IrCmpOp::Gt
        | IrCmpOp::Ge => {}
        // INTENTIONALLY no `_ =>`. A new IrCmpOp variant must
        // break-to-update the `ir_cmp_op` generator (TASK-0341.02.01.02).
    }
}

#[allow(dead_code)]
fn violation_kind_completeness_guard(v: &ViolationKind) {
    match v {
        ViolationKind::Panic | ViolationKind::Log | ViolationKind::Count => {}
        // INTENTIONALLY no `_ =>`. A new ViolationKind variant must
        // break-to-update the `violation_kind` generator.
    }
}

// --------------------------------------------------------------------
// Leaf / scalar strategies
// --------------------------------------------------------------------

/// A small ASCII identifier (the alphabet is `[a-z_][a-z0-9_]{0,5}`).
/// Used for every `String` field; see module "Generator honest limits".
fn ident() -> impl Strategy<Value = String> {
    "[a-z_][a-z0-9_]{0,5}".prop_map(|s| s.to_string())
}

/// An arbitrary half-open `Range<i64>` over the FULL `i64` range so
/// inverted (`start > end`), empty (`start == end`), and extreme-bound
/// ranges are all in the input space (the serde edge cases worth hitting
/// — `Range` serialises as `{ "start": .., "end": .. }`).
fn range_i64() -> impl Strategy<Value = Range<i64>> {
    (any::<i64>(), any::<i64>()).prop_map(|(a, b)| a..b)
}

/// `ScalarType` — a fieldless tag enum. `prop_oneof!` over all 13
/// variants so the (unit-variant) serde encoding of each is exercised.
/// NB a `prop_oneof!` list is NOT exhaustiveness-checked, so this list
/// alone would go stale silently if a variant were added — the
/// break-to-update teeth live in `scalar_type_completeness_guard` (a
/// wildcard-free `match`), which fails to compile on a new variant and
/// prompts extending this list. Defence in depth; `ScalarType` is not the
/// task's headline contract.
fn scalar_type() -> impl Strategy<Value = ScalarType> {
    prop_oneof![
        Just(ScalarType::Usize),
        Just(ScalarType::Isize),
        Just(ScalarType::U8),
        Just(ScalarType::U16),
        Just(ScalarType::U32),
        Just(ScalarType::U64),
        Just(ScalarType::I8),
        Just(ScalarType::I16),
        Just(ScalarType::I32),
        Just(ScalarType::I64),
        Just(ScalarType::F32),
        Just(ScalarType::F64),
        Just(ScalarType::Bool),
    ]
}

/// `ResolvedType` = scalar + 0..=3 dimensions (`usize` per dim).
fn resolved_type() -> impl Strategy<Value = ResolvedType> {
    (scalar_type(), prop::collection::vec(any::<usize>(), 0..=3))
        .prop_map(|(scalar, dims)| ResolvedType { scalar, dims })
}

// --------------------------------------------------------------------
// IrExpr (carried verbatim inside ArgBinding::Scalar and DataSlice
// indices). Bounded-depth recursive — a serde-recursion bug at depth
// >2 is out of scope (see module limits).
// --------------------------------------------------------------------

fn ir_bin_op() -> impl Strategy<Value = IrBinOp> {
    prop_oneof![
        Just(IrBinOp::Add),
        Just(IrBinOp::Sub),
        Just(IrBinOp::Mul),
        Just(IrBinOp::Div),
        Just(IrBinOp::Mod),
    ]
}

/// Arbitrary [`IrCmpOp`] (TASK-0341.02.01.02 / S2). Exhaustive over all
/// six relational variants so the serde round-trip / determinism gate
/// exercises every operator (comparison on ints is exact / order-free,
/// so there is no determinism concern in the operator itself; this only
/// pins the serialise→deserialise identity).
fn ir_cmp_op() -> impl Strategy<Value = IrCmpOp> {
    prop_oneof![
        Just(IrCmpOp::Le),
        Just(IrCmpOp::Lt),
        Just(IrCmpOp::Eq),
        Just(IrCmpOp::Ne),
        Just(IrCmpOp::Gt),
        Just(IrCmpOp::Ge),
    ]
}

/// Arbitrary [`IrExpr`] tree, bounded depth ≤ 2 over the recursive arms
/// (`Neg`, `BinOp`, `Compare`, `DataRef` indices, `Call` args).
/// Exhaustive over all SEVEN `IrExpr` variants at the leaf + recursive
/// levels (the seventh is the bool-valued `Compare`, TASK-0341.02.01.02).
fn ir_expr() -> impl Strategy<Value = IrExpr> {
    let leaf = prop_oneof![
        any::<i64>().prop_map(IrExpr::IntLit),
        ident().prop_map(IrExpr::Ident),
    ];
    leaf.prop_recursive(2, 16, 3, |inner| {
        prop_oneof![
            inner.clone().prop_map(|e| IrExpr::Neg(Box::new(e))),
            (ir_bin_op(), inner.clone(), inner.clone())
                .prop_map(|(op, a, b)| IrExpr::BinOp(op, Box::new(a), Box::new(b))),
            // TASK-0341.02.01.02 / S2: cover the bool-valued Compare node
            // in the round-trip strategy too.
            (ir_cmp_op(), inner.clone(), inner.clone())
                .prop_map(|(op, a, b)| IrExpr::Compare(op, Box::new(a), Box::new(b))),
            (ident(), prop::collection::vec(inner.clone(), 0..=2))
                .prop_map(|(name, indices)| IrExpr::DataRef(IndexedRef { name, indices })),
            (ident(), prop::collection::vec(inner, 0..=2))
                .prop_map(|(callee, args)| IrExpr::Call { callee, args }),
        ]
    })
}

// --------------------------------------------------------------------
// Event field sub-strategies
// --------------------------------------------------------------------

fn iter_var() -> impl Strategy<Value = IterVar> {
    any::<u64>().prop_map(IterVar)
}

/// [`IterTile`] = ordered `(IterVar, Range<i64>)` pairs, incl. the empty
/// tile (rank 0, a non-iterated firing) and degenerate/inverted ranges.
fn iter_tile() -> impl Strategy<Value = IterTile> {
    prop::collection::vec((iter_var(), range_i64()), 0..=3).prop_map(IterTile::new)
}

fn data_slice() -> impl Strategy<Value = DataSlice> {
    (any::<u64>(), prop::collection::vec(ir_expr(), 0..=2))
        .prop_map(|(data, indices)| DataSlice {
            data: DataId(data),
            indices,
        })
}

/// Arbitrary [`ArgBinding`], bounded depth ≤ 2 over the recursive
/// `Nested` arm. Exhaustive over all three variants.
fn arg_binding() -> impl Strategy<Value = ArgBinding> {
    let leaf = prop_oneof![
        data_slice().prop_map(ArgBinding::Data),
        ir_expr().prop_map(ArgBinding::Scalar),
    ];
    leaf.prop_recursive(2, 8, 3, |inner| {
        (ident(), prop::collection::vec(inner, 0..=2))
            .prop_map(|(callee, args)| ArgBinding::Nested { callee, args })
    })
}

fn fire_binding() -> impl Strategy<Value = FireBinding> {
    (
        prop::collection::vec(arg_binding(), 0..=3),
        prop::option::of(data_slice()),
    )
        .prop_map(|(inputs, output)| FireBinding { inputs, output })
}

fn block_tag() -> impl Strategy<Value = BlockTag> {
    (any::<i64>(), any::<i64>(), any::<bool>()).prop_map(|(block_n, num_full, is_partial)| {
        BlockTag {
            block_n,
            num_full,
            is_partial,
        }
    })
}

fn violation_kind() -> impl Strategy<Value = ViolationKind> {
    prop_oneof![
        Just(ViolationKind::Panic),
        Just(ViolationKind::Log),
        Just(ViolationKind::Count),
    ]
}

fn check_frame() -> impl Strategy<Value = CheckFrame> {
    (any::<u64>(), violation_kind(), ident()).prop_map(|(latency_max_ns, on_violation, loop_var)| {
        CheckFrame {
            latency_max_ns,
            on_violation,
            loop_var,
        }
    })
}

// --------------------------------------------------------------------
// Event — EXHAUSTIVE-BY-CONSTRUCTION generator (AC#1)
//
// One `prop_oneof!` arm per `Event` variant. The recursive `Loop` arm
// carries a bounded-depth body. `event_variant_completeness_guard`
// above is the compile-time teeth that force this list to stay
// complete.
// --------------------------------------------------------------------

/// Non-recursive `Event` leaf arms (every variant except `Loop`).
fn event_leaf() -> impl Strategy<Value = Event> {
    prop_oneof![
        // Fire { kernel, tile, bindings }
        (any::<u64>(), iter_tile(), fire_binding())
            .prop_map(|(k, tile, bindings)| Event::Fire {
                kernel: KernelId(k),
                tile,
                bindings,
            }),
        // Alloc { data, tile, region }
        (any::<u64>(), iter_tile(), any::<u64>()).prop_map(|(d, tile, r)| Event::Alloc {
            data: DataId(d),
            tile,
            region: Region(r),
        }),
        // Push { dst, data, tile, seq }
        (any::<u64>(), any::<u64>(), iter_tile(), any::<u64>()).prop_map(
            |(dst, data, tile, seq)| Event::Push {
                dst: WorkerId(dst),
                data: DataId(data),
                tile,
                seq: SeqTag(seq),
            }
        ),
        // Wait { src, data, tile, seq }
        (any::<u64>(), any::<u64>(), iter_tile(), any::<u64>()).prop_map(
            |(src, data, tile, seq)| Event::Wait {
                src: WorkerId(src),
                data: DataId(data),
                tile,
                seq: SeqTag(seq),
            }
        ),
        // Sync { participants, kind, sync } — multi-participant sets
        // (0..=4) incl. the empty set; SyncKind has one current variant
        // (Barrier), break-to-update guarded by
        // `sync_kind_completeness_guard` — add the variant here when that
        // match fails to compile.
        (
            prop::collection::btree_set(any::<u64>().prop_map(WorkerId), 0..=4),
            Just(SyncKind::Barrier),
            any::<u64>(),
        )
            .prop_map(|(participants, kind, sync)| Event::Sync {
                participants,
                kind,
                sync: SyncTag(sync),
            }),
        // Free { data, tile }
        (any::<u64>(), iter_tile()).prop_map(|(d, tile)| Event::Free {
            data: DataId(d),
            tile,
        }),
    ]
}

/// Full `Event` strategy: the leaf arms plus the recursive `Loop` arm
/// (bounded depth ≤ 2; the body recurses into [`event_leaf`] +
/// shallower `Loop`s). `block_tag`/`check_frame` are independently
/// `Some`/`None` (arbitrary combinations, including a `check_frame` on a
/// `block_tag.is_some()` inner loop — a shape no real pass emits but a
/// stronger serde test; see module docs).
fn event_strategy() -> impl Strategy<Value = Event> {
    event_leaf().prop_recursive(2, 32, 3, |inner| {
        (
            iter_var(),
            range_i64(),
            prop::collection::vec(inner, 0..=3),
            prop::option::of(block_tag()),
            prop::option::of(check_frame()),
        )
            .prop_map(|(iter_var, range, body, block_tag, check_frame)| Event::Loop {
                iter_var,
                range,
                body,
                block_tag,
                check_frame,
            })
    })
}

// --------------------------------------------------------------------
// NameSidecar — per-field arbitrary generator (AC#2)
//
// Covers every serde-bearing field. The field list is kept in lockstep
// with `sidecar_field_completeness_guard` above (both destructure all
// 11 fields by name; a new field breaks both to compile).
// --------------------------------------------------------------------

fn kernel_sig() -> impl Strategy<Value = KernelSig> {
    (
        prop::collection::vec(resolved_type(), 0..=3),
        prop::option::of(resolved_type()),
    )
        .prop_map(|(params, ret)| KernelSig { params, ret })
}

fn const_value() -> impl Strategy<Value = ConstValue> {
    (scalar_type(), any::<i64>()).prop_map(|(ty, value)| ConstValue { ty, value })
}

fn loop_bound() -> impl Strategy<Value = LoopBound> {
    (ir_expr(), ir_expr()).prop_map(|(lo, hi)| LoopBound { lo, hi })
}

fn reuse_slot() -> impl Strategy<Value = ReuseSlot> {
    (any::<i64>(), any::<u64>()).prop_map(|(min_offset, length)| ReuseSlot { min_offset, length })
}

/// A small `BTreeMap<DataId, V>` (helper for nested maps).
fn small_map<V: std::fmt::Debug, S: Strategy<Value = V>>(
    vstrat: S,
) -> impl Strategy<Value = BTreeMap<u64, V>> {
    prop::collection::btree_map(any::<u64>(), vstrat, 0..=3)
}

/// Arbitrary [`NameSidecar`] over ALL serde-bearing fields (AC#2). Each
/// map is kept small (0..=3 entries, nesting included) for speed; the
/// round-trip is structural so a few entries per nesting level suffices.
fn namesidecar_strategy() -> impl Strategy<Value = NameSidecar> {
    // data_types: BTreeMap<DataId, ResolvedType>
    let data_types =
        prop::collection::btree_map(any::<u64>().prop_map(DataId), resolved_type(), 0..=3);
    // consts: BTreeMap<String, ConstValue>
    let consts = prop::collection::btree_map(ident(), const_value(), 0..=3);
    // loop_bounds: BTreeMap<IterVar, LoopBound>
    let loop_bounds = prop::collection::btree_map(iter_var(), loop_bound(), 0..=3);
    // kernel_sigs: BTreeMap<KernelId, KernelSig>
    let kernel_sigs =
        prop::collection::btree_map(any::<u64>().prop_map(KernelId), kernel_sig(), 0..=3);
    // partition_worker_ranges: BTreeMap<IterVar, BTreeMap<WorkerId, Range<i64>>>
    let partition_worker_ranges = prop::collection::btree_map(
        iter_var(),
        prop::collection::btree_map(any::<u64>().prop_map(WorkerId), range_i64(), 0..=3),
        0..=3,
    );
    // transfer_buffer_for_seq: BTreeMap<SeqTag, u64>
    let transfer_buffer_for_seq =
        prop::collection::btree_map(any::<u64>().prop_map(SeqTag), any::<u64>(), 0..=3);
    // halo_widths: BTreeMap<KernelId, BTreeMap<IterVar, u64>>
    let halo_widths = prop::collection::btree_map(
        any::<u64>().prop_map(KernelId),
        prop::collection::btree_map(iter_var(), any::<u64>(), 0..=3),
        0..=3,
    );
    // reuse_widths: BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>>
    let reuse_widths = prop::collection::btree_map(
        iter_var(),
        prop::collection::btree_map(
            any::<u64>().prop_map(DataId),
            small_map(reuse_slot()),
            0..=3,
        ),
        0..=3,
    );
    // partition_pairs: BTreeMap<IterVar, IterVar>
    let partition_pairs = prop::collection::btree_map(iter_var(), iter_var(), 0..=3);
    // grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)>
    let grid_shape_for_outer_iv =
        prop::collection::btree_map(iter_var(), (any::<u32>(), any::<u32>()), 0..=3);
    // cumulative_data: BTreeSet<DataId>
    let cumulative_data = prop::collection::btree_set(any::<u64>().prop_map(DataId), 0..=3);

    // `prop_oneof!`/tuple strategies cap at 12-ish elements; bundle into
    // nested tuples to stay under the arity ceiling, then re-spread.
    (
        data_types,
        consts,
        loop_bounds,
        kernel_sigs,
        partition_worker_ranges,
        transfer_buffer_for_seq,
        halo_widths,
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
        cumulative_data,
    )
        .prop_map(
            |(
                data_types,
                consts,
                loop_bounds,
                kernel_sigs,
                partition_worker_ranges,
                transfer_buffer_for_seq,
                halo_widths,
                reuse_widths,
                partition_pairs,
                grid_shape_for_outer_iv,
                cumulative_data,
            )| NameSidecar {
                data_types,
                consts,
                loop_bounds,
                kernel_sigs,
                partition_worker_ranges,
                transfer_buffer_for_seq,
                halo_widths,
                reuse_widths,
                partition_pairs,
                grid_shape_for_outer_iv,
                cumulative_data,
            },
        )
}

// --------------------------------------------------------------------
// The two round-trip properties (AC#1 + AC#2)
// --------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(PROPTEST_CASES))]

    /// AC#1 — every arbitrary `Event` tree round-trips through serde
    /// JSON byte-exact-semantically (`from_str(to_string(e)) == e`).
    ///
    /// A failure here is a P1 round-trip defect (a serde attribute
    /// dropping a field, a `Range`/`Option<..>`-default edge case): STOP,
    /// file a prereq task with the shrunk seed + value, leave this
    /// `#[ignore]`d. Do NOT narrow `event_strategy` to dodge it. See the
    /// module "Honest-failure path".
    #[test]
    fn event_serde_roundtrip(e in event_strategy()) {
        let json = serde_json::to_string(&e).expect("serialise Event");
        let back: Event = serde_json::from_str(&json).expect("deserialise Event");
        prop_assert_eq!(&back, &e, "Event did not round-trip; json = {}", json);
    }

    /// AC#2 — every arbitrary `NameSidecar` round-trips through serde
    /// JSON byte-exact-semantically. Covers all 11 serde-bearing fields
    /// (deep-nested maps included). Same honest-failure discipline as
    /// `event_serde_roundtrip`.
    #[test]
    fn sidecar_serde_roundtrip(s in namesidecar_strategy()) {
        let json = serde_json::to_string(&s).expect("serialise NameSidecar");
        let back: NameSidecar =
            serde_json::from_str(&json).expect("deserialise NameSidecar");
        prop_assert_eq!(&back, &s, "NameSidecar did not round-trip; json = {}", json);
    }
}
