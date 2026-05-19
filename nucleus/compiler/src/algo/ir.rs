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
//!
//! Kernel-purity vs statement-form enforcement (TASK-0089) lives in
//! [`crate::algo::lower::lower_stmt`]: an `EffectStmt` (bare-call
//! statement) callee MUST be an [`super::ast::Purity::Effectful`]
//! kernel — the grammar §2 note 5 rule, the only direction the formal
//! grammar specifies. A bare-call to a [`super::ast::Purity::Pure`]
//! kernel emits [`LowerErrorKind::EffectCalleeNotEffectful`] at the
//! callee's identifier span.
//!
//! The OTHER direction — whether a `DataflowStmt` RHS Call must
//! reference a pure kernel — is **intentionally not enforced**. The
//! grammar §2 note 5 is unidirectional and every shipped example
//! (01..07, 13, 14) puts an effectful load/capture kernel on the RHS
//! of `<--` (`a <-- load_input();`, `mic_in[frame] <-- fe_capture();`).
//! Whether PRD §6.2.2 row 77's wording ("where pure mandatory; where
//! !effectful opt-in") was meant bidirectional is filed as TASK-0201
//! for a spec decision; until then this pass enforces only the
//! grammar-supported direction.
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
    /// An `EffectStmt` (bare-call statement) names a kernel declared
    /// `pure`. Grammar §2 note 5: bare-call statements are only valid
    /// when the called kernel is `effectful` (a pure call with the
    /// value discarded is meaningless — pure kernels are reorderable,
    /// deduplicable, eliminable, so calling one purely for its
    /// side-effect is a contradiction). TASK-0089. The OTHER direction
    /// (DataflowStmt RHS Call must be pure) is intentionally NOT
    /// enforced — see [`crate::algo::ir`] module docs and TASK-0201.
    EffectCalleeNotEffectful { callee: String },
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
            LowerErrorKind::EffectCalleeNotEffectful { callee } => write!(
                f,
                "effect-statement callee `{callee}` references a pure kernel; expected effectful (grammar §2 note 5)"
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
/// representation end-to-end (matching [`crate::span::Spanned`])
/// and lowering source-text-free.
///
/// # `span` is `Option` (honest-partial per variant — TASK-0090)
///
/// Most variants have one obviously-offending node and carry its span.
/// Exactly one genuinely does not: [`LowerErrorKind::ConstCycle`] spans
/// several declarations (no single primary node) and is the SOLE
/// position-less variant (`span: None`) — a documented missing position
/// is honest; a fabricated one is not. NOTE: the synthetic
/// `<index/loop-bound expression>` [`LowerErrorKind::NonIntegerShapeExpr`]
/// DOES carry a real span — only its `decl` *label* string is synthetic;
/// the offending expression it points at is a genuine source node, so
/// its real `expr.span` is used (it is NOT `None`).
///
/// # Equality semantics (load-bearing — AC#4, mirrors `Spanned`)
///
/// [`PartialEq`] / [`Eq`] are **hand-written to forward to `kind`
/// only**; `span` is deliberately EXCLUDED from value identity. This
/// is the same decision, for the same reason, as
/// [`crate::span::Spanned`] (TASK-0082): the source position is
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
    /// node exists. `None` only for the genuinely multi-site
    /// `ConstCycle` (see type docs). Feed `span.start` to
    /// [`crate::error::offset_to_line_col`] for a 1-based
    /// `(line, column)`.
    pub span: Option<Range<usize>>,
}

impl LowerError {
    /// A lowering error with no source position (the multi-site
    /// `ConstCycle` — see type docs). Prefer [`LowerError::at`] whenever
    /// a single offending [`crate::span::Spanned`] is in scope.
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

/// A non-empty, deterministically-ordered bundle of [`LowerError`]s —
/// the multi-error result of one [`lower_algo`](super::lower::lower_algo)
/// pass (TASK-0092).
///
/// # Why a new owner and NOT `crate::error::ParseErrors`
///
/// `ParseErrors` is the *parser* layer's owner ([`ParseError`], not
/// [`LowerError`]); they are different types at a different pipeline
/// stage. The SURFACING pattern (non-empty owner, `.errors()`,
/// driver iterates one located line per error) is the proven template
/// from TASK-0080/0081, but the type is layer-specific — reusing
/// `ParseErrors` would conflate the two layers' error vocabularies.
///
/// # Non-empty invariant (load-bearing)
///
/// A `LowerErrors` is constructed *only* when lowering actually
/// failed, so the inner `Vec` is never empty. The single constructor
/// [`LowerErrors::from_nonempty`] is the sole entry point and
/// `debug_assert!`s this; [`LowerErrors::first`] therefore never has
/// an empty slice to handle. Construction is private to the crate so
/// no external caller can forge an empty bundle.
///
/// # Ordering / determinism (PRD §10.1)
///
/// The vector is in **source / declaration order** — lowering walks
/// `AlgoAst::items` in order and pushes each error as it is found.
/// There is NO `HashMap`/`HashSet` iteration on the error-collection
/// path (the cascade-suppression bookkeeping is a `BTreeMap`), so the
/// emitted error sequence is a pure deterministic function of the
/// input. Two builds of the same broken program emit byte-identical
/// diagnostics.
///
/// # Equality
///
/// Derived `PartialEq`/`Eq` — element-wise over [`LowerError`], whose
/// own equality forwards to `kind` (span excluded; same rationale as
/// [`crate::span::Spanned`]). So bundle equality compares the ordered
/// sequence of *semantic kinds*, not byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerErrors(Vec<LowerError>);

impl LowerErrors {
    /// Construct from a non-empty `Vec<LowerError>`. The sole
    /// constructor; crate-private so the non-empty invariant cannot be
    /// violated from outside lowering. `debug_assert!`s non-emptiness
    /// (a caller handing an empty vec is a lowering-pass bug, not a
    /// user-input condition — decision-0003: invariant violation, so
    /// `debug_assert!`, not a typed error).
    pub(crate) fn from_nonempty(errors: Vec<LowerError>) -> Self {
        debug_assert!(
            !errors.is_empty(),
            "LowerErrors is constructed only on a non-empty failure set \
             (lowering-pass invariant); an empty vec here is a compiler bug"
        );
        Self(errors)
    }

    /// The first (source-order-earliest) error. Equivalent to the
    /// single error the pre-multi-error pass would have `?`-returned,
    /// so negative tests that previously asserted *the* error migrate
    /// by calling `.first()` with the SAME discriminating match — no
    /// loss of assertion strength.
    pub fn first(&self) -> &LowerError {
        self.0
            .first()
            .expect("LowerErrors is constructed non-empty (invariant)")
    }

    /// All errors in source order. The driver iterates this to surface
    /// every violation in one compile cycle.
    pub fn errors(&self) -> &[LowerError] {
        &self.0
    }
}

impl std::ops::Deref for LowerErrors {
    type Target = [LowerError];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// One error per line, each via the span-free [`LowerError`] `Display`
/// (the located form is the driver's `display_with_src`, which holds
/// the source). This is the fallback for a caller that just `{}`s the
/// whole bundle.
impl std::fmt::Display for LowerErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for LowerErrors {}
