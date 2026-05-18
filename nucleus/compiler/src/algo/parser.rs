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
//! - Mature error-recovery primitives (we use minimal recovery for now).
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
//! # Known limitations (mirrored in `algo/mod.rs` and TASK-0007 notes)
//!
//! - Only the first parse error is reported. Multi-error reporting is
//!   a follow-up task.
//! - Minimal error recovery — the parser bails on the first syntactic
//!   failure rather than skipping to the next plausible statement.
//! - AST nodes do not carry spans yet; only `ParseError` does. Adding
//!   per-node spans is a follow-up task.
//! - Semantic constraints (single-assignment, kernel-purity-vs-effect-
//!   statement, forward references) are deliberately NOT enforced here
//!   — they belong to TASK-0009 (AlgoIR lowering).

use chumsky::prelude::*;

use super::ast::{
    AlgoAst, BinOp, Call, ConstDecl, DataDecl, Expr, IndexedLValue, Item, KernelDecl, KernelSig,
    Purity, ScalarType, Stmt, Type, UnaryOp,
};

/// Internal: the tail of an identifier-led atom — either a call's
/// argument list or zero-or-more index suffixes. Kept local because it
/// has no meaning outside the expression parser.
enum IdentTail {
    Call(Vec<Expr>),
    Indices(Vec<Expr>),
}

/// A parse error with `(line, column)` source location.
///
/// Only the first error in the source is reported; see module-level
/// limitations. The `kind` distinguishes the broad failure category so
/// tests can match on a variant without scraping a message string.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub kind: ParseErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// Unexpected token / character. The default for combinator
    /// failures that don't map to a more specific variant.
    Unexpected,
    /// Unexpected end of input mid-construct.
    UnexpectedEof,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "parse error at line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Parse a `*.algo.nuc` source string into an [`AlgoAst`].
///
/// Errors carry `(line, column)` (1-based) of the first failure. The
/// parser does not recover; multiple-error reporting is a follow-up.
pub fn parse_algo(src: &str) -> Result<AlgoAst, ParseError> {
    let parser = program_parser();
    match parser.parse(src) {
        Ok(items) => Ok(AlgoAst { items }),
        Err(errors) => Err(map_first_error(src, errors)),
    }
}

fn map_first_error(src: &str, errors: Vec<Simple<char>>) -> ParseError {
    // Take the first error. `chumsky` may surface several alternatives
    // at the same position; we treat them as one logical failure.
    let err = errors.into_iter().next().expect("non-empty error list");
    let span = err.span();
    let offset = span.start.min(src.len());
    let (line, column) = offset_to_line_col(src, offset);

    let kind = match err.reason() {
        chumsky::error::SimpleReason::Unexpected if err.found().is_none() => {
            ParseErrorKind::UnexpectedEof
        }
        _ => ParseErrorKind::Unexpected,
    };

    // chumsky's default Display gives a serviceable message; we keep
    // it as-is to avoid hiding information. Callers can match `kind`
    // for behavioural checks.
    let message = err.to_string();

    ParseError {
        line,
        column,
        kind,
        message,
    }
}

/// 1-based `(line, column)` for a byte offset into `src`. UTF-8 safe
/// because the grammar restricts source to ASCII (grammar §6 #5), but
/// counting bytes is still correct for ASCII columns.
fn offset_to_line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in src.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
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
fn program_parser() -> impl Parser<char, Vec<Item>, Error = Simple<char>> {
    item_parser()
        .padded_by(comment_or_ws())
        .repeated()
        .then_ignore(end())
}

/// Whitespace + line comments. Grammar §1 lexical rules.
fn comment_or_ws() -> impl Parser<char, (), Error = Simple<char>> + Clone {
    let line_comment = just("//")
        .then(take_until(text::newline().or(end())))
        .ignored();
    line_comment.or(one_of(" \t\r\n").ignored()).repeated().ignored()
}

/// Helper: token followed by trailing whitespace/comments.
fn pad<P, T>(p: P) -> impl Parser<char, T, Error = Simple<char>> + Clone
where
    P: Parser<char, T, Error = Simple<char>> + Clone,
{
    p.then_ignore(comment_or_ws())
}

/// Identifier matcher. Rejects keywords.
fn ident() -> impl Parser<char, String, Error = Simple<char>> + Clone {
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
            .then_ignore(none_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_").rewind())
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
fn expr_parser() -> impl Parser<char, Expr, Error = Simple<char>> + Clone {
    recursive(|expr| {
        // Atom: int | ident-or-call | parenthesised expr
        let int_atom = pad(int_lit()).map(Expr::IntLit);

        let paren = pad(just('('))
            .ignore_then(expr.clone())
            .then_ignore(pad(just(')')));

        // After an identifier we may see either a call `(args)` or a
        // sequence of index suffixes `[i][j]...`. The grammar makes
        // these mutually exclusive: only an LValue can be indexed, a
        // CallExpr cannot. We model that here with a single choice on
        // the tail.
        let call_tail = pad(just('('))
            .ignore_then(
                expr.clone()
                    .separated_by(pad(just(',')))
                    .allow_trailing(),
            )
            .then_ignore(pad(just(')')))
            .map(IdentTail::Call);

        let index_tail = pad(just('['))
            .ignore_then(expr.clone())
            .then_ignore(pad(just(']')))
            .repeated()
            .map(IdentTail::Indices);

        let ident_or_call = pad(ident())
            .then(call_tail.or(index_tail))
            .map(|(name, tail)| match tail {
                IdentTail::Call(args) => Expr::Call(Call { callee: name, args }),
                IdentTail::Indices(indices) => Expr::LValue(IndexedLValue { name, indices }),
            });

        let atom = choice((int_atom, paren, ident_or_call));

        // Unary `-`.
        let unary = pad(just('-'))
            .repeated()
            .then(atom)
            .foldr(|_, rhs| Expr::Unary(UnaryOp::Neg, Box::new(rhs)));

        // Multiplicative.
        let mul_op = choice((
            pad(just('*')).to(BinOp::Mul),
            pad(just('/')).to(BinOp::Div),
            pad(just('%')).to(BinOp::Mod),
        ));
        let mul = unary
            .clone()
            .then(mul_op.then(unary).repeated())
            .foldl(|lhs, (op, rhs)| Expr::Binary(op, Box::new(lhs), Box::new(rhs)));

        // Additive.
        let add_op = choice((pad(just('+')).to(BinOp::Add), pad(just('-')).to(BinOp::Sub)));
        mul.clone()
            .then(add_op.then(mul).repeated())
            .foldl(|lhs, (op, rhs)| Expr::Binary(op, Box::new(lhs), Box::new(rhs)))
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
        .then_ignore(pad(just(';')))
        .map(|((name, ty), value)| ConstDecl { name, ty, value })
}

/// `DataDecl ::= 'data' Ident ':' DataType ';'`.
fn data_decl_parser() -> impl Parser<char, DataDecl, Error = Simple<char>> + Clone {
    pad(keyword("data"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just(':')))
        .then(data_type_parser())
        .then_ignore(pad(just(';')))
        .map(|(name, ty)| DataDecl { name, ty })
}

/// `KernelDecl ::= 'kernel' Ident ':' KernelSig Purity ';'`.
fn kernel_decl_parser() -> impl Parser<char, KernelDecl, Error = Simple<char>> + Clone {
    let unit = pad(just('('))
        .then(pad(just(')')))
        .to(None::<Type>);
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
        .then_ignore(pad(just(';')))
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
fn stmt_parser() -> impl Parser<char, Stmt, Error = Simple<char>> + Clone {
    recursive(|stmt| {
        // Dataflow vs effect both start with an ident. We disambiguate
        // by trying dataflow (which needs `<--` after the LValue) and
        // falling back to a bare call statement.
        let dataflow = lvalue_parser()
            .then_ignore(pad(just('<')).then(pad(just('-'))).then(pad(just('-'))))
            .then(expr_parser())
            .then_ignore(pad(just(';')))
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
            .then_ignore(pad(just(';')))
            .map(|(callee, args)| Stmt::Effect(Call { callee, args }));

        let for_stmt = pad(keyword("for"))
            .ignore_then(pad(ident()))
            .then_ignore(pad(just(':')))
            .then(expr_parser())
            .then_ignore(pad(just('.')).then(pad(just('.'))))
            .then(expr_parser())
            .then_ignore(pad(just('{')))
            .then(stmt.clone().repeated())
            .then_ignore(pad(just('}')))
            .map(|(((var, lo), hi), body)| Stmt::For { var, lo, hi, body });

        // Order: `for_stmt` first (distinct keyword), then dataflow
        // (uses `<--`), then bare-call as fallback.
        choice((for_stmt, dataflow, bare_call))
    })
}

/// Top-level item.
fn item_parser() -> impl Parser<char, Item, Error = Simple<char>> + Clone {
    choice((
        const_decl_parser().map(Item::Const),
        data_decl_parser().map(Item::Data),
        kernel_decl_parser().map(Item::Kernel),
        stmt_parser().map(Item::Stmt),
    ))
}
