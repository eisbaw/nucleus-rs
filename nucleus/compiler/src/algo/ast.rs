//! AST types for the algorithm sublanguage.
//!
//! Shape and naming follow `docs/grammar-algo.md` 1:1 — one Rust type
//! per nonterminal where that aids readability, collapsed where the
//! grammar splits something purely for parsing convenience (e.g.
//! `IndexExpr` and `ConstExpr` share the same shape, so both are
//! [`Expr`]).
//!
//! Spans (line/column) are not tracked on AST nodes in this iteration.
//! Only [`crate::algo::ParseError`] carries position. This is a known
//! limitation; downstream passes (TASK-0009, TASK-0011) will want span
//! info for good diagnostics. Filed as follow-up.
//!
//! Equality semantics: `PartialEq` is derived to make tests cheap. Two
//! ASTs compare structurally; this is fine because the AST holds no
//! interned IDs or other identity-bearing state.

/// Scalar types per grammar §1, rule `ScalarType`. The set is closed
/// (PRD §6.2.4: no user-defined scalars, no `()` here — unit is only a
/// kernel return type, see [`KernelSig`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    pub dims: Vec<Expr>,
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
    pub name: String,
    pub ty: ScalarType,
    pub value: Expr,
}

/// `data NAME : TYPE ;`
#[derive(Debug, Clone, PartialEq)]
pub struct DataDecl {
    pub name: String,
    pub ty: Type,
}

/// `kernel NAME : SIG PURITY ;`
#[derive(Debug, Clone, PartialEq)]
pub struct KernelDecl {
    pub name: String,
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
    pub name: String,
    pub indices: Vec<Expr>,
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
    /// Bare identifier reference (a const name or loop variable).
    Ident(String),
    /// Unary `-EXPR`. Grammar `UnaryExpr ::= ('-')? Atom`.
    Unary(UnaryOp, Box<Expr>),
    /// Binary arithmetic: `+ - * / %` with standard precedence
    /// (grammar §1 `AddExpr`, `MulExpr`).
    Binary(BinOp, Box<Expr>, Box<Expr>),
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
    pub callee: String,
    pub args: Vec<Expr>,
}

/// Statements that appear inside a `for` body and at the top level
/// (grammar §1: `TopItem ::= ... | Stmt`; `Stmt ::= DataflowStmt |
/// EffectStmt | ForStmt`).
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `LVALUE <-- RVALUE ;`
    Dataflow { lhs: IndexedLValue, rhs: Expr },
    /// `CALL ;` — bare effectful call as a statement (grammar
    /// `EffectStmt`). Purity check happens later (grammar §2 note 5).
    Effect(Call),
    /// `for IDENT : EXPR .. EXPR { Stmt* }`
    For {
        var: String,
        lo: Expr,
        hi: Expr,
        body: Vec<Stmt>,
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
    Stmt(Stmt),
}

/// Root AST node: a `Program ::= TopItem*`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AlgoAst {
    pub items: Vec<Item>,
}

impl AlgoAst {
    /// Convenience: count items of each kind. Used by tests and by
    /// any tool that wants a quick structural summary.
    pub fn count_consts(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Const(_)))
            .count()
    }

    pub fn count_data(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Data(_)))
            .count()
    }

    pub fn count_kernels(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Kernel(_)))
            .count()
    }

    pub fn count_stmts(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Stmt(_)))
            .count()
    }
}
