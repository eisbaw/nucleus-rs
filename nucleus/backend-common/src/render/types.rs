//! Rust type / zero-literal / init-expression renderers shared across
//! every tier-1 backend. Split from `render.rs` for file-size hygiene;
//! no behaviour change.

use nucleus_compiler::algo::{ResolvedType, ScalarType};

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
pub fn render_array_init_for(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_zero(&ty.scalar).to_string()
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        format!("vec![{zero}; {total}]")
    }
}
