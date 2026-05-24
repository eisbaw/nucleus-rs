//! AST types for the schedule sublanguage.
//!
//! Shape and naming follow `docs/grammar-sched.md` 1:1 — one Rust type
//! per nonterminal where that aids readability. Mirrors the algorithm
//! sub-language AST style (`crate::algo::ast`) for consistency.
//!
//! Per-node source spans ARE tracked (TASK-0086): the diagnosable
//! nodes are wrapped in [`Spanned<T>`][crate::span::Spanned] (the
//! shared wrapper, promoted from the algorithm side — see
//! `crate::span` for the share-vs-duplicate rationale and the exact
//! granularity / why the option-enum leaves are not wrapped).
//! `parse_sched` populates byte ranges; lowering still IGNORES them.
//! Threading these spans into `SchedLowerError` is the separate
//! TASK-0196 (the schedule analog of the algorithm's TASK-0090).
//!
//! Equality semantics: the inner-node `#[derive(PartialEq)]`s below
//! still make tests cheap, and `Spanned<T>`'s *manual* `PartialEq`
//! forwards to the node only (span EXCLUDED — see `crate::span`), so
//! two ASTs still compare structurally regardless of source position.
//! The AST holds no interned IDs. (Field-projection tests that
//! compare a [`SpName`] directly to a `&str` project through `.node`;
//! the comparison strength is unchanged — see `crate::span` docs.)
//!
//! Design choices for the schedule AST (see TASK-0008 notes):
//!
//! - The simple and typed worker forms are unified under a single
//!   [`WorkersDecl`] with each entry carrying an optional class name
//!   (`None` = simple form). This avoids duplicating downstream
//!   resolution code for two surface forms that mean the same thing
//!   when the typed form has one class. Grammar §1 keeps them
//!   syntactically distinct; we collapse semantically.
//! - Loop, transfer, and check option lists are flat `Vec`s of typed
//!   enums. Semantic conflicts (`sync` and `async` in the same
//!   transfer, duplicate `block=N`, etc.) are linker-pass concerns
//!   (grammar §2 notes 5, 7; TASK-0010 / TASK-0011). The parser
//!   accepts; later passes reject.
//! - Time literals are normalised to nanoseconds at parse time
//!   (`u64` ns). The original unit is dropped. Rationale: downstream
//!   consumers compare durations; keeping the unit only forces
//!   reconversion. The grammar says no fractional time literals
//!   (§6 #5), so integer ns is lossless. See [`TimeLit`].
//! - Size literals are normalised to bytes at parse time (`u64`).
//!   Same rationale.

use crate::span::Spanned;

/// An identifier / name as written in source, plus the byte range of
/// the identifier token. Diagnostics that name a worker, class,
/// region, kernel, loop variable, etc. ("duplicate worker `w0`",
/// "undeclared class `core`") underline `span`; the `String`
/// compares/hashes structurally (span excluded — see `crate::span`).
pub type SpName = Spanned<String>;

/// A schedule directive plus its source span. Every entry of
/// [`SchedAst::directives`] is wrapped so a directive rejected as a
/// whole (a duplicate `workers` decl, a `place` with an undeclared
/// target) can point at the directive (TASK-0196).
pub type SpDirective = Spanned<Directive>;

/// Root AST node for a `*.sched.nuc` file.
///
/// Per grammar `Program ::= ScheduleBlock`, exactly one top-level
/// `schedule for "..."` block per source file.
#[derive(Debug, Clone, PartialEq)]
pub struct SchedAst {
    /// Path-string from `schedule for "<algo_path>" { ... }`. Stored
    /// EXACTLY as it appears in source — no normalisation, no
    /// canonicalisation, no resolution. The build driver takes the
    /// algorithm path from `--algo` (see `driver/src/main.rs` first
    /// use of `a.algo`); this string is documentation, not input.
    /// TASK-0277 contract-pinned by
    /// `algo_path_stored_verbatim_no_resolution` +
    /// `algo_path_invariant_under_schedule_file_directory` in
    /// `tests/sched_parser.rs`.
    pub algo_path: String,
    /// In source order. Order is informational; semantic passes
    /// enforce "declare before reference" but not stylistic ordering.
    /// Each directive carries its source span via [`SpDirective`].
    pub directives: Vec<SpDirective>,
}

/// One item in a schedule block (grammar `SchedItem`).
#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    WorkerClass(WorkerClassDecl),
    MemoryRegion(MemoryRegionDecl),
    Workers(WorkersDecl),
    Place(PlaceDirective),
    PlaceData(PlaceDataDirective),
    Loop(LoopDirective),
    Transfer(TransferDirective),
    Check(CheckDirective),
}

impl SchedAst {
    /// Convenience: count directives of each kind. Used by tests for
    /// quick structural assertions.
    pub fn count_workers(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::Workers(_)))
            .count()
    }

    pub fn count_worker_classes(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::WorkerClass(_)))
            .count()
    }

    pub fn count_memory_regions(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::MemoryRegion(_)))
            .count()
    }

    pub fn count_places(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::Place(_)))
            .count()
    }

    pub fn count_place_data(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::PlaceData(_)))
            .count()
    }

    pub fn count_loops(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::Loop(_)))
            .count()
    }

    pub fn count_transfers(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::Transfer(_)))
            .count()
    }

    pub fn count_checks(&self) -> usize {
        self.directives
            .iter()
            .filter(|d| matches!(&d.node, Directive::Check(_)))
            .count()
    }
}

// --------------------------------------------------------------------
// Worker topology
// --------------------------------------------------------------------

/// `worker_class IDENT { ClassField* };`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerClassDecl {
    pub name: SpName,
    /// `simd = ...` field, if present. The grammar lists the field as
    /// optional via `ClassField*` (zero or more). Absent fields are
    /// `None`; the linker rejects classes that omit mandatory fields.
    pub simd: Option<SimdSpec>,
    /// `memory = ...` field.
    pub memory: Option<MemorySpec>,
}

/// `SimdSpec ::= 'none' | Ident`. We collapse to one enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimdSpec {
    None,
    /// Backend-interpreted SIMD name, e.g. `neon128`, `avx2`. The
    /// grammar imposes no restriction on the identifier.
    Named(String),
}

/// `MemorySpec ::= MemoryAtom ('+' MemoryAtom)*`. A flat `Vec` of
/// atoms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySpec {
    pub atoms: Vec<MemoryAtom>,
}

/// `MemoryAtom ::= 'shared' | Ident ('[' SizeLit ']')?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryAtom {
    Shared,
    Named {
        name: String,
        /// `[64KB]` size, in bytes. `None` if no bracketed size.
        size_bytes: Option<u64>,
    },
}

/// `memory_region IDENT { RegionField* };`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRegionDecl {
    pub name: SpName,
    /// `size = SizeLit` field (in bytes). `None` if absent.
    pub size_bytes: Option<u64>,
    /// `accessible_by = { id, id, ... }` — class names or worker names
    /// (resolution is the linker's job, grammar §2 note 4). `None` if
    /// the field is absent; empty `Some(Vec::new())` for `{}`. Each
    /// name is an [`SpName`] so an undeclared-`accessible_by`-name
    /// error (TASK-0196) can underline the offending name token.
    pub accessible_by: Option<Vec<SpName>>,
    /// `per_worker = true|false`. `None` if absent; the linker
    /// defaults absent to `false`.
    pub per_worker: Option<bool>,
}

/// `workers = { ... };`. The grammar (§1) defines two forms (simple,
/// typed) but the AST stores them uniformly: each entry has an
/// optional class. All-`None` classes equals the simple form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkersDecl {
    pub entries: Vec<WorkerEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerEntry {
    pub name: SpName,
    /// `None` for the simple form `{ host, w0 }`; `Some(class)` for
    /// the typed form `{ host : control_core }`.
    pub class: Option<SpName>,
}

// --------------------------------------------------------------------
// Placement
// --------------------------------------------------------------------

/// `place IDENT on PlaceTarget;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceDirective {
    pub kernel: SpName,
    pub target: PlaceTarget,
}

/// `PlaceTarget ::= Ident | '{' IdentList '}'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceTarget {
    /// Single worker (or single worker name). [`SpName`] so an
    /// undeclared-worker error points at the worker token.
    One(SpName),
    /// Distributed across a set of workers. The grammar uses
    /// `IdentList` (non-empty); we surface this with `Vec<SpName>` and
    /// let the parser reject the empty-set case as a syntax error.
    Many(Vec<SpName>),
}

/// `place_data IDENT in IDENT;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceDataDirective {
    pub data: SpName,
    pub region: SpName,
}

// --------------------------------------------------------------------
// Loop transformations
// --------------------------------------------------------------------

/// `loop IDENT : LoopOpt (, LoopOpt)*;`.
///
/// `options` is non-empty by construction — the grammar requires at
/// least one option, and the parser rejects `loop x : ;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopDirective {
    pub var: SpName,
    pub options: Vec<LoopOption>,
}

/// `LoopOpt` from the grammar §1. Bool-only options (`reuse`) are
/// nullary variants; numeric ones carry their literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopOption {
    Block(u64),
    // Vectorize removed (TASK-0292, 2026-05-25). SIMD vectorisation is
    // delegated to the host Rust compiler + LLVM. See PRD §6.3.3.
    Unroll(u64),
    Pipeline(u64),
    Reuse,
    Partition(PartitionKind),
}

/// `PartitionKind ::= 'rows' | 'blocks2d' | 'workers'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartitionKind {
    Rows,
    Blocks2d,
    Workers,
}

// --------------------------------------------------------------------
// Transfer / IO semantics
// --------------------------------------------------------------------

/// `transfer IDENT : XferOpt (, XferOpt)*;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferDirective {
    pub data: SpName,
    pub options: Vec<TransferOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOption {
    Sync,
    Async,
    Buffer(u64),
    Notify(NotifyKind),
}

/// `NotifyKind ::= 'event' | 'poll'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotifyKind {
    Event,
    Poll,
}

// --------------------------------------------------------------------
// Runtime assertions
// --------------------------------------------------------------------

/// `check loop IDENT : CheckAssert (, CheckAssert)*;`.
///
/// Grammar §2 note 8: the variable named here is a *loop variable from
/// the algorithm*. No `loop`-directive needs to exist for that
/// variable. Resolution is the linker's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckDirective {
    pub var: SpName,
    pub asserts: Vec<CheckAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckAssert {
    /// `latency_max = TimeLit`. Stored as nanoseconds (`u64`) after
    /// unit normalisation. See module docstring.
    LatencyMax(TimeLit),
    OnViolation(ViolationKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViolationKind {
    Panic,
    Log,
    Count,
}

/// A time literal normalised to nanoseconds.
///
/// Original unit preserved for diagnostics / round-trip serialisation
/// in case a future pass wants to format the value back in its source
/// unit. The compiler proper compares `nanos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeLit {
    pub nanos: u64,
    pub original_unit: TimeUnit,
    pub original_value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    Ns,
    Us,
    Ms,
    S,
}

impl TimeUnit {
    /// Multiplier from this unit to nanoseconds. The grammar restricts
    /// time literals to `IntLit TimeUnit` (no fractions), so a `u64`
    /// multiplier is lossless.
    pub const fn nanos_per_unit(self) -> u64 {
        match self {
            TimeUnit::Ns => 1,
            TimeUnit::Us => 1_000,
            TimeUnit::Ms => 1_000_000,
            TimeUnit::S => 1_000_000_000,
        }
    }
}
