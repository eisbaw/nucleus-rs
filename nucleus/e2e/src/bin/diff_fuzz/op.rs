//! Scalar i32 operators, in two transcriptions.
//!
//! Each operator carries BOTH an in-process reference (`apply`, used by
//! the oracle) and the Rust expression body emitted into the generated
//! `kernels.rs` (`kernel_body`). These are two transcriptions of the SAME
//! operator definition. That is deliberate and is the honest bound on
//! what the in-process reference buys (see the binary crate docstring): a
//! conceptual error in an operator's *definition* would appear identically
//! in both and escape — the reference guards against COMPILER common-mode
//! (all backends mistranslating the same kernel), not against
//! SPECIFICATION common-mode. Keeping `apply` and `kernel_body` adjacent
//! makes that audit one screenful.

use crate::rng::Rng;

/// One scalar `(i32, i32) -> i32` op used in elementwise/stencil stages.
/// `apply` MUST match the Rust body emitted by `kernel_body` exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    BitAnd,
    BitOr,
    BitXor,
    Min,
    Max,
    /// affine: `x * k + m` on the FIRST argument (the second is ignored so
    /// the stage signature stays uniformly `(i32, i32) -> i32`).
    Affine(i32, i32),
}

impl Op {
    const SIMPLE: [Op; 8] = [
        Op::WrappingAdd,
        Op::WrappingSub,
        Op::WrappingMul,
        Op::BitAnd,
        Op::BitOr,
        Op::BitXor,
        Op::Min,
        Op::Max,
    ];

    /// In-process reference. `wrapping_*` so overflow is two's-complement
    /// deterministic, matching the emitted kernel bodies (PRD §10.1).
    pub(crate) fn apply(&self, x: i32, y: i32) -> i32 {
        match self {
            Op::WrappingAdd => x.wrapping_add(y),
            Op::WrappingSub => x.wrapping_sub(y),
            Op::WrappingMul => x.wrapping_mul(y),
            Op::BitAnd => x & y,
            Op::BitOr => x | y,
            Op::BitXor => x ^ y,
            Op::Min => x.min(y),
            Op::Max => x.max(y),
            Op::Affine(k, m) => x.wrapping_mul(*k).wrapping_add(*m),
        }
    }

    /// The Rust expression body for a kernel `fn(a: i32, b: i32) -> i32`.
    /// Identical arithmetic to `apply` — the single source of truth is the
    /// op variant.
    pub(crate) fn kernel_body(&self) -> String {
        match self {
            Op::WrappingAdd => "a.wrapping_add(b)".to_string(),
            Op::WrappingSub => "a.wrapping_sub(b)".to_string(),
            Op::WrappingMul => "a.wrapping_mul(b)".to_string(),
            Op::BitAnd => "a & b".to_string(),
            Op::BitOr => "a | b".to_string(),
            Op::BitXor => "a ^ b".to_string(),
            Op::Min => "a.min(b)".to_string(),
            Op::Max => "a.max(b)".to_string(),
            // `b` is named in the signature but unused for affine; the
            // fixed signature forbids `_b`, so reference it in a no-op to
            // avoid an unused-variable warning in the generated crate.
            Op::Affine(k, m) => {
                format!("{{ let _ = b; a.wrapping_mul({k}).wrapping_add({m}) }}")
            }
        }
    }

    pub(crate) fn random(rng: &mut Rng) -> Op {
        // ~1-in-3 chance of an affine op (with random constants); else a
        // simple two-arg op.
        if rng.chance(1, 3) {
            Op::Affine(rng.i32_value(), rng.i32_value())
        } else {
            *rng.choice(&Op::SIMPLE)
        }
    }
}

/// A reduction combine operator: an ASSOCIATIVE + COMMUTATIVE binary
/// `(i32, i32) -> i32` op together with its IDENTITY element, so an
/// empty partition / empty bin folds to a well-defined value.
///
/// All six required by the task: sum, or, xor, min, max, and. Identities:
///   sum -> 0, or -> 0, xor -> 0, and -> -1 (all-ones), min -> i32::MAX,
///   max -> i32::MIN.
///
/// Determinism note: every op here is associative AND commutative over
/// `i32`, so a partitioned tree-reduce produces the SAME result regardless
/// of worker count or fold order — which is exactly what lets the
/// distributed schedule be byte-identical to a sequential reference
/// (PRD §10.1: no floating point, so no reordering hazard).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CombineOp {
    Sum,
    Or,
    Xor,
    And,
    Min,
    Max,
}

impl CombineOp {
    pub(crate) const ALL: [CombineOp; 6] = [
        CombineOp::Sum,
        CombineOp::Or,
        CombineOp::Xor,
        CombineOp::And,
        CombineOp::Min,
        CombineOp::Max,
    ];

    /// The identity element: `combine(identity, x) == x` for all `x`. This
    /// is the value the codegen-visible accumulator slot must start at, and
    /// the value an EMPTY reduction must return. The min/max identities are
    /// the deliberate edge case the task calls out (min identity = type
    /// max, max identity = type min).
    pub(crate) fn identity(&self) -> i32 {
        match self {
            CombineOp::Sum => 0,
            CombineOp::Or => 0,
            CombineOp::Xor => 0,
            CombineOp::And => -1, // all-ones; x & -1 == x
            CombineOp::Min => i32::MAX,
            CombineOp::Max => i32::MIN,
        }
    }

    /// In-process reference for the binary combine step.
    pub(crate) fn apply(&self, a: i32, b: i32) -> i32 {
        match self {
            CombineOp::Sum => a.wrapping_add(b),
            CombineOp::Or => a | b,
            CombineOp::Xor => a ^ b,
            CombineOp::And => a & b,
            CombineOp::Min => a.min(b),
            CombineOp::Max => a.max(b),
        }
    }

    /// A short stable name for the failure report / program description.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            CombineOp::Sum => "sum",
            CombineOp::Or => "or",
            CombineOp::Xor => "xor",
            CombineOp::And => "and",
            CombineOp::Min => "min",
            CombineOp::Max => "max",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_apply_matches_intent() {
        assert_eq!(Op::WrappingAdd.apply(2, 3), 5);
        assert_eq!(Op::WrappingMul.apply(i32::MAX, 2), -2); // two's-complement wrap
        assert_eq!(Op::BitXor.apply(0b1010, 0b0110), 0b1100);
        assert_eq!(Op::Min.apply(-5, 3), -5);
        assert_eq!(Op::Affine(3, 1).apply(4, 999), 13); // 4*3+1, second arg ignored
    }

    #[test]
    fn affine_kernel_body_references_b() {
        // Guards the unused-variable warning fix in the generated crate.
        let body = Op::Affine(2, 3).kernel_body();
        assert!(body.contains("let _ = b"));
    }

    #[test]
    fn combine_identities_are_neutral() {
        // For every combine op and a spread of values, the identity must
        // be a true left/right neutral element. This is the property the
        // empty-bin / accumulator pre-init relies on.
        let probes = [i32::MIN, -7, -1, 0, 1, 42, i32::MAX];
        for op in CombineOp::ALL {
            let id = op.identity();
            for &x in &probes {
                assert_eq!(op.apply(id, x), x, "{:?} left-identity on {x}", op);
                assert_eq!(op.apply(x, id), x, "{:?} right-identity on {x}", op);
            }
        }
    }

    #[test]
    fn min_max_identities_are_type_extremes() {
        // The explicit edge case the task names.
        assert_eq!(CombineOp::Min.identity(), i32::MAX);
        assert_eq!(CombineOp::Max.identity(), i32::MIN);
    }

    #[test]
    fn combine_is_associative_and_commutative_on_samples() {
        // Not a proof, but a tripwire: a transcription slip that broke
        // commutativity/associativity would corrupt the distributed-vs-
        // sequential byte-identity, so pin it on representative values.
        let vals = [3i32, -8, 17, i32::MIN, i32::MAX, 0, -1];
        for op in CombineOp::ALL {
            for &a in &vals {
                for &b in &vals {
                    assert_eq!(op.apply(a, b), op.apply(b, a), "{:?} comm", op);
                    for &c in &vals {
                        assert_eq!(
                            op.apply(op.apply(a, b), c),
                            op.apply(a, op.apply(b, c)),
                            "{:?} assoc",
                            op
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn combine_names_are_stable() {
        // The names feed the algo `combine=<name>` attribute and the
        // failure report; pin them.
        assert_eq!(CombineOp::Sum.name(), "sum");
        assert_eq!(CombineOp::Or.name(), "or");
        assert_eq!(CombineOp::Xor.name(), "xor");
        assert_eq!(CombineOp::And.name(), "and");
        assert_eq!(CombineOp::Min.name(), "min");
        assert_eq!(CombineOp::Max.name(), "max");
    }
}
