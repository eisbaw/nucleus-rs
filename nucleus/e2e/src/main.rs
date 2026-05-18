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
    required: Vec<Cell>,
    #[serde(default)]
    skip: Vec<SkipEntry>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct Cell {
    example: String,
    schedule: String,
    backend: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkipEntry {
    example: String,
    schedule: String,
    backend: String,
    reason: String,
}

/// Subset of a backend's `capabilities.toml`. The harness only sniffs
/// that the file *parses as TOML* — the compiler's
/// `load_capabilities` is the authoritative schema validator and the
/// driver invokes it on every compile. Keeping this struct empty
/// makes the schema's source-of-truth split obvious: the harness
/// merely confirms the file is reachable + lexically valid.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct CapabilitiesSniff {}

// --------------------------------------------------------------------
// CLI args
// --------------------------------------------------------------------

#[derive(Debug, Default)]
struct Args {
    example: Option<String>,
    schedule: Option<String>,
    backend: Option<String>,
    /// Reserved for the post-M1 milestone-tagged subsets in PRD §11.
    /// At M1 the flag is accepted but only logged — the manifest is
    /// the milestone gate today.
    milestone: Option<String>,
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
                a.milestone = Some(need_val(i)?);
                i += 2;
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
             nucleus-e2e [--example NAME] [--schedule NAME] [--backend NAME] [--milestone ID]\n\
         \n\
         Bare invocation runs every cell declared in\n\
         `nuc-nucleus/e2e-matrix.toml`. Flags narrow the matrix to\n\
         matching cells.\n"
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
    let required_set: BTreeSet<Cell> = manifest.required.iter().cloned().collect();
    let mut skip_map: std::collections::BTreeMap<Cell, String> = std::collections::BTreeMap::new();
    for s in &manifest.skip {
        skip_map.insert(
            Cell {
                example: s.example.clone(),
                schedule: s.schedule.clone(),
                backend: s.backend.clone(),
            },
            s.reason.clone(),
        );
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
                let required = required_set.contains(&cell);
                let pre_skip = skip_map.get(&cell).cloned();
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
    let _caps: CapabilitiesSniff = match toml::from_str(&caps_src) {
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

    // ---- Phase 3: run the generated binary -----------------------------
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
    let output_bin = scratch.join("output.bin");
    let t2 = Instant::now();
    let run = Command::new(&exe)
        .env("NUC_INPUT_PATH", &input_bin)
        .env("NUC_OUTPUT_PATH", &output_bin)
        .output();
    timings.run = Some(t2.elapsed());
    let run = match run {
        Ok(o) => o,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Run,
                    detail: format!("spawn {}: {e}", exe.display()),
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
        // Reserved for PRD §11's milestone-tagged matrix subsets.
        // Today we accept and announce; the manifest is still the
        // source of truth.
        eprintln!(
            "nucleus-e2e: --milestone={m} accepted but ignored at M1 \
             (manifest gates required cells)"
        );
    }

    let planned = plan_cells(&paths, &manifest, &args)?;
    if planned.is_empty() {
        return Err("no cells matched the given filters; nothing to run".to_string());
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
        assert_eq!(a.milestone.as_deref(), Some("M1"));
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
            required: vec![Cell {
                example: "01-elementwise-add".to_string(),
                schedule: "naive".to_string(),
                backend: "pthreads-sync".to_string(),
            }],
            skip: vec![],
        };
        let args = Args {
            example: Some("01-elementwise-add".into()),
            schedule: Some("naive".into()),
            backend: Some("pthreads-sync".into()),
            milestone: None,
        };
        let planned = plan_cells(&paths, &manifest, &args).expect("plan");
        assert_eq!(planned.len(), 1);
        assert!(planned[0].required);
        assert!(planned[0].pre_skip.is_none());
        assert_eq!(planned[0].cell.schedule, "naive");
    }

    /// Skip-entry plumbing: a [[skip]] entry produces a SKIPPED
    /// status with the manifest's reason, *without* invoking cargo.
    /// Drives run_cell directly with a manifest-synthesised skip;
    /// validates we bail out before touching the filesystem.
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
}
