//! Algorithm-grammar token / lexical layer.
//!
//! The leaf combinators of the `*.algo.nuc` grammar — whitespace,
//! padding, identifiers, the loop-variable position, integer literals,
//! scalar-type keywords, and the reserved-word matcher — extracted from
//! [`super::parser`] (TASK-0435) so the parser module stays under the
//! 1000-LoC mega-file fence. Pure mechanical move: no behaviour change.
//!
//! The identifier *token shape* and the reserved-word reject *decision*
//! are not duplicated here; they live in [`crate::lexical`] and are
//! shared with the schedule parser. This module supplies the algorithm
//! grammar's own [`KEYWORDS`] set to that shared decision.

use chumsky::prelude::*;

use super::ast::{ScalarType, SpIdent};
use crate::lexical::{ident_chars, ident_collision_message};
use crate::span::Spanned;

/// Reserved words — identifiers may not collide with these. Listed
/// explicitly per grammar §6 note 4.
pub(super) const KEYWORDS: &[&str] = &[
    "const",
    "data",
    "kernel",
    "pure",
    "effectful",
    "for",
    // `until` is the bounded early-exit loop halt-clause keyword
    // (`for i : 0 .. N until COND { … }`, TASK-0341.02.01.03 / epic S1).
    // Reserved so it cannot be a body/iter identifier — this keeps the
    // optional-`until` clause LL(1) (a single token distinguishes the
    // `until` clause from the `{` body-opener after the upper bound).
    "until",
    "usize",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "i8",
    "i16",
    "i32",
    "i64",
    "f32",
    "f64",
    "bool",
];

/// Whitespace + line comments. Grammar §1 lexical rules.
pub(super) fn comment_or_ws() -> impl Parser<char, (), Error = Simple<char>> + Clone {
    let line_comment = just("//")
        .then(take_until(text::newline().or(end())))
        .ignored();
    line_comment
        .or(one_of(" \t\r\n").ignored())
        .repeated()
        .ignored()
}

/// Helper: capture a node's span on the bare token **before** the
/// trailing whitespace/comment padding is consumed, so the span is the
/// tight extent of the source text the node was parsed from (no
/// trailing layout). Leading layout is already consumed by the
/// *previous* token's pad, matching `ParseError`'s offset convention.
///
/// This is the span-correctness primitive (TASK-0082): `pad(p)` alone
/// would, if `.map_with_span`-wrapped on the outside, fold the trailing
/// whitespace into the span — wrong for a diagnostic that underlines a
/// token. `padded_spanned(p)` wraps with the span fixed first, then
/// eats trailing layout.
pub(super) fn padded_spanned<P, T>(
    p: P,
) -> impl Parser<char, Spanned<T>, Error = Simple<char>> + Clone
where
    P: Parser<char, T, Error = Simple<char>> + Clone,
{
    p.map_with_span(Spanned::new).then_ignore(comment_or_ws())
}

/// Helper: token followed by trailing whitespace/comments.
pub(super) fn pad<P, T>(p: P) -> impl Parser<char, T, Error = Simple<char>> + Clone
where
    P: Parser<char, T, Error = Simple<char>> + Clone,
{
    p.then_ignore(comment_or_ws())
}

/// Identifier matcher. Rejects keywords. Yields a [`SpIdent`] whose
/// span is exactly the identifier token's byte range (no surrounding
/// whitespace — `pad` consumes trailing space *after* this combinator),
/// so an "undeclared / duplicate `X`" diagnostic underlines just `X`.
///
/// Token shape and reject decision are the shared
/// [`crate::lexical::ident_chars`] / [`ident_collision_message`], fed
/// the algorithm grammar's [`KEYWORDS`].
pub(super) fn ident() -> impl Parser<char, SpIdent, Error = Simple<char>> + Clone {
    ident_chars()
        .try_map(|s, span| match ident_collision_message(&s, KEYWORDS) {
            Some(msg) => Err(Simple::custom(span, msg)),
            None => Ok(s),
        })
        .map_with_span(Spanned::new)
}

/// The loop-variable position of `for VAR : lo .. hi { … }` (TASK-0434).
///
/// On a valid `VAR` this is exactly [`ident`] (same `SpIdent`, no extra
/// consumption — positive for-loops and codegen unchanged). The
/// difference is the ERROR PATH: `ident`'s reject fires at the variable
/// position, but `for_stmt` is in a `choice` and chumsky 0.9 merges
/// alternative errors by furthest input position (greater `at` wins —
/// `chumsky::error::Located::max`), so the trailing-`{` mismatch PAST
/// the keyword `VAR` won the merge and the user saw a confusing "found
/// `{`" (TASK-0433 pinned this). Mirroring
/// `sched_directive_hint_stmt` (in [`super::parser`]): on a collision we
/// `take_until` the `{`, pushing our error's `at` past the brace,
/// while keeping its display span on `var_span` so the diagnostic
/// underlines `VAR`. Message is the shared [`ident_collision_message`]
/// — identical to the data/kernel one.
///
/// The `take_until` terminator is `'{' | end-of-input` (TASK-0434.01):
/// on TRUNCATED input with no opening brace (e.g. `for loop : 0 .. N`
/// at EOF) the bare `just('{')` form would run `take_until` to EOF and
/// FAIL before the `try_map` emits the VAR-anchored message, so the
/// user got a generic "found end of input" at EOF instead. Adding the
/// `end()` alternative lets `take_until` terminate at EOF too, so the
/// VAR-anchored diagnostic fires on the brace-less case as well. The
/// braced case is unchanged: `take_until` is non-greedy and still stops
/// at the FIRST `{` (pinned by
/// `for_loop_var_keyword_collision_is_anchored_at_the_variable_token`;
/// brace-less case pinned by `for_loop_var_keyword_collision_anchored_on_truncated_braceless_input`).
pub(super) fn for_loop_var() -> impl Parser<char, SpIdent, Error = Simple<char>> + Clone {
    ident_chars()
        .map_with_span(|s, span: std::ops::Range<usize>| (s, span))
        // Both arms are `.boxed()` to one concrete parser type (the
        // `then_with` closure must return a single `P`). See the
        // docstring for the chumsky furthest-`at` merge rationale.
        .then_with(|(s, var_span)| match ident_collision_message(&s, KEYWORDS) {
            None => {
                let ident = Spanned::new(s.clone(), var_span.clone());
                empty().map(move |()| ident.clone()).boxed()
            }
            // Terminate at the first `{` OR end-of-input (both `()`-typed
            // so the `.or()` arms unify) so the VAR-anchored error also
            // fires on truncated brace-less input (TASK-0434.01).
            Some(msg) => take_until(just('{').ignored().or(end()))
                .try_map(move |_, _outer_span| {
                    Err(Simple::custom(var_span.clone(), msg.clone()))
                })
                .boxed(),
        })
}

/// Integer literal — decimal only (grammar §1).
pub(super) fn int_lit() -> impl Parser<char, i64, Error = Simple<char>> + Clone {
    filter(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .try_map(|s, span| {
            s.parse::<i64>()
                .map_err(|e| Simple::custom(span, format!("invalid integer `{}`: {}", s, e)))
        })
}

/// Scalar-type keyword.
pub(super) fn scalar_type() -> impl Parser<char, ScalarType, Error = Simple<char>> + Clone {
    // Order matters within `choice` when prefixes overlap (e.g. `i8`
    // vs `i16`); `text::keyword` would help but isn't in chumsky 0.9
    // for this token shape. We disambiguate by sorting longest-first.
    // Match an exact scalar-type keyword and ensure it is not the
    // prefix of a longer identifier (e.g. `u8` must not start `u8_t`).
    // We use `rewind()` on a one-char alnum/_ peek so the check is
    // truly non-consuming and the alternation in `choice` below
    // remains well-behaved.
    let kw = |s: &'static str, t: ScalarType| {
        just(s)
            .then_ignore(
                none_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_").rewind(),
            )
            .to(t)
    };
    choice((
        kw("usize", ScalarType::Usize),
        kw("isize", ScalarType::Isize),
        kw("u16", ScalarType::U16),
        kw("u32", ScalarType::U32),
        kw("u64", ScalarType::U64),
        kw("u8", ScalarType::U8),
        kw("i16", ScalarType::I16),
        kw("i32", ScalarType::I32),
        kw("i64", ScalarType::I64),
        kw("i8", ScalarType::I8),
        kw("f32", ScalarType::F32),
        kw("f64", ScalarType::F64),
        kw("bool", ScalarType::Bool),
    ))
}

/// A reserved-word matcher that ensures the keyword is not the prefix
/// of a longer identifier (e.g. `for_each` does not start `for`).
pub(super) fn keyword(kw: &'static str) -> impl Parser<char, (), Error = Simple<char>> + Clone {
    // See `scalar_type` for the `rewind()` rationale: ensure the
    // keyword is not the prefix of a longer identifier without
    // consuming the lookahead character.
    just(kw)
        .then_ignore(
            none_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_").rewind(),
        )
        .ignored()
}
