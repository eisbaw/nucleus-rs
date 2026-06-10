//! Command-line argument parsing for the `nucleus-e2e` harness.
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// CLI args
// --------------------------------------------------------------------

/// Output format selector (TASK-0023.02). `Text` is the existing
/// human-readable summary table; `Junit` emits a JUnit XML
/// `<testsuites>` document on stdout so CI runners (GitHub Actions /
/// GitLab Pipelines) can surface individual matrix cells as named
/// test cases. Default is `Text` so `just e2e` is byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Text,
    Junit,
}

impl Format {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
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
pub(crate) struct Args {
    pub(crate) example: Option<String>,
    pub(crate) schedule: Option<String>,
    pub(crate) backend: Option<String>,
    /// Milestone gate (PRD §11). When set, the required/skip matrix
    /// is narrowed to cells tagged at or before this milestone — the
    /// gate is CUMULATIVE: `--milestone M3` runs the M1 ∪ M2 ∪ M3
    /// required cells (a regression gate should never drop an
    /// earlier-milestone cell). Absent ⇒ the full matrix (unchanged
    /// behaviour). Validated to a [`Milestone`] at parse time so a
    /// bad value fails LOUD before any work.
    pub(crate) milestone: Option<Milestone>,
    /// When set, the harness switches modes: instead of running the
    /// compile/build/run/diff pipeline, it invokes `nucleus build`
    /// twice per cell into two distinct out dirs and byte-compares
    /// every generated file. See TASK-0033 and PRD §1 / §10.1.
    pub(crate) check_determinism: bool,
    /// Number of worker threads for parallel cell execution
    /// (TASK-0023.01). Default 1 = sequential, byte-for-byte identical
    /// to pre-flag behaviour. Capped at `MAX_JOBS` to avoid pathological
    /// fork bombs (each concurrent `cargo build --release` of an
    /// emitted project costs ~200-500MB peak; --jobs 4 on a typical
    /// 14-core / 30 GB host is comfortable, much higher risks OOM).
    /// Validated >= 1 at parse time. Each cell's scratch dir is already
    /// unique-by-construction via TASK-0182's run-id segment, so
    /// in-process parallel cells do not collide on disk.
    pub(crate) jobs: usize,
    /// Output format for the per-cell summary (TASK-0023.02). `Text`
    /// (default) writes the existing human-readable table to stdout;
    /// `Junit` writes a JUnit XML `<testsuites>` document to stdout so
    /// CI runners can surface cells as test cases. The exit-code +
    /// gate-signal semantics (required-fail / `NUC_XBACKEND_*` /
    /// `NUC_NONDET_*`) are independent of this choice.
    pub(crate) format: Format,
    /// Optional path to write per-cell wall-clock timings as JSON
    /// (TASK-0023.03 Stage 1). When `Some`, after the matrix completes
    /// the harness writes a JSON document mirroring the planned-order
    /// `Vec<CellResult>` so a downstream comparator can flag perf
    /// regressions against a stored baseline. Default `None` =
    /// byte-identical to the pre-flag harness output. Stage 1 only
    /// covers RUN-mode results (`run_cell`); `--check-determinism` is
    /// out of scope for this stage and is filed as a follow-up.
    pub(crate) emit_timings: Option<PathBuf>,
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
    pub(crate) baseline: Option<PathBuf>,
    /// Run the tier-2 MPI tier (`mpi_backends`) INSTEAD of the default
    /// `backends` tier (TASK-0444). Set by `--with-mpi` (driven by
    /// `just e2e-mpi`, which enters the `.#mpi` dev shell). When true,
    /// `plan_cells` / `required_coverage_gaps` / `fault_assert_orphans`
    /// scope to the mpi tier via `Manifest::is_mpi_backend`, and
    /// `run_inner` HARD-FAILS at startup if `mpiexec` is not on PATH —
    /// silently skipping the tier-2 cells (e.g. when this flag is set
    /// from the default shell by mistake) would be the silent-coverage-
    /// loss class the project repeatedly guards against. Default false =
    /// byte-identical to the pre-flag harness (bare `just e2e`).
    pub(crate) with_mpi: bool,
}

/// Upper bound on `--jobs N`. See [`Args::jobs`].
pub(crate) const MAX_JOBS: usize = 64;

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
            with_mpi: false,
        }
    }
}

pub(crate) fn parse_args(argv: &[OsString]) -> Result<Args, String> {
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
            "--with-mpi" => {
                // TASK-0444: run the tier-2 `mpi_backends` tier instead
                // of the default `backends` tier. Requires the `.#mpi`
                // dev shell; `run_inner` probes `mpiexec` up front and
                // hard-fails if absent (no silent skip). Invoke via
                // `just e2e-mpi`.
                a.with_mpi = true;
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

pub(crate) fn print_help() {
    eprintln!(
        "nucleus-e2e — Nuc v2 differential test matrix harness\n\
         \n\
         USAGE:\n    \
             nucleus-e2e [--example NAME] [--schedule NAME] [--backend NAME] \
[--milestone ID] [--check-determinism] [--with-mpi] [--jobs N | -j N] [--format text|junit] \
[--emit-timings PATH] [--baseline PATH]\n\
         \n\
         Bare invocation runs every cell declared in\n\
         `nuc-nucleus/e2e-matrix.toml`. Flags narrow the matrix to\n\
         matching cells.\n\
         \n\
         --with-mpi: run the tier-2 `mpi_backends` tier (mpi-blocking +\n\
         mpi-nonblocking)\n\
         INSTEAD of the default `backends` tier (TASK-0444). Requires\n\
         the `.#mpi` dev shell — HARD-FAILS at startup if `mpiexec` is\n\
         not on PATH (no silent skip). Invoke via `just e2e-mpi`. The\n\
         out-of-default-matrix sibling of `just renode-multimcu-gate`.\n\
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

