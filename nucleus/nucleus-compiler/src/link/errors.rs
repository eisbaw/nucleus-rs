//! `LinkError` family — the named-error contract surface for the link
//! pass. `LinkErrorKind` is the semantic-violation enum (one variant
//! per contract violation between the algorithm and schedule);
//! `LinkError` is the position-tagged wrapper that adds an optional
//! byte `span` + a `LinkErrorSource` tag picking which source string
//! (algorithm vs schedule) the span indexes into.
//!
//! Equality semantics: `LinkError`'s `PartialEq` is hand-written to
//! forward to `kind` only, deliberately EXCLUDING `span` and `source`
//! from value identity (the position is informational-for-humans, not
//! part of *which semantic error this is*). Mirrors the same decision
//! on [`crate::span::Spanned`] (TASK-0082) and
//! [`crate::algo::ir::LowerError`] (TASK-0090).

use core::ops::Range;

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
                "kernel `{name}` is declared in the algorithm but has no `place` directive in the schedule\n  \
                 help: add a placement to the schedule, e.g. `place {name} on <worker>;` \
                 (replace `<worker>` with one of the schedule's declared workers)"
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
    /// path through [`link`](super::build::link), where the IR carries
    /// `Option<Range<usize>>` directly. `None` collapses to [`LinkError::new`] (no fabricated
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
