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
//! # Known limitations
//!
//! - Recovery sync token is `;` only. A malformed `for { … }` whose
//!   body contains `;`-terminated statements causes the OUTER
//!   program-level `skip_until([';'])` recovery to consume the
//!   typo's `;` and land mid-body. **Measured** (deterministic,
//!   parametric over the number `n` of valid trailing for-body
//!   statements after a single primary `@`-typo, `n ∈ {0,1,2,5}` and
//!   out-of-fixture `{3,8,12}`): the total error count is the
//!   **constant `2`** — primary + structural close-`}` `Unexpected`
//!   follow-on — INDEPENDENT of `n`. This is strictly *better* than
//!   the sched sibling's `n+2` linear cascade
//!   (`sched/parser.rs::nested_brace_body_error_surfaces_n_plus_two_*`),
//!   for a STRUCTURAL reason verified by reading the parsers (NOT a
//!   recovery-layer-shape claim — neither parser has inner
//!   field/stmt-level `recover_with`). The divergence is in what
//!   the OUTER grammar accepts for the residue after recovery:
//!   (a) algo's top-level grammar accepts `Stmt` items, so each
//!   residual `x[i] <-- inc(i);` line parses cleanly as a top-level
//!   Item — zero re-failures, hence no cascade; (b) sched's
//!   directive grammar accepts only directive-keyword-led items, so
//!   each residual field-keyword line re-fails directive parsing
//!   and re-triggers the directive-level `recover_with` — one
//!   cascade per residual `;`. Pinned parametrically by
//!   `compiler/tests/algo_parser.rs::for_body_error_surfaces_constant_two_parametric`
//!   (TASK-0207). Boundedness and determinism — the invariants that
//!   matter — both hold. A keyword-anchored sync set is a possible
//!   future refinement (TASK-0199); when it lands, the algo count
//!   collapses from `2` to `1` (primary only) — same mechanical
//!   edit as the sched fixtures (replace the `expected = 2` literal
//!   with `1`).
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
    Purity, ScalarType, SpExpr, SpIdent, SpItem, SpStmt, Stmt, Type, UnaryOp,
};
use crate::span::Spanned;
use crate::error::map_all_chumsky_errors;
pub use crate::error::{ParseError, ParseErrorKind, ParseErrors};

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

/// Reserved words — identifiers may not collide with these. Listed
/// explicitly per grammar §6 note 4.
const KEYWORDS: &[&str] = &[
    "const",
    "data",
    "kernel",
    "pure",
    "effectful",
    "for",
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

/// Builds the top-level `Program ::= TopItem*` parser.
///
/// Wrapped in a function so callers don't depend on chumsky's exact
/// return type.
///
/// # Error recovery (TASK-0081)
///
/// The per-item parser is lifted to `Option<SpItem>` (`Some` on a
/// clean parse) and given
/// `recover_with(skip_until([';'], |_| None).consume_end())`. On a
/// syntactic failure anywhere inside one top-level item, chumsky:
///   1. records the error (collected by `parse_recovery`),
///   2. skips input characters until it finds a `;` — the universal
///      statement / declaration terminator (`const`/`data`/`kernel`/
///      dataflow/bare-call all end in `;`),
///   3. **consumes** that `;` (`consume_end`) so the next iteration
///      starts at the following item, not re-failing on the same `;`,
///   4. recovers this element to the value `None`.
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
/// Boundedness: each `skip_until` recovery consumes ≥1 character or
/// reaches end-of-input, where chumsky's strategy terminates (the
/// `Err(_) if stream.save() > pre_state` / `Err(...)` arms in
/// `SkipUntil::recover`) — no infinite retry, and `.repeated()` makes
/// finite progress every iteration. Determinism: the sync set is a
/// single fixed concrete token, chumsky's error list is positional,
/// and the message is rebuilt with a sorted expected-set
/// ([`crate::error`]) — no hashing / iteration-order dependence on the
/// whole path.
///
/// Sync token is `;` only — see the module-doc "Known limitations"
/// note on the algo `for { … }` body's measured **constant-2** error
/// shape (primary + structural close-`}` follow-on, INDEPENDENT of
/// the number of valid trailing body statements). This is the
/// load-bearing structural difference from the sched `worker_class` /
/// `memory_region` brace-body case, which scales as `n+2`. Pinned by
/// `tests/algo_parser.rs::for_body_error_surfaces_constant_two_parametric`
/// (TASK-0207); TASK-0199 will collapse both to `1`.
fn program_parser() -> impl Parser<char, Vec<SpItem>, Error = Simple<char>> {
    item_parser()
        .map(Some)
        .padded_by(comment_or_ws())
        .recover_with(skip_until([';'], |_| None).consume_end())
        .repeated()
        .flatten()
        .then_ignore(end())
}

/// Whitespace + line comments. Grammar §1 lexical rules.
fn comment_or_ws() -> impl Parser<char, (), Error = Simple<char>> + Clone {
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
fn padded_spanned<P, T>(p: P) -> impl Parser<char, Spanned<T>, Error = Simple<char>> + Clone
where
    P: Parser<char, T, Error = Simple<char>> + Clone,
{
    p.map_with_span(Spanned::new).then_ignore(comment_or_ws())
}

/// Helper: token followed by trailing whitespace/comments.
fn pad<P, T>(p: P) -> impl Parser<char, T, Error = Simple<char>> + Clone
where
    P: Parser<char, T, Error = Simple<char>> + Clone,
{
    p.then_ignore(comment_or_ws())
}

/// Identifier matcher. Rejects keywords. Yields a [`SpIdent`] whose
/// span is exactly the identifier token's byte range (no surrounding
/// whitespace — `pad` consumes trailing space *after* this combinator),
/// so an "undeclared / duplicate `X`" diagnostic underlines just `X`.
fn ident() -> impl Parser<char, SpIdent, Error = Simple<char>> + Clone {
    let start = filter(|c: &char| c.is_ascii_alphabetic() || *c == '_');
    let cont = filter(|c: &char| c.is_ascii_alphanumeric() || *c == '_');
    start
        .chain(cont.repeated())
        .collect::<String>()
        .try_map(|s, span| {
            if KEYWORDS.contains(&s.as_str()) {
                Err(Simple::custom(
                    span,
                    format!("expected identifier, found keyword `{}`", s),
                ))
            } else {
                Ok(s)
            }
        })
        .map_with_span(Spanned::new)
}

/// Integer literal — decimal only (grammar §1).
fn int_lit() -> impl Parser<char, i64, Error = Simple<char>> + Clone {
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
fn scalar_type() -> impl Parser<char, ScalarType, Error = Simple<char>> + Clone {
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

/// Expression parser — covers `IndexExpr` and `ConstExpr` (grammar §1
/// merges them at parse time).
fn expr_parser() -> impl Parser<char, SpExpr, Error = Simple<char>> + Clone {
    // Recurses on `SpExpr`; every alternative ends in
    // `.map_with_span(Spanned::new)` so the span of a composed node is
    // the byte range covering the whole sub-expression (operator and
    // both operands for a binary, the `-` and operand for a unary,
    // etc.). Atoms span just their token; the parenthesised form spans
    // the inner expression (the parens are structural).
    recursive(|expr: chumsky::recursive::Recursive<char, SpExpr, Simple<char>>| {
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
        let ident_or_call = pad(ident())
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
        let mul = unary
            .clone()
            .then(mul_op.then(unary).repeated())
            .foldl(|lhs: SpExpr, (op, rhs): (BinOp, SpExpr)| {
                let span = lhs.span.start..rhs.span.end;
                Spanned::new(Expr::Binary(op, Box::new(lhs), Box::new(rhs)), span)
            });

        // Additive.
        let add_op = choice((pad(just('+')).to(BinOp::Add), pad(just('-')).to(BinOp::Sub)));
        mul.clone()
            .then(add_op.then(mul).repeated())
            .foldl(|lhs: SpExpr, (op, rhs): (BinOp, SpExpr)| {
                let span = lhs.span.start..rhs.span.end;
                Spanned::new(Expr::Binary(op, Box::new(lhs), Box::new(rhs)), span)
            })
    })
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

/// A reserved-word matcher that ensures the keyword is not the prefix
/// of a longer identifier (e.g. `for_each` does not start `for`).
fn keyword(kw: &'static str) -> impl Parser<char, (), Error = Simple<char>> + Clone {
    // See `scalar_type` for the `rewind()` rationale: ensure the
    // keyword is not the prefix of a longer identifier without
    // consuming the lookahead character.
    just(kw)
        .then_ignore(
            none_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_").rewind(),
        )
        .ignored()
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

/// Statements (dataflow / effect / for).
fn stmt_parser() -> impl Parser<char, SpStmt, Error = Simple<char>> + Clone {
    recursive(|stmt: chumsky::recursive::Recursive<char, SpStmt, Simple<char>>| {
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
            .ignore_then(pad(ident()))
            .then_ignore(pad(just(':')))
            .then(expr_parser())
            .then_ignore(pad(just('.')).then(pad(just('.'))))
            .then(expr_parser())
            .then_ignore(pad(just('{')))
            .then(stmt.clone().repeated())
            .then_ignore(just('}'))
            .map(|(((var, lo), hi), body)| Stmt::For { var, lo, hi, body });

        // Order: `for_stmt` first (distinct keyword), then dataflow
        // (uses `<--`), then bare-call as fallback. Span fixed at the
        // bare terminator, then trailing layout consumed off-span.
        choice((for_stmt, dataflow, bare_call))
            .map_with_span(Spanned::new)
            .then_ignore(comment_or_ws())
    })
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
