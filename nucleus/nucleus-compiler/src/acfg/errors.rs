//! ACFG construction error surface: [`LoopBoundEnd`] +
//! [`BuildAcfgError`]. This is the *diagnosable user-input* failure
//! type for [`super::build_acfg`]; the panics reachable from
//! [`super::build_acfg`] are genuine link-pass invariant violations
//! (`link` rejects such programs first) so they stay panics.

use crate::algo::IrExpr;

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Which end of a `for` loop's range failed to evaluate to a constant.
///
/// Carried in [`BuildAcfgError::NonConstLoopBound`] so the diagnostic
/// can name the offending bound precisely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopBoundEnd {
    /// The lower bound (`for v : LO .. hi`).
    Lower,
    /// The upper bound (`for v : lo .. HI`).
    Upper,
}

impl std::fmt::Display for LoopBoundEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopBoundEnd::Lower => write!(f, "lower"),
            LoopBoundEnd::Upper => write!(f, "upper"),
        }
    }
}

/// Errors produced by [`build_acfg`](crate::acfg::build_acfg).
///
/// Each variant carries enough context to produce a user-facing
/// diagnostic without the caller needing to thread additional state —
/// the same contract as
/// [`crate::passes::block_transform::BlockTransformError`] and
/// [`crate::sidecar::SidecarError`].
///
/// This enum exists for the *diagnosable user-input* failure only. The
/// other `panic!`s reachable from [`build_acfg`](crate::acfg::build_acfg) (a kernel with no
/// placement, an undeclared bound symbol, a worker not in the name
/// table) are genuine link-pass invariant violations — `link` rejects
/// such programs first — so they stay `panic!`s, not variants here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildAcfgError {
    /// A `for` loop bound is not a compile-time `i64` constant.
    ///
    /// The algorithm grammar admits any in-scope identifier in a loop
    /// bound (a `const` *or* an enclosing iteration variable — see
    /// `algo::lower::lower_index_expr`), so a *triangular* loop like
    /// `for j : 0 .. i { ... }` lowers and links cleanly but cannot be
    /// folded to a concrete `Range<i64>` here (PRD §6.2 loop bounds are
    /// const expressions; `eval_const` only resolves declared consts).
    /// That is **diagnosable user input**, not a compiler invariant, so
    /// it is a typed error surfaced cleanly by the driver rather than a
    /// Rust panic. Carries the loop variable, which end failed, and the
    /// offending expression verbatim so the diagnostic is actionable
    /// without re-reading the source.
    ///
    /// A first-class fix (an iter-var-dependent / clamped loop form) is
    /// future language work tracked alongside the in-array-scan
    /// limitation; see PRD §6.2.5.
    NonConstLoopBound {
        /// The loop variable whose bound is non-const (e.g. `j`).
        var: String,
        /// Which end of the range failed (`lower` or `upper`).
        end: LoopBoundEnd,
        /// The offending bound expression, verbatim from the IR.
        expr: IrExpr,
    },
    /// A `for` loop bound IS a compile-time constant expression, but
    /// folding it to an `i64` FAILED — the arithmetic overflowed `i64`
    /// or divided by zero.
    ///
    /// This is DISTINCT from [`BuildAcfgError::NonConstLoopBound`]
    /// (TASK-0398): there the bound is not a constant at all (an
    /// iter-var / data / call); here the user *did* write a constant,
    /// it just does not fit `i64` (e.g. `-(0 - 9223372036854775807 - 1)`
    /// = `i64::MIN`, whose negation overflows) or divides by zero. Before
    /// TASK-0398 both cases collapsed to `NonConstLoopBound`, whose
    /// "use a constant bound" advice is wrong for this case — the user
    /// already used a constant. Carries the loop variable, which end
    /// failed, the offending expression verbatim, and a human `detail`
    /// naming the failure ("arithmetic overflow (mul)" / "division by
    /// zero").
    ///
    /// Overflow and division-by-zero share this ONE variant by design;
    /// the `detail` string is the human discriminator. There is
    /// deliberately no separate div-by-zero variant — a programmatic
    /// consumer must NOT branch on the `detail` text (it is for display).
    /// (The `algo::lower` layer does split `ConstOverflow` /
    /// `ConstDivByZero`; the loop-bound diagnostic does not need that
    /// granularity.)
    OverflowingLoopBound {
        /// The loop variable whose bound overflowed (e.g. `j`).
        var: String,
        /// Which end of the range failed (`lower` or `upper`).
        end: LoopBoundEnd,
        /// The offending bound expression, verbatim from the IR.
        expr: IrExpr,
        /// Human detail naming the fold failure, e.g.
        /// `"arithmetic overflow (mul)"` or `"division by zero"`.
        detail: String,
    },
    /// A dataflow statement `LHS <-- RHS` whose RHS is **not a kernel
    /// call** — a bare identity copy (`c <-- a`), a scalar/arithmetic
    /// expression (`c <-- a + b`), or a literal (`c <-- 5`).
    ///
    /// In Nucleus v2 every dataflow production / data movement is
    /// expressed through a kernel: an [`super::Operation`] /
    /// [`super::DataflowEdge`] / `Event::Fire` all carry a *non-optional*
    /// `KernelId`, and there is no schedule directive mapping a data
    /// symbol to a worker set (only `place_data D in REGION`, a memory
    /// region — see [`crate::sched`]). A kernel-less RHS therefore has no
    /// representable [`super::Operation`].
    ///
    /// This is **diagnosable user input**, structurally analogous to
    /// [`BuildAcfgError::NonConstLoopBound`]: a grammar-legal form the
    /// codegen does not (yet) support. Before TASK-0360's design slice
    /// it was a SILENT DROP — `build_dataflow` returned `None`, the
    /// statement produced no `Operation`, and a *same-worker* copy
    /// compiled to nothing (the LHS array stayed at its allocation
    /// default — a silent wrong answer; the `link`-layer
    /// `MissingCrossWorkerTransfer` check only catches the *cross-worker*
    /// case). The fix-loud guard converts that silent drop into this
    /// typed error.
    ///
    /// The actionable workaround (and the canonical v2 surface for a
    /// data move today) is an explicit identity kernel, exactly as
    /// `15-transpose` uses `kernel xpose : (i32) -> i32 pure`. A
    /// first-class kernel-less data-move IR node is deferred — see
    /// TASK-0360's closure / its re-open trigger.
    ///
    /// Carries the LHS data symbol and the offending RHS expression
    /// verbatim so the diagnostic is actionable without re-reading the
    /// source.
    KernelLessDataflowRhs {
        /// The data symbol being assigned (e.g. `c`).
        lhs: String,
        /// The offending non-kernel-call RHS expression, verbatim.
        rhs: IrExpr,
    },
    /// A `for VAR : LO .. HI until COND { … }` bounded early-exit loop —
    /// the `until` halt clause is parsed + lowered (epic S1,
    /// TASK-0341.02.01.03) but is NOT YET consumable by any downstream
    /// pass / backend. This is the INERT rejection boundary: the surface
    /// syntax, AST node, IR node, and bool-accepting COND lowering all
    /// land, but the loop is rejected HERE — the first pre-mediation pass
    /// (`build_acfg`) — with this typed error naming the epic, BEFORE any
    /// analysis pass (block/partition/halo/reuse/sync/transfer/sidecar)
    /// observes the `until` field. That ordering is what makes the
    /// `{ var, .., }`-ignoring downstream matches sound (epic S1 design
    /// decision; see `crate::pipeline::run_pre_mediation_passes`).
    ///
    /// This is **diagnosable user input**, not a compiler invariant
    /// (a typed error, NOT a `panic!`), structurally analogous to
    /// [`BuildAcfgError::NonConstLoopBound`]: a grammar-legal form the
    /// codegen does not yet support. Carries the loop variable and the
    /// epic id so the diagnostic names where the work is tracked.
    ///
    /// Runtime early-exit (Event break-condition + codegen) is epic S4/S5
    /// work (TASK-0341.02.01.05 / .06); until then every `until`-loop is
    /// rejected here.
    UntilLoopUnsupported {
        /// The loop variable of the rejected `until`-loop (e.g. `i`).
        var: String,
        /// The epic this surface form is tracked under, for the
        /// diagnostic. Always `"TASK-0341.02.01"`.
        epic: &'static str,
    },
    /// A `for..until COND` loop's halt predicate `COND` is not a
    /// relational comparison.
    ///
    /// Epic S4 (TASK-0341.02.01.05.01) makes `for..until` non-inert by
    /// lowering it to a capped `Repeat`, so the halt predicate now
    /// reaches codegen and must be a **bool**-valued expression. The only
    /// bool-valued `IrExpr` in v2 today is [`IrExpr::Compare`](crate::algo::ir::IrExpr::Compare)
    /// (a single relational comparison — epic S2); a plain integer rvalue
    /// (`until x` where `x : i32`), a kernel call, or any non-Compare
    /// expression is NOT bool and is rejected here.
    ///
    /// This is a DELIBERATE pragmatic bool-context gate, not a full bool
    /// type system: the rule is "an `until`-COND must be a relational
    /// comparison", which is exactly the convergence-check shape
    /// (`diff <= tol`). S1 lowered COND through the bool-accepting
    /// `lower_rvalue` WITHOUT a bool gate (harmless then because the whole
    /// loop was rejected by [`BuildAcfgError::UntilLoopUnsupported`]); this
    /// variant closes that S1-left gap. A first-class bool type-checker is
    /// a larger follow-up (file if warranted).
    ///
    /// This is **diagnosable user input**, a typed error, NOT a `panic!`
    /// and NOT a silent accept (the silent-accept affordance is the
    /// `feedback-option-none-skip-arm-silent-drop` anti-pattern). Carries
    /// the loop variable and the offending predicate verbatim.
    UntilCondNotComparison {
        /// The loop variable of the offending `until`-loop (e.g. `t`).
        var: String,
        /// The non-comparison COND expression, verbatim from the IR.
        cond: IrExpr,
    },
}

impl std::fmt::Display for BuildAcfgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildAcfgError::NonConstLoopBound { var, end, expr } => write!(
                f,
                "loop `{var}` has a non-constant {end} bound `{expr:?}`; \
                 loop bounds must evaluate to a compile-time `i64` \
                 constant (PRD §6.2 — only declared `const`s resolve, \
                 not iteration variables, so triangular / \
                 iter-var-dependent bounds like `for j : 0 .. i` are \
                 not expressible in v2). Use a constant bound, or move \
                 the data-dependent extent into a kernel. (First-class \
                 support is future language work — see PRD §6.2.5.)"
            ),
            BuildAcfgError::OverflowingLoopBound {
                var,
                end,
                expr,
                detail,
            } => write!(
                f,
                "loop `{var}` has a {end} bound `{expr:?}` that is a \
                 compile-time constant expression but cannot be folded to \
                 an `i64`: {detail}. The bound must evaluate within `i64` \
                 range and must not divide by zero. Adjust the constant so \
                 it fits (this is NOT the `for j : 0 .. i` non-const case — \
                 the bound here IS constant, it just overflows)."
            ),
            BuildAcfgError::KernelLessDataflowRhs { lhs, rhs } => write!(
                f,
                "dataflow `{lhs} <-- {rhs:?}` has a non-kernel-call RHS; \
                 every dataflow production / data move in Nucleus v2 must \
                 go through a kernel (an Operation carries a non-optional \
                 KernelId, and no schedule directive maps a data symbol to \
                 a worker set). Wrap the move in an explicit kernel — e.g. \
                 an identity passthrough `kernel id : (T) -> T pure` then \
                 `{lhs} <-- id(...)`, exactly as 15-transpose uses `xpose`. \
                 (A first-class kernel-less data-move is deferred — see \
                 TASK-0360.)"
            ),
            BuildAcfgError::UntilLoopUnsupported { var, epic } => write!(
                f,
                "loop `{var}` uses an `until COND` bounded early-exit halt \
                 clause, which is not yet supported: the surface syntax, \
                 AST/IR nodes, and condition lowering land (epic S1), but \
                 runtime early-exit (Event break-condition + backend \
                 codegen) is later epic work. This loop is rejected at the \
                 ACFG-build boundary. Use a plain fixed-iteration loop \
                 (`for {var} : LO .. HI {{ … }}`, no `until`) for now. \
                 (Tracked under {epic} — the data-dependent loop-termination \
                 grammar-extension epic.)"
            ),
            BuildAcfgError::UntilCondNotComparison { var, cond } => write!(
                f,
                "loop `{var}` has an `until COND` halt predicate `{cond:?}` \
                 that is not a relational comparison. An `until`-COND must \
                 be a bool-valued expression, and the only bool-valued form \
                 in v2 is a single relational comparison (`a <= b`, `a < b`, \
                 `a == b`, etc.) — exactly the convergence-check shape \
                 (`diff <= tol`). A plain integer (`until x` where `x` is \
                 an i32), a kernel call, or any other non-comparison \
                 expression is not bool. Use a relational comparison as the \
                 halt predicate."
            ),
        }
    }
}

impl std::error::Error for BuildAcfgError {}
