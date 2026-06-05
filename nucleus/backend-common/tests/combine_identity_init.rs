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
