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
//!
//! Per-node source spans ARE tracked, via [`span::Spanned`] (TASK-0082);
//! `parse_algo` populates byte ranges. Lowering still ignores them —
//! threading spans into `LowerError` is TASK-0090.

pub mod ast;
pub mod ir;
pub mod lower;
pub mod parser;
pub mod span;

pub use ast::{
    AlgoAst, BinOp, ConstDecl, DataDecl, Expr, IndexedLValue, Item, KernelDecl, KernelSig, Purity,
    ScalarType, Stmt, Type, UnaryOp,
};
pub use span::Spanned;
pub use ir::{
    AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, LowerError, ResolvedConst, ResolvedData,
    ResolvedKernel, ResolvedType,
};
pub use lower::lower_algo;
pub use parser::{parse_algo, ParseError};
