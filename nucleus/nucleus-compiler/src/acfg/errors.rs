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
        }
    }
}

impl std::error::Error for BuildAcfgError {}
