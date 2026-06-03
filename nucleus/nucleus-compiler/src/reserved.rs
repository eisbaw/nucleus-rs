//! Rust-keyword reserved set for DSL identifiers (TASK-0433).
//!
//! # Why this exists
//!
//! Both sublanguage parsers ([`crate::algo::parser`] and
//! [`crate::sched::parser`]) admit any `[A-Za-z_][A-Za-z0-9_]*`
//! identifier that is not a *grammar* keyword. Those identifiers are
//! emitted **verbatim** into generated Rust by every backend: a data
//! symbol becomes `let mut {name} = …`, a kernel becomes
//! `kernels::{name}`, a worker becomes a thread/process binding. So a
//! `.nuc` symbol named `in` / `match` / `loop` / `fn` / `move` / … is
//! accepted by the front-end and then produces a generated crate that
//! fails `rustc` with an error pointing at code the user never wrote
//! (the project's "panic-not-diagnostic / usability-footgun" class —
//! surfaced concretely in TASK-0431, where a data symbol `in` lowered
//! to `let mut in = …` and was worked around by renaming `in`→`src`).
//!
//! This module is the **single source of truth** for the set of Rust
//! keywords a Nucleus identifier may not equal. It is CONCEPTUALLY
//! DISTINCT from each parser's grammar `KEYWORDS` const: a grammar
//! keyword (`data`, `kernel`, `for`, `usize`, …) has *syntactic*
//! meaning in the DSL; a word here is rejected *only* because codegen
//! cannot emit it as a bare Rust binding/path segment. The grammar
//! reject is checked first and wins for the overlap (`const`, `for`),
//! so those keep their grammar-specific diagnostic; this set fires
//! only for the remainder.
//!
//! # Strategy: fail-loud reject, not `r#`-escape
//!
//! We reject at the source site rather than `r#`-escaping in codegen
//! because (1) it localizes the fix to two parser chokepoints instead
//! of every backend's `let mut {name}` site, and (2) raw identifiers
//! do **not** cover everything anyway — `r#crate`, `r#self`,
//! `r#super`, `r#Self` are rejected by `rustc`, so even an escaping
//! strategy would still need a source-site reject for those four.
//!
//! # Determinism
//!
//! The set is a sorted `&[&str]` checked with [`slice::contains`], not
//! a `HashSet` — the parser-error-determinism invariant
//! (`project-chumsky-error-determinism`) forbids any HashSet iteration
//! on an error path. The sort is also a human-auditable property of
//! the source.

/// Rust strict keywords (2015 + 2018 editions) and reserved keywords,
/// per the Rust reference, that a Nucleus identifier may not equal.
///
/// Sorted lexicographically (enforced by a unit test below) so the
/// membership probe order is a stable, auditable property of the
/// source — no hash-set iteration on the error path.
///
/// Edition note on the strict-vs-contextual boundary. `dyn`, `async`,
/// `await` are STRICT keywords (2018+), so a bare Rust binding named
/// for any of them is illegal — they MUST be in the set. `try` is a
/// reserved word (2018) and `gen` is reserved (2024); both are
/// included as conservative reserved-set members. `union` is the one
/// genuinely CONTEXTUAL keyword present here — it is legal as a plain
/// Rust identifier, so rejecting it is a conservative over-inclusion,
/// not a correctness requirement; it costs the user nothing (no
/// example uses it). Words that are contextual *but never collide with
/// a bare binding/path segment* — e.g. `macro_rules`, `raw` — are
/// deliberately NOT in the set.
pub const RUST_RESERVED: &[&str] = &[
    "Self",
    "abstract",
    "as",
    "async",
    "await",
    "become",
    "box",
    "break",
    "const",
    "continue",
    "crate",
    "do",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "final",
    "fn",
    "for",
    "gen",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "macro",
    "match",
    "mod",
    "move",
    "mut",
    "override",
    "priv",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "union",
    "unsafe",
    "unsized",
    "use",
    "virtual",
    "where",
    "while",
    "yield",
];

/// Returns `true` if `s` is a Rust keyword that cannot be emitted as a
/// bare Nucleus identifier (see [`RUST_RESERVED`]).
#[must_use]
pub fn is_rust_reserved(s: &str) -> bool {
    RUST_RESERVED.contains(&s)
}

/// The diagnostic message for a Nucleus identifier that collides with
/// a Rust keyword. Kept here (not inlined at the two parser call
/// sites) so the wording is a single source of truth and the two
/// chokepoints can never drift (silent-sibling defense).
///
/// The message names the codegen reason explicitly so the user
/// understands the constraint is about *generated* Rust, not the DSL
/// grammar — distinguishing it from the grammar `KEYWORDS` reject's
/// "expected identifier, found keyword `X`".
#[must_use]
pub fn collision_message(ident: &str) -> String {
    format!(
        "`{ident}` cannot be used as a Nucleus identifier: it is a Rust \
         reserved word and would collide with generated code (rename it, \
         e.g. `{ident}_`)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_reserved_is_sorted_and_unique() {
        // Determinism + auditability: the set must stay sorted so the
        // membership probe order is stable, and unique so no entry is
        // accidentally double-listed.
        let mut sorted = RUST_RESERVED.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            RUST_RESERVED,
            sorted.as_slice(),
            "RUST_RESERVED must be kept lexicographically sorted"
        );
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(
            sorted.len(),
            deduped.len(),
            "RUST_RESERVED must not contain duplicates"
        );
    }

    #[test]
    fn known_collisions_are_reserved() {
        // The concrete TASK-0431 trigger plus the raw-identifier-
        // INCOMPATIBLE four (crate/self/super/Self) that even an
        // `r#`-escape strategy could not have rescued.
        for kw in ["in", "let", "match", "move", "loop", "fn", "type", "as"] {
            assert!(is_rust_reserved(kw), "`{kw}` must be reserved");
        }
        for kw in ["crate", "self", "super", "Self"] {
            assert!(
                is_rust_reserved(kw),
                "raw-incompatible keyword `{kw}` must be reserved"
            );
        }
    }

    #[test]
    fn near_misses_are_not_reserved() {
        // The reject must not over-fire on identifiers that merely
        // contain a keyword as a prefix/substring.
        for ok in ["in_", "match_thing", "selfish", "crater", "iN", "loops"] {
            assert!(!is_rust_reserved(ok), "`{ok}` must NOT be reserved");
        }
    }

    #[test]
    fn collision_message_names_the_ident_and_reason() {
        let msg = collision_message("match");
        assert!(msg.contains("`match`"), "message must quote the ident");
        assert!(
            msg.contains("Rust reserved word"),
            "message must name the codegen-collision reason"
        );
    }
}
