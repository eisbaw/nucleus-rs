//! Shared lexical primitives for the algorithm and schedule parsers.
//!
//! Both grammars share the same identifier *token shape*
//! (`[A-Za-z_][A-Za-z0-9_]*`) and the same two-stage reserved-word
//! reject *decision*: the grammar's own `KEYWORDS` set is checked
//! FIRST, then the Rust-reserved codegen-collision set via
//! [`crate::reserved`]. Before TASK-0435 each parser carried its own
//! copy of both. The algo copy was the `ident_collision_message`
//! single-source-of-truth introduced by TASK-0434, but the schedule
//! `ident()` still inlined a byte-identical twin, so a future third
//! reserved class would have had to be added in two places — a live
//! silent-sibling hazard (`feedback-silent-sibling-defect`). They are
//! unified here.
//!
//! The two grammars do NOT share their `KEYWORDS` *sets* (algo reserves
//! `data`/`kernel`/scalar-type words; sched reserves
//! `block`/`place`/…), so `ident_collision_message` takes the
//! caller's keyword slice as a parameter rather than closing over a
//! single global — passing each parser's own set preserves its exact
//! diagnostics.

use chumsky::prelude::*;

/// Raw identifier characters `[A-Za-z_][A-Za-z0-9_]*` as a `String`,
/// with NO reserved-word check (that is the caller's job). Shared by
/// both parsers' `ident` combinators so the token shape lives in one
/// place.
pub(crate) fn ident_chars() -> impl Parser<char, String, Error = Simple<char>> + Clone {
    filter(|c: &char| c.is_ascii_alphabetic() || *c == '_')
        .chain(filter(|c: &char| c.is_ascii_alphanumeric() || *c == '_').repeated())
        .collect::<String>()
}

/// SINGLE source of truth for the identifier reject decision + wording,
/// shared by the algorithm and schedule parsers.
///
/// `Some(message)` ⇒ `s` collides with a reserved word and must be
/// rejected; `None` ⇒ legal identifier. `keywords` is the *caller's*
/// grammar keyword set (algo and sched reserve different words),
/// checked FIRST so an overlap with the Rust-reserved set keeps the
/// grammar message; a non-grammar Rust keyword falls through to the
/// codegen-collision message (TASK-0433: codegen emits `let mut {name}`
/// and rustc would fail).
pub(crate) fn ident_collision_message(s: &str, keywords: &[&str]) -> Option<String> {
    if keywords.contains(&s) {
        Some(format!("expected identifier, found keyword `{}`", s))
    } else if crate::reserved::is_rust_reserved(s) {
        Some(crate::reserved::collision_message(s))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirror the algorithm grammar's overlap-with-Rust-reserved words:
    // `const` and `for` are BOTH grammar keywords and Rust-reserved.
    const SAMPLE_KEYWORDS: &[&str] = &["const", "data", "for"];

    #[test]
    fn legal_identifier_yields_none() {
        assert_eq!(ident_collision_message("acc", SAMPLE_KEYWORDS), None);
        assert_eq!(ident_collision_message("x_1", SAMPLE_KEYWORDS), None);
    }

    #[test]
    fn grammar_keyword_checked_first_keeps_grammar_message() {
        // `const` is in BOTH sets; the grammar message must win because
        // KEYWORDS is consulted first.
        assert_eq!(
            ident_collision_message("const", SAMPLE_KEYWORDS),
            Some("expected identifier, found keyword `const`".to_string()),
        );
    }

    #[test]
    fn non_grammar_rust_keyword_falls_through_to_collision_message() {
        // `match` is Rust-reserved but not a grammar keyword: it must
        // get the codegen-collision message, not the grammar one.
        assert_eq!(
            ident_collision_message("match", SAMPLE_KEYWORDS),
            Some(crate::reserved::collision_message("match")),
        );
    }

    #[test]
    fn keyword_set_is_caller_specific() {
        // `data` is reserved in the sample (algo-like) set but a legal
        // identifier under an empty set — proving the parameterization.
        assert!(ident_collision_message("data", SAMPLE_KEYWORDS).is_some());
        assert_eq!(ident_collision_message("data", &[]), None);
    }
}
