//! AST types for the algorithm sublanguage.
//!
//! Shape and naming follow `docs/grammar-algo.md` 1:1 — one Rust type
//! per nonterminal where that aids readability, collapsed where the
//! grammar splits something purely for parsing convenience (e.g.
//! `IndexExpr` and `ConstExpr` share the same shape, so both are
//! [`Expr`]).
//!
//! Per-node source spans ARE tracked (TASK-0082): the diagnosable
//! nodes are wrapped in [`Spanned<T>`][crate::span::Spanned],
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

use crate::span::Spanned;

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

impl ScalarType {
    /// The FIXED on-wire byte width of one element of this scalar type,
    /// or `None` for the platform-dependent widths (`Usize`/`Isize`)
    /// whose size differs between a 64-bit host and a 32-bit embedded
    /// target.
    ///
    /// This is the EXACT width a fixed-width little-endian serialiser
    /// (e.g. the reference `input.bin`/`reference.bin` generators, or
    /// the embedded multi-MCU input partition) must use — NOT a budget
    /// estimate. A consumer that needs a width for `Usize`/`Isize` must
    /// decide the target word size itself (or fail loud) rather than
    /// have this method silently guess; that is why this returns
    /// `Option` instead of defaulting `Usize`/`Isize` to 8.
    pub fn fixed_byte_width(&self) -> Option<usize> {
        use ScalarType::*;
        match self {
            U8 | I8 | Bool => Some(1),
            U16 | I16 => Some(2),
            U32 | I32 | F32 => Some(4),
            U64 | I64 | F64 => Some(8),
            Usize | Isize => None,
        }
    }
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Purity {
    Pure,
    Effectful,
}

/// Overlapping-write accumulator combine identity, declared by an
/// optional `combine = <op>` kernel attribute (TASK-0343.01.01).
///
/// When a kernel owns an overlapping-write accumulator fan-in (the
/// distributed `acc[b] <-- k(acc[b], ...)` shape, partitioned over an
/// OUTER loop so each worker whole-array-replicates `acc`), the host
/// element-wise combine of the per-worker partials must use this
/// algebraic identity instead of the pre-TASK-0343.01.01 hardcoded
/// `wrapping_add`.
///
/// # Identity per op
///
/// `Sum`/`Or`/`Xor` share the additive identity ZERO; their init was
/// already correct in TASK-0343.01.01. The NON-zero-identity ops
/// `Min` (identity = type MAX), `Max` (identity = type MIN), `And`
/// (identity = all-ones) were added in TASK-0343.01.02 and REQUIRE
/// identity-aware accumulator pre-init (the init literal is chosen by
/// `combine_identity_literal` in `backend-common`'s render layer, not
/// the hardcoded zero). All six ops are associative + commutative ⇒
/// order-independent across worker arrival ⇒ bit-identical (PRD §10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CombineOp {
    /// `wrapping_add` — additive sum (identity 0). The pre-TASK-0343.01.01
    /// hardcoded behaviour; `combine = sum` is byte-identical to it.
    Sum,
    /// Bitwise OR `|` (identity 0).
    Or,
    /// Bitwise XOR `^` (identity 0).
    Xor,
    /// `.min()` — minimum (identity = type MAX, so `min(MAX, x) == x`).
    /// TASK-0343.01.02; requires identity-aware init.
    Min,
    /// `.max()` — maximum (identity = type MIN). TASK-0343.01.02.
    Max,
    /// Bitwise AND `&` (identity = all-ones, spelled `!0T`).
    /// TASK-0343.01.02.
    And,
    /// `+` — FLOAT-ONLY reproducible (fixed-order) sum (identity 0.0).
    /// TASK-0453.03 (rigour epic P3). The opt-in float-sum combine.
    ///
    /// Plain `Sum` is REJECTED on float scalars because IEEE-754 addition
    /// is non-associative, so an order-varying host fan-in would diverge
    /// across backends (PRD §10.1). `Fsum` is the user's explicit opt-in
    /// to the *fixed-order* fold the compiler already emits: the host
    /// combines per-worker partials in worker-id-sorted event-list order
    /// (TASK-0389), identical across all backends, so the reduced bits
    /// are cross-backend reproducible FOR A GIVEN SCHEDULE. It is NOT the
    /// naive single-pass IEEE sum, and NOT schedule-invariant (a
    /// different worker count folds differently) — that is the documented
    /// residual. Float-only: rejected on integer (`use sum`) and bool.
    Fsum,
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

/// `kernel NAME : SIG PURITY [ 'combine' '=' CombineOp ] ;`
#[derive(Debug, Clone, PartialEq)]
pub struct KernelDecl {
    pub name: SpIdent,
    pub sig: KernelSig,
    pub purity: Purity,
    /// Optional overlapping-write accumulator combine identity
    /// (TASK-0343.01.01). `Some(_)` iff the decl carries a
    /// `combine = <op>` attribute (parsed contextually after purity).
    /// `None` for every kernel that does not own an accumulator
    /// fan-in — which is every kernel shipped before TASK-0343.01.01.
    pub combine: Option<CombineOp>,
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
    // TASK-0194: the former `Ident(SpIdent)` variant was removed. The
    // parser never constructs it: `parser.rs::ident_or_call` always
    // routes a bare identifier through `index_tail` (`.repeated()`,
    // possibly empty) producing `Expr::LValue(IndexedLValue{ indices:
    // [] })`, never `Expr::Ident`. It was parser-unreachable
    // dead-at-construction (predating TASK-0082, which only re-typed
    // its payload). The real bare-identifier path is the
    // empty-indices arm of `Expr::LValue` in `algo::lower`. Removing
    // the variant and its (defensive, equally dead) lower.rs arms is
    // a no-behaviour-change cleanup (proven by the determinism gate).
    /// Unary `-EXPR`. Grammar `UnaryExpr ::= ('-')? Atom`.
    Unary(UnaryOp, Box<SpExpr>),
    /// Binary arithmetic: `+ - * / %` with standard precedence
    /// (grammar §1 `AddExpr`, `MulExpr`).
    Binary(BinOp, Box<SpExpr>, Box<SpExpr>),
    /// A single relational comparison `lhs CMPOP rhs` producing a
    /// **bool**-typed value (grammar §1 `RelExpr`, TASK-0341.02.01.02 /
    /// epic S2). Deliberately a DEDICATED node, NOT a [`BinOp`] variant:
    /// `BinOp` is closed over integer-valued arithmetic, whereas a
    /// comparison yields `bool` (a distinct result type) and sits at its
    /// own precedence level BELOW additive. Folding it into `BinOp` would
    /// invite later passes to treat a comparison as an integer operand.
    ///
    /// Precedence is non-associative single comparison: `RelExpr ::=
    /// AddExpr (CmpOp AddExpr)?`. Chaining (`a < b < c`) does not parse.
    ///
    /// SCOPE (S2): a comparison is legal ONLY where a bool is expected —
    /// today that is a bool-typed dataflow RHS (and, future, the S1
    /// `for..until` condition, TASK-0341.02.01.03). It is REJECTED with a
    /// typed [`crate::algo::ir::LowerErrorKind::ComparisonNotAllowedHere`]
    /// in index / loop-bound / const / shape position (bool-in-int).
    /// Full bool-DATA codegen (`Vec<bool>` buffer / input.bin layout) is
    /// OUT OF SCOPE for S2 and deferred to the S1 until-condition
    /// consumer (TASK-0341.02.01.03).
    Compare(CmpOp, Box<SpExpr>, Box<SpExpr>),
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

/// Relational comparison operators (grammar §1 `RelExpr`,
/// TASK-0341.02.01.02 / epic S2). Each produces a **bool**-typed value
/// from two integer-valued operands. Single, non-associative: the
/// grammar admits at most one per `RelExpr` (no `a < b < c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CmpOp {
    /// `<=`
    Le,
    /// `<`
    Lt,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `>`
    Gt,
    /// `>=`
    Ge,
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
    /// `for IDENT : EXPR .. EXPR (until COND)? { Stmt* }`
    ///
    /// `until` is the OPTIONAL bounded early-exit halt clause
    /// (TASK-0341.02.01.03 / epic S1). When `Some`, the loop is a
    /// capped early-exit loop: the `hi` bound stays the compile-time CAP
    /// (statically bounded) and `until` carries the halt predicate COND.
    ///
    /// It is an OPTIONAL FIELD, not a new `Stmt` variant, deliberately:
    /// a field is compiler-forced at every CONSTRUCTION site (so no
    /// construction can silently omit COND), while the existing match
    /// sites that destructure with `{ var, .., }` remain inert and ignore
    /// it. That inertness is SOUND only because an `until`-loop is
    /// rejected at the ACFG-build boundary (`build_acfg`, the FIRST
    /// pre-mediation pass — see `crate::pipeline::run_pre_mediation_passes`)
    /// BEFORE any downstream analysis pass (block/partition/halo/reuse/
    /// sync/transfer/sidecar) consumes the IR. S1 is INERT.
    For {
        var: SpIdent,
        lo: SpExpr,
        hi: SpExpr,
        /// Optional `until COND` halt predicate (epic S1). `None` for an
        /// ordinary fixed-iteration loop. Spanned at the COND expression.
        until: Option<SpExpr>,
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
