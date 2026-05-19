//! AlgoIR: the semantically-validated intermediate representation of
//! an algorithm program.
//!
//! The IR is produced by [`crate::algo::lower_algo`] from an [`AlgoAst`]
//! (TASK-0007 output). Compared to the AST, the IR:
//!
//! - Resolves const expressions in data shape declarations to concrete
//!   `usize` values (e.g. `f32[B][H/2]` becomes `[16, 14]` when
//!   `B=16, H=28`).
//! - Enforces global declaration-name uniqueness (consts, data,
//!   kernels each in their own namespace; loop variables shadow at
//!   their loop and go out of scope at its end — PRD §6.2.3).
//! - Enforces single-assignment: each `data` symbol is the LHS of at
//!   most one dataflow statement per scope (PRD §6.2.1).
//! - Enforces lexical scoping of iteration variables: a `for y`
//!   variable is only visible inside its body. Referring to `y`
//!   outside is a [`LowerErrorKind::IterVarOutOfScope`].
//!
//! What this IR explicitly does NOT do (deferred to later passes):
//!
//! - Type-check kernel signatures against call-site arguments
//!   (whether `conv_block_1(input[n])` matches the declared parameter
//!   shape). Filed as a follow-up task.
//! - Resolve which kernel a `Call` actually refers to (the IR keeps
//!   the textual name; binding to a [`KernelDecl`] is straightforward
//!   but we don't store the back-reference yet).
//! - Validate kernel-purity vs the statement form (effect-statement
//!   bodies must call effectful kernels, dataflow-statement RHS must
//!   be pure). The information is preserved on [`KernelDecl::purity`];
//!   the check belongs to a later pass.
//!
//! Design choice — separate IR types vs annotated AST: separate.
//! Rationale: invariants differ (declarations bucketed, shapes
//! concrete, statements scope-checked). Annotating the AST in place
//! would force downstream passes to keep handling 'shape may or may
//! not be resolved' cases. A clean type boundary is cheaper.

use core::ops::Range;
use std::collections::BTreeMap;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::ast::{Purity, ScalarType};

/// A concrete data shape: scalar type + zero or more resolved
/// dimensions. Dimensions are evaluated at lower time, so this is a
/// `usize` per dim — no expression nodes left.
///
/// Zero dimensions == scalar. Matches `ScalarType` + `dims=[]` in
/// the AST.
///
/// `serde` is derived (feature-gated, like the `event`/`contract`
/// types) so the codegen-contract [`crate::sidecar::NameSidecar`]
/// (TASK-0160) can carry the per-`DataId` `ResolvedType` in a
/// committable/serialisable table. Trait impls only under the
/// `serde` feature; no behaviour change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResolvedType {
    pub scalar: ScalarType,
    pub dims: Vec<usize>,
}

impl ResolvedType {
    /// True if this is a scalar (zero dimensions).
    pub fn is_scalar(&self) -> bool {
        self.dims.is_empty()
    }
}

/// `const NAME : SCALAR = VALUE`. The value has been evaluated to an
/// `i64` during lowering. We keep `i64` because the const evaluator
/// is integer-arithmetic only; range-narrowing to the declared scalar
/// type is a later concern.
///
/// `serde` is derived (feature-gated) so the codegen-contract
/// [`crate::sidecar::NameSidecar`] (TASK-0160) can carry the const
/// name→value table the backend uses to re-render loop bounds
/// without the AlgoIR. Trait impls only under `serde`; no behaviour
/// change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ResolvedConst {
    pub name: String,
    pub ty: ScalarType,
    pub value: i64,
}

/// `data NAME : RESOLVED_TYPE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedData {
    pub name: String,
    pub ty: ResolvedType,
}

/// `kernel NAME : SIG PURITY`. The signature uses resolved shapes —
/// kernel parameters and return types are subject to the same const
/// evaluation as data declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedKernel {
    pub name: String,
    pub params: Vec<ResolvedType>,
    /// `None` for unit return; `Some(t)` for a typed return.
    pub ret: Option<ResolvedType>,
    pub purity: Purity,
}

/// Indexed reference to a data symbol. Indices are kept as IR
/// expressions (not folded to numbers) because they typically depend
/// on iteration variables.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IndexedRef {
    pub name: String,
    pub indices: Vec<IrExpr>,
}

/// IR expressions. Same shape as [`super::ast::Expr`] minus the
/// surface-only distinctions. Kept structurally close to the AST so
/// the lowering of an expression is mostly a copy; later passes may
/// fold integer subtrees, but we don't speculate here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum IrExpr {
    /// Integer literal. Stored as `i64`; range-narrowing happens later.
    IntLit(i64),
    /// Reference to a const or iteration variable. The lowering pass
    /// has already verified the name is in scope.
    Ident(String),
    /// `- expr`.
    Neg(Box<IrExpr>),
    /// Binary arithmetic.
    BinOp(IrBinOp, Box<IrExpr>, Box<IrExpr>),
    /// Indexed read of a data symbol. Only legal as an argument to
    /// a kernel call (or as an identity-copy RHS).
    DataRef(IndexedRef),
    /// Kernel call. The callee is kept as a textual name; binding to
    /// a [`ResolvedKernel`] is straightforward but not stored here.
    Call { callee: String, args: Vec<IrExpr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum IrBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

/// IR statements. Mirrors [`super::ast::Stmt`] but with name-scoped
/// validation done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrStmt {
    /// `LVALUE <-- RHS`.
    Dataflow { lhs: IndexedRef, rhs: IrExpr },
    /// `CALL(...)` as a statement (effectful by convention; the
    /// purity check is a later pass).
    Effect { callee: String, args: Vec<IrExpr> },
    /// `for VAR : LO .. HI { BODY }`.
    For {
        var: String,
        lo: IrExpr,
        hi: IrExpr,
        body: Vec<IrStmt>,
    },
}

/// Root IR node.
///
/// Declarations are bucketed for O(1) lookup by name. `consts` keeps
/// insertion order via [`BTreeMap`] sorted by name; if source order
/// becomes important later we can switch to an index-vec.
///
/// Statements retain source order — execution order in the algorithm
/// is meaningful (PRD §6.2.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AlgoIR {
    /// Resolved consts, keyed by name.
    pub consts: BTreeMap<String, ResolvedConst>,
    /// Resolved data declarations, keyed by name.
    pub data: BTreeMap<String, ResolvedData>,
    /// Resolved kernel declarations, keyed by name.
    pub kernels: BTreeMap<String, ResolvedKernel>,
    /// Top-level statements in source order.
    pub stmts: Vec<IrStmt>,
}

/// The semantic-violation *kind* produced by the lowering pass.
///
/// Each variant names a single semantic violation and carries the
/// payload a diagnostic message needs (the offending name, the owning
/// declaration, etc.). The *source position* of the violation is NOT
/// here — it is carried separately on [`LowerError`] so that adding
/// positions did not change any variant's payload shape (TASK-0090).
/// Equality / hashing / Display of a located error forward to this
/// kind; see [`LowerError`] for why position is excluded from value
/// identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerErrorKind {
    /// Two `const` declarations share a name.
    DuplicateConst(String),
    /// Two `data` declarations share a name.
    DuplicateData(String),
    /// Two `kernel` declarations share a name.
    DuplicateKernel(String),
    /// The same data symbol appears as the LHS of two dataflow
    /// statements in the same scope. PRD §6.2.1: single-assignment.
    DoubleAssignment {
        data: String,
        /// Containing scope description, used in error messages.
        /// `"<top-level>"` for the program root; `"for <var>"` for a
        /// loop body.
        scope: String,
    },
    /// A reference to a name that is not a declared const and not an
    /// iteration variable currently in scope.
    UnknownIdent(String),
    /// An iteration variable was referenced outside its `for` body.
    /// Carries the variable name. (We do not carry the surrounding
    /// loop's identifier — by the time the reference is seen, the
    /// loop has lexically closed; identifying *which* loop owned the
    /// name is interesting for diagnostics but not for soundness.)
    IterVarOutOfScope(String),
    /// LHS of a dataflow statement names something that is not a
    /// declared `data` symbol. (Iteration variables, consts, and
    /// kernels are not assignable.)
    AssignmentTargetNotData(String),
    /// A const declaration references an identifier that is not a
    /// previously declared const. (Const evaluation requires the
    /// referent to be known.)
    ConstRefersToNonConst {
        in_const: String,
        unknown_ident: String,
    },
    /// Const declarations form a reference cycle (a -> b -> a).
    /// Detected by repeated visit during evaluation. The vector
    /// records the cycle path in visit order.
    ConstCycle(Vec<String>),
    /// Const expression contains a non-integer construct (call,
    /// data-index, etc.).
    NonIntegerConstExpr { in_const: String, reason: String },
    /// Integer overflow during const evaluation.
    ConstOverflow { in_const: String, op: String },
    /// Divide-by-zero during const evaluation.
    ConstDivByZero { in_const: String },
    /// Shape dimension evaluated to a non-positive value (zero or
    /// negative). PRD requires positive dimensions.
    NonPositiveDim { decl: String, value: i64 },
    /// Shape dimension contains an identifier that is not a
    /// previously declared const. (Shape evaluation happens at lower
    /// time, before any data is computed, so only consts can appear.)
    ShapeRefersToNonConst { decl: String, unknown_ident: String },
    /// Shape dimension is a non-integer construct.
    NonIntegerShapeExpr { decl: String, reason: String },
    /// Shape dimension overflowed during evaluation.
    ShapeOverflow { decl: String, op: String },
    /// Shape dimension divided by zero.
    ShapeDivByZero { decl: String },
    /// A loop's iteration variable shadows a declared name. We treat
    /// this as an error rather than silently shadowing — the PRD
    /// makes iteration variables and data variables share one
    /// namespace (PRD §6.2.3) and allows shadowing at loop scope, but
    /// shadowing a const or a data symbol with a loop variable invites
    /// confusion. Flagged conservatively; relax if a real example
    /// needs it.
    IterVarShadowsDecl { var: String, shadows: String },
}

impl std::fmt::Display for LowerErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerErrorKind::DuplicateConst(n) => write!(f, "duplicate const `{n}`"),
            LowerErrorKind::DuplicateData(n) => write!(f, "duplicate data `{n}`"),
            LowerErrorKind::DuplicateKernel(n) => write!(f, "duplicate kernel `{n}`"),
            LowerErrorKind::DoubleAssignment { data, scope } => write!(
                f,
                "data `{data}` is assigned twice in the same scope ({scope}); single-assignment violated"
            ),
            LowerErrorKind::UnknownIdent(n) => write!(f, "unknown identifier `{n}`"),
            LowerErrorKind::IterVarOutOfScope(n) => write!(
                f,
                "iteration variable `{n}` is referenced outside its loop body"
            ),
            LowerErrorKind::AssignmentTargetNotData(n) => {
                write!(f, "assignment target `{n}` is not a declared `data` symbol")
            }
            LowerErrorKind::ConstRefersToNonConst {
                in_const,
                unknown_ident,
            } => write!(
                f,
                "const `{in_const}` refers to `{unknown_ident}`, which is not a declared const"
            ),
            LowerErrorKind::ConstCycle(path) => {
                write!(f, "const reference cycle: {}", path.join(" -> "))
            }
            LowerErrorKind::NonIntegerConstExpr { in_const, reason } => {
                write!(f, "const `{in_const}` has a non-integer expression: {reason}")
            }
            LowerErrorKind::ConstOverflow { in_const, op } => {
                write!(f, "integer overflow in const `{in_const}` during `{op}`")
            }
            LowerErrorKind::ConstDivByZero { in_const } => {
                write!(f, "divide-by-zero in const `{in_const}`")
            }
            LowerErrorKind::NonPositiveDim { decl, value } => {
                write!(f, "shape dimension in `{decl}` evaluated to {value}; must be positive")
            }
            LowerErrorKind::ShapeRefersToNonConst {
                decl,
                unknown_ident,
            } => write!(
                f,
                "shape of `{decl}` refers to `{unknown_ident}`, which is not a declared const"
            ),
            LowerErrorKind::NonIntegerShapeExpr { decl, reason } => {
                write!(f, "shape of `{decl}` has a non-integer expression: {reason}")
            }
            LowerErrorKind::ShapeOverflow { decl, op } => {
                write!(f, "integer overflow in shape of `{decl}` during `{op}`")
            }
            LowerErrorKind::ShapeDivByZero { decl } => {
                write!(f, "divide-by-zero in shape of `{decl}`")
            }
            LowerErrorKind::IterVarShadowsDecl { var, shadows } => write!(
                f,
                "iteration variable `{var}` shadows a declaration `{shadows}`"
            ),
        }
    }
}

/// A lowering error: a [`LowerErrorKind`] plus, where a single
/// offending source node exists, the byte [`Range`] it was parsed
/// from (TASK-0090).
///
/// # Why a struct wrapping a kind (not `(line, column)` fields per
/// variant)
///
/// Putting a position on a *wrapper* instead of widening every variant
/// means no variant's payload shape changed: the existing negative
/// tests still pattern-match `LowerErrorKind::X(payload)` with the same
/// payload, only through `err.kind`. The byte range — not `(line,
/// column)` — is stored because lowering takes `&AlgoAst` only and has
/// no source string; the driver (which holds the source) converts via
/// [`crate::error::offset_to_line_col`] at display time, exactly as
/// [`crate::error::ParseError`] is surfaced. This keeps one span
/// representation end-to-end (matching [`crate::algo::span::Spanned`])
/// and lowering source-text-free.
///
/// # `span` is `Option` (honest-partial per variant — TASK-0090)
///
/// Most variants have one obviously-offending node and carry its span.
/// A few genuinely do not: [`LowerErrorKind::ConstCycle`] spans several
/// declarations (no single primary node), and the
/// `<index/loop-bound expression>` synthetic
/// [`LowerErrorKind::NonIntegerShapeExpr`] is reported against a
/// pseudo-decl, not a real source span. Those are left `None` rather
/// than fabricating a misleading location: a documented missing
/// position is honest; a wrong one is not.
///
/// # Equality semantics (load-bearing — AC#4, mirrors `Spanned`)
///
/// [`PartialEq`] / [`Eq`] are **hand-written to forward to `kind`
/// only**; `span` is deliberately EXCLUDED from value identity. This
/// is the same decision, for the same reason, as
/// [`crate::algo::span::Spanned`] (TASK-0082): the source position is
/// informational-for-humans, not part of *which semantic error this
/// is*. Excluding it keeps every existing `LowerErrorKind`-asserting
/// negative test valid (they assert the semantic kind + payload, never
/// the byte offset); a dedicated test asserts the position separately.
/// `#[derive(PartialEq)]` would (wrongly) fold the span into equality.
#[derive(Debug, Clone)]
pub struct LowerError {
    /// The semantic violation.
    pub kind: LowerErrorKind,
    /// Byte range into the original source, when a single offending
    /// node exists. `None` for genuinely multi-site / synthetic
    /// variants (see type docs). Feed `span.start` to
    /// [`crate::error::offset_to_line_col`] for a 1-based
    /// `(line, column)`.
    pub span: Option<Range<usize>>,
}

impl LowerError {
    /// A lowering error with no source position (multi-site or
    /// synthetic — see type docs). Prefer [`LowerError::at`] whenever a
    /// single offending [`crate::algo::span::Spanned`] is in scope.
    pub fn new(kind: LowerErrorKind) -> Self {
        Self { kind, span: None }
    }

    /// A lowering error located at `span` — the byte range of the
    /// offending source node (`spanned.span`). This is the path AC#1
    /// requires for every diagnosable variant that has a single
    /// offending node.
    pub fn at(kind: LowerErrorKind, span: Range<usize>) -> Self {
        Self {
            kind,
            span: Some(span),
        }
    }

    /// Render the error with a source location resolved against `src`.
    ///
    /// This is the driver-facing surface (AC#2): the driver holds the
    /// algorithm source, so it — not lowering — turns the stored byte
    /// offset into a `line:column`. Mirrors how
    /// [`crate::error::ParseError`] is surfaced. When the variant has
    /// no position (see type docs), the message is the kind alone, with
    /// no fabricated location.
    pub fn display_with_src(&self, src: &str) -> String {
        match &self.span {
            Some(span) => {
                let offset = span.start.min(src.len());
                let (line, col) = crate::error::offset_to_line_col(src, offset);
                format!("{} at {line}:{col}", self.kind)
            }
            None => self.kind.to_string(),
        }
    }
}

// Hand-written: forward to `kind`, EXCLUDE `span` from identity
// (AC#4, same rationale as `Spanned`). Deriving would fold the span in
// and break every existing `LowerErrorKind`-asserting negative test.
impl PartialEq for LowerError {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for LowerError {}

// Span-free Display: library callers / tests without source text get
// the semantic message unchanged from before TASK-0090. The located
// form is `display_with_src` (driver-side).
impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for LowerError {}
