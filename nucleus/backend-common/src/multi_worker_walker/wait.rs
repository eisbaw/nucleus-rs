//! Receiver-side gather shape dispatch for `Event::Wait` (TASK-0117 1D
//! leading-axis path + TASK-0294 2D row-loop path). Consumed by the
//! shared event walker AND directly by mp-tcp-bufsync (which bypasses
//! [`super::event_walker::render_worker_events`] and calls
//! [`render_wait_assign`] from its own event walker).
//!
//! TASK-0455.07: the receiver `_tmp` offset basis is no longer derived
//! here. [`render_wait_assign`] now derives the ONE
//! [`super::wire_shape::WireShape`] for the `(data, seq)` edge and
//! dispatches on its [`RecvBasis`]; the sender sites read the SAME
//! `WireShape`, so the wire shape lives in exactly one place (the
//! pre-work for the TASK-0453.22 whole-array → inferred-region flip).
//! The `WaitSlice` enum + `wait_slice`/`nd_banded_slice` derivation that
//! used to live here moved to `wire_shape` as `RecvBasis` /
//! `recv_basis` / `nd_banded_basis`, unchanged.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::algo::{CombineOp, ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, IterTile, SeqTag};
use nucleus_compiler::sidecar::NameSidecar;

use super::wire_shape::{RecvBasis, WireShape};
use crate::render::EmitError;

/// Render the receiver-side assignment statement for one Wait event.
/// Returns one statement (no trailing newline).
///
/// Three shapes, dispatched by `wait_slice`:
/// - **Whole-array assign** (`name = <rhs>;`) — the pre-TASK-0117
///   single-pair behaviour. Selected when the pair's tile is empty
///   (no enclosing iteration nest, e.g. a top-level load_input ⇒ host
///   transfer), OR when every consulted axis of the tile covers the
///   data's full range on the corresponding dim (i.e. the producer
///   sent the whole array on this pair).
/// - **1D slice-paste** (`{ let _tmp = <rhs>; name[lo..hi]
///   .copy_from_slice(&_tmp[lo..hi]); }`) — TASK-0117 leading-axis
///   gather. Selected when the tile has a single bound (or only one
///   bound is consultable against the data's dim rank).
/// - **2D row-loop slice-paste** (`{ let _tmp = <rhs>; for _y in
///   outer_lo..outer_hi { let _r = _y * row_stride; name[_r +
///   inner_lo_off.._r + inner_hi_off].copy_from_slice(&_tmp[_r +
///   inner_lo_off.._r + inner_hi_off]); } }`) — TASK-0294
///   `partition=blocks2d` gather. Selected when the tile has rank >=
///   2 AND the data has dim rank >= 2; each outer-axis iteration
///   copies one row's inner-axis sub-range. The 1D leading-axis
///   path would paste each worker's whole y-band (overwriting
///   adjacent workers' columns with default-zero values).
#[allow(clippy::too_many_arguments)]
pub fn render_wait_assign(
    sidecar: &NameSidecar,
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
    name: &str,
    data: DataId,
    seq: SeqTag,
    rhs: &str,
    accumulate: bool,
    let_at_wait: &BTreeSet<DataId>,
) -> Result<String, EmitError> {
    // TASK-0455.07: the receiver `_tmp` basis is the ONE WireShape
    // derivation shared with every sender site. `recv_basis == None`
    // is the whole-array case (the pre-TASK-0117 path); the present
    // variants are the slice-paste arms.
    let wire = WireShape::derive(sidecar, pair_tiles, data, seq)?;
    match wire.recv_basis {
        None => {
            // Empty tile (or whole-array match) — pre-cycle-189 path
            // was whole-array assign; cycle-189 adds the accumulate
            // dispatch.
            if accumulate {
                // TASK-0343 cycle 189: overlapping-write accumulator
                // fan-in (N>=2 whole-array Waits on the same data,
                // detected by `collect_accumulate_waits` in sibling
                // collect.rs). Emit element-wise wrapping_add into
                // the pre-initialised destination instead of a bare
                // overwrite assign. Pre-cycle-189 every such Wait
                // emitted `name = rhs;` producing last-write-wins on
                // 08-histogram/distributed (4 sequential overwrites
                // yielded one worker's standalone partial).
                let ty = sidecar.data_type(data).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "accumulate Wait of data {data:?} has no ResolvedType in NameSidecar"
                    ))
                })?;
                render_accumulate_assign(sidecar, data, name, rhs, ty)
            } else if let_at_wait.contains(&data) {
                // TASK-0349 cycle 220: whole-array recv on a data
                // symbol whose ONLY Waits are whole-array (and which
                // is not accumulate-fan-in and not indexed-Fire-
                // written). The per-backend pre-init pass omits the
                // `let mut <name>: Vec<..> = vec![0; N];` line for
                // these data; the emit MUST declare-and-assign in
                // one statement so the variable comes into scope at
                // the recv site. Type inference picks up the Vec<T>
                // from the rendezvous slot's `.wait()` return type.
                //
                // SCOPE HAZARD (TASK-0356 cycle 222, characterized;
                // guard LANDED as TASK-0364, OPTION B): this `let
                // {name}` comes into scope at the WAIT'S lexical
                // position. When the Wait sits inside an `Event::Loop`
                // body, the `let` lands inside the emitted `for { }`
                // block; a consumer of `{name}` at the ENCLOSING outer
                // scope would then read it out of scope (non-compiling
                // Rust). This is NOT producible today —
                // `transfer_inject` (`inject_in_sequence`) co-locates
                // every cross-worker Wait in the SAME sequence as its
                // consuming Operation, so an outer-scope consumer gets
                // its Wait at the outer scope, never in a nested loop.
                // This emit is therefore unchanged by TASK-0364; the
                // guard is the sibling
                // `super::collect::check_let_at_wait_scope_safety`,
                // called once at `super::event_walker::
                // render_worker_events` entry, which fails LOUD with
                // `EmitError::ContractGap` BEFORE this emit runs if a
                // future pass ever constructs the at-risk scope. The
                // boundary is pinned by
                // `tests/wait_let_at_wait_loop_scope.rs`.
                Ok(format!("let {name} = {rhs};"))
            } else {
                Ok(format!("{name} = {rhs};"))
            }
        }
        Some(_) if accumulate => {
            // Defensive: `collect_accumulate_waits` (sibling
            // collect.rs) only classifies whole-array Waits as
            // accumulate, so reaching here with a slice tile means a
            // caller fabricated the flag against a slice-paste tile.
            // Fail LOUD so the divergence between the collector and
            // the consumer surfaces at compile time rather than
            // silently mis-combining a disjoint-write slice gather.
            Err(EmitError::ContractGap(format!(
                "render_wait_assign: accumulate=true on Wait of data {data:?} \
                 (seq {seq:?}) whose tile resolves to a slice-paste arm; \
                 `collect_accumulate_waits` only classifies whole-array tiles \
                 as accumulate. This is a contract gap between the caller and \
                 the accumulate collector (TASK-0343 cycle 189)."
            )))
        }
        Some(RecvBasis::Flat { lo, hi }) => {
            // 1D leading-axis slice-paste — TASK-0117.
            //
            // TASK-0453.22: `Flat` is the CONTIGUOUS narrowable arm
            // (`wire.contiguous_span()` returns `Some((lo, hi))` for it),
            // so the paired sender narrows its payload to `name[lo..hi]`.
            // The decoded/pushed `_tmp` is therefore the BAND, length
            // `hi - lo`, indexed FROM 0 — not from the destination offset
            // `lo` (which would be out of bounds on the band). The
            // DESTINATION keeps the absolute `[lo..hi]` range. Both the
            // sender narrowing and this from-0 rebase derive from the ONE
            // `WireShape` for this `(data, seq)`, so they cannot diverge.
            let span = hi - lo;
            Ok(format!(
                "{{ let _tmp = {rhs}; \
                 {name}[{lo}usize..{hi}usize].copy_from_slice(\
                 &_tmp[0usize..{span}usize]); }}"
            ))
        }
        Some(RecvBasis::Rows {
            outer_lo,
            outer_hi,
            row_stride,
            inner_lo_off,
            inner_hi_off,
        }) => {
            // 2D row-loop slice-paste — TASK-0294. The `_y`/`_r`
            // local names are underscore-prefixed AND introduced
            // inside a `{ ... }` block — Rust block-shadowing makes
            // them safe regardless of where this Wait is placed
            // (host main() body, worker pre-compute halo-strip
            // landing site, or a future multi-pass time-step Repeat
            // body). The cycle-115 placement happens to keep
            // halo-strip Waits at the root Sequence (TASK-0290), so
            // collision was structurally impossible; this argument
            // stays sound when that placement moves under TASK-0294
            // multi-pass follow-ups.
            Ok(format!(
                "{{ let _tmp = {rhs}; \
                 for _y in {outer_lo}usize..{outer_hi}usize {{ \
                 let _r = _y * {row_stride}usize; \
                 {name}[_r + {inner_lo_off}usize.._r + {inner_hi_off}usize]\
                 .copy_from_slice(\
                 &_tmp[_r + {inner_lo_off}usize.._r + {inner_hi_off}usize]); \
                 }} }}"
            ))
        }
        Some(RecvBasis::NestedRows {
            leading,
            band_lo_off,
            band_hi_off,
        }) => {
            // N-D nested-loop slice-paste (TASK-0341.02.02.01.01). Emit
            // one `for _aK in 0..dim` loop per full leading axis,
            // accumulating a flat `_base` offset, then a single
            // contiguous `copy_from_slice` over the banded span within
            // each base block. The `_aK` / `_base` locals are
            // underscore-prefixed AND inside a `{ ... }` block (Rust
            // block-shadowing makes them collision-safe wherever this
            // Wait is placed). 16-jacobi/distributed: `leading = [(5,
            // 64)]` (the t axis), `band_lo_off..band_hi_off` = the
            // per-row y-band flat span (e.g. `8..24` for band 1..3 on a
            // [.][8][8] data symbol).
            //
            // TASK-0453.22 narrowing split — driven by the SAME
            // `WireShape::contiguous_span()` predicate the sender uses:
            //
            // - `leading` EMPTY ⇒ the band is a SINGLE contiguous span
            //   `[band_lo_off, band_hi_off)` (banded axis is dim 0, all
            //   trailing axes copied whole). `contiguous_span()` returns
            //   `Some` for it, so the paired sender narrowed its payload
            //   to that band; `_tmp` is the BAND, indexed FROM 0.
            // - `leading` NON-EMPTY ⇒ the band recurs once per
            //   leading-axis iteration with STRIDED GAPS, so it is NOT a
            //   single contiguous slice; `contiguous_span()` returns
            //   `None`, the sender kept the WHOLE array, and `_tmp` is
            //   indexed by the SAME `_base`-relative offsets as the
            //   destination (the 16-jacobi cumulative-array halo — kept
            //   whole-array, which is also the only combine that does not
            //   xN-double-count the shared cross-iteration history).
            let narrowed = leading.is_empty();
            let mut s = String::from("{ let _tmp = ");
            s.push_str(rhs);
            s.push_str("; ");
            // `_base` starts at 0 and each loop level adds `_aK * stride`.
            // We open the loops, accumulating the base via a fresh
            // shadowed binding at each level so the innermost `_base`
            // sees the full accumulation.
            s.push_str("let _base = 0usize; ");
            let mut closers = 0usize;
            for (lvl, (dim, stride)) in leading.iter().enumerate() {
                s.push_str(&format!(
                    "for _a{lvl} in 0usize..{dim}usize {{ let _base = _base + _a{lvl} * {stride}usize; "
                ));
                closers += 1;
            }
            // Source index: from-0 over the band when narrowed (no
            // leading axes ⇒ `_base` is always 0, so the destination's
            // `_base + band_lo_off` equals `band_lo_off` and the source
            // is the band `0..span`); `_base`-relative whole-array index
            // otherwise.
            let src_range = if narrowed {
                let span = band_hi_off - band_lo_off;
                format!("0usize..{span}usize")
            } else {
                format!("_base + {band_lo_off}usize.._base + {band_hi_off}usize")
            };
            s.push_str(&format!(
                "{name}[_base + {band_lo_off}usize.._base + {band_hi_off}usize]\
                 .copy_from_slice(&_tmp[{src_range}]); "
            ));
            for _ in 0..closers {
                s.push('}');
                s.push(' ');
            }
            s.push('}');
            Ok(s)
        }
    }
}

/// Module-private classifier for sibling collect-pass consumers
/// (TASK-0349 cycle 220 `collect_let_at_wait_data` in
/// `super::collect`). Returns `Ok(true)` when the (data, tile) pair
/// resolves to whole-array recv (the `recv_basis == None` arm of the
/// one [`WireShape`] derivation); `Ok(false)` when it resolves to
/// slice-paste; `Err` on shape-error or sidecar-lookup invariant
/// violations (the derivation's Err arms span both classes — rank > 2,
/// out-of-bounds range, AND `NameSidecar::data_type` returning `None`).
///
/// TASK-0455.07: now routes through [`WireShape::from_tile`] /
/// [`WireShape::is_whole_array`] so the whole-array-vs-slice
/// classification has ONE source of truth shared with
/// [`render_wait_assign`] and every sender site.
///
/// Visibility narrowed to `pub(super)` cycle 220b per architect P2.2
/// — no consumer outside `multi_worker_walker` needs to call this
/// directly; the only call site is `super::collect::collect_let_at_wait_inner`.
pub(super) fn is_whole_array_recv(
    sidecar: &NameSidecar,
    data: DataId,
    tile: &IterTile,
) -> Result<bool, EmitError> {
    Ok(WireShape::from_tile(sidecar, data, Some(tile))?.is_whole_array())
}

/// Combine emit form for one [`CombineOp`] (TASK-0343.01.01).
///
/// - `Method(m)` → `<lhs> = <lhs>.<m>(<rhs>);` — the `sum`
///   (`wrapping_add`) form, preserved byte-identical with the
///   pre-TASK-0343.01.01 hardcoded emit.
/// - `Operator(o)` → `<lhs> = <lhs> <o> <rhs>;` — the bitwise `or`
///   (`|`) / `xor` (`^`) forms.
enum CombineForm {
    Method(&'static str),
    Operator(&'static str),
}

impl CombineForm {
    /// Emit `<lhs> = <combine of lhs and rhs>` (no trailing `;`).
    fn emit(&self, lhs: &str, rhs: &str) -> String {
        match self {
            CombineForm::Method(m) => format!("{lhs} = {lhs}.{m}({rhs})"),
            CombineForm::Operator(o) => format!("{lhs} = {lhs} {o} {rhs}"),
        }
    }
}

/// Element-wise overlapping-write accumulate emit for the fan-in arm
/// of `render_wait_assign` (TASK-0343 cycle 189; combine identity
/// generalised TASK-0343.01.01).
///
/// The combine identity is DECLARED by the accumulator's owning kernel
/// (`combine = sum|or|xor`) and resolved here via
/// `sidecar.combine_for_data[data]`. Emits one of:
/// - Array: `{ let _tmp = <rhs>; for _k in 0..<LEN>usize { <combine of
///   name[_k], _tmp[_k]>; } }`. `<LEN>` is the product of `ty.dims`
///   (flat element count; matches `render_array_init`'s per-backend
///   zero-init).
/// - Scalar: `<combine of name, rhs>;`. Structurally degenerate but
///   kept for completeness.
///
/// where `<combine>` is `name[_k].wrapping_add(_tmp[_k])` for `sum`,
/// `name[_k] | _tmp[_k]` for `or`, `name[_k] ^ _tmp[_k]` for `xor`.
/// All three share the additive-identity ZERO so the existing
/// per-backend zero-init is correct unchanged.
///
/// # Soundness reject (TASK-0343.01.01 AC#4 / #7)
///
/// `data` ABSENT from `sidecar.combine_for_data` means its owning
/// kernel declared NO combine identity. Pre-TASK-0343.01.01 this arm
/// silently assumed `sum`; now it fails LOUD with `EmitError::ContractGap`.
/// (The driver gate `check_accumulator_consistency` catches this
/// earlier, but the render path stays fail-loud as a defence in depth.)
/// `min`/`max`/`and` (non-zero identity) are now accepted
/// (TASK-0343.01.02); their accumulator pre-init carries the matching
/// identity (`combine_identity_literal`), so there is no silent
/// fallthrough.
///
/// # Scalar-type admissibility (TASK-0343.02)
///
/// `combine_form_for_scalar` admits a combine op ONLY when it is
/// order-independent (associative + commutative) on the scalar type,
/// the PRD §10.1 bit-identity precondition. Integer: all six ops.
/// Float (f32/f64): `min`/`max` only — `sum` is non-associative
/// (different per-backend reduction orders give different bits) and
/// `or`/`xor`/`and` are undefined on float; both reject with a typed
/// `EmitError::ContractGap`. Bool: `and`/`or`/`xor` only — `sum` (no
/// canonical identity) and `min`/`max` (spelled and/or) reject. No
/// panic, no silent fallthrough.
fn render_accumulate_assign(
    sidecar: &NameSidecar,
    data: DataId,
    name: &str,
    rhs: &str,
    ty: &ResolvedType,
) -> Result<String, EmitError> {
    let op = sidecar.combine_for_data.get(&data).copied().ok_or_else(|| {
        EmitError::ContractGap(format!(
            "render_wait_assign: accumulate fan-in on data {data:?} (`{name}`) whose \
             owning kernel declares NO `combine = <op>` identity. Pre-TASK-0343.01.01 \
             this arm silently assumed `sum`; it now fails loud. Declare a combine \
             identity on the kernel that writes `{name}` on its `<--` RHS \
             (`combine = sum|or|xor|min|max|and`)."
        ))
    })?;
    let form = combine_form_for_scalar(op, &ty.scalar)?;
    if ty.dims.is_empty() {
        // Scalar accumulator. The pre-init in each backend has set
        // `<name>` to the combine identity (0 for sum/or/xor; MAX/MIN/
        // all-ones for min/max/and; see `combine_identity_literal`).
        Ok(format!("{};", form.emit(name, rhs)))
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let body = form.emit(&format!("{name}[_k]"), "_tmp[_k]");
        Ok(format!(
            "{{ let _tmp = {rhs}; \
             for _k in 0..{total}usize {{ \
             {body}; \
             }} }}"
        ))
    }
}

/// Resolve the [`CombineForm`] for `(op, scalar)`, admitting a combine
/// op ONLY when it is ORDER-INDEPENDENT (associative + commutative) on
/// the scalar type. Order-independence is the load-bearing
/// admissibility predicate: the cross-backend differential reduces the
/// per-worker partials in DIFFERENT orders, so a host fan-in combine
/// must give bit-identical bytes regardless of arrival order (PRD
/// §10.1). Every reject is a distinct typed `EmitError::ContractGap`
/// (no panic, no silent fallthrough); float-sum's message cites
/// non-associativity + PRD §10.1 explicitly.
///
/// Per scalar-type class (TASK-0343.02):
/// - **Integer** (all width/sign): all six ops admitted unchanged.
///   `sum` → `wrapping_add`, `min`/`max` → `min`/`max` (methods);
///   `or` → `|`, `xor` → `^`, `and` → `&` (operators).
/// - **Float** (f32/f64): `min`/`max` ADMITTED — they are
///   order-independent for distinct finite non-NaN values, so the
///   reduced bits are reduction-order-independent (`f32::min` /
///   `f32::max` methods; the accumulator pre-inits to `INFINITY` /
///   `NEG_INFINITY`, see `combine_identity_literal` in
///   `render::types`).
///   `sum` REJECTED — IEEE-754 float addition is NOT associative, so
///   different per-backend reduction orders yield different bits,
///   violating the PRD §10.1 bit-identity invariant. `or`/`xor`/`and`
///   REJECTED — bitwise ops are undefined on float.
/// - **Bool**: `and`/`or`/`xor` ADMITTED (`&`/`|`/`^`, all valid +
///   associative + commutative on `bool`; identities `true`/`false`/
///   `false`). `sum` REJECTED — no canonical bool sum. `min`/`max`
///   REJECTED — use `and`/`or` (the bool lattice meet/join) instead.
///
/// # NaN / signed-zero caveat (AC#5)
///
/// Float `min`/`max` admissibility rests on order-independence for
/// distinct FINITE NON-NaN values. Rust `f32::min` "ignores NaN" and
/// treats `-0.0`/`+0.0` as equal, so a bin containing both `±0.0`, or
/// an all-NaN bin, is NOT guaranteed bit-stable under reordering —
/// which bit pattern surfaces can depend on reduction order. That is an
/// OUT-OF-SCOPE documented caveat; the committed 27-bin-fmin fixture is
/// NaN-free with distinct positive finite values, so the guarantee
/// holds there.
fn combine_form_for_scalar(op: CombineOp, t: &ScalarType) -> Result<CombineForm, EmitError> {
    use CombineOp::*;
    use ScalarType::*;
    match t {
        // Integer — every op order-independent, admitted unchanged.
        Usize | Isize | U8 | U16 | U32 | U64 | I8 | I16 | I32 | I64 => match op {
            Sum => Ok(CombineForm::Method("wrapping_add")),
            Or => Ok(CombineForm::Operator("|")),
            Xor => Ok(CombineForm::Operator("^")),
            // TASK-0343.01.02 non-zero-identity ops. `min`/`max` use
            // the inherent `Ord` methods; `and` is the bitwise
            // operator. The accumulator's pre-init must already hold
            // the matching identity (MAX / MIN / all-ones) — see
            // `combine_identity_literal`.
            Min => Ok(CombineForm::Method("min")),
            Max => Ok(CombineForm::Method("max")),
            And => Ok(CombineForm::Operator("&")),
            // `fsum` (TASK-0453.03) is FLOAT-ONLY: it is the opt-in for
            // float's non-associativity. Integer `sum` is already exact
            // and order-independent, so `fsum` on an integer accumulator
            // is a category error, not a silent alias.
            Fsum => Err(EmitError::ContractGap(format!(
                "render_wait_assign: accumulate fan-in (combine=fsum) on an INTEGER-scalar \
                 data symbol ({t:?}) is REJECTED: `fsum` is the float-only opt-in for the \
                 fixed-order reproducible float sum. Integer `sum` is already exact and \
                 order-independent — use combine=sum."
            ))),
        },
        // Float — the order-independent lattice ops (min/max) and the
        // opt-in fixed-order reproducible sum (fsum) are admissible;
        // plain sum is non-associative, bitwise undefined.
        F32 | F64 => match op {
            Min => Ok(CombineForm::Method("min")),
            Max => Ok(CombineForm::Method("max")),
            // TASK-0453.03: opt-in reproducible float sum. The fan-in
            // emits a plain `+` fold; cross-backend bit-identity comes
            // from the FIXED fold order — the host combines per-worker
            // partials in worker-id-sorted event-list order (TASK-0389),
            // identical across all backends. It is NOT the naive
            // single-pass IEEE sum and NOT schedule-invariant (different
            // worker counts fold differently); that residual is the
            // user's explicit acceptance when they spell `fsum`.
            Fsum => Ok(CombineForm::Operator("+")),
            Sum => Err(EmitError::ContractGap(format!(
                "render_wait_assign: accumulate fan-in (combine=sum) on a float-scalar \
                 data symbol ({t:?}) is REJECTED: IEEE-754 float addition is NOT \
                 associative, so an order-varying host fan-in would yield different bits \
                 across backends — violating the PRD §10.1 bit-identity invariant. Use \
                 combine=min/max (order-independent on distinct finite values), or opt in \
                 to the fixed-order reproducible sum with combine=fsum (cross-backend \
                 bit-identical for a given schedule; NOT the naive IEEE sum — see \
                 TASK-0453.03)."
            ))),
            Or | Xor | And => Err(EmitError::ContractGap(format!(
                "render_wait_assign: accumulate fan-in (combine={op:?}) on a float-scalar \
                 data symbol ({t:?}) is REJECTED: bitwise combine ops (or/xor/and) are \
                 undefined on float. Use combine=min/max/fsum for a float accumulator."
            ))),
        },
        // Bool — the lattice/parity ops are order-independent; sum has
        // no canonical bool identity; min/max are spelled and/or.
        Bool => match op {
            And => Ok(CombineForm::Operator("&")),
            Or => Ok(CombineForm::Operator("|")),
            Xor => Ok(CombineForm::Operator("^")),
            Sum => Err(EmitError::ContractGap(
                "render_wait_assign: accumulate fan-in (combine=sum) on a bool-scalar \
                 data symbol is REJECTED: bool has no canonical sum identity in v2. \
                 Declare the intended fan-in explicitly: combine=or (any), combine=and \
                 (all), or combine=xor (parity)."
                    .to_string(),
            )),
            Min | Max => Err(EmitError::ContractGap(format!(
                "render_wait_assign: accumulate fan-in (combine={op:?}) on a bool-scalar \
                 data symbol is REJECTED: min/max on bool are ambiguous in v2. Use the \
                 bool lattice ops instead — combine=and is the meet (min), combine=or \
                 is the join (max)."
            ))),
            Fsum => Err(EmitError::ContractGap(
                "render_wait_assign: accumulate fan-in (combine=fsum) on a bool-scalar \
                 data symbol is REJECTED: `fsum` is the float-only reproducible-sum opt-in. \
                 Use combine=and/or/xor for a bool accumulator."
                    .to_string(),
            )),
        },
    }
}
