//! Algorithm sublanguage (`*.algo.nuc`).
//!
//! Implements the grammar specified in `docs/grammar-algo.md` and
//! `nuc-nucleus/PRD.md` §6.2. The parser is hand-written on top of the
//! `chumsky` combinator library; see `parser.rs` for the choice
//! rationale.
//!
//! Public surface:
//! - [`parse_algo`]: parse a source string into [`AlgoAst`].
//! - AST node types ([`AlgoAst`], [`Item`], [`ConstDecl`], [`DataDecl`],
//!   [`KernelDecl`], [`Stmt`], [`Expr`], [`Type`], [`Purity`], etc.).
//! - [`ParseError`]: failures carry `(line, column)` from the input.
//!
//! What this module does NOT do (deliberately — see PRD §6.2 and the
//! grammar doc §3):
//! - Resolve identifiers. Forward references and unbound names are a
//!   semantic-pass concern (TASK-0009).
//! - Type-check expressions or kernel signatures (TASK-0009).
//! - Enforce single-assignment (TASK-0009).
//! - Track AST node spans beyond the top-level error position. Adding
//!   per-node spans is a follow-up task — see TASK-0007 self-report.

pub mod ast;
pub mod parser;

pub use ast::{
    AlgoAst, BinOp, ConstDecl, DataDecl, Expr, IndexedLValue, Item, KernelDecl, KernelSig, Purity,
    ScalarType, Stmt, Type, UnaryOp,
};
pub use parser::{parse_algo, ParseError};
