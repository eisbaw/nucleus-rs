//! Rust type / zero-literal / init-expression renderers shared across
//! every tier-1 backend. Split from `render.rs` for file-size hygiene;
//! no behaviour change.

use nucleus_compiler::algo::{CombineOp, ResolvedType, ScalarType};

/// Rust spelling of a Nuc `ScalarType`. Internal default; the public
/// re-export `rust_scalar_type_pub` keeps the name stable for external
/// callers.
pub fn rust_scalar_type(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::Usize => "usize",
        ScalarType::Isize => "isize",
        ScalarType::U8 => "u8",
        ScalarType::U16 => "u16",
        ScalarType::U32 => "u32",
        ScalarType::U64 => "u64",
        ScalarType::I8 => "i8",
        ScalarType::I16 => "i16",
        ScalarType::I32 => "i32",
        ScalarType::I64 => "i64",
        ScalarType::F32 => "f32",
        ScalarType::F64 => "f64",
        ScalarType::Bool => "bool",
    }
}

/// Public spelling of the Rust scalar type. Identity wrapper kept
/// for source-level compatibility — `rust_scalar_type` is already
/// `pub` in backend-common, but callers historically imported the
/// `_pub` variant from pthreads-sync.
pub fn rust_scalar_type_pub(t: &ScalarType) -> &'static str {
    rust_scalar_type(t)
}

/// The Rust literal for "zero" of a scalar type.
pub fn rust_scalar_zero(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::Usize | ScalarType::Isize => "0",
        ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 => "0",
        ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 => "0",
        ScalarType::F32 | ScalarType::F64 => "0.0",
        ScalarType::Bool => "false",
    }
}

/// Rust surface type for a `ResolvedType`: scalars natural, arrays
/// flatten to `Vec<T>`. Shared so slot/buffer typing cannot drift
/// between backends.
pub fn rust_type_of(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_type(&ty.scalar).to_string()
    } else {
        format!("Vec<{}>", rust_scalar_type(&ty.scalar))
    }
}

/// `vec![<zero>; product(dims)]` (array) or the scalar zero literal,
/// sized + typed entirely from the sidecar `ResolvedType`. Shared so
/// the per-backend pre-init allocation cannot drift.
///
/// Thin wrapper over [`render_array_init_for_combine`] with `None`
/// (zero identity) — the behaviour every non-accumulator pre-init
/// wants.
pub fn render_array_init_for(ty: &ResolvedType) -> String {
    render_array_init_for_combine(ty, None)
}

/// The Rust literal for the IDENTITY ELEMENT of `combine` at scalar
/// type `t`. This is the SINGLE SOURCE OF TRUTH for accumulator
/// pre-init across every backend (TASK-0343.01.02) — every duplicated
/// `render_array_init` site routes here, so a wrong/missing identity
/// cannot drift silently between backends.
///
/// - `None` / `Sum` / `Or` / `Xor` → zero (additive / bitwise-OR /
///   bitwise-XOR identity, and the bool OR/XOR identity `false`),
///   spelled by [`rust_scalar_zero`].
/// - `Min` → the greatest element: integer `{T}::MAX`; **float
///   `{f32|f64}::INFINITY`** (`x.min(INFINITY) == x` for any non-NaN
///   `x`, where `{T}::MAX` would be the largest FINITE value and so
///   wrongly clamp out a `+INFINITY` input value).
/// - `Max` → the least element: integer `{T}::MIN`; **float
///   `{f32|f64}::NEG_INFINITY`**.
/// - `And` → all-ones for integers, spelled `!0T` UNIFORMLY for both
///   signed and unsigned (`!0i32 == -1`, `!0u32 == u32::MAX`; `!0T`
///   parses as `!(0T)`). NOTE: `T::MAX` would be WRONG for signed `&`
///   (it is `0111…1`, not all-ones). For **bool**, the AND identity is
///   `true` (`x && true == x`).
///
/// # Admissibility (TASK-0343.02)
///
/// The non-zero ops are admitted per-type-class by
/// `combine_form_for_scalar`: float admits `min`/`max` (rejecting
/// `sum`/bitwise), bool admits `and`/`or`/`xor` (rejecting
/// `sum`/`min`/`max`). So float `Min`/`Max` and bool `And` ARE reached
/// here (with the identities above); the combos that never pass the
/// gate (float `And`, bool `Min`/`Max`) still return a sane literal
/// below — they can only arrive via a path the gate already rejected,
/// so the value is never emitted, but it must not panic.
///
/// # NaN / signed-zero caveat
///
/// Float `min`/`max` are admitted because they are order-independent
/// for DISTINCT FINITE NON-NaN values — so the reduced bits are
/// reduction-order-independent and bit-identical across backends (PRD
/// §10.1). A bin mixing `-0.0`/`+0.0`, or an all-NaN bin, is NOT
/// guaranteed bit-stable under reordering (`f32::min` ignores NaN and
/// treats ±0 as equal); that is an out-of-scope documented caveat. The
/// 27-bin-fmin fixture is NaN-free with distinct positive finite values.
pub fn combine_identity_literal(t: &ScalarType, combine: Option<CombineOp>) -> String {
    let rty = rust_scalar_type(t);
    let is_float = matches!(t, ScalarType::F32 | ScalarType::F64);
    let is_bool = matches!(t, ScalarType::Bool);
    match combine {
        None | Some(CombineOp::Sum) | Some(CombineOp::Or) | Some(CombineOp::Xor) => {
            rust_scalar_zero(t).to_string()
        }
        // Min identity is the GREATEST element of the type. Integers:
        // `T::MAX`. Floats: `+INFINITY` (NOT `f32::MAX`, which is the
        // largest finite — it would clamp a genuine `+INFINITY` value).
        Some(CombineOp::Min) if is_float => format!("{rty}::INFINITY"),
        Some(CombineOp::Min) => format!("{rty}::MAX"),
        // Max identity is the LEAST element. Floats: `-INFINITY`.
        Some(CombineOp::Max) if is_float => format!("{rty}::NEG_INFINITY"),
        Some(CombineOp::Max) => format!("{rty}::MIN"),
        // And identity: bool `true`; integer all-ones `!0T`.
        Some(CombineOp::And) if is_bool => "true".to_string(),
        Some(CombineOp::And) => format!("!0{rty}"),
    }
}

/// Identity-aware variant of [`render_array_init_for`]: emits
/// `vec![<identity>; product(dims)]` (array) or the scalar identity
/// literal, where the identity is chosen by `combine` via
/// [`combine_identity_literal`]. Callers pass
/// `sidecar.combine_for_data.get(did).copied()` so the accumulator's
/// pre-init holds the combine identity, not the hardcoded zero.
pub fn render_array_init_for_combine(ty: &ResolvedType, combine: Option<CombineOp>) -> String {
    let lit = combine_identity_literal(&ty.scalar, combine);
    if ty.is_scalar() {
        lit
    } else {
        let total: usize = ty.dims.iter().copied().product();
        format!("vec![{lit}; {total}]")
    }
}
