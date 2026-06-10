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
//! `nucleus-compiler` (then named `compiler`; renamed in TASK-0084
//! cycle 76). test-common can therefore depend on nucleus-compiler
//! and return a pre-built `nucleus_compiler::NameTables` directly in
//! [`LowerForTestResult`] — eliminating the 5-field literal block
//! that previously lived at every call site (3 backend tests + the
//! driver; cycle-24 close).
//!
//! # Scope vs the driver's pipeline
//!
//! `lower_for_test` runs the IR-stage subsequence the driver runs:
//! parse → lower → link → build_acfg → `apply_block_transforms` →
//! `apply_partition_workers` → inject_syncs → inject_transfers →
//! acfg_to_events → `inject_check_frames` → build_sidecar. Code-spanned
//! stages are gated by `LowerForTestOpts` (cycle-24 review-gate A.2).
//!
//! The driver ALSO runs `check_kernels_contract` (warning-only, no IR
//! effect) between lower and link. test-common does NOT (it's a
//! diagnostic surface a test doesn't exercise). This is therefore not
//! a strict mirror of the driver — it's an IR-shape mirror.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

/// Shared per-command wall-clock timeout with process-group teardown.
/// One implementation consumed by the diff-fuzz harness, the curated
/// `just e2e` harness, and the backend pingpong unit-test runners
/// (TASK-0466 / TASK-0461). See the module docs for why a hung child
/// must become a reported FAILURE, not a silent stall.
pub mod proc_timeout;

/// Create a process-and-call-unique scratch directory under `parent`.
///
/// Returns `parent/{leaf}-{pid}-{counter}`, where `pid` is this
/// process's id and `counter` is a per-process atomic that increments
/// on every call. The directory is created once and **never removed**.
///
/// # Why this exists (TASK-0426.01)
///
/// The backend test suites used to derive a scratch dir as
/// `parent/{leaf}` (a fixed, *profile-independent* path — `parent`
/// roots at `CARGO_MANIFEST_DIR`, which is identical for the dev and
/// release builds) and then did `remove_dir_all + create_dir_all` on
/// it. That `remove`/`create` dance is shared mutable filesystem state:
///
/// - Across THREADS within one `cargo test` process, two tests that
///   happened to share a leaf would race on the same path.
/// - Across PROCESSES, `just test` (dev profile) and `just test-release`
///   (release profile) run two binaries with different pids but the
///   SAME pid-less leaf path; one process's `remove_dir_all` can delete
///   a dir the other is mid-`fs::write`-ing, surfacing ENOENT.
///
/// Embedding the pid AND a per-call atomic counter makes every returned
/// path unique by construction, so no two callers — across threads OR
/// processes — ever share a path. With no sharing there is nothing to
/// `remove_dir_all`, so the race CLASS is eliminated structurally rather
/// than papered over.
///
/// HONESTY CAVEAT (carried from TASK-0426): the original ENOENT flake
/// was NOT reproducible in 16 runs; this is defense-in-depth that
/// removes a latent race class, not a reproduction-verified bug fix.
///
/// # Panics
///
/// Panics (via `.expect`) if `create_dir_all` fails — this is a test
/// helper and a failure here is an environment/test-author problem.
pub fn unique_scratch_dir(parent: &Path, leaf: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let _ = std::fs::create_dir_all(parent);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = parent.join(format!("{leaf}-{}-{}", std::process::id(), nonce));
    std::fs::create_dir_all(&dir).expect("create unique scratch dir");
    dir
}

/// Toggles for optional pipeline stages.
///
/// `apply_block_transforms` — run `nucleus_compiler::apply_block_transforms`
/// (needed for any schedule that uses `loop X : block=N`). **Default
/// true** to match the driver's unconditional behaviour. Tests that
/// want to reproduce the pre-cycle-24 byte-faithful behaviour of a
/// helper that DIDN'T call this pass (e.g. cycle-17 pthreads-async
/// `lower_example_01_naive`) can set this false, but the modern
/// expectation is `true` — block_transform is a no-op on schedules
/// with no `block=N` directives, so the default is safe for naive
/// schedules.
///
/// `apply_partition_workers` — run `nucleus_compiler::apply_partition_workers`
/// (needed for any schedule that uses `loop X : partition=workers`).
/// **Default false.**
///
/// `inject_check_frames` — run `nucleus_compiler::inject_check_frames` on the
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
/// which is now part of `nucleus-compiler`'s public API.
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
/// production code in `nucleus-compiler` returns typed errors instead.
/// Calling `lower_for_test` with malformed source is a test-author
/// bug, not a runtime concern.
pub fn lower_for_test(
    algo_src: &str,
    sched_src: &str,
    opts: &LowerForTestOpts,
) -> LowerForTestResult {
    use nucleus_compiler::{
        acfg_to_events,
        algo::{lower_algo, parse_algo},
        apply_block_transforms, apply_partition_workers, build_acfg, build_sidecar,
        inject_check_frames, inject_syncs, inject_transfers, link,
        sched::{lower_sched, parse_sched},
    };

    let algo_ir = lower_algo(&parse_algo(algo_src).expect("parse_algo")).expect("lower_algo");
    let sched_ir = lower_sched(&parse_sched(sched_src).expect("parse_sched")).expect("lower_sched");
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
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

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

// --------------------------------------------------------------------
// TASK-0245: Const-in-IndexExpr fixture (cycle-36 audit).
// --------------------------------------------------------------------
//
// Cycle 35 (TASK-0042.04 / commit 894f63f) discovered + fixed a bug in
// `pthreads_sync::render_int_expr`: a bare const ident (e.g. `ITERS`)
// inside an `IndexExpr` rendered as a bare Rust identifier rather
// than the const's literal value. That bare identifier is not in
// scope in the emitted code, so the generated `main.rs` failed to
// compile. The fix routed `render_int_expr` through `RenderCtx` and
// added the `sidecar.consts` lookup, matching `render_const_expr`'s
// precedence (`abs_subst > consts > bare-ident`).
//
// Architect review-gate (cycle 35) flagged that the fix lives in a
// PRIVATE pthreads-sync function. Both other backends (mp-tcp-bufsync,
// pthreads-async) MUST consume `IndexExpr` rendering ONLY through the
// pub shims (`render_flat_index_pub`, `render_const_expr_pub` plus
// the `render_fire_args_pub` / `render_fire_output_assign_pub` that
// call `render_flat_index` internally). The cycle-36 audit
// (TASK-0245) verified by `grep -rnE "fn render_int_expr|fn
// render_flat_index|fn render_const_expr"` on both backends that
// they declare NO such functions of their own — the shim is the only
// IndexExpr code path on all three backends.
//
// These fixture constants drive a per-backend test (one in each
// backend's `tests/` directory) that pins the contract STRUCTURALLY:
// the emitted `main.rs` MUST contain the resolved const value (`8`)
// at the IndexExpr site, and MUST NOT carry the bare const ident
// (`ITERS`) anywhere in the rendered source.
//
// If a future cycle adds a private parallel renderer in either
// backend without consulting `sidecar.consts`, the corresponding
// per-backend test fails immediately — the bug is caught at the
// emit-string layer, before the (slow, optional) cargo build/run.
//
// Fixture shape: a 2-worker schedule with `transfer x : sync`, no
// `partition=workers`, no `pipeline=`, no `block=` — the minimum
// shape that exercises (a) cross-worker dataflow on BOTH backends'
// multi-worker codegen paths and (b) `ITERS` inside an IndexExpr on
// the writer worker (w0) AND the reader worker (host). The
// per-backend tests do NOT cargo-build; they assert ONLY the emit-
// string shape (fast, drift-detection focused — TASK-0245's stated
// goal). Cycle-35's e2e already proved end-to-end correctness via
// example 11/pipelined × pthreads-async.

/// Algorithm with a `const ITERS : usize = 8` used inside an
/// `IndexExpr` — `y[ITERS][i]` writer and `y[ITERS][0]` reader.
/// `N` is also declared as a const but only used in TYPE positions
/// (it lowers to a folded literal at link time), so it does NOT
/// stress the `render_int_expr` Ident arm; `ITERS` does.
pub const CONST_IN_INDEXEXPR_ALGO_SRC: &str = r#"
const ITERS : usize = 8;
const N : usize = 4;
data x : i32[N];
data y : i32[ITERS+1][N];

kernel produce  : ()    -> i32[N] effectful;
kernel id_at    : (i32) -> i32    pure;
kernel sink_one : (i32) -> ()     effectful;

x <-- produce();
for i : 0 .. N {
    y[ITERS][i] <-- id_at(x[i]);
}
sink_one(y[ITERS][0]);
"#;

/// Two-worker schedule with cross-worker sync transfers — the
/// minimum shape to drive every backend's multi-worker codegen
/// (mp-tcp-bufsync rejects async / buffered transfers). `w0`
/// writes `y[ITERS][i]`; `host` reads `y[ITERS][0]`. Both
/// participate in the `transfer y : sync` cross-worker hop, so
/// the ITERS literal appears on BOTH worker bodies' rendered
/// IndexExpr sites.
pub const CONST_IN_INDEXEXPR_SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host, w0 };

    place produce  on host;
    place id_at    on w0;
    place sink_one on host;

    transfer x : sync;
    transfer y : sync;
}
"#;

/// The decimal value of `const ITERS` in
/// [`CONST_IN_INDEXEXPR_ALGO_SRC`]. The render_int_expr cycle-35
/// fix emits this as `{value}` (NO `_i64` suffix — the IndexExpr
/// context already casts to `usize` at the slice site). Tests
/// assert this literal appears at the IndexExpr arithmetic site
/// in the emitted `main.rs`.
pub const CONST_IN_INDEXEXPR_ITERS_VALUE: u64 = 8;

/// The bare-ident spelling of `ITERS` — this string MUST NOT
/// appear in the emitted `main.rs`. If it does, the const was
/// not resolved by `render_int_expr` and the codegen will fail
/// to compile (the bug cycle-35 fixed). Tests assert the absence
/// of this substring as the primary regression-pin.
pub const CONST_IN_INDEXEXPR_ITERS_IDENT: &str = "ITERS";

/// Find the bid (`SyncTag.0`) of the host-EXCLUDING barrier in a
/// per-worker [`Event`] projection: the [`Event::Sync`] whose
/// `participants` set does NOT contain `host`. Returns `None` if every
/// barrier includes `host` (i.e. there is no host-excluding barrier).
///
/// # Why this exists (TASK-0044.11)
///
/// The host-mediation oracle tests (`transpose_15_distributed_rows_*` in
/// the mp-tcp-event / mp-uds-event suites) assert that, after mediation,
/// the host bin declares the per-barrier shim `Bar{bid}` for the formerly
/// host-excluding barrier — where `worker_program.rs` emits
/// `bid == SyncTag.0`. Hardcoding `bid = 2` let the post-mediation half
/// SILENT-PASS under a sync-tag renumber: a host-INCLUDED barrier could
/// inherit `bid = 2` and satisfy the `Bar2` substring without the
/// mediated host-excluding barrier being present (architect P3-1 on
/// TASK-0044.08). Deriving the bid from the UNMEDIATED projection
/// re-targets the anchor to the correct barrier.
///
/// Must be read BEFORE mediation: `apply_host_mediation_inject` adds host
/// to the participant set, after which no barrier is host-excluding.
/// Recurses into [`Event::Loop`] bodies (the only nesting variant).
pub fn host_excluding_barrier_bid(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> Option<u64> {
    fn walk(events: &[Event], host: WorkerId) -> Option<u64> {
        for ev in events {
            match ev {
                Event::Sync {
                    participants, sync, ..
                } if !participants.contains(&host) => return Some(sync.0),
                Event::Loop { body, .. } => {
                    if let Some(bid) = walk(body, host) {
                        return Some(bid);
                    }
                }
                _ => {}
            }
        }
        None
    }
    per_worker.values().find_map(|evs| walk(evs, host))
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
        let r = lower_for_test(ALGO_SRC, SCHED_SRC, &LowerForTestOpts::default());
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
        // No check directives in this schedule, so the unified
        // transfer-facts map must be empty.
        assert!(
            r.sidecar.xfer_facts.is_empty(),
            "single-worker schedule produces no cross-worker transfers; \
             xfer_facts must be empty"
        );
    }

    #[test]
    fn unique_scratch_dir_returns_distinct_existing_paths_per_call() {
        // Two calls with the SAME leaf must yield DISTINCT directories
        // (the per-call atomic counter differentiates them), and both
        // must actually exist on disk. This is the core invariant that
        // eliminates the TASK-0426 scratch-path race class.
        let parent = std::env::temp_dir().join(format!(
            "nucleus-test-common-unique-scratch-{}",
            std::process::id()
        ));
        let a = unique_scratch_dir(&parent, "same-leaf");
        let b = unique_scratch_dir(&parent, "same-leaf");
        assert_ne!(a, b, "two calls with the same leaf must not collide");
        assert!(a.is_dir(), "first scratch dir must exist");
        assert!(b.is_dir(), "second scratch dir must exist");
        assert!(
            a.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(&std::process::id().to_string()),
            "leaf must embed the pid for cross-process disjointness"
        );
        // Cleanup: this helper deliberately never removes, so the test
        // tidies up its own parent (pid-scoped, safe to remove here).
        let _ = std::fs::remove_dir_all(&parent);
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

    // ---- host_excluding_barrier_bid (TASK-0044.11) ----

    use nucleus_compiler::event::{SyncKind, SyncTag};

    fn sync(parts: &[WorkerId], tag: u64) -> Event {
        Event::Sync {
            participants: parts.iter().copied().collect(),
            kind: SyncKind::Barrier,
            sync: SyncTag(tag),
        }
    }

    #[test]
    fn host_excluding_bid_picks_the_host_absent_barrier() {
        let host = WorkerId(0);
        let (w1, w2, w3) = (WorkerId(1), WorkerId(2), WorkerId(3));
        // A host-INCLUDED barrier (tag 1) and a host-EXCLUDING one (tag 7).
        let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
        pw.insert(host, vec![sync(&[host, w1, w2, w3], 1)]);
        pw.insert(
            w1,
            vec![sync(&[host, w1, w2, w3], 1), sync(&[w1, w2, w3], 7)],
        );
        pw.insert(w2, vec![sync(&[w1, w2, w3], 7)]);
        assert_eq!(
            host_excluding_barrier_bid(&pw, host),
            Some(7),
            "must return the SyncTag of the barrier whose participants exclude host"
        );
    }

    #[test]
    fn host_excluding_bid_is_none_when_every_barrier_includes_host() {
        let host = WorkerId(0);
        let (w1, w2) = (WorkerId(1), WorkerId(2));
        let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
        pw.insert(host, vec![sync(&[host, w1, w2], 1)]);
        pw.insert(w1, vec![sync(&[host, w1, w2], 1)]);
        assert_eq!(
            host_excluding_barrier_bid(&pw, host),
            None,
            "no host-excluding barrier => None (the post-mediation oracle would \
             then fail loud on the .expect(), not silent-pass)"
        );
    }

    #[test]
    fn host_excluding_bid_recurses_into_loop_body() {
        use nucleus_compiler::event::IterVar;
        let host = WorkerId(0);
        let (w1, w2) = (WorkerId(1), WorkerId(2));
        let nested = Event::Loop {
            iter_var: IterVar(0),
            range: 0..4,
            body: vec![sync(&[w1, w2], 5)],
            block_tag: None,
            check_frame: None,
            break_cond: None,
        };
        let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
        pw.insert(w1, vec![nested]);
        assert_eq!(
            host_excluding_barrier_bid(&pw, host),
            Some(5),
            "a host-excluding barrier nested inside an Event::Loop body must be found"
        );
    }
}
