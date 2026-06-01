//! Property-based / fuzz tests for the two `.nuc` front-end parsers
//! (TASK-0399).
//!
//! ## Scope
//!
//! `parse_algo` (`src/algo/parser.rs`) and `parse_sched`
//! (`src/sched/parser.rs`) are the untrusted-input boundary of the
//! compiler. Three boundary invariants are pinned here as properties
//! over generated input; they complement (do NOT replace) the
//! hand-curated cases in `tests/algo_parser.rs` (1251 LoC) and
//! `tests/sched_parser.rs` (1325 LoC), which pin specific messages and
//! shapes. These drive the same two entry points with generated input.
//!
//! The three invariants:
//!
//! 1. PANIC-FREEDOM (recurring defect class: panic-not-diagnostic).
//!    Neither parser may panic / unwind on ANY input; malformed input
//!    must surface as a typed `Err(ParseErrors)`, never a crash.
//!    `proptest` catches a panic raised inside the test closure and
//!    shrinks to the minimal offending input, so a bare `parse_*(&s)`
//!    call under `proptest!` IS the panic assertion. (The parser entry
//!    carries an `out.expect("chumsky: empty error list implies a
//!    complete parse")` and the int/size literal lexers `try_map`
//!    overflow to a chumsky error rather than unwrapping — this fuzz
//!    pins that those defensive choices hold across the input space.)
//!
//! 2. `ParseErrors` NON-EMPTY INVARIANT (`src/error.rs` "Invariants").
//!    A `ParseErrors` is only constructed on failure and is documented
//!    non-empty; `ParseErrors::first` `.expect()`s exactly this. Every
//!    `Err`-producing generated input is asserted to carry at least one
//!    `ParseError`.
//!
//! 3. ERROR-DETERMINISM (PRD §10.1 reproducibility; TASK-0080/0081).
//!    chumsky 0.9 `Simple` Display is `HashSet`-backed and therefore
//!    non-deterministic; the fix routes every parser error through a
//!    sorting builder (`error::chumsky_message`). Parsing the SAME
//!    input twice must yield an identical `Result` — `AlgoAst` /
//!    `SchedAst` and `ParseErrors` all derive `PartialEq`, so any
//!    `HashSet`-iteration-order seep into the error payload would
//!    regress this property. (Mirrors the `b.2` determinism property in
//!    `tests/proptest_petri.rs`, which guards the Petri passes the same
//!    way.)
//!
//! ## Honest scope LIMIT
//!
//! This is panic / invariant / determinism FUZZING, NOT a valid-source
//! round-trip: the AST has no unparser, so `parse -> render -> parse`
//! is out of scope. Input length is bounded (`<= 200` chars; token soup
//! `<= 40` tokens) so each case is fast and pathological deep nesting is
//! unlikely. A true non-terminating input or a stack overflow on deep
//! nesting is NOT catchable by `proptest` (there is no per-case
//! timeout, and a stack overflow aborts rather than unwinds); were such
//! an input ever discovered it is a distinct recursion-depth concern to
//! be filed and fixed separately, not papered over here.

use proptest::prelude::*;

use nucleus_compiler::algo::parse_algo;
use nucleus_compiler::sched::parse_sched;

/// Lexical fragments of the two sublanguages plus structural
/// punctuation and whitespace. Concatenated "token soup" drives far
/// deeper into the grammar than random bytes — and therefore into the
/// parser states with multiple expected-token alternatives, which is
/// the exact place the chumsky `HashSet`-order bug used to surface.
/// Most entries are real `keyword(..)` / `just(..)` literals from the
/// two parsers; a few are multi-char operators the grammar builds from
/// char sequences rather than single literals (`<--`, `->`, `..`) and a
/// handful are plausible-but-not-live tokens (e.g. `&`, `|`, `irq`,
/// `barrier`). The set is deliberately broad and approximate: an entry
/// that is not (or is no longer) a real token only weakens the
/// generator — it lands in the "unexpected" bucket and cannot make a
/// property wrong.
const TOKENS: &[&str] = &[
    // algorithm keywords
    "kernel", "data", "for", "in", "pure", "effectful", "const",
    // scalar type keywords
    "usize", "isize", "u64", "i64", "i32", "u32", "bool", "f32", "f64",
    // schedule keywords
    "schedule", "place", "place_data", "workers", "transfer", "partition",
    "on", "loop", "reuse", "notify", "buffer", "async", "sync", "event",
    "poll", "blocking", "memory", "memory_region", "check", "on_violation",
    "panic", "log", "count", "rows", "blocks2d", "per_worker", "size",
    "latency_max", "true", "false", "none", "shared", "simd", "accessible_by",
    "worker_class", "irq", "barrier",
    // identifiers / numeric / unit literals
    "x", "foo", "out", "i", "j", "k", "N", "0", "1", "42", "1024",
    "B", "KB", "MB", "GB", "ns", "us", "ms", "s",
    // punctuation / structure
    "{", "}", "(", ")", "[", "]", "<", ">", ":", ";", ",", ".", "=",
    "<--", "->", "..", "//", "\"", "/", "*", "+", "-", "@", "%", "&", "|",
    // whitespace
    " ", "\n", "\t",
];

/// Token-soup strategy: 0..=40 tokens drawn from [`TOKENS`] and
/// concatenated. Bounded length keeps every case fast and avoids
/// pathological deep nesting (see the module-level scope LIMIT).
fn token_soup() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(TOKENS), 0..=40)
        .prop_map(|toks| toks.concat())
}

/// Arbitrary (possibly-newline-containing) UTF-8 up to 200 chars. The
/// `(?s)` flag lets `.` match newlines so control/line structure is in
/// the input space too.
const ARBITRARY_UTF8: &str = "(?s).{0,200}";

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// AC#1 + AC#2 (algo, arbitrary UTF-8): `parse_algo` returns
    /// without panicking on arbitrary input, and any `Err` is non-empty.
    #[test]
    fn algo_arbitrary_utf8_never_panics_and_err_nonempty(s in ARBITRARY_UTF8) {
        if let Err(e) = parse_algo(&s) {
            prop_assert!(
                !e.is_empty(),
                "parse_algo returned an EMPTY ParseErrors (non-empty invariant violated) for {s:?}",
            );
        }
    }

    /// AC#1 + AC#2 (sched, arbitrary UTF-8).
    #[test]
    fn sched_arbitrary_utf8_never_panics_and_err_nonempty(s in ARBITRARY_UTF8) {
        if let Err(e) = parse_sched(&s) {
            prop_assert!(
                !e.is_empty(),
                "parse_sched returned an EMPTY ParseErrors (non-empty invariant violated) for {s:?}",
            );
        }
    }

    /// AC#1 + AC#2 (algo, token soup): same invariants under grammar
    /// token soup, which reaches deeper into the parser than random bytes.
    #[test]
    fn algo_token_soup_never_panics_and_err_nonempty(s in token_soup()) {
        if let Err(e) = parse_algo(&s) {
            prop_assert!(
                !e.is_empty(),
                "parse_algo returned an EMPTY ParseErrors for token soup {s:?}",
            );
        }
    }

    /// AC#1 + AC#2 (sched, token soup).
    #[test]
    fn sched_token_soup_never_panics_and_err_nonempty(s in token_soup()) {
        if let Err(e) = parse_sched(&s) {
            prop_assert!(
                !e.is_empty(),
                "parse_sched returned an EMPTY ParseErrors for token soup {s:?}",
            );
        }
    }

    /// AC#3 (algo): determinism. The same input parses to an identical
    /// `Result` on repeated calls; pins the chumsky `HashSet`-order sort
    /// fix (TASK-0080/0081) against a regression that re-introduces
    /// non-deterministic error payloads.
    #[test]
    fn algo_parse_is_deterministic(s in token_soup()) {
        prop_assert_eq!(parse_algo(&s), parse_algo(&s));
    }

    /// AC#3 (sched): determinism.
    #[test]
    fn sched_parse_is_deterministic(s in token_soup()) {
        prop_assert_eq!(parse_sched(&s), parse_sched(&s));
    }

    /// AC#3 (algo, arbitrary UTF-8): determinism over the wider random
    /// input space too, not just token soup.
    #[test]
    fn algo_parse_is_deterministic_arbitrary(s in ARBITRARY_UTF8) {
        prop_assert_eq!(parse_algo(&s), parse_algo(&s));
    }

    /// AC#3 (sched, arbitrary UTF-8): determinism over the wider random
    /// input space too.
    #[test]
    fn sched_parse_is_deterministic_arbitrary(s in ARBITRARY_UTF8) {
        prop_assert_eq!(parse_sched(&s), parse_sched(&s));
    }
}
