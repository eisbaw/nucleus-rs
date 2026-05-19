//! AST types for the algorithm sublanguage.
//!
//! Shape and naming follow `docs/grammar-algo.md` 1:1 — one Rust type
//! per nonterminal where that aids readability, collapsed where the
//! grammar splits something purely for parsing convenience (e.g.
//! `IndexExpr` and `ConstExpr` share the same shape, so both are
//! [`Expr`]).
//!
//! Per-node source spans ARE tracked (TASK-0082): the diagnosable
//! nodes are wrapped in [`Spanned<T>`][crate::algo::span::Spanned],
//! which carries the byte [`core::ops::Range`] the node was parsed
//! from. See `span.rs` for the exact granularity and rationale (which
//! nodes are wrapped and why the leaves are not). `parse_algo`
//! populates the ranges; lowering still ignores them (TASK-0090 wires
//! them into `LowerError`).
//!
//! Equality semantics: the inner-node `#[derive(PartialEq)]`s below
//! still make tests cheap, and `Spanned<T>`'s *manual* `PartialEq`
//! forwards to the node only (span EXCLUDED — see `span.rs`), so two
//! ASTs still compare structurally regardless of source position. The
//! AST holds no interned IDs or other identity-bearing state.

use super::span::Spanned;

/// An identifier as written in source, plus the byte range of the
/// identifier token. Diagnostics that name an identifier ("undeclared
/// `foo`", "duplicate kernel `k`") underline `span`; the `String`
/// compares/hashes structurally (span excluded — see `span.rs`).
pub type SpIdent = Spanned<String>;

/// An expression plus its source span. Used at every recursive
/// expression position so a future type / scope error can point at the
/// offending sub-expression (TASK-0090).
pub type SpExpr = Spanned<Expr>;

/// A statement plus its source span (top level and every nested
/// `for`-body statement).
pub type SpStmt = Spanned<Stmt>;

/// A top-level item plus its source span.
pub type SpItem = Spanned<Item>;

/// Scalar types per grammar §1, rule `ScalarType`. The set is closed
/// (PRD §6.2.4: no user-defined scalars, no `()` here — unit is only a
/// kernel return type, see [`KernelSig`]).
///
/// `serde` is derived (feature-gated, like the `event`/`contract`
/// types) so the codegen-contract [`crate::sidecar::NameSidecar`]
/// (TASK-0160) — which carries `ScalarType` per `DataId` for vec!
/// element / slot typing — is committable/serialisable. This adds
/// trait impls only under the `serde` feature; no behaviour change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ScalarType {
    Usize,
    Isize,
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
}

/// A typed data shape: a scalar plus zero or more dimensions.
///
/// `DimList` in the grammar is `('[' ConstExpr ']')+`. Here an empty
/// `dims` represents a bare scalar (no `DimList`), which is what
/// `ConstDecl`'s `ScalarType` parses to. The parser keeps `Type` and
/// `ScalarType` distinct surface forms but they share storage.
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub scalar: ScalarType,
    pub dims: Vec<SpExpr>,
}

/// Kernel purity (grammar §1, rule `Purity`; PRD §6.2.2 #5).
///
/// Stored on every [`KernelDecl`]. Later passes consult this to decide
/// whether reordering, duplication, or elimination of a call is legal
/// (PRD §6.2.2 #5, grammar §2 note 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purity {
    Pure,
    Effectful,
}

/// `const NAME : SCALAR = EXPR ;`
#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: SpIdent,
    pub ty: ScalarType,
    pub value: SpExpr,
}

/// `data NAME : TYPE ;`
#[derive(Debug, Clone, PartialEq)]
pub struct DataDecl {
    pub name: SpIdent,
    pub ty: Type,
}

/// `kernel NAME : SIG PURITY ;`
#[derive(Debug, Clone, PartialEq)]
pub struct KernelDecl {
    pub name: SpIdent,
    pub sig: KernelSig,
    pub purity: Purity,
}

/// `( T1, T2, ... ) -> RET`. `ret` is `None` for the explicit unit
/// return `()` (grammar `KernelRetType ::= DataType | '(' ')'`).
#[derive(Debug, Clone, PartialEq)]
pub struct KernelSig {
    pub params: Vec<Type>,
    /// `None` for unit return; `Some(t)` for a typed return.
    pub ret: Option<Type>,
}

/// LValue: `IDENT ('[' EXPR ']')*`.
///
/// Stored as the base identifier plus zero or more index expressions.
/// Used both as the left-hand side of a `<--` (dataflow target) and as
/// a `LValue`-shaped RValue (bare data reference; grammar §1).
#[derive(Debug, Clone, PartialEq)]
pub struct IndexedLValue {
    pub name: SpIdent,
    pub indices: Vec<SpExpr>,
}

/// Expressions cover both `IndexExpr` and `ConstExpr` (grammar §1).
/// The two share surface; scope rules are imposed by later passes
/// (grammar §2 note 6).
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Decimal integer literal. Range is not validated at parse time;
    /// overflow detection is a `ConstExpr` evaluation concern (grammar
    /// §2 note 3).
    IntLit(i64),
    /// Bare identifier reference (a const name or loop variable). The
    /// identifier carries its own span so an "undeclared `x`" error
    /// can underline the reference itself.
    Ident(SpIdent),
    /// Unary `-EXPR`. Grammar `UnaryExpr ::= ('-')? Atom`.
    Unary(UnaryOp, Box<SpExpr>),
    /// Binary arithmetic: `+ - * / %` with standard precedence
    /// (grammar §1 `AddExpr`, `MulExpr`).
    Binary(BinOp, Box<SpExpr>, Box<SpExpr>),
    /// A call expression used as an RValue (grammar `RValue ::=
    /// CallExpr | LValue`).
    Call(Call),
    /// A bare LValue used as an RValue (identity-copy form).
    LValue(IndexedLValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// `IDENT '(' (RValue (',' RValue)*)? ')'` (grammar `CallExpr`).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub callee: SpIdent,
    pub args: Vec<SpExpr>,
}

/// Statements that appear inside a `for` body and at the top level
/// (grammar §1: `TopItem ::= ... | Stmt`; `Stmt ::= DataflowStmt |
/// EffectStmt | ForStmt`).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `LVALUE <-- RVALUE ;`
    Dataflow { lhs: IndexedLValue, rhs: SpExpr },
    /// `CALL ;` — bare effectful call as a statement (grammar
    /// `EffectStmt`). Purity check happens later (grammar §2 note 5).
    Effect(Call),
    /// `for IDENT : EXPR .. EXPR { Stmt* }`
    For {
        var: SpIdent,
        lo: SpExpr,
        hi: SpExpr,
        body: Vec<SpStmt>,
    },
}

/// Top-level item. Per grammar `TopItem ::= ConstDecl | DataDecl |
/// KernelDecl | Stmt`. The grammar imposes no order; semantic passes
/// enforce declarations-before-use (grammar §5.5).
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Const(ConstDecl),
    Data(DataDecl),
    Kernel(KernelDecl),
    /// A top-level statement. Held as a [`SpStmt`] so every statement
    /// — top-level and nested `for`-body alike — is uniformly spanned;
    /// the enclosing [`SpItem`] additionally spans the whole item
    /// (same byte range as the statement for a bare top-level stmt).
    Stmt(SpStmt),
}

/// Root AST node: a `Program ::= TopItem*`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AlgoAst {
    pub items: Vec<SpItem>,
}

impl AlgoAst {
    /// Convenience: count items of each kind. Used by tests and by
    /// any tool that wants a quick structural summary.
    pub fn count_consts(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.node, Item::Const(_)))
            .count()
    }

    pub fn count_data(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.node, Item::Data(_)))
            .count()
    }

    pub fn count_kernels(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.node, Item::Kernel(_)))
            .count()
    }

    pub fn count_stmts(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.node, Item::Stmt(_)))
            .count()
    }
}
