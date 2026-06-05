//! Unit pins for the identity-aware accumulator pre-init helpers
//! (`combine_identity_literal` + `render_array_init_for_combine`,
//! TASK-0343.01.02). These are the SINGLE SOURCE OF TRUTH for the
//! per-(scalar, combine) init literal across every tier-1 backend, so
//! a wrong identity here is a 7-way bit-identity break. The fixtures
//! below pin:
//!
//! 1. Zero-identity ops (`None`/`Sum`/`Or`/`Xor`) → the unchanged zero
//!    literal — the pre-TASK-0343.01.02 behaviour must not drift.
//! 2. `Min` → `T::MAX`, `Max` → `T::MIN` (so `min(MAX, x) == x`).
//! 3. `And` → `!0T` UNIFORMLY — the load-bearing all-ones spelling that
//!    is correct for BOTH signed (`!0i32 == -1`) and unsigned
//!    (`!0u32 == u32::MAX`). `T::MAX` would be WRONG for signed `&`.
//! 4. Array shape: `vec![<identity>; N]`.

use backend_common::render::{combine_identity_literal, render_array_init_for_combine};
use nucleus_compiler::algo::{CombineOp, ResolvedType, ScalarType};

#[test]
fn zero_identity_ops_unchanged_per_scalar() {
    // None and the three zero-identity ops all spell the zero literal.
    for combine in [
        None,
        Some(CombineOp::Sum),
        Some(CombineOp::Or),
        Some(CombineOp::Xor),
    ] {
        assert_eq!(
            combine_identity_literal(&ScalarType::I32, combine),
            "0",
            "i32 zero-identity ({combine:?}) must stay `0`"
        );
        assert_eq!(
            combine_identity_literal(&ScalarType::U64, combine),
            "0",
            "u64 zero-identity ({combine:?}) must stay `0`"
        );
        // Float/bool can only reach here via a zero-identity arm (the
        // non-zero ops are integer-gated before init), and must keep
        // their natural zero literal.
        assert_eq!(
            combine_identity_literal(&ScalarType::F32, combine),
            "0.0"
        );
        assert_eq!(
            combine_identity_literal(&ScalarType::Bool, combine),
            "false"
        );
    }
}

#[test]
fn min_identity_is_type_max() {
    assert_eq!(
        combine_identity_literal(&ScalarType::I32, Some(CombineOp::Min)),
        "i32::MAX"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::U8, Some(CombineOp::Min)),
        "u8::MAX"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::Usize, Some(CombineOp::Min)),
        "usize::MAX"
    );
}

#[test]
fn max_identity_is_type_min() {
    assert_eq!(
        combine_identity_literal(&ScalarType::I32, Some(CombineOp::Max)),
        "i32::MIN"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::U8, Some(CombineOp::Max)),
        "u8::MIN"
    );
}

#[test]
fn and_identity_is_all_ones_uniform_signed_and_unsigned() {
    // The load-bearing case: `!0T` is all-ones for BOTH signednesses.
    // `!0i32 == -1` (all bits set); `!0u32 == u32::MAX`. `i32::MAX`
    // would be `0x7fff_ffff` — NOT all-ones — and would break `&`.
    assert_eq!(
        combine_identity_literal(&ScalarType::I32, Some(CombineOp::And)),
        "!0i32",
        "And on signed i32 must be `!0i32` (all-ones == -1), NOT i32::MAX"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::U32, Some(CombineOp::And)),
        "!0u32"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::I8, Some(CombineOp::And)),
        "!0i8"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::Usize, Some(CombineOp::And)),
        "!0usize"
    );

    // Sanity: the spelled literals actually evaluate to all-ones at
    // runtime for both signednesses. (A const-eval cross-check that
    // the *concept*, not just the string, is right.)
    assert_eq!(!0i32, -1);
    assert_eq!(!0u32, u32::MAX);
    // The all-ones literal is the identity element of `&`: ANDing any
    // value against it leaves it unchanged. Use `std::hint::black_box`
    // so clippy can't constant-fold the AND into an `identity_op`.
    let ones_i = !0i32;
    let ones_u = !0u32;
    assert_eq!(
        std::hint::black_box(0x55_i32) & ones_i,
        0x55,
        "all-ones AND-mask is the identity for i32 &"
    );
    assert_eq!(std::hint::black_box(0xAA_u32) & ones_u, 0xAA);
}

#[test]
fn array_init_wraps_identity_in_vec() {
    let ty = ResolvedType {
        scalar: ScalarType::I32,
        dims: vec![4],
    };
    assert_eq!(
        render_array_init_for_combine(&ty, Some(CombineOp::Min)),
        "vec![i32::MAX; 4]",
        "min array accumulator pre-inits to vec![T::MAX; N]"
    );
    assert_eq!(
        render_array_init_for_combine(&ty, Some(CombineOp::And)),
        "vec![!0i32; 4]"
    );
    assert_eq!(
        render_array_init_for_combine(&ty, None),
        "vec![0; 4]",
        "no-combine array stays the zero init"
    );
}

#[test]
fn scalar_init_is_bare_identity_literal() {
    let ty = ResolvedType {
        scalar: ScalarType::I32,
        dims: vec![],
    };
    assert_eq!(
        render_array_init_for_combine(&ty, Some(CombineOp::Max)),
        "i32::MIN",
        "scalar accumulator init is the bare identity literal (no vec!)"
    );
}

/// Sensitivity pin (AC#6): documents that a 0-init under `combine=min`
/// with a positive accumuland DIVERGES from the identity init. This is
/// the property the 7-way differential exploits — every empty bin must
/// read `i32::MAX`, and a 0-init would yield 0. If this assertion ever
/// fails it means the zero literal sneaked back into the min path.
#[test]
fn zero_init_would_diverge_from_min_identity() {
    let min_init = combine_identity_literal(&ScalarType::I32, Some(CombineOp::Min));
    let zero_init = combine_identity_literal(&ScalarType::I32, None);
    assert_ne!(
        min_init, zero_init,
        "a `combine=min` accumulator MUST init to `i32::MAX`, never `0` — \
         a 0-init makes min(0, positive)=0 and corrupts every output \
         element (the 7-way bit-identity differential bites this)"
    );
}

// ---------------------------------------------------------------------
// TASK-0343.02 — float / bool identity literals.
// ---------------------------------------------------------------------

#[test]
fn float_min_identity_is_positive_infinity() {
    // The MIN identity is the GREATEST element of the ordered type: for
    // float that is `+INFINITY`, NOT `f32::MAX` (the largest FINITE
    // value, which would wrongly clamp out a genuine `+INFINITY` input).
    assert_eq!(
        combine_identity_literal(&ScalarType::F32, Some(CombineOp::Min)),
        "f32::INFINITY"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::F64, Some(CombineOp::Min)),
        "f64::INFINITY"
    );
    // Concept cross-check: `x.min(INFINITY) == x` for any finite x.
    assert_eq!(std::hint::black_box(3.5_f32).min(f32::INFINITY), 3.5_f32);
}

#[test]
fn float_max_identity_is_negative_infinity() {
    assert_eq!(
        combine_identity_literal(&ScalarType::F32, Some(CombineOp::Max)),
        "f32::NEG_INFINITY"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::F64, Some(CombineOp::Max)),
        "f64::NEG_INFINITY"
    );
    assert_eq!(
        std::hint::black_box(3.5_f32).max(f32::NEG_INFINITY),
        3.5_f32
    );
}

#[test]
fn bool_and_identity_is_true() {
    // The AND identity is `true` (`x && true == x`); OR/XOR keep the
    // zero literal `false` via the zero-identity path.
    assert_eq!(
        combine_identity_literal(&ScalarType::Bool, Some(CombineOp::And)),
        "true"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::Bool, Some(CombineOp::Or)),
        "false",
        "bool OR identity is `false` (the zero literal)"
    );
    assert_eq!(
        combine_identity_literal(&ScalarType::Bool, Some(CombineOp::Xor)),
        "false",
        "bool XOR identity is `false` (the zero literal)"
    );
}

/// Float-min sensitivity pin (AC#7): a 0-init (or any finite init) under
/// `combine=min` over strictly-positive floats with an EMPTY bin would
/// DIVERGE from the `f32::INFINITY` identity. An empty bin must surface
/// `f32::INFINITY` (bits 0x7F800000); a 0.0-init would yield `0.0`, and
/// `min(0.0, positive) == 0.0` corrupts every non-empty bin too. This is
/// exactly why 27-bin-fmin uses MIN (not MAX) over positive values.
#[test]
fn zero_init_would_diverge_from_float_min_identity() {
    let min_init = combine_identity_literal(&ScalarType::F32, Some(CombineOp::Min));
    let zero_init = combine_identity_literal(&ScalarType::F32, None);
    assert_eq!(min_init, "f32::INFINITY");
    assert_eq!(zero_init, "0.0");
    assert_ne!(
        min_init, zero_init,
        "a float `combine=min` accumulator MUST init to f32::INFINITY, \
         never 0.0 — min(0.0, positive)=0.0 corrupts the output and the \
         empty bin must read INFINITY bits (0x7F800000)"
    );
    // The empty-bin output bits the reference oracle commits to.
    assert_eq!(f32::INFINITY.to_bits(), 0x7F80_0000);
}

#[test]
fn float_array_init_wraps_infinity_in_vec() {
    let ty = ResolvedType {
        scalar: ScalarType::F32,
        dims: vec![16],
    };
    assert_eq!(
        render_array_init_for_combine(&ty, Some(CombineOp::Min)),
        "vec![f32::INFINITY; 16]",
        "float min array accumulator pre-inits to vec![f32::INFINITY; N]"
    );
    assert_eq!(
        render_array_init_for_combine(&ty, Some(CombineOp::Max)),
        "vec![f32::NEG_INFINITY; 16]"
    );
}
