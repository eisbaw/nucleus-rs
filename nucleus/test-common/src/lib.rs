//! Shared test-helper crate (TASK-0237). Houses the lower-link-inject
//! pipeline boilerplate that was duplicated across three backend test
//! suites before this crate landed.
//!
//! # Why this crate exists
//!
//! Before cycle 24, the parse → lower → link → ACFG → block-transforms
//! → partition-workers → inject_syncs/transfers → acfg_to_events →
//! inject_check_frames → build_sidecar pipeline was duplicated as a
//! ~30-line helper in three test files:
//!
//! - `nucleus/backends/pthreads-sync/tests/multi_worker.rs::lower_multi_worker_check_schedule`
//! - `nucleus/backends/mp-tcp-bufsync/tests/check_frame_emit.rs::build_per_worker_partitioned`
//! - `nucleus/backends/pthreads-async/tests/skeleton.rs::lower_example_01_naive`
//!
//! The architect's cycle-23 review (commit 8a5ee26) noted that Wave
//! B-2 of TASK-0228 would make this a 4-way duplication. Extracting
//! now (before Wave B-2) prevents that.
//!
//! # API shape (single entry point)
//!
//! [`lower_for_test`] runs the full pipeline and returns a
//! [`LowerForTestResult`] carrying the per-worker EventList +
//! NameSidecar + the five raw reverse-name-table BTreeMaps. Each
//! backend test composes its own `pthreads_sync::NameTables` from the
//! five maps (one 5-line local block per call site — but isolated to
//! the test files, not duplicated across the lowering pipeline).
//!
//! Why the result returns a pre-built NameTables:
//! TASK-0238 (cycle 25) moved NameTables from `pthreads-sync` to
//! `compiler`. test-common can therefore depend on compiler and
//! return a pre-built `compiler::NameTables` directly in
//! [`LowerForTestResult`] — eliminating the 5-field literal block
//! that previously lived at every call site (3 backend tests + the
//! driver; cycle-24 close).
//!
//! # Scope vs the driver's pipeline
//!
//! `lower_for_test` runs the IR-stage subsequence the driver runs:
//! parse → lower → link → build_acfg → [apply_block_transforms] →
//! [apply_partition_workers] → inject_syncs → inject_transfers →
//! acfg_to_events → [inject_check_frames] → build_sidecar. Bracketed
//! stages are gated by `LowerForTestOpts` (cycle-24 review-gate A.2).
//!
//! The driver ALSO runs `check_kernels_contract` (warning-only, no IR
//! effect) between lower and link. test-common does NOT (it's a
//! diagnostic surface a test doesn't exercise). This is therefore not
//! a strict mirror of the driver — it's an IR-shape mirror.

use std::collections::BTreeMap;

use compiler::event::{Event, WorkerId};
use compiler::sidecar::NameSidecar;
use compiler::NameTables;

/// Toggles for optional pipeline stages.
///
/// `apply_block_transforms` — run `compiler::apply_block_transforms`
/// (needed for any schedule that uses `loop X : block=N`). **Default
/// true** to match the driver's unconditional behaviour. Tests that
/// want to reproduce the pre-cycle-24 byte-faithful behaviour of a
/// helper that DIDN'T call this pass (e.g. cycle-17 pthreads-async
/// `lower_example_01_naive`) can set this false, but the modern
/// expectation is `true` — block_transform is a no-op on schedules
/// with no `block=N` directives, so the default is safe for naive
/// schedules.
///
/// `apply_partition_workers` — run `compiler::apply_partition_workers`
/// (needed for any schedule that uses `loop X : partition=workers`).
/// **Default false.**
///
/// `inject_check_frames` — run `compiler::inject_check_frames` on the
/// per-worker event lists (needed for any schedule with `check loop V`
/// directives). **Default false.**
#[derive(Debug, Clone, Copy)]
pub struct LowerForTestOpts {
    pub apply_block_transforms: bool,
    pub apply_partition_workers: bool,
    pub inject_check_frames: bool,
}

impl Default for LowerForTestOpts {
    fn default() -> Self {
        // `apply_block_transforms` defaults to TRUE to match the
        // driver's unconditional behaviour (cycle-24 review-gate A.2
        // finding: leaving block_transforms unconditional was the
        // pthreads-async skeleton's silent drift; now opt-in by
        // default but explicitly settable false to restore
        // pre-cycle-24 behaviour).
        Self {
            apply_block_transforms: true,
            apply_partition_workers: false,
            inject_check_frames: false,
        }
    }
}

/// Result of [`lower_for_test`]: the per-worker EventList + sidecar +
/// pre-built NameTables (built from the post-pass ACFG via
/// [`NameTables::from_acfg`]). Cycle-25 / TASK-0238 collapsed the
/// previous 5-raw-map shape into the canonical NameTables struct,
/// which is now part of `compiler`'s public API.
#[derive(Debug, Clone)]
pub struct LowerForTestResult {
    /// The per-worker EventList projection that `acfg_to_events`
    /// returned, optionally further annotated by `inject_check_frames`.
    pub per_worker: BTreeMap<WorkerId, Vec<Event>>,
    /// The sidecar `build_sidecar` produced from the post-pass ACFG.
    pub sidecar: NameSidecar,
    /// The reverse name tables built from the post-pass ACFG. The
    /// 3 backend tests can now use `r.names` directly instead of
    /// composing their own from raw maps.
    pub names: NameTables,
}

/// Run the lower-link-inject IR-stage pipeline for a (algo_src,
/// sched_src) pair, returning the contract inputs every backend test
/// consumes.
///
/// Runs the IR-stage subsequence of the driver's pre-emit pipeline at
/// `nucleus/driver/src/main.rs`. The driver also runs
/// `check_kernels_contract` (a warning-only diagnostic with no IR
/// effect) — this helper does NOT, since tests don't need the
/// diagnostic surface. Backend tests exercise the SAME IR shape the
/// driver feeds them via this helper.
///
/// # Panics
///
/// This helper is for TESTS. It panics (via `.expect`) on any
/// pipeline-stage failure, with a context message naming the stage —
/// production code in `compiler` returns typed errors instead.
/// Calling `lower_for_test` with malformed source is a test-author
/// bug, not a runtime concern.
pub fn lower_for_test(
    algo_src: &str,
    sched_src: &str,
    opts: &LowerForTestOpts,
) -> LowerForTestResult {
    use compiler::{
        acfg_to_events, apply_block_transforms, apply_partition_workers, build_acfg,
        build_sidecar,
        algo::{lower_algo, parse_algo},
        inject_check_frames, inject_syncs, inject_transfers, link,
        sched::{lower_sched, parse_sched},
    };

    let algo_ir =
        lower_algo(&parse_algo(algo_src).expect("parse_algo")).expect("lower_algo");
    let sched_ir =
        lower_sched(&parse_sched(sched_src).expect("parse_sched")).expect("lower_sched");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = if opts.apply_block_transforms {
        apply_block_transforms(&linked, acfg).expect("apply_block_transforms")
    } else {
        acfg
    };
    let acfg = if opts.apply_partition_workers {
        apply_partition_workers(&linked, acfg).expect("apply_partition_workers")
    } else {
        acfg
    };
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);

    let per_worker = acfg_to_events(&acfg);
    let per_worker = if opts.inject_check_frames {
        inject_check_frames(per_worker, &linked.sched.checks, &acfg.name_iter_vars)
    } else {
        per_worker
    };
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");

    let names = NameTables::from_acfg(&acfg);

    LowerForTestResult {
        per_worker,
        sidecar,
        names,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal smoke test — proves the pipeline runs end-to-end on a
    // tiny self-contained source. The 3 backend test suites
    // (consumers) carry the substantive coverage.
    const ALGO_SRC: &str = r#"
const N : usize = 4;
data x : i32[N];
data y : i32[N];

kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel inc         : (i32)    -> i32   pure;

x <-- load_input();
for n : 0 .. N {
    y[n] <-- inc(x[n]);
}
save_output(y);
"#;

    const SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place save_output on host;
    place inc         on host;
}
"#;

    #[test]
    fn single_worker_default_opts_produces_complete_result() {
        let r = lower_for_test(
            ALGO_SRC,
            SCHED_SRC,
            &LowerForTestOpts::default(),
        );
        assert!(!r.per_worker.is_empty(), "per_worker must be non-empty");
        assert!(
            !r.names.data.is_empty(),
            "names.data must contain x + y at minimum"
        );
        assert!(
            r.names.kernel.values().any(|n| n == "inc"),
            "names.kernel must contain the inc kernel"
        );
        assert!(
            r.names.worker.values().any(|n| n == "host"),
            "names.worker must contain host"
        );
        // No check directives in this schedule, so the transfer
        // buffer map must be empty.
        assert!(
            r.sidecar.transfer_buffer_for_seq.is_empty(),
            "single-worker schedule produces no cross-worker transfers; \
             transfer_buffer_for_seq must be empty"
        );
    }

    #[test]
    fn opts_default_block_transforms_on_others_off() {
        let opts = LowerForTestOpts::default();
        assert!(
            opts.apply_block_transforms,
            "apply_block_transforms defaults TRUE to match driver behaviour \
             (cycle-24 review-gate A.2)"
        );
        assert!(!opts.apply_partition_workers);
        assert!(!opts.inject_check_frames);
    }
}
