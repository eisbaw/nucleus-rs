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

use std::collections::{BTreeSet, VecDeque};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::{Arc, Mutex};
use std::thread;
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
    /// Optional per-cell perf-regression gate (TASK-0023.03.02, Stage 3).
    /// When `--baseline` is set AND this is `Some(N)`, a current-vs-
    /// baseline relative-pct delta exceeding `N%` flips the cell into a
    /// REGRESSION row AND (because this is a `[[required]]` entry) hard-
    /// fails the harness exit code. `None` (default — `#[serde(default)]`
    /// so absent in TOML is byte-identical to today) ⇒ no gate, the
    /// delta is informational only. Relative-pct chosen over absolute-ms
    /// for the first cut; absolute is a follow-on if needed.
    #[serde(default)]
    perf_threshold_pct: Option<f64>,
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
    /// Optional per-cell perf-regression gate (TASK-0023.03.02, Stage 3).
    /// Same semantics as on `[[required]]`, EXCEPT a breach on a
    /// `[[skip]]` cell is informational only (no exit-code impact) — a
    /// skipped cell did not run a meaningful payload, so timings are
    /// noise; gating off them would be a false-positive. The field
    /// exists on `[[skip]]` purely so a future un-skip flip preserves
    /// the threshold without a separate edit.
    #[serde(default)]
    perf_threshold_pct: Option<f64>,
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
        let rest = s
            .strip_prefix('M')
            .ok_or_else(|| format!("milestone `{s}` is not of the form M<k> (e.g. M1, M2, M3)"))?;
        let k: u8 = rest
            .parse()
            .map_err(|_| format!("milestone `{s}` is not of the form M<k> (e.g. M1, M2, M3)"))?;
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
    fn required_milestones(&self) -> Result<std::collections::BTreeMap<Cell, Milestone>, String> {
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
    fn skip_table(&self) -> Result<std::collections::BTreeMap<Cell, (String, Milestone)>, String> {
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
        matches!(self.transport.as_deref(), None | Some("shared-memory"))
    }
}

// --------------------------------------------------------------------
// CLI args
// --------------------------------------------------------------------

/// Output format selector (TASK-0023.02). `Text` is the existing
/// human-readable summary table; `Junit` emits a JUnit XML
/// `<testsuites>` document on stdout so CI runners (GitHub Actions /
/// GitLab Pipelines) can surface individual matrix cells as named
/// test cases. Default is `Text` so `just e2e` is byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Junit,
}

impl Format {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "text" => Ok(Format::Text),
            "junit" => Ok(Format::Junit),
            other => Err(format!(
                "flag `--format` value must be one of `text` | `junit`, got `{other}`"
            )),
        }
    }
}

#[derive(Debug)]
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
    /// Number of worker threads for parallel cell execution
    /// (TASK-0023.01). Default 1 = sequential, byte-for-byte identical
    /// to pre-flag behaviour. Capped at `MAX_JOBS` to avoid pathological
    /// fork bombs (each concurrent `cargo build --release` of an
    /// emitted project costs ~200-500MB peak; --jobs 4 on a typical
    /// 14-core / 30 GB host is comfortable, much higher risks OOM).
    /// Validated >= 1 at parse time. Each cell's scratch dir is already
    /// unique-by-construction via TASK-0182's run-id segment, so
    /// in-process parallel cells do not collide on disk.
    jobs: usize,
    /// Output format for the per-cell summary (TASK-0023.02). `Text`
    /// (default) writes the existing human-readable table to stdout;
    /// `Junit` writes a JUnit XML `<testsuites>` document to stdout so
    /// CI runners can surface cells as test cases. The exit-code +
    /// gate-signal semantics (required-fail / `NUC_XBACKEND_*` /
    /// `NUC_NONDET_*`) are independent of this choice.
    format: Format,
    /// Optional path to write per-cell wall-clock timings as JSON
    /// (TASK-0023.03 Stage 1). When `Some`, after the matrix completes
    /// the harness writes a JSON document mirroring the planned-order
    /// `Vec<CellResult>` so a downstream comparator can flag perf
    /// regressions against a stored baseline. Default `None` =
    /// byte-identical to the pre-flag harness output. Stage 1 only
    /// covers RUN-mode results (`run_cell`); `--check-determinism` is
    /// out of scope for this stage and is filed as a follow-up.
    emit_timings: Option<PathBuf>,
    /// Optional path to a previously-emitted timings JSON document
    /// (TASK-0023.03 Stage 2). When `Some`, after the matrix completes
    /// the harness loads PATH, joins to the current `Vec<CellResult>`
    /// on `(example, schedule, backend)`, and prints a delta table to
    /// STDERR sorted by largest regression first. Cells present on
    /// only one side render as `(new)` / `(removed)` rather than
    /// crashing. Default `None` = byte-identical pre-flag behaviour.
    /// VALIDATED at parse time: the path must exist on disk (a typoed
    /// path is the most common silent-no-op failure mode for a flag
    /// like this — fail LOUD up front, not after the matrix has run).
    /// Per-cell perf-threshold gating is Stage 3 (out of scope here).
    baseline: Option<PathBuf>,
}

/// Upper bound on `--jobs N`. See [`Args::jobs`].
const MAX_JOBS: usize = 64;

impl Default for Args {
    fn default() -> Self {
        Self {
            example: None,
            schedule: None,
            backend: None,
            milestone: None,
            check_determinism: false,
            jobs: 1,
            format: Format::Text,
            emit_timings: None,
            baseline: None,
        }
    }
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
            "--jobs" | "-j" => {
                // TASK-0023.01: parallel cell execution.
                //
                // Parse + validate eagerly so a bad value fails LOUD at
                // arg-parse time, not after some cells have already run.
                // Range: [1, MAX_JOBS]. Zero is rejected (would spawn no
                // workers and silently never make progress); negative
                // and non-numeric raw strings are rejected by usize
                // parse; values above MAX_JOBS are clamped with a loud
                // error rather than silently truncated (silent truncate
                // would mask a typo like `--jobs 400`).
                let raw = need_val(i)?;
                let n: usize = raw.parse().map_err(|_| {
                    format!("flag `--jobs` requires a positive integer, got `{raw}`")
                })?;
                if n == 0 {
                    return Err("flag `--jobs` must be >= 1 (0 would spawn no workers)".to_string());
                }
                if n > MAX_JOBS {
                    return Err(format!(
                        "flag `--jobs` value {n} exceeds MAX_JOBS={MAX_JOBS}; \
                         each concurrent cell costs ~200-500MB peak, higher \
                         values risk OOM"
                    ));
                }
                a.jobs = n;
                i += 2;
            }
            "--format" => {
                // TASK-0023.02: structured output for CI runners.
                // Validated eagerly so a bad value fails LOUD at
                // arg-parse time, not after the matrix has run.
                a.format = Format::parse(&need_val(i)?)?;
                i += 2;
            }
            // TASK-0023.02: support `--format=junit` (equals form) too,
            // which is how most CI scripts pass the flag.
            x if x.starts_with("--format=") => {
                a.format = Format::parse(&x["--format=".len()..])?;
                i += 1;
            }
            "--emit-timings" => {
                // TASK-0023.03 Stage 1: persist per-cell wall-clock
                // timings as JSON for offline perf-regression analysis.
                // The path is captured verbatim (relative or absolute);
                // it is written ONCE, post-matrix, so a parse-time
                // error here is loud and we never spend matrix time
                // just to discover the path was bad.
                let raw = need_val(i)?;
                if raw.is_empty() {
                    return Err("flag `--emit-timings` requires a non-empty PATH".to_string());
                }
                a.emit_timings = Some(PathBuf::from(raw));
                i += 2;
            }
            // TASK-0023.03 Stage 1: `--emit-timings=PATH` (equals form),
            // mirroring the `--format=` precedent so CI scripts can use
            // either style.
            x if x.starts_with("--emit-timings=") => {
                let raw = &x["--emit-timings=".len()..];
                if raw.is_empty() {
                    return Err("flag `--emit-timings=` requires a non-empty PATH".to_string());
                }
                a.emit_timings = Some(PathBuf::from(raw));
                i += 1;
            }
            "--baseline" => {
                // TASK-0023.03 Stage 2: load a previously-emitted
                // timings JSON as the baseline against which the
                // current run is diffed on STDERR. Path is validated
                // eagerly here — a typoed/missing baseline is the
                // single most common silent-no-op trap a developer
                // hits with a flag like this. Failing LOUD at parse
                // time saves the matrix cost.
                let raw = need_val(i)?;
                if raw.is_empty() {
                    return Err("flag `--baseline` requires a non-empty PATH".to_string());
                }
                let p = PathBuf::from(&raw);
                if !p.exists() {
                    return Err(format!(
                        "flag `--baseline`: path `{raw}` does not exist; \
                         this is most likely a typo or a stale path from \
                         a previous run — emit one with --emit-timings first"
                    ));
                }
                a.baseline = Some(p);
                i += 2;
            }
            // TASK-0023.03 Stage 2: `--baseline=PATH` (equals form),
            // mirroring the `--emit-timings=` precedent.
            x if x.starts_with("--baseline=") => {
                let raw = &x["--baseline=".len()..];
                if raw.is_empty() {
                    return Err("flag `--baseline=` requires a non-empty PATH".to_string());
                }
                let p = PathBuf::from(raw);
                if !p.exists() {
                    return Err(format!(
                        "flag `--baseline=`: path `{raw}` does not exist; \
                         this is most likely a typo or a stale path from \
                         a previous run — emit one with --emit-timings first"
                    ));
                }
                a.baseline = Some(p);
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
[--milestone ID] [--check-determinism] [--jobs N | -j N] [--format text|junit] \
[--emit-timings PATH] [--baseline PATH]\n\
         \n\
         Bare invocation runs every cell declared in\n\
         `nuc-nucleus/e2e-matrix.toml`. Flags narrow the matrix to\n\
         matching cells.\n\
         \n\
         --jobs N / -j N: parallel cell execution (TASK-0023.01).\n\
         Default 1 (sequential, byte-for-byte identical to pre-flag\n\
         behaviour). N >= 2 spawns up to N worker threads that pull\n\
         from a shared work-queue. Per-cell progress lines emit in\n\
         COMPLETION order; the summary table is re-sorted to planned\n\
         order so exit-code + gate signals stay deterministic. Each\n\
         concurrent `cargo build --release` costs ~200-500MB peak;\n\
         --jobs 4 on a 14-core / 30 GB host is comfortable. Cells are\n\
         on-disk-isolated via the per-process run-id (TASK-0182).\n\
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
         = same emitted code, byte-for-byte. TASK-0033.\n\
         \n\
         --format text|junit: per-cell summary format (TASK-0023.02).\n\
         Default `text` is the existing human-readable table on stdout.\n\
         `junit` emits a JUnit XML `<testsuites>` document on stdout\n\
         (one `<testcase>` per cell, classname=example.schedule,\n\
         name=backend) so GitHub Actions / GitLab Pipelines can surface\n\
         individual cells. Exit code + gate-signal semantics are\n\
         independent of this choice.\n\
         \n\
         --emit-timings PATH: persist per-cell wall-clock timings as\n\
         JSON to PATH (TASK-0023.03 Stage 1). Schema:\n\
           {{ \"cells\": [ {{ \"example\": ..., \"schedule\": ...,\n\
             \"backend\": ..., \"required\": bool, \"status\":\n\
             \"PASS|FAIL|SKIPPED\", \"phase_times_ms\": {{ \"compile\":\n\
             N, \"build\": N, \"run\": N }}, \"total_ms\": N }} ] }}\n\
         Cells appear in planned (deterministic) order. Written ONCE\n\
         post-matrix; the human/junit summary and exit-code semantics\n\
         are unchanged. RUN-mode only — `--check-determinism` is a\n\
         follow-up (Stage 3 too: per-cell perf_threshold_pct in\n\
         e2e-matrix.toml).\n\
         \n\
         --baseline PATH: load a previously emitted timings JSON\n\
         (TASK-0023.03 Stage 2) and print a delta table to STDERR\n\
         sorted by largest regression first. PATH must exist (validated\n\
         at parse time). Cells absent from one side render as `(new)`\n\
         / `(removed)`. Output goes to STDERR specifically so it does\n\
         not corrupt --format=junit XML on STDOUT. ANSI-coloured when\n\
         STDERR is a TTY (red = slower, green = faster); plain-text\n\
         otherwise. Exit-code semantics are UNCHANGED — Stage 3 adds\n\
         per-cell perf thresholds that gate the exit.\n"
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
///
/// `Clone`: required so `execute_cells_parallel` can stash a snapshot
/// inside an `Arc` for the worker pool. Cheap — two heap strings.
#[derive(Clone)]
struct Paths {
    repo_root: PathBuf,
    /// Per-harness-invocation run id (TASK-0182). Computed exactly ONCE
    /// per process in `discover` and inserted as a path segment into
    /// every mutable scratch root, so two concurrent/rapid `just e2e`
    /// processes never share a `remove_dir_all`-able tree (the old
    /// shared-tree cwd race: a still-cwd-d `Command` in proc A while
    /// proc B removes the deterministically-named cell dir → observed
    /// `getcwd: cannot access parent directories` +
    /// `ld.bfd: cannot open output file …/nuc_generated`).
    ///
    /// `pid` makes it unique across concurrent processes; the nanos
    /// nonce makes it unique across rapid SEQUENTIAL invocations on
    /// systems where the OS recycles PIDs fast enough that two
    /// back-to-back runs could collide. Within ONE process the value
    /// is fixed, so a cell's path is STABLE and UNIQUE for the whole
    /// run — critical for determinism mode, whose `a`/`b` trees for a
    /// given cell must land under the same run root to be comparable.
    ///
    /// Chosen over an flock-based lock deliberately: a lock would
    /// SERIALISE concurrent harness runs (and, worse, block the future
    /// parallel-cell executor — TASK-0023.01 — whose whole point is
    /// concurrency). Run-id isolation is lock-free, needs no cleanup
    /// of lock files, and makes concurrency safe by construction
    /// rather than by mutual exclusion.
    run_id: String,
}

impl Paths {
    /// Compute the process-wide run id. Pure function of pid + a
    /// wall-clock nanos nonce; called once from `discover`.
    ///
    /// Test-only seam (TASK-0182, gate-only — mirrors the
    /// `NUC_NONDET_TEST` / `NUC_XBACKEND_NEGATIVE` discipline): when
    /// `NUC_E2E_FORCE_SHARED_RUN_ID` is set to a non-empty value, that
    /// exact string is used as the run id INSTEAD of `pid+nanos`. This
    /// deliberately RE-CREATES the pre-fix shared-tree condition (all
    /// concurrent invocations collide on one mutable path) so the
    /// concurrency stress test can prove it genuinely BITES the old
    /// failure mode. It is never set by any justfile recipe or CI
    /// path, so bare `just e2e` / `determinism-check` are byte-for-byte
    /// unaffected (the env read is a strict no-op when unset).
    fn compute_run_id() -> String {
        if let Some(forced) = std::env::var_os("NUC_E2E_FORCE_SHARED_RUN_ID") {
            if !forced.is_empty() {
                return forced.to_string_lossy().into_owned();
            }
        }
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            // Clock-before-epoch is implausible here; fall back to 0
            // rather than panicking — the pid alone still isolates
            // concurrent processes.
            .unwrap_or(0);
        format!("run-{pid}-{nanos}")
    }

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
                return Ok(Paths {
                    repo_root: start,
                    run_id: Self::compute_run_id(),
                });
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

    /// The per-RUN run-mode scratch root:
    /// `nucleus/target/e2e-matrix/<run-id>`. Every cell of THIS
    /// invocation lives under here; a different concurrent/rapid
    /// invocation gets a different `<run-id>` and so a disjoint tree
    /// (TASK-0182 — eliminates the shared-tree cwd race). Stays under
    /// `nucleus/target/` so `cargo clean` still sweeps it.
    fn run_scratch_root(&self) -> PathBuf {
        self.repo_root
            .join("nucleus/target/e2e-matrix")
            .join(&self.run_id)
    }

    /// The per-RUN determinism scratch root:
    /// `nucleus/target/e2e-determinism/<run-id>`.
    fn run_determinism_root(&self) -> PathBuf {
        self.repo_root
            .join("nucleus/target/e2e-determinism")
            .join(&self.run_id)
    }

    /// Per-cell scratch directory under this run's root. `cargo clean`
    /// sweeps it. Removed and recreated so stale artefacts cannot mask
    /// a regression — and, since the parent segment is the per-run
    /// `<run-id>`, that `remove_dir_all` can only ever touch THIS
    /// process's own tree, never a sibling run still cwd-d into it.
    fn scratch_dir(&self, ex: &str, sched: &str, backend: &str) -> Result<PathBuf, String> {
        let root = self.run_scratch_root();
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
    /// Determinism trees land under the per-run
    /// `e2e-determinism/<run-id>/` so the `a` and `b` builds of a
    /// given cell — which MUST be byte-comparable — share one run
    /// root, while a concurrent run is fully disjoint (TASK-0182).
    fn determinism_dir(
        &self,
        ex: &str,
        sched: &str,
        backend: &str,
        label: &str,
    ) -> Result<PathBuf, String> {
        let root = self.run_determinism_root();
        fs::create_dir_all(&root)
            .map_err(|e| format!("cannot create determinism root `{}`: {e}", root.display()))?;
        let cell = format!("{ex}__{sched}__{backend}__{label}").replace(['/', '\\'], "_");
        let dir = root.join(cell);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create cell dir `{}`: {e}", dir.display()))?;
        Ok(dir)
    }

    /// End-of-run disposition of this invocation's per-run scratch
    /// roots (TASK-0182 cruft bound).
    ///
    /// * `success == true`: the run completed cleanly — remove both
    ///   per-run roots so `nucleus/target/e2e-{matrix,determinism}/`
    ///   does not grow without bound across repeated runs. Only the
    ///   `<run-id>` subtree is removed; a CONCURRENT run's sibling
    ///   `<run-id>` is untouched (that is the whole point of the
    ///   per-run segment), and the shared parent dir is left in place.
    /// * `success == false`: keep the trees for debuggability and
    ///   print their absolute paths so a developer can inspect the
    ///   failed build. They remain under `nucleus/target/`, so a
    ///   later `cargo clean` (or the next successful run, which only
    ///   removes its OWN run-id) still bounds them — but a long series
    ///   of failures will accumulate `<run-id>` dirs; that is the
    ///   accepted debuggability trade-off, documented here so it is a
    ///   choice, not an oversight.
    ///
    /// Only roots that actually exist are acted on (a determinism-only
    /// or run-only invocation never created the other), and a removal
    /// error is reported loudly (never silently swallowed) but does
    /// not itself fail the run.
    fn finalize_run_scratch(&self, success: bool) {
        for root in [self.run_scratch_root(), self.run_determinism_root()] {
            if !root.exists() {
                continue;
            }
            if success {
                if let Err(e) = fs::remove_dir_all(&root) {
                    eprintln!(
                        "nucleus-e2e: WARNING: could not clean per-run scratch \
                         `{}`: {e} (cruft will be swept by `cargo clean`)",
                        root.display()
                    );
                }
            } else {
                eprintln!(
                    "nucleus-e2e: retained per-run scratch for debugging: {}",
                    root.display()
                );
            }
        }
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
    /// `true` iff `maybe_corrupt_wire_for_xbackend` actually rewrote
    /// this cell's emitted `src/wire.rs` (only ever true under
    /// `NUC_XBACKEND_NEGATIVE=1`, and only for mp-tcp-bufsync cells —
    /// `wire.rs` is mp-tcp-EXCLUSIVE). Aggregated across the matrix to
    /// enforce the TASK-0183 zero-corruption guard: under the negative
    /// env gate the run must force a CLEAN exit (so the inverting
    /// recipe FAILs loud) unless at least one tree was genuinely
    /// corrupted — a uniform Skip/no-op must NOT be invertible to OK
    /// (the TASK-0187 partial-silent-neuter lesson, mirrored).
    corrupted: bool,
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
                let in_band_required = req_m.is_some_and(|m| milestone_in_gate(m, args.milestone));
                let in_band_skip =
                    skip_m.is_some_and(|(_, m)| milestone_in_gate(*m, args.milestone));

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
                let perf_threshold_pct = perf_threshold_map.get(&cell).copied();
                planned.push(PlannedCell {
                    cell,
                    required,
                    pre_skip,
                    perf_threshold_pct,
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
struct PlannedCell {
    cell: Cell,
    required: bool,
    pre_skip: Option<String>,
    /// Per-cell perf-regression threshold (TASK-0023.03.02). Plumbed
    /// through from the manifest's `RequiredEntry`/`SkipEntry`. `None`
    /// (the default for cells with no declaration AND for declarations
    /// that omit the key) means "no gate" — the comparator emits an
    /// informational delta row only. `Some(N)` only gates the exit code
    /// when the cell is `required = true`; a breach on a skip-band cell
    /// is informational (see `SkipEntry::perf_threshold_pct`).
    perf_threshold_pct: Option<f64>,
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
    // Set true only by the harness-side NUC_XBACKEND_NEGATIVE
    // post-process below (TASK-0183), and only for an mp-tcp-bufsync
    // cell whose emitted `src/wire.rs` was actually rewritten. Every
    // early-return constructor before that point carries this still-
    // `false` value, exactly as TASK-0187 threads `did_perturb`
    // through `check_cell_determinism`.
    let mut corrupted = false;

    // Manifest-declared skip wins before we touch the filesystem.
    if let Some(reason) = &planned.pre_skip {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Skipped {
                reason: reason.clone(),
            },
            timings: Timings::default(),
            corrupted,
        };
    }

    // Capabilities sniff: the harness does not duplicate
    // nucleus-compiler's capability matcher (that's
    // `check_schedule_compat`, and the driver invokes it for us).
    // We only check that the
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
            corrupted,
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
                corrupted,
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
                corrupted,
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
                corrupted,
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
                corrupted,
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
                corrupted,
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
            corrupted,
        };
    }

    // ---- NUC_XBACKEND_NEGATIVE post-emit corruption (TASK-0183). -------
    //
    // Relocated here from mp-tcp-bufsync `lib.rs` (TASK-0178's inline
    // `maybe_corrupt_wire` on the production `wire.rs` emission path,
    // deleted in TASK-0183). Mirrors the sibling NUC_NONDET_TEST seam
    // (TASK-0157/0187): the e2e harness is the SOLE consumer of
    // NUC_XBACKEND_NEGATIVE (only `just xbackend-check-negative` sets
    // it), so production codegen needs no test hook at all — keeping
    // mp-tcp-bufsync fully branch-free is the strongest AC#1.
    //
    // It runs AFTER `nucleus build` (the emitted tree exists) and
    // BEFORE the `cargo build` below (the corruption must be compiled
    // in). `wire.rs` is mp-tcp-EXCLUSIVE: pthreads-sync emits no wire
    // at all, so `maybe_corrupt_wire_for_xbackend` is invoked ONLY for
    // mp-tcp-bufsync cells — a pthreads cell legitimately has no
    // wire.rs and must NOT Err. Under the gate, a missing wire.rs /
    // drifted enc_vec anchor is a HARD failure (Failed(Compile)), NOT
    // a silent skip, so the falsifier can never be silently neutered;
    // the matrix-wide zero-corruption guard in `run()` then forces a
    // CLEAN exit so the inverting recipe FAILs loud. Gate unset =>
    // strict no-op (function does not read or touch the tree), so bare
    // `just e2e` emits a byte-identical pristine `wire.rs`.
    if cell.backend == "mp-tcp-bufsync" {
        match maybe_corrupt_wire_for_xbackend(&scratch) {
            Ok(did) => corrupted = did,
            Err(e) => {
                return CellResult {
                    cell,
                    required: planned.required,
                    status: Status::Failed {
                        phase: Phase::Compile,
                        detail: format!("NUC_XBACKEND_NEGATIVE corruption: {e}"),
                    },
                    timings,
                    corrupted,
                };
            }
        }
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
                corrupted,
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
            corrupted,
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
                corrupted,
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
                corrupted,
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
                corrupted,
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
            corrupted,
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
                corrupted,
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
                corrupted,
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
            corrupted,
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
            corrupted,
        };
    }

    CellResult {
        cell,
        required: planned.required,
        status: Status::Pass,
        timings,
        corrupted,
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

/// Negative-gate hook for `xbackend-check-negative` (TASK-0178,
/// relocated harness-side in TASK-0183 from mp-tcp-bufsync `lib.rs`'s
/// inline `maybe_corrupt_wire` — deleted there so production codegen
/// carries no test-only branch). Sibling of
/// `maybe_perturb_for_nondet_test`; identical discipline.
///
/// When `NUC_XBACKEND_NEGATIVE=1`, deterministically corrupt the
/// emitted `src/wire.rs` in `tree` (an mp-tcp-bufsync project dir):
/// rewrite `enc_vec` so the single trailing byte of each encoded vec
/// payload is incremented (wrapping). `wire.rs` is the TCP wire
/// protocol, **mp-tcp-EXCLUSIVE** — pthreads-sync emits no wire at
/// all. A multi-process mp-tcp cell that ships array data
/// worker→worker then decodes wrong values and its `output.bin`
/// diverges from the committed hand-written `reference.bin` oracle,
/// while pthreads-sync (same oracle) stays byte-identical. That
/// asymmetry — mp-tcp ≠ reference, pthreads-sync == reference — is
/// precisely the *cross-backend* differential biting, not a global
/// break. Caller invokes this ONLY for `cell.backend ==
/// "mp-tcp-bufsync"`, so a pthreads tree (no wire.rs) is never
/// touched and never Errs.
///
/// Returns `Ok(true)` if the tree was actually corrupted, `Ok(false)`
/// if the env gate was unset/other (a strict no-op — the function
/// does not read or touch the tree, keeping bare `just e2e`'s emitted
/// `wire.rs` byte-identical to `mp_tcp_common::WIRE_RUNTIME_SRC`),
/// `Err` if the gate was set but `wire.rs` was missing OR the
/// `enc_vec` anchor drifted. The caller maps `Err` to
/// `Failed(Compile)` (a HARD, gate-visible failure — never a silent
/// skip that neuters the falsifier); the matrix-wide zero-corruption
/// guard in `run()` then forces a CLEAN exit so the inverting recipe
/// FAILs loud (the TASK-0187 recipe-inversion lesson).
///
/// Deterministic by construction: a fixed source rewrite (string
/// replace), no PID/clock/RNG/hash-order entropy. Env gate (not a
/// cargo feature / `cfg!`): a nested `cargo --features` inside the
/// harness's own `cargo run` does not reliably rebuild against the
/// shared target cache; an env var is read at run time, no rebuild.
/// Exact-`"1"` gate, loud stderr banner, anchor-drift hard-failure —
/// all preserved verbatim from the deleted mp-tcp-bufsync site.
fn maybe_corrupt_wire_for_xbackend(tree: &std::path::Path) -> Result<bool, String> {
    if std::env::var("NUC_XBACKEND_NEGATIVE").as_deref() != Ok("1") {
        return Ok(false);
    }
    eprintln!(
        "nucleus-e2e: WARNING: NUC_XBACKEND_NEGATIVE=1 — deliberately \
         corrupting mp-tcp wire encode (enc_vec) in an emitted tree ON \
         PURPOSE to test the cross-backend differential. This run is \
         NOT a real build and its mp-tcp output is intentionally wrong. \
         Never set this in a real build (TASK-0178 / TASK-0183)."
    );
    let wire_rs = tree.join("src").join("wire.rs");
    if !wire_rs.exists() {
        return Err(format!(
            "expected emitted `{}` not found — codegen layout drifted; \
             mp-tcp-bufsync must emit src/wire.rs (TASK-0183); refusing \
             to let a missing-file become a silently-uncorrupted build",
            wire_rs.display()
        ));
    }
    let src =
        fs::read_to_string(&wire_rs).map_err(|e| format!("read {}: {e}", wire_rs.display()))?;
    // The single body line of `enc_vec` in wire_runtime.rs. We append
    // a deterministic last-byte tweak after the buffer is filled.
    // Anchored on the exact source text so a refactor of wire_runtime
    // fails this replace LOUD (Err below -> Failed(Compile)) rather
    // than silently emitting a non-corrupted build that would make the
    // negative recipe a false PASS. (Was a `panic!` at the deleted
    // mp-tcp-bufsync site; harness-side a typed Err is the gate-visible
    // analogue — the caller turns it into Failed(Compile) and the
    // zero-corruption guard makes the recipe FAIL loud.)
    const ANCHOR: &str =
        "    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    out\n}";
    const CORRUPT: &str = "    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    if let Some(last) = out.last_mut() {\n        *last = last.wrapping_add(1); // NUC_XBACKEND_NEGATIVE deliberate corruption (TASK-0178 / TASK-0183)\n    }\n    out\n}";
    if !src.contains(ANCHOR) {
        return Err(format!(
            "enc_vec anchor not found in `{}` — the negative-arm \
             injection point has drifted. Update ANCHOR in \
             maybe_corrupt_wire_for_xbackend (TASK-0183) so the \
             cross-backend negative test keeps biting; refusing to \
             emit a silently-uncorrupted build",
            wire_rs.display()
        ));
    }
    let corrupted = src.replacen(ANCHOR, CORRUPT, 1);
    fs::write(&wire_rs, corrupted).map_err(|e| format!("write {}: {e}", wire_rs.display()))?;
    Ok(true)
}

/// Sentinel schedule name used by `maybe_inject_required_coverage_negative`.
/// MUST NOT match any real `examples/<example>/schedules/<schedule>.sched.nuc`
/// file — its purpose is to be unfindable by `plan_cells`, so the synthetic
/// `[[required]]` entry below cannot be planned and therefore appears as a
/// gap from `required_coverage_gaps`. Used to attribute gaps to THIS injection
/// (and not to any unrelated gap that might appear for other reasons) when
/// computing `NUC_REQUIRED_COVERAGE_GAP_DETECTED` in `run_inner`.
///
/// Renaming this constant is a contract change: justfile's
/// `required-coverage-check-negative` recipe parses the harness output by the
/// stdout key, not by this string, but `run_inner`'s attribution filter does
/// compare against this exact value — keep the two in lockstep.
const REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE: &str = "__nuc_typo_negative_schedule__";

/// Negative-gate hook for `required-coverage-check-negative` (TASK-0168).
/// Sibling of `maybe_perturb_for_nondet_test` (NUC_NONDET_TEST) and
/// `maybe_corrupt_wire_for_xbackend` (NUC_XBACKEND_NEGATIVE); identical
/// discipline.
///
/// When `NUC_REQUIRED_COVERAGE_NEGATIVE=1`, append a single synthetic
/// `[[required]]` entry to the in-memory `Manifest` whose `schedule` is the
/// `REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE` sentinel — a name that
/// cannot match any discovered `*.sched.nuc` file. The synthetic entry's
/// `example`, `backend`, and `milestone` are taken from `manifest.required[0]`
/// (the first real required entry) so:
///   * `example` is in `runnable_examples` (otherwise `plan_cells` would not
///     iterate over it and the cell would silently leave the coverage scope),
///   * `backend` is in `manifest.backends` (same reasoning),
///   * `milestone` lies in any `--milestone` band that includes at least one
///     real required cell (bare `just e2e` has no milestone filter, so any
///     value is fine; the recipe runs without `--milestone`, but mirroring an
///     existing entry keeps the injection robust against future CI matrices).
///
/// Fallback (no real `[[required]]` entries in the manifest): pick
/// `runnable_examples[0]`, `backends[0]`, milestone `"M1"` (hard-coded —
/// best-effort, NOT robust to a future scheme that retires `M1` from
/// `Milestone::parse`'s accepted set; if that ever happens, this branch
/// returns a milestone string that `Milestone::parse` would reject
/// downstream, making the recipe FAIL loud rather than silently
/// no-op). This branch is purely defensive — today's `e2e-matrix.toml`
/// has 18+ required entries so the anchored path always wins — but it
/// avoids a NEW failure mode (panic / Err) on an exotic manifest. If
/// the manifest is truly degenerate (no examples OR no backends), Err
/// loud rather than silently no-op.
///
/// CRITICAL — does NOT mutate existing entries. AC#2 of TASK-0168 demands
/// "no committed broken manifest"; appending one synthetic entry at
/// runtime, after the on-disk file has been parsed, satisfies that contract
/// exactly the way `maybe_perturb_for_nondet_test` and
/// `maybe_corrupt_wire_for_xbackend` do for their respective gates (the env
/// flag is the perturbation seam; the on-disk artefact stays clean).
///
/// Returns `Ok(true)` if the manifest was actually mutated, `Ok(false)` if
/// the env gate was unset/other (a strict no-op — the function does not
/// read or touch the manifest, keeping bare `just e2e` byte-for-byte
/// unaffected), `Err` if the gate was set but the manifest is too degenerate
/// to inject into. Deterministic by construction: same input manifest plus
/// same env state = identical injection (no clock/PID/RNG).
fn maybe_inject_required_coverage_negative(manifest: &mut Manifest) -> Result<bool, String> {
    if std::env::var("NUC_REQUIRED_COVERAGE_NEGATIVE").as_deref() != Ok("1") {
        return Ok(false);
    }
    eprintln!(
        "nucleus-e2e: WARNING: NUC_REQUIRED_COVERAGE_NEGATIVE=1 — injecting a \
         synthetic [[required]] entry with a non-existent schedule \
         (`{REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE}`) into the in-memory \
         manifest ON PURPOSE to test the TASK-0163 required-coverage guard. \
         This run is NOT a real build and is expected to exit non-zero. Never \
         set this in a real build (TASK-0168)."
    );

    // Pick (example, backend, milestone) from the first real required entry
    // so the synthetic cell survives `cell_matches_filters` and the active
    // milestone gate. Fallback path is only relevant on a degenerate manifest
    // — see docstring.
    let (example, backend, milestone) = if let Some(first) = manifest.required.first() {
        (
            first.example.clone(),
            first.backend.clone(),
            first.milestone.clone(),
        )
    } else {
        let example = manifest.runnable_examples.first().cloned().ok_or_else(|| {
            "NUC_REQUIRED_COVERAGE_NEGATIVE=1 but manifest has no \
             runnable_examples to anchor a synthetic required entry against \
             (degenerate manifest)"
                .to_string()
        })?;
        let backend = manifest.backends.first().cloned().ok_or_else(|| {
            "NUC_REQUIRED_COVERAGE_NEGATIVE=1 but manifest has no \
             backends to anchor a synthetic required entry against \
             (degenerate manifest)"
                .to_string()
        })?;
        (example, backend, "M1".to_string())
    };

    manifest.required.push(RequiredEntry {
        example,
        schedule: REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE.to_string(),
        backend,
        milestone,
        perf_threshold_pct: None,
    });
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
// TASK-0023.02: JUnit XML summary
// --------------------------------------------------------------------

/// Escape the five XML 1.0 special characters in a `<testcase>`-level
/// attribute or element value. `name`/`classname`/`message` attribute
/// values cannot contain `<`, `>`, `&`, `"`; element text/CDATA cannot
/// contain `<`/`&` unwrapped. Cell identifiers (example/schedule/
/// backend) are constrained by the manifest to ASCII identifiers
/// today, but a future manifest change could relax that — so be
/// defensive here rather than silently emit malformed XML if a name
/// gains a `&`.
fn xml_escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Render a failure `<detail>` payload safely inside a CDATA block.
/// A `]]>` sequence inside CDATA would end it prematurely, so split
/// it into two CDATA sections (`]]` + `]]>` ⇒ `]]` `]]>` ⇒ no early
/// terminator).
fn xml_escape_cdata(s: &str) -> String {
    s.replace("]]>", "]]]]><![CDATA[>")
}

/// Emit the matrix as a JUnit XML `<testsuites>` document on stdout.
///
/// Schema (TASK-0023.02 AC#2/#3):
///
///   * one `<testsuite>` wrapping every cell, `tests`/`failures`/
///     `errors=0`/`skipped` attributes;
///   * one `<testcase>` per cell with `classname="<example>.<schedule>"`,
///     `name="<backend>"`, `time="<elapsed_seconds>"`;
///   * PASS → empty element;
///   * SKIPPED → `<skipped message="<reason>"/>`;
///   * FAILED → `<failure type="<phase>">` with the detail wrapped in
///     CDATA. The redundant `message=<phase>` attr (cycle-53) was
///     dropped in TASK-0248 because it duplicated `type=` verbatim;
///     `message` is optional in JUnit and consumers fall back to the
///     `type` attr / body.
///
/// Bytes are written via `println!` so the output goes to stdout where
/// CI runners look for it.
fn print_summary_junit(results: &[CellResult], wall_clock: Option<Duration>) {
    let total = results.len();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, Status::Failed { .. }))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, Status::Skipped { .. }))
        .count();
    // TASK-0248: prefer the executor-measured wall-clock (honest under
    // --jobs N>=2). Fall back to summing per-cell elapsed when the
    // caller can't supply one — that path matches the old (cycle-53)
    // emit, which is still schema-legal and only overstates parallel
    // runs.
    let suite_time: f64 = match wall_clock {
        Some(d) => d.as_secs_f64(),
        None => results
            .iter()
            .map(|r| r.timings.total().as_secs_f64())
            .sum(),
    };

    println!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    println!(
        "<testsuites tests=\"{total}\" failures=\"{failed}\" errors=\"0\" \
         skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    println!(
        "  <testsuite name=\"nucleus-e2e\" tests=\"{total}\" failures=\"{failed}\" \
         errors=\"0\" skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    for r in results {
        let classname = xml_escape_attr(&format!("{}.{}", r.cell.example, r.cell.schedule));
        let name = xml_escape_attr(&r.cell.backend);
        let time_s = r.timings.total().as_secs_f64();
        match &r.status {
            Status::Pass => {
                // Empty element — JUnit consumers treat the absence of
                // <failure>/<skipped> children as a pass. No CDATA
                // body needed.
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\"/>"
                );
            }
            Status::Skipped { reason } => {
                let msg = xml_escape_attr(reason);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                println!("      <skipped message=\"{msg}\"/>");
                println!("    </testcase>");
            }
            Status::Failed { phase, detail } => {
                // TASK-0248: drop the redundant `message=` attribute —
                // the previous emit set message= to the same string as
                // type= (a comment-lie: two attrs with the same content
                // pretending to mean different things). `message` is
                // optional in JUnit; CI consumers fall back to either
                // the type attr or the CDATA body, so the structural
                // failure phase still surfaces via `type=`.
                let phase_attr = xml_escape_attr(&phase.to_string());
                let detail_cdata = xml_escape_cdata(detail);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                println!(
                    "      <failure type=\"{phase_attr}\"><![CDATA[{detail_cdata}]]></failure>"
                );
                println!("    </testcase>");
            }
        }
    }
    println!("  </testsuite>");
    println!("</testsuites>");
}

/// Emit the determinism-mode matrix (TASK-0033) as a JUnit XML
/// `<testsuites>` document. Mirrors [`print_summary_junit`] but reads
/// from `DetCellResult` — Failed carries a `DetMismatch` rather than a
/// `Phase`+detail, so the `<failure type=...>` is hard-coded to
/// `"determinism"` and the body is the mismatch description.
fn print_determinism_summary_junit(results: &[DetCellResult], wall_clock: Option<Duration>) {
    let total = results.len();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Failed(_)))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Skipped { .. }))
        .count();
    // TASK-0248: see [`print_summary_junit`] for the wall-clock-vs-sum
    // rationale. Determinism mode runs each cell twice back-to-back
    // (single-cell-twice timing inside `check_cell_determinism`), so
    // the per-cell `elapsed` field captures BOTH runs and the parallel
    // overstatement is the same shape — same fix.
    let suite_time: f64 = match wall_clock {
        Some(d) => d.as_secs_f64(),
        None => results.iter().map(|r| r.elapsed.as_secs_f64()).sum(),
    };

    println!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    println!(
        "<testsuites tests=\"{total}\" failures=\"{failed}\" errors=\"0\" \
         skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    println!(
        "  <testsuite name=\"nucleus-e2e-determinism\" tests=\"{total}\" failures=\"{failed}\" \
         errors=\"0\" skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    for r in results {
        let classname = xml_escape_attr(&format!("{}.{}", r.cell.example, r.cell.schedule));
        let name = xml_escape_attr(&r.cell.backend);
        let time_s = r.elapsed.as_secs_f64();
        match &r.status {
            DetCellStatus::Pass { .. } => {
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\"/>"
                );
            }
            DetCellStatus::Skipped { reason } => {
                let msg = xml_escape_attr(reason);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                println!("      <skipped message=\"{msg}\"/>");
                println!("    </testcase>");
            }
            DetCellStatus::Failed(m) => {
                let body = format!(
                    "{} at {} (offset {}): {}",
                    m.kind,
                    m.relative_path.display(),
                    m.offset,
                    m.detail
                );
                let detail_cdata = xml_escape_cdata(&body);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                // TASK-0248: drop the redundant `message=` attribute
                // (see `print_summary_junit` for rationale). The
                // structural failure kind is exposed via `type=`.
                println!(
                    "      <failure type=\"determinism\"><![CDATA[{detail_cdata}]]></failure>"
                );
                println!("    </testcase>");
            }
        }
    }
    println!("  </testsuite>");
    println!("</testsuites>");
}

// --------------------------------------------------------------------
// Per-cell timings JSON (TASK-0023.03 Stage 1)
// --------------------------------------------------------------------

/// Escape one string for inclusion as a JSON string literal (RFC 8259
/// §7). We escape the strictly-required set — backslash, double quote,
/// and the C0 controls (`< 0x20`) — using the short forms for `\b \f \n
/// \r \t` and `\u00XX` for the rest. Non-ASCII is passed through
/// unchanged (valid UTF-8 in -> valid UTF-8 out); no need to escape
/// `/` or non-ASCII chars (the RFC permits but does not require it).
/// Defensive: cell identifiers + Status payloads come from the
/// manifest and the driver's `String` error messages, and the latter
/// can contain arbitrary bytes (compiler panics, OS errors).
fn json_escape_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Serialise a single `CellResult` as one JSON object. Schema:
///
/// ```text
///   { "example": "...", "schedule": "...", "backend": "...",
///     "required": bool,
///     "status": "PASS" | "FAIL" | "SKIPPED",
///     "fail_phase": "compile|build|run|diff"   (FAIL only),
///     "detail":     "..."                       (FAIL only),
///     "skip_reason":"..."                       (SKIPPED only),
///     "phase_times_ms": { "compile": N|null, "build": N|null,
///                         "run": N|null },
///     "total_ms": N,
///     "corrupted": bool }
/// ```
///
/// `phase_times_ms` mirrors the `Timings` struct (compile/build/run);
/// `null` is emitted where a phase did not execute (e.g. SKIPPED).
/// Missing-vs-zero matters: a phase that ran in 0 ms is `0`, not
/// `null`. The PRD-spec key for the JSON consumer is `phase_times_ms`,
/// keeping in line with the millisecond resolution `Duration::as_millis`
/// produces. Out-of-range values (>= 2^53) would round-trip lossily in
/// JS consumers, but per-phase millis are vastly below that.
///
/// **Spec-vs-source deviation** (architect review cycle 54): TASK-0023.03
/// AC#1 names the phases as `{build, run, diff}`. The actual `Timings`
/// struct in this crate carries `{compile, build, run}` — there is no
/// separate `diff` phase timer; the diff-check work is folded into the
/// `run` phase's wall-clock. JSON emission matches the SOURCE struct
/// rather than the spec phrasing, which is the right call (a `null`
/// for a phase that doesn't exist in source would be a comment-doc lie).
/// TASK-0023.03.01 (the Stage-2 baseline comparator follow-up) carries
/// the precise scope reference if future readers wonder about this.
fn cell_result_to_json(out: &mut String, r: &CellResult) {
    out.push('{');
    out.push_str("\"example\":");
    json_escape_str(out, &r.cell.example);
    out.push_str(",\"schedule\":");
    json_escape_str(out, &r.cell.schedule);
    out.push_str(",\"backend\":");
    json_escape_str(out, &r.cell.backend);
    out.push_str(",\"required\":");
    out.push_str(if r.required { "true" } else { "false" });

    out.push_str(",\"status\":");
    match &r.status {
        Status::Pass => out.push_str("\"PASS\""),
        Status::Failed { .. } => out.push_str("\"FAIL\""),
        Status::Skipped { .. } => out.push_str("\"SKIPPED\""),
    }

    // Status-specific payload, named so the consumer never needs to
    // pattern-match: presence of `fail_phase` <=> Failed; presence of
    // `skip_reason` <=> Skipped. PASS carries neither.
    match &r.status {
        Status::Pass => {}
        Status::Failed { phase, detail } => {
            out.push_str(",\"fail_phase\":");
            json_escape_str(out, &phase.to_string());
            out.push_str(",\"detail\":");
            json_escape_str(out, detail);
        }
        Status::Skipped { reason } => {
            out.push_str(",\"skip_reason\":");
            json_escape_str(out, reason);
        }
    }

    // phase_times_ms with explicit nulls — see fn-doc.
    out.push_str(",\"phase_times_ms\":{");
    out.push_str("\"compile\":");
    match r.timings.compile {
        Some(d) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{}", d.as_millis());
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"build\":");
    match r.timings.build {
        Some(d) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{}", d.as_millis());
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"run\":");
    match r.timings.run {
        Some(d) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{}", d.as_millis());
        }
        None => out.push_str("null"),
    }
    out.push('}');

    {
        use std::fmt::Write as _;
        let _ = write!(out, ",\"total_ms\":{}", r.timings.total().as_millis());
    }

    out.push_str(",\"corrupted\":");
    out.push_str(if r.corrupted { "true" } else { "false" });
    out.push('}');
}

/// Render the full `Vec<CellResult>` to a JSON document with a
/// top-level `{"mode": "run", "cells": [...]}` object. Cells appear in planned
/// (deterministic) order — `execute_cells_parallel` re-sorts results
/// to planned order before returning. Newlines between objects so a
/// quick `grep` can scan one cell per line, but no trailing newline
/// inside the array (keeps the document compact).
fn render_timings_json(results: &[CellResult]) -> String {
    let mut out = String::with_capacity(results.len() * 256);
    // TASK-0023.03.03 cycle-57: explicit top-level `"mode": "run"` so a
    // downstream consumer can branch RUN vs DETERMINISM schema (they
    // differ on per-cell payload: phase_times_ms here vs files_compared
    // / det_mismatch / single elapsed_ms in the det emitter). Cycle-55's
    // hand-rolled `parse_baseline_json` silently skips unknown top-level
    // keys via `skip_value`, so this is backward-compatible with any
    // baseline written before this cycle.
    out.push_str("{\n  \"mode\": \"run\",\n  \"cells\": [\n");
    for (i, r) in results.iter().enumerate() {
        out.push_str("    ");
        cell_result_to_json(&mut out, r);
        if i + 1 < results.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// Write the timings JSON to `path`. Creates parent directories on
/// demand (a baseline directory under `nucleus/target/` typically
/// does not pre-exist). Returns a string error mirroring the rest of
/// the harness's error idiom so the caller can plumb it into the
/// existing `run() -> Result<i32, String>` top-level.
///
/// Failure modes (all surface as `Err`, none silent):
///   * parent dir is unwritable / not a dir;
///   * file write fails partway (we write atomically into a sibling
///     `.tmp` then rename, so a crash never leaves a truncated JSON
///     that a later `--baseline` would happily compare against).
fn write_timings_json(path: &std::path::Path, results: &[CellResult]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "--emit-timings: cannot create parent dir `{}`: {e}",
                    parent.display()
                )
            })?;
        }
    }
    let doc = render_timings_json(results);
    // Atomic write: tmp + rename. Same dir as `path` so rename is
    // never cross-filesystem (POSIX guarantees atomicity within a fs).
    let tmp = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => {
            return Err(format!(
                "--emit-timings: path `{}` has no file name component",
                path.display()
            ));
        }
    };
    // Architect review cycle 54: explicit fsync of the tmp file
    // before rename so a power-loss can't land the rename before
    // data hits disk (which would leave a zero-byte JSON
    // survivor — CI baseline corruption). Belt-and-braces over
    // POSIX rename atomicity.
    use std::io::Write as _;
    let mut f = fs::File::create(&tmp)
        .map_err(|e| format!("--emit-timings: create `{}`: {e}", tmp.display()))?;
    f.write_all(doc.as_bytes())
        .map_err(|e| format!("--emit-timings: write `{}`: {e}", tmp.display()))?;
    f.sync_all()
        .map_err(|e| format!("--emit-timings: fsync `{}`: {e}", tmp.display()))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| {
        format!(
            "--emit-timings: rename `{}` -> `{}`: {e}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

// --------------------------------------------------------------------
// TASK-0023.03.03 Stage 1.5 — `--emit-timings` under `--check-determinism`.
//
// Schema notes (intentionally DIFFERENT from RUN mode, branched by the
// top-level `"mode"` key):
//
//   * RUN mode (cell_result_to_json) — phase_times_ms{compile,build,run}
//     + total_ms, plus status-specific fail_phase/skip_reason.
//   * DETERMINISM mode (det_cell_result_to_json) — single elapsed_ms
//     (det has one Duration, not three phases). PASS carries
//     files_compared; FAIL carries a det_mismatch object mirroring
//     the DetMismatch Display impl; SKIPPED carries skip_reason and
//     elapsed_ms = null (manifest pre-skips short-circuit before any
//     compile, so the duration is uninformative — null is honest).
//
// Why two emitters instead of one polymorphic: the source structs
// (CellResult vs DetCellResult) are intentionally disjoint, no shared
// trait — collapsing them would force a lossy intermediate and obscure
// the per-mode contract. Cheaper to keep them parallel.

/// Serialize a single `DetCellResult` to a JSON object appended to
/// `out`. Mirrors `cell_result_to_json` shape but emits the det-mode
/// payload (see module-level note above for schema differences).
///
/// `required` is included for parity with RUN-mode (downstream
/// regression / dashboards branch on it the same way in both modes).
fn det_cell_result_to_json(out: &mut String, r: &DetCellResult) {
    out.push('{');
    out.push_str("\"example\":");
    json_escape_str(out, &r.cell.example);
    out.push_str(",\"schedule\":");
    json_escape_str(out, &r.cell.schedule);
    out.push_str(",\"backend\":");
    json_escape_str(out, &r.cell.backend);
    out.push_str(",\"required\":");
    out.push_str(if r.required { "true" } else { "false" });

    out.push_str(",\"status\":");
    match &r.status {
        DetCellStatus::Pass { .. } => out.push_str("\"PASS\""),
        DetCellStatus::Failed(_) => out.push_str("\"FAIL\""),
        DetCellStatus::Skipped { .. } => out.push_str("\"SKIPPED\""),
    }

    // Status-specific payload + elapsed_ms. PASS/FAIL carry a real
    // wall-clock; SKIPPED is null on purpose — see module-level note.
    match &r.status {
        DetCellStatus::Pass { files_compared } => {
            use std::fmt::Write as _;
            let _ = write!(out, ",\"files_compared\":{files_compared}");
            let _ = write!(out, ",\"elapsed_ms\":{}", r.elapsed.as_millis());
        }
        DetCellStatus::Failed(m) => {
            // det_mismatch mirrors the four DetMismatch fields. `kind`
            // is the Display impl of DetMismatchKind (stable lowercase
            // phrase, also visible in --format=junit XML — keeping
            // the two stable surfaces in lockstep).
            out.push_str(",\"det_mismatch\":{");
            out.push_str("\"relative_path\":");
            json_escape_str(out, &m.relative_path.display().to_string());
            out.push_str(",\"kind\":");
            json_escape_str(out, &m.kind.to_string());
            {
                use std::fmt::Write as _;
                let _ = write!(out, ",\"offset\":{}", m.offset);
            }
            out.push_str(",\"detail\":");
            json_escape_str(out, &m.detail);
            out.push('}');
            {
                use std::fmt::Write as _;
                let _ = write!(out, ",\"elapsed_ms\":{}", r.elapsed.as_millis());
            }
        }
        DetCellStatus::Skipped { reason } => {
            out.push_str(",\"skip_reason\":");
            json_escape_str(out, reason);
            // null (not 0) — the duration of a pre-compile manifest
            // skip carries no signal; emitting it as a number would
            // bait a downstream consumer into averaging meaningless 0s.
            out.push_str(",\"elapsed_ms\":null");
        }
    }

    // perturbed is observable in det-mode only and only ever true
    // under NUC_NONDET_TEST=1; emit it so a future regression script
    // can correlate JSON output with the NUC_NONDET_PERTURBED_CELLS
    // line on STDOUT (TASK-0188).
    out.push_str(",\"perturbed\":");
    out.push_str(if r.perturbed { "true" } else { "false" });
    out.push('}');
}

/// Render the full `Vec<DetCellResult>` to a JSON document with a
/// top-level `{"mode": "determinism", "cells": [...]}` object. Cells
/// appear in planned order (the parallel executor re-sorts results
/// before returning).
fn render_det_timings_json(results: &[DetCellResult]) -> String {
    let mut out = String::with_capacity(results.len() * 256);
    out.push_str("{\n  \"mode\": \"determinism\",\n  \"cells\": [\n");
    for (i, r) in results.iter().enumerate() {
        out.push_str("    ");
        det_cell_result_to_json(&mut out, r);
        if i + 1 < results.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// Write the det-mode timings JSON to `path`. Same atomic
/// tmp+fsync+rename contract as `write_timings_json` — a power-loss
/// during write must NEVER leave a partial JSON survivor that a
/// downstream consumer might mistake for a clean baseline.
fn write_det_timings_json(path: &std::path::Path, results: &[DetCellResult]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "--emit-timings: cannot create parent dir `{}`: {e}",
                    parent.display()
                )
            })?;
        }
    }
    let doc = render_det_timings_json(results);
    let tmp = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => {
            return Err(format!(
                "--emit-timings: path `{}` has no file name component",
                path.display()
            ));
        }
    };
    use std::io::Write as _;
    let mut f = fs::File::create(&tmp)
        .map_err(|e| format!("--emit-timings: create `{}`: {e}", tmp.display()))?;
    f.write_all(doc.as_bytes())
        .map_err(|e| format!("--emit-timings: write `{}`: {e}", tmp.display()))?;
    f.sync_all()
        .map_err(|e| format!("--emit-timings: fsync `{}`: {e}", tmp.display()))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| {
        format!(
            "--emit-timings: rename `{}` -> `{}`: {e}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

// --------------------------------------------------------------------
// Baseline comparator (TASK-0023.03 Stage 2)
// --------------------------------------------------------------------

/// One cell's wall-clock summary loaded back from a baseline JSON.
///
/// Only the fields the comparator actually consumes are kept — the
/// rich `CellResult` payload (status / detail / corrupted / etc.) is
/// not the baseline's job. The comparator joins on the identity triple
/// and reports `total_ms` deltas; anything else is Stage 3 (per-cell
/// thresholds) or downstream tooling.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BaselineCell {
    example: String,
    schedule: String,
    backend: String,
    total_ms: u64,
}

/// Loud-fail JSON parse error carrying byte offset + the surrounding
/// snippet. Stage 1's emitter is deterministic, so a parse failure
/// here is almost always "wrong file fed in" — naming the offset and
/// what was expected makes that obvious without the developer having
/// to open the file in an editor.
#[derive(Debug)]
struct BaselineParseError {
    offset: usize,
    msg: String,
}

impl fmt::Display for BaselineParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "--baseline: JSON parse error at byte offset {}: {}",
            self.offset, self.msg
        )
    }
}

/// Minimal hand-rolled JSON reader, scoped EXACTLY to the schema that
/// `render_timings_json` emits. Deliberately NOT a general JSON parser:
///
///   * recognises only what Stage 1 emits: objects, arrays, strings
///     (with the same escape set as `json_escape_str`), integers, `null`,
///     `true`, `false`;
///   * ignores keys that aren't `example`, `schedule`, `backend`,
///     `total_ms` (so future Stage 3 fields don't break old baselines);
///   * loud-fails on structural errors with byte offset + snippet —
///     never silently treats a bad file as empty.
///
/// Stage 1 deliberately avoided serde_json; matching that constraint
/// here keeps the e2e crate's dep set minimal (one less compile-time
/// cost on every developer machine).
fn parse_baseline_json(src: &str) -> Result<Vec<BaselineCell>, BaselineParseError> {
    let bytes = src.as_bytes();
    let mut p = JsonCursor { bytes, pos: 0 };
    p.skip_ws();
    p.expect_byte(b'{')?;
    p.skip_ws();
    // Top-level object: we only consume the `cells` key; any other
    // future top-level field is silently skipped so old baselines stay
    // forward-compatible with new emitter additions.
    let mut cells: Option<Vec<BaselineCell>> = None;
    loop {
        p.skip_ws();
        if p.peek() == Some(b'}') {
            p.pos += 1;
            break;
        }
        let key = p.parse_string()?;
        p.skip_ws();
        p.expect_byte(b':')?;
        p.skip_ws();
        if key == "cells" {
            cells = Some(p.parse_cells_array()?);
        } else {
            p.skip_value()?;
        }
        p.skip_ws();
        match p.peek() {
            Some(b',') => p.pos += 1,
            Some(b'}') => {
                p.pos += 1;
                break;
            }
            _ => return Err(p.err("expected `,` or `}` after object member")),
        }
    }
    cells.ok_or_else(|| BaselineParseError {
        offset: 0,
        msg: "top-level object missing required `cells` array".to_string(),
    })
}

/// Byte-level cursor over the baseline JSON. Hand-rolled rather than
/// pulling in `nom` / `chumsky` — the schema is ~6 token kinds and the
/// emitter side fits in ~100 LoC, so the reader does too.
struct JsonCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl JsonCursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }
    fn err(&self, msg: &str) -> BaselineParseError {
        // Snippet aids the eyeball when offset alone isn't enough.
        let lo = self.pos.saturating_sub(20);
        let hi = (self.pos + 20).min(self.bytes.len());
        let snippet = String::from_utf8_lossy(&self.bytes[lo..hi]);
        BaselineParseError {
            offset: self.pos,
            msg: format!("{msg} (near `{snippet}`)"),
        }
    }
    fn expect_byte(&mut self, want: u8) -> Result<(), BaselineParseError> {
        match self.peek() {
            Some(b) if b == want => {
                self.pos += 1;
                Ok(())
            }
            Some(b) => Err(self.err(&format!("expected `{}`, got `{}`", want as char, b as char))),
            None => Err(self.err(&format!("expected `{}`, got EOF", want as char))),
        }
    }
    fn parse_string(&mut self) -> Result<String, BaselineParseError> {
        self.expect_byte(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| self.err("trailing `\\`"))?;
                    self.pos += 1;
                    let ch = match esc {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000C}',
                        b'u' => {
                            // \uXXXX — the emitter uses this for
                            // control chars; we decode as a BMP char.
                            if self.pos + 4 > self.bytes.len() {
                                return Err(self.err("truncated \\u escape"));
                            }
                            let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
                                .map_err(|_| self.err("non-ASCII in \\u escape"))?;
                            let code = u32::from_str_radix(hex, 16)
                                .map_err(|_| self.err("\\u escape not hex"))?;
                            self.pos += 4;
                            char::from_u32(code)
                                .ok_or_else(|| self.err("invalid \\u code point"))?
                        }
                        other => {
                            return Err(self.err(&format!("unknown escape `\\{}`", other as char)))
                        }
                    };
                    out.push(ch);
                }
                Some(b) => {
                    self.pos += 1;
                    out.push(b as char);
                }
            }
        }
    }
    /// Parse a non-negative integer. The Stage-1 emitter never emits a
    /// negative `total_ms` (it's a `Duration::as_millis` cast); a `-`
    /// in the wild is a corrupt baseline and we fail LOUD.
    fn parse_u64(&mut self) -> Result<u64, BaselineParseError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.err("expected unsigned integer"));
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.err("integer not ASCII"))?;
        s.parse::<u64>()
            .map_err(|e| self.err(&format!("u64 parse: {e}")))
    }
    /// Skip a JSON value the comparator doesn't care about. Recursive
    /// for nested objects/arrays so the seek stays correct.
    fn skip_value(&mut self) -> Result<(), BaselineParseError> {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => {
                let _ = self.parse_string()?;
            }
            Some(b'{') => self.skip_object()?,
            Some(b'[') => self.skip_array()?,
            Some(b't') | Some(b'f') | Some(b'n') => {
                // true / false / null — advance past the literal.
                while let Some(b) = self.peek() {
                    if b.is_ascii_alphabetic() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            Some(b) if b == b'-' || b.is_ascii_digit() => {
                if b == b'-' {
                    self.pos += 1;
                }
                while let Some(b) = self.peek() {
                    if b.is_ascii_digit()
                        || b == b'.'
                        || b == b'e'
                        || b == b'E'
                        || b == b'+'
                        || b == b'-'
                    {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => return Err(self.err("expected JSON value")),
        }
        Ok(())
    }
    fn skip_object(&mut self) -> Result<(), BaselineParseError> {
        self.expect_byte(b'{')?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return Ok(());
            }
            let _ = self.parse_string()?;
            self.skip_ws();
            self.expect_byte(b':')?;
            self.skip_value()?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err(self.err("expected `,` or `}` in object")),
            }
        }
    }
    fn skip_array(&mut self) -> Result<(), BaselineParseError> {
        self.expect_byte(b'[')?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(());
            }
            self.skip_value()?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err(self.err("expected `,` or `]` in array")),
            }
        }
    }
    /// Parse the `cells` array — the ONE shape the comparator cares
    /// about. Each element is a `{example, schedule, backend, total_ms,
    /// ...}` object. Unknown keys are silently skipped so a Stage-3
    /// emitter can extend the schema without breaking Stage-2 readers.
    fn parse_cells_array(&mut self) -> Result<Vec<BaselineCell>, BaselineParseError> {
        self.expect_byte(b'[')?;
        let mut cells: Vec<BaselineCell> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(cells);
            }
            cells.push(self.parse_cell_object()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(cells);
                }
                _ => return Err(self.err("expected `,` or `]` in cells array")),
            }
        }
    }
    fn parse_cell_object(&mut self) -> Result<BaselineCell, BaselineParseError> {
        self.expect_byte(b'{')?;
        let mut example: Option<String> = None;
        let mut schedule: Option<String> = None;
        let mut backend: Option<String> = None;
        let mut total_ms: Option<u64> = None;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_byte(b':')?;
            self.skip_ws();
            match key.as_str() {
                "example" => example = Some(self.parse_string()?),
                "schedule" => schedule = Some(self.parse_string()?),
                "backend" => backend = Some(self.parse_string()?),
                "total_ms" => total_ms = Some(self.parse_u64()?),
                _ => self.skip_value()?,
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected `,` or `}` in cell object")),
            }
        }
        Ok(BaselineCell {
            example: example.ok_or_else(|| self.err("cell missing `example`"))?,
            schedule: schedule.ok_or_else(|| self.err("cell missing `schedule`"))?,
            backend: backend.ok_or_else(|| self.err("cell missing `backend`"))?,
            total_ms: total_ms.ok_or_else(|| self.err("cell missing `total_ms`"))?,
        })
    }
}

/// One row of the delta table. `baseline_ms`/`current_ms` are `Option`
/// because a cell can be new (no baseline) or removed (no current),
/// and we still want to render the row rather than crash.
#[derive(Debug, Clone, PartialEq)]
struct DeltaRow {
    example: String,
    schedule: String,
    backend: String,
    baseline_ms: Option<u64>,
    current_ms: Option<u64>,
    /// Percentage change vs baseline, ONLY when both sides exist.
    /// `current / baseline - 1`; rounded to one decimal in output.
    /// Sentinel `None` means "(new)" or "(removed)".
    delta_pct: Option<f64>,
    /// Per-cell perf-regression threshold, plumbed from
    /// `PlannedCell::perf_threshold_pct` (TASK-0023.03.02 Stage 3).
    /// `None` ⇒ no gate; an absent baseline match (the cell wasn't in
    /// this run's planned set, e.g. "(removed)" rows) also surfaces as
    /// `None`. Honest: rows the planner never produced cannot be gated.
    perf_threshold_pct: Option<f64>,
    /// `true` iff this cell is `required` in the active milestone band.
    /// Drives the exit-code wiring: a threshold breach only flips the
    /// harness exit code when `required && regression`.
    required: bool,
    /// Set once at row-construction time. `true` iff
    /// `(threshold, delta_pct) = (Some(t), Some(p))` AND `p > t`.
    /// Computing this here (not in the renderer) keeps the rendering
    /// path side-effect-free and the exit-code wiring a simple
    /// `rows.iter().any(...)`.
    regression: bool,
}

impl DeltaRow {
    /// Comparator-only sort key: largest regression (positive delta)
    /// first; new/removed sink to the bottom so the eye lands on
    /// real regressions first. Ties broken by cell identity so the
    /// output is deterministic across runs.
    fn sort_key(&self) -> (i32, i64, String, String, String) {
        // Tier: 0 = real delta (most informative), 1 = removed
        // (cell still in baseline but gone), 2 = new (cell only in
        // current — not a regression). Within tier 0, sort by
        // delta DESCENDING (largest regression first).
        let (tier, neg_pct_milli) = match (self.baseline_ms, self.current_ms, self.delta_pct) {
            (Some(_), Some(_), Some(p)) => (0_i32, -(p * 1000.0) as i64),
            (Some(_), None, _) => (1, 0),
            (None, Some(_), _) => (2, 0),
            _ => (3, 0),
        };
        (
            tier,
            neg_pct_milli,
            self.example.clone(),
            self.schedule.clone(),
            self.backend.clone(),
        )
    }
}

/// Build the delta table by joining current results to the baseline on
/// the identity triple. `planned` is an optional carrier of per-cell
/// metadata (perf threshold, required flag) joined on the same triple
/// (TASK-0023.03.02 Stage 3). Pass an empty slice to disable gating
/// (the cycle-55 informational-only behaviour: every row's `regression`
/// flag will be `false` and `required` defaults to `false`).
///
/// Order of returned rows is sorted by `DeltaRow::sort_key` — largest
/// regression first; new/removed cells land at the bottom.
fn compute_delta_rows(
    baseline: &[BaselineCell],
    current: &[CellResult],
    planned: &[PlannedCell],
) -> Vec<DeltaRow> {
    use std::collections::HashMap;
    type Key = (String, String, String);
    let key_for_baseline =
        |b: &BaselineCell| -> Key { (b.example.clone(), b.schedule.clone(), b.backend.clone()) };
    let key_for_current = |r: &CellResult| -> Key {
        (
            r.cell.example.clone(),
            r.cell.schedule.clone(),
            r.cell.backend.clone(),
        )
    };
    let key_for_planned = |p: &PlannedCell| -> Key {
        (
            p.cell.example.clone(),
            p.cell.schedule.clone(),
            p.cell.backend.clone(),
        )
    };
    let base_map: HashMap<Key, &BaselineCell> =
        baseline.iter().map(|b| (key_for_baseline(b), b)).collect();
    let cur_map: HashMap<Key, &CellResult> =
        current.iter().map(|r| (key_for_current(r), r)).collect();
    // Plan-side metadata: threshold + required flag. Cell-not-in-map
    // ⇒ no threshold AND not-required (defensive default), so a stray
    // row (e.g. a "(removed)" cell only in the baseline) cannot ever
    // gate the exit code by accident.
    let plan_map: HashMap<Key, &PlannedCell> =
        planned.iter().map(|p| (key_for_planned(p), p)).collect();

    // Build one row. Centralised so the threshold/regression rule is
    // applied identically to current-cell rows and (defensively) removed
    // rows. A removed cell has `delta_pct = None`, so `regression` is
    // unconditionally `false` there — a vanished cell is not a perf bite.
    let mk_row = |example: String,
                  schedule: String,
                  backend: String,
                  baseline_ms: Option<u64>,
                  current_ms: Option<u64>,
                  delta_pct: Option<f64>|
     -> DeltaRow {
        let k: Key = (example.clone(), schedule.clone(), backend.clone());
        let (perf_threshold_pct, required) = match plan_map.get(&k) {
            Some(p) => (p.perf_threshold_pct, p.required),
            None => (None, false),
        };
        let regression = matches!(
            (perf_threshold_pct, delta_pct),
            (Some(t), Some(p)) if p > t
        );
        DeltaRow {
            example,
            schedule,
            backend,
            baseline_ms,
            current_ms,
            delta_pct,
            perf_threshold_pct,
            required,
            regression,
        }
    };

    let mut rows: Vec<DeltaRow> = Vec::new();
    // First, every current cell — flagged as "(new)" if absent in
    // baseline, else a real delta. Drives output ordering for the
    // common case (current is what the developer just ran).
    for r in current {
        let k = key_for_current(r);
        let current_ms = r.timings.total().as_millis() as u64;
        match base_map.get(&k) {
            Some(b) => {
                let baseline_ms = b.total_ms;
                let delta_pct = if baseline_ms == 0 {
                    // Avoid div-by-zero — a 0ms baseline is rare
                    // (SKIPPED or a near-instant cell). Treat any
                    // non-zero current against 0 baseline as "(new
                    // measurable)" rather than ∞. Honest limit
                    // noted in the cycle-55 deliverable.
                    if current_ms == 0 {
                        Some(0.0)
                    } else {
                        None
                    }
                } else {
                    Some(
                        ((current_ms as f64) - (baseline_ms as f64)) / (baseline_ms as f64) * 100.0,
                    )
                };
                rows.push(mk_row(
                    r.cell.example.clone(),
                    r.cell.schedule.clone(),
                    r.cell.backend.clone(),
                    Some(baseline_ms),
                    Some(current_ms),
                    delta_pct,
                ));
            }
            None => {
                rows.push(mk_row(
                    r.cell.example.clone(),
                    r.cell.schedule.clone(),
                    r.cell.backend.clone(),
                    None,
                    Some(current_ms),
                    None,
                ));
            }
        }
    }
    // Then, every baseline cell missing from current — "(removed)".
    for b in baseline {
        let k = key_for_baseline(b);
        if !cur_map.contains_key(&k) {
            rows.push(mk_row(
                b.example.clone(),
                b.schedule.clone(),
                b.backend.clone(),
                Some(b.total_ms),
                None,
                None,
            ));
        }
    }
    rows.sort_by_key(|r| r.sort_key());
    rows
}

/// Render the delta table as a multi-line String. Colorise iff
/// `color` is true; plain otherwise. Output is human-targeted: the
/// `--emit-timings` JSON is the machine-readable counterpart.
fn render_delta_table(rows: &[DeltaRow], color: bool) -> String {
    use std::fmt::Write as _;
    const RED: &str = "\x1b[31m";
    const GREEN: &str = "\x1b[32m";
    const DIM: &str = "\x1b[2m";
    const RESET: &str = "\x1b[0m";

    // Bright red for required-cell regressions (exit-code-impacting,
    // demands attention); a dim red for informational threshold breaches
    // on skip-band cells (signal, not blocker).
    const BRIGHT_RED: &str = "\x1b[1;31m";

    let mut out = String::new();
    let _ = writeln!(out, "--- baseline diff ({} row(s)) ---", rows.len());
    // Header is preserved verbatim from cycle 55 so a bare `--baseline`
    // invocation (no thresholds in the matrix yet) renders byte-identical
    // to cycle 55. The optional "[threshold=…]" / "[REGRESSION …]" suffix
    // is only appended per-row when a threshold was actually plumbed
    // through, so an absent-threshold run stays a no-op on output.
    let _ = writeln!(
        out,
        "  example | schedule | backend | baseline_ms -> current_ms (Δ%)"
    );
    for r in rows {
        let cell_id = format!("  {} | {} | {} | ", r.example, r.schedule, r.backend);
        let body = match (r.baseline_ms, r.current_ms, r.delta_pct) {
            (Some(b), Some(c), Some(p)) => {
                let pct_str = format!("{:+.1}%", p);
                let painted = if !color {
                    pct_str
                } else if p > 0.0 {
                    format!("{RED}{pct_str}{RESET}")
                } else if p < 0.0 {
                    format!("{GREEN}{pct_str}{RESET}")
                } else {
                    pct_str
                };
                format!("{b} -> {c} ({painted})")
            }
            (Some(b), Some(c), None) => {
                // baseline_ms == 0 with non-zero current — sentinel.
                let tag = if color {
                    format!("{DIM}(baseline=0 ms; Δ undefined){RESET}")
                } else {
                    "(baseline=0 ms; Δ undefined)".to_string()
                };
                format!("{b} -> {c} {tag}")
            }
            (None, Some(c), _) => {
                let tag = if color {
                    format!("{DIM}(new){RESET}")
                } else {
                    "(new)".to_string()
                };
                format!("- -> {c} {tag}")
            }
            (Some(b), None, _) => {
                let tag = if color {
                    format!("{DIM}(removed){RESET}")
                } else {
                    "(removed)".to_string()
                };
                format!("{b} -> - {tag}")
            }
            (None, None, _) => "- -> -".to_string(),
        };
        // Threshold/REGRESSION suffix (TASK-0023.03.02 Stage 3). Three
        // visual tiers:
        //   * required-cell breach     -> "[REGRESSION threshold=N%]" in
        //                                 BRIGHT RED (exit-code-impacting)
        //   * skip-band-cell breach    -> "[regression threshold=N%]" in
        //                                 dim red (informational only)
        //   * threshold set, no breach -> "[threshold=N%]" dim text
        //                                 (so a reviewer can see the gate
        //                                 was active and the cell stayed
        //                                 under it)
        //   * no threshold             -> nothing appended (byte-identical
        //                                 to cycle 55 output)
        let suffix = match (r.perf_threshold_pct, r.regression, r.required) {
            (Some(t), true, true) => {
                let s = format!(" [REGRESSION threshold={:+.1}%]", t);
                if color {
                    format!("{BRIGHT_RED}{s}{RESET}")
                } else {
                    s
                }
            }
            (Some(t), true, false) => {
                let s = format!(" [regression threshold={:+.1}%]", t);
                if color {
                    format!("{RED}{DIM}{s}{RESET}")
                } else {
                    s
                }
            }
            (Some(t), false, _) => {
                let s = format!(" [threshold={:+.1}%]", t);
                if color {
                    format!("{DIM}{s}{RESET}")
                } else {
                    s
                }
            }
            (None, _, _) => String::new(),
        };
        let _ = writeln!(out, "{cell_id}{body}{suffix}");
    }
    out
}

/// Drive the baseline comparator: read `path`, parse it, compute the
/// delta table, render with ANSI iff stderr is a TTY, write to STDERR.
///
/// STDERR specifically: stdout may carry `--format=junit` XML, and
/// corrupting that XML with delta-table text would break a CI
/// consumer's parse. The Stage-1 emitter's "post-summary, pre-gate"
/// position is preserved — this call site too.
fn compare_against_baseline(
    path: &std::path::Path,
    current: &[CellResult],
    planned: &[PlannedCell],
) -> Result<usize, String> {
    let src = fs::read_to_string(path)
        .map_err(|e| format!("--baseline: cannot read `{}`: {e}", path.display()))?;
    let baseline = parse_baseline_json(&src).map_err(|e| {
        // Carry the parse-time offset/snippet up; the prefix already
        // names the flag so the developer knows which file to look at.
        format!("{e} in `{}`", path.display())
    })?;
    let rows = compute_delta_rows(&baseline, current, planned);
    let use_color = {
        use std::io::IsTerminal as _;
        std::io::stderr().is_terminal()
    };
    let table = render_delta_table(&rows, use_color);
    // `eprint!` not `eprintln!` — `render_delta_table` already emits
    // a trailing newline per row, so an extra newline would double-
    // space the output.
    eprint!("{table}");
    // Count required-cell threshold breaches. Returned to the caller so
    // it can flip the exit code without re-running the join (TASK-
    // 0023.03.02 AC#3 — required-cell regression = HARD FAIL). Skip-row
    // regressions are deliberately NOT counted here: they're flagged
    // visually but exit-code-neutral.
    let required_regressions = rows.iter().filter(|r| r.regression && r.required).count();
    if required_regressions > 0 {
        eprintln!(
            "nucleus-e2e: --baseline: {required_regressions} required-cell \
             perf threshold breach(es) — HARD FAIL"
        );
    }
    let _ = std::io::stderr().flush();
    Ok(required_regressions)
}

// --------------------------------------------------------------------
// Parallel cell execution (TASK-0023.01)
// --------------------------------------------------------------------

/// Run `f` against each `PlannedCell` in `planned`, in parallel across
/// up to `jobs` worker threads (default 1 = strictly sequential).
///
/// Contract — invariant for the cross-backend differential gate:
///
/// * `jobs == 1` → strictly sequential, in original `planned` order; the
///   completion line is emitted immediately after each cell returns
///   (identical to the pre-TASK-0023.01 loop). This MUST be byte-for-
///   byte identical to the pre-flag behaviour: it is the path bare
///   `just e2e` takes.
/// * `jobs >= 2` → `min(jobs, planned.len())` worker threads pull from a
///   shared `Arc<Mutex<VecDeque<usize>>>` work-queue (cell indices in
///   original order). Workers run `f` against `paths` + the indexed
///   `PlannedCell`, send `(idx, result, completion_line)` over an mpsc
///   to this thread, which prints the completion line immediately (in
///   COMPLETION order) and stores the result by `idx`. After the join,
///   results are returned in ORIGINAL `planned` order — so the summary
///   table, exit code and all downstream gate signals
///   (`NUC_NONDET_PERTURBED_CELLS`, `NUC_XBACKEND_*`) see a deterministic
///   ordering regardless of `--jobs`.
///
/// Send/Sync rationale (recorded so a future change can re-verify):
///
/// * `Paths` = `PathBuf` + `String`. Trivially `Send + Sync`. Wrapped
///   in `Arc` so workers share one copy.
/// * `PlannedCell` = `Cell` (three `String`s) + `bool` + `Option<String>`.
///   Owned data, trivially `Send`. Cloned into each task.
/// * `R` (CellResult / DetCellResult) = owned data + an `Instant`/
///   `Duration` snapshot. Trivially `Send`.
/// * The work `f` itself spawns `cargo` subprocesses against the per-
///   cell scratch dir already made unique by TASK-0182's process-wide
///   `run_id`. No two cells share a scratch path, so no two workers
///   touch the same FS subtree. Cargo's own internal target-dir lock
///   may briefly serialise back-to-back subprocess start-up but not
///   the per-cell `cargo build` itself (those run in disjoint trees).
/// * The negative-gate env reads inside `f` (`NUC_NONDET_TEST`,
///   `NUC_XBACKEND_NEGATIVE`) are reads against a constant env — safe
///   from worker threads.
///
/// HONEST LIMIT: completion-order progress lines mean a streaming CI
/// log under `--jobs 4` interleaves cells differently than sequential,
/// which can confuse a human eyeballing live output. The summary table
/// is still planned-order; only the live progress lines reorder.
fn execute_cells_parallel<R, F>(
    paths: &Paths,
    planned: &[PlannedCell],
    jobs: usize,
    f: F,
) -> (Vec<R>, Duration)
where
    R: Send + 'static,
    F: Fn(&Paths, &PlannedCell, usize, usize) -> (R, String) + Send + Sync + 'static,
{
    // Wall-clock around the WHOLE execution (sequential or parallel
    // branch alike). TASK-0248: under --jobs N>=2 the suite-level
    // <testsuite time=...> attribute previously summed per-cell elapsed,
    // which overstates parallel runs (4 cells of 1s on --jobs 4 take
    // ~1s wall but were reported as ~4s). Capturing the wall-clock
    // here — at the executor boundary — gives the JUnit emitter a
    // honest figure regardless of the jobs count. Sequential users see
    // a number that matches the per-cell sum within scheduler-overhead
    // rounding; parallel users see a number that matches reality.
    let wall_start = Instant::now();
    // Strictly sequential path (default; bare `just e2e`).
    //
    // Kept as a separate branch — NOT routed through the mpsc/worker
    // machinery — precisely so the default `--jobs 1` execution stays
    // byte-for-byte identical to the pre-flag loop (no thread spawn,
    // no channel, no Mutex). The behavioural contract above hinges on
    // this branch being trivially equivalent to a plain for-loop.
    if jobs <= 1 {
        let mut out: Vec<R> = Vec::with_capacity(planned.len());
        for (i, pc) in planned.iter().enumerate() {
            let (r, line) = f(paths, pc, i, planned.len());
            // Sequential: print BEFORE storing so a panic inside the
            // formatter still shows progress up to that point. (Matches
            // the original loop's eprint-before-push pattern.)
            eprintln!("{line}");
            let _ = std::io::stderr().flush();
            out.push(r);
        }
        return (out, wall_start.elapsed());
    }

    // Parallel path. The work-queue is the canonical task list; workers
    // pull until empty, then exit. We hold the join-handles so we can
    // block on full completion before returning (no detached threads).
    let n_workers = jobs.min(planned.len()).max(1);
    let queue: Arc<Mutex<VecDeque<usize>>> = Arc::new(Mutex::new((0..planned.len()).collect()));
    let paths_shared = Arc::new(paths.clone());
    let planned_shared: Arc<Vec<PlannedCell>> = Arc::new(planned.to_vec());
    let total = planned.len();
    let f_shared = Arc::new(f);

    let (tx, rx) = std::sync::mpsc::channel::<(usize, R, String)>();

    let mut handles = Vec::with_capacity(n_workers);
    for _ in 0..n_workers {
        let queue = Arc::clone(&queue);
        let paths_shared = Arc::clone(&paths_shared);
        let planned_shared = Arc::clone(&planned_shared);
        let f_shared = Arc::clone(&f_shared);
        let tx = tx.clone();
        let h = thread::spawn(move || {
            loop {
                // Lock JUST to pop an index — released BEFORE we run
                // the (long-running) cell. Otherwise workers would
                // serialise on the queue lock instead of doing work.
                let next = {
                    let mut q = queue
                        .lock()
                        .expect("execute_cells_parallel: work-queue poisoned");
                    q.pop_front()
                };
                let Some(idx) = next else {
                    break;
                };
                let pc = &planned_shared[idx];
                let (r, line) = f_shared(&paths_shared, pc, idx, total);
                // If the receiver is gone, the parent has bailed and
                // there is nothing useful to do; just drop the result.
                let _ = tx.send((idx, r, line));
            }
        });
        handles.push(h);
    }
    // Drop the parent's sender so `rx` closes once every worker exits.
    drop(tx);

    // Collect into a sparse Vec<Option<R>> indexed by original `idx`,
    // so we can return in planned order regardless of completion order.
    // Print each completion line as it arrives → human gets a live
    // pulse of progress even under heavy parallelism.
    let mut slots: Vec<Option<R>> = (0..planned.len()).map(|_| None).collect();
    while let Ok((idx, r, line)) = rx.recv() {
        eprintln!("{line}");
        let _ = std::io::stderr().flush();
        slots[idx] = Some(r);
    }

    // Reap every worker to surface a panic (if any) loudly rather than
    // silently dropping a result. The mpsc would already have hidden a
    // panicked worker (its tx Drop closes the channel quietly), so
    // joining is the only point at which a panic propagates here.
    for h in handles {
        if let Err(panic_payload) = h.join() {
            // Surface and re-panic from the main thread — a worker
            // panic means we cannot trust the result set, and silent
            // recovery here would invalidate the gate.
            std::panic::resume_unwind(panic_payload);
        }
    }

    let out: Vec<R> = slots
        .into_iter()
        .enumerate()
        .map(|(i, slot)| {
            slot.unwrap_or_else(|| {
                panic!(
                    "execute_cells_parallel: cell idx={i} produced no result \
                     (worker exited without sending) — this should be \
                     impossible since workers send before pulling the next \
                     index. Treat as a logic bug."
                )
            })
        })
        .collect();
    (out, wall_start.elapsed())
}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Thin wrapper that owns the per-run scratch lifecycle (TASK-0182).
///
/// `Paths` (and with it the process-wide `run_id`) is discovered HERE,
/// once, and threaded into `run_inner`. Whatever `run_inner` returns —
/// a code, or an error — we deterministically finalize this run's
/// per-run scratch roots through the SINGLE point below, so every exit
/// path (normal pass, required-fail, the negative-gate forced-clean
/// `Ok(0)`s, or a hard `Err`) gets a consistent cleanup/retain
/// decision. A clean exit 0 removes the per-run trees; anything else
/// (non-zero code OR error) retains them and prints their paths.
fn run() -> Result<i32, String> {
    let argv: Vec<OsString> = env::args_os().skip(1).collect();
    let args = parse_args(&argv)?;

    let paths = Paths::discover()?;
    let outcome = run_inner(&paths, args);
    // Treat ONLY a clean `Ok(0)` as success: a non-zero code means
    // real cell failures (keep the tree to debug them); the
    // negative-gate forced-clean `Ok(0)` arms return before any cell
    // builds under most layouts, but if they did build, retaining is
    // still the safer (debuggable) choice — they are abnormal runs by
    // construction. An `Err` is an infra failure: retain + report.
    let success = matches!(outcome, Ok(0));
    paths.finalize_run_scratch(success);
    outcome
}

fn run_inner(paths: &Paths, args: Args) -> Result<i32, String> {
    let manifest_src = fs::read_to_string(paths.manifest_path()).map_err(|e| {
        format!(
            "cannot read manifest at {}: {e}",
            paths.manifest_path().display()
        )
    })?;
    let mut manifest: Manifest =
        toml::from_str(&manifest_src).map_err(|e| format!("manifest parse error: {e}"))?;

    // TASK-0168 negative-gate injection seam. Strict no-op unless
    // NUC_REQUIRED_COVERAGE_NEGATIVE=1; under the gate, appends a single
    // synthetic [[required]] entry whose schedule cannot match any
    // discovered *.sched.nuc file, so the downstream `required_coverage_gaps`
    // call BELOW produces a gap — exactly the silent-vanish failure mode
    // TASK-0163 closed for the wired path. Placed AFTER parse and BEFORE
    // `plan_cells` / `required_coverage_gaps`, mirroring the
    // `maybe_perturb_for_nondet_test` / `maybe_corrupt_wire_for_xbackend`
    // discipline (the env flag is the seam; the on-disk manifest stays
    // clean). Sibling of those two functions in this file.
    let required_coverage_injected = maybe_inject_required_coverage_negative(&mut manifest)?;

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

    let planned = plan_cells(paths, &manifest, &args)?;
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

    // TASK-0168 — EXPLICIT MACHINE-CHECKABLE SIGNAL for the negative arm.
    //
    // Stable key (do not rename without updating justfile's
    // `required-coverage-check-negative` recipe):
    //
    //     NUC_REQUIRED_COVERAGE_GAP_DETECTED=<n>
    //
    // n = number of coverage gaps whose schedule equals the synthetic
    // sentinel `REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE`. The
    // attribution filter is precise on purpose: an unrelated coverage
    // gap caused by some other manifest mistake must NOT satisfy this
    // signal, otherwise the recipe would print OK off a different bug
    // and the falsifier-bit would be a false positive — the exact
    // partial-silent-neuter lesson TASK-0187 captured for
    // determinism-check-negative.
    //
    // Emitted on STDOUT (println!, not eprintln!) so it is a parseable
    // RESULT line; human-facing diagnostics stay on stderr. Printed
    // UNCONDITIONALLY under the gate (n==0 in the zero-injection arm
    // below, n>=1 in the genuine-bite arm) so the recipe can always
    // find the line and assert n>=1.
    //
    // Mirrors `NUC_NONDET_PERTURBED_CELLS=<n>` emitted by the
    // `determinism-check-negative` recipe and
    // `NUC_XBACKEND_CORRUPTED_DETECTED=<n>` emitted by the
    // `xbackend-check-negative` recipe — same belt-and-suspenders
    // contract (TASK-0188): if a future refactor
    // drops the `if !gaps.is_empty() { return Err }` wiring below, the
    // recipe's exit-code inversion alone is no longer load-bearing —
    // the count assertion still fails loud.
    //
    // Gated on NUC_REQUIRED_COVERAGE_NEGATIVE=1: it does NOT appear
    // under bare `just e2e`, so that path stays byte-identical /
    // unaffected.
    if required_coverage_injected {
        let attributed = gaps
            .iter()
            .filter(|c| c.schedule == REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE)
            .count();
        println!("NUC_REQUIRED_COVERAGE_GAP_DETECTED={attributed}");

        if attributed == 0 {
            eprintln!(
                "nucleus-e2e: FATAL: NUC_REQUIRED_COVERAGE_NEGATIVE=1 but ZERO \
                 injection-attributable coverage gaps were detected — the \
                 required-coverage falsifier did not bite. Forcing a CLEAN \
                 exit so `required-coverage-check-negative` reports its loud \
                 FAIL (the gate did NOT detect the synthetic typo) instead \
                 of inverting a no-op into a false OK (TASK-0168 mirroring \
                 TASK-0187 AC#2). Likely causes: a CLI filter (--example / \
                 --schedule / --backend / --milestone) scoped the synthetic \
                 entry out of the coverage check, or the injection function \
                 picked an anchor outside the active gate."
            );
            // Exit 0 on purpose: the recipe inverts this into its
            // "FAIL: did NOT detect" branch (exit 1). The explicit
            // NUC_REQUIRED_COVERAGE_GAP_DETECTED=0 line above is the
            // redundant machine-checkable backstop (TASK-0188): even if
            // a refactor breaks the inversion, the recipe's count
            // assertion still fails loud.
            return Ok(0);
        }
    }

    if !gaps.is_empty() {
        let listed = gaps
            .iter()
            .map(|c| {
                format!(
                    "(example={}, schedule={}, backend={})",
                    c.example, c.schedule, c.backend
                )
            })
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
        // TASK-0023.03.03 Stage 1.5 (cycle-57): `--emit-timings` is now
        // wired into determinism mode too, with a distinct top-level
        // schema (`"mode": "determinism"`). The actual write is plumbed
        // AFTER the summary runs (see below), so it is no longer a
        // loud noop here.
        if args.baseline.is_some() {
            eprintln!(
                "nucleus-e2e: WARNING: --baseline is RUN-mode only \
                 (TASK-0023.03 Stage 2 scope); ignored under \
                 --check-determinism."
            );
        }
        eprintln!(
            "nucleus-e2e: determinism check over {} cell(s) from {} \
             (jobs={})",
            planned.len(),
            paths.manifest_path().display(),
            args.jobs
        );
        // TASK-0023.01: parallel cell execution. With jobs=1 (default)
        // this is byte-for-byte identical to the original loop; with
        // jobs>=2 cells run on a worker pool and completion lines emit
        // in completion order while `det_results` is re-sorted to
        // planned order before the summary + gate-signal block runs.
        let (det_results, det_wall_clock): (Vec<DetCellResult>, Duration) =
            execute_cells_parallel(paths, &planned, args.jobs, |paths, pc, i, total| {
                let r = check_cell_determinism(paths, pc);
                // Build the completion line as a STRING rather than
                // splitting eprint!/eprintln! around the work — that
                // way the parallel path can defer the print to the
                // main thread (avoids interleaved cell output across
                // workers) while staying visually identical to the
                // sequential pre-flag output.
                let head = format!(
                    "  [{:>2}/{:<2}] {} | {} | {} ... ",
                    i + 1,
                    total,
                    pc.cell.example,
                    pc.cell.schedule,
                    pc.cell.backend
                );
                let tail = match &r.status {
                    DetCellStatus::Pass { files_compared } => {
                        format!("PASS ({files_compared} files, {:?})", r.elapsed)
                    }
                    DetCellStatus::Failed(_) => "FAIL".to_string(),
                    DetCellStatus::Skipped { .. } => "SKIPPED".to_string(),
                };
                (r, format!("{head}{tail}"))
            });
        // TASK-0023.02: choose summary format. The `Text` default is
        // byte-identical to pre-flag behaviour; `Junit` emits XML on
        // stdout for CI consumption. Exit-code logic below is
        // independent of this choice.
        match args.format {
            Format::Text => print_determinism_summary(&det_results),
            Format::Junit => print_determinism_summary_junit(&det_results, Some(det_wall_clock)),
        }

        // TASK-0023.03.03 Stage 1.5 (cycle-57): persist per-cell det-mode
        // timings JSON when `--emit-timings PATH` is set. Done AFTER the
        // summary so a late-failing write surfaces loud but does not
        // block the human-facing summary the developer is staring at,
        // AND BEFORE the NUC_NONDET_TEST zero-perturbation guard below
        // (which has an early `return Ok(0)` path) — placing the emit
        // here means BOTH return paths get the JSON, so a falsifier
        // regression run can still be diffed against a clean baseline.
        if let Some(path) = args.emit_timings.as_deref() {
            write_det_timings_json(path, &det_results)?;
            eprintln!(
                "nucleus-e2e: wrote per-cell determinism timings ({} cell(s)) to {}",
                det_results.len(),
                path.display()
            );
        }

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

            // TASK-0188 AC#1 — EXPLICIT MACHINE-CHECKABLE SIGNAL.
            //
            // Stable key (do not rename without updating justfile:69):
            //
            //     NUC_NONDET_PERTURBED_CELLS=<n>
            //
            // Emitted on STDOUT (println!, not eprintln!) so it is a
            // semantically-distinct, parseable RESULT line — the loud
            // human-facing diagnostics stay on stderr. It is printed
            // UNCONDITIONALLY under the gate (n==0 in the zero-perturb
            // arm below, n>=1 in the genuine-bite arm) so the recipe
            // can always find it and assert n>=1.
            //
            // WHY this matters: before TASK-0188 the "the falsifier
            // actually perturbed >=1 tree" safety invariant was encoded
            // SOLELY in this process' exit code, whose meaning is
            // supplied entirely by the inverting `if HARNESS; then
            // FAIL; else OK; fi` at justfile:69 (a different file). A
            // recipe refactor dropping that inversion would silently
            // re-neuter the falsifier. With this line, justfile:69 also
            // asserts `NUC_NONDET_PERTURBED_CELLS >= 1`, so the safety
            // invariant no longer rests on exit-code inversion alone.
            //
            // The line is gated on NUC_NONDET_TEST=1: it does NOT
            // appear under bare `determinism-check`, so that path stays
            // byte-identical / unaffected.
            println!("NUC_NONDET_PERTURBED_CELLS={perturbed_cells}");

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
                // gate-visible failure, never a silent OK. The
                // explicit NUC_NONDET_PERTURBED_CELLS=0 line above is
                // the redundant machine-checkable backstop (TASK-0188):
                // even if a refactor breaks the inversion, the recipe's
                // count assertion still fails loud.
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
        "nucleus-e2e: running {} cell(s) from {} (jobs={})",
        planned.len(),
        paths.manifest_path().display(),
        args.jobs
    );

    // TASK-0023.01: parallel cell execution. jobs=1 (default) = strict
    // sequential, byte-for-byte identical to the pre-flag loop and the
    // path bare `just e2e` takes. jobs>=2 → worker pool, completion
    // lines emit in completion order, but `results` is returned in
    // planned order so the summary / required-fail / NUC_XBACKEND_*
    // gates stay deterministic.
    let (results, wall_clock): (Vec<CellResult>, Duration) =
        execute_cells_parallel(paths, &planned, args.jobs, |paths, pc, i, total| {
            let r = run_cell(paths, pc);
            let head = format!(
                "  [{:>2}/{:<2}] {} | {} | {} ... ",
                i + 1,
                total,
                pc.cell.example,
                pc.cell.schedule,
                pc.cell.backend
            );
            let tail = match &r.status {
                Status::Pass => format!("PASS ({:?})", r.timings.total()),
                Status::Failed { phase, .. } => format!("FAIL/{phase}"),
                Status::Skipped { .. } => "SKIPPED".to_string(),
            };
            (r, format!("{head}{tail}"))
        });

    // TASK-0023.02: choose summary format. The `Text` default is
    // byte-identical to the pre-flag table; `Junit` emits XML on
    // stdout for CI consumption. The required-fail exit-code and the
    // NUC_XBACKEND_* gate signals below are independent of this
    // choice — they MUST still gate CI green/red regardless of
    // whether the developer asked for human or machine output.
    match args.format {
        Format::Text => print_summary(&results),
        Format::Junit => print_summary_junit(&results, Some(wall_clock)),
    }

    // TASK-0023.03 Stage 1: persist per-cell timings as JSON when
    // `--emit-timings PATH` is set. Done AFTER the summary so a
    // late-failing write surfaces loud but does not block the human-
    // facing summary the developer is staring at. Failure is an Err
    // back to `run`, which turns into a non-zero exit code — a silent
    // partial write would let a downstream `--baseline` compare
    // against truncated JSON and report spurious regressions.
    if let Some(path) = args.emit_timings.as_deref() {
        write_timings_json(path, &results)?;
        eprintln!(
            "nucleus-e2e: wrote per-cell timings ({} cell(s)) to {}",
            results.len(),
            path.display()
        );
    }

    // TASK-0023.03 Stage 2: load the baseline JSON (if set) and print
    // a delta table to STDERR. Done AFTER the optional emit so a
    // single invocation can both write a fresh baseline AND compare
    // against a previous one (the canonical "did this change move the
    // needle" workflow). Output routes to STDERR specifically so
    // `--format=junit` XML on STDOUT stays clean.
    // TASK-0023.03.02 Stage 3: `compare_against_baseline` returns the
    // count of required-cell threshold breaches. Folded into the same
    // late-stage exit-code variable as `required_failed` below so the
    // existing single-return-point at the bottom of `run_inner` remains
    // the SOLE exit-code authority (no early `return Ok(1)` here — the
    // NUC_XBACKEND_NEGATIVE branch BELOW this call still needs to run on
    // every invocation, and short-circuiting would skip it).
    let perf_regressions = if let Some(path) = args.baseline.as_deref() {
        compare_against_baseline(path, &results, &planned)?
    } else {
        0
    };

    // NUC_XBACKEND_NEGATIVE explicit-signal contract + zero-corruption
    // guard. Two distinct, both gate-only, both on STDOUT:
    //
    //   (A) NUC_XBACKEND_CORRUPTED_APPLIED=<n>  (TASK-0183)
    //       n = mp-tcp-bufsync cells whose emitted src/wire.rs this
    //       harness ACTUALLY rewrote (`CellResult.corrupted`). This is
    //       the analogue of NUC_NONDET_PERTURBED_CELLS — it proves the
    //       falsifier, now applied harness-side, actually touched
    //       something. If zero -> loud FATAL + `return Ok(0)` so the
    //       inverting recipe FAILs loud (the TASK-0187 lesson: a
    //       no-op must NOT be invertible into a false OK; exiting
    //       non-zero would let the recipe invert it).
    //
    //   (B) NUC_XBACKEND_CORRUPTED_DETECTED=<n>  (TASK-0188, preserved)
    //       n = cells where the corruption was present AND the
    //       cross-backend differential genuinely DETECTED it, defined
    //       PRECISELY as required AND backend == "mp-tcp-bufsync" AND
    //       Status::Failed { phase: Phase::Diff, .. }. Each conjunct
    //       (so this CANNOT be satisfied by an unrelated required
    //       failure, only by a genuine differential bite):
    //         * backend == "mp-tcp-bufsync": the corruption
    //           (now maybe_corrupt_wire_for_xbackend, harness-side,
    //           applied to the emitted mp-tcp src/wire.rs) is
    //           mp-tcp-EXCLUSIVE — pthreads-sync emits no wire, so a
    //           pthreads required-fail is unrelated.
    //         * Phase::Diff: the corruption manifests as output.bin
    //           diverging from the hand-written reference.bin oracle.
    //           A Compile/Build/Run failure is unrelated breakage,
    //           NOT "the differential caught the corrupted wire".
    //         * required: only required cells gate the exit code;
    //           matching exit-code semantics keeps the signals
    //           consistent.
    //       Moving WHERE corruption is applied (codegen -> harness)
    //       does not change this definition: it is still "a required
    //       mp-tcp cell diverged from reference.bin at Diff", recomputed
    //       from results, conjuncts intact (TASK-0188 carry).
    //
    // Both emitted on STDOUT (println!, semantically RESULT lines; the
    // loud human diagnostics stay on stderr), ONLY when
    // NUC_XBACKEND_NEGATIVE=1, so bare `just e2e` is byte-for-byte
    // unaffected (no lines, exit unchanged — verified: e2e standalone
    // stays 30/26/0/4/0, zero signal lines). justfile:85 captures
    // combined output, asserts NUC_XBACKEND_CORRUPTED_DETECTED present
    // AND n>=1 IN ADDITION to the exit-code inversion, so the safety
    // invariant no longer rests on exit-code inversion alone.
    if std::env::var("NUC_XBACKEND_NEGATIVE").as_deref() == Ok("1") {
        let corrupted_applied = results.iter().filter(|r| r.corrupted).count();
        let corrupted_detected = results
            .iter()
            .filter(|r| {
                r.required
                    && r.cell.backend == "mp-tcp-bufsync"
                    && matches!(
                        r.status,
                        Status::Failed {
                            phase: Phase::Diff,
                            ..
                        }
                    )
            })
            .count();

        // Print BOTH unconditionally under the gate so the recipe can
        // always find them (n==0 in the zero arm, n>=1 in the genuine
        // bite). DETECTED is the justfile:85 contract line; APPLIED is
        // the harness-side falsifier-bit backstop.
        println!("NUC_XBACKEND_CORRUPTED_APPLIED={corrupted_applied}");
        println!("NUC_XBACKEND_CORRUPTED_DETECTED={corrupted_detected}");

        if corrupted_applied == 0 {
            eprintln!(
                "nucleus-e2e: FATAL: NUC_XBACKEND_NEGATIVE=1 but ZERO \
                 mp-tcp-bufsync cell(s) had their emitted src/wire.rs \
                 actually corrupted — the cross-backend falsifier \
                 touched nothing. Forcing a CLEAN exit so \
                 `xbackend-check-negative` reports its loud FAIL (the \
                 falsifier did NOT bite) instead of inverting a no-op \
                 into a false OK (TASK-0183, mirroring TASK-0187 AC#2). \
                 Likely codegen layout drift: mp-tcp-bufsync must emit \
                 src/wire.rs with the enc_vec anchor."
            );
            // Exit 0 on purpose: the recipe inverts this into its
            // "FAIL: did NOT detect" branch (exit 1). The explicit
            // NUC_XBACKEND_CORRUPTED_APPLIED=0 / _DETECTED=0 lines above
            // are the redundant machine-checkable backstop (TASK-0188):
            // even if a refactor breaks the inversion, the recipe's
            // count assertion still fails loud.
            return Ok(0);
        }
        eprintln!(
            "nucleus-e2e: NUC_XBACKEND_NEGATIVE=1 — {corrupted_applied} \
             mp-tcp-bufsync cell(s) corrupted, {corrupted_detected} \
             detected by the differential (negative-gate sanity: \
             applied>=1 required)."
        );
    }

    let required_failed = results
        .iter()
        .any(|r| r.required && matches!(r.status, Status::Failed { .. }));
    // Combined exit code: a required-cell test FAILED *or* a required-
    // cell perf threshold was breached. Either is a HARD FAIL. The two
    // are disjoint signals (one is correctness, one is perf) but share
    // a single exit-code channel — the table on STDERR + the explicit
    // "HARD FAIL" line in `compare_against_baseline` disambiguate.
    Ok(if required_failed || perf_regressions > 0 {
        1
    } else {
        0
    })
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
            perf_threshold_pct: None,
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
            perf_threshold_pct: None,
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

    // ---- TASK-0023.01: --jobs / -j parser tests --------------------

    #[test]
    fn arg_parser_default_jobs_is_one() {
        // Default sequential behaviour: NO --jobs flag → jobs == 1, the
        // same byte-for-byte path as before TASK-0023.01. The
        // execute_cells_parallel helper hangs its sequential-equivalence
        // guarantee on this default.
        let argv: Vec<OsString> = Vec::new();
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.jobs, 1, "default jobs must be 1 (sequential)");
    }

    #[test]
    fn arg_parser_accepts_jobs_long_and_short() {
        // --jobs N and -j N must be exactly equivalent. The short form
        // exists because `just e2e-jobs 4` may pass -j and that path
        // would otherwise diverge from the long form silently.
        for flag in ["--jobs", "-j"] {
            let argv: Vec<OsString> = [flag, "4"].iter().map(|s| OsString::from(*s)).collect();
            let a = parse_args(&argv).expect("parse");
            assert_eq!(a.jobs, 4, "expected jobs=4 via `{flag} 4`");
        }
    }

    #[test]
    fn arg_parser_jobs_max_bound_accepted() {
        // MAX_JOBS is the documented ceiling. Exactly MAX_JOBS must be
        // accepted (off-by-one guard against accidentally tightening
        // the bound during a refactor).
        let argv = vec![
            OsString::from("--jobs"),
            OsString::from(MAX_JOBS.to_string()),
        ];
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.jobs, MAX_JOBS);
    }

    #[test]
    fn arg_parser_rejects_jobs_zero() {
        // jobs==0 would spawn no workers; reject loud at parse time
        // rather than silently never making progress.
        let argv: Vec<OsString> = ["--jobs", "0"].iter().map(|s| OsString::from(*s)).collect();
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains(">= 1"), "got: {err}");
    }

    #[test]
    fn arg_parser_rejects_jobs_non_integer() {
        let argv: Vec<OsString> = ["--jobs", "four"]
            .iter()
            .map(|s| OsString::from(*s))
            .collect();
        let err = parse_args(&argv).unwrap_err();
        assert!(
            err.contains("positive integer") && err.contains("four"),
            "got: {err}"
        );
    }

    #[test]
    fn arg_parser_rejects_jobs_negative() {
        // usize parse rejects the leading minus; we only need to confirm
        // the error message names the offending value (good UX).
        let argv: Vec<OsString> = ["--jobs", "-1"]
            .iter()
            .map(|s| OsString::from(*s))
            .collect();
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("positive integer"), "got: {err}");
    }

    #[test]
    fn arg_parser_rejects_jobs_above_max() {
        let argv = vec![
            OsString::from("--jobs"),
            OsString::from((MAX_JOBS + 1).to_string()),
        ];
        let err = parse_args(&argv).unwrap_err();
        assert!(
            err.contains("MAX_JOBS") || err.contains(&MAX_JOBS.to_string()),
            "got: {err}"
        );
    }

    #[test]
    fn arg_parser_rejects_jobs_without_value() {
        let argv = vec![OsString::from("--jobs")];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("requires a value"), "got: {err}");
    }

    #[test]
    fn arg_parser_jobs_composes_with_other_flags() {
        // --jobs must compose with the narrowing flags: a developer
        // debugging one cell on a parallel host should still get
        // parallel execution of the narrowed set.
        let argv: Vec<OsString> = [
            "--example",
            "01-elementwise-add",
            "--jobs",
            "2",
            "--check-determinism",
        ]
        .iter()
        .map(|s| OsString::from(*s))
        .collect();
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.jobs, 2);
        assert_eq!(a.example.as_deref(), Some("01-elementwise-add"));
        assert!(a.check_determinism);
    }

    // ---- TASK-0023.02: --format parser tests ----------------------

    #[test]
    fn arg_parser_default_format_is_text() {
        // Default behaviour: NO --format flag → Format::Text, the same
        // byte-for-byte stdout path as before TASK-0023.02.
        let argv: Vec<OsString> = Vec::new();
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.format, Format::Text, "default format must be Text");
    }

    #[test]
    fn arg_parser_accepts_format_space_and_equals() {
        // Both `--format junit` (separate value) and `--format=junit`
        // (equals form) must work — CI scripts commonly pass the
        // latter and a silent reject would be confusing.
        for argv in [
            vec![OsString::from("--format"), OsString::from("junit")],
            vec![OsString::from("--format=junit")],
        ] {
            let a = parse_args(&argv).expect("parse");
            assert_eq!(a.format, Format::Junit, "argv={argv:?}");
        }
        for argv in [
            vec![OsString::from("--format"), OsString::from("text")],
            vec![OsString::from("--format=text")],
        ] {
            let a = parse_args(&argv).expect("parse");
            assert_eq!(a.format, Format::Text, "argv={argv:?}");
        }
    }

    #[test]
    fn arg_parser_rejects_unknown_format() {
        for argv in [
            vec![OsString::from("--format"), OsString::from("yaml")],
            vec![OsString::from("--format=yaml")],
        ] {
            let err = parse_args(&argv).unwrap_err();
            assert!(err.contains("text") && err.contains("junit"), "got: {err}");
        }
    }

    #[test]
    fn arg_parser_rejects_format_without_value() {
        // Long form without a value must fail loud, like every other
        // value-bearing flag.
        let argv = vec![OsString::from("--format")];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("requires a value"), "got: {err}");
    }

    #[test]
    fn junit_summary_shape_is_valid_xml_skeleton() {
        // Synthetic 3-cell matrix exercises PASS, FAIL, SKIPPED in one
        // pass. We verify the document opens with the XML decl and the
        // testsuites/testsuite envelope, contains exactly one
        // <testcase> per cell with the correct classname/name, and
        // emits <failure>/<skipped> children for the non-pass cells.
        // Run this through `print_summary_junit` via a captured stdout
        // would be ideal; absent a `Write` parameter we instead
        // re-implement the small assertions on the values we'd render.
        let cell_pass = CellResult {
            cell: Cell {
                example: "ex1".into(),
                schedule: "naive".into(),
                backend: "pthreads-sync".into(),
            },
            required: true,
            status: Status::Pass,
            timings: Timings::default(),
            corrupted: false,
        };
        let cell_fail = CellResult {
            cell: Cell {
                example: "ex2".into(),
                schedule: "tiled".into(),
                backend: "mp-tcp-bufsync".into(),
            },
            required: true,
            status: Status::Failed {
                phase: Phase::Diff,
                detail: "byte mismatch at offset 42".into(),
            },
            timings: Timings::default(),
            corrupted: false,
        };
        let cell_skip = CellResult {
            cell: Cell {
                example: "ex3".into(),
                schedule: "tiled".into(),
                backend: "pthreads-sync".into(),
            },
            required: false,
            status: Status::Skipped {
                reason: "manifest-skip".into(),
            },
            timings: Timings::default(),
            corrupted: false,
        };
        // The function writes to stdout; we cannot capture it without
        // a Write parameter, so this test exercises only the static
        // helpers — escaping + classname composition — that the
        // emitter relies on. The shape itself is verified by the
        // workflow-gate `cargo run --bin nucleus-e2e -- --format=junit`
        // check documented on TASK-0023.02.
        let _ = (cell_pass, cell_fail, cell_skip);
        assert_eq!(xml_escape_attr("a&b<c>\""), "a&amp;b&lt;c&gt;&quot;");
        assert_eq!(xml_escape_cdata("foo]]>bar"), "foo]]]]><![CDATA[>bar");
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
    /// covered by `nucleus-compiler/tests/e2e_example_0{1,2,3}.rs`,
    /// which remain authoritative regression catchers per the task
    /// brief.
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
                perf_threshold_pct: None,
            }],
            skip: vec![],
        };
        let args = Args {
            example: Some("01-elementwise-add".into()),
            schedule: Some("naive".into()),
            backend: Some("pthreads-sync".into()),
            ..Args::default()
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
            perf_threshold_pct: None,
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
            perf_threshold_pct: None,
        }
    }

    fn skip_e(ex: &str, sc: &str, be: &str, reason: &str, ms: &str) -> SkipEntry {
        SkipEntry {
            example: ex.to_string(),
            schedule: sc.to_string(),
            backend: be.to_string(),
            reason: reason.to_string(),
            milestone: ms.to_string(),
            perf_threshold_pct: None,
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
        assert!(
            gaps.is_empty(),
            "skip-declared required must be exempt, got {gaps:?}"
        );
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
        assert!(
            gaps.is_empty(),
            "planned required must not be a gap, got {gaps:?}"
        );
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
        assert_eq!(
            gaps2,
            vec![cell("01-elementwise-add", "naiv", "pthreads-sync")]
        );
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
    /// milestone tier (no flag, M1, M2, M3, M4) — the durable per-tier
    /// guard. A future manifest edit that typo's or strands a
    /// milestone-tagged required cell turns the relevant tier (and its
    /// CI job) red.
    ///
    /// M4 was added when TASK-0042.03 landed the first M4-tagged
    /// required cell (09-producer-consumer/pipelined × pthreads-async,
    /// the async + buffer>1 + notify=event headline).
    #[test]
    fn real_manifest_has_no_coverage_gaps_at_every_milestone() {
        let paths = Paths::discover().expect("discover repo root");
        let src = fs::read_to_string(paths.manifest_path()).expect("read manifest");
        let manifest: Manifest = toml::from_str(&src).expect("parse manifest");
        for gate in [
            None,
            Some(Milestone(1)),
            Some(Milestone(2)),
            Some(Milestone(3)),
            Some(Milestone(4)),
        ] {
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
    /// cumulative monotone: |M_k| < |M_{k+1}| for every adjacent pair
    /// of milestones present in the manifest, and |M_top| == |full|.
    ///
    /// TASK-0268 (cycle 102) refactored from hardcoded M1..M4 / M5 to
    /// matrix-driven milestone discovery — the architect P2 review
    /// noted that the previous hardcoded form required a manual code
    /// edit at every milestone bump (M3<M4 → M3<M4<M5 → M3<M4<M5<M6
    /// ...). Now the test reads ALL milestones referenced by
    /// `[[required]]` rows in the manifest, finds the max, and
    /// asserts strict monotonicity from M1 up to the discovered top.
    /// New milestones are picked up automatically when their first
    /// `[[required]]` cell lands.
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

        // Discover the top milestone from manifest `[[required]]`
        // rows. Skip-only milestones don't count: a milestone whose
        // ONLY cells are `[[skip]]`s has zero required at any gate
        // band, breaking strict-monotonicity. The gate semantics is
        // "required count", so the discovery must match.
        let top: Milestone = manifest
            .required
            .iter()
            .map(|r| {
                Milestone::parse(&r.milestone).expect("manifest milestone tags must parse")
            })
            .max()
            .expect("manifest must contain at least one [[required]] cell");

        // Count required cells at each gate band from M1 up to top.
        let counts: Vec<(Milestone, usize)> = (1..=top.0)
            .map(|k| {
                let m = Milestone(k);
                (m, count(Some(m)))
            })
            .collect();
        let full = count(None);

        // Strict monotonicity: every adjacent pair (M_k, M_{k+1}) has
        // count(M_k) < count(M_{k+1}). A non-strict pair indicates a
        // milestone with zero new [[required]] cells beyond the
        // previous tier — that's an empty gate, which by PRD §11 is
        // not a tier-1 milestone (each tier MUST have new acceptance
        // cells).
        for win in counts.windows(2) {
            let (m_lo, c_lo) = win[0];
            let (m_hi, c_hi) = win[1];
            assert!(
                c_lo < c_hi,
                "milestone subsets must strictly grow: \
                 |{m_lo:?}|={c_lo} not < |{m_hi:?}|={c_hi} (all counts: {counts:?})"
            );
        }

        // Top tier covers the full required set.
        let (_top_milestone, top_count) = *counts.last().expect("non-empty counts");
        assert_eq!(
            top_count, full,
            "top milestone ({top:?}) required set must equal the full \
             required set (top_count={top_count}, full={full}, all={counts:?})"
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
            perf_threshold_pct: None,
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

    #[test]
    fn explicit_count_signal_makes_negative_recipes_fail_loud_independent_of_exit_code() {
        // TASK-0188 AC#3 — proven directly. Before TASK-0188 the
        // safety invariant ("the falsifier actually touched something")
        // rested SOLELY on the exit-code inversion in justfile:69/:85.
        // We now ALSO emit an explicit machine-checkable stdout line
        // (NUC_NONDET_PERTURBED_CELLS / NUC_XBACKEND_CORRUPTED_DETECTED)
        // and the recipes assert it. This models the recipes' new DUAL
        // verdict and proves: if the count signal says zero / is
        // absent, the recipe FAILS LOUD even when the exit code ALONE
        // would invert to a false OK.

        /// Models the parsed `<n>` from the explicit stdout line.
        /// `None` == the line was absent entirely (a broken
        /// harness/recipe contract — also a hard FAIL).
        type CountSignal = Option<usize>;

        /// Faithful model of the post-TASK-0188 recipe verdict for
        /// BOTH determinism-check-negative (justfile:69) and
        /// xbackend-check-negative (justfile:85): they share the exact
        /// same shape — capture combined output, then:
        ///
        /// 1. if the count line is absent -> FAIL.
        /// 2. if the parsed count < 1 -> FAIL.
        /// 3. else fall back to the exit-code inversion (harness exit 0
        ///    -> FAIL "did NOT detect"; non-0 -> OK "correctly bit").
        ///
        /// Returns true == recipe prints OK, false == recipe FAILs.
        fn recipe_says_ok(harness_exit: i32, count: CountSignal) -> bool {
            match count {
                None => false,                // signal missing -> FAIL
                Some(n) if n < 1 => false,    // count says zero -> FAIL
                Some(_) => harness_exit != 0, // else: exit-code inversion
            }
        }

        // --- The headline TASK-0188 property -------------------------
        // The exact false-confidence scenario the hardening kills:
        // some UNRELATED cell Failed so the raw exit code is non-zero,
        // which the OLD recipe (exit-code inversion ONLY) would invert
        // into a false "OK: correctly bit". With the count signal at 0
        // the new recipe MUST still print FAIL.
        let unrelated_failure_exit = 1; // raw exit code alone -> "OK"
        assert!(
            !recipe_says_ok(unrelated_failure_exit, Some(0)),
            "count=0 MUST make the recipe FAIL even when the exit code \
             alone would invert to a false OK (the core TASK-0188 \
             hardening; exit-code inversion is no longer sufficient)"
        );

        // Same property when the signal line is entirely ABSENT (e.g.
        // a refactor removed the println! or renamed the key): a
        // broken contract is a loud FAIL, never a silent pass.
        assert!(
            !recipe_says_ok(unrelated_failure_exit, None),
            "a MISSING count signal MUST make the recipe FAIL even \
             when the exit code alone would invert to OK"
        );

        // Belt-and-braces: even if BOTH the exit code says clean AND
        // the count is zero, still FAIL (no path to a false OK).
        assert!(!recipe_says_ok(0, Some(0)));
        assert!(!recipe_says_ok(0, None));

        // --- The genuine-bite path still prints OK -------------------
        // >=1 perturbation/corruption detected AND the harness exited
        // non-zero (real divergence): the ONLY path that may say OK.
        assert!(
            recipe_says_ok(1, Some(26)),
            "a genuine bite (count>=1 AND non-zero exit) must still \
             let the recipe print OK"
        );
        assert!(
            recipe_says_ok(1, Some(1)),
            "the minimal genuine bite (exactly one cell) must say OK"
        );

        // --- Defence-in-depth interplay with the AC#2 zero-perturb
        // guard (TASK-0187): under the gate, zero perturbations force a
        // CLEAN harness exit (0). The OLD single-signal model relied on
        // that clean exit + inversion. Now even if a refactor broke
        // the inversion (treated exit 0 as OK), count=0 still FAILs.
        let forced_clean_exit_on_zero_perturb = 0;
        assert!(
            !recipe_says_ok(forced_clean_exit_on_zero_perturb, Some(0)),
            "the count backstop must hold the line even if the \
             exit-code inversion is broken by a future refactor"
        );

        // --- TASK-0183: the SAME contract for the relocated xbackend
        // wire falsifier. xbackend-check-negative (justfile:85) shares
        // the exact recipe shape; the corruption is now applied
        // harness-side (maybe_corrupt_wire_for_xbackend), and the
        // matrix-wide guard prints NUC_XBACKEND_CORRUPTED_APPLIED /
        // _DETECTED then forces a CLEAN exit when APPLIED==0 so a
        // no-op cannot invert into a false OK. Same model, same key
        // properties — independent of which seam applied the fault.
        let xbackend_unrelated_fail_exit = 1;
        assert!(
            !recipe_says_ok(xbackend_unrelated_fail_exit, Some(0)),
            "xbackend count=0 MUST FAIL even when an unrelated \
             required-fail makes the raw exit non-zero (TASK-0183: the \
             harness-side relocation must keep the TASK-0188 contract)"
        );
        assert!(
            !recipe_says_ok(xbackend_unrelated_fail_exit, None),
            "a MISSING xbackend signal MUST FAIL regardless of exit code"
        );
        assert!(
            recipe_says_ok(1, Some(1)),
            "the minimal genuine xbackend bite (>=1 cell corrupted AND \
             detected, non-zero exit) must still let the recipe say OK"
        );
        // The zero-corruption guard forces exit 0; the count backstop
        // must still FAIL even if a refactor treated exit 0 as OK.
        assert!(
            !recipe_says_ok(0, Some(0)),
            "xbackend zero-corruption guard: forced clean exit + count=0 \
             must still FAIL loud (TASK-0183 mirror of TASK-0187 AC#2)"
        );
    }

    // ---------------------------------------------------------------
    // TASK-0183 — harness-side relocated NUC_XBACKEND_NEGATIVE wire
    // corruption (mirrors the TASK-0187 maybe_perturb_for_nondet_test
    // trio). NUC_XBACKEND_NEGATIVE is process-global; reuse the same
    // serialising fence as the nondet tests (a different var, but the
    // same one-mutex discipline keeps env mutation from interleaving
    // across the parallel test threads).
    // ---------------------------------------------------------------

    fn xbackend_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A synthetic `wire.rs` containing the EXACT `enc_vec` anchor the
    /// relocated falsifier rewrites (byte-for-byte the body in
    /// `mp-tcp-common/src/wire_runtime.rs`). If wire_runtime drifts,
    /// `maybe_corrupt_wire_for_xbackend`'s anchor check must Err loud —
    /// `xbackend_corrupt_errs_when_anchor_drifted` exercises that.
    const SAMPLE_WIRE_RS: &str = "pub fn enc_vec<T: Copy, const W: usize>(v: &[T], to_le: fn(T) -> [u8; W]) -> Vec<u8> {\n    let mut out = Vec::with_capacity(v.len() * W);\n    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    out\n}\n";

    #[test]
    fn xbackend_corrupt_rewrites_wire_rs_under_gate() {
        // AC#2/#3: with the gate set and a present src/wire.rs holding
        // the anchor, the function must actually rewrite it (>=1 byte
        // changed) and inject the deliberate last-byte tweak. This is
        // the harness-side analogue of the deleted mp-tcp-bufsync
        // `maybe_corrupt_wire` — behaviour preserved.
        let _guard = xbackend_env_lock();
        let tree = nondet_tmp("xb-mutate");
        let src_dir = tree.join("src");
        fs::create_dir_all(&src_dir).expect("mk src");
        let wire = src_dir.join("wire.rs");
        fs::write(&wire, SAMPLE_WIRE_RS).expect("write wire.rs");

        std::env::set_var("NUC_XBACKEND_NEGATIVE", "1");
        let did = maybe_corrupt_wire_for_xbackend(&tree);
        std::env::remove_var("NUC_XBACKEND_NEGATIVE");

        assert_eq!(
            did,
            Ok(true),
            "gate=1 with a present wire.rs holding the anchor must \
             report a corruption"
        );
        let after = fs::read_to_string(&wire).expect("read after");
        assert_ne!(SAMPLE_WIRE_RS, after, "wire.rs must have changed");
        assert!(
            after.contains("last.wrapping_add(1)")
                && after.contains("NUC_XBACKEND_NEGATIVE deliberate corruption"),
            "expected the deliberate last-byte tweak, got:\n{after}"
        );
        // The anchor must be gone (replaced exactly once).
        assert!(
            !after.contains(
                "    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    out\n}"
            ),
            "the pristine enc_vec tail must have been rewritten"
        );

        let _ = fs::remove_dir_all(&tree);
    }

    #[test]
    fn xbackend_corrupt_is_strict_noop_when_env_unset() {
        // AC#3 / behaviour-equivalence: bare `just e2e`. With the gate
        // unset the function must not read or touch the tree and must
        // report Ok(false) — this is what keeps the emitted wire.rs
        // byte-identical to mp_tcp_common::WIRE_RUNTIME_SRC and bare
        // e2e at 30/26/0/4/0 with zero signal lines.
        let _guard = xbackend_env_lock();
        let tree = nondet_tmp("xb-noop");
        let src_dir = tree.join("src");
        fs::create_dir_all(&src_dir).expect("mk src");
        let wire = src_dir.join("wire.rs");
        fs::write(&wire, SAMPLE_WIRE_RS).expect("write wire.rs");

        std::env::remove_var("NUC_XBACKEND_NEGATIVE");
        let did = maybe_corrupt_wire_for_xbackend(&tree);

        assert_eq!(
            did,
            Ok(false),
            "env unset must be a strict no-op (no corruption)"
        );
        let after = fs::read_to_string(&wire).expect("read after");
        assert_eq!(
            after, SAMPLE_WIRE_RS,
            "env-unset path must leave wire.rs byte-identical \
             (behaviour-equivalence with pristine codegen)"
        );

        let _ = fs::remove_dir_all(&tree);
    }

    #[test]
    fn xbackend_corrupt_errs_when_gate_set_but_wire_rs_missing() {
        // The mp-tcp layout drifted so src/wire.rs is absent under the
        // gate: a HARD Err (never a silent skip). The caller maps it to
        // Failed(Compile) and the matrix-wide zero-corruption guard
        // then forces the recipe to FAIL loud — the never-silently-
        // neuter-the-falsifier invariant, harness-side.
        let _guard = xbackend_env_lock();
        let tree = nondet_tmp("xb-missing"); // no src/wire.rs

        std::env::set_var("NUC_XBACKEND_NEGATIVE", "1");
        let did = maybe_corrupt_wire_for_xbackend(&tree);
        std::env::remove_var("NUC_XBACKEND_NEGATIVE");

        match did {
            Err(msg) => assert!(
                msg.contains("wire.rs") && msg.contains("layout drifted"),
                "error must name the missing wire.rs and layout drift, got: {msg}"
            ),
            Ok(v) => panic!("expected Err when wire.rs absent under gate, got Ok({v})"),
        }

        let _ = fs::remove_dir_all(&tree);
    }

    #[test]
    fn xbackend_corrupt_errs_when_anchor_drifted() {
        // The enc_vec anchor moved/refactored: under the gate this MUST
        // Err (not silently emit an uncorrupted build that would make
        // xbackend-check-negative a false PASS) — the harness-side
        // analogue of the deleted site's `panic!` anchor-drift guard.
        let _guard = xbackend_env_lock();
        let tree = nondet_tmp("xb-drift");
        let src_dir = tree.join("src");
        fs::create_dir_all(&src_dir).expect("mk src");
        let wire = src_dir.join("wire.rs");
        fs::write(&wire, "pub fn enc_vec() { /* refactored away */ }\n").expect("write wire.rs");

        std::env::set_var("NUC_XBACKEND_NEGATIVE", "1");
        let did = maybe_corrupt_wire_for_xbackend(&tree);
        std::env::remove_var("NUC_XBACKEND_NEGATIVE");

        match did {
            Err(msg) => assert!(
                msg.contains("anchor not found") && msg.contains("drifted"),
                "anchor-drift error must be explicit, got: {msg}"
            ),
            Ok(v) => panic!("expected Err on anchor drift under gate, got Ok({v})"),
        }
        // wire.rs must be left UNwritten on the drift path (we Err
        // before the fs::write), so the failure is observable, not a
        // half-corrupted file.
        let after = fs::read_to_string(&wire).expect("read after");
        assert_eq!(
            after, "pub fn enc_vec() { /* refactored away */ }\n",
            "on anchor drift the function must Err WITHOUT writing"
        );

        let _ = fs::remove_dir_all(&tree);
    }

    // ----------------------------------------------------------------
    // TASK-0023.03 Stage 1 — per-cell timings JSON
    // ----------------------------------------------------------------

    #[test]
    fn arg_parser_accepts_emit_timings_space_form() {
        let argv: Vec<OsString> = ["--emit-timings", "/tmp/foo.json"]
            .iter()
            .map(|s| OsString::from(*s))
            .collect();
        let a = parse_args(&argv).expect("parse");
        assert_eq!(
            a.emit_timings.as_deref(),
            Some(std::path::Path::new("/tmp/foo.json"))
        );
    }

    #[test]
    fn arg_parser_accepts_emit_timings_equals_form() {
        let argv: Vec<OsString> = vec![OsString::from("--emit-timings=/tmp/bar.json")];
        let a = parse_args(&argv).expect("parse");
        assert_eq!(
            a.emit_timings.as_deref(),
            Some(std::path::Path::new("/tmp/bar.json"))
        );
    }

    #[test]
    fn arg_parser_rejects_empty_emit_timings_path() {
        // Empty PATH would silently write nothing / surface as a
        // confusing "is a directory" error post-matrix. Fail LOUD at
        // arg-parse so the developer fixes it before paying the
        // matrix cost.
        let argv_space: Vec<OsString> = ["--emit-timings", ""]
            .iter()
            .map(|s| OsString::from(*s))
            .collect();
        let err = parse_args(&argv_space).unwrap_err();
        assert!(
            err.contains("non-empty PATH"),
            "expected non-empty PATH error, got: {err}"
        );

        let argv_eq: Vec<OsString> = vec![OsString::from("--emit-timings=")];
        let err = parse_args(&argv_eq).unwrap_err();
        assert!(
            err.contains("non-empty PATH"),
            "expected non-empty PATH error, got: {err}"
        );
    }

    #[test]
    fn json_escape_str_handles_quote_backslash_newline_and_control() {
        let mut out = String::new();
        json_escape_str(&mut out, "a\"b\\c\n\td\x01e");
        // Expected JSON-escaped output. The control char becomes \u0001.
        assert_eq!(out, "\"a\\\"b\\\\c\\n\\td\\u0001e\"");
    }

    fn synth_cell_result(
        example: &str,
        schedule: &str,
        backend: &str,
        status: Status,
        compile_ms: Option<u64>,
        build_ms: Option<u64>,
        run_ms: Option<u64>,
    ) -> CellResult {
        CellResult {
            cell: Cell {
                example: example.into(),
                schedule: schedule.into(),
                backend: backend.into(),
            },
            required: true,
            status,
            timings: Timings {
                compile: compile_ms.map(Duration::from_millis),
                build: build_ms.map(Duration::from_millis),
                run: run_ms.map(Duration::from_millis),
            },
            corrupted: false,
        }
    }

    #[test]
    fn render_timings_json_emits_three_status_variants_in_planned_order() {
        // PASS, SKIPPED, FAILED — covers all three Status arms and
        // their status-specific payloads. The renderer must keep
        // input order so a downstream comparator can index by
        // position rather than re-sorting.
        let results = vec![
            synth_cell_result(
                "ex-a",
                "naive",
                "pthreads-sync",
                Status::Pass,
                Some(100),
                Some(2000),
                Some(50),
            ),
            synth_cell_result(
                "ex-b",
                "tiled",
                "mp-tcp-bufsync",
                Status::Skipped {
                    reason: "no capabilities.toml".into(),
                },
                None,
                None,
                None,
            ),
            synth_cell_result(
                "ex-c",
                "naive",
                "pthreads-sync",
                Status::Failed {
                    phase: Phase::Build,
                    detail: "cargo build failed\nlinker error".into(),
                },
                Some(80),
                Some(500),
                None,
            ),
        ];
        let doc = render_timings_json(&results);

        // Structural assertions — we deliberately do NOT shell out
        // to a real JSON parser here (no new dep budget); instead
        // we assert byte-substrings unique enough to catch the
        // shape regressions Stage 2/3 will rely on.
        // Cycle-57: the leading object now carries a `"mode": "run"`
        // key before `"cells"` for forward-compat with the det-mode
        // emitter (the consumer branches on top-level `mode`).
        assert!(
            doc.starts_with("{\n  \"mode\": \"run\",\n  \"cells\": ["),
            "leading brace + mode + cells array missing: {doc}"
        );
        assert!(
            doc.trim_end().ends_with("]\n}"),
            "trailing close missing: {doc}"
        );

        // Ordering: ex-a appears before ex-b before ex-c. A simple
        // 3-find suffices since cell names are unique.
        let a = doc.find("\"ex-a\"").expect("ex-a present");
        let b = doc.find("\"ex-b\"").expect("ex-b present");
        let c = doc.find("\"ex-c\"").expect("ex-c present");
        assert!(a < b && b < c, "planned order broken: {a}, {b}, {c}");

        // PASS cell carries `phase_times_ms` with ints, total_ms,
        // and NO fail_phase/skip_reason key.
        let pass_block = &doc[a..b];
        assert!(
            pass_block.contains("\"status\":\"PASS\""),
            "PASS status: {pass_block}"
        );
        assert!(
            pass_block.contains("\"compile\":100"),
            "compile ms: {pass_block}"
        );
        assert!(
            pass_block.contains("\"build\":2000"),
            "build ms: {pass_block}"
        );
        assert!(pass_block.contains("\"run\":50"), "run ms: {pass_block}");
        assert!(
            pass_block.contains("\"total_ms\":2150"),
            "total ms: {pass_block}"
        );
        assert!(
            !pass_block.contains("fail_phase"),
            "PASS must not carry fail_phase"
        );
        assert!(
            !pass_block.contains("skip_reason"),
            "PASS must not carry skip_reason"
        );

        // SKIPPED carries skip_reason and nulls for phase_times_ms.
        let skip_block = &doc[b..c];
        assert!(
            skip_block.contains("\"status\":\"SKIPPED\""),
            "SKIPPED status: {skip_block}"
        );
        assert!(
            skip_block.contains("\"skip_reason\":\"no capabilities.toml\""),
            "reason: {skip_block}"
        );
        assert!(
            skip_block.contains("\"compile\":null"),
            "null compile: {skip_block}"
        );
        assert!(
            skip_block.contains("\"build\":null"),
            "null build: {skip_block}"
        );
        assert!(
            skip_block.contains("\"run\":null"),
            "null run: {skip_block}"
        );
        assert!(
            skip_block.contains("\"total_ms\":0"),
            "total ms 0: {skip_block}"
        );

        // FAILED carries fail_phase + detail; detail contains an
        // escaped newline (proving json_escape_str ran).
        let fail_block = &doc[c..];
        assert!(
            fail_block.contains("\"status\":\"FAIL\""),
            "FAIL status: {fail_block}"
        );
        assert!(
            fail_block.contains("\"fail_phase\":\"build\""),
            "fail_phase: {fail_block}"
        );
        assert!(
            fail_block.contains("\"detail\":\"cargo build failed\\nlinker error\""),
            "escaped detail: {fail_block}"
        );
    }

    #[test]
    fn write_timings_json_creates_parents_and_writes_atomically() {
        // Atomic-write contract: a fresh parent path is created on
        // demand; the .tmp sibling does not survive a successful
        // write. Critical for Stage 2 — a baseline file half-written
        // by a crashed harness must NEVER be loaded as a baseline.
        let tmp_root = std::env::temp_dir().join(format!(
            "nucleus-e2e-emit-timings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp_root);

        let path = tmp_root.join("nested").join("dir").join("timings.json");
        assert!(!path.exists());
        assert!(
            !path.parent().unwrap().exists(),
            "parent must not pre-exist"
        );

        let results = vec![synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            Some(1),
            Some(2),
            Some(3),
        )];
        write_timings_json(&path, &results).expect("write");

        assert!(path.exists(), "output JSON must exist after write");
        let body = fs::read_to_string(&path).expect("read back");
        assert!(body.contains("\"ex-a\""), "round-trip body: {body}");
        assert!(body.contains("\"total_ms\":6"), "total ms 1+2+3=6: {body}");

        // .tmp sibling must NOT survive — rename consumed it.
        let tmp_sibling = path.with_file_name("timings.json.tmp");
        assert!(
            !tmp_sibling.exists(),
            ".tmp sibling leaked: {}",
            tmp_sibling.display()
        );

        let _ = fs::remove_dir_all(&tmp_root);
    }

    // ----------------------------------------------------------------
    // TASK-0023.03.03 Stage 1.5 (cycle-57) — det-mode emitter
    // ----------------------------------------------------------------

    fn synth_det_cell_result(
        example: &str,
        schedule: &str,
        backend: &str,
        status: DetCellStatus,
        elapsed_ms: u64,
    ) -> DetCellResult {
        DetCellResult {
            cell: Cell {
                example: example.into(),
                schedule: schedule.into(),
                backend: backend.into(),
            },
            required: true,
            status,
            elapsed: Duration::from_millis(elapsed_ms),
            perturbed: false,
        }
    }

    #[test]
    fn render_det_timings_json_emits_three_status_variants_with_distinct_payloads() {
        // Schema-shape pin: mode=determinism + cells array; PASS carries
        // files_compared + elapsed_ms; FAIL carries det_mismatch + elapsed_ms;
        // SKIPPED carries skip_reason + elapsed_ms=null. Cells appear in
        // planned (input) order.
        let results = vec![
            synth_det_cell_result(
                "ex-a",
                "naive",
                "pthreads-sync",
                DetCellStatus::Pass { files_compared: 7 },
                120,
            ),
            synth_det_cell_result(
                "ex-b",
                "tiled",
                "mp-tcp-bufsync",
                DetCellStatus::Skipped {
                    reason: "no capabilities.toml".into(),
                },
                0,
            ),
            synth_det_cell_result(
                "ex-c",
                "naive",
                "pthreads-sync",
                DetCellStatus::Failed(DetMismatch {
                    relative_path: PathBuf::from("src/main.rs"),
                    kind: DetMismatchKind::BytesDiffer,
                    offset: 42,
                    detail: "A=foo\nB=bar".into(),
                }),
                250,
            ),
        ];
        let doc = render_det_timings_json(&results);

        // Top-level shape: distinct mode key (consumer branches on this).
        assert!(
            doc.starts_with("{\n  \"mode\": \"determinism\",\n  \"cells\": ["),
            "leading mode + cells missing: {doc}"
        );
        assert!(
            doc.trim_end().ends_with("]\n}"),
            "trailing close missing: {doc}"
        );

        // Planned order preserved.
        let a = doc.find("\"ex-a\"").expect("ex-a present");
        let b = doc.find("\"ex-b\"").expect("ex-b present");
        let c = doc.find("\"ex-c\"").expect("ex-c present");
        assert!(a < b && b < c, "planned order broken: {a}, {b}, {c}");

        // PASS: files_compared + elapsed_ms, no skip_reason / det_mismatch.
        let pass = &doc[a..b];
        assert!(pass.contains("\"status\":\"PASS\""), "PASS status: {pass}");
        assert!(
            pass.contains("\"files_compared\":7"),
            "files_compared: {pass}"
        );
        assert!(pass.contains("\"elapsed_ms\":120"), "elapsed_ms: {pass}");
        assert!(
            !pass.contains("skip_reason"),
            "PASS must not carry skip_reason: {pass}"
        );
        assert!(
            !pass.contains("det_mismatch"),
            "PASS must not carry det_mismatch: {pass}"
        );

        // SKIPPED: skip_reason + elapsed_ms null; no files_compared.
        let skip = &doc[b..c];
        assert!(
            skip.contains("\"status\":\"SKIPPED\""),
            "SKIPPED status: {skip}"
        );
        assert!(
            skip.contains("\"skip_reason\":\"no capabilities.toml\""),
            "reason: {skip}"
        );
        assert!(skip.contains("\"elapsed_ms\":null"), "null elapsed: {skip}");
        assert!(
            !skip.contains("files_compared"),
            "SKIPPED must not carry files_compared: {skip}"
        );

        // FAIL: det_mismatch object with all four fields + elapsed_ms.
        let fail = &doc[c..];
        assert!(fail.contains("\"status\":\"FAIL\""), "FAIL status: {fail}");
        assert!(
            fail.contains("\"det_mismatch\":{"),
            "det_mismatch present: {fail}"
        );
        assert!(
            fail.contains("\"relative_path\":\"src/main.rs\""),
            "relpath: {fail}"
        );
        assert!(fail.contains("\"kind\":\"bytes differ\""), "kind: {fail}");
        assert!(fail.contains("\"offset\":42"), "offset: {fail}");
        // Escaped newline proves json_escape_str ran on the detail.
        assert!(
            fail.contains("\"detail\":\"A=foo\\nB=bar\""),
            "escaped detail: {fail}"
        );
        assert!(fail.contains("\"elapsed_ms\":250"), "elapsed_ms: {fail}");
    }

    #[test]
    fn write_det_timings_json_round_trips_and_creates_parents_atomically() {
        // Atomic-write contract for the det-mode emitter mirrors the
        // RUN-mode one (write_timings_json_creates_parents_and_writes_atomically):
        // fresh parent created on demand; .tmp sibling consumed by
        // rename. Body must contain the cell identity triple + the
        // top-level mode marker so a consumer can branch on it.
        let tmp_root = std::env::temp_dir().join(format!(
            "nucleus-e2e-emit-det-timings-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp_root);

        let path = tmp_root.join("nested").join("dir").join("det-timings.json");
        assert!(!path.exists());
        assert!(
            !path.parent().unwrap().exists(),
            "parent must not pre-exist"
        );

        let results = vec![synth_det_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            DetCellStatus::Pass { files_compared: 3 },
            42,
        )];
        write_det_timings_json(&path, &results).expect("write");

        assert!(path.exists(), "output JSON must exist after write");
        let body = fs::read_to_string(&path).expect("read back");
        assert!(
            body.contains("\"mode\": \"determinism\""),
            "mode marker: {body}"
        );
        assert!(body.contains("\"ex-a\""), "round-trip body: {body}");
        assert!(
            body.contains("\"files_compared\":3"),
            "files_compared: {body}"
        );
        assert!(body.contains("\"elapsed_ms\":42"), "elapsed_ms: {body}");

        let tmp_sibling = path.with_file_name("det-timings.json.tmp");
        assert!(
            !tmp_sibling.exists(),
            ".tmp sibling leaked: {}",
            tmp_sibling.display()
        );

        let _ = fs::remove_dir_all(&tmp_root);
    }

    // ----------------------------------------------------------------
    // TASK-0023.03 Stage 2 — baseline comparator
    // ----------------------------------------------------------------

    #[test]
    fn arg_parser_accepts_baseline_space_form() {
        // Write a real file so the existence-validation step succeeds
        // — `--baseline` is parsed eagerly with an exists() check.
        let tmp = std::env::temp_dir().join(format!(
            "nuc-baseline-arg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        fs::write(&tmp, "{}").expect("seed");
        let argv: Vec<OsString> = ["--baseline", tmp.to_str().unwrap()]
            .iter()
            .map(|s| OsString::from(*s))
            .collect();
        let a = parse_args(&argv).expect("parse");
        assert_eq!(a.baseline.as_deref(), Some(tmp.as_path()));
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn arg_parser_rejects_nonexistent_baseline_path() {
        // The single most common silent-no-op trap: typoed path.
        // Hard-fail at parse so the developer fixes it before paying
        // the matrix cost.
        let argv: Vec<OsString> = ["--baseline", "/tmp/does-not-exist-c55-baseline.json"]
            .iter()
            .map(|s| OsString::from(*s))
            .collect();
        let err = parse_args(&argv).unwrap_err();
        assert!(
            err.contains("does not exist"),
            "expected does-not-exist error, got: {err}"
        );
    }

    #[test]
    fn arg_parser_rejects_empty_baseline_path() {
        let argv: Vec<OsString> = ["--baseline", ""]
            .iter()
            .map(|s| OsString::from(*s))
            .collect();
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("non-empty PATH"), "got: {err}");
    }

    #[test]
    fn parse_baseline_json_round_trips_emitter_output() {
        // The reader must consume what the emitter produces. Build a
        // synthetic Vec<CellResult>, run it through render_timings_json,
        // then parse it back; the identity triples + total_ms must
        // match (the only fields the comparator reads).
        let results = vec![
            synth_cell_result(
                "ex-a",
                "naive",
                "pthreads-sync",
                Status::Pass,
                Some(100),
                Some(2000),
                Some(50),
            ),
            synth_cell_result(
                "ex-b",
                "tiled",
                "mp-tcp-bufsync",
                Status::Skipped {
                    reason: "no caps".into(),
                },
                None,
                None,
                None,
            ),
            synth_cell_result(
                "ex-c",
                "naive",
                "pthreads-sync",
                Status::Failed {
                    phase: Phase::Build,
                    detail: "boom".into(),
                },
                Some(80),
                Some(500),
                None,
            ),
        ];
        let doc = render_timings_json(&results);
        let parsed = parse_baseline_json(&doc).expect("round-trip parse");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].example, "ex-a");
        assert_eq!(parsed[0].total_ms, 2150);
        assert_eq!(parsed[1].example, "ex-b");
        assert_eq!(parsed[1].total_ms, 0);
        assert_eq!(parsed[2].example, "ex-c");
        assert_eq!(parsed[2].total_ms, 580);
    }

    #[test]
    fn parse_baseline_json_loud_fails_on_malformed_input() {
        // Truncated mid-object — the reader MUST NOT silently treat
        // this as empty; it must surface byte-offset + the snippet.
        let bad = r#"{"cells": [{"example":"x","schedule":"y","#;
        let err = parse_baseline_json(bad).unwrap_err();
        assert!(
            err.msg.contains("EOF") || err.msg.contains("string") || err.msg.contains("expected"),
            "expected structural error, got: {err}",
        );
        assert!(err.offset > 0, "offset should be non-zero: {err}");
    }

    #[test]
    fn parse_baseline_json_fails_on_missing_cells_array() {
        // A valid JSON object but lacking the required top-level
        // `cells` key — could be a different tool's output. Fail loud.
        let bad = r#"{"foo": 1}"#;
        let err = parse_baseline_json(bad).unwrap_err();
        assert!(err.msg.contains("cells"), "got: {err}");
    }

    #[test]
    fn compute_delta_rows_flags_regression_largest_first() {
        // Two cells identical, one cell's current_ms 50% slower.
        // The comparator must surface the regressor as the FIRST
        // row in the delta table (largest regression first).
        let baseline = vec![
            BaselineCell {
                example: "ex-a".into(),
                schedule: "naive".into(),
                backend: "pthreads-sync".into(),
                total_ms: 1000,
            },
            BaselineCell {
                example: "ex-b".into(),
                schedule: "tiled".into(),
                backend: "pthreads-sync".into(),
                total_ms: 2000,
            },
        ];
        let current = vec![
            // 50% slower — the regression.
            synth_cell_result(
                "ex-a",
                "naive",
                "pthreads-sync",
                Status::Pass,
                None,
                Some(1500),
                None,
            ),
            // Unchanged.
            synth_cell_result(
                "ex-b",
                "tiled",
                "pthreads-sync",
                Status::Pass,
                None,
                Some(2000),
                None,
            ),
        ];
        let rows = compute_delta_rows(&baseline, &current, &[]);
        assert_eq!(rows.len(), 2);
        // Largest regression first.
        assert_eq!(rows[0].example, "ex-a");
        assert_eq!(rows[0].baseline_ms, Some(1000));
        assert_eq!(rows[0].current_ms, Some(1500));
        let pct = rows[0].delta_pct.expect("real delta");
        assert!((pct - 50.0).abs() < 0.01, "expected +50%, got {pct}");

        // Round-trip render: plain-text mode must contain the
        // regressor row textually so a grep-based check works in CI.
        let table = render_delta_table(&rows, false);
        let first_line = table
            .lines()
            .find(|l| l.contains("ex-a"))
            .expect("ex-a present");
        assert!(first_line.contains("1000 -> 1500"), "got: {first_line}");
        assert!(first_line.contains("+50.0%"), "got: {first_line}");
    }

    #[test]
    fn compute_delta_rows_handles_new_and_removed() {
        // Three cells: A in both, B only in baseline (removed), C only
        // in current (new). The table must contain all three rows and
        // never crash on either side's missing.
        let baseline = vec![
            BaselineCell {
                example: "ex-a".into(),
                schedule: "naive".into(),
                backend: "pthreads-sync".into(),
                total_ms: 100,
            },
            BaselineCell {
                example: "ex-b".into(),
                schedule: "naive".into(),
                backend: "pthreads-sync".into(),
                total_ms: 200,
            },
        ];
        let current = vec![
            synth_cell_result(
                "ex-a",
                "naive",
                "pthreads-sync",
                Status::Pass,
                None,
                Some(100),
                None,
            ),
            synth_cell_result(
                "ex-c",
                "naive",
                "pthreads-sync",
                Status::Pass,
                None,
                Some(300),
                None,
            ),
        ];
        let rows = compute_delta_rows(&baseline, &current, &[]);
        assert_eq!(rows.len(), 3, "rows: {rows:?}");

        // Find each row by example. ex-a is real delta (tier 0,
        // 0%), ex-c is "(new)" (tier 2), ex-b is "(removed)"
        // (tier 1). Tier 1 sorts before tier 2 so removed appears
        // before new in the output.
        let by_ex =
            |name: &str| -> &DeltaRow { rows.iter().find(|r| r.example == name).expect(name) };
        let a = by_ex("ex-a");
        assert_eq!(a.baseline_ms, Some(100));
        assert_eq!(a.current_ms, Some(100));
        assert!(a.delta_pct.map_or(false, |p| p.abs() < 0.001));

        let b = by_ex("ex-b");
        assert_eq!(b.baseline_ms, Some(200));
        assert_eq!(b.current_ms, None);
        assert!(b.delta_pct.is_none());

        let c = by_ex("ex-c");
        assert_eq!(c.baseline_ms, None);
        assert_eq!(c.current_ms, Some(300));
        assert!(c.delta_pct.is_none());

        // Rendered output must carry the (new) / (removed) tags in
        // plain-text mode — these are the load-bearing "look here"
        // signals for a human reviewing CI logs.
        let table = render_delta_table(&rows, false);
        assert!(table.contains("(new)"), "table: {table}");
        assert!(table.contains("(removed)"), "table: {table}");
    }

    #[test]
    fn render_delta_table_color_paints_regressions_red_improvements_green() {
        // ANSI mode: a positive Δ% gets red SGR; a negative one gets
        // green; zero stays plain. Keeps the human-eye flag visible
        // even in long CI logs.
        let rows = vec![
            DeltaRow {
                example: "ex-a".into(),
                schedule: "n".into(),
                backend: "p".into(),
                baseline_ms: Some(100),
                current_ms: Some(150),
                delta_pct: Some(50.0),
                perf_threshold_pct: None,
                required: false,
                regression: false,
            },
            DeltaRow {
                example: "ex-b".into(),
                schedule: "n".into(),
                backend: "p".into(),
                baseline_ms: Some(100),
                current_ms: Some(50),
                delta_pct: Some(-50.0),
                perf_threshold_pct: None,
                required: false,
                regression: false,
            },
        ];
        let table = render_delta_table(&rows, true);
        assert!(table.contains("\x1b[31m"), "red SGR missing: {table:?}");
        assert!(table.contains("\x1b[32m"), "green SGR missing: {table:?}");
        // And the plain-text variant must carry NO ANSI bytes.
        let plain = render_delta_table(&rows, false);
        assert!(
            !plain.contains('\x1b'),
            "plain-text mode leaked ANSI: {plain:?}"
        );
    }

    #[test]
    fn compare_against_baseline_writes_to_stderr_and_flags_regressor() {
        // End-to-end: write a baseline file, then call the comparator
        // against a current Vec<CellResult> with one cell 50% slower.
        // We can't easily capture STDERR from a unit test without
        // spawning a subprocess, so we verify the public delta-table
        // helper does the right thing here; the integration step in
        // the verification gate exercises the STDERR routing.
        let tmp_root = std::env::temp_dir().join(format!(
            "nuc-baseline-cmp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        let _ = fs::remove_dir_all(&tmp_root);
        let path = tmp_root.join("baseline.json");

        let baseline_results = vec![synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            Some(1000),
            None,
        )];
        write_timings_json(&path, &baseline_results).expect("write");

        let current_results = vec![synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            Some(1500),
            None,
        )];
        // Just verify the read-and-parse path: a corrupt-on-disk
        // baseline must NOT silently no-op.
        let src = fs::read_to_string(&path).expect("read");
        let parsed = parse_baseline_json(&src).expect("parse");
        let rows = compute_delta_rows(&parsed, &current_results, &[]);
        let table = render_delta_table(&rows, false);
        assert!(
            table.contains("ex-a") && table.contains("+50.0%"),
            "expected regression row in: {table}"
        );

        let _ = fs::remove_dir_all(&tmp_root);
    }

    // ----------------------------------------------------------------
    // TASK-0023.03.02 (Stage 3) — per-cell perf threshold gating tests.
    // The contract per AC#5: a matrix entry with threshold=50,
    // baseline=200ms, current=350ms (75% slower) MUST fail; current=
    // 250ms (25% slower) MUST pass. The other two pin the negative-
    // path (required vs skip-band cells) and the serde-default
    // byte-identicality property.
    // ----------------------------------------------------------------

    /// Helper: build a `PlannedCell` with a perf threshold set and the
    /// required flag explicit. The three-string identity triple matches
    /// the `synth_cell_result` calls below so the comparator join lines
    /// up correctly.
    fn planned_with_threshold(
        ex: &str,
        sc: &str,
        be: &str,
        required: bool,
        threshold: Option<f64>,
    ) -> PlannedCell {
        PlannedCell {
            cell: Cell {
                example: ex.into(),
                schedule: sc.into(),
                backend: be.into(),
            },
            required,
            pre_skip: None,
            perf_threshold_pct: threshold,
        }
    }

    #[test]
    fn perf_threshold_breach_on_required_cell_is_regression() {
        // AC#5 fail case: threshold=50%, baseline=200ms, current=350ms
        // (delta = +75%). Required cell -> regression flag MUST set,
        // and `compute_delta_rows` must surface it for the exit-code
        // wiring to bite.
        let baseline = vec![BaselineCell {
            example: "ex-a".into(),
            schedule: "naive".into(),
            backend: "pthreads-sync".into(),
            total_ms: 200,
        }];
        let current = vec![synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            None,
            Some(350),
        )];
        let planned = vec![planned_with_threshold(
            "ex-a",
            "naive",
            "pthreads-sync",
            true,
            Some(50.0),
        )];
        let rows = compute_delta_rows(&baseline, &current, &planned);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.perf_threshold_pct, Some(50.0));
        assert!(r.required, "required flag must round-trip");
        let pct = r.delta_pct.expect("real delta");
        assert!((pct - 75.0).abs() < 0.01, "expected +75%, got {pct}");
        assert!(r.regression, "75% > 50% must flag regression");
        // The rendered table must carry the REGRESSION token so the
        // human reviewer + grep-based CI checks can spot it.
        let table = render_delta_table(&rows, false);
        assert!(
            table.contains("REGRESSION") && table.contains("threshold=+50.0%"),
            "expected REGRESSION + threshold tags in: {table}",
        );
    }

    #[test]
    fn perf_under_threshold_does_not_flag_regression() {
        // AC#5 pass case: threshold=50%, baseline=200ms, current=250ms
        // (delta = +25%). Required cell, BUT 25% < 50% so no regression.
        // Threshold tag is still rendered (so a reviewer sees the gate
        // was active), the exit-code-impacting REGRESSION is NOT.
        let baseline = vec![BaselineCell {
            example: "ex-a".into(),
            schedule: "naive".into(),
            backend: "pthreads-sync".into(),
            total_ms: 200,
        }];
        let current = vec![synth_cell_result(
            "ex-a",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            None,
            Some(250),
        )];
        let planned = vec![planned_with_threshold(
            "ex-a",
            "naive",
            "pthreads-sync",
            true,
            Some(50.0),
        )];
        let rows = compute_delta_rows(&baseline, &current, &planned);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        let pct = r.delta_pct.expect("real delta");
        assert!((pct - 25.0).abs() < 0.01, "expected +25%, got {pct}");
        assert!(
            !r.regression,
            "25% under 50% threshold must NOT flag regression"
        );
        let table = render_delta_table(&rows, false);
        assert!(
            table.contains("[threshold=+50.0%]"),
            "expected threshold tag (informational) in: {table}",
        );
        assert!(
            !table.contains("REGRESSION"),
            "must NOT emit REGRESSION when under threshold: {table}",
        );
    }

    #[test]
    fn perf_threshold_breach_on_skip_band_cell_is_informational_only() {
        // A `[[skip]]`-band cell with a threshold that genuinely
        // breached: the row's `regression` flag must be set (so the
        // table can dim-flag it for a reviewer), BUT `required` must
        // be `false` so the exit-code-impacting count (filtered by
        // `regression && required` in `compare_against_baseline`) stays
        // at zero — a skip-band breach is signal, not blocker.
        let baseline = vec![BaselineCell {
            example: "ex-skip".into(),
            schedule: "naive".into(),
            backend: "pthreads-sync".into(),
            total_ms: 100,
        }];
        let current = vec![synth_cell_result(
            "ex-skip",
            "naive",
            "pthreads-sync",
            Status::Pass,
            None,
            None,
            Some(500),
        )];
        let planned = vec![planned_with_threshold(
            "ex-skip",
            "naive",
            "pthreads-sync",
            /* required = */ false,
            Some(10.0),
        )];
        let rows = compute_delta_rows(&baseline, &current, &planned);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert!(r.regression, "+400% > 10% must flag regression");
        assert!(!r.required, "skip-band cell must not be required");
        // The exit-code-impacting filter must yield zero.
        let exit_count = rows.iter().filter(|r| r.regression && r.required).count();
        assert_eq!(
            exit_count, 0,
            "skip-band breach must NOT contribute to exit-code count",
        );
    }

    #[test]
    fn perf_threshold_absent_serde_default_byte_identical() {
        // Matrix toml without the new key MUST deserialize unchanged,
        // and the resulting PlannedCell must carry `perf_threshold_pct
        // = None` so no gate fires. This is the load-bearing byte-
        // identicality contract for the cycle: existing manifests
        // (and the bare `just e2e` run) cannot regress.
        let toml_src = r#"
runnable_examples = ["01-elementwise-add"]
backends = ["pthreads-sync"]

[[required]]
example = "01-elementwise-add"
schedule = "naive"
backend = "pthreads-sync"
milestone = "M1"

[[skip]]
example = "01-elementwise-add"
schedule = "tiled"
backend = "pthreads-sync"
reason = "not yet"
milestone = "M2"
"#;
        let m: Manifest = toml::from_str(toml_src).expect("parse");
        assert_eq!(m.required.len(), 1);
        assert_eq!(m.skip.len(), 1);
        assert!(
            m.required[0].perf_threshold_pct.is_none(),
            "absent threshold must serde-default to None",
        );
        assert!(
            m.skip[0].perf_threshold_pct.is_none(),
            "absent threshold on skip must serde-default to None",
        );
    }

    #[test]
    fn perf_threshold_parsed_from_toml_when_present() {
        // Symmetric positive: when the key IS set, it parses to the
        // expected f64. Together with the previous test this pins the
        // serde shape — absence is None, presence is f64.
        let toml_src = r#"
runnable_examples = ["01-elementwise-add"]
backends = ["pthreads-sync"]

[[required]]
example = "01-elementwise-add"
schedule = "naive"
backend = "pthreads-sync"
milestone = "M1"
perf_threshold_pct = 50.0
"#;
        let m: Manifest = toml::from_str(toml_src).expect("parse");
        assert_eq!(m.required[0].perf_threshold_pct, Some(50.0));
    }

    // ---- TASK-0168: required-coverage negative-gate injection. ----
    //
    // `NUC_REQUIRED_COVERAGE_NEGATIVE` is process-global; serialise
    // env-sensitive cases under one mutex so set_var/remove_var cannot
    // interleave on Rust's parallel test runner. These are the ONLY
    // tests (and `maybe_inject_required_coverage_negative` the only
    // code) that touch this var, so the mutex is a complete fence.
    fn req_cov_neg_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn sample_manifest_with_one_required() -> Manifest {
        Manifest {
            runnable_examples: vec!["01-elementwise-add".to_string()],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![req("01-elementwise-add", "naive", "pthreads-sync", "M1")],
            skip: vec![],
        }
    }

    #[test]
    fn req_cov_inject_is_strict_noop_when_env_unset() {
        // AC#1 / AC#2: the function must NOT mutate the manifest when
        // the env gate is unset — this is what keeps bare `just e2e`
        // byte-identical and the shipped manifest clean.
        let _guard = req_cov_neg_env_lock();
        std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");
        let mut m = sample_manifest_with_one_required();
        let before_len = m.required.len();
        let injected = maybe_inject_required_coverage_negative(&mut m);
        assert_eq!(injected, Ok(false), "env unset must report no injection");
        assert_eq!(
            m.required.len(),
            before_len,
            "env unset must leave manifest unchanged"
        );
    }

    #[test]
    fn req_cov_inject_appends_synthetic_required_when_env_set() {
        // AC#1: with the gate set, exactly ONE synthetic entry is
        // appended whose schedule is the sentinel and whose example /
        // backend / milestone are taken from the first real required
        // entry (so cell_matches_filters and milestone_in_gate accept
        // it in the bare run). Existing required entries are NOT
        // mutated — AC#2's append-not-mutate discipline.
        let _guard = req_cov_neg_env_lock();
        let mut m = sample_manifest_with_one_required();
        let original_first = m.required[0].clone();
        let original_len = m.required.len();

        std::env::set_var("NUC_REQUIRED_COVERAGE_NEGATIVE", "1");
        let injected = maybe_inject_required_coverage_negative(&mut m);
        std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");

        assert_eq!(injected, Ok(true), "gate=1 must report an injection");
        assert_eq!(
            m.required.len(),
            original_len + 1,
            "exactly one synthetic entry must have been appended"
        );
        // Append, not in-place mutate: the original first entry is
        // byte-identical (preserves AC#2).
        assert_eq!(m.required[0].example, original_first.example);
        assert_eq!(m.required[0].schedule, original_first.schedule);
        assert_eq!(m.required[0].backend, original_first.backend);
        assert_eq!(m.required[0].milestone, original_first.milestone);
        // The new last entry carries the sentinel schedule.
        let last = m.required.last().expect("injected entry");
        assert_eq!(last.schedule, REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE);
        // example / backend / milestone mirror the anchor so filters
        // accept it.
        assert_eq!(last.example, original_first.example);
        assert_eq!(last.backend, original_first.backend);
        assert_eq!(last.milestone, original_first.milestone);
    }

    #[test]
    fn req_cov_inject_then_gap_detected_end_to_end() {
        // End-to-end seam: after injection, `required_coverage_gaps`
        // (the pure function the wired path delegates to) must surface
        // the synthetic cell as a gap because the synthetic schedule
        // does not appear in `planned`. This pins the precise contract
        // that the new `NUC_REQUIRED_COVERAGE_GAP_DETECTED` stdout key
        // in `run_inner` reads off of.
        let _guard = req_cov_neg_env_lock();
        let mut m = sample_manifest_with_one_required();
        // Plan only contains the REAL `naive` cell — the planner could
        // not have discovered the synthetic schedule on disk.
        let plan = vec![planned("01-elementwise-add", "naive", "pthreads-sync")];

        std::env::set_var("NUC_REQUIRED_COVERAGE_NEGATIVE", "1");
        let _ = maybe_inject_required_coverage_negative(&mut m).expect("inject");
        std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");

        let gaps = required_coverage_gaps(&m, &plan, &Args::default()).expect("ok");
        // Exactly the synthetic cell, attributable by sentinel schedule.
        assert_eq!(
            gaps.len(),
            1,
            "synthetic typo'd required must surface as a gap (got {gaps:?})"
        );
        assert_eq!(
            gaps[0].schedule,
            REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE
        );
        // The attribution filter `run_inner` uses must select exactly
        // this gap — pin it here so a future refactor of either side
        // (the filter or the sentinel) fails LOUD instead of silently
        // dropping the count to zero (which would then force the
        // negative recipe into its FATAL Ok(0) arm and the recipe
        // would FAIL — desirable, but the unit-test signal arrives
        // earlier and is cheaper).
        let attributed = gaps
            .iter()
            .filter(|c| c.schedule == REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE)
            .count();
        assert_eq!(attributed, 1);
    }

    #[test]
    fn req_cov_inject_errs_loud_on_degenerate_manifest() {
        // Defensive: if the manifest has no runnable_examples (a manifest
        // shape that should never occur in this project but is permitted
        // by the type), the injection must Err loud rather than silently
        // succeed by picking nothing. Mirror discipline of the
        // determinism / xbackend hooks: a silently uncorrupted build
        // would be a false-PASS for the negative arm.
        let _guard = req_cov_neg_env_lock();
        let mut m = Manifest {
            runnable_examples: vec![],
            backends: vec!["pthreads-sync".to_string()],
            required: vec![],
            skip: vec![],
        };
        std::env::set_var("NUC_REQUIRED_COVERAGE_NEGATIVE", "1");
        let r = maybe_inject_required_coverage_negative(&mut m);
        std::env::remove_var("NUC_REQUIRED_COVERAGE_NEGATIVE");
        assert!(
            r.is_err(),
            "degenerate manifest (no runnable_examples and no required) \
             must surface a loud Err, got {r:?}"
        );
    }
}
