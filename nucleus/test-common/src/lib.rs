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
//! Why the result returns the maps instead of a pre-built NameTables:
//! NameTables lives in `pthreads-sync`. test-common cannot depend on
//! pthreads-sync (would create a circular arrow: pthreads-sync
//! dev-dep test-common → test-common deps pthreads-sync). Returning
//! the raw maps lets every backend's test compose its own NameTables
//! type — and Wave B-2's pthreads-async tests will do the same.

use std::collections::{BTreeMap, BTreeSet};

use compiler::event::{DataId, Event, IterVar, KernelId, WorkerId};
use compiler::sidecar::NameSidecar;

/// Toggles for optional pipeline stages.
///
/// `apply_partition_workers` — run `compiler::apply_partition_workers`
/// (needed for any schedule that uses `loop X : partition=workers`).
///
/// `inject_check_frames` — run `compiler::inject_check_frames` on the
/// per-worker event lists (needed for any schedule with `check loop V`
/// directives).
#[derive(Debug, Clone, Copy, Default)]
pub struct LowerForTestOpts {
    pub apply_partition_workers: bool,
    pub inject_check_frames: bool,
}

/// Result of [`lower_for_test`]: the per-worker EventList + sidecar +
/// the five raw reverse-name-table maps the caller needs to compose
/// its backend's `NameTables` type.
#[derive(Debug, Clone)]
pub struct LowerForTestResult {
    /// The per-worker EventList projection that `acfg_to_events`
    /// returned, optionally further annotated by `inject_check_frames`.
    pub per_worker: BTreeMap<WorkerId, Vec<Event>>,
    /// The sidecar `build_sidecar` produced from the post-pass ACFG.
    pub sidecar: NameSidecar,
    /// Reverse of `ACFG::name_data` (id → name).
    pub name_data: BTreeMap<DataId, String>,
    /// Reverse of `ACFG::name_kernels` (id → name).
    pub name_kernel: BTreeMap<KernelId, String>,
    /// Reverse of `ACFG::name_workers` (id → name).
    pub name_worker: BTreeMap<WorkerId, String>,
    /// Reverse of `ACFG::name_iter_vars` (id → name).
    pub name_iter_var: BTreeMap<IterVar, String>,
    /// `ACFG::inner_block_iter_vars` cloned (the inner intra-tile
    /// loop iter-var set produced by `block_transform`; carried so
    /// the backend's NameTables.inner_block_iter_vars is identical
    /// to what the driver produces).
    pub inner_block_iter_vars: BTreeSet<IterVar>,
}

/// Run the full lower-link-inject pipeline for a (algo_src, sched_src)
/// pair, returning the contract inputs every backend test consumes.
///
/// Mirrors the driver's pre-emit pipeline at
/// `nucleus/driver/src/main.rs` so backend tests exercise the SAME
/// shape the driver feeds them — no test-only shortcuts that drift
/// from production behaviour.
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
    let acfg = apply_block_transforms(&linked, acfg).expect("apply_block_transforms");
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

    LowerForTestResult {
        per_worker,
        sidecar,
        name_data: acfg.name_data.iter().map(|(n, i)| (*i, n.clone())).collect(),
        name_kernel: acfg
            .name_kernels
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        name_worker: acfg
            .name_workers
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        name_iter_var: acfg
            .name_iter_vars
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        inner_block_iter_vars: acfg.inner_block_iter_vars.clone(),
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
            !r.name_data.is_empty(),
            "name_data must contain x + y at minimum"
        );
        assert!(
            r.name_kernel.values().any(|n| n == "inc"),
            "name_kernel must contain the inc kernel"
        );
        assert!(
            r.name_worker.values().any(|n| n == "host"),
            "name_worker must contain host"
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
    fn opts_default_is_off_off() {
        let opts = LowerForTestOpts::default();
        assert!(!opts.apply_partition_workers);
        assert!(!opts.inject_check_frames);
    }
}
