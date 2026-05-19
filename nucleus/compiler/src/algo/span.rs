//! Per-node source-span wrapper for the algorithm AST (TASK-0082).
//!
//! # Why this exists
//!
//! Diagnostics that point at user source ("undeclared `foo` at
//! line 7, column 3", "duplicate kernel `k`", a malformed expression)
//! need every diagnosable AST node to remember the byte range of the
//! source text it was parsed from. [`crate::algo::ParseError`] already
//! carries a position, but only for the *first* syntactic failure;
//! semantic passes (lowering, link) have no position to attach to a
//! node they reject. This wrapper is the substrate those passes build
//! on. Using the wrapped span in `LowerError` is TASK-0090; this task
//! only adds and *populates* the substrate — lowering still ignores
//! spans (proven by the determinism gate staying byte-identical).
//!
//! # Span representation
//!
//! The span is a byte [`core::ops::Range<usize>`] into the original
//! source string, exactly matching what `chumsky` 0.9 hands to
//! `.map_with_span` for `char` input and what
//! [`crate::error::offset_to_line_col`] consumes. Keeping one
//! representation end-to-end means no lossy conversions: a node's
//! `span.start` fed to `offset_to_line_col` yields the `(line, column)`
//! a user-facing diagnostic prints.
//!
//! # Equality semantics (load-bearing — AC#2)
//!
//! [`PartialEq`], [`Eq`], [`Hash`] and [`std::fmt::Debug`] are
//! **manually implemented to forward to `self.node` only**; the span
//! is deliberately EXCLUDED. This is not a stylistic choice: the
//! existing AST-equality / structural tests (`tests/algo_parser.rs`,
//! and any future expected-AST literal) compare *parsed* trees against
//! *hand-built* expected trees. A hand-built tree cannot know byte
//! offsets, so if equality included the span every such test would
//! break. Excluding the span keeps two structurally-identical programs
//! equal regardless of where in the source they were written.
//! `#[derive(PartialEq)]` would (wrongly) include the span — hence the
//! hand-written impls below.
//!
//! # Granularity (the deliberate, bounded set)
//!
//! Nodes are wrapped at exactly the granularity diagnostics must point
//! at, and no finer:
//!
//! - `Item` — a malformed / duplicate top-level declaration or
//!   statement points at the whole item.
//! - `Stmt` (including every nested `for`-body statement) — a rejected
//!   statement points at the statement.
//! - `Expr` (every recursive position: unary/binary operands, call
//!   arguments, index expressions, shape dimensions, `for` bounds) —
//!   a bad / type-wrong / undeclared-reference expression points at
//!   the sub-expression.
//! - The identifier-bearing `String` fields a diagnostic names —
//!   `ConstDecl.name`, `DataDecl.name`, `KernelDecl.name`,
//!   `IndexedLValue.name`, `Call.callee`, `Stmt::For.var` — become
//!   `Spanned<String>` so an "undeclared / duplicate `X`" error can
//!   underline the *identifier token itself*, not the enclosing line.
//!
//! Deliberately NOT wrapped, to bound the AST/parser blast radius:
//! `ScalarType`, `Purity`, `UnaryOp`, `BinOp` (`Copy` leaves that are
//! never independently diagnosed), and the `Type` / `KernelSig`
//! structural containers (their diagnosable content — a bad shape
//! dimension `Spanned<Expr>`, the owning declaration's name span — is
//! already reachable; a malformed signature still points via its
//! item / decl-name span). Under-wrapping would lose a needed
//! diagnostic site; over-wrapping bloats every node and parser
//! combinator for sites no error references. This set is the minimum
//! that lets a future error point precisely at an undeclared/duplicate
//! identifier, a bad expression, or a malformed declaration/statement.

use core::ops::{Deref, DerefMut, Range};

/// A value of type `T` plus the byte [`Range`] of source it was parsed
/// from.
///
/// Equality / hashing / debug formatting forward to `node` only; the
/// `span` is metadata and is excluded so structural comparisons (and
/// the existing AST-equality tests) are unaffected by source position.
/// See the module docs for the full rationale (AC#2).
pub struct Spanned<T> {
    /// The wrapped AST node.
    pub node: T,
    /// Byte range `start..end` into the original source string. Feed
    /// `span.start` to [`crate::error::offset_to_line_col`] for a
    /// 1-based `(line, column)`.
    pub span: Range<usize>,
}

impl<T> Spanned<T> {
    /// Wrap `node` with the source `span`. Infallible by construction;
    /// no user-reachable failure path here (decision-0003: nothing to
    /// turn into a typed error — span capture cannot fail).
    pub fn new(node: T, span: Range<usize>) -> Self {
        Self { node, span }
    }

    /// Apply `f` to the inner node, preserving the span. Useful in the
    /// parser when an already-spanned sub-result is rewrapped into a
    /// larger node that shares the same source range.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }
}

// `Deref`/`DerefMut` to `T` keep call sites that only read the inner
// node terse (`spanned.some_field` instead of `spanned.node.some_field`)
// and bound the mechanical churn in `lower.rs` / the parser tests. This
// does NOT hide intent: `.span` is still an explicit field access, and
// pattern-matching the inner enum still goes through `.node` (Deref does
// not apply to `match`), so a site that needs the span cannot get it by
// accident.
impl<T> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.node
    }
}

impl<T> DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.node
    }
}

// --- Manual trait impls: forward to `node`, EXCLUDE `span` (AC#2) ---
//
// These are hand-written precisely so the span is NOT part of value
// identity. Deriving any of them would fold `span` into the comparison
// / hash / debug output and break every existing structural test.

impl<T: PartialEq> PartialEq for Spanned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
    }
}

impl<T: Eq> Eq for Spanned<T> {}

impl<T: core::hash::Hash> core::hash::Hash for Spanned<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.node.hash(state);
    }
}

// Debug forwards transparently to the node so test failure output and
// `{:?}` dumps read exactly as they did before wrapping (the existing
// parser tests print nodes with `{:?}` in panic messages — keeping the
// representation identical avoids gratuitous churn and keeps those
// messages honest).
impl<T: core::fmt::Debug> core::fmt::Debug for Spanned<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&self.node, f)
    }
}

impl<T: Clone> Clone for Spanned<T> {
    fn clone(&self) -> Self {
        Spanned {
            node: self.node.clone(),
            span: self.span.clone(),
        }
    }
}

// Serde: transparent over the node so that, if a wrapped type ever
// becomes serde-derived (none are today — the serialised codegen
// contract uses IR types, not the algo AST), the span is simply not
// part of the wire form, consistent with it not being part of value
// identity. Feature-gated identically to the rest of the crate
// (`serde` is on by default; see compiler/Cargo.toml). No behaviour
// change: adds trait impls only under the feature.
#[cfg(feature = "serde")]
impl<T: serde::Serialize> serde::Serialize for Spanned<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.node.serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Spanned<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // No span in the wire form; default to an empty range. A
        // deserialised AST has no source text to point at anyway.
        Ok(Spanned {
            node: T::deserialize(deserializer)?,
            span: 0..0,
        })
    }
}
