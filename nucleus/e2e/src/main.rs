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
// Submodules (TASK-0460 content-preserving mega-file split)
//
// The harness was one 5371-LoC `main.rs`; it is carved into cohesive
// sibling modules along the section banners that already structured the
// file. Each carved name is re-exported into the crate root via
// `pub(crate) use`, so existing call sites in `run_inner` / `tests.rs`
// resolve unchanged through `use super::*`.
// --------------------------------------------------------------------

mod baseline;
mod cli;
mod determinism;
mod manifest;
mod paths;
mod plan;
mod report;
mod run;

pub(crate) use baseline::*;
pub(crate) use cli::*;
pub(crate) use determinism::*;
pub(crate) use manifest::*;
pub(crate) use paths::*;
pub(crate) use plan::*;
pub(crate) use report::*;
pub(crate) use report::ansi;
pub(crate) use run::*;

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
    // TASK-0446: pass the active tier so the synthetic cell anchors on a
    // backend the downstream `required_coverage_gaps` tier filter actually
    // sees (tier-1 by default; mpi backend under `--with-mpi`). The probe
    // below has not run yet, but injection is a pure manifest mutation that
    // needs no mpiexec; the gap check fires before any cell is built.
    let required_coverage_injected =
        maybe_inject_required_coverage_negative(&mut manifest, args.with_mpi)?;

    // Tier-2 MPI gate (TASK-0444). `--with-mpi` runs the `mpi_backends`
    // tier, which REQUIRES the `.#mpi` dev shell (rsmpi build deps + a
    // localhost MPI launcher). Probe `mpiexec` BEFORE planning and
    // HARD-FAIL if absent: silently skipping the tier-2 cells (e.g. when
    // `just e2e-mpi`'s `nix develop .#mpi` wrapper is bypassed, or the
    // flag is passed from the default shell) would be the silent-
    // coverage-loss class TASK-0163 closed for the required matrix and
    // that the project's memory repeatedly flags. The default-shell
    // matrix (`--with-mpi` unset) never reaches this probe, so bare
    // `just e2e` / `just ci` are unaffected and need no MPI.
    if args.with_mpi {
        let probe = Command::new("mpiexec").arg("--version").output();
        let ok = matches!(&probe, Ok(o) if o.status.success());
        if !ok {
            return Err(
                "--with-mpi requires the `.#mpi` dev shell: `mpiexec` was not \
                 found on PATH (or failed to run). Launch via `just e2e-mpi`, \
                 which enters `nix develop .#mpi`. Refusing to run so the \
                 tier-2 mpi cells (mpi-blocking + mpi-nonblocking) are \
                 never silently skipped."
                    .to_string(),
            );
        }
        eprintln!(
            "nucleus-e2e: tier-2 MPI gate (--with-mpi) — running the \
             `mpi_backends` tier under the `.#mpi` shell"
        );
    }

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

    // TASK-0369: a `[[fault_assert]]` triple that matches no planned cell
    // silently asserts nothing — the same silent-vanish class as a
    // typo'd `[[required]]`, applied to the fault path. Hard-fail here,
    // naming every orphaned triple, before spending minutes building
    // cells. Runs in BOTH run-mode and determinism-mode (both trust the
    // manifest), exactly like the required-coverage gate above.
    let fault_orphans = fault_assert_orphans(&manifest, &planned, &args)?;
    if !fault_orphans.is_empty() {
        let listed = fault_orphans
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
            "{} `[[fault_assert]]` cell(s) in {} can never fire: {}. A \
             fault_assert must name an existing (example, schedule, \
             backend) triple that actually RUNS this tier (check for a \
             typo, a stale entry after a rename, a missing matching \
             `[[required]]` declaration, or a fault_assert that lands on a \
             `[[skip]]`'d cell — a skipped cell short-circuits before the \
             artefact runs, so the fault check never executes).",
            fault_orphans.len(),
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
// running the full real matrix. Carved out to `src/tests.rs` cycle 185
// (TASK-0340 AC#4 / TASK-0340.09); see that file's docstring for the
// honesty disclosure about the original audit-text framing.
// --------------------------------------------------------------------

#[cfg(test)]
mod tests;
