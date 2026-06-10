//! SchedIR: the semantically-validated intermediate representation of a
//! schedule program.
//!
//! Produced by [`crate::sched::lower_sched`] from a [`SchedAst`](crate::sched::ast::SchedAst)
//! (TASK-0008 output). Compared to the AST, the IR:
//!
//! - Resolves the typed-vs-simple worker form into a single shape:
//!   every [`ResolvedWorker`] carries a `class` name. Simple-form
//!   entries are bound to the synthetic class
//!   [`DEFAULT_WORKER_CLASS`], which is auto-injected into
//!   [`SchedIR::worker_classes`] only when at least one simple-form
//!   entry appears.
//! - Validates that every `class` named on a worker entry has a
//!   matching [`ResolvedWorkerClass`] declaration; same for every
//!   [`ResolvedMemoryRegion`] referenced from `place_data`.
//! - Validates numeric option ranges: `block=N`/
//!   `unroll=N`/`pipeline=N` on loops, `buffer=N` on transfers, all
//!   must be strictly positive. The parser accepts any `u64` because
//!   zero-or-positive is a semantic constraint, not a lexical one.
//! - Validates uniqueness:
//!   * no two `worker_class` decls with the same name,
//!   * no two `memory_region` decls with the same name,
//!   * no two `worker` entries (across all `workers = ...` decls and
//!     within a single decl) with the same name,
//!   * at most one `place` per kernel,
//!   * at most one `transfer` per data symbol,
//!   * at most one `check loop` per loop variable.
//! - Bucketed lookup. Declarations are indexed by name for O(1)
//!   resolution by later passes.
//!
//! What this IR explicitly does NOT do (deferred to TASK-0011 link
//! step, which is the first pass that has BOTH the algo IR and the
//! sched IR in hand):
//!
//! - Kernel-name resolution. A `place foo on ...` whose `foo` isn't
//!   a declared kernel in the algorithm is undetectable here because
//!   we don't see the algorithm.
//! - Data-symbol resolution. `place_data D in R` validates that `R`
//!   is a declared memory region but leaves `D` as a textual name.
//!   `transfer D : ...` likewise. The link step intersects these
//!   against `AlgoIR::data` and rejects unknowns.
//! - Loop-variable resolution. `loop x : ...` and `check loop x : ...`
//!   both name an algorithm-side iteration variable; whether it
//!   exists is a link-step check.
//! - Completeness checks: every kernel has a place; every
//!   cross-worker data symbol has a transfer. The first needs the
//!   algorithm's kernel set; the second needs the algorithm's
//!   placement-by-kernel + data-flow graph. Link step.
//! - Capability-matrix validation (e.g. `async` on a sync-only
//!   backend, `notify=event` on a poll-only backend). Done as the
//!   driver's capability gate (`check_schedule_compat`, TASK-0019),
//!   which runs once the backend's `capabilities.toml` is in hand.
//!
//! Design choice — separate IR types vs annotated AST: separate, same
//! reasoning as the algorithm side (`crate::algo::ir`). The AST keeps
//! the surface forms distinct (simple vs typed workers; options as
//! flat vectors); the IR collapses them and indexes them. Annotating
//! the AST in place would force every downstream pass to keep handling
//! "is this resolved yet?" cases.

use core::ops::Range;
use std::collections::BTreeMap;

use super::ast::{
    MemorySpec, NotifyKind, PartitionKind, SimdSpec, TimeLit, TransportMode, ViolationKind,
};

/// Synthetic class name used for simple-form worker entries
/// (`workers = { host, w0 }` — no explicit class per entry).
///
/// Chosen with a leading `__` so it cannot collide with a user-written
/// identifier: the parser's identifier rule (see `sched/parser.rs`)
/// permits `_` as a leading character, but writing `__default_class`
/// in source is allowed. The name is namespaced enough that an
/// accidental collision is extremely unlikely; if a real example
/// declares a class with this exact name, the lowering pass will
/// reject the collision via [`SchedLowerErrorKind::DuplicateWorkerClass`].
/// That's the safest failure mode — loud, not silent.
pub const DEFAULT_WORKER_CLASS: &str = "__default";

// --------------------------------------------------------------------
// Worker topology
// --------------------------------------------------------------------

/// Resolved worker-class declaration. Same shape as
/// [`super::ast::WorkerClassDecl`]; the lowering pass copies the AST
/// payload through and indexes it by name.
///
/// The `simd` and `memory` fields are kept as `Option` because the
/// grammar permits an empty class body (`worker_class X {};`). Whether
/// downstream backends require both fields is a capability-matrix
/// concern (handled by `crate::capabilities`, TASK-0019), not a
/// lowering concern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorkerClass {
    pub name: String,
    pub simd: Option<SimdSpec>,
    pub memory: Option<MemorySpec>,
    /// True if this class was synthesised for the simple worker form
    /// (i.e. there was no `worker_class` declaration with this name in
    /// the source). Useful for diagnostics and for the backend to
    /// know it has free rein over the implementation.
    pub is_default: bool,
}

/// Resolved memory-region declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMemoryRegion {
    pub name: String,
    pub size_bytes: Option<u64>,
    pub accessible_by: Option<Vec<String>>,
    pub per_worker: Option<bool>,
}

/// Resolved worker: a name and the class it belongs to.
///
/// Simple-form entries are bound to [`DEFAULT_WORKER_CLASS`]; typed-form
/// entries keep the class name they were written with. This collapse is
/// the core simplification of SchedIR vs the AST: downstream code
/// branches on the class (e.g. "what SIMD does this worker have?")
/// rather than on the surface syntactic form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWorker {
    pub name: String,
    pub class: String,
}

// --------------------------------------------------------------------
// Placement
// --------------------------------------------------------------------

/// Resolved `place KERNEL on TARGET;`.
///
/// `target` keeps the AST's One/Many shape because the distinction is
/// semantically meaningful (a `Many` target requests partitioning,
/// which the loop directive's `partition=` option configures). The
/// worker-name strings are NOT cross-checked against the declared
/// [`SchedIR::workers`] at this stage because the grammar does not
/// require the `workers = ...` directive to precede `place` — and the
/// task spec scopes link-time checks (kernel resolution) for later
/// anyway. We DO at least require that the named workers are declared,
/// because a `place X on bogus_worker;` is a clear schedule-side bug.
///
/// # `kernel_span` (TASK-0099)
///
/// Byte range of the schedule's `place K on ...` *kernel identifier*
/// token (`PlaceDirective.kernel.span`), threaded through `lower_place`
/// for use by the link step. Populated by `sched/lower.rs`; `None` if
/// this `ResolvedPlacement` was constructed manually in a test that has
/// no source text to point at. Used by [`crate::link::LinkError`]
/// (`UnknownKernel`) to underline the offending identifier in the
/// schedule. **Excluded from value identity** (hand-written `PartialEq`
/// forwards to `kernel` + `target` only) — same rationale as
/// [`crate::span::Spanned`] / [`crate::algo::ir::LowerError`]: positions
/// are informational-for-humans, not part of *which placement this is*.
#[derive(Debug, Clone, Eq)]
pub struct ResolvedPlacement {
    pub kernel: String,
    pub target: ResolvedPlaceTarget,
    /// Byte range of the schedule-source kernel identifier token (the
    /// `place K on ...` `K`). `None` for manually-constructed test
    /// instances. See type docs.
    pub kernel_span: Option<Range<usize>>,
}

// Hand-written: forward to `kernel` + `target`, EXCLUDE `kernel_span`
// from identity (TASK-0099, mirroring TASK-0090 / TASK-0082). Deriving
// would fold the span in and (a) break the manual-test struct
// `assert_eq!`s that don't populate spans, (b) make two
// otherwise-identical placements parsed from different sources unequal.
impl PartialEq for ResolvedPlacement {
    fn eq(&self, other: &Self) -> bool {
        self.kernel == other.kernel && self.target == other.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPlaceTarget {
    One(String),
    Many(Vec<String>),
}

/// Resolved `place_data DATA in REGION;`.
///
/// The `region` field has been validated to refer to a declared
/// [`ResolvedMemoryRegion`]. The `data` field is a textual symbol
/// kept for the link step to cross-check against `AlgoIR::data`.
///
/// # `data_span` (TASK-0099)
///
/// Byte range of the schedule's `place_data D in R` *data identifier*
/// token, threaded through `lower_place_data` for the link step's
/// `UnknownData` diagnostic. See [`ResolvedPlacement`] for the
/// equality/identity rationale (excluded from `PartialEq`).
#[derive(Debug, Clone, Eq)]
pub struct ResolvedPlaceData {
    pub data: String,
    pub region: String,
    /// Byte range of the schedule-source data identifier token (the
    /// `place_data D in R` `D`). `None` for manually-constructed test
    /// instances.
    pub data_span: Option<Range<usize>>,
}

impl PartialEq for ResolvedPlaceData {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data && self.region == other.region
    }
}

// --------------------------------------------------------------------
// Loop transformations
// --------------------------------------------------------------------

/// Resolved `loop VAR : options;`.
///
/// Options have been validated:
/// - numeric options are strictly positive,
/// - `partition=`, `notify=`, `on_violation=` values are the parser's
///   enum types and need no further validation.
///
/// Duplicate-option detection (e.g. `block=64, block=128`) is NOT done
/// here. The grammar §2 note 5 calls this out as a linker concern, and
/// the existing examples don't exercise it. Filed as a follow-up if a
/// real example does. The AST stores options as a `Vec`, and we
/// preserve that order so the conflict-detection follow-up has the
/// information available.
/// # `var_span` (TASK-0099)
///
/// Byte range of the schedule's `loop V : ...` *loop-variable
/// identifier* token, threaded through `lower_loop` for the link step's
/// `UnknownLoop` / `PipelineExceedsBuffer` /
/// `PipelineExceedsIterationCount` diagnostics. See [`ResolvedPlacement`]
/// for the equality/identity rationale (excluded from `PartialEq`).
#[derive(Debug, Clone, Eq)]
pub struct ResolvedLoopDirective {
    pub var: String,
    pub options: Vec<ResolvedLoopOption>,
    /// Byte range of the schedule-source loop-variable identifier token
    /// (the `loop V : ...` `V`). `None` for manually-constructed test
    /// instances.
    pub var_span: Option<Range<usize>>,
}

impl PartialEq for ResolvedLoopDirective {
    fn eq(&self, other: &Self) -> bool {
        self.var == other.var && self.options == other.options
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedLoopOption {
    /// `block=N`, N > 0.
    Block(u64),
    // `Vectorize(u64)` removed (TASK-0292, 2026-05-25). SIMD
    // vectorisation is delegated to the host Rust compiler + LLVM. See
    // PRD §6.3.3.
    /// `unroll=N`, N > 0.
    Unroll(u64),
    /// `pipeline=N`, N > 0.
    Pipeline(u64),
    /// `reuse` (no value).
    Reuse,
    /// `partition=rows|blocks2d|workers`.
    Partition(PartitionKind),
}

// --------------------------------------------------------------------
// Transfer / IO semantics
// --------------------------------------------------------------------

/// Resolved `transfer DATA : options;`.
///
/// `data` is a textual symbol left for the link step. Options have
/// been validated (`buffer=N` > 0). Sync/Async conflict and duplicate
/// options are linker concerns (grammar §2 note 7); the AST order is
/// preserved.
/// # `data_span` (TASK-0099)
///
/// Byte range of the schedule's `transfer D : ...` *data identifier*
/// token, threaded through `lower_transfer` for the link step's
/// `UnknownTransferData` diagnostic. See [`ResolvedPlacement`] for the
/// equality/identity rationale (excluded from `PartialEq`).
#[derive(Debug, Clone, Eq)]
pub struct ResolvedTransferDirective {
    pub data: String,
    pub options: Vec<ResolvedTransferOption>,
    /// Byte range of the schedule-source data identifier token (the
    /// `transfer D : ...` `D`). `None` for manually-constructed test
    /// instances.
    pub data_span: Option<Range<usize>>,
}

impl PartialEq for ResolvedTransferDirective {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data && self.options == other.options
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTransferOption {
    Sync,
    Async,
    /// `buffer=N`, N > 0.
    Buffer(u64),
    Notify(NotifyKind),
    /// `mode=pio|dma` backend transport-path hint. See [`TransportMode`].
    Transport(TransportMode),
}

// --------------------------------------------------------------------
// Runtime assertions
// --------------------------------------------------------------------

/// Resolved `check loop VAR : asserts;`.
///
/// The `var` is a loop-variable name in the algorithm; whether the
/// algorithm actually has a loop with that variable is a link-step
/// check. Asserts inherit parser-side enum validation.
/// # `var_span` (TASK-0099)
///
/// Byte range of the schedule's `check loop V : ...` *loop-variable
/// identifier* token, threaded through `lower_check` for the link
/// step's `UnknownLoop` diagnostic (which is raised for both `loop` and
/// `check loop` directives). See [`ResolvedPlacement`] for the
/// equality/identity rationale (excluded from `PartialEq`).
#[derive(Debug, Clone, Eq)]
pub struct ResolvedCheckDirective {
    pub var: String,
    pub asserts: Vec<ResolvedCheckAssert>,
    /// Byte range of the schedule-source loop-variable identifier token
    /// (the `check loop V : ...` `V`). `None` for manually-constructed
    /// test instances.
    pub var_span: Option<Range<usize>>,
}

impl PartialEq for ResolvedCheckDirective {
    fn eq(&self, other: &Self) -> bool {
        self.var == other.var && self.asserts == other.asserts
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCheckAssert {
    /// `latency_max = T`, T is already normalised to nanoseconds by
    /// the parser. The original unit is preserved on [`TimeLit`] for
    /// diagnostics.
    LatencyMax(TimeLit),
    OnViolation(ViolationKind),
}

// --------------------------------------------------------------------
// Root IR node
// --------------------------------------------------------------------

/// Root SchedIR node.
///
/// Declarations are bucketed for O(1) lookup by name. `worker_classes`
/// and `memory_regions` use [`BTreeMap`] (sorted-by-name) — schedule
/// programs are small, so the constant factor doesn't matter and the
/// sorted iteration order keeps test assertions deterministic.
///
/// Directive-shaped items (`places`, `place_data`, `loops`,
/// `transfers`, `checks`) are also indexed by their identifying key
/// (kernel/data/var) because the uniqueness check enforced during
/// lowering means a `BTreeMap` is the natural shape. Source order is
/// not preserved on these — schedule semantics don't depend on
/// directive order within a kind (grammar §2 note 2: directives are
/// declarative; order within a kind is informational only).
///
/// The `algo_path` is forwarded from the AST verbatim. The build
/// driver does NOT consume this string — see
/// [`crate::sched::ast::SchedAst::algo_path`] for the contract
/// (TASK-0277 contract-pinned by
/// `algo_path_stored_verbatim_no_resolution` in
/// `tests/sched_parser.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchedIR {
    /// Path to the algorithm file, verbatim from the source.
    /// Forwarded unchanged from [`crate::sched::ast::SchedAst::algo_path`].
    pub algo_path: String,
    /// Resolved worker classes, keyed by class name. Includes the
    /// synthetic [`DEFAULT_WORKER_CLASS`] entry iff any worker uses
    /// the simple form.
    pub worker_classes: BTreeMap<String, ResolvedWorkerClass>,
    /// Resolved memory regions, keyed by region name.
    pub memory_regions: BTreeMap<String, ResolvedMemoryRegion>,
    /// Resolved workers, keyed by worker name. Order-insensitive —
    /// see module docstring.
    pub workers: BTreeMap<String, ResolvedWorker>,
    /// Resolved `place` directives, keyed by kernel name (uniqueness
    /// is enforced).
    pub places: BTreeMap<String, ResolvedPlacement>,
    /// Resolved `place_data` directives, keyed by data symbol name
    /// (uniqueness enforced — a data symbol cannot be placed in two
    /// regions).
    pub place_data: BTreeMap<String, ResolvedPlaceData>,
    /// Resolved `loop` directives, keyed by loop variable.
    pub loops: BTreeMap<String, ResolvedLoopDirective>,
    /// Resolved `transfer` directives, keyed by data symbol.
    pub transfers: BTreeMap<String, ResolvedTransferDirective>,
    /// Resolved `check loop` directives, keyed by loop variable.
    pub checks: BTreeMap<String, ResolvedCheckDirective>,
}

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// The semantic-violation *kind* produced by the schedule lowering
/// pass.
///
/// Each variant names a single semantic violation and carries the
/// payload a diagnostic message needs (the offending name, the owning
/// directive, etc.). The *source position* of the violation is NOT
/// here — it is carried separately on [`SchedLowerError`] so that
/// adding positions did not change any variant's payload shape
/// (TASK-0196, the schedule mirror of the algorithm-side TASK-0090).
/// Equality / Display of a located error forward to this kind; see
/// [`SchedLowerError`] for why position is excluded from value
/// identity. This is the prior `SchedLowerError` enum verbatim — no
/// variant or payload changed — so the existing `sched_lower` negative
/// tests migrate mechanically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedLowerErrorKind {
    // ----- Uniqueness -----
    /// Two `worker_class` decls share a name.
    DuplicateWorkerClass(String),
    /// Two `memory_region` decls share a name.
    DuplicateMemoryRegion(String),
    /// A worker name appears twice — either within one
    /// `workers = ...` decl or across multiple decls.
    DuplicateWorker(String),
    /// More than one `place` for the same kernel.
    DuplicatePlace { kernel: String },
    /// More than one `place_data` for the same data symbol.
    DuplicatePlaceData { data: String },
    /// More than one `loop` directive for the same loop variable.
    DuplicateLoop { var: String },
    /// More than one `transfer` for the same data symbol.
    DuplicateTransfer { data: String },
    /// More than one `check loop` for the same loop variable.
    DuplicateCheck { var: String },

    // ----- Reference resolution -----
    //
    // # The four unknown-name variants carry a did-you-mean suggestion
    //
    // `UnknownWorkerClass` / `UnknownMemoryRegion` /
    // `UnknownPlaceWorker` / `UnknownAccessibleByName` each carry a
    // `suggestion: Option<String>`: the closest declared schedule-side
    // symbol within a bounded edit distance (or `None`), computed by
    // [`crate::error::suggest`] against the in-hand symbol table for
    // that variant (TASK-0198, the schedule-side sibling of the
    // link-step TASK-0096). The field lives on these *kind* variants —
    // NOT on the [`SchedLowerError`] wrapper — deliberately: a
    // suggestion is a deterministic pure function of `(offending name,
    // in-hand table)`, so it is part of *which semantic error this is*
    // and belongs in the derived-`PartialEq` kind identity (so the
    // negative tests assert it as part of the expected value). The
    // wrapper's hand-written `PartialEq` continues to exclude ONLY
    // `span` (the TASK-0196 positional-noise contract) — see
    // [`SchedLowerError`]. The remaining `Duplicate*` /
    // option-validation variants are unaffected: there is no single
    // offending *unknown* name to fuzzy-match (the named entity exists
    // — it is duplicated/zero/conflicting), so only these four gain
    // the field, exactly as the link-step kept `UnplacedKernel` /
    // `MissingCrossWorkerTransfer` plain.
    /// A worker entry names a class that has no `worker_class` decl.
    /// `suggestion` is the closest declared `worker_class` name within
    /// the edit-distance bound, else `None`.
    UnknownWorkerClass {
        worker: String,
        class: String,
        suggestion: Option<String>,
    },
    /// A `place_data D in R` names a region with no `memory_region`
    /// decl. `suggestion` is the closest declared `memory_region`
    /// name within the bound, else `None`.
    UnknownMemoryRegion {
        data: String,
        region: String,
        suggestion: Option<String>,
    },
    /// A `place K on W` names a worker with no `workers` entry.
    /// Carries the kernel for diagnostic context. `suggestion` is the
    /// closest declared worker name within the bound, else `None`.
    UnknownPlaceWorker {
        kernel: String,
        worker: String,
        suggestion: Option<String>,
    },
    /// A name in `memory_region R { accessible_by = { ... } }` is
    /// neither a declared `worker_class` nor a declared worker.
    /// Resolution is schedule-internal (TASK-0095) — not deferred to
    /// the linker — because every legal target is declared in the
    /// same schedule. `region` is the owning memory region; `name`
    /// is the offending identifier as written. `suggestion` is the
    /// closest symbol within the bound across the *union* of declared
    /// `worker_class` and worker names (matching this variant's own
    /// validity rule), else `None`.
    UnknownAccessibleByName {
        region: String,
        name: String,
        suggestion: Option<String>,
    },

    // ----- Duplicate / conflicting placement targets -----
    /// `place K on { w0, w0 }` — the same worker named twice in one
    /// placement set. Rejected as a hard error rather than silently
    /// folded to a unique set (TASK-0094): a repeated worker in a
    /// distributed placement is a user mistake, and a silent fold
    /// would hide it (fail-fast; decision-0003). `kernel` is the
    /// placed kernel; `worker` is the duplicated name.
    DuplicatePlaceWorker { kernel: String, worker: String },

    // ----- Duplicate / conflicting directive options -----
    /// A value-bearing `loop` option keyword appears more than once
    /// on one `loop` directive (e.g. `loop i : block=64, block=128`).
    /// Grammar §2 note 7 / §5.1: option order is insignificant and
    /// the option list is a *set*, so a repeated value-bearing key is
    /// a semantic conflict the lowering rejects. `var` is the loop
    /// variable; `option` is the duplicated keyword.
    DuplicateLoopOption { var: String, option: String },
    /// A value-bearing `transfer` option keyword appears more than
    /// once on one `transfer` directive (e.g. `buffer=1, buffer=2`).
    /// `data` is the transfer's data symbol; `option` is the keyword.
    DuplicateTransferOption { data: String, option: String },
    /// The transfer-mode flag set on one `transfer` directive is not
    /// exactly one of `sync` / `async`. Two distinct user errors share
    /// this variant because they are the same error class — the
    /// directive does not name exactly one transfer mode. The two
    /// surface mistakes are (1) a mutual-exclusion conflict, both
    /// `sync` AND `async` appear (e.g. `transfer x : sync, async`),
    /// and (2) a repeated mode, the same flag appears twice (e.g.
    /// `transfer x : sync, sync` / `async, async`).
    /// Grammar §2 note 5 / §5.3: the option list is a *set* and the
    /// transfer mode is exactly-one. `data` is the transfer's data
    /// symbol. The Display is generalized so it is literally accurate
    /// on BOTH paths (TASK-0193): it claims neither "both" (false on
    /// the repeated-mode path) nor "repeated" (false on the conflict
    /// path) but the union invariant true for both.
    ConflictingTransferMode { data: String },

    // ----- Option-value validation -----
    /// `block=0`, `unroll=0`, `pipeline=0` — strictly
    /// positive required. `option` is the keyword as written, `var`
    /// is the owning loop variable.
    ZeroLoopOption { var: String, option: String },
    /// `buffer=0` on a transfer.
    ZeroBufferOption { data: String },
    /// `check loop V : latency_max = 0<UNIT>` (TASK-0052.01). A
    /// zero-time latency assertion is semantically degenerate: every
    /// non-trivial iteration violates it, so the assertion gives no
    /// information about the schedule. Rejecting it forces the
    /// schedule author to either specify a real budget or omit the
    /// directive. PRD §6.3.5 frames `latency_max` as a strictly
    /// positive upper bound. `var` is the loop variable.
    ZeroLatencyMax { var: String },
    /// `check loop V : KIND = ..., KIND = ...` — the same assertion
    /// keyword appears more than once on one `check loop` directive
    /// (TASK-0052.01). PRD §6.3.5 makes each assertion kind a unique
    /// slot on its loop; two values are an internal conflict (which
    /// budget wins?). Rejecting at the IR layer is the simplest
    /// honest answer. `var` is the loop variable; `kind` is the
    /// duplicated keyword (`latency_max`, `on_violation`, …).
    DuplicateCheckAssertion { var: String, kind: String },
    /// `check loop V : on_violation=panic;` — the grammar allows a
    /// `check` directive whose asserts contain only `on_violation` and
    /// no `latency_max`. PRD §6.3.5 frames `on_violation` as "Action
    /// when an assertion fails"; without a measurement assert there is
    /// nothing to violate, so the directive is semantically empty.
    /// Reject at sched-lower with the explicit MissingLatencyMax to
    /// close the panic-not-diagnostic gap (TASK-0052.02 review-gate
    /// finding #1).
    MissingLatencyMax { var: String },
    /// `loop V : block=N, pipeline=D` — both options on the same
    /// loop. PRD §6.3.3: "Loop options are orthogonal where possible.
    /// Bad combinations ... are rejected at compile time, not at
    /// runtime." The block + pipeline combo has ambiguous semantics:
    /// after `block_transform` tiles V into outer/inner, where does
    /// pipeline=D apply? Per-tile (D in-flight tiles)? Per-iter
    /// (D in-flight intra-tile iterations)? The current block_transform
    /// reuses the iter-var id for the inner intra-tile loop, so the
    /// downstream pipeline=D would land on intra-tile iterations —
    /// almost certainly not what the schedule author intended. Reject
    /// at sched-lower with a precise diagnostic so the user picks
    /// ONE of (block, pipeline) for now (TASK-0215).
    BlockPipelineConflict { var: String },
    /// `loop V : block=N, unroll=M` where `M` does not divide `N`
    /// (TASK-0144 Stage 2). PRD §6.3.3: "bad combinations rejected at
    /// compile time, not at runtime." `block=N` strip-mines the loop
    /// into outer/inner tiles of length `N`; `unroll=M` then asks the
    /// backend to unroll the (intra-tile) iter count by `M`. If `M`
    /// does not divide `N`, the tail of every full tile is the
    /// remainder `N % M` iterations — i.e. the schedule author wrote
    /// two integer constants that disagree at compile time, before any
    /// loop bound is known. The cleanest answer is to refuse the
    /// schedule: there is no honest, range-independent unroll factor.
    /// Both integers are static, so the check is purely on the option
    /// payloads. `var` is the loop variable; `unroll` and `block` are
    /// the literal values written.
    UnrollNotDivisibleByBlock {
        var: String,
        unroll: u64,
        block: u64,
    },
    // `VectorizeNotDivisibleByBlock` removed (TASK-0292, 2026-05-25).
    // The `vectorize=M` directive was dropped from the grammar; this
    // validation rule (TASK-0144.01 Stage 3) is dead with no
    // producer.
    /// `loop V : block=N;` + `check loop V : latency_max=T;` — the
    /// loop V has both a strip-mine directive AND a latency check.
    /// `block_transform` tiles V into outer/inner; the inner Event::Loop
    /// is the strip-mined block-tile loop (`block_tag.is_some()`) and
    /// `inject_check_frames` does not attach `check_frame` to those by
    /// design — so the user's check would silently vanish. PRD §6.3.5
    /// frames `latency_max` as "wall-clock duration of one iteration";
    /// after strip-mining, "one source iteration" is no longer a single
    /// Event::Loop boundary, so the semantic translation is unclear.
    /// Reject at sched-lower until the semantics are decided
    /// (TASK-0052.02 review-gate finding #3 / TASK-0220 follow-up).
    /// `var` is the loop variable carrying both directives.
    CheckOnStripMinedLoop { var: String },
    /// `pipeline=1` on a loop (TASK-0134). A pipeline depth of 1 is
    /// "no pipelining" — one iteration in flight is the default
    /// sequential mode. Accepting it would lower to
    /// `initial_marking = 1` on every buffer place in the loop body,
    /// which is observable but semantically indistinguishable from
    /// the producer firing once before the consumer (i.e. the
    /// default behaviour). Rejecting it forces the schedule author
    /// to either specify a real pipelined depth (>= 2) or omit the
    /// directive entirely — eliminating a silent-no-op footgun.
    /// `var` is the loop variable carrying the option.
    UnitPipelineOption { var: String },
    /// `unroll=N` on a loop (TASK-0458). The directive is parsed
    /// (parser.rs), positivity-checked, and lowered to
    /// `ResolvedLoopOption::Unroll` — but consumed by NO downstream
    /// pass. A schedule author writing `unroll=8` to tune the backend
    /// would silently get nothing: the exact silent-downgrade pattern
    /// the capability matrix forbids elsewhere (fail-fast violation).
    /// PRD §6.3.3 defers the real transform to TASK-0293 (reopen on
    /// concrete LLVM-vs-DSL divergence evidence); until that consumer
    /// lands, the only honest behaviour is a loud reject naming the
    /// option as accepted-but-unimplemented. When TASK-0293 lands the
    /// real unroll transform, REMOVE this variant + its reject and
    /// route `ResolvedLoopOption::Unroll` to the new consumer instead.
    /// `var` is the loop variable carrying the option.
    UnrollUnimplemented { var: String },

    // ----- Multiple workers decls -----
    /// More than one `workers = ...` directive in a single schedule.
    /// Grammar §1 phrases the workers decl as a single declaration;
    /// the parser accepts repetition (Vec) and the IR rejects it.
    DuplicateWorkersDecl,
    /// No `workers = ...` directive at all. Every schedule must
    /// declare its worker set (grammar §1: WorkersDecl appears in
    /// SchedItem; the program section requires at least one). The
    /// parser accepts a workers-less program for now; the IR rejects.
    MissingWorkersDecl,
}

/// Append ` -- did you mean `X`?` when a suggestion exists; emit
/// nothing when `None` (the message is byte-identical to the
/// pre-TASK-0198 form in that case — the zero-behaviour-change-for-
/// the-no-suggestion-path guarantee). Mirrors the link-step
/// `write_suggestion` (TASK-0096) verbatim.
fn write_suggestion(
    f: &mut std::fmt::Formatter<'_>,
    suggestion: &Option<String>,
) -> std::fmt::Result {
    match suggestion {
        Some(s) => write!(f, " -- did you mean `{s}`?"),
        None => Ok(()),
    }
}

impl std::fmt::Display for SchedLowerErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedLowerErrorKind::DuplicateWorkerClass(n) => {
                write!(f, "duplicate `worker_class` declaration `{n}`")
            }
            SchedLowerErrorKind::DuplicateMemoryRegion(n) => {
                write!(f, "duplicate `memory_region` declaration `{n}`")
            }
            SchedLowerErrorKind::DuplicateWorker(n) => write!(f, "duplicate worker name `{n}`"),
            SchedLowerErrorKind::DuplicatePlace { kernel } => {
                write!(f, "kernel `{kernel}` has more than one `place` directive")
            }
            SchedLowerErrorKind::DuplicatePlaceData { data } => {
                write!(
                    f,
                    "data symbol `{data}` has more than one `place_data` directive"
                )
            }
            SchedLowerErrorKind::DuplicateLoop { var } => {
                write!(
                    f,
                    "loop variable `{var}` has more than one `loop` directive"
                )
            }
            SchedLowerErrorKind::DuplicateTransfer { data } => {
                write!(
                    f,
                    "data symbol `{data}` has more than one `transfer` directive"
                )
            }
            SchedLowerErrorKind::DuplicateCheck { var } => {
                write!(
                    f,
                    "loop variable `{var}` has more than one `check loop` directive"
                )
            }
            SchedLowerErrorKind::UnknownWorkerClass {
                worker,
                class,
                suggestion,
            } => {
                write!(
                    f,
                    "worker `{worker}` references undeclared `worker_class` `{class}`"
                )?;
                write_suggestion(f, suggestion)
            }
            SchedLowerErrorKind::UnknownMemoryRegion {
                data,
                region,
                suggestion,
            } => {
                write!(
                    f,
                    "`place_data {data} in {region}` references undeclared `memory_region` `{region}`"
                )?;
                write_suggestion(f, suggestion)
            }
            SchedLowerErrorKind::UnknownPlaceWorker {
                kernel,
                worker,
                suggestion,
            } => {
                write!(
                    f,
                    "`place {kernel} on {worker}` references undeclared worker `{worker}`"
                )?;
                write_suggestion(f, suggestion)
            }
            SchedLowerErrorKind::UnknownAccessibleByName {
                region,
                name,
                suggestion,
            } => {
                write!(
                    f,
                    "`memory_region {region}` `accessible_by` lists `{name}`, \
                     which is not a declared `worker_class` or worker"
                )?;
                write_suggestion(f, suggestion)
            }
            SchedLowerErrorKind::DuplicatePlaceWorker { kernel, worker } => write!(
                f,
                "`place {kernel}` lists worker `{worker}` more than once \
                 in its placement set"
            ),
            SchedLowerErrorKind::DuplicateLoopOption { var, option } => write!(
                f,
                "loop `{var}` has more than one `{option}` option; \
                 each option may appear at most once"
            ),
            SchedLowerErrorKind::DuplicateTransferOption { data, option } => write!(
                f,
                "transfer `{data}` has more than one `{option}` option; \
                 each option may appear at most once"
            ),
            SchedLowerErrorKind::ConflictingTransferMode { data } => write!(
                f,
                "transfer `{data}` must specify exactly one of `sync` or `async`; \
                 they are mutually exclusive and neither may be repeated"
            ),
            SchedLowerErrorKind::ZeroLoopOption { var, option } => write!(
                f,
                "loop `{var}` has `{option}=0`; option requires a strictly positive value"
            ),
            SchedLowerErrorKind::ZeroBufferOption { data } => write!(
                f,
                "transfer `{data}` has `buffer=0`; `buffer` requires a strictly positive value"
            ),
            SchedLowerErrorKind::UnitPipelineOption { var } => write!(
                f,
                "loop `{var}` has `pipeline=1`; specify `pipeline=D` with `D >= 2` or omit the option \
                 (pipeline=1 is a no-op — one iteration in flight is the default sequential mode)"
            ),
            SchedLowerErrorKind::UnrollUnimplemented { var } => write!(
                f,
                "loop `{var}` has `unroll=N`, which is accepted by the grammar but \
                 not yet implemented — no compiler pass consumes it, so the option \
                 would silently do nothing. Remove the `unroll` option for now. \
                 Implementation is deferred to TASK-0293 (PRD §6.3.3); SIMD/unroll \
                 is currently delegated to the host Rust compiler + LLVM."
            ),
            SchedLowerErrorKind::ZeroLatencyMax { var } => write!(
                f,
                "`check loop {var} : latency_max = 0...` is rejected; \
                 latency_max requires a strictly positive budget \
                 (PRD §6.3.5: latency_max is an upper bound on per-iteration wall-clock time)"
            ),
            SchedLowerErrorKind::DuplicateCheckAssertion { var, kind } => write!(
                f,
                "`check loop {var}` has more than one `{kind}` assertion; \
                 each assertion kind may appear at most once per check directive"
            ),
            SchedLowerErrorKind::MissingLatencyMax { var } => write!(
                f,
                "`check loop {var}` has no `latency_max` assertion; \
                 a check directive must contain a measurement assert \
                 (`on_violation` alone is meaningless — it is the action \
                 when an assertion fails)"
            ),
            SchedLowerErrorKind::BlockPipelineConflict { var } => write!(
                f,
                "loop `{var}` has both `block=N` and `pipeline=D`; the combination has \
                 ambiguous semantics (per-tile vs per-iteration pipelining) and is not \
                 yet supported. Pick ONE: drop `block=N` to pipeline the full loop, or \
                 drop `pipeline=D` to tile without pipelining. PRD §6.3.3 (TASK-0215)."
            ),
            SchedLowerErrorKind::UnrollNotDivisibleByBlock { var, unroll, block } => write!(
                f,
                "loop `{var}` has `unroll={unroll}, block={block}` but `unroll` must divide \
                 `block` (got {unroll}\u{2224}{block}); pick an unroll factor that divides the \
                 tile size or drop one of the options. PRD §6.3.3 (TASK-0144)."
            ),
            SchedLowerErrorKind::CheckOnStripMinedLoop { var } => write!(
                f,
                "`check loop {var}` cannot apply when `loop {var}` is strip-mined by `block=N`; \
                 after strip-mining, one source iteration is no longer one Event::Loop boundary, \
                 so the latency assertion's semantic is unclear (TASK-0220). \
                 Either remove the `block=N` option on `loop {var}` or remove the `check loop {var}` directive."
            ),
            SchedLowerErrorKind::DuplicateWorkersDecl => write!(
                f,
                "more than one `workers = ...` directive in this schedule; at most one is allowed"
            ),
            SchedLowerErrorKind::MissingWorkersDecl => {
                write!(f, "schedule is missing a `workers = ...` directive")
            }
        }
    }
}

/// A schedule-lowering error: a [`SchedLowerErrorKind`] plus, where a
/// single offending source node exists, the byte [`Range`] it was
/// parsed from (TASK-0196 — the schedule mirror of the algorithm-side
/// [`crate::algo::ir::LowerError`], TASK-0090).
///
/// # Why a struct wrapping a kind (not `(line, column)` fields per
/// variant)
///
/// Putting a position on a *wrapper* instead of widening every variant
/// means no variant's payload shape changed: the existing negative
/// tests still pattern-match `SchedLowerErrorKind::X(payload)` with the
/// same payload, only through `err.kind`. The byte range — not `(line,
/// column)` — is stored because lowering takes `&SchedAst` only and has
/// no source string; the driver (which holds the source) converts via
/// [`crate::error::offset_to_line_col`] at display time, exactly as
/// [`crate::error::ParseError`] and [`crate::algo::ir::LowerError`] are
/// surfaced. This keeps one span representation end-to-end (matching
/// [`crate::span::Spanned`]) and lowering source-text-free.
///
/// # `span` is `Option` (honest-partial per variant — TASK-0196)
///
/// Most variants have one obviously-offending source token and carry
/// its span (the duplicated/undeclared identifier, the second
/// `workers` directive, etc.). Exactly two variants are genuinely
/// position-less and stay `span: None` — a documented missing position
/// is honest; a fabricated one is not:
///
/// - [`SchedLowerErrorKind::MissingWorkersDecl`]: the error is the
///   *absence* of a `workers = ...` directive. There is no source token
///   to point at.
/// - [`SchedLowerErrorKind::DuplicateWorkerClass`] **only when raised
///   from the synthetic-default-class collision branch** (a user class
///   literally named [`DEFAULT_WORKER_CLASS`] colliding with the
///   compiler-injected one): the collision is between a real user decl
///   and a *synthesised* class that has no source token, and the branch
///   does not have the user decl's `Spanned` in scope (it iterates the
///   post-collected class table). The far more common
///   `DuplicateWorkerClass` from two real `worker_class` decls DOES
///   carry the duplicate decl's `c.name.span`. (See `sched/lower.rs`;
///   pinned by `position_less_variants_have_no_span`.)
///
/// Every other variant carries a real span. This doc is kept exactly in
/// sync with the code (the TASK-0090 review caught a doc that
/// overclaimed position-lessness — that lesson is applied here: the
/// position-less set above is the precise, code-verified set, pinned by
/// a test).
///
/// # Equality semantics (load-bearing — AC#1, mirrors `Spanned` /
/// `LowerError`)
///
/// [`PartialEq`] / [`Eq`] are **hand-written to forward to `kind`
/// only**; `span` is deliberately EXCLUDED from value identity. This
/// is the same decision, for the same reason, as
/// [`crate::span::Spanned`] (TASK-0082) and
/// [`crate::algo::ir::LowerError`] (TASK-0090): the source position is
/// informational-for-humans, not part of *which semantic error this
/// is*. Excluding it keeps every existing `SchedLowerErrorKind`-
/// asserting negative test valid (they assert the semantic kind +
/// payload, never the byte offset). `#[derive(PartialEq)]` would
/// (wrongly) fold the span into equality. No `Hash` is derived or
/// implemented (mirrors `LowerError` — the type is not used as a map
/// key; deriving `Hash` alongside a manual `PartialEq` is also the
/// `derived_hash_with_manual_eq` clippy hazard).
#[derive(Debug, Clone)]
pub struct SchedLowerError {
    /// The semantic violation.
    pub kind: SchedLowerErrorKind,
    /// Byte range into the original schedule source, when a single
    /// offending node exists. `None` only for the two genuinely
    /// position-less cases (see type docs). Feed `span.start` to
    /// [`crate::error::offset_to_line_col`] for a 1-based
    /// `(line, column)`.
    pub span: Option<Range<usize>>,
}

impl SchedLowerError {
    /// A schedule-lowering error with no source position (the two
    /// genuinely position-less cases — see type docs). Prefer
    /// [`SchedLowerError::at`] whenever a single offending
    /// [`crate::span::Spanned`] is in scope.
    pub fn new(kind: SchedLowerErrorKind) -> Self {
        Self { kind, span: None }
    }

    /// A schedule-lowering error located at `span` — the byte range of
    /// the offending source node (`spanned.span`). This is the path
    /// AC#2 requires for every diagnosable variant that has a single
    /// offending node.
    pub fn at(kind: SchedLowerErrorKind, span: Range<usize>) -> Self {
        Self {
            kind,
            span: Some(span),
        }
    }

    /// Render the error with a source location resolved against `src`.
    ///
    /// This is the driver-facing surface (AC#2): the driver holds the
    /// schedule source, so it — not lowering — turns the stored byte
    /// offset into a `line:column`. Mirrors how
    /// [`crate::error::ParseError`] and
    /// [`crate::algo::ir::LowerError`] are surfaced. The offset is
    /// clamped to `src.len()` (decision-0003) — and
    /// [`crate::error::offset_to_line_col`] additionally clamps by
    /// construction. When the variant has no position (see type docs),
    /// the message is the kind alone, with no fabricated location.
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
// (AC#1, same rationale as `Spanned` / `LowerError`). Deriving would
// fold the span in and break every existing
// `SchedLowerErrorKind`-asserting negative test.
impl PartialEq for SchedLowerError {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for SchedLowerError {}

// Span-free Display: library callers / tests without source text get
// the semantic message unchanged from before TASK-0196. The located
// form is `display_with_src` (driver-side).
impl std::fmt::Display for SchedLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for SchedLowerError {}

/// A non-empty, deterministically-ordered bundle of [`SchedLowerError`]s
/// — the multi-error result of one [`lower_sched`](super::lower::lower_sched)
/// pass (TASK-0200, the schedule analog of the algorithm-side TASK-0092
/// [`crate::algo::ir::LowerErrors`]).
///
/// # Why a new owner and NOT a re-use
///
/// `ParseErrors` is the *parser* layer's owner ([`ParseError`](crate::error::ParseError), not
/// [`SchedLowerError`]); [`crate::algo::ir::LowerErrors`] is the
/// *algorithm* lowering layer's owner. They are different types at
/// different pipeline stages. The SURFACING pattern (non-empty owner,
/// `.errors()`, driver iterates one located line per error) is the
/// proven template from TASK-0080/0081/0087/0092, but the type is
/// layer-specific — reusing either would conflate the layers' error
/// vocabularies.
///
/// # Non-empty invariant (load-bearing)
///
/// A `SchedLowerErrors` is constructed *only* when lowering actually
/// failed, so the inner `Vec` is never empty. The single constructor
/// `SchedLowerErrors::from_nonempty` is the sole entry point and
/// `debug_assert!`s this; [`SchedLowerErrors::first`] therefore never
/// has an empty slice to handle. Construction is `pub(crate)` so no
/// external caller can forge an empty bundle.
///
/// # Ordering / determinism (PRD §10.1)
///
/// The vector is in **source / directive order** — lowering walks
/// `SchedAst::directives` in order and pushes each error as it is
/// found. There is NO `HashMap`/`HashSet` iteration on the
/// error-collection path (the cascade-suppression bookkeeping is a
/// `BTreeMap`), so the emitted error sequence is a pure deterministic
/// function of the input. Two builds of the same broken schedule emit
/// byte-identical diagnostics.
///
/// # Cascade landscape disclosure (honest-partial — TASK-0200)
///
/// The algorithm-side counting contract (cycle-3, transitive poison)
/// applies here with TWO suppression paths in the `Accum` of
/// [`super::lower::lower_sched`]:
///
/// 1. **`failed_decls`-keyed name cascade** (algo cycle-3 design,
///    transferred verbatim, including the transitive-poison case-1
///    logic). The four reference-resolution variants
///    (`UnknownWorkerClass`, `UnknownMemoryRegion`,
///    `UnknownPlaceWorker`, `UnknownAccessibleByName`) are the
///    cascade-candidate kinds at this path. **NO LIVE TRIGGER on
///    today's variant set** — every `worker_class` / `memory_region` /
///    worker entry that survives its duplicate check is
///    unconditionally inserted into the symbol table (there is no
///    arithmetic-expression evaluation at the sched layer the way
///    the algorithm side has `const N = 1/0`). Forward-looking
///    infrastructure.
///
/// 2. **`workers_missing`-keyed `UnknownPlaceWorker` suppression**.
///    **FIRES TODAY** on the unique `MissingWorkersDecl` path: with
///    no `workers = ...` directive, `ir.workers` stays empty by
///    construction, and every subsequent `place X on W` necessarily
///    fires `UnknownPlaceWorker{W}` as a pure cascade of the
///    already-reported root. Suppressed so the user sees one root
///    diagnostic instead of N cascade lines. **Narrow:**
///    `UnknownAccessibleByName` is NOT suppressed at this path
///    because the referenced name could be a class OR a worker;
///    only the worker-side miss is a cascade of
///    `MissingWorkersDecl`, and an unknown class is independent.
///
/// The parametric K×L independent-count fixture and the parametric
/// over-N Path-2 fixture together pin AC#3. See
/// [`super::lower::lower_sched`] for the per-variant classification
/// and the soundness argument.
///
/// # Equality
///
/// Derived `PartialEq`/`Eq` — element-wise over [`SchedLowerError`],
/// whose own equality forwards to `kind` (span excluded; same
/// rationale as [`crate::span::Spanned`]). So bundle equality
/// compares the ordered sequence of *semantic kinds*, not byte
/// offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedLowerErrors(Vec<SchedLowerError>);

impl SchedLowerErrors {
    /// Construct from a non-empty `Vec<SchedLowerError>`. The sole
    /// constructor; crate-private so the non-empty invariant cannot be
    /// violated from outside lowering. `debug_assert!`s non-emptiness
    /// (a caller handing an empty vec is a lowering-pass bug, not a
    /// user-input condition — decision-0003: invariant violation, so
    /// `debug_assert!`, not a typed error).
    pub(crate) fn from_nonempty(errors: Vec<SchedLowerError>) -> Self {
        debug_assert!(
            !errors.is_empty(),
            "SchedLowerErrors is constructed only on a non-empty failure set \
             (lowering-pass invariant); an empty vec here is a compiler bug"
        );
        Self(errors)
    }

    /// The first (source-order-earliest) error. Equivalent to the
    /// single error the pre-multi-error pass would have `?`-returned,
    /// so negative tests that previously asserted *the* error migrate
    /// by calling `.first()` with the SAME discriminating match — no
    /// loss of assertion strength.
    pub fn first(&self) -> &SchedLowerError {
        self.0
            .first()
            .expect("SchedLowerErrors is constructed non-empty (invariant)")
    }

    /// All errors in source order. The driver iterates this to surface
    /// every violation in one compile cycle.
    pub fn errors(&self) -> &[SchedLowerError] {
        &self.0
    }
}

impl std::ops::Deref for SchedLowerErrors {
    type Target = [SchedLowerError];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// One error per line, each via the span-free [`SchedLowerError`]
/// `Display` (the located form is the driver's `display_with_src`,
/// which holds the source). This is the fallback for a caller that
/// just `{}`s the whole bundle.
impl std::fmt::Display for SchedLowerErrors {
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

impl std::error::Error for SchedLowerErrors {}
