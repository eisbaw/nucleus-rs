//! Cell outcome types + matrix discovery: filter/expand the manifest into
//! the concrete `(example × schedule × backend)` cells to run, plus
//! required-coverage and fault-assert orphan detection.
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// Cell outcome
// --------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) enum Status {
    Pass,
    Failed { phase: Phase, detail: String },
    Skipped { reason: String },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Phase {
    Compile,
    Build,
    Run,
    Diff,
    /// Fault-report stderr assertion (TASK-0369). Runs AFTER `Diff`
    /// passes: a `[[fault_assert]]` cell additionally requires that the
    /// run's stderr contains every declared substring (the timing-
    /// independent fault-line shape). A missing substring is a `Fault`
    /// failure, distinct from a numeric `Diff` mismatch so the summary
    /// names the right surface.
    Fault,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Phase::Compile => "compile",
            Phase::Build => "build",
            Phase::Run => "run",
            Phase::Diff => "diff",
            Phase::Fault => "fault",
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Timings {
    pub(crate) compile: Option<Duration>,
    pub(crate) build: Option<Duration>,
    pub(crate) run: Option<Duration>,
}

impl Timings {
    pub(crate) fn total(&self) -> Duration {
        self.compile.unwrap_or_default()
            + self.build.unwrap_or_default()
            + self.run.unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CellResult {
    pub(crate) cell: Cell,
    pub(crate) required: bool,
    pub(crate) status: Status,
    pub(crate) timings: Timings,
    /// `true` iff `maybe_corrupt_wire_for_xbackend` actually rewrote
    /// this cell's emitted `src/wire.rs` (only ever true under
    /// `NUC_XBACKEND_NEGATIVE=1`, and only for mp-tcp-bufsync cells —
    /// `wire.rs` is mp-tcp-EXCLUSIVE). Aggregated across the matrix to
    /// enforce the TASK-0183 zero-corruption guard: under the negative
    /// env gate the run must force a CLEAN exit (so the inverting
    /// recipe FAILs loud) unless at least one tree was genuinely
    /// corrupted — a uniform Skip/no-op must NOT be invertible to OK
    /// (the TASK-0187 partial-silent-neuter lesson, mirrored).
    pub(crate) corrupted: bool,
}

// --------------------------------------------------------------------
// Matrix discovery
// --------------------------------------------------------------------

/// Build the planned list of cells from the manifest. Filters by CLI
/// args. Cells that are listed neither under `[[required]]` nor
/// `[[skip]]` but appear in the discovered (schedule × backend) cross
/// product are *informational* — they run but do not gate exit.
pub(crate) fn plan_cells(paths: &Paths, manifest: &Manifest, args: &Args) -> Result<Vec<PlannedCell>, String> {
    // Discover available schedules per example by listing the
    // `schedules/` directory. The schedule name is the file stem
    // without `.sched`.
    let mut planned: Vec<PlannedCell> = Vec::new();

    // Required/skip declarations carry a milestone tag. The active
    // `--milestone` gate is CUMULATIVE: a cell counts iff its tag is
    // at or before the requested milestone. A required cell whose
    // milestone is OUTSIDE the gate is not flagged required for this
    // run (it must not gate the exit, and the TASK-0163 coverage
    // guard scopes itself with the SAME predicate so it is not a
    // coverage obligation either — see `required_coverage_gaps`).
    let required_map = manifest.required_milestones()?;
    let skip_map = manifest.skip_table()?;
    // Per-cell fault-report stderr assertions (TASK-0369). Structural
    // validation (non-empty list/substrings, no duplicate triple)
    // happens here; the orphan/typo cross-check is `fault_assert_orphans`.
    let fault_assert_map = manifest.fault_assert_table()?;
    // Per-cell perf threshold lookup (TASK-0023.03.02 Stage 3). Built
    // by walking BOTH `[[required]]` and `[[skip]]`. Precedence: a
    // `[[required]]` entry's threshold wins over a `[[skip]]` entry's
    // on the same identity triple (the required declaration is what
    // actually gates exit; a skip threshold is informational either way,
    // so this is the conservative tie-break — but no legitimate manifest
    // should have both). Cell-not-in-map ⇒ no threshold ⇒ no gate.
    let mut perf_threshold_map: std::collections::BTreeMap<Cell, f64> =
        std::collections::BTreeMap::new();
    for s in &manifest.skip {
        if let Some(t) = s.perf_threshold_pct {
            perf_threshold_map.insert(s.cell(), t);
        }
    }
    for r in &manifest.required {
        if let Some(t) = r.perf_threshold_pct {
            // Required overwrites any prior skip-side entry on the same
            // triple — see precedence note above.
            perf_threshold_map.insert(r.cell(), t);
        }
    }

    for ex in &manifest.runnable_examples {
        if let Some(want) = &args.example {
            if want != ex {
                continue;
            }
        }
        let sched_dir = paths.example_dir(ex).join("schedules");
        let entries = match fs::read_dir(&sched_dir) {
            Ok(it) => it,
            Err(e) => {
                return Err(format!(
                    "cannot read schedules dir `{}`: {e}",
                    sched_dir.display()
                ))
            }
        };
        let mut schedules: Vec<String> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("readdir error: {e}"))?;
            let path = entry.path();
            // Schedule files are `<name>.sched.nuc`.
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if let Some(stem) = name.strip_suffix(".sched.nuc") {
                schedules.push(stem.to_string());
            }
        }
        schedules.sort();

        for sched in &schedules {
            if let Some(want) = &args.schedule {
                if want != sched {
                    continue;
                }
            }
            // Tier gate (TASK-0444): `--with-mpi` runs the `mpi_backends`
            // tier INSTEAD of `backends` (mutually exclusive). A tier-1
            // run never plans mpi cells (they need the `.#mpi` shell) and
            // an mpi run never plans tier-1 cells. `required_coverage_gaps`
            // / `fault_assert_orphans` apply the SAME `is_mpi_backend`
            // predicate so the coverage obligation stays in lockstep.
            for backend in manifest.active_backends(args.with_mpi) {
                if let Some(want) = &args.backend {
                    if want != backend {
                        continue;
                    }
                }
                let cell = Cell {
                    example: ex.clone(),
                    schedule: sched.clone(),
                    backend: backend.clone(),
                };
                let req_m = required_map.get(&cell).copied();
                let skip_m = skip_map.get(&cell);

                // Milestone gate semantics:
                //  - No `--milestone`  : unchanged — every discovered
                //    cell is planned (required / skip / informational).
                //  - `--milestone M<k>`: run a TIGHT tier. Only the
                //    in-band required cells and in-band declared skips
                //    are planned; out-of-band cells and purely
                //    informational cells are NOT executed (a milestone
                //    job should run exactly its tier, no noise, all
                //    pass ⇒ exit 0). This is AC#1's "subset the
                //    required set by milestone".
                let in_band_required = req_m.is_some_and(|m| milestone_in_gate(m, args.milestone));
                let in_band_skip =
                    skip_m.is_some_and(|(_, m)| milestone_in_gate(*m, args.milestone));

                // TASK-0444: `--with-mpi` runs a TIGHT tier too (like
                // `--milestone`): only DECLARED cells (in-band required +
                // declared skips) are planned, NOT the bare informational
                // cells. Without this, the mpi tier would build+run every
                // example×schedule on mpi-blocking (~60 cells, most
                // capability-rejected or irrelevant) — minutes of wasted
                // cross-compilation for a focused 3-cell gate. The mpi
                // gate's coverage is exactly its declared cells, the same
                // explicit-declaration discipline the tier-1 matrix uses.
                let tight_tier = args.milestone.is_some() || args.with_mpi;
                if tight_tier && !in_band_required && !in_band_skip {
                    // Out of this tier (milestone band or mpi gate) — do
                    // not plan it.
                    continue;
                }

                // `required` is the in-band-required flag. A skip is
                // honoured only within its milestone band; pairing the
                // two predicates keeps the TASK-0163 coverage guard in
                // lockstep with what actually executes.
                let required = in_band_required;
                let pre_skip = skip_m.and_then(|(reason, m)| {
                    milestone_in_gate(*m, args.milestone).then(|| reason.clone())
                });
                let perf_threshold_pct = perf_threshold_map.get(&cell).copied();
                let fault_assert = fault_assert_map.get(&cell).cloned().unwrap_or_default();
                planned.push(PlannedCell {
                    cell,
                    required,
                    pre_skip,
                    perf_threshold_pct,
                    fault_assert,
                });
            }
        }
    }
    Ok(planned)
}

/// `Clone`: required so `execute_cells_parallel` can snapshot the plan
/// into an `Arc<Vec<PlannedCell>>` shared across workers. All fields
/// are already owned data, so the clone is one heap allocation per cell.
#[derive(Clone)]
pub(crate) struct PlannedCell {
    pub(crate) cell: Cell,
    pub(crate) required: bool,
    pub(crate) pre_skip: Option<String>,
    /// Per-cell perf-regression threshold (TASK-0023.03.02). Plumbed
    /// through from the manifest's `RequiredEntry`/`SkipEntry`. `None`
    /// (the default for cells with no declaration AND for declarations
    /// that omit the key) means "no gate" — the comparator emits an
    /// informational delta row only. `Some(N)` only gates the exit code
    /// when the cell is `required = true`; a breach on a skip-band cell
    /// is informational (see `SkipEntry::perf_threshold_pct`).
    pub(crate) perf_threshold_pct: Option<f64>,
    /// Fault-report stderr substrings this cell must surface (TASK-0369).
    /// Empty (the default for every cell with no `[[fault_assert]]`
    /// declaration) ⇒ no fault check; `run_cell` skips the assertion
    /// entirely, so cells without a declaration are byte-for-byte
    /// unaffected. Non-empty ⇒ after the output.bin diff passes, every
    /// listed substring must appear in the run's stderr or the cell is a
    /// `Phase::Fault` failure. Unused in determinism mode (which never
    /// runs the artefact).
    pub(crate) fault_assert: Vec<String>,
}

/// Return `true` iff `cell` passes the active CLI narrowing flags
/// (`--example`/`--schedule`/`--backend`). Mirrors the per-axis
/// `if want != ... { continue }` filters inside `plan_cells` so the
/// coverage check below scopes itself to exactly the cells a given
/// invocation is responsible for.
///
/// NOTE: the `--milestone` axis is intentionally NOT folded in here —
/// a discovered `Cell` carries no milestone (milestone is metadata of
/// a `[[required]]`/`[[skip]]` declaration). The milestone gate is
/// applied by the caller via `milestone_in_gate` against the
/// declaration's tag, in lockstep with `plan_cells` (which does the
/// same). Folding a non-existent cell milestone in here would be the
/// drift TASK-0163 warns about.
pub(crate) fn cell_matches_filters(cell: &Cell, args: &Args) -> bool {
    if let Some(want) = &args.example {
        if want != &cell.example {
            return false;
        }
    }
    if let Some(want) = &args.schedule {
        if want != &cell.schedule {
            return false;
        }
    }
    if let Some(want) = &args.backend {
        if want != &cell.backend {
            return false;
        }
    }
    true
}

/// Coverage guard for the `[[required]]` matrix (TASK-0163).
///
/// `plan_cells` only ever emits cells for schedule *files* it
/// discovered on disk (`<example>/schedules/<name>.sched.nuc`). A
/// `[[required]]` triple whose schedule does not match any discovered
/// file is therefore never planned, never executed, and so never
/// FAILs — it silently vanishes. A one-character typo in a required
/// schedule name (or a stale required entry after a schedule rename)
/// would delete a gating cell with a fully GREEN `just e2e` / CI.
/// That is a false-negative in the falsifier the whole project + CI
/// trusts, the exact class `determinism-check-negative` exists to
/// prevent but previously unguarded for the required matrix.
///
/// This function returns every required triple that is NOT accounted
/// for. A required triple is *accounted for* iff it is either:
///   - present in the planned set (it will execute, and a real
///     FAIL/PASS verdict gates the exit code as before), OR
///   - present in the manifest's `[[skip]]` table (the exit contract
///     explicitly permits a required cell to not execute when it is
///     a declared skip — see the module-level "Exit status" doc).
///
/// Only the active CLI filter scope is checked: a narrowed run such
/// as `just e2e --example 01-elementwise-add` is not responsible for
/// the 07-matmul required cells and must not be failed for their
/// absence. The bare `just e2e` (no filters) checks the full set.
///
/// `--milestone` is a NARROWING AXIS handled here IN LOCKSTEP with
/// `plan_cells`: a required cell whose milestone tag is OUTSIDE the
/// cumulative gate (`milestone_in_gate`) is not a coverage obligation
/// for this run (it was not flagged required either), and a `[[skip]]`
/// only exempts within its own milestone band. The lockstep is the
/// load-bearing TASK-0163 invariant: if `plan_cells` narrowed by
/// milestone but this guard did not (or vice-versa), a typo'd/stale
/// M3-tagged required cell run under `--milestone M3` would silently
/// vanish — the exact blind spot TASK-0163 closed, reopened per
/// milestone subset. The shared `milestone_in_gate` predicate makes
/// the two physically the same rule.
///
/// Returned gaps are de-duplicated and sorted so the error surface is
/// deterministic regardless of manifest ordering.
pub(crate) fn required_coverage_gaps(
    manifest: &Manifest,
    planned: &[PlannedCell],
    args: &Args,
) -> Result<Vec<Cell>, String> {
    let planned_set: BTreeSet<&Cell> = planned.iter().map(|p| &p.cell).collect();

    // Skips that are in milestone-band for THIS run. A skip for an
    // M3-only cell must not exempt anything under `--milestone M1` —
    // mirror plan_cells' skip gating exactly.
    let skip_table = manifest.skip_table()?;
    let in_band_skips: BTreeSet<Cell> = skip_table
        .iter()
        .filter(|(_, (_, m))| milestone_in_gate(*m, args.milestone))
        .map(|(c, _)| c.clone())
        .collect();

    let mut gaps: BTreeSet<Cell> = BTreeSet::new();
    for req in &manifest.required {
        let cell = req.cell();
        let m = Milestone::parse(&req.milestone).map_err(|e| {
            format!(
                "[[required]] (example={}, schedule={}, backend={}): {e}",
                req.example, req.schedule, req.backend
            )
        })?;
        if !cell_matches_filters(&cell, args) {
            // Out of example/schedule/backend scope.
            continue;
        }
        if manifest.is_mpi_backend(&cell.backend) != args.with_mpi {
            // Tier gate (TASK-0444), lockstep with plan_cells'
            // `active_backends` selection: an mpi-tier required cell is a
            // coverage obligation ONLY under `--with-mpi`, and a tier-1
            // required cell only when NOT `--with-mpi`. A cell out of the
            // active tier was never planned, so it must not be flagged a
            // gap (that would make `just e2e` fail on the M7 cells it
            // deliberately does not run in the default shell).
            continue;
        }
        if !milestone_in_gate(m, args.milestone) {
            // Out of the cumulative milestone band — NOT a gating
            // cell this run (plan_cells did not flag it required
            // either). Lockstep with plan_cells.
            continue;
        }
        if planned_set.contains(&cell) {
            continue; // Will execute; its verdict gates the exit.
        }
        if in_band_skips.contains(&cell) {
            continue; // Declared skip in-band — exempt by the contract.
        }
        gaps.insert(cell);
    }
    Ok(gaps.into_iter().collect())
}

/// Dead-fault-assert guard for `[[fault_assert]]` (TASK-0369), the
/// sibling of `required_coverage_gaps`. A fault assertion that can never
/// fire silently checks NOTHING — a fault-path gate that is green for the
/// wrong reason, the same silent-vanish class TASK-0163 closed for
/// `[[required]]`. Return the in-scope fault-assert triples that will
/// never run so the caller can hard-fail BEFORE building cells.
///
/// A fault assert can never fire in three ways; scoping mirrors
/// `required_coverage_gaps` so the two stay in lockstep:
///   - out of `--example`/`--schedule`/`--backend` scope ⇒ exempt (this
///     run is simply not responsible for the cell);
///   - matches a planned cell that genuinely RUNS (no `pre_skip`) ⇒ fine,
///     the Phase-5 fault check fires for it;
///   - matches a planned cell that is `pre_skip`'d ⇒ FLAG: a skipped cell
///     short-circuits in `run_cell` before the artefact runs, so the
///     fault check never executes — asserting on it is a dead no-op
///     (the silent hole moves from "typo" to "lands on a `[[skip]]`",
///     same class; mped-architect P2, cycle-222);
///   - matches no planned cell AND its matching `[[required]]` entry is
///     out of the cumulative milestone band ⇒ legitimately not run this
///     tier, exempt (the fault assert has no milestone of its own; it
///     inherits the required cell's band);
///   - matches no planned cell otherwise ⇒ FLAG: a typo or a fault_assert
///     on a cell that no longer exists.
pub(crate) fn fault_assert_orphans(
    manifest: &Manifest,
    planned: &[PlannedCell],
    args: &Args,
) -> Result<Vec<Cell>, String> {
    let required_map = manifest.required_milestones()?;

    let mut orphans: BTreeSet<Cell> = BTreeSet::new();
    for fa in &manifest.fault_assert {
        let cell = fa.cell();
        if !cell_matches_filters(&cell, args) {
            continue;
        }
        if manifest.is_mpi_backend(&cell.backend) != args.with_mpi {
            // Tier gate (TASK-0444), lockstep with plan_cells /
            // required_coverage_gaps: a fault assert on a cell outside
            // the active tier is not a dead no-op for this run — the
            // cell legitimately does not run here (it belongs to the
            // other tier's gate). No mpi `[[fault_assert]]` exists today;
            // this keeps the three call sites from drifting if one lands.
            continue;
        }
        match planned.iter().find(|p| p.cell == cell) {
            // Planned AND runs: the Phase-5 fault check will fire. Fine.
            Some(pc) if pc.pre_skip.is_none() => continue,
            // Planned but pre_skip'd: never runs the artefact, so the
            // fault assertion is dead. Flag it (loud > silent no-op).
            Some(_) => {
                orphans.insert(cell);
            }
            // Not planned. Legitimate iff a matching required entry is out
            // of the active milestone band (same reason plan_cells dropped
            // it); otherwise it is a typo / stale triple.
            None => {
                if let Some(m) = required_map.get(&cell) {
                    if !milestone_in_gate(*m, args.milestone) {
                        continue;
                    }
                }
                orphans.insert(cell);
            }
        }
    }
    Ok(orphans.into_iter().collect())
}

