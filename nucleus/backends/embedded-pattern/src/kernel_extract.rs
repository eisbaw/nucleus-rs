//! Verbatim extraction of a `pub fn <name>` definition from a
//! `kernels.rs` source string (TASK-0047).
//!
//! # Why textual extraction (and not a parse / re-emit)
//!
//! PRD §6.2.2: "Nucleus does not interpolate text into kernel bodies;
//! they are compiled by the host toolchain unmodified." The embedded
//! backend honours that by copying the PURE kernel's source bytes
//! verbatim into the generated `no_std` lib — no re-rendering of the
//! body, no AST round-trip. A full Rust parser would be the heavyweight
//! alternative, but it would also have to RE-EMIT the body (losing the
//! "unmodified" guarantee unless it preserves spans exactly). Verbatim
//! byte extraction is the simplest defensible approach (the orchestrator
//! brief's option (a)), and the pure kernels are tiny.
//!
//! # Honest limitations of the textual extractor
//!
//! The brace-matcher is deliberately simple: it finds `pub fn <name>`
//! at a word boundary, scans to the first `{` (the body open brace),
//! then brace-counts to the matching close. It does NOT tokenise, so a
//! `{` or `}` inside a string / char / comment literal INSIDE a kernel
//! body would miscount. The tier-1 pure kernels (`add`, `blur3`) contain
//! none, so this is correct for the M9 examples. A kernel body that
//! needs string/char braces would require the full-parser path — filed
//! as a documented future-work boundary (TASK-0361).
//!
//! The failure mode is NOT uniform — be precise about which direction
//! fails how:
//! - A stray *opening* brace inside a literal, or a genuinely
//!   unbalanced body (missing close), scans off the end and returns
//!   `None` → a loud `ContractGap` at the call site.
//! - A stray *closing* brace inside a string / char / comment makes the
//!   matcher stop EARLY and return a TRUNCATED body (`Some(..)`). That
//!   truncation is NOT caught here; it surfaces as a Rust syntax error
//!   when the generated `no_std` lib is `cargo check`ed. Still loud, but
//!   at the codegen layer, not as a backend `ContractGap`.
//!
//! The tier-1 pure kernels (`add`, `blur3`) contain no literal braces,
//! so neither path triggers for the M9 examples. The robust fix (a
//! tokeniser, or a re-parse sanity check of the extracted span) is
//! filed as TASK-0361.

/// Extract the full `pub fn <name>(...) { ... }` definition (the
/// signature and body together, verbatim) from `src`. Returns `None` if
/// no `pub fn <name>` is found at a word boundary, or if the braces do
/// not balance (a malformed / unsupported body — fail loud at the
/// caller).
pub fn extract_pub_fn(src: &str, name: &str) -> Option<String> {
    let start = find_pub_fn_start(src, name)?;
    // Find the body's opening brace at/after the signature start. The
    // signature may span multiple lines (e.g. `blur3`'s 9 params).
    let open_rel = src[start..].find('{')?;
    let open = start + open_rel;

    // Brace-count from the opening brace to its match.
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    // Inclusive of the closing brace.
                    return Some(src[start..=i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    // Unbalanced braces — fail loud at the caller.
    None
}

/// Find the byte offset of `pub fn <name>` where `<name>` is followed by
/// a non-identifier byte (so `add` does not match `address`). Scans for
/// `fn ` occurrences to keep the match robust to whitespace between
/// `pub` and `fn`.
fn find_pub_fn_start(src: &str, name: &str) -> Option<usize> {
    let needle = format!("fn {name}");
    let mut search_from = 0usize;
    while let Some(rel) = src[search_from..].find(&needle) {
        let fn_pos = search_from + rel;
        // The byte AFTER `fn <name>` must not be an identifier
        // continuation (so `fn add` does not match inside `fn address`).
        let after = fn_pos + needle.len();
        let boundary_ok = src.as_bytes().get(after).is_none_or(|b| {
            !(b.is_ascii_alphanumeric() || *b == b'_')
        });
        if boundary_ok && is_pub_before(src, fn_pos) {
            // Anchor the returned span at `pub` so the emitted def keeps
            // the visibility modifier (it lives in a private `mod
            // kernels`, so `pub` keeps it reachable from `run`).
            return Some(pub_anchor(src, fn_pos));
        }
        search_from = fn_pos + needle.len();
    }
    None
}

/// True iff the token immediately preceding `fn_pos` (skipping
/// whitespace) is `pub`. Pure kernels are declared `pub fn` in the
/// tier-1 convention; a private `fn` is not a Nuc kernel surface.
fn is_pub_before(src: &str, fn_pos: usize) -> bool {
    let prefix = src[..fn_pos].trim_end();
    prefix.ends_with("pub")
}

/// The byte offset of the `pub` keyword preceding the `fn` at `fn_pos`
/// (whitespace-skipping). Caller guarantees `is_pub_before` held.
fn pub_anchor(src: &str, fn_pos: usize) -> usize {
    let prefix = src[..fn_pos].trim_end();
    prefix.len() - "pub".len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_single_line_fn() {
        let src = "use std::env;\n\npub fn add(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}\n\npub fn other() {}\n";
        let got = extract_pub_fn(src, "add").expect("add present");
        assert_eq!(got, "pub fn add(a: i32, b: i32) -> i32 {\n    a.wrapping_add(b)\n}");
    }

    #[test]
    fn extracts_multiline_signature_fn() {
        let src = "pub fn blur3(\n    p0: i32,\n    p1: i32,\n) -> i32 {\n    let s = p0.wrapping_add(p1);\n    s / 2\n}\n";
        let got = extract_pub_fn(src, "blur3").expect("blur3 present");
        assert!(got.starts_with("pub fn blur3("));
        assert!(got.ends_with("s / 2\n}"));
        assert!(got.contains("p1: i32,"));
    }

    #[test]
    fn handles_nested_braces_in_body() {
        let src = "pub fn f() -> i32 {\n    let x = { 1 + 2 };\n    x\n}\n";
        let got = extract_pub_fn(src, "f").expect("f present");
        assert_eq!(got, "pub fn f() -> i32 {\n    let x = { 1 + 2 };\n    x\n}");
    }

    #[test]
    fn word_boundary_does_not_match_prefix() {
        // `address` must NOT be matched when asking for `add`.
        let src = "pub fn address() -> i32 { 0 }\npub fn add(a: i32) -> i32 { a }\n";
        let got = extract_pub_fn(src, "add").expect("add present");
        assert_eq!(got, "pub fn add(a: i32) -> i32 { a }");
    }

    #[test]
    fn missing_fn_returns_none() {
        let src = "pub fn add(a: i32) -> i32 { a }\n";
        assert!(extract_pub_fn(src, "nonexistent").is_none());
    }

    #[test]
    fn private_fn_is_not_extracted() {
        // A non-`pub` helper (e.g. read_i32_le_slice) is not a kernel
        // surface; the extractor only anchors on `pub fn`.
        let src = "fn helper() -> i32 { 0 }\n";
        assert!(extract_pub_fn(src, "helper").is_none());
    }

    #[test]
    fn unbalanced_braces_returns_none() {
        let src = "pub fn broken() -> i32 {\n    let x = 1;\n";
        assert!(extract_pub_fn(src, "broken").is_none());
    }
}
