//! Wire-format helpers used by [`super::Plan`]'s worker-program emit
//! (per-channel `encode` / `decode` paths) and `max_payload_bytes`
//! (scalar width sizing) for the async event-reactor backends.
//!
//! Transport-agnostic: the on-wire byte format is identical for TCP
//! loopback (mp-tcp-event) and Unix domain sockets (mp-uds-event) —
//! only the socket TYPE differs, and that lives in
//! [`super::EventTransport`], not here. Lifted from the two backends'
//! verbatim-duplicate `multi_worker/encode.rs` (TASK-0044.03.02).

use nucleus_compiler::algo::{ResolvedType, ScalarType};

/// Encoder/decoder fn-path for a ResolvedType. Returns `(encode_path,
/// decode_path)` as Rust expressions usable in `Chan::new(...)`.
///
/// The encoder takes `&T` (where `T = rust_type_of(ty)`) and returns
/// `Vec<u8>`. The decoder takes `&[u8]` and returns `T`.
pub fn encode_decode_paths(ty: &ResolvedType) -> (String, String) {
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        // Encoder: |v: &T| wire::enc_<s>(*v); Decoder: wire::dec_<s>.
        (
            format!("|v: &_| wire::enc_{s}(*v)"),
            format!("|b: &[u8]| wire::dec_{s}(b)"),
        )
    } else if ty.scalar == ScalarType::Bool {
        (
            "|v: &Vec<bool>| wire::enc_vec_bool(v)".to_string(),
            "|b: &[u8]| wire::dec_vec_bool(b)".to_string(),
        )
    } else {
        let rs = match &ty.scalar {
            ScalarType::I8 => "i8",
            ScalarType::I16 => "i16",
            ScalarType::I32 => "i32",
            ScalarType::I64 => "i64",
            ScalarType::U8 => "u8",
            ScalarType::U16 => "u16",
            ScalarType::U32 => "u32",
            ScalarType::U64 => "u64",
            ScalarType::F32 => "f32",
            ScalarType::F64 => "f64",
            ScalarType::Bool => unreachable!("handled above"),
            ScalarType::Usize => "u64", // wire-coerced
            ScalarType::Isize => "i64",
        };
        (
            format!("|v: &Vec<{rs}>| wire::enc_vec(v, {rs}::to_le_bytes)"),
            format!("|b: &[u8]| wire::dec_vec(b, {rs}::from_le_bytes)"),
        )
    }
}

fn scalar_fn_suffix(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::I8 => "i8",
        ScalarType::I16 => "i16",
        ScalarType::I32 => "i32",
        ScalarType::I64 => "i64",
        ScalarType::U8 => "u8",
        ScalarType::U16 => "u16",
        ScalarType::U32 => "u32",
        ScalarType::U64 => "u64",
        ScalarType::F32 => "f32",
        ScalarType::F64 => "f64",
        ScalarType::Bool => "bool",
        ScalarType::Usize => "u64",
        ScalarType::Isize => "i64",
    }
}

pub fn scalar_width(t: &ScalarType) -> usize {
    match t {
        ScalarType::I8 | ScalarType::U8 | ScalarType::Bool => 1,
        ScalarType::I16 | ScalarType::U16 => 2,
        ScalarType::I32 | ScalarType::U32 | ScalarType::F32 => 4,
        ScalarType::I64 | ScalarType::U64 | ScalarType::F64 => 8,
        ScalarType::Usize | ScalarType::Isize => 8,
    }
}
