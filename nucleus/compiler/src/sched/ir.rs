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

use core::ops::Range;
use std::collections::BTreeMap;

use super::ast::{MemorySpec, NotifyKind, PartitionKind, SimdSpec, TimeLit, ViolationKind};

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
    /// A worker entry names a class that has no `worker_class` decl.
    UnknownWorkerClass { worker: String, class: String },
    /// A `place_data D in R` names a region with no `memory_region`
    /// decl.
    UnknownMemoryRegion { data: String, region: String },
    /// A `place K on W` names a worker with no `workers` entry.
    /// Carries the kernel for diagnostic context.
    UnknownPlaceWorker { kernel: String, worker: String },
    /// A name in `memory_region R { accessible_by = { ... } }` is
    /// neither a declared `worker_class` nor a declared worker.
    /// Resolution is schedule-internal (TASK-0095) — not deferred to
    /// the linker — because every legal target is declared in the
    /// same schedule. `region` is the owning memory region; `name`
    /// is the offending identifier as written.
    UnknownAccessibleByName { region: String, name: String },

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
    /// `sync` and `async` both appear on one `transfer` directive
    /// (e.g. `transfer x : sync, async`). Grammar §2 note 5 / §5.3:
    /// they are mutually exclusive. `data` is the transfer's data
    /// symbol. Also covers `sync, sync` / `async, async` (a repeated
    /// transfer-mode flag is the same user error class).
    ConflictingTransferMode { data: String },

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
            SchedLowerErrorKind::UnknownWorkerClass { worker, class } => write!(
                f,
                "worker `{worker}` references undeclared `worker_class` `{class}`"
            ),
            SchedLowerErrorKind::UnknownMemoryRegion { data, region } => write!(
                f,
                "`place_data {data} in {region}` references undeclared `memory_region` `{region}`"
            ),
            SchedLowerErrorKind::UnknownPlaceWorker { kernel, worker } => write!(
                f,
                "`place {kernel} on {worker}` references undeclared worker `{worker}`"
            ),
            SchedLowerErrorKind::UnknownAccessibleByName { region, name } => write!(
                f,
                "`memory_region {region}` `accessible_by` lists `{name}`, \
                 which is not a declared `worker_class` or worker"
            ),
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
                "transfer `{data}` is both `sync` and `async`; \
                 these options are mutually exclusive"
            ),
            SchedLowerErrorKind::ZeroLoopOption { var, option } => write!(
                f,
                "loop `{var}` has `{option}=0`; option requires a strictly positive value"
            ),
            SchedLowerErrorKind::ZeroBufferOption { data } => write!(
                f,
                "transfer `{data}` has `buffer=0`; `buffer` requires a strictly positive value"
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
