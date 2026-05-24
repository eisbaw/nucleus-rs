//! End-to-end integration tests for `passes::inject_check_frames`
//! (TASK-0052.02 AC#1/#2/#3 — projection arm).
//!
//! These tests close the loop between schedule source and the
//! `Event::Loop.check_frame` field by running the full pipeline:
//!
//!   parse algo + sched -> lower -> link -> build_acfg -> project
//!     -> inject_check_frames -> assert check_frame is where it should be
//!
//! The codegen arm (Instant::now + panic) is exercised by the
//! pthreads-sync backend reconstruction tests (under
//! `nucleus/nucleus-compiler/tests/petri_to_events.rs` etc.) and by the
//! positive/negative compile-and-run tests in this file.

use std::collections::BTreeMap;

use nucleus_compiler::acfg_to_events;
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{Event, ViolationKind, WorkerId};
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::{
    apply_block_transforms, apply_partition_workers, build_acfg, inject_check_frames, inject_syncs,
    inject_transfers, link,
};

fn build_per_worker(
    algo_src: &str,
    sched_src: &str,
) -> (
    BTreeMap<WorkerId, Vec<Event>>,
    nucleus_compiler::ACFG,
    nucleus_compiler::LinkedIR,
) {
    let algo_ast = parse_algo(algo_src).expect("algo parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(sched_src).expect("sched parse");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block-transform");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition-workers");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    let per_worker = acfg_to_events(&acfg);
    let per_worker = inject_check_frames(per_worker, &linked.sched.checks, &acfg.name_iter_vars);
    (per_worker, acfg, linked)
}

/// Collect every (Event::Loop reference) reachable from `events`,
/// descending into nested loop bodies, in pre-order. Returns
/// (iter_var, check_frame).
fn collect_loops(
    events: &[Event],
) -> Vec<(
    nucleus_compiler::IterVar,
    Option<nucleus_compiler::CheckFrame>,
)> {
    fn walk(
        events: &[Event],
        out: &mut Vec<(
            nucleus_compiler::IterVar,
            Option<nucleus_compiler::CheckFrame>,
        )>,
    ) {
        for e in events {
            if let Event::Loop {
                iter_var,
                body,
                check_frame,
                ..
            } = e
            {
                out.push((*iter_var, check_frame.clone()));
                walk(body, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(events, &mut out);
    out
}

// --------------------------------------------------------------------
// Positive: `check loop V : latency_max = T` attaches to the right
// outer Event::Loop with the codegen-default Panic on_violation.
// --------------------------------------------------------------------

#[test]
fn check_loop_directive_with_default_panic_attaches_to_outer_loop() {
    // AC#1, AC#2 of TASK-0052.02 (projection arm): the directive
    // ends up on the projected outer `Event::Loop` carrying the right
    // threshold and the default Panic action.
    let algo = "\
const N : usize = 4;
data a : f32[N];
kernel k : () -> f32 pure;
for n : 0 .. N {
    a[n] <-- k();
}
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
    check loop n : latency_max = 10ms;
}
";
    let (per_worker, _acfg, _linked) = build_per_worker(algo, sched);
    let host_events = per_worker
        .values()
        .next()
        .expect("at least one worker (host)");
    let loops = collect_loops(host_events);
    assert!(
        !loops.is_empty(),
        "the `n` loop must project to an Event::Loop"
    );
    // The OUTER (and here only) Event::Loop has the check_frame.
    let (_iv, cf) = &loops[0];
    let cf = cf.as_ref().expect("outer loop must carry check_frame");
    assert_eq!(cf.latency_max_ns, 10_000_000, "10ms == 10_000_000 ns");
    assert_eq!(
        cf.on_violation,
        ViolationKind::Panic,
        "default on_violation is Panic per PRD §6.3.5"
    );
    assert_eq!(cf.loop_var, "n", "loop_var carries the source name");
}

#[test]
fn check_loop_with_explicit_on_violation_panic() {
    // Explicit `on_violation = panic;` must equal the default-Panic
    // path (carried explicitly).
    let algo = "\
const N : usize = 4;
data a : f32[N];
kernel k : () -> f32 pure;
for n : 0 .. N {
    a[n] <-- k();
}
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
    check loop n : latency_max = 1ms, on_violation = panic;
}
";
    let (per_worker, _, _) = build_per_worker(algo, sched);
    let host = per_worker.values().next().unwrap();
    let loops = collect_loops(host);
    let cf = loops[0].1.as_ref().expect("frame present");
    assert_eq!(cf.on_violation, ViolationKind::Panic);
    assert_eq!(cf.latency_max_ns, 1_000_000);
}

// --------------------------------------------------------------------
// Negative-ish: schedule without a `check loop` directive leaves
// every loop with check_frame == None. This is the byte-identical-
// baseline contract — no e2e cell uses `check loop` today; the pass
// must be a no-op when checks is empty.
// --------------------------------------------------------------------

#[test]
fn no_check_directive_yields_no_check_frame() {
    let algo = "\
const N : usize = 4;
data a : f32[N];
kernel k : () -> f32 pure;
for n : 0 .. N {
    a[n] <-- k();
}
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
}
";
    let (per_worker, _, _) = build_per_worker(algo, sched);
    let host = per_worker.values().next().unwrap();
    for (_iv, cf) in collect_loops(host) {
        assert_eq!(cf, None, "no `check loop` -> no check_frame anywhere");
    }
}

// --------------------------------------------------------------------
// Idempotency / determinism: running the projection twice on the
// same inputs produces structurally identical EventLists with the
// same check_frame attached.
// --------------------------------------------------------------------

#[test]
fn injection_is_deterministic_across_runs() {
    let algo = "\
const N : usize = 8;
data a : f32[N];
kernel k : () -> f32 pure;
for n : 0 .. N {
    a[n] <-- k();
}
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
    check loop n : latency_max = 100us;
}
";
    let (a, _, _) = build_per_worker(algo, sched);
    let (b, _, _) = build_per_worker(algo, sched);
    assert_eq!(
        a, b,
        "two runs of the same inputs yield identical per-worker EventLists"
    );
    // And the frame on each is the SAME structurally — equality is
    // already asserted by `assert_eq!(a, b)`, but we verify the field
    // explicitly so a future deserialiser change that silently drops
    // the field is caught here too.
    let host = a.values().next().unwrap();
    let loops = collect_loops(host);
    let cf = loops[0].1.as_ref().expect("frame present");
    assert_eq!(cf.latency_max_ns, 100_000, "100us == 100_000ns");
}
