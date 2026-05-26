//! Link step: cross-reference resolution between AlgoIR and SchedIR.
//!
//! This is the first compiler pass that sees BOTH IRs in hand. It
//! validates every name the schedule borrows from the algorithm and
//! every coverage obligation the schedule has against the algorithm
//! (every kernel placed, every cross-worker dataflow has a transfer).
//!
//! See PRD §5 (link step in the pipeline diagram), §6.3.2 (every
//! kernel must have exactly one `place`), §6.3.4 (cross-worker
//! transfers must be declared), and §12 (algorithm changes that break
//! schedules must surface here as named errors).
//!
//! Design choices, called out so they aren't surprises later:
//!
//! - **Collect all errors in one pass.** PRD §12 emphasises that
//!   silent or one-at-a-time failure modes for the algorithm-vs-
//!   schedule contract make the two files dishonest with each other.
//!   We walk the schedule once, gather every dangling reference and
//!   every missing-transfer obligation, then return them as a `Vec`.
//!   No fail-fast.
//!
//! - **Distributed placements treated as a single worker entity for
//!   transfer inference.** A kernel `place k on { w0, w1, w2, w3 }`
//!   is, from the perspective of cross-worker transfer, ONE entity:
//!   when producer and consumer are both placed on the same
//!   `{w0..w3}` set, no transfer is required (the per-element
//!   decomposition is handled in downstream passes: TASK-0016 lands
//!   the ACFG layer, TASK-0117 the Push/Wait replication across the
//!   set, and TASK-0258 / TASK-0259 / TASK-0249 the per-partition=
//!   slicing). This matches `13-cnn-inference/batch_parallel.sched.nuc`
//!   where `feat1`/`feat2` move within the `{w0..w3}` set with no
//!   transfer directive, but `input` and `output` (between `host` and
//!   the set) do carry transfer directives.
//!
//! - **Producer/consumer derived from AlgoIR statements + SchedIR
//!   placements.** The producer of data symbol `D` is the placement
//!   of the kernel on the RHS of a `Dataflow { lhs: D, rhs: Call }`.
//!   Consumers are the placements of kernels that read `D` either as
//!   a `Call` argument or as an `Effect` argument. Identity copies
//!   (`D <-- E` where the RHS is a bare `DataRef`, no kernel) are
//!   not in the current examples; we treat them as "no kernel
//!   involved, no producer worker recorded" and file a follow-up.
//!
//! - **Fuzzy-match "did you mean?" suggestions for typos.** The four
//!   unknown-name errors (`UnknownKernel`/`UnknownData`/`UnknownLoop`/
//!   `UnknownTransferData`) carry an `Option<String>` did-you-mean
//!   suggestion: the closest algorithm-side symbol within a bounded
//!   edit distance, computed via the zero-dep [`crate::error::suggest`]
//!   helper against the in-hand symbol table for that variant
//!   (kernels / data / loop vars). The suggestion is a deterministic
//!   pure function of (offending name, table) — see [`LinkError`] and
//!   the helper's docs for the threshold and tie-break (TASK-0096).
//!
//! What this pass explicitly DOES NOT do:
//!
//! - Validate `partition=` policies against placement cardinality —
//!   genuinely still deferred (no current consumer; filed only as an
//!   inline limitation in this module today).
//! - Detect data symbols that have no producer at all (could be a
//!   genuine bug; not in the spec for this task).
//!
//! What this pass also does not do, but where the work has landed in
//! adjacent passes (so the "deferred" framing would mislead a reader):
//!
//! - Type-check kernel signatures against call sites: addressed by the
//!   contract pass (`crate::contract`, TASK-0012) and the link step's
//!   own `UnknownKernel`/`UnknownData` checks; TASK-0088 closed as
//!   ADDRESSED in-large-part.
//! - Per-worker slicing of distributed placements: addressed by the
//!   `partition_workers` / `partition_rows` / `partition_blocks2d`
//!   passes downstream (TASK-0117 + TASK-0258 + TASK-0259, all Done).
//! - Resolve transfer/notify semantics against the backend capability
//!   matrix: addressed by `crate::capabilities` (TASK-0019, Done).

pub mod build;
pub mod dataflow;
pub mod errors;
pub mod pipeline;
pub mod types;

pub use build::link;
pub use errors::{LinkError, LinkErrorKind, LinkErrorSource};
pub use types::{LinkedIR, WorkerEntity};
