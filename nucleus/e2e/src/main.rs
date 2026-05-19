//! `nucleus-e2e` — the differential test matrix harness.
//!
//! Drives every (example × schedule × backend) cell declared by
//! `nuc-nucleus/e2e-matrix.toml`. Each cell:
//!
//!   1. Invoke the `nucleus` driver (`cargo run --bin nucleus -- build ...`)
//!      to lower (algo, sched) to a Cargo project for the given backend.
//!   2. `cargo build --release` the emitted project.
//!   3. Run the resulting `nuc-generated` binary against the example's
//!      `input.bin`, producing `output.bin`.
//!   4. Byte-diff `output.bin` vs `reference.bin`. Bit-identity is the
//!      PRD §10.1 success criterion.
//!
//! A cell that fails any of those phases reports the offending phase
//! (`FAILED (compile)`, `FAILED (build)`, `FAILED (run)`, `FAILED (diff)`).
//! Cells listed in the manifest's `[[skip]]` table report as
//! `SKIPPED` with the manifest's stated reason; cells that the
//! backend's capability matrix cannot satisfy report as
//! `SKIPPED (capability)`.
//!
//! Bare invocation (`just e2e`) runs the full matrix. Flags narrow it:
//!
//!   --example NAME    -- restrict to a single example
//!   --schedule NAME   -- restrict to a single schedule
//!   --backend NAME    -- restrict to a single backend
//!   --milestone ID    -- (currently informational; reserved for the
//!                        post-M1 milestone-tagged subsets in PRD §11)
//!   --check-determinism
//!                     -- for every cell that would normally PASS,
//!                        invoke `nucleus build` twice into separate
//!                        out dirs and byte-compare every generated
//!                        file. Any mismatch is a hard failure. PRD §1
//!                        / §10.1: same source + same backend = same
//!                        emitted code, byte-for-byte. TASK-0033.
//!   -h | --help       -- usage
//!
//! Flag parser is hand-rolled — same style as `nucleus/driver/src/main.rs`.
//! Pulling clap in would be one more workspace-wide dep + one more
//! MSRV constraint to track; for ~5 flags the hand roll is shorter
//! than the cargo plumbing.
//!
//! Exit status: 0 iff every cell listed under `[[required]]` in the
//! manifest reports PASS (or SKIPPED-with-reason from `[[skip]]`).
//! Any required-cell FAIL is non-zero. Informational cells (those not
//! in `[[required]]`) do not influence the exit code.
//!
//! Sequential execution: cells run one after the other. Cargo builds
//! inside `cargo build --release` already saturate available cores for
//! a single build; running multiple cells in parallel mostly contends
//! for the same build cache. Filed as TASK-0151 (follow-up) if matrix
//! growth makes wall-clock matter.
//!
//! TASK-0023.

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

use serde::Deserialize;

// --------------------------------------------------------------------
// Manifest
// --------------------------------------------------------------------

/// Top-level matrix manifest, deserialised from
/// `nuc-nucleus/e2e-matrix.toml`. Schema mirrors that file's header
/// comments.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    runnable_examples: Vec<String>,
    backends: Vec<String>,
    #[serde(default)]
    required: Vec<RequiredEntry>,
    #[serde(default)]
    skip: Vec<SkipEntry>,
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
struct Cell {
    example: String,
    schedule: String,
    backend: String,
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
struct RequiredEntry {
    example: String,
    schedule: String,
    backend: String,
    /// Milestone tag, e.g. "M1"/"M2"/"M3". The scheme (documented in
    /// the manifest header) is "the milestone whose acceptance task
    /// owns this cell" per PRD §11.
    milestone: String,
}

impl RequiredEntry {
    /// The identity triple this declaration refers to.
    fn cell(&self) -> Cell {
        Cell {
            example: self.example.clone(),
            schedule: self.schedule.clone(),
            backend: self.backend.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkipEntry {
    example: String,
    schedule: String,
    backend: String,
    reason: String,
    /// Milestone tag for the cell this skip exempts. A `[[skip]]`
    /// carries a milestone so that, when a milestone subset is run,
    /// the coverage guard scopes skips to the same milestone band as
    /// the required cells it exempts (a skip for an M3-only cell must
    /// not exempt anything under `--milestone M1`).
    milestone: String,
}

impl SkipEntry {
    fn cell(&self) -> Cell {
        Cell {
            example: self.example.clone(),
            schedule: self.schedule.clone(),
            backend: self.backend.clone(),
        }
    }
}

/// A tier-1 milestone (PRD §11). Parsed from the `milestone` string
/// on every `[[required]]`/`[[skip]]` entry and from the
/// `--milestone` CLI flag. Ordering is the cumulative-gate ordering:
/// `M1 < M2 < M3`, so `--milestone M3` runs the M1 ∪ M2 ∪ M3 cells.
///
/// New milestones are added here as the project advances; an
/// unrecognised tag is a typed error (never a panic, never a silent
/// default) — a mis-typed milestone must not silently delete a cell
/// from a gating subset, which is the TASK-0163 failure class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Milestone(u8);

impl Milestone {
    /// Parse "M<k>" (k = 0..=6 for tier-1; the manifest only uses
    /// M1..M3 today, but the parser accepts the full tier-1 range so
    /// adding an M4+ cell does not require a code change here). Any
    /// other shape is a typed error.
    fn parse(s: &str) -> Result<Milestone, String> {
        let rest = s.strip_prefix('M').ok_or_else(|| {
            format!("milestone `{s}` is not of the form M<k> (e.g. M1, M2, M3)")
        })?;
        let k: u8 = rest.parse().map_err(|_| {
            format!("milestone `{s}` is not of the form M<k> (e.g. M1, M2, M3)")
        })?;
        if k > 6 {
            return Err(format!(
                "milestone `{s}` is out of the tier-1 range M0..M6 (PRD §11)"
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
fn milestone_in_gate(entry: Milestone, gate: Option<Milestone>) -> bool {
    match gate {
        None => true,
        Some(g) => entry <= g,
    }
}

impl Manifest {
    /// Parse + validate every `[[required]]` entry's milestone tag,
    /// returning a `Cell -> Milestone` map. A typo'd milestone tag is
    /// a typed error here (fail loud at load), never a silent
    /// mis-bucket — mis-bucketing would let `--milestone M1`
    /// silently drop a cell that was really M1, the TASK-0163
    /// silent-vanish class generalised to the milestone axis.
    fn required_milestones(
        &self,
    ) -> Result<std::collections::BTreeMap<Cell, Milestone>, String> {
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
    fn skip_table(
        &self,
    ) -> Result<std::collections::BTreeMap<Cell, (String, Milestone)>, String> {
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
}

/// Subset of a backend's `capabilities.toml`. The harness sniffs that
/// the file *parses as TOML* — the compiler's `load_capabilities` is
/// the authoritative schema validator and the driver invokes it on
/// every compile — PLUS the one field that changes how the *harness*
/// itself runs the artefact: `transport`. A `shared-memory` backend
/// emits one `nuc-generated` binary; a `tcp` (or other multi-process)
/// backend emits N per-worker binaries + a `run.sh` launcher
/// (TASK-0036). The harness must launch the right thing. This is the
/// minimal field set: anything the *driver* validates stays out.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct CapabilitiesSniff {
    /// `shared-memory` (default — single binary) vs `tcp`/etc.
    /// (multi-process — run via `run.sh`). Absent ⇒ treated as
    /// single-binary, preserving the pre-TASK-0036 behaviour exactly.
    transport: Option<String>,
}

impl CapabilitiesSniff {
    /// True when the emitted artefact is a single `nuc-generated`
    /// binary the harness can exec directly. `shared-memory` (or an
    /// absent/unknown transport, conservatively) ⇒ single binary.
    /// Anything else ⇒ multi-process, launched via `run.sh`.
    fn is_single_binary(&self) -> bool {
        matches!(
            self.transport.as_deref(),
            None | Some("shared-memory")
        )
    }
}

// --------------------------------------------------------------------
// CLI args
// --------------------------------------------------------------------

#[derive(Debug, Default)]
struct Args {
    example: Option<String>,
    schedule: Option<String>,
    backend: Option<String>,
    /// Milestone gate (PRD §11). When set, the required/skip matrix
    /// is narrowed to cells tagged at or before this milestone — the
    /// gate is CUMULATIVE: `--milestone M3` runs the M1 ∪ M2 ∪ M3
    /// required cells (a regression gate should never drop an
    /// earlier-milestone cell). Absent ⇒ the full matrix (unchanged
    /// behaviour). Validated to a [`Milestone`] at parse time so a
    /// bad value fails LOUD before any work.
    milestone: Option<Milestone>,
    /// When set, the harness switches modes: instead of running the
    /// compile/build/run/diff pipeline, it invokes `nucleus build`
    /// twice per cell into two distinct out dirs and byte-compares
    /// every generated file. See TASK-0033 and PRD §1 / §10.1.
    check_determinism: bool,
}

fn parse_args(argv: &[OsString]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < argv.len() {
        let cur = argv[i].to_string_lossy().into_owned();
        let need_val = |idx: usize| -> Result<String, String> {
            argv.get(idx + 1)
                .map(|s| s.to_string_lossy().into_owned())
                .ok_or_else(|| format!("flag `{cur}` requires a value"))
        };
        match cur.as_str() {
            "--example" => {
                a.example = Some(need_val(i)?);
                i += 2;
            }
            "--schedule" => {
                a.schedule = Some(need_val(i)?);
                i += 2;
            }
            "--backend" => {
                a.backend = Some(need_val(i)?);
                i += 2;
            }
            "--milestone" => {
                let raw = need_val(i)?;
                a.milestone = Some(Milestone::parse(&raw)?);
                i += 2;
            }
            "--check-determinism" => {
                a.check_determinism = true;
                i += 1;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag `{other}`; try `--help`")),
        }
    }
    Ok(a)
}

fn print_help() {
    eprintln!(
        "nucleus-e2e — Nuc v2 differential test matrix harness\n\
         \n\
         USAGE:\n    \
             nucleus-e2e [--example NAME] [--schedule NAME] [--backend NAME] \
[--milestone ID] [--check-determinism]\n\
         \n\
         Bare invocation runs every cell declared in\n\
         `nuc-nucleus/e2e-matrix.toml`. Flags narrow the matrix to\n\
         matching cells.\n\
         \n\
         --milestone M<k>: CUMULATIVE milestone gate (PRD §11). Runs\n\
         only the required/skip cells tagged at or before M<k>, so\n\
         `--milestone M3` runs the M1 ∪ M2 ∪ M3 required set (an\n\
         earlier milestone's cells are never dropped from a regression\n\
         gate). No flag = the full matrix. The TASK-0163 required-\n\
         coverage guard is scoped to the SAME milestone band, so a\n\
         typo'd/stale required cell still hard-fails inside its tier.\n\
         \n\
         --check-determinism: for every cell that would normally PASS,\n\
         build twice into distinct out dirs and byte-compare every\n\
         generated file. Verifies PRD §1: same source + same backend\n\
         = same emitted code, byte-for-byte. TASK-0033.\n"
    );
}

// --------------------------------------------------------------------
// Paths
// --------------------------------------------------------------------

/// Layout the harness depends on:
///
///   <repo_root>/
///     nucleus/                  # cargo workspace
///       Cargo.toml
///       e2e/                    # this crate
///       backends/<name>/capabilities.toml
///     nuc-nucleus/
///       e2e-matrix.toml
///       examples/<NN-name>/...
struct Paths {
    repo_root: PathBuf,
}

impl Paths {
    fn discover() -> Result<Self, String> {
        // CARGO_MANIFEST_DIR is set when the binary is invoked via
        // `cargo run`; in that case it points to this crate. We walk
        // up to find the directory that contains both `nucleus/` and
        // `nuc-nucleus/` — the repo root. When invoked directly (no
        // CARGO_MANIFEST_DIR), fall back to the current working
        // directory.
        let mut start = match env::var_os("CARGO_MANIFEST_DIR") {
            Some(s) => PathBuf::from(s),
            None => env::current_dir().map_err(|e| format!("cwd unavailable: {e}"))?,
        };
        loop {
            if start.join("nucleus").join("Cargo.toml").exists()
                && start.join("nuc-nucleus").join("PRD.md").exists()
            {
                return Ok(Paths { repo_root: start });
            }
            if !start.pop() {
                return Err(
                    "could not locate repo root (need both `nucleus/` and `nuc-nucleus/`)"
                        .to_string(),
                );
            }
        }
    }

    fn nucleus_ws(&self) -> PathBuf {
        self.repo_root.join("nucleus")
    }

    fn manifest_path(&self) -> PathBuf {
        self.repo_root.join("nuc-nucleus/e2e-matrix.toml")
    }

    fn example_dir(&self, ex: &str) -> PathBuf {
        self.repo_root.join("nuc-nucleus/examples").join(ex)
    }

    fn backend_caps(&self, backend: &str) -> PathBuf {
        self.repo_root
            .join("nucleus/backends")
            .join(backend)
            .join("capabilities.toml")
    }

    /// Per-cell scratch directory under `nucleus/target/`. `cargo
    /// clean` sweeps it. Removed and recreated on each invocation so
    /// stale artefacts cannot mask a regression.
    fn scratch_dir(&self, ex: &str, sched: &str, backend: &str) -> Result<PathBuf, String> {
        let root = self.repo_root.join("nucleus/target/e2e-matrix");
        fs::create_dir_all(&root)
            .map_err(|e| format!("cannot create scratch root `{}`: {e}", root.display()))?;
        // Cell directory name is sanitised to be filesystem-safe.
        let cell = format!("{ex}__{sched}__{backend}").replace(['/', '\\'], "_");
        let dir = root.join(cell);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create cell dir `{}`: {e}", dir.display()))?;
        Ok(dir)
    }

    /// Determinism-check scratch directory: one per (cell, label).
    /// We emit two of these per cell — labels `a` and `b` — so the
    /// downstream diff has two trees to compare without one
    /// invocation stomping on the other. Lives under
    /// `nucleus/target/e2e-determinism/` so a single `cargo clean`
    /// sweeps the lot.
    fn determinism_dir(
        &self,
        ex: &str,
        sched: &str,
        backend: &str,
        label: &str,
    ) -> Result<PathBuf, String> {
        let root = self.repo_root.join("nucleus/target/e2e-determinism");
        fs::create_dir_all(&root)
            .map_err(|e| format!("cannot create determinism root `{}`: {e}", root.display()))?;
        let cell = format!("{ex}__{sched}__{backend}__{label}").replace(['/', '\\'], "_");
        let dir = root.join(cell);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create cell dir `{}`: {e}", dir.display()))?;
        Ok(dir)
    }
}

// --------------------------------------------------------------------
// Cell outcome
// --------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Status {
    Pass,
    Failed { phase: Phase, detail: String },
    Skipped { reason: String },
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Compile,
    Build,
    Run,
    Diff,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Phase::Compile => "compile",
            Phase::Build => "build",
            Phase::Run => "run",
            Phase::Diff => "diff",
        })
    }
}

#[derive(Debug, Clone, Default)]
struct Timings {
    compile: Option<Duration>,
    build: Option<Duration>,
    run: Option<Duration>,
}

impl Timings {
    fn total(&self) -> Duration {
        self.compile.unwrap_or_default()
            + self.build.unwrap_or_default()
            + self.run.unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
struct CellResult {
    cell: Cell,
    required: bool,
    status: Status,
    timings: Timings,
}

// --------------------------------------------------------------------
// Matrix discovery
// --------------------------------------------------------------------

/// Build the planned list of cells from the manifest. Filters by CLI
/// args. Cells that are listed neither under `[[required]]` nor
/// `[[skip]]` but appear in the discovered (schedule × backend) cross
/// product are *informational* — they run but do not gate exit.
fn plan_cells(paths: &Paths, manifest: &Manifest, args: &Args) -> Result<Vec<PlannedCell>, String> {
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
            for backend in &manifest.backends {
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
                let in_band_required =
                    req_m.is_some_and(|m| milestone_in_gate(m, args.milestone));
                let in_band_skip = skip_m
                    .is_some_and(|(_, m)| milestone_in_gate(*m, args.milestone));

                if args.milestone.is_some() && !in_band_required && !in_band_skip {
                    // Out of this milestone tier — do not plan it.
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
                planned.push(PlannedCell {
                    cell,
                    required,
                    pre_skip,
                });
            }
        }
    }
    Ok(planned)
}

struct PlannedCell {
    cell: Cell,
    required: bool,
    pre_skip: Option<String>,
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
fn cell_matches_filters(cell: &Cell, args: &Args) -> bool {
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
fn required_coverage_gaps(
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

// --------------------------------------------------------------------
// Cell execution
// --------------------------------------------------------------------

fn run_cell(paths: &Paths, planned: &PlannedCell) -> CellResult {
    let cell = planned.cell.clone();

    // Manifest-declared skip wins before we touch the filesystem.
    if let Some(reason) = &planned.pre_skip {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Skipped {
                reason: reason.clone(),
            },
            timings: Timings::default(),
        };
    }

    // Capabilities sniff: the harness does not duplicate the
    // compiler's capability matcher (that's `check_schedule_compat`,
    // and the driver invokes it for us). We only check that the
    // backend's `capabilities.toml` exists and parses; if it doesn't
    // exist, the cell is SKIPPED (capability) — likely a missing
    // backend crate. The full schedule/backend compat check is the
    // driver's responsibility and will surface as a compile-phase
    // failure if it mismatches.
    let caps_path = paths.backend_caps(&cell.backend);
    if !caps_path.exists() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Skipped {
                reason: format!("no capabilities.toml at {}", caps_path.display()),
            },
            timings: Timings::default(),
        };
    }
    // Best-effort load; an unparseable capabilities file is a hard
    // error on the *driver* side. The harness only sniffs existence.
    let caps_src = match fs::read_to_string(&caps_path) {
        Ok(s) => s,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("read {}: {e}", caps_path.display()),
                },
                timings: Timings::default(),
            }
        }
    };
    let caps: CapabilitiesSniff = match toml::from_str(&caps_src) {
        Ok(c) => c,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("parse {}: {e}", caps_path.display()),
                },
                timings: Timings::default(),
            }
        }
    };

    // Sanity-check the example fixtures exist before we burn time on
    // a doomed compile.
    let ex_dir = paths.example_dir(&cell.example);
    let algo = ex_dir.join("prog.algo.nuc");
    let sched = ex_dir
        .join("schedules")
        .join(format!("{}.sched.nuc", cell.schedule));
    let kernels = ex_dir.join("kernels.rs");
    let input_bin = ex_dir.join("input.bin");
    let reference_bin = ex_dir.join("reference.bin");
    for (label, p) in [
        ("algo", &algo),
        ("sched", &sched),
        ("kernels", &kernels),
        ("input.bin", &input_bin),
        ("reference.bin", &reference_bin),
    ] {
        if !p.exists() {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("missing {} at {}", label, p.display()),
                },
                timings: Timings::default(),
            };
        }
    }

    let scratch = match paths.scratch_dir(&cell.example, &cell.schedule, &cell.backend) {
        Ok(p) => p,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: e,
                },
                timings: Timings::default(),
            }
        }
    };

    let mut timings = Timings::default();

    // ---- Phase 1: nucleus build ----------------------------------------
    let t0 = Instant::now();
    let compile = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus")
        .arg("--")
        .arg("build")
        .arg("--algo")
        .arg(&algo)
        .arg("--sched")
        .arg(&sched)
        .arg("--kernels")
        .arg(&kernels)
        .arg("--backend")
        .arg(&cell.backend)
        .arg("--out")
        .arg(&scratch)
        .current_dir(paths.nucleus_ws())
        .output();
    timings.compile = Some(t0.elapsed());
    let compile = match compile {
        Ok(o) => o,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("spawn cargo run: {e}"),
                },
                timings,
            }
        }
    };
    if !compile.status.success() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Compile,
                detail: short_tail(&compile.stderr, &compile.stdout, 4),
            },
            timings,
        };
    }

    // ---- Phase 2: cargo build the emitted project ----------------------
    let t1 = Instant::now();
    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&scratch)
        .output();
    timings.build = Some(t1.elapsed());
    let build = match build {
        Ok(o) => o,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Build,
                    detail: format!("spawn cargo build: {e}"),
                },
                timings,
            }
        }
    };
    if !build.status.success() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Build,
                detail: short_tail(&build.stderr, &build.stdout, 4),
            },
            timings,
        };
    }

    // ---- Phase 3: run the generated artefact ---------------------------
    //
    // Single-binary backend (pthreads-sync, `transport=shared-memory`):
    // exec `target/release/nuc-generated` directly — unchanged from
    // the pre-TASK-0036 path, so backend #1 cannot regress here.
    //
    // Multi-process backend (mp-tcp-bufsync, `transport=tcp`): there
    // is no single `nuc-generated`; the emitted `run.sh` is the entry
    // point — it launches one OS process per worker, wires the
    // loopback ports, waits, and exits non-zero if any worker fails
    // (TASK-0038). The harness invokes `run.sh INPUT OUTPUT` and
    // diffs `output.bin` exactly as for the single-binary case, so
    // the cross-backend differential is a real apples-to-apples
    // comparison (same reference oracle, same diff).
    let output_bin = scratch.join("output.bin");
    let t2 = Instant::now();
    let run = if caps.is_single_binary() {
        let exe = scratch.join("target/release/nuc-generated");
        if !exe.exists() {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Build,
                    detail: format!("expected `nuc-generated` at {}", exe.display()),
                },
                timings,
            };
        }
        Command::new(&exe)
            .env("NUC_INPUT_PATH", &input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin)
            .output()
    } else {
        let run_sh = scratch.join("run.sh");
        if !run_sh.exists() {
            let detail = format!(
                "multi-process backend `{}` emitted no run.sh at {}",
                cell.backend,
                run_sh.display()
            );
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Build,
                    detail,
                },
                timings,
            };
        }
        Command::new("bash")
            .arg(&run_sh)
            .arg(&input_bin)
            .arg(&output_bin)
            .current_dir(&scratch)
            .env("NUC_INPUT_PATH", &input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin)
            .output()
    };
    timings.run = Some(t2.elapsed());
    let run = match run {
        Ok(o) => o,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Run,
                    detail: format!("spawn run artefact: {e}"),
                },
                timings,
            }
        }
    };
    if !run.status.success() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Run,
                detail: short_tail(&run.stderr, &run.stdout, 4),
            },
            timings,
        };
    }

    // ---- Phase 4: diff output vs reference -----------------------------
    let expected = match fs::read(&reference_bin) {
        Ok(b) => b,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Diff,
                    detail: format!("read {}: {e}", reference_bin.display()),
                },
                timings,
            }
        }
    };
    let actual = match fs::read(&output_bin) {
        Ok(b) => b,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Diff,
                    detail: format!("read {}: {e}", output_bin.display()),
                },
                timings,
            }
        }
    };
    if actual.len() != expected.len() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Diff,
                detail: format!("length {} != reference {}", actual.len(), expected.len()),
            },
            timings,
        };
    }
    if actual != expected {
        // Find first differing byte for a useful error message.
        let mismatch_at = actual
            .iter()
            .zip(expected.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Diff,
                detail: format!("first byte differs at offset {mismatch_at}"),
            },
            timings,
        };
    }

    CellResult {
        cell,
        required: planned.required,
        status: Status::Pass,
        timings,
    }
}

// --------------------------------------------------------------------
// Determinism check (TASK-0033)
// --------------------------------------------------------------------
//
// For each cell, invoke `nucleus build` twice into two distinct out
// dirs and byte-compare every generated file. PRD §1 / §10.1 promise:
//   same source + same backend = same emitted code, byte-for-byte.
//
// Pre-skip cells (manifest [[skip]]) and capability-mismatched cells
// are reported as SKIPPED — same as the run-mode contract. We do not
// run the determinism check on cells the run-mode pipeline itself
// would reject; the check is meaningful only on cells that currently
// PASS, because that's the contract the PRD makes.
//
// What is compared:
//   - every regular file under the out dir, walked recursively;
//     paths are relativised against the out dir root so the
//     containing prefix (`out_dir_a/` vs `out_dir_b/`) doesn't
//     spuriously diverge.
//
// What is intentionally NOT compared:
//   - `cargo build` artefacts. Determinism is asserted on the
//     codegen output, not on `target/`. We never invoke `cargo
//     build` in this mode — the cell only goes through phase 1
//     (`nucleus build`) twice, no phase 2/3/4.
//
// Failure mode is loud: the first differing file gets a relative
// path, an offset, and the surrounding byte context (decoded as
// UTF-8 if possible — generated files in v2 are .rs/.toml/.sh, all
// UTF-8). The "fix" for any failure is in the codegen, not the test:
// the most common culprits are HashMap iteration order leaking into
// emitted names/arms, time-of-day comments, or random temp paths.

/// One file's determinism verdict. We don't materialise the full
/// content of every emitted file — for a green run we just need to
/// know how many bytes were compared; for a red run the
/// `MismatchDetail` carries enough context to locate the offending
/// codegen site.
#[derive(Debug, Clone)]
enum DetCellStatus {
    Pass {
        /// Number of regular files compared. Reported so a green run
        /// still shows that *something* was actually compared (zero
        /// files would also be "no mismatch" but is uninteresting).
        files_compared: usize,
    },
    Failed(DetMismatch),
    Skipped {
        reason: String,
    },
}

#[derive(Debug, Clone)]
struct DetMismatch {
    /// Relative path within the out dir that diverged. If the file
    /// exists in one tree but not the other, `kind` is `OnlyInA`/
    /// `OnlyInB` and `offset` is unused.
    relative_path: PathBuf,
    kind: DetMismatchKind,
    /// First differing byte offset (only meaningful for
    /// `BytesDiffer`).
    offset: usize,
    /// Up to ~80 bytes of context around the offset from each tree,
    /// decoded lossy. For OnlyIn* this names the side that *did* have
    /// the file.
    detail: String,
}

#[derive(Debug, Clone, Copy)]
enum DetMismatchKind {
    BytesDiffer,
    LengthDiffers,
    OnlyInA,
    OnlyInB,
}

impl fmt::Display for DetMismatchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DetMismatchKind::BytesDiffer => "bytes differ",
            DetMismatchKind::LengthDiffers => "length differs",
            DetMismatchKind::OnlyInA => "file only in run A",
            DetMismatchKind::OnlyInB => "file only in run B",
        })
    }
}

#[derive(Debug, Clone)]
struct DetCellResult {
    cell: Cell,
    required: bool,
    status: DetCellStatus,
    /// Combined wall-clock of both `nucleus build` invocations.
    elapsed: Duration,
    /// `true` iff `maybe_perturb_for_nondet_test` actually mutated this
    /// cell's `dir_b` tree (only ever true under `NUC_NONDET_TEST=1`).
    /// Aggregated across the matrix to enforce the TASK-0187 AC#2
    /// invariant: under the negative env gate, the `--check-determinism`
    /// run must exit non-zero unless at least one tree was genuinely
    /// perturbed — a uniform `Skipped` must NOT be invertible to OK.
    perturbed: bool,
}

/// Drive the determinism check for one cell. Caller has already
/// filtered the manifest down to cells worth checking. Returns the
/// verdict; never panics.
fn check_cell_determinism(paths: &Paths, planned: &PlannedCell) -> DetCellResult {
    let cell = planned.cell.clone();
    let started = Instant::now();

    // Manifest skip wins — same contract as the run-mode harness.
    if let Some(reason) = &planned.pre_skip {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: reason.clone(),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }

    // Capabilities sniff: a missing or unparseable capabilities file
    // shows up as a SKIPPED (rather than a hard FAIL) in determinism
    // mode, because we have no way to know whether the cell would
    // PASS without it. The run-mode harness treats this as
    // Failed(Compile); for determinism, the cell is simply not in
    // scope.
    let caps_path = paths.backend_caps(&cell.backend);
    if !caps_path.exists() {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: format!("no capabilities.toml at {}", caps_path.display()),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }

    // Sanity-check fixtures (subset of run-mode's check — we don't
    // need input.bin/reference.bin since we never run the binary).
    let ex_dir = paths.example_dir(&cell.example);
    let algo = ex_dir.join("prog.algo.nuc");
    let sched = ex_dir
        .join("schedules")
        .join(format!("{}.sched.nuc", cell.schedule));
    let kernels = ex_dir.join("kernels.rs");
    for (label, p) in [("algo", &algo), ("sched", &sched), ("kernels", &kernels)] {
        if !p.exists() {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("missing {} at {}", label, p.display()),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            };
        }
    }

    // Build into two distinct out dirs.
    let dir_a = match paths.determinism_dir(&cell.example, &cell.schedule, &cell.backend, "a") {
        Ok(p) => p,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("scratch a: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            }
        }
    };
    let dir_b = match paths.determinism_dir(&cell.example, &cell.schedule, &cell.backend, "b") {
        Ok(p) => p,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("scratch b: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            }
        }
    };

    if let Err(e) = run_nucleus_build(paths, &cell, &algo, &sched, &kernels, &dir_a) {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: format!("nucleus build a failed: {e}"),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }
    if let Err(e) = run_nucleus_build(paths, &cell, &algo, &sched, &kernels, &dir_b) {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: format!("nucleus build b failed: {e}"),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }

    // ---- NUC_NONDET_TEST negative-gate perturbation (TASK-0157). ----
    //
    // Relocated here from pthreads-sync `multi_worker.rs` (TASK-0145's
    // inline per-process nonce). Rationale: the e2e determinism harness
    // is the SOLE consumer of NUC_NONDET_TEST (only the justfile
    // `determinism-check-negative` recipe sets it), so production
    // codegen needs no test hook at all — keeping it fully branch-free
    // is the strongest form of AC#1. The old branch made two `nucleus
    // build` *processes* differ via a per-process nonce; the harness
    // already builds twice (dir_a / dir_b), so the clean analogue is to
    // perturb exactly ONE tree post-emit. dir_a is left pristine, dir_b
    // gets a per-process nonce appended -> the trees diverge -> the
    // diff below reports Failed -> `--check-determinism` exits non-zero
    // -> `determinism-check-negative` correctly says "OK ... bit".
    //
    // Runtime env gate (not a cargo feature / `cfg!`): unchanged
    // reasoning from the relocated site — a nested `cargo --features`
    // inside the harness's own `cargo run` does not reliably rebuild
    // against the shared target cache; an env var is read at run time
    // and needs no rebuild. Gated on the exact string "1"; a loud
    // stderr banner so a non-reproducible run is never silent. The bare
    // `determinism-check` path (env unset) does not touch either tree.
    let did_perturb = match maybe_perturb_for_nondet_test(&dir_b) {
        Ok(p) => p,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("NUC_NONDET_TEST perturbation: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            };
        }
    };

    // Diff. We walk dir_a, look each file up in dir_b. After that we
    // sweep dir_b to catch files that exist only in b.
    let files_a = match enumerate_files(&dir_a) {
        Ok(v) => v,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("walk dir a: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            }
        }
    };
    let files_b = match enumerate_files(&dir_b) {
        Ok(v) => v,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("walk dir b: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            }
        }
    };

    let set_a: BTreeSet<&PathBuf> = files_a.iter().collect();
    let set_b: BTreeSet<&PathBuf> = files_b.iter().collect();

    // OnlyInA: take the first one in BTreeSet order so the failure
    // surface is deterministic.
    if let Some(rel) = set_a.difference(&set_b).next() {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Failed(DetMismatch {
                relative_path: (*rel).clone(),
                kind: DetMismatchKind::OnlyInA,
                offset: 0,
                detail: format!(
                    "present in `{}` but not in `{}`",
                    dir_a.display(),
                    dir_b.display()
                ),
            }),
            elapsed: started.elapsed(),
            perturbed: did_perturb,
        };
    }
    if let Some(rel) = set_b.difference(&set_a).next() {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Failed(DetMismatch {
                relative_path: (*rel).clone(),
                kind: DetMismatchKind::OnlyInB,
                offset: 0,
                detail: format!(
                    "present in `{}` but not in `{}`",
                    dir_b.display(),
                    dir_a.display()
                ),
            }),
            elapsed: started.elapsed(),
            perturbed: did_perturb,
        };
    }

    // Now compare bytes. Iterate set_a (== set_b at this point) in
    // BTreeSet order so any failure surface is deterministic.
    let mut files_compared = 0usize;
    for rel in &set_a {
        let path_a = dir_a.join(rel);
        let path_b = dir_b.join(rel);
        let bytes_a = match fs::read(&path_a) {
            Ok(b) => b,
            Err(e) => {
                return DetCellResult {
                    cell,
                    required: planned.required,
                    status: DetCellStatus::Skipped {
                        reason: format!("read {}: {e}", path_a.display()),
                    },
                    elapsed: started.elapsed(),
                    perturbed: did_perturb,
                }
            }
        };
        let bytes_b = match fs::read(&path_b) {
            Ok(b) => b,
            Err(e) => {
                return DetCellResult {
                    cell,
                    required: planned.required,
                    status: DetCellStatus::Skipped {
                        reason: format!("read {}: {e}", path_b.display()),
                    },
                    elapsed: started.elapsed(),
                    perturbed: did_perturb,
                }
            }
        };
        if bytes_a.len() != bytes_b.len() {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Failed(DetMismatch {
                    relative_path: (*rel).clone(),
                    kind: DetMismatchKind::LengthDiffers,
                    offset: 0,
                    detail: format!("len a={} b={}", bytes_a.len(), bytes_b.len()),
                }),
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            };
        }
        if bytes_a != bytes_b {
            let off = bytes_a
                .iter()
                .zip(bytes_b.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Failed(DetMismatch {
                    relative_path: (*rel).clone(),
                    kind: DetMismatchKind::BytesDiffer,
                    offset: off,
                    detail: byte_context(&bytes_a, &bytes_b, off),
                }),
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            };
        }
        files_compared += 1;
    }

    DetCellResult {
        cell,
        required: planned.required,
        status: DetCellStatus::Pass { files_compared },
        elapsed: started.elapsed(),
        perturbed: did_perturb,
    }
}

/// Negative-gate hook for `determinism-check-negative` (TASK-0157,
/// relocated from TASK-0145's inline `multi_worker.rs` branch).
///
/// When `NUC_NONDET_TEST=1`, append a per-process nonce comment to the
/// emitted `Cargo.toml` in `tree` (the *b* build only — `a` is left
/// pristine), so the two determinism trees diverge and the diff bites.
///
/// Returns `Ok(true)` if a tree was actually perturbed, `Ok(false)` if
/// the env gate was unset/other (a strict no-op — the function does not
/// read or touch the tree), `Err` if the gate was set but the target
/// file was missing.
///
/// Why `Cargo.toml` and not `src/main.rs` (TASK-0187, fixing the
/// TASK-0157 partial-silent-neuter): EVERY backend emits `Cargo.toml`
/// at the project root (pthreads-sync lib.rs ~272, mp-tcp-bufsync
/// lib.rs ~132). `src/main.rs` is pthreads-ONLY — mp-tcp-bufsync emits
/// `src/bin/<worker>.rs` and no `main.rs`, so the old target silently
/// `Skipped` all ~13 mp-tcp cells on every run. `Cargo.toml` is
/// backend-layout-agnostic. The injected line is a `#` TOML COMMENT
/// (NOT a Rust `//` comment): `# NUC_NONDET_TEST nonce: …` is valid,
/// inert TOML, so any downstream `cargo` parse of a generated project
/// is unaffected, while `enumerate_files` (which walks the emitted
/// out-dir, Cargo.toml at its root) still diffs it and the negative
/// gate bites. Same per-process nonce (pid+nanos), same exact-`"1"`
/// gate, same loud stderr banner as the relocated TASK-0145/0157 site.
fn maybe_perturb_for_nondet_test(tree: &std::path::Path) -> Result<bool, String> {
    if std::env::var("NUC_NONDET_TEST").as_deref() != Ok("1") {
        return Ok(false);
    }
    eprintln!(
        "nucleus-e2e: WARNING: NUC_NONDET_TEST=1 — injecting a \
         per-process nonce into ONE emitted determinism tree ON PURPOSE \
         to test the determinism check. This run is NOT reproducible. \
         Never set this in a real build (TASK-0145 / TASK-0157 / \
         TASK-0187)."
    );
    let cargo_toml = tree.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(format!(
            "expected emitted `{}` not found — codegen layout drifted; \
             every backend must emit Cargo.toml (TASK-0187)",
            cargo_toml.display()
        ));
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut src = fs::read_to_string(&cargo_toml)
        .map_err(|e| format!("read {}: {e}", cargo_toml.display()))?;
    // `#` comment: valid, inert TOML. A Rust `//` line here would make
    // the generated Cargo.toml unparseable — do not change this.
    src.push_str(&format!(
        "\n# NUC_NONDET_TEST nonce: pid={} nanos={nanos}\n",
        std::process::id()
    ));
    fs::write(&cargo_toml, src).map_err(|e| format!("write {}: {e}", cargo_toml.display()))?;
    Ok(true)
}

/// Invoke `nucleus build` for one cell into `out_dir`. Returns Err
/// with a short tail of the compiler's stderr on failure.
fn run_nucleus_build(
    paths: &Paths,
    cell: &Cell,
    algo: &std::path::Path,
    sched: &std::path::Path,
    kernels: &std::path::Path,
    out_dir: &std::path::Path,
) -> Result<(), String> {
    let out = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus")
        .arg("--")
        .arg("build")
        .arg("--algo")
        .arg(algo)
        .arg("--sched")
        .arg(sched)
        .arg("--kernels")
        .arg(kernels)
        .arg("--backend")
        .arg(&cell.backend)
        .arg("--out")
        .arg(out_dir)
        .current_dir(paths.nucleus_ws())
        .output()
        .map_err(|e| format!("spawn cargo: {e}"))?;
    if !out.status.success() {
        return Err(short_tail(&out.stderr, &out.stdout, 4));
    }
    Ok(())
}

/// Walk `root` recursively, return every regular file's path *relative
/// to root*. Deterministic order is left to the caller (we hand back
/// a `Vec` so a `BTreeSet` can be built outside; this keeps the walk
/// allocation light). Skips no entries — generated trees in v2 are
/// small and we want false positives to be loud.
fn enumerate_files(root: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("read_dir `{}`: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("readdir error: {e}"))?;
            let path = entry.path();
            let ty = entry
                .file_type()
                .map_err(|e| format!("file_type `{}`: {e}", path.display()))?;
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| format!("strip_prefix on `{}`: {e}", path.display()))?
                    .to_path_buf();
                out.push(rel);
            }
            // symlinks etc. are intentionally not followed — codegen
            // doesn't emit them in v2.
        }
    }
    Ok(out)
}

/// Render up to 24 bytes of UTF-8-ish context around byte `off` in
/// both trees. Generated files are .rs/.toml/.sh, so a lossy-decode
/// gives the developer something readable to grep on.
fn byte_context(a: &[u8], b: &[u8], off: usize) -> String {
    let span = 24usize;
    let start = off.saturating_sub(span / 2);
    let end_a = (off + span / 2).min(a.len());
    let end_b = (off + span / 2).min(b.len());
    let snip_a = String::from_utf8_lossy(&a[start..end_a]).replace('\n', "\\n");
    let snip_b = String::from_utf8_lossy(&b[start..end_b]).replace('\n', "\\n");
    format!("a={snip_a:?} b={snip_b:?}")
}

/// Print a determinism-summary table. Same general shape as
/// `print_summary` for the run-mode harness so the two outputs are
/// visually consistent; we don't share code because the columns
/// differ.
fn print_determinism_summary(results: &[DetCellResult]) {
    let colour = use_color();
    let pass = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::GREEN, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let fail = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::RED, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let skip = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::YELLOW, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let dim = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::DIM, ansi::RESET)
        } else {
            s.to_string()
        }
    };

    let ex_w = results
        .iter()
        .map(|r| r.cell.example.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let sc_w = results
        .iter()
        .map(|r| r.cell.schedule.len())
        .max()
        .unwrap_or(8)
        .max(8);
    let be_w = results
        .iter()
        .map(|r| r.cell.backend.len())
        .max()
        .unwrap_or(7)
        .max(7);

    println!();
    println!("e2e determinism matrix (TASK-0033):");
    println!(
        "  {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   detail",
        "example",
        "schedule",
        "backend",
        "status",
        "time",
        ex_w = ex_w,
        sc_w = sc_w,
        be_w = be_w
    );
    println!(
        "  {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   {}",
        "-".repeat(ex_w),
        "-".repeat(sc_w),
        "-".repeat(be_w),
        "-".repeat(10),
        "-".repeat(8),
        "-".repeat(20),
        ex_w = ex_w,
        sc_w = sc_w,
        be_w = be_w
    );

    for r in results {
        let (status_str, detail) = match &r.status {
            DetCellStatus::Pass { files_compared } => (
                pass("PASS"),
                dim(&format!("{files_compared} file(s) byte-identical")),
            ),
            DetCellStatus::Failed(m) => (
                fail("FAIL"),
                format!(
                    "{}: {} (offset {}, {})",
                    m.relative_path.display(),
                    m.kind,
                    m.offset,
                    m.detail
                ),
            ),
            DetCellStatus::Skipped { reason } => (skip("SKIPPED"), dim(reason)),
        };
        let mark = if r.required { "*" } else { " " };
        println!(
            "{mark} {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   {}",
            r.cell.example,
            r.cell.schedule,
            r.cell.backend,
            status_str,
            format_duration(r.elapsed),
            detail,
            ex_w = ex_w,
            sc_w = sc_w,
            be_w = be_w
        );
    }
    println!();
    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Pass { .. }))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Failed(_)))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Skipped { .. }))
        .count();
    println!("  total: {total}   pass: {passed}   fail: {failed}   skipped: {skipped}");
    println!("  (* = required cell; any FAIL is a hard failure regardless of required-bit)");
    println!();
}

/// Return the last `n` non-empty lines of stderr (preferred) or
/// stdout, joined by `; `. Keeps the table compact while still
/// surfacing actionable error context.
fn short_tail(stderr: &[u8], stdout: &[u8], n: usize) -> String {
    let s = if !stderr.is_empty() { stderr } else { stdout };
    let text = String::from_utf8_lossy(s);
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let tail = if lines.len() > n {
        &lines[lines.len() - n..]
    } else {
        &lines[..]
    };
    tail.join("; ")
}

// --------------------------------------------------------------------
// Reporting
// --------------------------------------------------------------------

/// ANSI colour codes. Only used when stdout is a TTY — falls back to
/// plain text under redirection (CI logs stay greppable).
mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const GREEN: &str = "\x1b[32m";
    pub const RED: &str = "\x1b[31m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const DIM: &str = "\x1b[2m";
}

fn use_color() -> bool {
    // We deliberately use *no* extra crate for isatty detection.
    // Honour `NO_COLOR` (the de facto opt-out) and assume colour off
    // when the harness is not invoked from a terminal context. The
    // best signal we have without a crate is `CARGO_TERM_COLOR`
    // (cargo sets `auto`/`always`/`never`) and the absence of
    // `CI`/`NO_COLOR`.
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    match env::var("CARGO_TERM_COLOR").as_deref() {
        Ok("never") => return false,
        Ok("always") => return true,
        _ => {}
    }
    // Default ON. The cells run under `just e2e` which the user
    // typically watches; ANSI codes in `tee` outputs are a minor
    // annoyance compared to losing colour in the common case.
    true
}

fn print_summary(results: &[CellResult]) {
    let colour = use_color();
    let pass = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::GREEN, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let fail = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::RED, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let skip = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::YELLOW, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let dim = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::DIM, ansi::RESET)
        } else {
            s.to_string()
        }
    };

    // Column widths sized off the longest entry, with a minimum so
    // headers don't crowd.
    let ex_w = results
        .iter()
        .map(|r| r.cell.example.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let sc_w = results
        .iter()
        .map(|r| r.cell.schedule.len())
        .max()
        .unwrap_or(8)
        .max(8);
    let be_w = results
        .iter()
        .map(|r| r.cell.backend.len())
        .max()
        .unwrap_or(7)
        .max(7);

    println!();
    println!("e2e matrix:");
    println!(
        "  {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   detail",
        "example",
        "schedule",
        "backend",
        "status",
        "time",
        ex_w = ex_w,
        sc_w = sc_w,
        be_w = be_w
    );
    println!(
        "  {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   {}",
        "-".repeat(ex_w),
        "-".repeat(sc_w),
        "-".repeat(be_w),
        "-".repeat(10),
        "-".repeat(8),
        "-".repeat(20),
        ex_w = ex_w,
        sc_w = sc_w,
        be_w = be_w
    );

    for r in results {
        let (status_str, detail) = match &r.status {
            Status::Pass => (pass("PASS"), String::new()),
            Status::Failed { phase, detail } => (fail(&format!("FAIL/{phase}")), detail.clone()),
            Status::Skipped { reason } => (skip("SKIPPED"), dim(reason)),
        };
        let mark = if r.required {
            // Required cells get an asterisk so a skim sees what
            // gates the exit code.
            "*"
        } else {
            " "
        };
        println!(
            "{mark} {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   {}",
            r.cell.example,
            r.cell.schedule,
            r.cell.backend,
            status_str,
            format_duration(r.timings.total()),
            detail,
            ex_w = ex_w,
            sc_w = sc_w,
            be_w = be_w
        );
    }
    println!();
    let total: usize = results.len();
    let passed: usize = results
        .iter()
        .filter(|r| matches!(r.status, Status::Pass))
        .count();
    let failed: usize = results
        .iter()
        .filter(|r| matches!(r.status, Status::Failed { .. }))
        .count();
    let skipped: usize = results
        .iter()
        .filter(|r| matches!(r.status, Status::Skipped { .. }))
        .count();
    let required_failed: usize = results
        .iter()
        .filter(|r| r.required && matches!(r.status, Status::Failed { .. }))
        .count();
    println!(
        "  total: {total}   pass: {passed}   fail: {failed}   skipped: {skipped}   \
         required-fail: {required_failed}"
    );
    println!("  (* = required cell)");
    println!();
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s >= 10.0 {
        format!("{s:5.1}s")
    } else if s >= 1.0 {
        format!("{s:5.2}s")
    } else {
        format!("{:5}ms", d.as_millis())
    }
}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

fn run() -> Result<i32, String> {
    let argv: Vec<OsString> = env::args_os().skip(1).collect();
    let args = parse_args(&argv)?;

    let paths = Paths::discover()?;
    let manifest_src = fs::read_to_string(paths.manifest_path()).map_err(|e| {
        format!(
            "cannot read manifest at {}: {e}",
            paths.manifest_path().display()
        )
    })?;
    let manifest: Manifest =
        toml::from_str(&manifest_src).map_err(|e| format!("manifest parse error: {e}"))?;

    if let Some(m) = &args.milestone {
        // PRD §11 milestone gate, now genuine (TASK-0167): the
        // required/skip matrix is narrowed CUMULATIVELY to cells
        // tagged at or before this milestone. Announced so a CI log
        // unambiguously records which tier ran.
        eprintln!(
            "nucleus-e2e: milestone gate {m} (cumulative — runs M1..{m} \
             required cells)"
        );
    }

    let planned = plan_cells(&paths, &manifest, &args)?;
    if planned.is_empty() {
        return Err("no cells matched the given filters; nothing to run".to_string());
    }

    // TASK-0163: a `[[required]]` triple whose schedule does not match
    // any discovered `*.sched.nuc` file is never planned and so never
    // FAILs — it silently vanishes, turning a one-char manifest typo
    // into a deleted gating cell with green CI. Hard-fail here, naming
    // every unaccounted-for required triple, BEFORE we spend minutes
    // building cells. Declared `[[skip]]` triples and out-of-filter-
    // scope triples are exempt (see `required_coverage_gaps`). This
    // gate runs in both run-mode and determinism-mode because both
    // trust the required matrix.
    let gaps = required_coverage_gaps(&manifest, &planned, &args)?;
    if !gaps.is_empty() {
        let listed = gaps
            .iter()
            .map(|c| format!("(example={}, schedule={}, backend={})", c.example, c.schedule, c.backend))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "{} required matrix cell(s) in {} were not matched to any \
             discovered schedule and are not declared `[[skip]]`: {}. \
             A `[[required]]` schedule must name an existing \
             `examples/<example>/schedules/<schedule>.sched.nuc` file \
             (check for a typo or a stale entry after a rename), or be \
             moved to `[[skip]]` with a reason.",
            gaps.len(),
            paths.manifest_path().display(),
            listed
        ));
    }

    if args.check_determinism {
        eprintln!(
            "nucleus-e2e: determinism check over {} cell(s) from {}",
            planned.len(),
            paths.manifest_path().display()
        );
        let mut det_results: Vec<DetCellResult> = Vec::with_capacity(planned.len());
        for (i, pc) in planned.iter().enumerate() {
            eprint!(
                "  [{:>2}/{:<2}] {} | {} | {} ... ",
                i + 1,
                planned.len(),
                pc.cell.example,
                pc.cell.schedule,
                pc.cell.backend
            );
            let _ = std::io::stderr().flush();
            let r = check_cell_determinism(&paths, pc);
            match &r.status {
                DetCellStatus::Pass { files_compared } => {
                    eprintln!("PASS ({files_compared} files, {:?})", r.elapsed)
                }
                DetCellStatus::Failed(_) => eprintln!("FAIL"),
                DetCellStatus::Skipped { .. } => eprintln!("SKIPPED"),
            }
            det_results.push(r);
        }
        print_determinism_summary(&det_results);
        let any_failed = det_results
            .iter()
            .any(|r| matches!(r.status, DetCellStatus::Failed(_)));

        // TASK-0187 AC#2 — zero-perturbation guard.
        //
        // Recipe semantics (`just determinism-check-negative`,
        // justfile:69) — note the INVERSION:
        //
        //   if  <harness>; then echo "FAIL: did NOT detect"; exit 1
        //   else                echo "OK: correctly bit"      (exit 0)
        //
        // i.e. the recipe says "OK, the falsifier bit" iff this process
        // exits NON-zero, and "FAIL, it did NOT detect" iff this
        // process exits ZERO. In the normal negative run perturbation
        // succeeds, the trees diverge, cells `Failed`, `any_failed` is
        // true, we exit non-zero -> recipe prints OK. Correct.
        //
        // The partial-silent-neuter (TASK-0157 / the old `src/main.rs`
        // target absent for every mp-tcp cell): those cells `Skipped`,
        // contributing nothing to `any_failed`. If NO cell perturbed
        // AND yet some unrelated cell `Failed`, the recipe would STILL
        // print OK off that unrelated failure while the falsifier
        // touched nothing — a false-confidence green.
        //
        // The invariant we must guarantee: under the negative env gate,
        // the recipe may print OK ONLY IF >=1 tree was actually
        // mutated. We enforce it by making zero perturbations force a
        // CLEAN (exit 0) result REGARDLESS of any incidental `Failed`,
        // so the recipe's `then` branch fires and it prints its loud
        // "FAIL: ... did NOT detect" and exits 1. (Exiting non-zero
        // here would be WRONG — the recipe would invert it to OK.)
        //
        // When the gate is unset, `perturbed` is false for every cell
        // by construction and this whole block is inert: bare
        // `determinism-check` keeps its normal Failed-driven exit and
        // is byte-identical / unaffected.
        if std::env::var("NUC_NONDET_TEST").as_deref() == Ok("1") {
            let perturbed_cells = det_results.iter().filter(|r| r.perturbed).count();
            if perturbed_cells == 0 {
                eprintln!(
                    "nucleus-e2e: FATAL: NUC_NONDET_TEST=1 but ZERO of \
                     {} cell(s) were actually perturbed — the \
                     determinism falsifier touched nothing. Forcing a \
                     CLEAN exit so `determinism-check-negative` reports \
                     its loud FAIL (the falsifier did NOT bite) instead \
                     of inverting a no-op into a false OK (TASK-0187 \
                     AC#2). Likely codegen layout drift: every backend \
                     must emit Cargo.toml.",
                    det_results.len()
                );
                // Exit 0 on purpose: the recipe inverts this into its
                // "FAIL: did NOT detect" branch (exit 1) — a loud,
                // gate-visible failure, never a silent OK.
                return Ok(0);
            }
            eprintln!(
                "nucleus-e2e: NUC_NONDET_TEST=1 — {perturbed_cells} of \
                 {} cell(s) were perturbed (negative-gate sanity: \
                 >=1 required).",
                det_results.len()
            );
        }

        return Ok(if any_failed { 1 } else { 0 });
    }

    eprintln!(
        "nucleus-e2e: running {} cell(s) from {}",
        planned.len(),
        paths.manifest_path().display()
    );

    let mut results: Vec<CellResult> = Vec::with_capacity(planned.len());
    for (i, pc) in planned.iter().enumerate() {
        eprint!(
            "  [{:>2}/{:<2}] {} | {} | {} ... ",
            i + 1,
            planned.len(),
            pc.cell.example,
            pc.cell.schedule,
            pc.cell.backend
        );
        let _ = std::io::stderr().flush();
        let r = run_cell(&paths, pc);
        match &r.status {
            Status::Pass => eprintln!("PASS ({:?})", r.timings.total()),
            Status::Failed { phase, .. } => eprintln!("FAIL/{phase}"),
            Status::Skipped { .. } => eprintln!("SKIPPED"),
        }
        results.push(r);
    }

    print_summary(&results);

    let required_failed = results
        .iter()
        .any(|r| r.required && matches!(r.status, Status::Failed { .. }));
    Ok(if required_failed { 1 } else { 0 })
}

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code as u8),
        Err(msg) => {
            eprintln!("nucleus-e2e: error: {msg}");
            ExitCode::FAILURE
        }
    }
}

// --------------------------------------------------------------------
// Tests — synthetic matrix to exercise the harness plumbing without
// running the full real matrix.
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_parser_accepts_all_documented_flags() {
        let argv: Vec<OsString> = [
            "--example",
            "01-elementwise-add",
            "--schedule",
            "naive",
            "--backend",
            "pthreads-sync",
            "--milestone",
            "M1",
        ]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.example.as_deref(), Some("01-elementwise-add"));
        assert_eq!(a.schedule.as_deref(), Some("naive"));
        assert_eq!(a.backend.as_deref(), Some("pthreads-sync"));
        assert_eq!(a.milestone, Some(Milestone(1)));
    }

    #[test]
    fn arg_parser_accepts_check_determinism() {
        let argv = vec![OsString::from("--check-determinism")];
        let a = parse_args(&argv).expect("parse");
        assert!(a.check_determinism, "flag was not picked up");
        // Other flags default to None / false.
        assert!(a.example.is_none());
    }

    #[test]
    fn arg_parser_combines_check_determinism_with_filters() {
        // Determinism mode should compose with the normal narrowing
        // flags so a developer can debug one cell quickly.
        let argv: Vec<OsString> = [
            "--check-determinism",
            "--example",
            "01-elementwise-add",
            "--backend",
            "pthreads-sync",
        ]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
        let a = parse_args(&argv).expect("parse");
        assert!(a.check_determinism);
        assert_eq!(a.example.as_deref(), Some("01-elementwise-add"));
        assert_eq!(a.backend.as_deref(), Some("pthreads-sync"));
    }

    #[test]
    fn enumerate_files_returns_paths_relative_to_root() {
        // Build a small synthetic tree and verify enumerate_files
        // walks it and returns paths relativised to root. The
        // determinism diff relies on this so two trees rooted at
        // different absolute paths compare equal when their contents
        // are byte-identical.
        let tmp =
            std::env::temp_dir().join(format!("nucleus-e2e-enumerate-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).expect("mk src");
        fs::write(tmp.join("Cargo.toml"), b"[package]").expect("write cargo");
        fs::write(tmp.join("src/main.rs"), b"fn main(){}").expect("write main");
        fs::write(tmp.join("src/kernels.rs"), b"// k").expect("write kernels");

        let mut files = enumerate_files(&tmp).expect("walk");
        files.sort();
        assert_eq!(
            files,
            vec![
                PathBuf::from("Cargo.toml"),
                PathBuf::from("src/kernels.rs"),
                PathBuf::from("src/main.rs"),
            ],
            "walk should return relative paths in stable order"
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn determinism_skip_entry_short_circuits() {
        // Manifest-declared skips short-circuit before running
        // nucleus build — same contract as run-mode. Test we never
        // attempt to spawn cargo in this case.
        let paths = Paths::discover().expect("discover");
        let pc = PlannedCell {
            cell: Cell {
                example: "ghost-example".to_string(),
                schedule: "ghost-sched".to_string(),
                backend: "ghost-backend".to_string(),
            },
            required: false,
            pre_skip: Some("manifest says skip".to_string()),
        };
        let r = check_cell_determinism(&paths, &pc);
        match r.status {
            DetCellStatus::Skipped { reason } => assert_eq!(reason, "manifest says skip"),
            other => panic!("expected SKIPPED, got {other:?}"),
        }
    }

    #[test]
    fn determinism_missing_capabilities_is_skipped() {
        // Cells targeting a non-existent backend report SKIPPED
        // (rather than FAILED) — determinism is meaningful only on
        // cells the harness can actually exercise.
        let paths = Paths::discover().expect("discover");
        let pc = PlannedCell {
            cell: Cell {
                example: "01-elementwise-add".to_string(),
                schedule: "naive".to_string(),
                backend: "definitely-not-a-real-backend-xyzzy".to_string(),
            },
            required: false,
            pre_skip: None,
        };
        let r = check_cell_determinism(&paths, &pc);
        match r.status {
            DetCellStatus::Skipped { reason } => {
                assert!(reason.contains("capabilities.toml"), "reason was: {reason}");
            }
            other => panic!("expected SKIPPED, got {other:?}"),
        }
    }

    #[test]
    fn arg_parser_rejects_unknown_flag() {
        let argv = vec![OsString::from("--frobnicate")];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("--frobnicate"), "got: {err}");
    }

    #[test]
    fn arg_parser_rejects_flag_without_value() {
        let argv = vec![OsString::from("--example")];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("requires a value"), "got: {err}");
    }

    #[test]
    fn manifest_roundtrip_minimal() {
        // Smallest valid manifest. The harness must accept zero
        // required and zero skip entries (e.g. a future "compile only"
        // backend with no required-pass cells yet).
        let src = r#"
runnable_examples = ["foo"]
backends = ["bar"]
"#;
        let m: Manifest = toml::from_str(src).expect("parse");
        assert_eq!(m.runnable_examples, vec!["foo"]);
        assert_eq!(m.backends, vec!["bar"]);
        assert!(m.required.is_empty());
        assert!(m.skip.is_empty());
    }

    #[test]
    fn manifest_rejects_unknown_field() {
        // deny_unknown_fields keeps typos from silently flipping the
        // matrix shape.
        let src = r#"
runnable_examples = ["foo"]
backends = ["bar"]
mystery = 42
"#;
        let r = toml::from_str::<Manifest>(src);
        assert!(r.is_err(), "expected parse failure on unknown field");
    }

    #[test]
    fn manifest_actual_file_parses() {
        // The shipped `e2e-matrix.toml` is the load-bearing artefact;
        // a typo there breaks every developer's `just e2e`. This test
        // is the canary.
        let paths = Paths::discover().expect("discover repo root");
        let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
        let m: std::result::Result<Manifest, toml::de::Error> = toml::from_str(&src);
        assert!(m.is_ok(), "parse failed: {:?}", m.err());
        let m = m.unwrap();
        assert!(
            !m.runnable_examples.is_empty(),
            "manifest declares zero runnable examples"
        );
        assert!(!m.backends.is_empty(), "manifest declares zero backends");
    }

    /// Synthetic single-cell matrix: build a manifest in memory that
    /// references just one example/schedule/backend triple known to
    /// be PASS at M1, drive plan_cells, and verify the planner picks
    /// it up exactly once. This exercises the harness's plumbing
    /// (manifest decode + plan_cells + paths discovery) without
    /// running cargo on the cell — the cell *execution* itself is
    /// covered by `compiler/tests/e2e_example_0{1,2,3}.rs`, which
    /// remain authoritative regression catchers per the task brief.
    #[test]
    fn plan_picks_required_cell_when_filtered() {
        let paths = Paths::discover().expect("discover");
        let manifest = Manifest {
            runnable_examples: vec!["01-elementwise-add".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![RequiredEntry {
                example: "01-elementwise-add".to_string(),
                schedule: "naive".to_string(),
                backend: "pthreads-sync".to_string(),
                milestone: "M1".to_string(),
            }],
            skip: vec![],
        };
        let args = Args {
            example: Some("01-elementwise-add".into()),
            schedule: Some("naive".into()),
            backend: Some("pthreads-sync".into()),
            milestone: None,
            check_determinism: false,
        };
        let planned = plan_cells(&paths, &manifest, &args).expect("plan");
        assert_eq!(planned.len(), 1);
        assert!(planned[0].required);
        assert!(planned[0].pre_skip.is_none());
        assert_eq!(planned[0].cell.schedule, "naive");
    }

    // Skip-entry plumbing: a [[skip]] entry produces a SKIPPED
    // status with the manifest's reason, *without* invoking cargo.
    // Drives run_cell directly with a manifest-synthesised skip;
    // validates we bail out before touching the filesystem.
    // ----------------------------------------------------------------
    // TASK-0163: required-matrix coverage guard. These pin the "prove
    // the falsifier works" property in the spirit of
    // `determinism-check-negative`: a required cell that silently
    // vanishes (typo / stale schedule) MUST become a hard error.
    // ----------------------------------------------------------------

    fn cell(ex: &str, sc: &str, be: &str) -> Cell {
        Cell {
            example: ex.to_string(),
            schedule: sc.to_string(),
            backend: be.to_string(),
        }
    }

    fn planned(ex: &str, sc: &str, be: &str) -> PlannedCell {
        PlannedCell {
            cell: cell(ex, sc, be),
            required: true,
            pre_skip: None,
        }
    }

    /// A `[[required]]` declaration tagged at milestone `ms`
    /// (e.g. "M1"). Most coverage-guard tests do not exercise the
    /// milestone axis, so they use "M1" (in-band for every gate).
    fn req(ex: &str, sc: &str, be: &str, ms: &str) -> RequiredEntry {
        RequiredEntry {
            example: ex.to_string(),
            schedule: sc.to_string(),
            backend: be.to_string(),
            milestone: ms.to_string(),
        }
    }

    fn skip_e(ex: &str, sc: &str, be: &str, reason: &str, ms: &str) -> SkipEntry {
        SkipEntry {
            example: ex.to_string(),
            schedule: sc.to_string(),
            backend: be.to_string(),
            reason: reason.to_string(),
            milestone: ms.to_string(),
        }
    }

    /// The core bite: a `[[required]]` triple whose schedule does not
    /// match any planned (discovered) cell, and is not in `[[skip]]`,
    /// is reported as a coverage gap with the exact triple named. This
    /// is the regression for the silent-vanish bug — before the fix
    /// this triple would simply never run and `just e2e` stayed green.
    #[test]
    fn typo_in_required_schedule_is_a_coverage_gap() {
        let manifest = Manifest {
            runnable_examples: vec!["01-elementwise-add".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            // `naiv` is a one-char typo of the real `naive` schedule.
            required: vec![req("01-elementwise-add", "naiv", "pthreads-sync", "M1")],
            skip: vec![],
        };
        // Planner only ever discovers the real `naive` file.
        let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
        let gaps = required_coverage_gaps(&manifest, &plan, &Args::default()).expect("ok");
        assert_eq!(gaps.len(), 1, "typo'd required must surface as a gap");
        assert_eq!(gaps[0], cell("01-elementwise-add", "naiv", "pthreads-sync"));
    }

    /// A required triple that is ALSO listed in `[[skip]]` is exempt —
    /// the exit contract permits a required cell to not execute when
    /// it is a declared skip. Getting this wrong would falsely fail
    /// legitimately-skipped cells (carried-context gotcha).
    #[test]
    fn required_also_in_skip_is_not_a_gap() {
        let manifest = Manifest {
            runnable_examples: vec!["03-reduction".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![req("03-reduction", "distributed", "pthreads-sync", "M1")],
            skip: vec![skip_e(
                "03-reduction",
                "distributed",
                "pthreads-sync",
                "not yet implemented",
                "M1",
            )],
        };
        // Skipped cell is never planned (run_cell would short-circuit
        // even if it were) — the point is the coverage guard must not
        // treat the absence as a gap.
        let plan: Vec<PlannedCell> = vec![];
        let gaps = required_coverage_gaps(&manifest, &plan, &Args::default()).expect("ok");
        assert!(gaps.is_empty(), "skip-declared required must be exempt, got {gaps:?}");
    }

    /// A required triple that IS planned (will execute) is accounted
    /// for — its real PASS/FAIL verdict gates the exit as before. No
    /// gap, no behaviour change for the happy path.
    #[test]
    fn planned_required_is_not_a_gap() {
        let manifest = Manifest {
            runnable_examples: vec!["01-elementwise-add".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![req("01-elementwise-add", "naive", "pthreads-sync", "M1")],
            skip: vec![],
        };
        let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
        let gaps = required_coverage_gaps(&manifest, &plan, &Args::default()).expect("ok");
        assert!(gaps.is_empty(), "planned required must not be a gap, got {gaps:?}");
    }

    /// CLI narrowing must scope the coverage check: `--example 01`
    /// is not responsible for the 07-matmul required cell, so its
    /// absence from the (narrowed) plan must NOT be flagged. Without
    /// this, every `just e2e --example X` would falsely fail.
    #[test]
    fn cli_filter_scopes_coverage_check() {
        let manifest = Manifest {
            runnable_examples: vec!["01-elementwise-add".to_string(), "07-matmul".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![
                req("01-elementwise-add", "naive", "pthreads-sync", "M1"),
                req("07-matmul", "naive", "pthreads-sync", "M1"),
            ],
            skip: vec![],
        };
        // Narrowed run: only the 01 cell is planned.
        let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
        let args = Args {
            example: Some("01-elementwise-add".into()),
            ..Args::default()
        };
        let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
        assert!(
            gaps.is_empty(),
            "out-of-filter-scope required cells must not be gaps, got {gaps:?}"
        );

        // But a typo *within* the filtered example IS still caught.
        let manifest_typo = Manifest {
            required: vec![
                req("01-elementwise-add", "naiv", "pthreads-sync", "M1"),
                req("07-matmul", "naive", "pthreads-sync", "M1"),
            ],
            ..manifest
        };
        let plan2 = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];
        let gaps2 = required_coverage_gaps(&manifest_typo, &plan2, &args).expect("ok");
        assert_eq!(gaps2, vec![cell("01-elementwise-add", "naiv", "pthreads-sync")]);
    }

    /// The shipped `e2e-matrix.toml` against the real discovered
    /// schedules must have ZERO coverage gaps. This pins today's
    /// behaviour: the fix adds a new failure path but must not change
    /// the current 8/0/2 outcome. If a future manifest edit introduces
    /// a typo'd or stale required entry, THIS test (and `just e2e`)
    /// goes red — the durable guard.
    #[test]
    fn real_manifest_has_no_coverage_gaps() {
        let paths = Paths::discover().expect("discover repo root");
        let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
        let manifest: Manifest = toml::from_str(&src).expect("parse manifest");
        let args = Args::default();
        let plan = plan_cells(&paths, &manifest, &args).expect("plan");
        let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
        assert!(
            gaps.is_empty(),
            "shipped e2e-matrix.toml has unmatched required cells: {gaps:?}"
        );
    }

    // ----------------------------------------------------------------
    // TASK-0167: genuine `--milestone` parameterisation. The milestone
    // gate is a NEW narrowing axis; these pin (a) the cumulative
    // subsetting, (b) the typed-error on a bad milestone, and (c) —
    // the load-bearing one — that the TASK-0163 coverage guard is kept
    // in LOCKSTEP, i.e. a typo'd/stale milestone-tagged required cell
    // run under its `--milestone` still hard-fails with the triple
    // named (the silent-vanish blind spot must NOT reopen per subset).
    // ----------------------------------------------------------------

    /// `Milestone::parse` accepts the documented `M<k>` shape and
    /// rejects everything else with a typed error (never a panic,
    /// never a silent default — a mis-typed milestone must not
    /// silently mis-bucket / delete a gating cell).
    #[test]
    fn milestone_parse_accepts_valid_rejects_garbage() {
        assert_eq!(Milestone::parse("M1").unwrap(), Milestone(1));
        assert_eq!(Milestone::parse("M3").unwrap(), Milestone(3));
        assert_eq!(Milestone::parse("M0").unwrap(), Milestone(0));
        for bad in ["m1", "M", "3", "MX", "M-1", "M99", "", "M1.0", "milestone1"] {
            assert!(
                Milestone::parse(bad).is_err(),
                "`{bad}` must be a typed error, not silently accepted"
            );
        }
    }

    /// `--milestone` is parsed and validated at CLI parse time: a bad
    /// value fails LOUD before any work, not accepted-and-ignored.
    #[test]
    fn arg_parser_rejects_bad_milestone() {
        let argv = vec![OsString::from("--milestone"), OsString::from("M9")];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("tier-1 range"), "got: {err}");
        let argv2 = vec![OsString::from("--milestone"), OsString::from("banana")];
        assert!(parse_args(&argv2).is_err());
    }

    /// Cumulative subsetting: a milestone-tagged required matrix run
    /// under `--milestone M2` keeps M1 ∪ M2 cells and drops M3 ones;
    /// `--milestone M3` keeps everything; no flag keeps everything.
    /// This is AC#1's "required-set is subset by milestone".
    #[test]
    fn milestone_gate_is_cumulative_over_required_flagging() {
        let paths = Paths::discover().expect("discover");
        let manifest = Manifest {
            runnable_examples: vec!["01-elementwise-add".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![req("01-elementwise-add", "naive", "pthreads-sync", "M3")],
            skip: vec![],
        };
        // No gate: the single discovered cell is flagged required.
        let p_full = plan_cells(&paths, &manifest, &Args::default()).expect("plan");
        assert!(p_full.iter().any(|c| c.required));

        // --milestone M1: the only required cell is M3-tagged ⇒ it is
        // OUTSIDE the cumulative band ⇒ not flagged required (it still
        // runs as informational, but does not gate the exit).
        let m1 = Args {
            milestone: Some(Milestone(1)),
            ..Args::default()
        };
        let p_m1 = plan_cells(&paths, &manifest, &m1).expect("plan");
        assert!(
            !p_m1.iter().any(|c| c.required),
            "M3 cell must NOT be required under --milestone M1"
        );

        // --milestone M3: in-band ⇒ required again.
        let m3 = Args {
            milestone: Some(Milestone(3)),
            ..Args::default()
        };
        let p_m3 = plan_cells(&paths, &manifest, &m3).expect("plan");
        assert!(
            p_m3.iter().any(|c| c.required),
            "M3 cell must be required under --milestone M3 (cumulative)"
        );
    }

    /// THE LOCKSTEP REGRESSION (mirrors
    /// `typo_in_required_schedule_is_a_coverage_gap` but on the
    /// milestone axis). An M3-tagged `[[required]]` whose schedule is
    /// typo'd, run with `--milestone M3`, MUST still surface as a
    /// coverage gap with the exact triple named. If `plan_cells`
    /// narrowed by milestone but the guard did not (or the guard's
    /// milestone predicate diverged from plan_cells'), this cell would
    /// silently vanish from the M3 subset — the precise TASK-0163
    /// blind spot, reopened per milestone. This test fails if the two
    /// ever drift.
    #[test]
    fn typo_in_milestone_tagged_required_is_a_gap_under_that_milestone() {
        let manifest = Manifest {
            runnable_examples: vec!["06-separable-filter".to_string()],
            backends: vec!["mp-tcp-bufsync".to_string()],
            // `naiv` is a one-char typo; this cell is tagged M3.
            required: vec![req("06-separable-filter", "naiv", "mp-tcp-bufsync", "M3")],
            skip: vec![],
        };
        // Planner only ever discovers the real `naive` file.
        let plan = vec![planned("06-separable-filter", "naive", "mp-tcp-bufsync")];
        // Run scoped to exactly this cell's milestone.
        let args = Args {
            milestone: Some(Milestone(3)),
            ..Args::default()
        };
        let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
        assert_eq!(
            gaps,
            vec![cell("06-separable-filter", "naiv", "mp-tcp-bufsync")],
            "an M3 typo'd required cell run under --milestone M3 must \
             still be a hard coverage gap (TASK-0163 lockstep)"
        );

        // And the dual: under --milestone M1 the SAME M3 cell is
        // out-of-band ⇒ NOT this run's obligation ⇒ no false gap
        // (exactly mirrors plan_cells not flagging it required).
        let m1 = Args {
            milestone: Some(Milestone(1)),
            ..Args::default()
        };
        let gaps_m1 = required_coverage_gaps(&manifest, &[], &m1).expect("ok");
        assert!(
            gaps_m1.is_empty(),
            "an M3 cell is not an M1 run's coverage obligation, got {gaps_m1:?}"
        );
    }

    /// An out-of-band `[[skip]]` must not exempt anything: a skip
    /// tagged M3 does not silence an M1-tagged required of the same
    /// triple under `--milestone M1`. (Defends the skip-band scoping
    /// added in lockstep with plan_cells.)
    #[test]
    fn out_of_band_skip_does_not_exempt_in_band_required() {
        let manifest = Manifest {
            runnable_examples: vec!["03-reduction".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![req("03-reduction", "ghost", "pthreads-sync", "M1")],
            // Skip is tagged M3 — out of band for an M1 run.
            skip: vec![skip_e(
                "03-reduction",
                "ghost",
                "pthreads-sync",
                "blocked elsewhere",
                "M3",
            )],
        };
        let m1 = Args {
            milestone: Some(Milestone(1)),
            ..Args::default()
        };
        // `ghost` schedule is never discovered ⇒ never planned.
        let gaps = required_coverage_gaps(&manifest, &[], &m1).expect("ok");
        assert_eq!(
            gaps,
            vec![cell("03-reduction", "ghost", "pthreads-sync")],
            "an out-of-band skip must NOT exempt an in-band required"
        );
    }

    /// The shipped manifest must have ZERO coverage gaps at EVERY
    /// milestone tier (no flag, M1, M2, M3) — the durable per-tier
    /// guard. A future manifest edit that typo's or strands a
    /// milestone-tagged required cell turns the relevant tier (and its
    /// CI job) red.
    #[test]
    fn real_manifest_has_no_coverage_gaps_at_every_milestone() {
        let paths = Paths::discover().expect("discover repo root");
        let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
        let manifest: Manifest = toml::from_str(&src).expect("parse manifest");
        for gate in [None, Some(Milestone(1)), Some(Milestone(2)), Some(Milestone(3))] {
            let args = Args {
                milestone: gate,
                ..Args::default()
            };
            let plan = plan_cells(&paths, &manifest, &args).expect("plan");
            let gaps = required_coverage_gaps(&manifest, &plan, &args).expect("ok");
            assert!(
                gaps.is_empty(),
                "shipped e2e-matrix.toml has unmatched required cells at \
                 milestone {gate:?}: {gaps:?}"
            );
        }
    }

    /// The required set genuinely DIFFERS per milestone (the whole
    /// point of AC#3 — the CI jobs must not be identical). Pins the
    /// cumulative monotone: |M1| < |M2| < |M3| == |full|, and M1 ⊆ M2
    /// ⊆ M3 by construction of the gate.
    #[test]
    fn required_counts_strictly_grow_per_milestone() {
        let paths = Paths::discover().expect("discover repo root");
        let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
        let manifest: Manifest = toml::from_str(&src).expect("parse manifest");
        let count = |gate: Option<Milestone>| -> usize {
            let args = Args {
                milestone: gate,
                ..Args::default()
            };
            plan_cells(&paths, &manifest, &args)
                .expect("plan")
                .iter()
                .filter(|c| c.required)
                .count()
        };
        let m1 = count(Some(Milestone(1)));
        let m2 = count(Some(Milestone(2)));
        let m3 = count(Some(Milestone(3)));
        let full = count(None);
        assert!(
            m1 < m2 && m2 < m3,
            "milestone subsets must strictly grow: M1={m1} M2={m2} M3={m3}"
        );
        assert_eq!(
            m3, full,
            "M3 is the current top tier ⇒ its required set == the full set \
             (M1={m1} M2={m2} M3={m3} full={full})"
        );
    }

    #[test]
    fn skip_entry_short_circuits_run() {
        let paths = Paths::discover().expect("discover");
        let pc = PlannedCell {
            cell: Cell {
                example: "nonexistent-example".to_string(),
                schedule: "phantom".to_string(),
                backend: "imaginary".to_string(),
            },
            required: false,
            pre_skip: Some("test fixture".to_string()),
        };
        let r = run_cell(&paths, &pc);
        match r.status {
            Status::Skipped { reason } => assert_eq!(reason, "test fixture"),
            other => panic!("expected SKIPPED, got {other:?}"),
        }
        // No phase ran -> no timings recorded.
        assert!(r.timings.compile.is_none());
        assert!(r.timings.build.is_none());
        assert!(r.timings.run.is_none());
    }

    // ---- TASK-0187: layout-agnostic perturbation + zero-perturb guard.
    //
    // `NUC_NONDET_TEST` is process-global; Rust runs `#[test]`s on
    // parallel threads. Serialise the env-sensitive cases under one
    // mutex so set_var/remove_var cannot interleave. These are the
    // ONLY tests (and `maybe_perturb_for_nondet_test` the only code)
    // that touch this var, so the mutex is a complete fence.
    fn nondet_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        // A poisoned lock just means a prior env test panicked; we
        // still want a clean guard rather than cascading failures.
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn nondet_tmp(tag: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!(
            "nucleus-e2e-nondet-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).expect("mk tmp tree");
        t
    }

    const SAMPLE_CARGO_TOML: &str =
        "[package]\nname = \"nuc-generated\"\nversion = \"0.0.0\"\nedition = \"2021\"\n";

    #[test]
    fn perturb_mutates_cargo_toml_and_stays_valid_toml() {
        // AC#1 / AC#3: with the env gate set, the function must
        // actually mutate the tree's Cargo.toml (>=1 byte added) and
        // the result must still be parseable TOML — a `#` comment, not
        // a Rust `//` line. This is the file EVERY backend emits, so
        // mp-tcp cells (which emit no src/main.rs) are no longer
        // silently skipped.
        let _guard = nondet_env_lock();
        let tree = nondet_tmp("mutate");
        let cargo = tree.join("Cargo.toml");
        fs::write(&cargo, SAMPLE_CARGO_TOML).expect("write cargo");
        let before = fs::read_to_string(&cargo).expect("read before");

        std::env::set_var("NUC_NONDET_TEST", "1");
        let perturbed = maybe_perturb_for_nondet_test(&tree);
        std::env::remove_var("NUC_NONDET_TEST");

        assert_eq!(
            perturbed,
            Ok(true),
            "gate=1 with a present Cargo.toml must report a perturbation"
        );
        let after = fs::read_to_string(&cargo).expect("read after");
        assert_ne!(before, after, "Cargo.toml content must have changed");
        assert!(
            after.len() > before.len(),
            "perturbation must append bytes (got before={} after={})",
            before.len(),
            after.len()
        );
        assert!(
            after.contains("# NUC_NONDET_TEST nonce: pid="),
            "expected a `#` TOML comment nonce, got:\n{after}"
        );
        assert!(
            !after.contains("// NUC_NONDET_TEST"),
            "must NOT inject a Rust `//` line into TOML"
        );
        // The whole point of Cargo.toml-over-main.rs: the generated
        // project's manifest must still parse so downstream `cargo`
        // is unaffected.
        let parsed: Result<toml::Table, _> = after.parse();
        assert!(
            parsed.is_ok(),
            "perturbed Cargo.toml must remain valid TOML, parse error: {:?}",
            parsed.err()
        );

        let _ = fs::remove_dir_all(&tree);
    }

    #[test]
    fn perturb_is_strict_noop_when_env_unset() {
        // AC#3: the bare `determinism-check` path. With the gate
        // unset the function must not read or touch the tree and must
        // report `Ok(false)` (no perturbation) — this is what keeps
        // `just determinism-check` byte-identical.
        let _guard = nondet_env_lock();
        let tree = nondet_tmp("noop");
        let cargo = tree.join("Cargo.toml");
        fs::write(&cargo, SAMPLE_CARGO_TOML).expect("write cargo");

        // Ensure the var is genuinely unset for this case.
        std::env::remove_var("NUC_NONDET_TEST");
        let perturbed = maybe_perturb_for_nondet_test(&tree);

        assert_eq!(
            perturbed,
            Ok(false),
            "env unset must be a strict no-op (no perturbation)"
        );
        let after = fs::read_to_string(&cargo).expect("read after");
        assert_eq!(
            after, SAMPLE_CARGO_TOML,
            "env-unset path must leave Cargo.toml byte-identical"
        );

        let _ = fs::remove_dir_all(&tree);
    }

    #[test]
    fn perturb_errs_when_gate_set_but_cargo_toml_missing() {
        // AC#2 (per-cell arm): if the gate is set but the layout
        // drifted so Cargo.toml is absent, the function returns Err.
        // The harness maps that to a non-Pass and the matrix-wide
        // zero-perturb guard (asserted below) then makes the run a
        // hard fail rather than a recipe-inverted false OK.
        let _guard = nondet_env_lock();
        let tree = nondet_tmp("missing"); // empty: no Cargo.toml

        std::env::set_var("NUC_NONDET_TEST", "1");
        let perturbed = maybe_perturb_for_nondet_test(&tree);
        std::env::remove_var("NUC_NONDET_TEST");

        match perturbed {
            Err(msg) => assert!(
                msg.contains("Cargo.toml") && msg.contains("layout drifted"),
                "error should name the missing Cargo.toml and layout drift, got: {msg}"
            ),
            Ok(v) => panic!("expected Err when Cargo.toml absent under gate, got Ok({v})"),
        }

        let _ = fs::remove_dir_all(&tree);
    }

    #[test]
    fn zero_perturbation_guard_makes_negative_recipe_fail() {
        // AC#2 (matrix arm), proven directly. The TRUE invariant is
        // the *recipe verdict*, not the raw exit code, because the
        // `determinism-check-negative` recipe INVERTS the exit code:
        //
        //   harness exit 0       -> recipe prints "FAIL: did NOT
        //                            detect" and exits 1
        //   harness exit non-0   -> recipe prints "OK: correctly bit"
        //
        // So to make a zero-perturbation run a loud gate FAIL we must
        // exit the harness *cleanly* (0) under the gate; the recipe
        // then fires its FAIL branch. This models that exact seam plus
        // the recipe inversion against synthetic result sets. (The
        // behavioural >=5-run live bite is the gate's job — this locks
        // the *decision* so it can't silently regress.)

        // Mirrors the AC#2 seam in main(): returns the harness process
        // exit code.
        fn harness_exit_code(gate_on: bool, perturbed: &[bool], any_failed: bool) -> i32 {
            if gate_on && perturbed.iter().filter(|p| **p).count() == 0 {
                // Force a CLEAN exit so the recipe's FAIL branch fires.
                return 0;
            }
            if any_failed {
                1
            } else {
                0
            }
        }
        // Mirrors justfile:69: recipe verdict from harness exit code.
        // true == "OK: correctly bit", false == "FAIL: did NOT detect".
        fn recipe_says_ok(harness_exit: i32) -> bool {
            harness_exit != 0
        }

        // Partial-silent-neuter: gate on, NOTHING perturbed, but some
        // unrelated cell Failed (the exact false-confidence scenario).
        // The harness must exit CLEAN so the recipe says FAIL.
        let ex = harness_exit_code(true, &[false, false, false], true);
        assert_eq!(ex, 0, "zero perturbations under gate must exit CLEAN");
        assert!(
            !recipe_says_ok(ex),
            "zero perturbations under gate MUST make the recipe print \
             FAIL (not invert a no-op into OK)"
        );

        // Genuine bite: gate on, >=1 cell perturbed, diff Failed ->
        // harness exits non-zero -> recipe says OK. This is the only
        // path that may print OK.
        let ex = harness_exit_code(true, &[false, true, false], true);
        assert_eq!(ex, 1, "genuine perturbation+diff must exit non-zero");
        assert!(
            recipe_says_ok(ex),
            "a genuine >=1-perturbation bite must let the recipe print OK"
        );

        // Gate OFF (bare determinism-check): guard inert; exit driven
        // purely by Failed cells, byte-identical path undisturbed.
        assert_eq!(
            harness_exit_code(false, &[false, false, false], false),
            0,
            "gate off, no failures -> clean exit (byte-identical path)"
        );
        assert_eq!(
            harness_exit_code(false, &[false, false, false], true),
            1,
            "gate off must keep normal Failed-driven non-zero exit"
        );
    }
}
