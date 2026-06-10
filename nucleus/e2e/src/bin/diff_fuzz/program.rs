//! The generated-program model: a family-tagged synthesised program plus
//! the shared scaffolding every family emits.
//!
//! # Families (the synthesised subclass)
//!
//! TASK-0453.01 shipped exactly ONE family (1-D elementwise pipeline).
//! TASK-0455.05 + TASK-0453.01.01 widen the subclass to five, each a
//! structured, provably-compilable shape modelled on a proven curated
//! example:
//!
//!   * [`Family::Pipeline1d`]  — 1-D elementwise integer pipeline, host+w0
//!     split (`02-split-add`). 7-backend.
//!   * [`Family::Stencil2d`]   — 2-D 3-point stencil with neighbour reads
//!     that FORCE halo inference, row-band `partition=rows`, plain `sync`
//!     transfers (`05-stencil`, sync variant). 7-backend.
//!   * [`Family::Reduction`]   — partitioned two-phase reduction over a
//!     combine operator with an identity element (`03-reduction`
//!     distributed). 7-backend. Covers all six combine ops incl. the
//!     min/max type-extreme identity edge case and the empty-bin case.
//!   * [`Family::PartitionWorkers`] — multi-COMPUTE-worker elementwise map
//!     under `place ... on {w0,w1,...}` / `loop ... : partition=workers`
//!     (owned by TASK-0453.01.01). 7-backend.
//!   * [`Family::ForUntil1d`]  — bounded `for..until` single-worker shape
//!     (cap + EXACT integer halt predicate). pthreads-sync ONLY — see
//!     [`Program::backends`].
//!
//! # Honest residual on backend coverage
//!
//! Four of the five families compile + run byte-identically across all
//! SEVEN tier-1 backends. The `for..until` family does NOT — by the
//! GENERATOR's own deliberate narrowness, not a construct gap: since S7
//! (TASK-0341.02.01.08) the curated matrix runs single-worker
//! `for..until` required on all seven backends, but this generative
//! family stays pinned to pthreads-sync until the generator composes
//! the same shapes the curated cells pin (see the UNTIL_BACKENDS
//! docstring below). So the for..until family is checked for
//! self-consistency + reference agreement on its pinned backend, not
//! for 7-way identity; the per-family backend set
//! ([`Program::backends`]) encodes the truth.

use crate::rng::Rng;

use crate::family::{partition, pipeline, reduction, stencil, until};

/// The set of tier-1 backends a generated program is checked against, and
/// whether each emits a single binary (shared-memory) vs a `run.sh`
/// launcher (multi-process). Mirrors the e2e harness's
/// `transport == "shared-memory"` single-binary rule.
pub(crate) const ALL_BACKENDS: [(&str, bool); 7] = [
    ("pthreads-sync", true),
    ("pthreads-async", true),
    ("openmp-rs", true),
    ("mp-tcp-bufsync", false),
    ("mp-tcp-event", false),
    ("mp-tcp-poll", false),
    ("mp-uds-event", false),
];

/// The single backend the `for..until` family is checked on. Since S7
/// (TASK-0341.02.01.08) the curated matrix runs single-worker
/// `for..until` on ALL SEVEN tier-1 backends — the shared
/// single_worker_main emitter carries it — but this GENERATIVE family
/// deliberately stays narrower (one backend) until the generator
/// composes the same shapes the curated cells pin; widening it is part
/// of the family's own growth plan, not a matrix mirror.
pub(crate) const UNTIL_BACKENDS: [(&str, bool); 1] = [("pthreads-sync", true)];

/// The synthesised family of a generated program. Each variant carries the
/// fully-resolved parameters drawn from the seed, so emission is a pure
/// function of the variant (determinism-in-seed).
#[derive(Clone, Debug)]
pub(crate) enum Family {
    Pipeline1d(pipeline::Pipeline1d),
    Stencil2d(stencil::Stencil2d),
    Reduction(reduction::Reduction),
    PartitionWorkers(partition::PartitionWorkers),
    ForUntil1d(until::ForUntil1d),
}

/// A generated program: a family plus the per-program reproducer seed.
pub(crate) struct Program {
    /// RNG-state snapshot taken immediately before this program was drawn.
    /// `--prog-seed <this>` regenerates exactly this program.
    pub(crate) seed: u64,
    pub(crate) family: Family,
}

/// The four source artefacts + input a family emits, ready to write to a
/// scratch dir. Keeping this as a struct lets every family share the
/// write/run path in `backend.rs` regardless of shape.
pub(crate) struct SourceBundle {
    pub(crate) algo: String,
    pub(crate) sched: String,
    pub(crate) kernels: String,
    pub(crate) input: Vec<u8>,
    /// The in-process reference output (the second transcription oracle).
    pub(crate) reference: Vec<u8>,
}

impl Program {
    /// Draw one program from the stream. The family is chosen first (so a
    /// fixed seed maps to a fixed family), then the family's parameters.
    pub(crate) fn generate(seed: u64, rng: &mut Rng) -> Program {
        // Family weights: keep the cheap, broad 7-backend families common;
        // the for..until family is single-backend, so it is rarer but
        // present. Order is FIXED — appending a family must go at the end
        // to preserve seed->program reproduction for existing seeds.
        let family = match rng.range(0, 99) {
            0..=29 => Family::Pipeline1d(pipeline::Pipeline1d::generate(rng)),
            30..=49 => Family::Stencil2d(stencil::Stencil2d::generate(rng)),
            50..=74 => Family::Reduction(reduction::Reduction::generate(rng)),
            75..=89 => Family::PartitionWorkers(partition::PartitionWorkers::generate(rng)),
            _ => Family::ForUntil1d(until::ForUntil1d::generate(rng)),
        };
        Program { seed, family }
    }

    /// Build the source bundle. Pure in the family params.
    pub(crate) fn bundle(&self) -> SourceBundle {
        match &self.family {
            Family::Pipeline1d(f) => f.bundle(self.seed),
            Family::Stencil2d(f) => f.bundle(self.seed),
            Family::Reduction(f) => f.bundle(self.seed),
            Family::PartitionWorkers(f) => f.bundle(self.seed),
            Family::ForUntil1d(f) => f.bundle(self.seed),
        }
    }

    /// One-line human-readable description for the progress line + report.
    pub(crate) fn describe(&self) -> String {
        let body = match &self.family {
            Family::Pipeline1d(f) => f.describe(),
            Family::Stencil2d(f) => f.describe(),
            Family::Reduction(f) => f.describe(),
            Family::PartitionWorkers(f) => f.describe(),
            Family::ForUntil1d(f) => f.describe(),
        };
        format!("seed={} {}", self.seed, body)
    }

    /// The (name, single_binary) backends this program is checked against.
    /// All families are 7-backend EXCEPT `for..until`, which is
    /// pthreads-sync only (see the module docstring).
    pub(crate) fn backends(&self) -> &'static [(&'static str, bool)] {
        match &self.family {
            Family::ForUntil1d(_) => &UNTIL_BACKENDS,
            _ => &ALL_BACKENDS,
        }
    }
}

// ----------------------------------------------------------------------
// Shared little helpers used by more than one family.
// ----------------------------------------------------------------------

/// Append i32 values to `bytes` in little-endian layout — the byte layout
/// every generated kernel reads/writes.
pub(crate) fn push_i32_le(bytes: &mut Vec<u8>, vals: &[i32]) {
    for v in vals {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_family_and_program() {
        let mut r1 = Rng::new(123);
        let p1 = Program::generate(123, &mut r1);
        let mut r2 = Rng::new(123);
        let p2 = Program::generate(123, &mut r2);
        // Same family discriminant + identical emitted sources.
        assert_eq!(
            std::mem::discriminant(&p1.family),
            std::mem::discriminant(&p2.family)
        );
        let b1 = p1.bundle();
        let b2 = p2.bundle();
        assert_eq!(b1.algo, b2.algo);
        assert_eq!(b1.sched, b2.sched);
        assert_eq!(b1.kernels, b2.kernels);
        assert_eq!(b1.input, b2.input);
        assert_eq!(b1.reference, b2.reference);
    }

    #[test]
    fn until_family_is_single_backend_rest_are_seven() {
        // Drive enough seeds to hit every family, asserting the backend
        // set invariant per family.
        let mut seen_until = false;
        let mut seen_multi = false;
        for s in 0..400u64 {
            let mut r = Rng::new(s);
            let p = Program::generate(s, &mut r);
            match &p.family {
                Family::ForUntil1d(_) => {
                    assert_eq!(p.backends().len(), 1);
                    assert_eq!(p.backends()[0].0, "pthreads-sync");
                    seen_until = true;
                }
                _ => {
                    assert_eq!(p.backends().len(), 7);
                    seen_multi = true;
                }
            }
        }
        assert!(seen_until, "no for..until family drawn in 400 seeds");
        assert!(seen_multi, "no 7-backend family drawn in 400 seeds");
    }
}
