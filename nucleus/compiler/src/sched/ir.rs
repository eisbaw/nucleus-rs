//! SchedIR: the semantically-validated intermediate representation of a
//! schedule program.
//!
//! Produced by [`crate::sched::lower_sched`] from a [`SchedAst`]
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
//! - Validates numeric option ranges: `block=N`/`vectorize=N`/
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
//!   backend, `notify=event` on a poll-only backend). Belongs to a
//!   later pass that has the backend's `capabilities.toml` in hand.
//!   Filed as follow-up (TASK-0019 in M1 per task spec).
//!
//! Design choice — separate IR types vs annotated AST: separate, same
//! reasoning as the algorithm side (`crate::algo::ir`). The AST keeps
//! the surface forms distinct (simple vs typed workers; options as
//! flat vectors); the IR collapses them and indexes them. Annotating
//! the AST in place would force every downstream pass to keep handling
//! "is this resolved yet?" cases.

use std::collections::BTreeMap;

use super::ast::{
    MemorySpec, NotifyKind, PartitionKind, SimdSpec, TimeLit, ViolationKind,
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
/// reject the collision via [`SchedLowerError::DuplicateWorkerClass`].
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
/// concern (TASK-0019), not a lowering concern.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlacement {
    pub kernel: String,
    pub target: ResolvedPlaceTarget,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlaceData {
    pub data: String,
    pub region: String,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLoopDirective {
    pub var: String,
    pub options: Vec<ResolvedLoopOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedLoopOption {
    /// `block=N`, N > 0.
    Block(u64),
    /// `vectorize=N`, N > 0.
    Vectorize(u64),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTransferDirective {
    pub data: String,
    pub options: Vec<ResolvedTransferOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTransferOption {
    Sync,
    Async,
    /// `buffer=N`, N > 0.
    Buffer(u64),
    Notify(NotifyKind),
}

// --------------------------------------------------------------------
// Runtime assertions
// --------------------------------------------------------------------

/// Resolved `check loop VAR : asserts;`.
///
/// The `var` is a loop-variable name in the algorithm; whether the
/// algorithm actually has a loop with that variable is a link-step
/// check. Asserts inherit parser-side enum validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCheckDirective {
    pub var: String,
    pub asserts: Vec<ResolvedCheckAssert>,
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
/// driver resolves it relative to the schedule file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchedIR {
    /// Path to the algorithm file, verbatim from the source.
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

/// Errors produced by the schedule lowering pass.
///
/// Each variant names a single semantic violation. As with
/// [`crate::algo::ir::LowerError`], position information from the AST
/// is not available yet (per-node spans are a TASK-0086 follow-up).
/// Variants carry identifying names so the caller can format a
/// human-meaningful message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedLowerError {
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
    /// A worker entry names a class that has no `worker_class` decl.
    UnknownWorkerClass { worker: String, class: String },
    /// A `place_data D in R` names a region with no `memory_region`
    /// decl.
    UnknownMemoryRegion { data: String, region: String },
    /// A `place K on W` names a worker with no `workers` entry.
    /// Carries the kernel for diagnostic context.
    UnknownPlaceWorker { kernel: String, worker: String },

    // ----- Option-value validation -----
    /// `block=0`, `vectorize=0`, `unroll=0`, `pipeline=0` — strictly
    /// positive required. `option` is the keyword as written, `var`
    /// is the owning loop variable.
    ZeroLoopOption { var: String, option: String },
    /// `buffer=0` on a transfer.
    ZeroBufferOption { data: String },

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

impl std::fmt::Display for SchedLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedLowerError::DuplicateWorkerClass(n) => {
                write!(f, "duplicate `worker_class` declaration `{n}`")
            }
            SchedLowerError::DuplicateMemoryRegion(n) => {
                write!(f, "duplicate `memory_region` declaration `{n}`")
            }
            SchedLowerError::DuplicateWorker(n) => write!(f, "duplicate worker name `{n}`"),
            SchedLowerError::DuplicatePlace { kernel } => {
                write!(f, "kernel `{kernel}` has more than one `place` directive")
            }
            SchedLowerError::DuplicatePlaceData { data } => {
                write!(
                    f,
                    "data symbol `{data}` has more than one `place_data` directive"
                )
            }
            SchedLowerError::DuplicateLoop { var } => {
                write!(f, "loop variable `{var}` has more than one `loop` directive")
            }
            SchedLowerError::DuplicateTransfer { data } => {
                write!(
                    f,
                    "data symbol `{data}` has more than one `transfer` directive"
                )
            }
            SchedLowerError::DuplicateCheck { var } => {
                write!(
                    f,
                    "loop variable `{var}` has more than one `check loop` directive"
                )
            }
            SchedLowerError::UnknownWorkerClass { worker, class } => write!(
                f,
                "worker `{worker}` references undeclared `worker_class` `{class}`"
            ),
            SchedLowerError::UnknownMemoryRegion { data, region } => write!(
                f,
                "`place_data {data} in {region}` references undeclared `memory_region` `{region}`"
            ),
            SchedLowerError::UnknownPlaceWorker { kernel, worker } => write!(
                f,
                "`place {kernel} on {worker}` references undeclared worker `{worker}`"
            ),
            SchedLowerError::ZeroLoopOption { var, option } => write!(
                f,
                "loop `{var}` has `{option}=0`; option requires a strictly positive value"
            ),
            SchedLowerError::ZeroBufferOption { data } => write!(
                f,
                "transfer `{data}` has `buffer=0`; `buffer` requires a strictly positive value"
            ),
            SchedLowerError::DuplicateWorkersDecl => write!(
                f,
                "more than one `workers = ...` directive in this schedule; at most one is allowed"
            ),
            SchedLowerError::MissingWorkersDecl => {
                write!(f, "schedule is missing a `workers = ...` directive")
            }
        }
    }
}

impl std::error::Error for SchedLowerError {}
