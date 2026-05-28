//! Wire-encoding helpers for the sync-TCP multi-process backends.
//!
//! `scalar_width` / `scalar_fn_suffix` map [`nucleus_compiler::algo::
//! ScalarType`] to its byte width and `wire::enc_*` / `wire::dec_*`
//! suffix respectively; `encode_expr` / `decode_expr` build the call
//! expression against the generated crate's `wire.rs` that the emitter
//! splices into Push/Wait sites. The encoder is fixed at compile time
//! from the sidecar (TASK-0037 AC#3) — sender/receiver agree by
//! construction.
//!
//! Wire-primitive-agnostic: the on-wire byte format is identical for
//! the blocking (mp-tcp-bufsync) and the nonblocking-poll
//! (mp-tcp-poll) transports — only the WAIT primitive differs, and
//! that lives in [`super::WirePrimitives`], not here. Lifted from the
//! two backends' verbatim-duplicate `encode.rs` (TASK-0044.02.03).

use crate::render::rust_scalar_type_pub;
use crate::EmitError;

/// Byte width of a scalar on the only supported (x86-64 loopback)
/// target. Drives SO_*BUF sizing in `run.sh`.
pub fn scalar_width(t: &nucleus_compiler::algo::ScalarType) -> usize {
    use nucleus_compiler::algo::ScalarType::*;
    match t {
        I8 | U8 | Bool => 1,
        I16 | U16 => 2,
        I32 | U32 | F32 => 4,
        I64 | U64 | F64 => 8,
        // usize/isize: 8 on every supported (x86-64 loopback) target.
        Usize | Isize => 8,
    }
}

/// `wire::enc_*` / `wire::enc_vec(...)` call for a value named `name`
/// of resolved type `ty`. Encoder is fixed at compile time from the
/// sidecar (TASK-0037 AC#3) — sender/receiver agree by construction.
pub fn encode_expr(
    name: &str,
    ty: &nucleus_compiler::algo::ResolvedType,
) -> Result<String, EmitError> {
    use nucleus_compiler::algo::ScalarType::Bool;
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        Ok(format!("wire::enc_{s}({name})"))
    } else if ty.scalar == Bool {
        // `bool` has no `to_le_bytes`; dedicated 1-byte-per-element.
        Ok(format!("wire::enc_vec_bool(&{name})"))
    } else {
        let rs = rust_scalar_type_pub(&ty.scalar);
        Ok(format!("wire::enc_vec(&{name}, {rs}::to_le_bytes)"))
    }
}

/// Expression that decodes `__buf` back into the value's Rust type.
pub fn decode_expr(ty: &nucleus_compiler::algo::ResolvedType) -> Result<String, EmitError> {
    use nucleus_compiler::algo::ScalarType::Bool;
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        Ok(format!("wire::dec_{s}(&__buf)"))
    } else if ty.scalar == Bool {
        Ok("wire::dec_vec_bool(&__buf)".to_string())
    } else {
        let rs = rust_scalar_type_pub(&ty.scalar);
        Ok(format!("wire::dec_vec(&__buf, {rs}::from_le_bytes)"))
    }
}

/// `wire::enc_*` / `wire::dec_*` function-name suffix for a scalar.
pub fn scalar_fn_suffix(t: &nucleus_compiler::algo::ScalarType) -> &'static str {
    use nucleus_compiler::algo::ScalarType::*;
    match t {
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        F32 => "f32",
        F64 => "f64",
        Bool => "bool",
        // usize/isize encoded as their 8-byte counterparts on the
        // only supported target (x86-64 loopback). A mixed-width
        // target would bump the protocol version (which v0 lacks).
        Usize => "u64",
        Isize => "i64",
    }
}
