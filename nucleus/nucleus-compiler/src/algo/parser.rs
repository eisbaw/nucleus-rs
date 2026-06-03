//! Hand-written parser for `*.algo.nuc` over the `chumsky` combinator
//! library. Implements the EBNF in `docs/grammar-algo.md`.
//!
//! # Parser-library choice
//!
//! Chosen: `chumsky` 0.9.
//!
//! Rationale:
//! - Library, not framework — composable combinators returning Rust
//!   functions; we keep grammar in Rust source rather than an external
//!   `.pest` / `.lalrpop` file (single source of truth, no codegen step).
//! - First-class span / position tracking on errors.
//! - Pure Rust dependency, builds offline, no proc-macro.
//! - Mature error-recovery primitives — we recover at the
//!   statement/item boundary (the `;` terminator) so one syntactic
//!   failure does not hide the rest of the program's errors
//!   (TASK-0081); see [`parse_algo`].
//!
//! Rejected alternatives:
//! - `lalrpop`: external grammar file + build script (codegen step);
//!   LR(1) diagnostics are harder to make user-friendly without manual
//!   error-mapping. The grammar is small enough that the LALRPOP win
//!   (formal-grammar derivation) is not worth the extra build complexity.
//! - `pest`: external `.pest` grammar file would split the source of
//!   truth from the AST types. The grammar doc is already informative;
//!   adding a second grammar artefact invites drift.
//! - `nom` / `winnow`: lower-level byte-stream combinators. Span and
//!   error reporting need significantly more hand-wiring. Fine choice
//!   in principle; chumsky has better DX for tree-shaped error reports.
//! - Hand-rolled recursive descent: rejected because chumsky's span and
//!   error machinery is exactly what we need; rebuilding it adds bug
//!   surface for no readability win.
//!
//! # Error reporting & recovery (TASK-0080 / TASK-0081)
//!
//! `parse_algo` reports **every** parse error in a single pass, not
//! just the first. On a syntactic failure inside one top-level item
//! the parser recovers by skipping to (and past) the next `;`
//! statement/declaration terminator and resuming item parsing, so
//! later valid items are still parsed and later errors still
//! reported. Recovery is **bounded** (each recovery step consumes at
//! least one input character or stops at end-of-input — no
//! infinite-retry) and **deterministic** (chumsky's positional error
//! order is preserved; exact-duplicate diagnostics are collapsed by
//! an order-preserving scan, no hashing — see
//! [`crate::error::map_all_chumsky_errors`]). The errors are bundled
//! into a non-empty [`ParseErrors`]; valid input is wholly unaffected
//! (same AST, byte-identical downstream codegen).
//!
//! # Recovery shape (TASK-0199 — brace-balanced sync)
//!
//! Recovery uses a brace-balanced `skip_parser` (see
//! `brace_balanced_recovery`) — NOT a `;`-only `skip_until`.
//! When a top-level item fails, recovery consumes ONE "logical item
//! span": either a bare stray `;`, or one-or-more outer atoms
//! followed by an optional terminating `;`. An outer atom is either
//! a recursively-balanced `{ … }` block (inner `;` absorbed as
//! nested content) or any single character that is NOT `{`, `}`, or
//! `;` at the outer depth.
//!
//! The historical `;`-only sync set had a genuine recovery defect:
//! a typo inside a `for { … }` body caused recovery to consume the
//! typo's inner `;` and land mid-body, producing a structural
//! close-`}` `Unexpected` follow-on (measured `constant 2`,
//! parametric over `n ∈ {0,1,2,5}` and out-of-fixture `{3,8,12}`).
//! Sched had the parallel `n + 2` linear cascade for the same root
//! reason. The brace-balanced recovery collapses both shapes to
//! **`EXACTLY 1`** error — the genuine primary alone — by consuming
//! the entire brace-delimited body wholesale (the recursive
//! `{ … }`-balanced inner content swallows all inner `;` and the
//! closing `}` as nested content, leaving the stream cleanly at
//! the next top-level item position or at EOF). Pinned post-fix by
//! `tests/algo_parser.rs::for_body_error_surfaces_single_primary_after_keyword_sync`
//! (algo, was `_constant_two_parametric`, TASK-0207) and
//! `tests/sched_parser.rs::nested_brace_body_error_surfaces_single_primary_after_keyword_sync_*`
//! (sched, was `_n_plus_two_parametric_*`, TASK-0087 cycle-4).
//!
//! Boundedness and determinism are preserved: each recovery
//! invocation consumes ≥1 character or fails non-consumingly so
//! `.repeated()` stops cleanly at EOF (no panic — chumsky 0.9.3's
//! `Repeated::parse_inner` requires non-zero progress per
//! successful element); all combinators are pure/positional
//! (`recursive`, `choice`, `just`, `none_of`, `or_not`) with no
//! hash-iteration.
//! - AST nodes carry per-node byte-range spans (TASK-0082): every
//!   wrapped node is built with `.map_with_span(Spanned::new)`, so a
//!   node's span is the `start..end` of exactly the source text it was
//!   parsed from (leading `pad`/comment whitespace is consumed by the
//!   *following* token, matching `ParseError`'s offset convention).
//!   Lowering still ignores spans (TASK-0090 wires them in).
//! - Semantic constraints (single-assignment, kernel-purity-vs-effect-
//!   statement, forward references) are deliberately NOT enforced here
//!   — they belong to TASK-0009 (AlgoIR lowering).

use chumsky::prelude::*;

use super::ast::{
    AlgoAst, BinOp, Call, ConstDecl, DataDecl, Expr, IndexedLValue, Item, KernelDecl, KernelSig,
    Purity, SpExpr, SpItem, SpStmt, Stmt, Type, UnaryOp,
};
use super::tokens::{
    comment_or_ws, for_loop_var, ident, int_lit, keyword, pad, padded_spanned, scalar_type,
};
use crate::error::map_all_chumsky_errors;
pub use crate::error::{ParseError, ParseErrorKind, ParseErrors};
use crate::lexical::ident_chars;
use crate::span::Spanned;

/// Internal: the tail of an identifier-led atom — either a call's
/// argument list or zero-or-more index suffixes. Kept local because it
/// has no meaning outside the expression parser.
enum IdentTail {
    Call(Vec<SpExpr>),
    Indices(Vec<SpExpr>),
}

/// Parse a `*.algo.nuc` source string into an [`AlgoAst`].
///
/// On success returns the [`AlgoAst`] exactly as before recovery was
/// added — valid input is wholly unaffected (same AST → byte-identical
/// downstream codegen; this is load-bearing and gated by
/// `determinism-check`).
///
/// On failure returns a non-empty [`ParseErrors`]: the parser recovers
/// at the `;` statement/item boundary so one syntactic error does not
/// hide the rest, and every error (each with its own correct
/// 1-based `(line, column)`) is reported in one pass. Recovery is
/// bounded and deterministic — see the module docs and
/// [`crate::error::map_all_chumsky_errors`].
///
/// We use chumsky's `parse_recovery`, which yields
/// `(Option<partial AST>, errors)`. We deliberately discard any
/// partial AST on failure: downstream passes require a structurally
/// complete program, and surfacing a half-recovered tree would only
/// produce confusing secondary errors. The contract is "valid → AST,
/// invalid → all parse errors", nothing in between.
pub fn parse_algo(src: &str) -> Result<AlgoAst, ParseErrors> {
    let parser = program_parser();
    let (out, errors) = parser.parse_recovery(src);
    if errors.is_empty() {
        // chumsky guarantees: empty error list ⇒ parse fully
        // succeeded ⇒ `out` is `Some`. This `expect` is an
        // earlier-guaranteed library invariant, not diagnosable user
        // input (decision-0003): a missing AST with no errors would be
        // a chumsky contract violation, not a user mistake.
        let items = out.expect("chumsky: empty error list implies a complete parse");
        Ok(AlgoAst { items })
    } else {
        Err(map_all_chumsky_errors(src, errors))
    }
}

// --------------------------------------------------------------------
// Grammar
// --------------------------------------------------------------------

/// Schedule-directive reserved words that, when they appear in
/// statement position of an algorithm file, get an actionable hint
/// ("did you mean to put it in a `*.sched.nuc` file?") rather than a
/// generic "unexpected `=`" / "unexpected ident" diagnostic
/// (TASK-0083; promised by `docs/grammar-algo.md` §3).
///
/// These keywords are NOT in [`KEYWORDS`](super::tokens::KEYWORDS) (the algorithm grammar's
/// reserved set) — they remain legal as plain identifiers (e.g.
/// `data block : f32[16];` is still accepted). The hint only fires
/// when the surrounding shape is unambiguously a schedule directive:
/// `<kw> =`, `place <ident>`, `place_data <ident>`, or
/// `check loop`. See [`sched_directive_hint_stmt`].
///
/// Sorted lexicographically so the (rare) probe-order is a stable,
/// human-auditable property of the source — no hash-set iteration.
const SCHED_RESERVED_EQ: &[&str] = &[
    "block",
    "buffer",
    "notify",
    "partition",
    "pipeline",
    "transfer",
    "unroll",
    // `vectorize` removed 2026-05-25 (TASK-0292). It is no longer a
    // schedule directive — SIMD is delegated to the host Rust
    // compiler.
];

/// Builds the top-level `Program ::= TopItem*` parser.
///
/// Wrapped in a function so callers don't depend on chumsky's exact
/// return type.
///
/// # Error recovery (TASK-0081 / TASK-0199)
///
/// The per-item parser is lifted to `Option<SpItem>` (`Some` on a
/// clean parse) and given
/// `recover_with(skip_parser(brace_balanced_recovery().map(|_| None)))`.
/// On a syntactic failure anywhere inside one top-level item,
/// chumsky:
///   1. records the error (collected by `parse_recovery`),
///   2. invokes [`brace_balanced_recovery`] to skip ONE logical item
///      span — either a stray `;` or one-or-more outer atoms then an
///      optional `;`, where an outer atom is a recursively-balanced
///      `{ … }` block (so a `for { stmt; stmt; … }` body is consumed
///      WHOLESALE, inner `;` and closing `}` absorbed as nested
///      content) or any non-`{`/`}`/`;` char,
///   3. recovers this element to the value `None`.
///
/// The `None` recovery value is the load-bearing difference from
/// `skip_then_retry_until` (chumsky 0.9): because the failed element
/// yields a value, the outer `.repeated()` *continues* to the next
/// item and can fail-and-recover again, so **every** broken item
/// contributes its own error — that is what makes multi-error
/// reporting actually work in chumsky 0.9 (`skip_then_retry_until`
/// surfaces only the single first error per recovery site and stops
/// the repetition once no further item parses). The `None`s are
/// stripped by `flatten`; on the failure path the partial AST is
/// discarded by `parse_algo` anyway, so no synthetic/placeholder node
/// ever reaches a caller and the `Item` AST shape is unchanged
/// (TASK-0082 substrate untouched).
///
/// Boundedness: each `brace_balanced_recovery` invocation consumes
/// ≥1 character or fails non-consumingly (the `lone_semicolon` arm
/// consumes exactly `;`; the `normal` arm requires at least one
/// initial atom). At EOF the recovery fails — `.repeated()` detects
/// the non-consuming failure and stops cleanly (no panic — chumsky
/// 0.9.3's `Repeated::parse_inner` requires non-zero progress per
/// successful element; see `combinator.rs:550-553`). Determinism:
/// every combinator in the recovery is pure/positional
/// (`recursive`/`choice`/`just`/`none_of`/`or_not`), no
/// hash-iteration; chumsky's error list is positional; the message
/// is rebuilt with a sorted expected-set ([`crate::error`]).
///
/// Brace-balanced sync collapses the brace-body cascade — see the
/// module-doc "Recovery shape" section for the pre-fix vs post-fix
/// numbers; pinned post-fix by
/// `tests/algo_parser.rs::for_body_error_surfaces_single_primary_after_keyword_sync`
/// (algo, TASK-0207 → TASK-0199) and the sched sibling
/// `tests/sched_parser.rs::nested_brace_body_error_surfaces_single_primary_after_keyword_sync_*`
/// (sched, TASK-0087 cycle-4 → TASK-0199).
fn program_parser() -> impl Parser<char, Vec<SpItem>, Error = Simple<char>> {
    item_parser()
        .map(Some)
        .padded_by(comment_or_ws())
        .recover_with(skip_parser(brace_balanced_recovery().map(|_| None)))
        .repeated()
        .flatten()
        .then_ignore(end())
}

/// Brace-balanced recovery parser (TASK-0199).
///
/// Replaces the historical `;`-only `skip_until` sync set, which had a
/// genuine recovery defect when a syntactic error fell inside a
/// brace-delimited body (algo `for { … }`, sched `worker_class { … }` /
/// `memory_region { … }`): the OUTER recovery consumed the typo's
/// inner `;` and landed mid-body, producing follow-on errors (constant
/// `2` on the algo side, `n + 2` linear cascade on the sched side).
/// Post-fix pins live at
/// `tests/algo_parser.rs::for_body_error_surfaces_single_primary_after_keyword_sync`
/// (renamed from `_constant_two_parametric`, TASK-0207) and
/// `tests/sched_parser.rs::nested_brace_body_error_surfaces_single_primary_after_keyword_sync_*`
/// (renamed from `_n_plus_two_parametric_*` and
/// `_bounded_follow_ons_*`, TASK-0087 cycle-4).
///
/// # Mechanism
///
/// On a top-level item failure, recovery consumes ONE "logical item
/// span" — a stretch of source that is either:
/// - a single `;` (degenerate item: the parser was sitting on a stray
///   `;` with no preceding content; consume it and let `.repeated()`
///   try the next item), or
/// - one or more "outer atoms" optionally followed by a terminating
///   `;`. An outer atom is either:
///     - a fully-balanced `{ … }` block (recursively skipped, inner
///       `;` consumed transparently as nested-content); or
///     - any single character that is NOT `{`, `}`, or `;` at the
///       outer depth.
///
/// At the outer depth the loop stops at the first `;` (consumed as
/// the item terminator) OR at the first `}` (NOT consumed — left for
/// the enclosing block parser; this is the sched-block-close path).
///
/// # Effect on the historical cascade shapes
///
/// - Algo `for { stmt; stmt; … }`: when a stmt inside the body fails,
///   recovery skips through `for i : 0 .. N` (safe chars), then
///   recursively-consumes the entire `{ … }` body (inner `;` are
///   safely consumed as nested-content, brace nesting respected),
///   leaving the stream at the position after `}`. The outer loop
///   resumes cleanly with no residual follow-on, collapsing the
///   pre-fix `constant 2` (primary + structural close-`}`
///   `Unexpected`) to `EXACTLY 1` (primary only).
///
/// - Sched `worker_class IDENT { field; field; … };` and
///   `memory_region IDENT { field; field; … };`: same trail — safe
///   chars through `worker_class IDENT`, brace-balanced consumption
///   of `{ … }`, then `;.or_not()` consumes the directive terminator.
///   The pre-fix `n + 2` linear cascade — caused by each residual
///   `field;` line re-failing the directive parser and re-triggering
///   recovery — disappears: the entire directive (body + terminator)
///   is consumed in one recovery step. Collapses to `EXACTLY 1`.
///
/// # Multi-error preservation
///
/// For genuinely-independent errors in DIFFERENT items, each item
/// fails on its own and triggers its own recovery; the recovery
/// consumes only THAT item's span (up to its terminating `;` or
/// brace-block), leaving subsequent items intact. So N independent
/// errors still produce N errors (verified by the existing
/// `multi_error_two_independent_errors_both_reported` fixtures in
/// both `tests/algo_parser.rs` and `tests/sched_parser.rs`).
///
/// # Boundedness + determinism
///
/// Each invocation of the recovery parser consumes ≥1 character or
/// fails (the `lone_semicolon` branch consumes exactly the `;`; the
/// `normal` branch requires at least one initial atom). At EOF the
/// recovery fails non-consumingly so the outer `.repeated()` stops
/// cleanly (no panic — see chumsky's `Repeated::parse_inner`
/// no-progress contract). All combinators used are pure/positional
/// (`recursive`, `choice`, `just`, `none_of`, `or_not`), no
/// hash-iteration, so the resulting error set+order is a
/// deterministic function of the source — verified by the
/// `recovery_resumes_and_is_deterministic` and `pathological_input_*`
/// fixtures.
fn brace_balanced_recovery() -> impl Parser<char, (), Error = Simple<char>> + Clone {
    // Inner balanced content (used inside a `{ … }` block). Allows `;`
    // as a regular nested character; the only structural concerns at
    // this level are the matching `{` (recurse) and `}` (terminate
    // the enclosing block).
    let inner_balanced = recursive(
        |inner: chumsky::recursive::Recursive<char, (), Simple<char>>| {
            let brace_block = just('{')
                .ignore_then(inner.clone())
                .then_ignore(just('}'))
                .ignored();
            let any_non_brace = none_of(['{', '}']).ignored();
            choice((brace_block, any_non_brace)).repeated().ignored()
        },
    );

    // Outer atom: at depth 0, `;` is the item terminator (NOT consumed
    // here — handled by the trailing `or_not`) and `}` is the enclosing
    // block's close (NOT consumed — left for the outer parser).
    let brace_block_outer = just('{')
        .ignore_then(inner_balanced.clone())
        .then_ignore(just('}'))
        .ignored();
    let outer_safe_char = none_of(['{', '}', ';']).ignored();

    // Degenerate case: the parser is sitting on a bare `;` (e.g. the
    // pathological-input fixture `@@@ ;; ??? ;;`). Consume just the
    // `;` so each stray sync token surfaces its own error and the loop
    // makes progress.
    let lone_semicolon = just(';').ignored();

    // Brace-bodied item case (TASK-0199 cycle-2 review-gate
    // correction): a balanced `{ … }` block IS a complete item span by
    // itself (e.g. an algo `for { … }` with no trailing `;`). Consume
    // exactly ONE brace block + an optional trailing `;`, then STOP.
    // Critically: do NOT allow further outer atoms after the brace
    // block — that was the over-consumption defect QA caught (a
    // failing `for { … }` followed by `const OK : usize = 7;` would
    // greedily consume the OK const's text up to its `;`, silently
    // swallowing it from both errors and the AST). The brace block
    // alone is ≥2 chars (`{` + `}`), so progress is guaranteed.
    let brace_block_item = brace_block_outer.then_ignore(just(';').or_not()).ignored();

    // Flat-item case: one-or-more outer safe chars (NOT including `{`,
    // `}`, `;`) terminated by EITHER `;` OR end-of-input. The `;` arm
    // is the pre-TASK-0199 `skip_until([';'])` behavior for
    // `;`-terminated items like a failing `const X = @;`. The
    // `end()` arm tolerates a malformed item at the very end of the
    // source (e.g. a single stray `?` with no following `;` — the
    // `parser_error_carries_line_and_column` fixture). Critically:
    // the terminator is REQUIRED (not `or_not`) so the safe-char
    // sequence has a bounded extent — prevents the over-consumption
    // QA Probe 6/6a defect (a failing `for { … }` followed by a valid
    // `const OK : usize = 7;` would otherwise greedily consume the
    // OK const's text up to its `;`, silently swallowing it).
    let flat_item = outer_safe_char
        .clone()
        .then(outer_safe_char.repeated())
        .then_ignore(just(';').ignored().or(end()))
        .ignored();

    choice((lone_semicolon, brace_block_item, flat_item))
}

/// Expression parser — covers `IndexExpr` and `ConstExpr` (grammar §1
/// merges them at parse time).
fn expr_parser() -> impl Parser<char, SpExpr, Error = Simple<char>> + Clone {
    // Recurses on `SpExpr`; every alternative ends in
    // `.map_with_span(Spanned::new)` so the span of a composed node is
    // the byte range covering the whole sub-expression (operator and
    // both operands for a binary, the `-` and operand for a unary,
    // etc.). Atoms span just their token; the parenthesised form spans
    // the inner expression (the parens are structural).
    recursive(
        |expr: chumsky::recursive::Recursive<char, SpExpr, Simple<char>>| {
            // Atom: int | ident-or-call | parenthesised expr. The int
            // span is the digits only (trailing layout excluded).
            let int_atom = padded_spanned(int_lit().map(Expr::IntLit));

            // The parenthesised expression keeps the *inner* expression's
            // span (already populated); the surrounding parens carry no
            // independent diagnostic meaning.
            let paren = pad(just('('))
                .ignore_then(expr.clone())
                .then_ignore(pad(just(')')));

            // After an identifier we may see either a call `(args)` or a
            // sequence of index suffixes `[i][j]...`. The grammar makes
            // these mutually exclusive: only an LValue can be indexed, a
            // CallExpr cannot. We model that here with a single choice on
            // the tail.
            // Each tail also reports the byte offset just past its closing
            // delimiter so the composed atom span ends tightly at `)` /
            // `]` (or at the identifier itself for a bare ident), never
            // including trailing layout.
            let call_tail = just('(')
                .ignore_then(comment_or_ws())
                .ignore_then(expr.clone().separated_by(pad(just(','))).allow_trailing())
                .then(just(')').map_with_span(|_, s: std::ops::Range<usize>| s.end))
                .then_ignore(comment_or_ws())
                .map(|(args, end)| (IdentTail::Call(args), Some(end)));

            let index_tail = just('[')
                .ignore_then(comment_or_ws())
                .ignore_then(expr.clone())
                .then(just(']').map_with_span(|_, s: std::ops::Range<usize>| s.end))
                .then_ignore(comment_or_ws())
                .repeated()
                .map(|pairs| {
                    let end = pairs.last().map(|(_, e)| *e);
                    let indices = pairs.into_iter().map(|(i, _)| i).collect();
                    (IdentTail::Indices(indices), end)
                });

            // `name` is already a tightly-spanned `SpIdent`. Behaviour
            // preserved exactly from before spans: `index_tail` is
            // `.repeated()` so it always succeeds (possibly empty) — a
            // bare identifier still lowers as
            // `Expr::LValue(IndexedLValue{ indices: [] })`, NOT
            // `Expr::Ident`, so lowering's existing bare-ident handling
            // (lower.rs `Expr::LValue` empty-indices arms) is unchanged.
            // The atom span is `name.start .. (close-delim end | name end)`
            // — tight, no trailing layout.
            let ident_or_call =
                pad(ident())
                    .then(call_tail.or(index_tail))
                    .map(|(name, (tail, tail_end))| {
                        let span = name.span.start..tail_end.unwrap_or(name.span.end);
                        let node = match tail {
                            IdentTail::Call(args) => Expr::Call(Call { callee: name, args }),
                            IdentTail::Indices(indices) => {
                                Expr::LValue(IndexedLValue { name, indices })
                            }
                        };
                        Spanned::new(node, span)
                    });

            let atom = choice((int_atom, paren, ident_or_call));

            // Unary `-`. `foldr` wraps right-to-left; each added `-`
            // re-spans to cover that `-` plus everything to its right.
            let unary = pad(just('-'))
                .map_with_span(|_, span: std::ops::Range<usize>| span)
                .repeated()
                .then(atom)
                .foldr(|minus_span, rhs: SpExpr| {
                    let span = minus_span.start..rhs.span.end;
                    Spanned::new(Expr::Unary(UnaryOp::Neg, Box::new(rhs)), span)
                });

            // Multiplicative. `foldl` re-spans each composed binary to
            // cover lhs.start..rhs.end.
            let mul_op = choice((
                pad(just('*')).to(BinOp::Mul),
                pad(just('/')).to(BinOp::Div),
                pad(just('%')).to(BinOp::Mod),
            ));
            let mul = unary.clone().then(mul_op.then(unary).repeated()).foldl(
                |lhs: SpExpr, (op, rhs): (BinOp, SpExpr)| {
                    let span = lhs.span.start..rhs.span.end;
                    Spanned::new(Expr::Binary(op, Box::new(lhs), Box::new(rhs)), span)
                },
            );

            // Additive.
            let add_op = choice((pad(just('+')).to(BinOp::Add), pad(just('-')).to(BinOp::Sub)));
            mul.clone().then(add_op.then(mul).repeated()).foldl(
                |lhs: SpExpr, (op, rhs): (BinOp, SpExpr)| {
                    let span = lhs.span.start..rhs.span.end;
                    Spanned::new(Expr::Binary(op, Box::new(lhs), Box::new(rhs)), span)
                },
            )
        },
    )
}

/// `DataType ::= ScalarType DimList?`.
fn data_type_parser() -> impl Parser<char, Type, Error = Simple<char>> + Clone {
    let dim = pad(just('['))
        .ignore_then(expr_parser())
        .then_ignore(pad(just(']')));
    pad(scalar_type())
        .then(dim.repeated())
        .map(|(scalar, dims)| Type { scalar, dims })
}

/// `ConstDecl ::= 'const' Ident ':' ScalarType '=' ConstExpr ';'`.
fn const_decl_parser() -> impl Parser<char, ConstDecl, Error = Simple<char>> + Clone {
    pad(keyword("const"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just(':')))
        .then(pad(scalar_type()))
        .then_ignore(pad(just('=')))
        .then(expr_parser())
        .then_ignore(just(';'))
        .map(|((name, ty), value)| ConstDecl { name, ty, value })
}

/// `DataDecl ::= 'data' Ident ':' DataType ';'`.
fn data_decl_parser() -> impl Parser<char, DataDecl, Error = Simple<char>> + Clone {
    pad(keyword("data"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just(':')))
        .then(data_type_parser())
        .then_ignore(just(';'))
        .map(|(name, ty)| DataDecl { name, ty })
}

/// `KernelDecl ::= 'kernel' Ident ':' KernelSig Purity ';'`.
fn kernel_decl_parser() -> impl Parser<char, KernelDecl, Error = Simple<char>> + Clone {
    let unit = pad(just('(')).then(pad(just(')'))).to(None::<Type>);
    let typed_ret = data_type_parser().map(Some);
    let ret = choice((unit, typed_ret));

    let params = data_type_parser()
        .separated_by(pad(just(',')))
        .allow_trailing();

    let sig = pad(just('('))
        .ignore_then(params)
        .then_ignore(pad(just(')')))
        .then_ignore(pad(just('-')).then(pad(just('>'))))
        .then(ret)
        .map(|(params, ret)| KernelSig { params, ret });

    let purity = choice((
        pad(keyword("pure")).to(Purity::Pure),
        pad(keyword("effectful")).to(Purity::Effectful),
    ));

    pad(keyword("kernel"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just(':')))
        .then(sig)
        .then(purity)
        .then_ignore(just(';'))
        .map(|((name, sig), purity)| KernelDecl { name, sig, purity })
}

/// Indexed LValue `IDENT ('[' EXPR ']')*`.
fn lvalue_parser() -> impl Parser<char, IndexedLValue, Error = Simple<char>> + Clone {
    pad(ident())
        .then(
            pad(just('['))
                .ignore_then(expr_parser())
                .then_ignore(pad(just(']')))
                .repeated(),
        )
        .map(|(name, indices)| IndexedLValue { name, indices })
}

/// Hint probe: detect a schedule-directive shape in algorithm
/// statement position and emit a tailored
/// `<kw> is a schedule directive — did you mean to put it in a
/// *.sched.nuc file?` (TASK-0083, promised by `docs/grammar-algo.md`
/// §3). Returns a never-succeeding parser (it always fires `Err` on
/// match); placed FIRST in [`stmt_parser`]'s `choice` so the
/// further-advanced custom error supersedes the generic "unexpected
/// `=`" follow-on from the dataflow/bare-call branches.
///
/// # Shapes detected
///
/// - `<kw> =` for `<kw>` in [`SCHED_RESERVED_EQ`]
///   (`block`, `buffer`, `notify`, `partition`, `pipeline`,
///   `transfer`, `unroll`) — the `kw = N` directive shape. (Was
///   also `vectorize` pre-2026-05-25; TASK-0292 dropped it.)
/// - `place <ident>` — the `place IDENT on host;` statement directive.
/// - `place_data <ident>` — the `place_data IDENT to MEM;` shape.
/// - `check loop` — the `check loop VAR : ASSERT;` shape.
///
/// # Disambiguation against valid algorithm uses
///
/// `block`/`place`/etc. are NOT in algorithm [`KEYWORDS`](super::tokens::KEYWORDS), so they
/// remain legal as plain identifiers (e.g.
/// `data block : f32[16]; block <-- foo();`). Detection therefore
/// requires the FULL shape — `<kw>` alone does not fire; a `<kw>`
/// followed by `=`/ident/`loop` is needed. Each arm consumes the
/// whole shape (keyword + follow-on token) before firing, so the
/// custom error's reported position is strictly past where the
/// fallback `dataflow`/`bare_call` branches fail — chumsky's
/// furthest-position error-merge rule selects this hint over the
/// generic "expected `<--`/`(`" follow-on.
///
/// The error span anchors at the **schedule keyword** itself (not at
/// the `=`) so the diagnostic underlines the offending word, matching
/// the grammar doc's example.
fn sched_directive_hint_stmt() -> impl Parser<char, Stmt, Error = Simple<char>> + Clone {
    // `<kw> =` shape. Build one alternative per keyword so each
    // consumes a tight, distinct prefix; `choice` then picks the
    // first that matches. We capture the keyword's span via
    // `map_with_span` BEFORE the trailing `=` is consumed, so the
    // diagnostic underlines just the offending keyword (matches the
    // §3 example wording).
    let eq_shape = {
        let alts: Vec<_> = SCHED_RESERVED_EQ
            .iter()
            .map(|kw| {
                pad(keyword(kw))
                    .map_with_span(move |(), span| (*kw, span))
                    .then_ignore(just('='))
                    .boxed()
            })
            .collect();
        choice(alts).try_map(|(kw, span), outer_span| {
            // Span COVERS the keyword + the trailing `=` so the
            // merged error position is past where `dataflow` (expects
            // `<--`) and `bare_call` (expects `(`) fail — see
            // `place_data_shape` for the chumsky furthest-end-position
            // merge rule that makes this necessary.
            Err(Simple::custom(
                span.start..outer_span.end,
                sched_hint_msg(kw),
            ))
        })
    };

    // `place_data <ident>` MUST be tried before `place <ident>`
    // because `place_data` has `place` as a prefix at the keyword
    // level — the `keyword` helper's trailing alnum/_ rewind correctly
    // distinguishes them, but probe order makes the intent explicit.
    //
    // We CONSUME the follow-on ident (rather than just peeking it via
    // `rewind`) so the custom error's span ends past where the
    // fallback `bare_call` branch fails — chumsky 0.9 merges
    // simultaneous errors by furthest end-position, and a `rewind`
    // would leave our hint error anchored at `place` (≈9-byte span)
    // while `bare_call`'s "expected `(`" fails at the start of the
    // ident (further). Consuming the ident pushes the hint span end
    // past `bare_call`'s failure point so the hint actually wins.
    // The error MESSAGE still names the schedule keyword, so the
    // user-visible diagnostic still reads "`place` is a schedule
    // directive — did you mean to put it in a *.sched.nuc file?".
    let place_data_shape = pad(keyword("place_data"))
        .map_with_span(|(), span| ("place_data", span))
        .then(ident_chars())
        .try_map(|((kw, span), _ident), outer_span| {
            Err(Simple::custom(
                span.start..outer_span.end,
                sched_hint_msg(kw),
            ))
        });

    let place_shape = pad(keyword("place"))
        .map_with_span(|(), span| ("place", span))
        .then(ident_chars())
        .try_map(|((kw, span), _ident), outer_span| {
            Err(Simple::custom(
                span.start..outer_span.end,
                sched_hint_msg(kw),
            ))
        });

    let check_loop_shape = pad(keyword("check"))
        .map_with_span(|(), span| ("check", span))
        .then_ignore(pad(keyword("loop")))
        .try_map(|(kw, span), outer_span| {
            Err(Simple::custom(
                span.start..outer_span.end,
                sched_hint_msg(kw),
            ))
        });

    // Order: `place_data` BEFORE `place` (prefix-rule disambiguation
    // even though `keyword` already enforces a non-alnum boundary —
    // belt and braces); `check_loop_shape` last because it's the only
    // one whose lookahead is another full keyword.
    choice((
        eq_shape.boxed(),
        place_data_shape.boxed(),
        place_shape.boxed(),
        check_loop_shape.boxed(),
    ))
    // The custom-error parsers above NEVER produce an `Ok` value, but
    // chumsky needs a concrete `Output` for `choice` to type-check.
    // We declare it as `Stmt` so the parser composes into
    // [`stmt_parser`]'s `choice` directly; on the (unreachable) `Ok`
    // path we'd materialise a placeholder `Stmt::Effect` with an
    // impossible call, but `try_map` always returns `Err`, so this is
    // a phantom branch in practice — kept only to satisfy the type.
    .map(|_: ()| {
        Stmt::Effect(Call {
            callee: Spanned::new("__unreachable_sched_hint__".to_string(), 0..0),
            args: vec![],
        })
    })
}

/// Build the hint message for a schedule-directive keyword. Kept as
/// a separate `fn` so the wording lives in one place and so any
/// future re-skinning (e.g. adding the surrounding source line) is a
/// single edit.
fn sched_hint_msg(kw: &str) -> String {
    format!("`{kw}` is a schedule directive — did you mean to put it in a `*.sched.nuc` file?")
}

/// Statements (dataflow / effect / for).
fn stmt_parser() -> impl Parser<char, SpStmt, Error = Simple<char>> + Clone {
    recursive(
        |stmt: chumsky::recursive::Recursive<char, SpStmt, Simple<char>>| {
            // Dataflow vs effect both start with an ident. We disambiguate
            // by trying dataflow (which needs `<--` after the LValue) and
            // falling back to a bare call statement.
            // Each alternative ends at its *bare* terminator (`;` / `}`)
            // — NOT `pad(just(..))` — so the `.map_with_span` below fixes
            // the statement span tight at the terminator; trailing layout
            // is consumed afterwards (`then_ignore(comment_or_ws())`),
            // outside the span. Without this the span would swallow the
            // newline after `;`, mislocating a statement-level diagnostic.
            let dataflow = lvalue_parser()
                .then_ignore(pad(just('<')).then(pad(just('-'))).then(pad(just('-'))))
                .then(expr_parser())
                .then_ignore(just(';'))
                .map(|(lhs, rhs)| Stmt::Dataflow { lhs, rhs });

            // Bare call statement: ident '(' args ')' ';'
            let bare_call = pad(ident())
                .then_ignore(pad(just('(')))
                .then(
                    expr_parser()
                        .separated_by(pad(just(',')))
                        .allow_trailing()
                        .or_not()
                        .map(|a| a.unwrap_or_default()),
                )
                .then_ignore(pad(just(')')))
                .then_ignore(just(';'))
                .map(|(callee, args)| Stmt::Effect(Call { callee, args }));

            let for_stmt = pad(keyword("for"))
                .ignore_then(pad(for_loop_var()))
                .then_ignore(pad(just(':')))
                .then(expr_parser())
                .then_ignore(pad(just('.')).then(pad(just('.'))))
                .then(expr_parser())
                .then_ignore(pad(just('{')))
                .then(stmt.clone().repeated())
                .then_ignore(just('}'))
                .map(|(((var, lo), hi), body)| Stmt::For { var, lo, hi, body });

            // Order: schedule-directive hint first (TASK-0083) — it
            // probes for the unambiguous `<sched_kw> =` / `place IDENT`
            // / `place_data IDENT` / `check loop` shapes and fires a
            // tailored hint via `Simple::custom`; on no-match it
            // consumes nothing and falls through to the real algorithm
            // grammar. Then `for_stmt` (distinct keyword), then dataflow
            // (uses `<--`), then bare-call as fallback. Span fixed at the
            // bare terminator, then trailing layout consumed off-span.
            let hint = sched_directive_hint_stmt();
            choice((hint, for_stmt, dataflow, bare_call))
                .map_with_span(Spanned::new)
                .then_ignore(comment_or_ws())
        },
    )
}

/// Top-level item.
fn item_parser() -> impl Parser<char, SpItem, Error = Simple<char>> + Clone {
    // `stmt_parser` already yields a `SpStmt`; unwrap its node into
    // `Item::Stmt` and re-span at the item level (same source range —
    // a top-level statement *is* its item). The decl arms span the
    // whole declaration.
    // Decl parsers end at the bare `;` (no trailing pad), so
    // `.map_with_span` fixes the item span tight at the terminator;
    // trailing layout is then consumed off-span. `stmt_parser` already
    // yields a tightly-spanned `SpStmt` (and ate its own trailing
    // layout); a top-level statement *is* its item, so the item span
    // is exactly the statement span.
    choice((
        const_decl_parser()
            .map(Item::Const)
            .map_with_span(Spanned::new)
            .then_ignore(comment_or_ws()),
        data_decl_parser()
            .map(Item::Data)
            .map_with_span(Spanned::new)
            .then_ignore(comment_or_ws()),
        kernel_decl_parser()
            .map(Item::Kernel)
            .map_with_span(Spanned::new)
            .then_ignore(comment_or_ws()),
        stmt_parser().map(|s| {
            let span = s.span.clone();
            Spanned::new(Item::Stmt(s), span)
        }),
    ))
}
