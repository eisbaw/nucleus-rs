//! Matrix manifest + capability-sniff config types, deserialised from
//! `e2e-matrix.toml` / per-backend `capabilities.toml`.
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// Manifest
// --------------------------------------------------------------------

/// Top-level matrix manifest, deserialised from
/// `nuc-nucleus/e2e-matrix.toml`. Schema mirrors that file's header
/// comments.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) runnable_examples: Vec<String>,
    pub(crate) backends: Vec<String>,
    /// Tier-2 (M7) MPI backends, kept SEPARATE from `backends` because
    /// they require the `.#mpi` dev shell (rsmpi build deps + a
    /// localhost MPI launcher) the default-shell matrix and `just ci`
    /// deliberately lack (TASK-0063/0068 tiered-shell design). They run
    /// ONLY under the `--with-mpi` flag (`just e2e-mpi`), which scopes a
    /// run to THIS tier INSTEAD of `backends` — mutually exclusive in a
    /// single invocation, mirroring the renode-multimcu-gate's
    /// out-of-default-matrix separation. `#[serde(default)]` so a
    /// manifest without the key is byte-identical to the pre-feature
    /// shape and bare `just e2e` is unaffected. TASK-0444.
    #[serde(default)]
    pub(crate) mpi_backends: Vec<String>,
    #[serde(default)]
    pub(crate) required: Vec<RequiredEntry>,
    #[serde(default)]
    pub(crate) skip: Vec<SkipEntry>,
    /// Per-cell fault-report stderr assertions (TASK-0369). `#[serde(default)]`
    /// so a manifest with no fault asserts is byte-identical to the
    /// pre-feature shape and bare `just e2e` is unaffected.
    #[serde(default)]
    pub(crate) fault_assert: Vec<FaultAssert>,
}

/// The (example, schedule, backend) identity triple. This is the
/// matrix coordinate the harness matches discovered cells against and
/// uses as a `BTreeSet` key. `milestone` is deliberately NOT a field
/// here: a cell discovered on disk has no milestone, and milestone is
/// metadata of a `[[required]]`/`[[skip]]` *declaration*, not part of
/// a cell's identity. Keeping the identity triple separate from the
/// declaration metadata is what lets `required_coverage_gaps` match a
/// required declaration to a planned cell by triple alone.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cell {
    pub(crate) example: String,
    pub(crate) schedule: String,
    pub(crate) backend: String,
}

/// A `[[required]]` declaration: the identity triple PLUS the
/// milestone at which this cell became (or will become) a mandatory
/// gating cell. `milestone` is parsed and validated into a
/// [`Milestone`] at manifest-load time so a typo'd milestone tag
/// fails LOUD (typed error) rather than silently mis-bucketing a
/// gating cell — see `Manifest::required_milestones` / `skip_table`
/// / `Milestone::parse`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequiredEntry {
    pub(crate) example: String,
    pub(crate) schedule: String,
    pub(crate) backend: String,
    /// Milestone tag, e.g. "M1"/"M2"/"M3". The scheme (documented in
    /// the manifest header) is "the milestone whose acceptance task
    /// owns this cell" per PRD §11.
    pub(crate) milestone: String,
    /// Optional per-cell perf-regression gate (TASK-0023.03.02, Stage 3).
    /// When `--baseline` is set AND this is `Some(N)`, a current-vs-
    /// baseline relative-pct delta exceeding `N%` flips the cell into a
    /// REGRESSION row AND (because this is a `[[required]]` entry) hard-
    /// fails the harness exit code. `None` (default — `#[serde(default)]`
    /// so absent in TOML is byte-identical to today) ⇒ no gate, the
    /// delta is informational only. Relative-pct chosen over absolute-ms
    /// for the first cut; absolute is a follow-on if needed.
    #[serde(default)]
    pub(crate) perf_threshold_pct: Option<f64>,
}

impl RequiredEntry {
    /// The identity triple this declaration refers to.
    pub(crate) fn cell(&self) -> Cell {
        Cell {
            example: self.example.clone(),
            schedule: self.schedule.clone(),
            backend: self.backend.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkipEntry {
    pub(crate) example: String,
    pub(crate) schedule: String,
    pub(crate) backend: String,
    pub(crate) reason: String,
    /// Milestone tag for the cell this skip exempts. A `[[skip]]`
    /// carries a milestone so that, when a milestone subset is run,
    /// the coverage guard scopes skips to the same milestone band as
    /// the required cells it exempts (a skip for an M3-only cell must
    /// not exempt anything under `--milestone M1`).
    pub(crate) milestone: String,
    /// Optional per-cell perf-regression gate (TASK-0023.03.02, Stage 3).
    /// Same semantics as on `[[required]]`, EXCEPT a breach on a
    /// `[[skip]]` cell is informational only (no exit-code impact) — a
    /// skipped cell did not run a meaningful payload, so timings are
    /// noise; gating off them would be a false-positive. The field
    /// exists on `[[skip]]` purely so a future un-skip flip preserves
    /// the threshold without a separate edit.
    #[serde(default)]
    pub(crate) perf_threshold_pct: Option<f64>,
}

impl SkipEntry {
    pub(crate) fn cell(&self) -> Cell {
        Cell {
            example: self.example.clone(),
            schedule: self.schedule.clone(),
            backend: self.backend.clone(),
        }
    }
}

/// A `[[fault_assert]]` declaration (TASK-0369). On top of the normal
/// output.bin differential, REQUIRE that the cell's run stderr contains
/// every substring in `stderr_contains`.
///
/// Why this exists: the runtime-assertion / fault-reporting surface
/// (`check loop V : on_violation = count|log`) writes its fault report
/// to STDERR, deliberately — so the cross-backend differential on
/// output.bin stays INDIFFERENT to check-loop presence (PRD §6.3.5).
/// The consequence is that an output.bin-only cell exercising a check
/// loop passes TRIVIALLY: the fault path never enters the comparison at
/// all. This table closes that gap by giving the fault report its own
/// cross-backend assertion.
///
/// Why a SUBSTRING (not a full-line / whole-stderr match): the fault
/// report's occurrence count and any elapsed-ns figure are TIMING-
/// derived (`_check_elapsed = monotonic_ns()-start`) and so NOT robustly
/// bit-identical across backends/runs (the 255-vs-256 band the embedded
/// fixture documents). Only the timing-INDEPENDENT shape is pinnable —
/// presence + loop-var name + threshold-ns echo (TASK-0369 AC#3).
/// Matching a substring also makes the assertion robust to incidental
/// stderr noise (e.g. a `cargo build` warning a multi-process backend's
/// `run.sh` rebuild emits before the program runs).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FaultAssert {
    pub(crate) example: String,
    pub(crate) schedule: String,
    pub(crate) backend: String,
    /// Every listed substring MUST be present in the run's stderr. An
    /// empty list (or an empty substring) is rejected at load
    /// (`fault_assert_table`) — a fault assertion that asserts nothing
    /// is a silent no-op, exactly the false-confidence class TASK-0163
    /// guards against.
    pub(crate) stderr_contains: Vec<String>,
}

impl FaultAssert {
    pub(crate) fn cell(&self) -> Cell {
        Cell {
            example: self.example.clone(),
            schedule: self.schedule.clone(),
            backend: self.backend.clone(),
        }
    }
}

/// A project milestone (PRD §11). Parsed from the `milestone` string
/// on every `[[required]]`/`[[skip]]` entry and from the
/// `--milestone` CLI flag. Ordering is the cumulative-gate ordering:
/// `M1 < M2 < M3`, so `--milestone M3` runs the M1 ∪ M2 ∪ M3 cells.
///
/// The accepted range is the full PRD §11 enum M0..M11: M0..M6 are the
/// tier-1 milestones the matrix gates today; M7..M11 are the future
/// tier-2 (M7/M8 MPI) and tier-3 (M9 embedded skeleton, M10 STM32H7
/// Renode, M11 multi-MCU Renode) milestones. A `[[skip]]` entry
/// deferred to a future milestone tags itself with that milestone
/// (e.g. the embedded_multimcu cells tag M11) so "what is deferred to
/// `M<k>`" stays greppable on the `milestone` field — TASK-0346.
///
/// An unrecognised tag is a typed error (never a panic, never a silent
/// default) — a mis-typed milestone must not silently delete a cell
/// from a gating subset, which is the TASK-0163 failure class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Milestone(pub(crate) u8);

impl Milestone {
    /// The highest milestone the parser accepts — the PRD §11 ceiling
    /// (M11, multi-MCU Renode). Bump this when the PRD adds a tier.
    pub(crate) const MAX: u8 = 11;

    /// Parse "`M<k>`" (k = 0..=11, the full PRD §11 enum). The matrix
    /// only gates M1..M6 today, but the parser accepts the future
    /// tier-2/3 range (M7..M11) so a `[[skip]]`/`[[required]]` entry
    /// can tag its real deferral milestone without a code change here.
    /// Any other shape is a typed error.
    pub(crate) fn parse(s: &str) -> Result<Milestone, String> {
        let rest = s
            .strip_prefix('M')
            .ok_or_else(|| format!("milestone `{s}` is not of the form M<k> (e.g. M1, M2, M3)"))?;
        let k: u8 = rest
            .parse()
            .map_err(|_| format!("milestone `{s}` is not of the form M<k> (e.g. M1, M2, M3)"))?;
        if k > Self::MAX {
            return Err(format!(
                "milestone `{s}` is out of the PRD §11 range M0..M11 (M0..M6 tier-1, M7..M11 tier-2/3)"
            ));
        }
        Ok(Milestone(k))
    }
}

impl fmt::Display for Milestone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "M{}", self.0)
    }
}

/// The cumulative milestone-gate predicate, the SINGLE definition of
/// "is this milestone-tagged cell in scope for this run". Used in
/// BOTH `plan_cells` (deciding required/skip status) and
/// `required_coverage_gaps` (deciding coverage obligation) so the two
/// can never drift — the TASK-0163 lockstep invariant. `None` gate
/// (no `--milestone`) ⇒ everything is in scope (full matrix,
/// unchanged behaviour). Cumulative: `entry <= gate`.
pub(crate) fn milestone_in_gate(entry: Milestone, gate: Option<Milestone>) -> bool {
    match gate {
        None => true,
        Some(g) => entry <= g,
    }
}

impl Manifest {
    /// The backends in scope for THIS run's tier (TASK-0444). The
    /// default-shell matrix runs `backends` (tier-1); `--with-mpi`
    /// runs `mpi_backends` INSTEAD. The two tiers are mutually
    /// exclusive within one invocation — `--with-mpi` is a focused
    /// out-of-default-matrix gate (sibling of renode-multimcu-gate),
    /// not an *additive* one, so it does not re-run the 427 tier-1
    /// cells under the heavier `.#mpi` shell.
    pub(crate) fn active_backends(&self, with_mpi: bool) -> &[String] {
        if with_mpi {
            &self.mpi_backends
        } else {
            &self.backends
        }
    }

    /// True iff `backend` is a tier-2 MPI backend (declared under
    /// `mpi_backends`). The tier-gate predicate: a cell whose backend's
    /// mpi-ness does not match the active `--with-mpi` mode is out of
    /// scope for this run (not planned, not a coverage obligation, not
    /// a dead-fault-assert). Two helpers express the gate from the SAME
    /// `mpi_backends` field, kept consistent by construction — so the
    /// planning and coverage sites cannot drift, the discipline
    /// `milestone_in_gate` enforces for the milestone axis:
    ///   - `plan_cells` selects via `active_backends(with_mpi)` (which
    ///     returns exactly the cells where `is_mpi_backend == with_mpi`);
    ///   - `required_coverage_gaps` and `fault_assert_orphans` call
    ///     `is_mpi_backend` directly to scope their obligation.
    pub(crate) fn is_mpi_backend(&self, backend: &str) -> bool {
        self.mpi_backends.iter().any(|b| b == backend)
    }

    /// Parse + validate every `[[required]]` entry's milestone tag,
    /// returning a `Cell -> Milestone` map. A typo'd milestone tag is
    /// a typed error here (fail loud at load), never a silent
    /// mis-bucket — mis-bucketing would let `--milestone M1`
    /// silently drop a cell that was really M1, the TASK-0163
    /// silent-vanish class generalised to the milestone axis.
    pub(crate) fn required_milestones(&self) -> Result<std::collections::BTreeMap<Cell, Milestone>, String> {
        let mut map = std::collections::BTreeMap::new();
        for r in &self.required {
            let m = Milestone::parse(&r.milestone).map_err(|e| {
                format!(
                    "[[required]] (example={}, schedule={}, backend={}): {e}",
                    r.example, r.schedule, r.backend
                )
            })?;
            map.insert(r.cell(), m);
        }
        Ok(map)
    }

    /// Parse + validate every `[[skip]]` entry, returning a
    /// `Cell -> (reason, Milestone)` map. Same fail-loud contract as
    /// `required_milestones`.
    pub(crate) fn skip_table(&self) -> Result<std::collections::BTreeMap<Cell, (String, Milestone)>, String> {
        let mut map = std::collections::BTreeMap::new();
        for s in &self.skip {
            let m = Milestone::parse(&s.milestone).map_err(|e| {
                format!(
                    "[[skip]] (example={}, schedule={}, backend={}): {e}",
                    s.example, s.schedule, s.backend
                )
            })?;
            map.insert(s.cell(), (s.reason.clone(), m));
        }
        Ok(map)
    }

    /// Parse + validate every `[[fault_assert]]` entry, returning a
    /// `Cell -> Vec<substring>` map (TASK-0369). Fail-loud on:
    ///   - an empty `stderr_contains` list (asserts nothing → silent
    ///     no-op, the TASK-0163 false-confidence class);
    ///   - an empty substring (matches every stderr → also a no-op);
    ///   - a duplicate identity triple (two fault asserts on one cell —
    ///     ambiguous; the author must merge them into one list).
    ///
    /// This is structural validation only. The cross-check that every
    /// fault-assert triple actually corresponds to a planned/required
    /// cell (the orphan/typo guard) is `fault_assert_orphans`, run after
    /// `plan_cells` — mirroring how `required_coverage_gaps` backstops
    /// `[[required]]`.
    pub(crate) fn fault_assert_table(&self) -> Result<std::collections::BTreeMap<Cell, Vec<String>>, String> {
        let mut map: std::collections::BTreeMap<Cell, Vec<String>> =
            std::collections::BTreeMap::new();
        for fa in &self.fault_assert {
            let where_ = || {
                format!(
                    "[[fault_assert]] (example={}, schedule={}, backend={})",
                    fa.example, fa.schedule, fa.backend
                )
            };
            if fa.stderr_contains.is_empty() {
                return Err(format!(
                    "{}: stderr_contains is empty — a fault assertion that \
                     asserts nothing is a silent no-op; list at least the \
                     timing-independent fault-line substring",
                    where_()
                ));
            }
            if fa.stderr_contains.iter().any(|s| s.is_empty()) {
                return Err(format!(
                    "{}: stderr_contains has an empty substring — an empty \
                     substring matches every stderr and asserts nothing",
                    where_()
                ));
            }
            if map.insert(fa.cell(), fa.stderr_contains.clone()).is_some() {
                return Err(format!(
                    "{}: duplicate fault_assert for this identity triple — \
                     merge the substrings into one [[fault_assert]] entry",
                    where_()
                ));
            }
        }
        Ok(map)
    }
}

/// Subset of a backend's `capabilities.toml`. The harness sniffs that
/// the file *parses as TOML* — `nucleus-compiler`'s `load_capabilities`
/// is the authoritative schema validator and the driver invokes it on
/// every compile — PLUS the one field that changes how the *harness*
/// itself runs the artefact: `transport`. A `shared-memory` backend
/// emits one `nuc-generated` binary; a `tcp` (or other multi-process)
/// backend emits N per-worker binaries + a `run.sh` launcher
/// (TASK-0036). The harness must launch the right thing. This is the
/// minimal field set: anything the *driver* validates stays out.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub(crate) struct CapabilitiesSniff {
    /// `shared-memory` (default — single binary) vs `tcp`/etc.
    /// (multi-process — run via `run.sh`). Absent ⇒ treated as
    /// single-binary, preserving the pre-TASK-0036 behaviour exactly.
    pub(crate) transport: Option<String>,
}

impl CapabilitiesSniff {
    /// True when the emitted artefact is a single `nuc-generated`
    /// binary the harness can exec directly. `shared-memory` (or an
    /// absent/unknown transport, conservatively) ⇒ single binary.
    /// Anything else ⇒ multi-process, launched via `run.sh`.
    pub(crate) fn is_single_binary(&self) -> bool {
        matches!(self.transport.as_deref(), None | Some("shared-memory"))
    }
}

