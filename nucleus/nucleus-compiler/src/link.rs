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
//!   `{w0..w3}` set, no transfer is required (partition= handles the
//!   per-element decomposition, scheduled later in TASK-0016+). This
//!   matches `13-cnn-inference/batch_parallel.sched.nuc` where
//!   `feat1`/`feat2` move within the `{w0..w3}` set with no transfer
//!   directive, but `input` and `output` (between `host` and the set)
//!   do carry transfer directives.
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
//! What this pass explicitly DOES NOT do (deferred):
//!
//! - Type-check kernel signatures against call sites (TASK-0088).
//! - Validate `partition=` policies against placement cardinality.
//! - Per-worker slicing of distributed placements (TASK-0016+).
//! - Resolve transfer/notify semantics against backend capability
//!   matrix (TASK-0019).
//! - Detect data symbols that have no producer at all (could be a
//!   genuine bug; not in the spec for this task).

use core::ops::Range;
use std::collections::{BTreeMap, BTreeSet};

use crate::algo::{AlgoIR, IndexedRef, IrExpr, IrStmt};
use crate::sched::{ResolvedLoopOption, ResolvedPlaceTarget, ResolvedPlacement, ResolvedTransferOption, SchedIR};

// --------------------------------------------------------------------
// Public types
// --------------------------------------------------------------------

/// A worker placement collapsed into a comparable set.
///
/// `One("host")` and `Many({w0, w1})` are both represented as a
/// `BTreeSet<String>`; the size and contents distinguish them. Using a
/// `BTreeSet` means equality is order-independent (so
/// `{w0, w1} == {w1, w0}`) and the type implements `Ord` so it can
/// key error-message maps deterministically.
///
/// Conceptually this is the "worker entity" the link step reasons
/// about. The PRD's distributed-placement note in §6.3.2 says the
/// compiler eventually partitions the iter space across the named
/// workers; until that pass runs (TASK-0016+) we treat the whole set
/// as one identity for transfer-existence purposes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkerEntity(pub BTreeSet<String>);

impl WorkerEntity {
    fn from_target(t: &ResolvedPlaceTarget) -> Self {
        match t {
            ResolvedPlaceTarget::One(w) => {
                let mut s = BTreeSet::new();
                s.insert(w.clone());
                WorkerEntity(s)
            }
            ResolvedPlaceTarget::Many(ws) => WorkerEntity(ws.iter().cloned().collect()),
        }
    }

    /// Stable, human-readable rendering for error messages and
    /// diagnostic maps. `{host}` for singletons, `{w0,w1,w2}` for
    /// sets; deterministic order (BTreeSet iterates sorted).
    pub fn display(&self) -> String {
        let names: Vec<&str> = self.0.iter().map(|s| s.as_str()).collect();
        format!("{{{}}}", names.join(","))
    }
}

/// The result of linking an [`AlgoIR`] and a [`SchedIR`].
///
/// The two source IRs are kept verbatim. Cross-references resolved
/// during linking are exposed as separate maps; downstream passes
/// (ACFG, Petri-net construction) read them rather than re-deriving
/// them from the source IRs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedIR {
    pub algo: AlgoIR,
    pub sched: SchedIR,
    /// For each kernel declared in the algorithm, the placement
    /// directive from the schedule. Keyed by kernel name.
    ///
    /// Invariant after a successful link: this map's keyset equals
    /// `algo.kernels` keyset. (Linking fails if a kernel is unplaced
    /// or if the schedule places a kernel that doesn't exist.)
    pub placements: BTreeMap<String, ResolvedPlacement>,
    /// For each kernel, the resolved worker entity it runs on.
    /// Convenience derivation of [`Self::placements`] — derived once
    /// here so downstream passes don't re-walk targets.
    pub kernel_workers: BTreeMap<String, WorkerEntity>,
    /// For each data symbol that has at least one producer kernel,
    /// the worker entity that produces it. A producer is the kernel
    /// on the RHS of `D <-- Call(...)` in the algorithm.
    ///
    /// Symbols with no producer (e.g. read-only inputs, identity-
    /// copy targets) are omitted. Symbols with multiple producers
    /// across statements: the AlgoIR lowering pass enforces single-
    /// assignment per scope (PRD §6.2.1), so this is unique per
    /// scope, but a `For`-loop body assigning `D[n]` per iteration
    /// is treated as a single producer placement (the body's kernel
    /// placement). We currently record the LAST observed producer
    /// placement — pre-condition: AlgoIR's single-assignment check
    /// has already rejected genuinely-duplicate producers.
    pub data_producers: BTreeMap<String, WorkerEntity>,
    /// For each data symbol read by some kernel, the set of worker
    /// entities that consume it. Multiple distinct consumer entities
    /// are normal (a hub data feeding several workers).
    pub data_consumers: BTreeMap<String, BTreeSet<WorkerEntity>>,
}

/// The semantic-violation *kind* produced by the link pass.
///
/// Each variant names a single contract violation between the
/// algorithm and schedule and carries the payload a diagnostic needs
/// (the offending name, the owning declaration, etc.). The *source
/// position* of the violation is NOT here — it is carried separately
/// on [`LinkError`] so that adding positions did not change any
/// variant's payload shape (TASK-0099, mirroring TASK-0090 verbatim).
/// Equality / Display of a located error forward to this kind; see
/// [`LinkError`] for why position is excluded from value identity.
///
/// # The four unknown-name variants carry a did-you-mean suggestion
///
/// `UnknownKernel`/`UnknownData`/`UnknownLoop`/`UnknownTransferData`
/// are `{ name, suggestion }` structs: `suggestion` is the closest
/// algorithm-side symbol within a bounded edit distance (or `None`),
/// computed by [`crate::error::suggest`] against the in-hand table for
/// that variant (TASK-0096). `UnplacedKernel` and
/// `MissingCrossWorkerTransfer` are unaffected (no single offending
/// *unknown* name to fuzzy-match — the named entities exist), so a
/// per-variant struct widening is used rather than a `{kind,
/// suggestion}` wrapper: only four of six variants gain the field.
///
/// # Equality includes the suggestion (deliberately — kind-level)
///
/// `PartialEq`/`Eq` are **derived** on `LinkErrorKind`, so the
/// `suggestion` field IS part of kind identity. A `suggestion` is a
/// deterministic pure function of `(offending name, in-hand symbol
/// table)`, so two `LinkErrorKind`s that are equal in name AND arose
/// against the same table necessarily have an equal suggestion;
/// folding it into kind equality cannot spuriously split equal errors.
/// The position-noise rule (TASK-0099, mirroring TASK-0082 / TASK-0090)
/// applies one level up at [`LinkError`]: the *wrapper* hand-excludes
/// `span` from value identity, the *kind* keeps every payload field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkErrorKind {
    /// The algorithm declares a kernel that no `place` directive
    /// names. PRD §6.3.2: "Every kernel referenced in the algorithm
    /// must have exactly one `place`. An unplaced kernel is a
    /// compile error."
    UnplacedKernel(String),
    /// The schedule has a `place K on ...` but the algorithm has no
    /// `kernel K` declaration. `suggestion` is the closest declared
    /// kernel name within the edit-distance bound, else `None`.
    UnknownKernel {
        name: String,
        suggestion: Option<String>,
    },
    /// The schedule has a `place_data D in ...` but the algorithm
    /// has no `data D` declaration. `suggestion` is the closest
    /// declared data symbol within the bound, else `None`.
    UnknownData {
        name: String,
        suggestion: Option<String>,
    },
    /// The schedule references a loop variable (via `loop VAR` or
    /// `check loop VAR`) that is not declared as the iteration
    /// variable of any `for VAR : ...` in the algorithm.
    /// `suggestion` is the closest algorithm loop variable within the
    /// bound, else `None`.
    UnknownLoop {
        name: String,
        suggestion: Option<String>,
    },
    /// The schedule has a `transfer D : ...` but the algorithm has
    /// no `data D` declaration. `suggestion` is the closest declared
    /// data symbol within the bound, else `None`.
    UnknownTransferData {
        name: String,
        suggestion: Option<String>,
    },
    /// A data symbol flows from one worker entity to a different
    /// worker entity, but no `transfer` directive exists for it.
    /// PRD §6.3.4: "A `transfer` directive that would cross workers
    /// ... must be present. Omitting it is a compile error".
    MissingCrossWorkerTransfer {
        data: String,
        producer_worker: String,
        consumer_worker: String,
    },
    /// A loop directive `loop VAR : pipeline=D` was applied to a
    /// loop whose body references a data symbol with a
    /// `transfer DATA : buffer=N` where `N < D` (TASK-0134).
    /// PRD §8.2: "Initial marking on a place = pipeline depth /
    /// latency-hiding head-start". A pipelined loop pre-seeds each
    /// inter-stage buffer place with `D` tokens; the place's
    /// capacity is `N`, so `D > N` would overflow the place at
    /// construction time. Caught here (rather than waiting for the
    /// boundedness pass to trip) because the link step has the
    /// schedule names in source-friendly form — the diagnostic can
    /// name the offending {loop_var, data, depth, buffer} directly.
    ///
    /// Why this lives in the link step and not in `acfg_to_petri`:
    /// loop-variable names and transfer data names live in the
    /// schedule directives. By the time `acfg_to_petri` runs, those
    /// names have been resolved to integer IDs and the offending
    /// loop iter-var may have been block-transformed into multiple
    /// new iter-vars — making a precise user-facing diagnostic
    /// harder. Catching at link time keeps the diagnostic close to
    /// the user's source.
    PipelineExceedsBuffer {
        loop_var: String,
        data: String,
        depth: u64,
        buffer: u64,
    },
    /// `loop V : pipeline=D` where `D > iteration_count(V)`. Two
    /// different N's are at play here (TASK-0217):
    /// - `D <= buffer=N`  — bounds the runtime ring (`PipelineExceedsBuffer`).
    /// - `D <= iter_count(V)` — ensures pipelining makes sense.
    ///   `pipeline=3` over a 2-iteration loop tries to put 3 iterations
    ///   in flight when at most 2 exist; the head-start cannot drain.
    ///
    /// Reject at link-time so the user gets a precise diagnostic, not
    /// "I produced a net with leftover initial-marking tokens" (which
    /// is what would happen at acfg_to_petri layer per TASK-0213's
    /// elision math). Filed as TASK-0217.
    PipelineExceedsIterationCount {
        loop_var: String,
        depth: u64,
        iteration_count: i64,
    },
}

/// Append ` -- did you mean `X`?` when a suggestion exists; emit
/// nothing when `None` (the message is byte-identical to the
/// pre-TASK-0096 form in that case).
fn write_suggestion(
    f: &mut std::fmt::Formatter<'_>,
    suggestion: &Option<String>,
) -> std::fmt::Result {
    match suggestion {
        Some(s) => write!(f, " -- did you mean `{s}`?"),
        None => Ok(()),
    }
}

impl std::fmt::Display for LinkErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkErrorKind::UnplacedKernel(name) => write!(
                f,
                "kernel `{name}` is declared in the algorithm but has no `place` directive in the schedule"
            ),
            LinkErrorKind::UnknownKernel { name, suggestion } => {
                write!(
                    f,
                    "schedule places kernel `{name}` but no such kernel is declared in the algorithm"
                )?;
                write_suggestion(f, suggestion)
            }
            LinkErrorKind::UnknownData { name, suggestion } => {
                write!(
                    f,
                    "schedule references data symbol `{name}` in `place_data` but no such data is declared in the algorithm"
                )?;
                write_suggestion(f, suggestion)
            }
            LinkErrorKind::UnknownLoop { name, suggestion } => {
                write!(
                    f,
                    "schedule references loop variable `{name}` but no `for {name} : ...` exists in the algorithm"
                )?;
                write_suggestion(f, suggestion)
            }
            LinkErrorKind::UnknownTransferData { name, suggestion } => {
                write!(
                    f,
                    "schedule has `transfer {name}` but no such data is declared in the algorithm"
                )?;
                write_suggestion(f, suggestion)
            }
            LinkErrorKind::MissingCrossWorkerTransfer {
                data,
                producer_worker,
                consumer_worker,
            } => write!(
                f,
                "data symbol `{data}` flows from {producer_worker} to \
                 {consumer_worker} but the schedule declares no \
                 `transfer {data} : ...` directive. \
                 Add `transfer {data} : sync;` \
                 (or `async`/`buffer=N` for buffered transports)."
            ),
            LinkErrorKind::PipelineExceedsBuffer {
                loop_var,
                data,
                depth,
                buffer,
            } => write!(
                f,
                "loop `{loop_var}` has `pipeline={depth}` but the schedule's \
                 `transfer {data} : buffer={buffer}` cannot hold {depth} in-flight \
                 tokens (pipeline depth must be <= buffer capacity; PRD §8.2)"
            ),
            LinkErrorKind::PipelineExceedsIterationCount {
                loop_var,
                depth,
                iteration_count,
            } => write!(
                f,
                "loop `{loop_var}` has `pipeline={depth}` but the loop's source range \
                 yields only {iteration_count} iteration(s); pipeline depth must be \
                 <= iteration count (a head-start of {depth} cannot drain through \
                 fewer iterations). TASK-0217."
            ),
        }
    }
}

/// Which source string [`LinkError::span`] indexes into. Tracked
/// because link errors can originate from EITHER the schedule source
/// (most located variants — `place`/`place_data`/`transfer`/`loop`/
/// `check loop` directives) OR the algorithm source
/// (`UnplacedKernel`, whose offending token is the `kernel K : ...`
/// decl in the algorithm). The driver holds both source strings and
/// uses this tag to pick the right one when rendering `at L:C`.
///
/// Position-less errors ([`LinkError::span`] is `None`) ignore this
/// tag — they render the kind alone, with no fabricated location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkErrorSource {
    /// The byte span indexes into the schedule source string.
    Schedule,
    /// The byte span indexes into the algorithm source string. Only
    /// [`LinkErrorKind::UnplacedKernel`] uses this today.
    Algorithm,
}

/// A link error: a [`LinkErrorKind`] plus, where a single offending
/// source node exists, the byte [`Range`] it was parsed from and which
/// source (`source`) it indexes into (TASK-0099, mirroring the
/// algorithm-side TASK-0090 [`crate::algo::ir::LowerError`] with the
/// added `source` tag because link spans can come from either source).
///
/// # Why a struct wrapping a kind (not `(line, column)` fields per
/// variant)
///
/// Same design decision as TASK-0090: putting a position on a *wrapper*
/// instead of widening every variant means no variant payload shape
/// changed. The existing negative tests still pattern-match
/// `LinkErrorKind::X { payload }` with the same payload, only through
/// `err.kind`. The byte range — not `(line, column)` — is stored
/// because the link step takes `&AlgoIR` + `&SchedIR` only and has no
/// source string; the driver (which holds both source strings)
/// converts via [`crate::error::offset_to_line_col`] at display time
/// through [`LinkError::display_with_src`], exactly as
/// [`crate::error::ParseError`] / [`crate::algo::ir::LowerError`] /
/// [`crate::sched::ir::SchedLowerError`] are surfaced.
///
/// # `span` is `Option` (honest-partial per variant — TASK-0099)
///
/// Most variants name a single offending schedule-AST identifier (the
/// offending `place K on ...` kernel token, `transfer D : ...` data
/// token, `loop V : ...` var token) — `source = Schedule`. The
/// `UnplacedKernel` variant points at the algorithm-AST `kernel K`
/// decl — `source = Algorithm`. The byte spans for all of these were
/// plumbed onto the resolved IR in the prep commit. Exactly one
/// variant is genuinely position-less:
/// [`LinkErrorKind::MissingCrossWorkerTransfer`] — the error is
/// *derived* from joining algorithm dataflow + schedule placements +
/// the *absence* of a transfer directive; there is no single offending
/// source token (the actionable fix is "add a transfer directive", not
/// "fix this token"). A documented missing position is honest; a
/// fabricated one is not.
///
/// # Equality semantics (load-bearing — mirrors `Spanned` / `LowerError`)
///
/// [`PartialEq`] / [`Eq`] are **hand-written to forward to `kind`
/// only**; `span` AND `source` are deliberately EXCLUDED from value
/// identity (they are jointly the positional-noise metadata). Same
/// decision, same rationale as [`crate::span::Spanned`] (TASK-0082) and
/// [`crate::algo::ir::LowerError`] (TASK-0090): position is
/// informational-for-humans, not part of *which semantic error this is*.
/// Excluding both keeps every existing `LinkErrorKind`-asserting
/// negative test valid (they assert the semantic kind + payload, never
/// the byte offset); dedicated tests assert the position separately.
/// `#[derive(PartialEq)]` would (wrongly) fold span + source into
/// equality.
#[derive(Debug, Clone)]
pub struct LinkError {
    /// The semantic violation.
    pub kind: LinkErrorKind,
    /// Byte range into the source identified by `source`, when a
    /// single offending node exists. `None` only for the genuinely
    /// multi-site [`LinkErrorKind::MissingCrossWorkerTransfer`] (see
    /// type docs). Feed `span.start` to
    /// [`crate::error::offset_to_line_col`] for a 1-based
    /// `(line, column)`.
    pub span: Option<Range<usize>>,
    /// Which source the `span` indexes into. Defaults to
    /// [`LinkErrorSource::Schedule`] when `span` is `None`; the value
    /// is unused in that case (it never reaches `display_with_src`'s
    /// indexing path).
    pub source: LinkErrorSource,
}

impl LinkError {
    /// A link error with no source position (the multi-site
    /// `MissingCrossWorkerTransfer` — see type docs). Prefer
    /// [`LinkError::at`] whenever a single offending span is in scope.
    pub fn new(kind: LinkErrorKind) -> Self {
        Self {
            kind,
            span: None,
            source: LinkErrorSource::Schedule,
        }
    }

    /// A link error located at `span` — the byte range of the
    /// offending source node, indexing into `source` — typically read
    /// off one of the span-bearing resolved IR fields
    /// (`ResolvedPlacement.kernel_span`, `ResolvedPlaceData.data_span`,
    /// `ResolvedLoopDirective.var_span`,
    /// `ResolvedTransferDirective.data_span`,
    /// `ResolvedCheckDirective.var_span`, `ResolvedKernel.name_span`).
    pub fn at(kind: LinkErrorKind, span: Range<usize>, source: LinkErrorSource) -> Self {
        Self {
            kind,
            span: Some(span),
            source,
        }
    }

    /// A link error located at `span_opt` if it is `Some` — the common
    /// path through link.rs, where the IR carries `Option<Range<usize>>`
    /// directly. `None` collapses to [`LinkError::new`] (no fabricated
    /// position when the upstream lowering had no source to point at —
    /// e.g. a hand-built test fixture; honest-partial per
    /// [`crate::span::Spanned`] semantics). `source` is unused when
    /// `span_opt` is `None`.
    pub fn maybe_at(
        kind: LinkErrorKind,
        span_opt: Option<Range<usize>>,
        source: LinkErrorSource,
    ) -> Self {
        match span_opt {
            Some(span) => Self::at(kind, span, source),
            None => Self::new(kind),
        }
    }

    /// Render the error with a source location resolved against the
    /// appropriate source string. The driver passes BOTH source
    /// strings; `display_with_src` picks the one matching `self.source`
    /// and feeds its span offset to
    /// [`crate::error::offset_to_line_col`]. Mirrors how
    /// [`crate::algo::ir::LowerError::display_with_src`] is surfaced
    /// (the extra `source`-tagged dispatch is the link-step bit — link
    /// errors can point into either source). When the variant has no
    /// position (see type docs), the message is the kind alone, with
    /// no fabricated location.
    pub fn display_with_src(&self, algo_src: &str, sched_src: &str) -> String {
        match &self.span {
            Some(span) => {
                let src = match self.source {
                    LinkErrorSource::Schedule => sched_src,
                    LinkErrorSource::Algorithm => algo_src,
                };
                let offset = span.start.min(src.len());
                let (line, col) = crate::error::offset_to_line_col(src, offset);
                format!("{} at {line}:{col}", self.kind)
            }
            None => self.kind.to_string(),
        }
    }
}

// Hand-written: forward to `kind`, EXCLUDE `span` from identity
// (TASK-0099, mirroring TASK-0082 / TASK-0090). Deriving would fold
// the span in and break every existing `LinkErrorKind`-asserting
// negative test (and the dedup-via-format-debug in `link()` would
// over-split equal-meaning errors that happened to be at different
// offsets).
impl PartialEq for LinkError {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for LinkError {}

// Span-free Display: library callers / tests without source text get
// the semantic message unchanged. The located form is
// `display_with_src` (driver-side).
impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for LinkError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Resolve schedule references against the algorithm and emit a
/// [`LinkedIR`]. Collects all errors before returning — never fails
/// fast (PRD §12).
///
/// Order of checks within the single pass:
/// 1. `place` directives: name a kernel that doesn't exist (UnknownKernel).
/// 2. `place_data` directives: name a data symbol that doesn't exist
///    (UnknownData).
/// 3. `transfer` directives: name a data symbol that doesn't exist
///    (UnknownTransferData).
/// 4. `loop` and `check loop` directives: name a loop variable not in
///    the algorithm (UnknownLoop).
/// 5. Coverage: every algorithm kernel has a `place` (UnplacedKernel).
/// 6. Cross-worker transfers: every data symbol whose producer and
///    consumer differ has a transfer directive
///    (MissingCrossWorkerTransfer).
///
/// Steps 1–5 can each report many errors. Step 6 depends on the
/// per-kernel placement having a meaningful worker entity, so we only
/// run cross-worker analysis on kernels whose placement IS in the
/// schedule (skipping kernels reported as UnplacedKernel). This is
/// the deliberate "best-effort downstream check" choice: if you
/// haven't placed a kernel, we can't infer where it produces or
/// consumes, so we don't make up workers to chase a follow-on error.
pub fn link(algo: AlgoIR, sched: SchedIR) -> Result<LinkedIR, Vec<LinkError>> {
    let mut errors: Vec<LinkError> = Vec::new();

    // --- 1, 2, 3: name resolution against algorithm declarations ---

    for placement in sched.places.values() {
        if !algo.kernels.contains_key(&placement.kernel) {
            // TASK-0099: span comes from the offending `place K on ...`
            // schedule kernel token (`PlaceDirective.kernel.span` ->
            // ResolvedPlacement.kernel_span). `maybe_at` keeps the
            // hand-built-test path (kernel_span: None) producing an
            // honest position-less error rather than a fabricated 0:0.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownKernel {
                    name: placement.kernel.clone(),
                    suggestion: crate::error::suggest(
                        &placement.kernel,
                        algo.kernels.keys().map(String::as_str),
                    ),
                },
                placement.kernel_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    for pd in sched.place_data.values() {
        if !algo.data.contains_key(&pd.data) {
            // TASK-0099: span from `place_data D in R` data token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownData {
                    name: pd.data.clone(),
                    suggestion: crate::error::suggest(
                        &pd.data,
                        algo.data.keys().map(String::as_str),
                    ),
                },
                pd.data_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    for tx in sched.transfers.values() {
        if !algo.data.contains_key(&tx.data) {
            // TASK-0099: span from `transfer D : ...` data token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownTransferData {
                    name: tx.data.clone(),
                    suggestion: crate::error::suggest(
                        &tx.data,
                        algo.data.keys().map(String::as_str),
                    ),
                },
                tx.data_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    // --- 4: loop variable resolution ---

    let loop_vars = collect_loop_vars(&algo);
    for loop_dir in sched.loops.values() {
        if !loop_vars.contains(&loop_dir.var) {
            // TASK-0099: span from `loop V : ...` loop-var token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownLoop {
                    name: loop_dir.var.clone(),
                    suggestion: crate::error::suggest(
                        &loop_dir.var,
                        loop_vars.iter().map(String::as_str),
                    ),
                },
                loop_dir.var_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }
    for check in sched.checks.values() {
        if !loop_vars.contains(&check.var) {
            // Same diagnostic — the `check loop VAR` and `loop VAR`
            // both name the same algorithm-side variable.
            // TASK-0099: span from `check loop V : ...` loop-var token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownLoop {
                    name: check.var.clone(),
                    suggestion: crate::error::suggest(
                        &check.var,
                        loop_vars.iter().map(String::as_str),
                    ),
                },
                check.var_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    // --- 5: coverage — every kernel has a place ---

    for (kernel_name, kernel_decl) in algo.kernels.iter() {
        if !sched.places.contains_key(kernel_name) {
            // TASK-0099: span comes from the algorithm-side
            // `kernel K : ...` decl identifier — this is the only
            // located link variant whose span is into the ALGORITHM
            // source (not the schedule). The `LinkErrorSource::Algorithm`
            // tag tells `display_with_src` to resolve against the
            // algorithm source string at render time.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnplacedKernel(kernel_name.clone()),
                kernel_decl.name_span.clone(),
                LinkErrorSource::Algorithm,
            ));
        }
    }

    // --- Build the convenience maps and producer/consumer index ---
    //
    // Even when there are errors above, we still build placements/
    // workers for the kernels we CAN resolve — the cross-worker check
    // below uses them. Kernels with no placement are simply skipped.

    let placements: BTreeMap<String, ResolvedPlacement> = sched
        .places
        .iter()
        .filter(|(k, _)| algo.kernels.contains_key(*k))
        .map(|(k, p)| (k.clone(), p.clone()))
        .collect();

    let kernel_workers: BTreeMap<String, WorkerEntity> = placements
        .iter()
        .map(|(k, p)| (k.clone(), WorkerEntity::from_target(&p.target)))
        .collect();

    let (data_producers, data_consumers) = analyse_dataflow(&algo, &kernel_workers);

    // --- 6: cross-worker transfer existence ---

    for (data, producer) in data_producers.iter() {
        if let Some(consumers) = data_consumers.get(data) {
            for consumer in consumers.iter() {
                if consumer != producer && !sched.transfers.contains_key(data) {
                    // TASK-0099: `MissingCrossWorkerTransfer` is the
                    // sole genuinely position-less variant. The error
                    // joins algorithm dataflow + schedule placements +
                    // the *absence* of a transfer directive; there is
                    // no single offending source token (the actionable
                    // fix is "add a transfer directive", not "fix this
                    // token"). A documented missing position is
                    // honest; a fabricated one is not. See type docs.
                    errors.push(LinkError::new(LinkErrorKind::MissingCrossWorkerTransfer {
                        data: data.clone(),
                        producer_worker: producer.display(),
                        consumer_worker: consumer.display(),
                    }));
                }
            }
        }
    }

    // --- 7: pipeline depth vs buffer capacity (TASK-0134) ---
    //
    // For each `loop V : pipeline=D` directive, find every data
    // symbol referenced inside `for V : ...`'s body. For each such
    // symbol that has a `transfer DATA : buffer=N` directive, assert
    // `D <= N`. Otherwise the buffer place's initial_marking would
    // exceed its capacity and `acfg_to_petri` would emit a Petri net
    // the boundedness pass rejects on the spot — moving the
    // diagnostic earlier preserves the {loop_var, data, depth,
    // buffer} naming.
    //
    // We iterate `sched.loops` in BTreeMap order and emit at most one
    // PipelineExceedsBuffer per (loop_var, data) pair, in source-stable
    // order — same determinism contract as every other check above.
    check_pipeline_buffer_constraints(&algo, &sched, &mut errors);

    // Deduplicate identical errors. Possible because two consumers on
    // the same different-entity could each emit the same
    // "MissingCrossWorkerTransfer" if the loop above visited them as
    // separate entries (it won't, BTreeSet collapses; but defensive).
    // Also catches the degenerate "report each kind once" pattern.
    //
    // TASK-0099: sort by `e.kind` debug-format, NOT by `e` debug-format.
    // After the LinkError{kind,span,source} restructure, deriving Debug
    // on the wrapper folds span+source into `{e:?}`, which (a) would
    // sort two same-kind errors at different offsets into different
    // sort buckets (non-determinism leaking via the byte-offset jitter
    // a future test refactor could introduce), and (b) would break
    // `errors.dedup()`: dedup uses our hand-written `PartialEq` (kind
    // only — span+source EXCLUDED), so two same-kind errors are equal
    // by `==` but would be non-adjacent under the wrapper-debug sort,
    // letting the dup survive. Sorting by `.kind` Debug only keeps the
    // pre-TASK-0099 invariant: same-kind errors are adjacent, dedup
    // collapses them.
    errors.sort_by_key(|e| format!("{:?}", e.kind));
    errors.dedup();

    if errors.is_empty() {
        Ok(LinkedIR {
            algo,
            sched,
            placements,
            kernel_workers,
            data_producers,
            data_consumers,
        })
    } else {
        Err(errors)
    }
}

// --------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------

/// Walk every `for VAR : ...` (including nested) and collect the
/// iteration variable names. Used to validate schedule `loop` and
/// `check loop` references.
fn collect_loop_vars(algo: &AlgoIR) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_loop_vars_in_stmts(&algo.stmts, &mut out);
    out
}

fn collect_loop_vars_in_stmts(stmts: &[IrStmt], out: &mut BTreeSet<String>) {
    for s in stmts {
        if let IrStmt::For { var, body, .. } = s {
            out.insert(var.clone());
            collect_loop_vars_in_stmts(body, out);
        }
    }
}

/// Walk every statement and record:
/// - For each `Dataflow { lhs, rhs: Call }`, the producer worker
///   entity of `lhs.name` is the placement of the called kernel.
/// - For each kernel call (in any expression position) or effect
///   statement, every `DataRef` argument's name becomes a consumer
///   of that kernel's worker entity.
///
/// Kernels with no placement (because the schedule didn't place
/// them — already an UnplacedKernel error) are skipped: we don't
/// know where they run, so we don't pretend to.
///
/// Identity-copy dataflow (`D <-- E` where the RHS is a bare
/// `DataRef`, not a `Call`) is currently NOT recorded as a producer
/// edge. No example exercises this, and the right semantics for
/// "kernel-less data move" are part of the partition/transfer pass
/// proper (TASK-0016+). Filed as a limitation.
fn analyse_dataflow(
    algo: &AlgoIR,
    kernel_workers: &BTreeMap<String, WorkerEntity>,
) -> (
    BTreeMap<String, WorkerEntity>,
    BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    let mut producers: BTreeMap<String, WorkerEntity> = BTreeMap::new();
    let mut consumers: BTreeMap<String, BTreeSet<WorkerEntity>> = BTreeMap::new();
    walk_stmts(&algo.stmts, kernel_workers, &mut producers, &mut consumers);
    (producers, consumers)
}

fn walk_stmts(
    stmts: &[IrStmt],
    kernel_workers: &BTreeMap<String, WorkerEntity>,
    producers: &mut BTreeMap<String, WorkerEntity>,
    consumers: &mut BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                match rhs {
                    IrExpr::Call { callee, args } => {
                        if let Some(worker) = kernel_workers.get(callee) {
                            // Producer of lhs.name is this kernel's worker.
                            producers.insert(lhs.name.clone(), worker.clone());
                            // Args read as consumers by the same kernel.
                            for arg in args {
                                collect_dataref_consumers(arg, worker, consumers);
                            }
                            // Indices on lhs may reference iter vars or
                            // consts; data symbols in indices are not
                            // anticipated by the algorithm grammar.
                            // Walking lhs.indices for DataRefs is a no-op
                            // for the current grammar, but defensive:
                            for idx in &lhs.indices {
                                collect_dataref_consumers(idx, worker, consumers);
                            }
                        }
                    }
                    // Identity-copy or arithmetic-only RHS: no kernel,
                    // no recorded producer. See module docstring.
                    other => {
                        // Even with no producer kernel, expressions in
                        // the RHS could refer to data. Without a kernel
                        // we have no worker entity to attribute the
                        // read to, so we cannot record it as a consumer.
                        // Filed as a limitation; not exercised by the
                        // current examples.
                        let _ = other;
                    }
                }
            }
            IrStmt::Effect { callee, args } => {
                if let Some(worker) = kernel_workers.get(callee) {
                    for arg in args {
                        collect_dataref_consumers(arg, worker, consumers);
                    }
                }
            }
            IrStmt::For { body, .. } => {
                walk_stmts(body, kernel_workers, producers, consumers);
            }
        }
    }
}

/// Recursively visit an expression and, for every `DataRef`, record
/// the named data symbol as consumed by `worker`. `Call` expressions
/// inside arbitrary positions also count: the callee's args may
/// reference data, and those reads are attributed to the OUTER call
/// (or here, to `worker`, which is the OUTER call when this helper
/// is invoked).
///
/// Note: a `Call` nested inside an arg expression is semantically
/// possible per the grammar but not exercised by current examples
/// (kernel calls live at statement RHS, not inside other kernel
/// args). When such a case lands, the inner call's worker should
/// arguably be the outer kernel's worker; we walk into args
/// recursively to remain consistent.
fn collect_dataref_consumers(
    e: &IrExpr,
    worker: &WorkerEntity,
    consumers: &mut BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            consumers
                .entry(name.clone())
                .or_default()
                .insert(worker.clone());
            for idx in indices {
                collect_dataref_consumers(idx, worker, consumers);
            }
        }
        IrExpr::Call { args, .. } => {
            for a in args {
                collect_dataref_consumers(a, worker, consumers);
            }
        }
        IrExpr::Neg(inner) => collect_dataref_consumers(inner, worker, consumers),
        IrExpr::BinOp(_, l, r) => {
            collect_dataref_consumers(l, worker, consumers);
            collect_dataref_consumers(r, worker, consumers);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}

// --------------------------------------------------------------------
// TASK-0134: pipeline-depth vs buffer-capacity constraint
// --------------------------------------------------------------------

/// Append [`LinkErrorKind::PipelineExceedsBuffer`] for each `loop V :
/// pipeline=D` whose body contains a cross-worker Push/Wait pair
/// (both producer kernel and consumer kernel inside the loop) for a
/// data symbol with `transfer DATA : buffer=N` where `D > N`.
///
/// "Both endpoints inside" mirrors specifically the
/// `hoist_invariant_waits` semantic in `transfer_inject`:
/// `hoist_invariant_waits` moves the Wait OUT of the loop body for
/// data symbols not produced inside (e.g. `input` in example 13's
/// pipeline-parallel schedule, where `load_input` lives on host
/// outside the loop). When the Wait is hoisted, the IR-level
/// `pipeline_depth_for_seq` annotation no longer applies (the Xfer's
/// tile no longer contains the pipelined iter-var), so the
/// constraint we are policing here also no longer applies.
///
/// **Same-worker carveout (TASK-0214 — closed):** the link check
/// now mirrors `transfer_inject`'s `src == dst` skip. A `transfer X
/// : buffer=N` directive on a same-worker data symbol is harmless
/// (no Xfer is emitted at runtime by `transfer_inject`) and the
/// pipeline-buffer check should not fire on it either, otherwise it
/// would misdirect the user (the actionable bug, if any, is the
/// placement — not the buffer). `data_is_cross_worker(algo, sched,
/// data_name)` is the new gate: it walks the kernels touching
/// `data_name` in `algo.stmts`, looks up each kernel's placement in
/// `sched.places`, and returns true iff MORE THAN ONE distinct
/// worker is named. An unplaced kernel is conservatively treated
/// as cross-worker so a broken-but-cross-worker schedule still
/// gets the pipeline diagnostic.
///
/// Dual-direction check: producer-and-consumer-both-inside (the
/// pipelined inter-stage case, e.g. `feat1`, `feat2` in example 13).
/// One-side-inside-other-outside cases (e.g. `input` flowing INTO
/// the loop from `load_input`, or `output` flowing OUT of the loop
/// to `save_output`) are NOT policed here: the IR will hoist them
/// and the pipeline-depth annotation will not fire.
///
/// Inputs:
/// - `algo` — needed for the body-data-symbol walk + producer/
///   consumer kernel identity per loop body.
/// - `sched` — for the loop directives, transfer buffer values, and
///   kernel placements (to determine which kernels are cross-worker).
/// - `errors` — appended to (one error per (loop_var, data) violation).
///
/// Determinism: outer iteration is `sched.loops.values()` (BTreeMap by
/// loop_var name); the inner check walks a `BTreeSet<String>`.
fn check_pipeline_buffer_constraints(
    algo: &AlgoIR,
    sched: &SchedIR,
    errors: &mut Vec<LinkError>,
) {
    for loop_dir in sched.loops.values() {
        // Find the pipeline depth (if any) on this loop. PRD §6.3.3
        // forbids duplicate option keywords (DuplicateLoopOption), so
        // at most one `Pipeline(_)` survives here; we still iterate
        // defensively so a future grammar relaxation doesn't change
        // the answer.
        let Some(depth) = loop_dir.options.iter().find_map(|opt| match opt {
            ResolvedLoopOption::Pipeline(d) => Some(*d),
            _ => None,
        }) else {
            continue;
        };

        // TASK-0217: pipeline depth D must be <= iteration count of the
        // source `for V : LO .. HI`. With D > iter_count, TASK-0213's
        // path-2 elision drops every Push (there are only iter_count of
        // them, but D > iter_count credits are pre-marked), leaving
        // `D - iter_count` leftover tokens in the buffer place — odd
        // semantics. Reject here so the user sees the precise
        // diagnostic rather than an analysis-net oddity.
        //
        // Iteration count is computed by walking algo.stmts to find the
        // `for V` matching this loop_dir.var and evaluating `hi - lo`
        // via the same const-evaluator the ACFG uses. If either bound
        // is non-const (impossible in v2 grammar — every loop bound is
        // a const expression per PRD §6.2) or the for isn't found
        // (would surface as `UnknownLoop` separately), we skip — this
        // is an independent check that doesn't cascade.
        if let Some(iter_count) = find_loop_iter_count(&algo.stmts, &loop_dir.var, &algo.consts) {
            if (depth as i64) > iter_count {
                // TASK-0099: span from `loop V : pipeline=D` loop-var
                // token — the offending pipeline=D lives on this loop
                // directive.
                errors.push(LinkError::maybe_at(
                    LinkErrorKind::PipelineExceedsIterationCount {
                        loop_var: loop_dir.var.clone(),
                        depth,
                        iteration_count: iter_count,
                    },
                    loop_dir.var_span.clone(),
                    LinkErrorSource::Schedule,
                ));
                // Do NOT continue — if both D > iter_count AND D > buffer,
                // report BOTH. They name different problems with
                // different actionable fixes (raise buffer vs reduce D vs
                // extend the loop).
            }
        }

        // Gather data symbols that are BOTH produced and consumed
        // inside `for VAR : ...`. The intersection is the set of
        // symbols whose Push/Wait pair the IR will keep inside the
        // loop (the IR contract that triggers `initial_marking = D`).
        let mut produced: BTreeSet<String> = BTreeSet::new();
        let mut consumed: BTreeSet<String> = BTreeSet::new();
        collect_data_in_loop(&algo.stmts, &loop_dir.var, false, &mut produced, &mut consumed);

        // Intersect (deterministic — both are BTreeSets).
        for data_name in produced.intersection(&consumed) {
            let Some(tx) = sched.transfers.get(data_name) else {
                continue;
            };
            // TASK-0214: skip same-worker data symbols. The
            // PipelineExceedsBuffer check polices the IR-level
            // initial_marking that `transfer_inject` would attach to
            // a Push/Wait pair — but `transfer_inject` does NOT emit
            // any Xfer when producer and consumer share a worker
            // (`src == dst` skip). A redundant `transfer X : buffer=N`
            // directive on a same-worker symbol therefore has no
            // runtime correlate, and complaining about pipeline depth
            // vs buffer would misdirect the user (the actual issue, if
            // any, is the placement — not the buffer). Mirror the
            // transfer_inject semantic: only police data symbols whose
            // producer + consumer kernels span more than one worker.
            if !data_is_cross_worker(algo, sched, data_name) {
                continue;
            }
            let buffer = tx
                .options
                .iter()
                .find_map(|opt| match opt {
                    ResolvedTransferOption::Buffer(n) => Some(*n),
                    _ => None,
                })
                // Default buffer is 1 (matches TransferPolicy::default,
                // PRD §6.3.4 row `buffer=N`).
                .unwrap_or(1);
            if depth > buffer {
                // TASK-0099: span from `loop V : pipeline=D` loop-var
                // token. Two source tokens are involved (the loop
                // pipeline=D and the transfer buffer=N); we point at
                // the loop because the depth value is what the link
                // pass primarily complains about, mirroring the
                // diagnostic text ("loop `V` has `pipeline=D` but ...").
                // The transfer span is reachable via `tx.data_span` if
                // a future task wants secondary highlighting.
                errors.push(LinkError::maybe_at(
                    LinkErrorKind::PipelineExceedsBuffer {
                        loop_var: loop_dir.var.clone(),
                        data: data_name.clone(),
                        depth,
                        buffer,
                    },
                    loop_dir.var_span.clone(),
                    LinkErrorSource::Schedule,
                ));
            }
        }
    }
}

/// TASK-0214: return `true` iff `data_name` has kernels on MORE THAN
/// ONE distinct worker (i.e., is genuinely cross-worker). When the
/// producer + consumer kernels of a data symbol all share a single
/// worker placement, the data is same-worker and `transfer_inject`'s
/// `src == dst` skip means no Xfer is emitted at runtime — so any
/// pipeline-vs-buffer check on it would misdirect the user.
///
/// Walks `algo.stmts` to find kernels referencing the data symbol on
/// either side of a dataflow stmt or as an arg, then looks up each
/// kernel's placement in `sched.places`. A kernel WITHOUT a placement
/// is treated as unknown — that's a separate `UnplacedKernel` error
/// the link step already reports; here we conservatively count it as
/// "could be on a different worker" so we don't accidentally squelch
/// the pipeline check on a broken schedule.
fn data_is_cross_worker(algo: &AlgoIR, sched: &SchedIR, data_name: &str) -> bool {
    let mut kernels: BTreeSet<String> = BTreeSet::new();
    collect_kernels_touching_data(&algo.stmts, data_name, &mut kernels);
    if kernels.is_empty() {
        // Unreferenced data — let the outer check proceed; the
        // intersect-of-produced-and-consumed already filtered this.
        return true;
    }
    let mut workers: BTreeSet<String> = BTreeSet::new();
    for kernel in &kernels {
        match sched.places.get(kernel) {
            Some(placement) => match &placement.target {
                crate::sched::ir::ResolvedPlaceTarget::One(w) => {
                    workers.insert(w.clone());
                }
                crate::sched::ir::ResolvedPlaceTarget::Many(ws) => {
                    for w in ws {
                        workers.insert(w.clone());
                    }
                }
            },
            None => {
                // Unplaced kernel — conservatively assume distinct
                // worker (don't squelch the check on a broken sched).
                return true;
            }
        }
    }
    workers.len() > 1
}

/// Walk `stmts` and record every kernel name that touches `data_name`
/// — as Dataflow LHS-producing-kernel (RHS expression is a Call), as a
/// Dataflow RHS DataRef inside a kernel's arg list, or as an Effect
/// arg. Recurses into For-bodies.
fn collect_kernels_touching_data(
    stmts: &[IrStmt],
    data_name: &str,
    out: &mut BTreeSet<String>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                let touches_lhs = lhs.name == data_name;
                let touches_rhs = expr_touches_data(rhs, data_name);
                if touches_lhs || touches_rhs {
                    // The kernel responsible is the RHS top-level Call,
                    // if any.
                    if let Some(name) = call_callee(rhs) {
                        out.insert(name.to_string());
                    }
                }
            }
            IrStmt::Effect { callee, args } => {
                if args.iter().any(|a| expr_touches_data(a, data_name)) {
                    out.insert(callee.clone());
                }
            }
            IrStmt::For { body, .. } => {
                collect_kernels_touching_data(body, data_name, out);
            }
        }
    }
}

/// True iff `e` references `data_name` directly or transitively.
fn expr_touches_data(e: &IrExpr, data_name: &str) -> bool {
    match e {
        IrExpr::DataRef(r) => r.name == data_name,
        IrExpr::Call { args, .. } => args.iter().any(|a| expr_touches_data(a, data_name)),
        IrExpr::BinOp(_, l, r) => expr_touches_data(l, data_name) || expr_touches_data(r, data_name),
        IrExpr::Neg(inner) => expr_touches_data(inner, data_name),
        IrExpr::IntLit(_) | IrExpr::Ident(_) => false,
    }
}

/// If `e` is a top-level Call, return the callee name. Otherwise None.
fn call_callee(e: &IrExpr) -> Option<&str> {
    match e {
        IrExpr::Call { callee, .. } => Some(callee.as_str()),
        _ => None,
    }
}

/// TASK-0217: find the iteration count for `for VAR : LO .. HI`
/// matching `target_var` anywhere in `stmts` (recursing into nested
/// for bodies). Returns `Some(hi - lo)` when a matching for is found
/// and BOTH bounds evaluate to a const i64, else `None`.
///
/// `None` covers two cases that callers want to skip (not report):
/// - The named loop doesn't exist (UnknownLoop is reported separately
///   by the link step's loop-resolution pass).
/// - A bound contains a non-const construct (impossible in v2 grammar;
///   defensive against a future relaxation).
fn find_loop_iter_count(
    stmts: &[IrStmt],
    target_var: &str,
    consts: &BTreeMap<String, crate::algo::ResolvedConst>,
) -> Option<i64> {
    for s in stmts {
        if let IrStmt::For { var, lo, hi, body } = s {
            if var == target_var {
                let lo_v = crate::acfg::eval_const(lo, consts)?;
                let hi_v = crate::acfg::eval_const(hi, consts)?;
                // Negative range = zero iterations; saturate to 0 so
                // `pipeline=D > 0` always fires the diagnostic.
                return Some((hi_v - lo_v).max(0));
            }
            if let Some(n) = find_loop_iter_count(body, target_var, consts) {
                return Some(n);
            }
        }
    }
    None
}

/// Walk `stmts` and, when inside (or under) `for var : ...`, record
/// data symbols by side:
/// - `produced`: every Dataflow LHS name (a kernel writes the symbol).
/// - `consumed`: every DataRef name in Dataflow RHS or Effect args
///   (a kernel reads the symbol).
///
/// `inside` is the accumulator: once we enter the target `for`, every
/// nested loop body is also "inside" — pipeline depth propagates to
/// every transfer in the loop's transitive body.
fn collect_data_in_loop(
    stmts: &[IrStmt],
    target_var: &str,
    inside: bool,
    produced: &mut BTreeSet<String>,
    consumed: &mut BTreeSet<String>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } if inside => {
                produced.insert(lhs.name.clone());
                collect_data_refs(rhs, consumed);
                for idx in &lhs.indices {
                    collect_data_refs(idx, consumed);
                }
            }
            IrStmt::Effect { args, .. } if inside => {
                for a in args {
                    collect_data_refs(a, consumed);
                }
            }
            IrStmt::For { var, body, .. } => {
                let now_inside = inside || var == target_var;
                collect_data_in_loop(body, target_var, now_inside, produced, consumed);
            }
            // Stmt outside any enclosing target loop: skip (we only
            // care about transfers happening *inside* the pipelined
            // loop).
            _ => {}
        }
    }
}

/// Recursively visit an expression and record every `DataRef`'s name.
fn collect_data_refs(e: &IrExpr, out: &mut BTreeSet<String>) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            out.insert(name.clone());
            for idx in indices {
                collect_data_refs(idx, out);
            }
        }
        IrExpr::Call { args, .. } => {
            for a in args {
                collect_data_refs(a, out);
            }
        }
        IrExpr::Neg(inner) => collect_data_refs(inner, out),
        IrExpr::BinOp(_, l, r) => {
            collect_data_refs(l, out);
            collect_data_refs(r, out);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}
