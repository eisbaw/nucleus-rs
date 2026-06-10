//! Shared kernels.rs I/O scaffolding emitted by every family.
//!
//! Every generated `kernels.rs` needs the same `load_input*` / `save_output`
//! plumbing reading/writing the little-endian i32 layout the harness uses.
//! Factoring it here keeps each family's `kernels_src` to JUST its compute
//! kernels, and keeps the one place that defines the on-disk layout single-
//! sourced (so a layout change can't drift between families).

use std::fmt::Write as _;

/// Emit the kernels.rs header (`use` lines + a `const N`) shared by the
/// flat (1-D) families.
pub(crate) fn kernels_header(n: usize) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "use std::env;");
    let _ = writeln!(s, "use std::fs;");
    let _ = writeln!(s, "use std::io::Write;");
    let _ = writeln!(s);
    let _ = writeln!(s, "const N: usize = {n};");
    let _ = writeln!(s);
    s
}

/// Emit a `pub fn <name>() -> Vec<i32>` that reads `count` i32 words
/// starting at word offset `start` from the input file. `count` is given
/// as a Rust expression string (e.g. `"N"` or `"H * W"`).
pub(crate) fn emit_load(name: &str, start_words: &str, count_words: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "pub fn {name}() -> Vec<i32> {{");
    let _ = writeln!(
        s,
        "    let path = env::var(\"NUC_INPUT_PATH\").unwrap_or_else(|_| \"input.bin\".to_string());"
    );
    let _ = writeln!(s, "    read_i32_le_slice(&path, {start_words}, {count_words})");
    let _ = writeln!(s, "}}");
    let _ = writeln!(s);
    s
}

/// Emit `pub fn save_output(data: Vec<i32>)` writing little-endian i32.
/// `expect_len` is a Rust expression string for the asserted length.
pub(crate) fn emit_save(expect_len: &str) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "pub fn save_output(data: Vec<i32>) {{");
    let _ = writeln!(s, "    assert_eq!(data.len(), {expect_len});");
    let _ = writeln!(
        s,
        "    let path = env::var(\"NUC_OUTPUT_PATH\").unwrap_or_else(|_| \"output.bin\".to_string());"
    );
    let _ = writeln!(s, "    let mut bytes = Vec::with_capacity(data.len() * 4);");
    let _ = writeln!(s, "    for v in &data {{");
    let _ = writeln!(s, "        bytes.extend_from_slice(&v.to_le_bytes());");
    let _ = writeln!(s, "    }}");
    let _ = writeln!(
        s,
        "    let mut f = fs::File::create(&path).expect(\"create output\");"
    );
    let _ = writeln!(s, "    f.write_all(&bytes).expect(\"write output\");");
    let _ = writeln!(s, "}}");
    let _ = writeln!(s);
    s
}

/// Emit the `read_i32_le_slice` helper (shared by every `load_*`).
pub(crate) fn emit_read_helper() -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "fn read_i32_le_slice(path: &str, start: usize, count: usize) -> Vec<i32> {{"
    );
    let _ = writeln!(s, "    let bytes = fs::read(path).expect(\"read input\");");
    let _ = writeln!(s, "    let need = (start + count) * 4;");
    let _ = writeln!(s, "    assert!(bytes.len() >= need);");
    let _ = writeln!(s, "    let mut out = Vec::with_capacity(count);");
    let _ = writeln!(s, "    for i in 0..count {{");
    let _ = writeln!(s, "        let off = (start + i) * 4;");
    let _ = writeln!(s, "        out.push(i32::from_le_bytes([");
    let _ = writeln!(
        s,
        "            bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3],"
    );
    let _ = writeln!(s, "        ]));");
    let _ = writeln!(s, "    }}");
    let _ = writeln!(s, "    out");
    let _ = writeln!(s, "}}");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_declares_n() {
        assert!(kernels_header(42).contains("const N: usize = 42;"));
    }

    #[test]
    fn load_reads_named_slice() {
        let s = emit_load("load_input_b", "N", "N");
        assert!(s.contains("pub fn load_input_b() -> Vec<i32>"));
        assert!(s.contains("read_i32_le_slice(&path, N, N)"));
    }

    #[test]
    fn save_asserts_length() {
        let s = emit_save("BINS");
        assert!(s.contains("assert_eq!(data.len(), BINS);"));
    }
}
