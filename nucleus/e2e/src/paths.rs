//! Repo-relative path discovery + the per-invocation run-id scratch root.
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// Paths
// --------------------------------------------------------------------

/// Layout the harness depends on:
///
/// ```text
/// <repo_root>/
///   nucleus/                  # cargo workspace
///     Cargo.toml
///     e2e/                    # this crate
///     backends/<name>/capabilities.toml
///   nuc-nucleus/
///     e2e-matrix.toml
///     examples/<NN-name>/...
/// ```
///
/// `Clone`: required so `execute_cells_parallel` can stash a snapshot
/// inside an `Arc` for the worker pool. Cheap — two heap strings.
#[derive(Clone)]
pub(crate) struct Paths {
    pub(crate) repo_root: PathBuf,
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
    pub(crate) run_id: String,
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
    pub(crate) fn compute_run_id() -> String {
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

    pub(crate) fn discover() -> Result<Self, String> {
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

    pub(crate) fn nucleus_ws(&self) -> PathBuf {
        self.repo_root.join("nucleus")
    }

    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.repo_root.join("nuc-nucleus/e2e-matrix.toml")
    }

    pub(crate) fn example_dir(&self, ex: &str) -> PathBuf {
        self.repo_root.join("nuc-nucleus/examples").join(ex)
    }

    pub(crate) fn backend_caps(&self, backend: &str) -> PathBuf {
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
    pub(crate) fn run_scratch_root(&self) -> PathBuf {
        self.repo_root
            .join("nucleus/target/e2e-matrix")
            .join(&self.run_id)
    }

    /// The per-RUN determinism scratch root:
    /// `nucleus/target/e2e-determinism/<run-id>`.
    pub(crate) fn run_determinism_root(&self) -> PathBuf {
        self.repo_root
            .join("nucleus/target/e2e-determinism")
            .join(&self.run_id)
    }

    /// Per-cell scratch directory under this run's root. `cargo clean`
    /// sweeps it. Removed and recreated so stale artefacts cannot mask
    /// a regression — and, since the parent segment is the per-run
    /// `<run-id>`, that `remove_dir_all` can only ever touch THIS
    /// process's own tree, never a sibling run still cwd-d into it.
    pub(crate) fn scratch_dir(&self, ex: &str, sched: &str, backend: &str) -> Result<PathBuf, String> {
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
    pub(crate) fn determinism_dir(
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
    ///
    /// Retained-run PRUNING (TASK-0461 AC#3, wave-6 ENOSPC addendum):
    /// on a FAILED run, after retaining THIS run's tree, prune the OLDEST
    /// previously-retained `<run-id>` dirs under the same parent, keeping
    /// only the [`RETAINED_RUNS_KEEP`] most recent. Without this a long
    /// series of failed `just e2e` invocations accumulated `<run-id>`
    /// trees unbounded (a contributor to the 70-84GB target/ blowups);
    /// keeping the last K preserves recent post-mortems while bounding
    /// the steady state. The CURRENT run is always among the kept K (it
    /// is the newest by mtime).
    pub(crate) fn finalize_run_scratch(&self, success: bool) {
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
                // Bound the retained-failure series (keep last K).
                if let Some(parent) = root.parent() {
                    prune_retained_runs(parent, RETAINED_RUNS_KEEP);
                }
            }
        }
    }
}

/// How many retained (failed-run) `<run-id>` trees to keep per scratch
/// parent before pruning the oldest. 4 keeps the last few post-mortems —
/// enough to compare a flaky regression across runs — while bounding the
/// disk a long failure series can hold. Documented as a deliberate
/// choice (the prior policy was unbounded retention).
pub(crate) const RETAINED_RUNS_KEEP: usize = 4;

/// Keep the `keep` most-recently-modified immediate child dirs of
/// `parent`, removing the rest. Used to bound retained failed-run scratch
/// (TASK-0461 AC#3). Best-effort: any IO error is reported but never
/// fails the run. Sort key is mtime (newest kept); the current run's tree
/// is the newest, so it is never pruned.
pub(crate) fn prune_retained_runs(parent: &std::path::Path, keep: usize) {
    let entries = match fs::read_dir(parent) {
        Ok(e) => e,
        Err(_) => return, // parent vanished / unreadable — nothing to do
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Liveness guard (hardening-wave review P2.1): a CONCURRENT run
        // mid-way through a long cold build can have a stale mtime and
        // sort older than freshly-retained failures — pruning it would
        // resurrect the cross-process scratch race TASK-0182 closed.
        // Run dirs embed the creating pid (`run-<pid>-<nanos>`); skip
        // any dir whose pid is still alive.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if let Some(pid) = name
                .strip_prefix("run-")
                .and_then(|r| r.split('-').next())
                .and_then(|p| p.parse::<u32>().ok())
            {
                if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                    continue;
                }
            }
        }
        // mtime; fall back to UNIX_EPOCH so an unreadable mtime sorts as
        // oldest (gets pruned first), never as newest (never retained
        // over a real recent run).
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        dirs.push((mtime, path));
    }
    if dirs.len() <= keep {
        return;
    }
    // Newest first; drop the first `keep`, remove the tail.
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in dirs.into_iter().skip(keep) {
        if let Err(e) = fs::remove_dir_all(&path) {
            eprintln!(
                "nucleus-e2e: WARNING: could not prune old retained scratch \
                 `{}`: {e}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod prune_tests {
    use super::prune_retained_runs;
    use std::fs;
    use std::time::Duration;

    /// `prune_retained_runs` keeps exactly the `keep` newest dirs and
    /// removes the older ones; a `keep`-or-fewer set is left untouched.
    /// Distinct mtimes are forced by a tiny sleep between creates so the
    /// "newest kept" ordering is deterministic on a coarse-mtime FS.
    #[test]
    fn keeps_last_k_by_mtime() {
        let parent = std::env::temp_dir().join(format!(
            "nuc-prune-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&parent).unwrap();

        // Create 6 run dirs oldest-to-newest with a stub file each.
        let mut names = Vec::new();
        for i in 0..6 {
            let d = parent.join(format!("run-{i}"));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("marker"), b"x").unwrap();
            names.push(d);
            // Coarse mtime granularity: nudge each create apart.
            std::thread::sleep(Duration::from_millis(15));
        }

        prune_retained_runs(&parent, 4);

        // The 4 newest (run-2..run-5) survive; run-0, run-1 are pruned.
        assert!(!names[0].exists(), "oldest run-0 should be pruned");
        assert!(!names[1].exists(), "run-1 should be pruned");
        for d in &names[2..] {
            assert!(d.exists(), "newest 4 must survive: {}", d.display());
        }

        // A second prune with keep >= remaining count is a no-op.
        prune_retained_runs(&parent, 4);
        for d in &names[2..] {
            assert!(d.exists(), "no-op prune must not remove survivors");
        }

        fs::remove_dir_all(&parent).ok();
    }

    /// A nonexistent / unreadable parent is a silent no-op (never panics).
    #[test]
    fn missing_parent_is_noop() {
        let nope = std::env::temp_dir().join("nuc-prune-does-not-exist-zzz-xyz");
        let _ = fs::remove_dir_all(&nope);
        prune_retained_runs(&nope, 4); // must not panic
    }
}

