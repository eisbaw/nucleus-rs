//! Integration tests for the per-worker EventList projection pass
//! (TASK-0027, PRD §8.1 and §8.3).
//!
//! Strategy mirrors `tests/acfg_to_petri.rs`:
//!
//! - **Synthetic positive cases**: hand-built tiny ACFGs targeting one
//!   variant per test (single Operation, Push/Wait pair, Sync, Repeat
//!   unrolling).
//! - **End-to-end**: build the ACFG for example 02-split-add under
//!   `split.sched.nuc`, run sync+transfer injection, lower to Petri
//!   net, then project to per-worker `EventList`s. Assert: one
//!   EventList per declared worker; bit-identical between two runs.
//!
//! What this file does NOT cover:
//! - The petri-net firing semantics (lives in `tests/petri.rs`).
//! - Capability validation. Out-of-scope.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{
    build_acfg, ACFGNode, DataflowDag, DataflowEdge, NotifyMode, Operation, SyncPlaceholder,
    TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{DataId, Event, IterTile, KernelId, SeqTag, SyncKind, WorkerId};
use nucleus_compiler::link;
use nucleus_compiler::passes::acfg_to_petri::acfg_to_net;
use nucleus_compiler::passes::petri_to_events::{acfg_to_events, petri_to_events};
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::sched::{lower_sched, parse_sched};

// One per-data expectation: (data symbol, expected vec! length,
// expected element type). Aliased to keep the table type below within
// clippy::type_complexity (TASK-0186).
type DataCheck = (&'static str, usize, &'static str);
// One row of the sidecar-sizing expectations table:
// (algo path, schedule path, per-data checks).
type SidecarExpectation = (&'static str, &'static str, &'static [DataCheck]);

// --------------------------------------------------------------------
// Synthetic-ACFG helpers (copied from tests/acfg_to_petri.rs so the
// two test files stay independent of each other).
// --------------------------------------------------------------------

fn ws(ids: &[u64]) -> BTreeSet<WorkerId> {
    ids.iter().copied().map(WorkerId).collect()
}

fn op_node(workers: &[u64], kernel: u64, data_in: Vec<u64>, data_out: Option<u64>) -> ACFGNode {
    let kid = KernelId(kernel);
    ACFGNode::Operation(Operation {
        kernel: kid,
        workers: ws(workers),
        dataflow: DataflowDag {
            edges: vec![DataflowEdge::new(
                data_in.into_iter().map(DataId).collect(),
                kid,
                data_out.map(DataId),
            )],
        },
    })
}

fn synthetic_acfg(
    root: ACFGNode,
    name_data_pairs: &[(&str, u64)],
    name_workers_pairs: &[(&str, u64)],
) -> ACFG {
    let name_data: BTreeMap<String, DataId> = name_data_pairs
        .iter()
        .map(|(n, i)| ((*n).to_string(), DataId(*i)))
        .collect();
    let name_workers: BTreeMap<String, WorkerId> = name_workers_pairs
        .iter()
        .map(|(n, i)| ((*n).to_string(), WorkerId(*i)))
        .collect();
    ACFG {
        root,
        name_kernels: Default::default(),
        name_data,
        name_workers,
        name_iter_vars: Default::default(),
        inner_block_iter_vars: Default::default(),
        partition_worker_ranges: Default::default(),
        pipeline_depth_for_seq: std::collections::BTreeMap::new(),
        halo_widths: std::collections::BTreeMap::new(),
        reuse_widths: std::collections::BTreeMap::new(),
        partition_pairs: std::collections::BTreeMap::new(),
        grid_shape_for_outer_iv: std::collections::BTreeMap::new(),
    }
}

// --------------------------------------------------------------------
// Synthetic case 1: single worker, single Operation
// --------------------------------------------------------------------

#[test]
fn single_worker_single_op_emits_one_fire() {
    let root = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);

    let events = acfg_to_events(&acfg);

    assert_eq!(events.len(), 1, "one worker -> one EventList");
    let list = events.get(&WorkerId(0)).expect("w0 has a list");
    assert_eq!(list.len(), 1);
    match &list[0] {
        Event::Fire { kernel, tile, .. } => {
            assert_eq!(*kernel, KernelId(100));
            assert!(tile.is_empty(), "M2: tile empty for unrolled ops");
        }
        other => panic!("expected Fire, got {:?}", other),
    }
}

#[test]
fn declared_workers_appear_even_if_silent() {
    // w1 is declared in name_workers but never appears in any
    // Operation; the projection should still surface it with an
    // empty EventList so the backend can deterministically iterate.
    let root = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let events = acfg_to_events(&acfg);

    assert_eq!(events.len(), 2);
    assert_eq!(events.get(&WorkerId(0)).unwrap().len(), 1);
    assert_eq!(events.get(&WorkerId(1)).unwrap().len(), 0, "w1 silent");
}

// --------------------------------------------------------------------
// Synthetic case 2: two workers, matched Push/Wait pair
// --------------------------------------------------------------------

#[test]
fn two_worker_push_wait_pair_routes_correctly_with_matching_seq() {
    let tile = IterTile::empty();
    let policy = TransferPolicy::default();
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(42),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(42),
        policy,
    });
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        push,
        wait,
        op_node(&[1], 101, vec![0], Some(1)),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0), ("c", 1)], &[("w0", 0), ("w1", 1)]);

    let events = acfg_to_events(&acfg);

    let w0 = events.get(&WorkerId(0)).expect("w0 present");
    let w1 = events.get(&WorkerId(1)).expect("w1 present");

    // w0: Fire(100), Push(seq=42).  No Wait.
    assert_eq!(w0.len(), 2);
    match &w0[0] {
        Event::Fire { kernel, .. } => assert_eq!(*kernel, KernelId(100)),
        e => panic!("w0[0] expected Fire, got {:?}", e),
    }
    let push_seq = match &w0[1] {
        Event::Push { dst, data, seq, .. } => {
            assert_eq!(*dst, WorkerId(1));
            assert_eq!(*data, DataId(0));
            *seq
        }
        e => panic!("w0[1] expected Push, got {:?}", e),
    };

    // w1: Wait(seq=42), Fire(101).  No Push.
    assert_eq!(w1.len(), 2);
    let wait_seq = match &w1[0] {
        Event::Wait { src, data, seq, .. } => {
            assert_eq!(*src, WorkerId(0));
            assert_eq!(*data, DataId(0));
            *seq
        }
        e => panic!("w1[0] expected Wait, got {:?}", e),
    };
    match &w1[1] {
        Event::Fire { kernel, .. } => assert_eq!(*kernel, KernelId(101)),
        e => panic!("w1[1] expected Fire, got {:?}", e),
    }

    assert_eq!(push_seq, wait_seq, "Push/Wait seq tags must match");
    assert_eq!(push_seq, SeqTag(42));

    // Each worker only carries its own endpoint of the pair.
    assert!(
        !w0.iter().any(|e| matches!(e, Event::Wait { .. })),
        "w0 must not see any Wait"
    );
    assert!(
        !w1.iter().any(|e| matches!(e, Event::Push { .. })),
        "w1 must not see any Push"
    );
}

// --------------------------------------------------------------------
// Synthetic case 3: Sync barrier
// --------------------------------------------------------------------

#[test]
fn sync_barrier_emitted_on_every_participant() {
    let mut participants = BTreeSet::new();
    participants.insert(WorkerId(0));
    participants.insert(WorkerId(1));
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        ACFGNode::Sync(SyncPlaceholder {
            participants: participants.clone(),
            ..Default::default()
        }),
        op_node(&[1], 101, vec![], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let events = acfg_to_events(&acfg);

    let w0 = events.get(&WorkerId(0)).unwrap();
    let w1 = events.get(&WorkerId(1)).unwrap();

    // w0: Fire(100), Sync({w0,w1}).
    assert_eq!(w0.len(), 2);
    match &w0[1] {
        Event::Sync {
            participants: ps,
            kind,
            ..
        } => {
            assert_eq!(*ps, participants);
            assert_eq!(*kind, SyncKind::Barrier);
        }
        e => panic!("w0[1] expected Sync, got {:?}", e),
    }
    // w1: Sync({w0,w1}), Fire(101).
    assert_eq!(w1.len(), 2);
    match &w1[0] {
        Event::Sync {
            participants: ps, ..
        } => assert_eq!(*ps, participants),
        e => panic!("w1[0] expected Sync, got {:?}", e),
    }
}

// --------------------------------------------------------------------
// Synthetic case 4: Repeat preserves loop-nest structure (TASK-0159)
//
// This test previously asserted the OLD unroll behaviour
// (`repeat_unrolls_in_event_list`: range 0..3 -> 3 flat Fires). That
// behaviour was the bug TASK-0159 fixes: a backend consuming only the
// EventList could not recover the rolled `for`. Flipped (coverage
// kept, assumption corrected — same pattern as the TASK-0142 stale
// test fixes) to assert structure preservation.
// --------------------------------------------------------------------

/// Recursively collect every `Event::Fire` in a worker's list,
/// descending into `Event::Loop` bodies. The structure-preserving
/// projection nests loop bodies, so any test that wants "all Fires
/// regardless of nesting" must recurse rather than iterate the flat
/// top level (which now only sees top-level events + the `Loop`s).
fn flatten_fires(events: &[Event]) -> Vec<&Event> {
    let mut out = Vec::new();
    for ev in events {
        match ev {
            Event::Fire { .. } => out.push(ev),
            Event::Loop { body, .. } => out.extend(flatten_fires(body)),
            _ => {}
        }
    }
    out
}

/// Recursively collect EVERY leaf event (Fire/Push/Wait/Sync/…),
/// descending into `Event::Loop` bodies but NOT yielding the `Loop`
/// wrapper itself. For tests that reason about Push/Wait pairing or
/// "no Push/Wait at all" regardless of loop nesting (TASK-0159).
fn flatten_all(events: &[Event]) -> Vec<&Event> {
    let mut out = Vec::new();
    for ev in events {
        match ev {
            Event::Loop { body, .. } => out.extend(flatten_all(body)),
            other => out.push(other),
        }
    }
    out
}

#[test]
fn repeat_preserves_structure_in_event_list() {
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: nucleus_compiler::event::IterVar(7),
        range: 0..3,
        body: Box::new(body),
        block_tag: None,
    }]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);

    let events = acfg_to_events(&acfg);
    let w0 = events.get(&WorkerId(0)).unwrap();

    // NOT 3 flat Fires: exactly one rolled Loop carrying the nest.
    assert_eq!(w0.len(), 1, "range 0..3 -> one rolled Loop, not 3 Fires");
    match &w0[0] {
        Event::Loop {
            iter_var,
            range,
            body,
            block_tag,
            check_frame,
        } => {
            assert_eq!(
                *iter_var,
                nucleus_compiler::event::IterVar(7),
                "iter-var carried"
            );
            assert_eq!(*range, 0..3, "concrete loop bound carried verbatim");
            // A plain source loop (no block= directive) is NOT
            // strip-mined, so it carries no rebinding tag (TASK-0180).
            assert_eq!(*block_tag, None, "source loop must be untagged");
            // TASK-0052.02: `acfg_to_events` does NOT populate
            // `check_frame` (the join with sched_ir happens in the
            // post-projection pass `inject_check_frames`).
            assert_eq!(
                *check_frame, None,
                "acfg_to_events leaves check_frame unset"
            );
            assert_eq!(body.len(), 1, "loop body projected once, not unrolled");
            assert!(
                matches!(&body[0], Event::Fire { kernel, .. } if *kernel == KernelId(100)),
                "body holds the enclosed Fire, got {:?}",
                body[0]
            );
        }
        other => panic!("expected one Event::Loop, got {:?}", other),
    }

    // The Fire is still reachable via recursion (helper sanity).
    let fires = flatten_fires(w0);
    assert_eq!(fires.len(), 1);
}

#[test]
fn repeat_empty_range_emits_loop_with_empty_range() {
    // An empty/inverted source range yields zero replays. The body
    // projects once into the scratch map; since the body IS non-empty
    // here we DO emit a Loop, carrying the (empty) range verbatim so a
    // backend re-emits `for v in 5..5 {}` exactly (zero iterations).
    // This is faithful to the source `for` — the projection does not
    // silently drop a degenerate loop. (Contrast the OLD behaviour
    // which emitted zero events; that lost the loop entirely.)
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: nucleus_compiler::event::IterVar(0),
        range: 5..5,
        body: Box::new(body),
        block_tag: None,
    }]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0)]);

    let events = acfg_to_events(&acfg);
    let w0 = events.get(&WorkerId(0)).unwrap();
    assert_eq!(w0.len(), 1, "degenerate-range Loop is still carried");
    match &w0[0] {
        Event::Loop { range, body, .. } => {
            assert_eq!(*range, 5..5, "empty range carried verbatim (zero replays)");
            assert_eq!(body.len(), 1, "body still projected once");
        }
        other => panic!("expected Event::Loop, got {:?}", other),
    }
    // Zero Fires would actually execute (range is empty), but the
    // structure survives — the recursion still finds the body Fire.
    assert_eq!(flatten_fires(w0).len(), 1);
}

#[test]
fn repeat_worker_with_empty_body_gets_no_loop() {
    // w1 is declared but does nothing inside the loop body. The old
    // unroll added nothing for it; structure preservation must NOT
    // add an empty-bodied Loop either (a backend wants "w1 does
    // nothing here", not "w1 spins an empty loop").
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0))]);
    let root = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: nucleus_compiler::event::IterVar(0),
        range: 0..4,
        body: Box::new(body),
        block_tag: None,
    }]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let events = acfg_to_events(&acfg);
    assert_eq!(
        events.get(&WorkerId(0)).unwrap().len(),
        1,
        "w0 gets the Loop"
    );
    assert_eq!(
        events.get(&WorkerId(1)).unwrap().len(),
        0,
        "w1 contributes nothing -> no empty Loop"
    );
}

// --------------------------------------------------------------------
// `petri_to_events` wrapper agrees with `acfg_to_events`
// --------------------------------------------------------------------

#[test]
fn petri_wrapper_agrees_with_acfg_entry_point() {
    let tile = IterTile::empty();
    let policy = TransferPolicy {
        synchronous: false,
        buffer: 4,
        notify: NotifyMode::Event,
    };
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(7),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(7),
        policy,
    });
    let root = ACFGNode::Sequence(vec![
        op_node(&[0], 100, vec![], Some(0)),
        push,
        wait,
        op_node(&[1], 101, vec![0], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let net = acfg_to_net(&acfg);
    let via_acfg = acfg_to_events(&acfg);
    let via_petri = petri_to_events(&acfg, &net);
    assert_eq!(
        via_acfg, via_petri,
        "petri_to_events wrapper must agree with acfg_to_events"
    );
}

// --------------------------------------------------------------------
// Determinism on a mixed synthetic case
// --------------------------------------------------------------------

// --------------------------------------------------------------------
// TASK-0156 AC#3 — the EventList ALONE carries enough to reconstruct
// the per-firing value binding (the kernel call), WITHOUT walking the
// AlgoIR. This proves the contract; it does NOT switch the
// pthreads-sync backend over (that is TASK-0124). We demonstrate
// "enough information" by reconstructing the binding from the
// EventList and checking it against an independent AlgoIR walk.
// --------------------------------------------------------------------

/// Run the full front pipeline and return the post-injection ACFG.
/// Reuses the file-scoped `read_example` defined later in this file.
fn full_pipeline_acfg(algo_rel: &str, sched_rel: &str) -> ACFG {
    let algo =
        lower_algo(&parse_algo(&read_example(algo_rel)).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(&read_example(sched_rel)).expect("sched parse"))
        .expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    inject_transfers(&linked, acfg).expect("inject_transfers")
}

// TASK-0428 (cycle-242): PRD §8.3 invariant (2) — Push/Wait events
// form matched pairs — holds on the projected EventList for the ENTIRE
// example corpus, contrary to the prior deferral docstrings which
// claimed `transfer_inject`'s cross-scope splicing limitation left
// legitimate shipping programs (e.g. 02-split-add) with unmatched Wait
// events. That premise was written before TASK-0136 (Pass A hoist +
// Pass B cross-scope Push splice), TASK-0149, TASK-0151 and TASK-0364
// landed; those passes close the gap. This test is the empirical proof
// and a regression pin: it mirrors the driver's backend-agnostic
// pre-`acfg_to_events` pass chain (build_acfg -> block_transforms ->
// partition_{workers,rows,blocks2d} -> halo -> reuse -> inject_syncs ->
// inject_transfers) for EVERY example schedule, resolving each
// schedule's algorithm from its `schedule for "..."` directive
// (several pair with a sibling prog.<variant>.algo.nuc, not the default
// prog.algo.nuc), projects to EventLists, and asserts
// validate_event_lists (the FULL surface, including inv(2)) returns Ok.
//
// SCOPE: this is the pthreads-{sync,async} backend-agnostic chain. The
// mp-tcp-{bufsync,event,poll} / mp-uds-event backends additionally run
// `host_mediation_inject` + `host_data_relay_inject` AFTER
// inject_transfers; those passes re-route Push/Wait through host and
// are NOT exercised here. inv(2) over THAT post-mediation EventList is
// proven separately by TASK-0422.01 (cycle-243,
// `driver/tests/task0422_01_inv2_post_mediation.rs`, 220 cells, 0
// violations). With both proofs green, `validate_event_lists` (the FULL
// surface incl inv(2)) is now wired as a HARD production gate at the
// driver's final EventList-consumption point (TASK-0422, cycle-244 —
// `driver/src/main.rs` `cmd_build`, before `dispatch::dispatch_backend`).
#[test]
fn task0428_inv2_holds_for_entire_example_corpus() {
    use nucleus_compiler::{
        apply_block_transforms, apply_halo_inference_partition_aware, apply_partition_blocks2d,
        apply_partition_rows, apply_partition_workers, apply_reuse_inference,
    };
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let examples = repo_root.join("nuc-nucleus").join("examples");
    // Discover (algo, sched) pairs deterministically.
    let mut exdirs: Vec<_> = std::fs::read_dir(&examples)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    exdirs.sort();
    let mut violations: Vec<String> = Vec::new();
    let mut ok = 0usize;
    let mut errd = 0usize;
    for d in exdirs {
        let algo_path = d.join("prog.algo.nuc");
        if !algo_path.exists() {
            continue;
        }
        let schdir = d.join("schedules");
        if !schdir.is_dir() {
            continue;
        }
        let mut scheds: Vec<_> = std::fs::read_dir(&schdir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "nuc").unwrap_or(false))
            .collect();
        scheds.sort();
        for sp in scheds {
            let label = format!(
                "{}/{}",
                d.file_name().unwrap().to_string_lossy(),
                sp.file_name().unwrap().to_string_lossy()
            );
            let sched_src = std::fs::read_to_string(&sp).unwrap();
            // Resolve the algorithm from the `schedule for "..."`
            // directive (relative to the schedule dir), mirroring the
            // real harness — several schedules pair with a sibling
            // prog.<variant>.algo.nuc, NOT the default prog.algo.nuc.
            let resolved_algo = sched_src
                .lines()
                .find_map(|l| {
                    let l = l.trim();
                    let i = l.find("schedule for \"")?;
                    let rest = &l[i + "schedule for \"".len()..];
                    let j = rest.find('"')?;
                    Some(schdir.join(&rest[..j]))
                })
                .unwrap_or_else(|| algo_path.clone());
            let algo_src = std::fs::read_to_string(&resolved_algo)
                .unwrap_or_else(|_| std::fs::read_to_string(&algo_path).unwrap());
            // Run the chain; ANY error = "not validated for inv(2) here".
            let res = (|| -> Result<_, String> {
                let algo = lower_algo(&parse_algo(&algo_src).map_err(|e| format!("{e:?}"))?)
                    .map_err(|e| format!("{e:?}"))?;
                let sched = lower_sched(&parse_sched(&sched_src).map_err(|e| format!("{e:?}"))?)
                    .map_err(|e| format!("{e:?}"))?;
                let linked = link::link(algo, sched).map_err(|e| format!("{e:?}"))?;
                let acfg = build_acfg(&linked).map_err(|e| format!("{e:?}"))?;
                let acfg = apply_block_transforms(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = apply_partition_workers(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = apply_partition_rows(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = apply_partition_blocks2d(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let (acfg, _adv) = apply_halo_inference_partition_aware(&linked, acfg)
                    .map_err(|e| format!("{e:?}"))?;
                let acfg = apply_reuse_inference(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = inject_syncs(acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = inject_transfers(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                Ok(acfg_to_events(&acfg))
            })();
            match res {
                Ok(events) => match nucleus_compiler::validate_event_lists(&events) {
                    Ok(()) => ok += 1,
                    Err(errs) => violations.push(format!("{label}: {errs:?}")),
                },
                Err(e) => {
                    // A pipeline error here is NOT an inv(2) violation —
                    // it means an EARLIER pass rejected this (algo,
                    // sched) pair, so no EventList exists to validate.
                    // The current corpus has ZERO such cases once the
                    // `schedule for` algo is resolved; we assert that
                    // below so a future schedule that silently fails to
                    // lower cannot hide behind this arm.
                    errd += 1;
                    eprintln!("[TASK-0428 pipeline ERR] {label}: {e}");
                }
            }
        }
    }
    // Hard regression pin: every shipping schedule's projected EventList
    // satisfies inv(2), and every (algo, sched) pair in the corpus
    // lowers (no silent pre-projection rejection masking the question).
    assert!(
        violations.is_empty(),
        "TASK-0428: PRD §8.3 inv(2) (matched Push/Wait pairs) violated for {} schedule(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
    assert_eq!(
        errd, 0,
        "TASK-0428: {errd} schedule(s) failed to lower through the pre-projection chain; \
         the inv(2) sweep cannot validate them — resolve the algo/schedule pairing or \
         file the regression before relying on this pin"
    );
    assert!(
        ok >= 55,
        "TASK-0428: expected the full corpus (>=55 schedules) to be inv(2)-validated, got {ok}; \
         did a schedule directory move or a `schedule for` path stop resolving?"
    );
}

// TASK-0428 (cycle-242): focused pin on the historical reproducer.
// 02-split-add (producer `load_input`/`load_input_b` at top level on
// host; consumer `for i { add(a[i], b[i]) }` inside a `for` on w0) was
// the program the deferral docstrings cited as leaving an unmatched
// Wait. TASK-0136's Pass A hoists the loop-invariant input Waits out of
// the `for` (one crossing, not one per iteration) and Pass B splices
// the matching host-side Push; the result is fully matched. This test
// asserts an inv(2)-clean EventList for exactly that shape via
// `full_pipeline_acfg` (build_acfg -> inject_syncs -> inject_transfers).
// 02-split-add carries no partition/block/halo/reuse directives, so that
// shorter chain is identical to the driver's for THIS schedule (the broad
// corpus sweep above runs the full driver chain). A regression in the
// hoist/splice machinery re-opens here with a named reproducer rather
// than only in the broad corpus sweep above.
#[test]
fn task0428_inv2_clean_for_02_split_add_reproducer() {
    let acfg = full_pipeline_acfg(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    let events = acfg_to_events(&acfg);
    assert_eq!(
        nucleus_compiler::validate_event_lists(&events),
        Ok(()),
        "02-split-add was the deferral docstrings' cited unmatched-Wait \
         reproducer; it must now satisfy PRD §8.3 inv(2) (TASK-0136 et al.)"
    );
}

/// Render a (DataId, indices) slice the way a backend would read it
/// from the EventList: `name[idx0][idx1]…`, using ONLY the name
/// table (DataId -> name) and the index IrExprs carried on the
/// event. No AlgoIR statement walk.
fn render_slice_from_event(
    s: &nucleus_compiler::DataSlice,
    id_to_name: &BTreeMap<DataId, String>,
) -> String {
    let name = id_to_name.get(&s.data).expect("data id in name table");
    if s.indices.is_empty() {
        name.clone()
    } else {
        let idx: Vec<String> = s.indices.iter().map(render_ir_expr).collect();
        format!("{name}[{}]", idx.join("]["))
    }
}

fn render_ir_expr(e: &nucleus_compiler::algo::IrExpr) -> String {
    use nucleus_compiler::algo::{IrBinOp, IrExpr};
    match e {
        IrExpr::IntLit(v) => format!("{v}"),
        IrExpr::Ident(n) => n.clone(),
        IrExpr::Neg(i) => format!("-({})", render_ir_expr(i)),
        IrExpr::BinOp(op, l, r) => {
            let o = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
            };
            format!("({} {o} {})", render_ir_expr(l), render_ir_expr(r))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => {
            unreachable!("index expressions are integer-only (PRD §6.2.3)")
        }
    }
}

/// Reconstruct the `kernel(args...) -> out` call string for a Fire,
/// reading ONLY the Event's FireBinding + the DataId/KernelId name
/// tables. This is exactly the information a backend consuming the
/// EventList alone would have.
fn reconstruct_call_from_event(
    ev: &Event,
    kid_to_name: &BTreeMap<KernelId, String>,
    did_to_name: &BTreeMap<DataId, String>,
) -> String {
    use nucleus_compiler::ArgBinding;
    let (kernel, bindings) = match ev {
        Event::Fire {
            kernel, bindings, ..
        } => (kernel, bindings),
        other => panic!("expected Fire, got {other:?}"),
    };
    let kname = kid_to_name.get(kernel).expect("kernel id in name table");
    fn render_arg(a: &ArgBinding, did_to_name: &BTreeMap<DataId, String>) -> String {
        match a {
            ArgBinding::Data(s) => render_slice_from_event(s, did_to_name),
            ArgBinding::Scalar(e) => render_ir_expr(e),
            ArgBinding::Nested { callee, args } => {
                let inner: Vec<String> = args.iter().map(|x| render_arg(x, did_to_name)).collect();
                format!("{callee}({})", inner.join(", "))
            }
        }
    }
    let args: Vec<String> = bindings
        .inputs
        .iter()
        .map(|a| render_arg(a, did_to_name))
        .collect();
    let call = format!("{kname}({})", args.join(", "));
    match &bindings.output {
        Some(o) => format!("{} <-- {call}", render_slice_from_event(o, did_to_name)),
        None => call,
    }
}

#[test]
fn eventlist_alone_reconstructs_stencil_kernel_call() {
    let acfg = full_pipeline_acfg(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/naive.sched.nuc",
    );

    // Invert the ACFG name tables (the backend gets these alongside
    // the EventList — they are part of the schedule pass output, not
    // the AlgoIR).
    let did_to_name: BTreeMap<DataId, String> = acfg
        .name_data
        .iter()
        .map(|(n, id)| (*id, n.clone()))
        .collect();
    let kid_to_name: BTreeMap<KernelId, String> = acfg
        .name_kernels
        .iter()
        .map(|(n, id)| (*id, n.clone()))
        .collect();

    let events = acfg_to_events(&acfg);

    // Find a blur3 firing (the one whose binding has 9 inputs).
    let blur3_id = *acfg.name_kernels.get("blur3").expect("blur3 declared");
    let mut reconstructed = None;
    // The stencil loop nest is now structure-preserving: the blur3
    // Fire lives INSIDE one or more `Event::Loop` bodies, not at the
    // top level. Recurse so this test still proves the EventList alone
    // carries the binding (TASK-0156 contract, unchanged) under the
    // new loop-nesting (TASK-0159).
    for evs in events.values() {
        for ev in flatten_fires(evs) {
            if let Event::Fire {
                kernel, bindings, ..
            } = ev
            {
                if *kernel == blur3_id && bindings.inputs.len() == 9 {
                    reconstructed =
                        Some(reconstruct_call_from_event(ev, &kid_to_name, &did_to_name));
                    break;
                }
            }
        }
        if reconstructed.is_some() {
            break;
        }
    }

    let got = reconstructed.expect("a blur3 Fire with 9 input bindings must exist");

    // The expected string is the literal source call (PRD §6.2.3
    // snippet for example 05). If the EventList carries enough, the
    // reconstruction — built WITHOUT touching AlgoIR statements —
    // equals this.
    let expect = "img_out[y][x] <-- blur3(\
img_in[(y - 1)][(x - 1)], img_in[(y - 1)][x], img_in[(y - 1)][(x + 1)], \
img_in[y][(x - 1)], img_in[y][x], img_in[y][(x + 1)], \
img_in[(y + 1)][(x - 1)], img_in[(y + 1)][x], img_in[(y + 1)][(x + 1)])";
    assert_eq!(
        got, expect,
        "EventList FireBinding must reconstruct the stencil call verbatim"
    );
}

// --------------------------------------------------------------------
// TASK-0159 + forward-carried from TASK-0142: the trailing PARTIAL
// tile must project as TWO SIBLING `Event::Loop`s with DIFFERENT
// ranges, NOT one parameterised loop and NOT a flattened unroll.
//
// 05-stencil walks y over 1..15 (length 14). `block=4` is a
// deliberately non-divisible block, so `apply_block_transforms`
// rewrites it (TASK-0142) into a
//   Sequence[ full-tile nest (3 tiles of 4 -> the body covers
//             the first 12 rows), trailing partial tile (1 tile of
//             2 rows) ]
// of static-range Repeats with DIFFERENT inner trip counts. Because
// the EventList projection mirrors `Repeat`/`Sequence` structurally,
// that falls out as two sibling `Event::Loop`s in `host`'s list.
// --------------------------------------------------------------------

/// Like [`full_pipeline_acfg`] but with the block-transform pass in
/// the chain (driver order: build -> block -> sync -> xfer). The
/// plain `full_pipeline_acfg` skips it, which is why the existing
/// reconstruction test uses the *naive* (un-blocked) schedule.
fn blocked_pipeline_acfg(algo_rel: &str, sched_rel: &str) -> ACFG {
    let algo =
        lower_algo(&parse_algo(&read_example(algo_rel)).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(&read_example(sched_rel)).expect("sched parse"))
        .expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = nucleus_compiler::apply_block_transforms(&linked, acfg)
        .expect("block transform (non-divisible block=4 must NOT reject post-TASK-0142)");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    inject_transfers(&linked, acfg).expect("inject_transfers")
}

/// Collect, in order, every top-level (and Sequence-nested but NOT
/// Loop-nested) `Event::Loop` in a worker's list. We stop at the
/// first `Loop` level: the tile loops are the outer siblings; their
/// intra-tile loop is *inside* the body and is found by recursing
/// separately.
fn top_level_loops(events: &[Event]) -> Vec<&Event> {
    events
        .iter()
        .filter(|e| matches!(e, Event::Loop { .. }))
        .collect()
}

#[test]
fn blocked_stencil_trailing_partial_tile_is_two_sibling_loops() {
    let acfg = blocked_pipeline_acfg(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/blocked.sched.nuc",
    );
    let events = acfg_to_events(&acfg);

    // 05-stencil/blocked is single `host`.
    let host = *acfg.name_workers.get("host").expect("host worker declared");
    let list = events.get(&host).expect("host EventList");

    // No blanket unroll: the loop nest survives. Find the tile-level
    // loops (the outer siblings). There must be at least TWO with
    // DIFFERENT ranges — the full-tile nest and the trailing partial
    // tile. (There may be unrelated sibling Loops from other kernels;
    // we assert the *partial-tile* property holds among them.)
    let tile_loops = top_level_loops(list);
    assert!(
        tile_loops.len() >= 2,
        "expected the full-tile + trailing-partial-tile sibling Loops, \
         got {} top-level Loop(s): {:#?}",
        tile_loops.len(),
        tile_loops
    );

    // Collect the distinct ranges of the sibling tile loops.
    let mut ranges: Vec<std::ops::Range<i64>> = tile_loops
        .iter()
        .map(|e| match e {
            Event::Loop { range, .. } => range.clone(),
            _ => unreachable!(),
        })
        .collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    ranges.dedup();

    // The two tiles have DIFFERENT trip counts (full=4 rows, trailing
    // partial=2 rows). That MUST surface as at least two distinct
    // sibling ranges — proving the projection did NOT collapse them
    // into one parameterised loop and did NOT unroll.
    assert!(
        ranges.len() >= 2,
        "trailing partial tile must be a SIBLING Loop with a DIFFERENT \
         range, not merged into one loop; distinct sibling ranges: {ranges:?}"
    );

    // Determinism: a second projection is byte-identical.
    let again = acfg_to_events(&acfg);
    assert_eq!(events, again, "blocked projection must be deterministic");

    // The blur3 Fire is still reconstructible from the EventList alone
    // even nested two loops deep (binding contract survives blocking).
    let blur3 = *acfg.name_kernels.get("blur3").expect("blur3 declared");
    let any_blur3 = flatten_fires(list)
        .into_iter()
        .any(|e| matches!(e, Event::Fire { kernel, .. } if *kernel == blur3));
    assert!(
        any_blur3,
        "blur3 Fire must survive (nested) inside the tile loop bodies"
    );
}

#[test]
fn eventlist_carries_bindings_for_all_e2e_examples() {
    // Every Fire in every e2e example must carry a binding whose
    // input arity is plausible and whose output presence matches the
    // dataflow-vs-effect distinction (effect firings -> no output).
    for (algo, sched) in [
        (
            "01-elementwise-add/prog.algo.nuc",
            "01-elementwise-add/schedules/naive.sched.nuc",
        ),
        (
            "02-split-add/prog.algo.nuc",
            "02-split-add/schedules/split.sched.nuc",
        ),
        (
            "03-reduction/prog.algo.nuc",
            "03-reduction/schedules/naive.sched.nuc",
        ),
        (
            "05-stencil/prog.algo.nuc",
            "05-stencil/schedules/naive.sched.nuc",
        ),
        (
            "07-matmul/prog.algo.nuc",
            "07-matmul/schedules/naive.sched.nuc",
        ),
    ] {
        let acfg = full_pipeline_acfg(algo, sched);
        let events = acfg_to_events(&acfg);
        let mut saw_fire = false;
        for evs in events.values() {
            // Recurse into Loop bodies (TASK-0159): iterated Fires now
            // nest inside `Event::Loop`, so a flat top-level walk would
            // miss them and falsely "see no Fire".
            for ev in flatten_fires(evs) {
                if let Event::Fire { bindings, .. } = ev {
                    saw_fire = true;
                    // Each input is either a data slice or a scalar;
                    // a zero-arg loader has zero inputs and an
                    // output; that is allowed. The contract we pin:
                    // the binding is *present* (not the default
                    // empty) for any firing that reads or writes
                    // data.
                    let has_value = !bindings.inputs.is_empty() || bindings.output.is_some();
                    assert!(
                        has_value,
                        "{algo}: a Fire carried an empty binding — \
                         EventList would not be self-sufficient"
                    );
                }
            }
        }
        assert!(saw_fire, "{algo}: expected at least one Fire");
    }
}

#[test]
fn determinism_two_projections_of_same_acfg_match() {
    let tile = IterTile::empty();
    let policy = TransferPolicy::default();
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: tile.clone(),
        seq: SeqTag(0),
        policy,
    });
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile,
        seq: SeqTag(0),
        policy,
    });
    let mut participants = BTreeSet::new();
    participants.insert(WorkerId(0));
    participants.insert(WorkerId(1));
    let body = ACFGNode::Sequence(vec![op_node(&[0], 100, vec![], Some(0)), push, wait]);
    let root = ACFGNode::Sequence(vec![
        ACFGNode::Repeat {
            iter_var: nucleus_compiler::event::IterVar(0),
            range: 0..2,
            body: Box::new(body),
            block_tag: None,
        },
        ACFGNode::Sync(SyncPlaceholder {
            participants,
            ..Default::default()
        }),
        op_node(&[1], 101, vec![0], None),
    ]);
    let acfg = synthetic_acfg(root, &[("d", 0)], &[("w0", 0), ("w1", 1)]);

    let a = acfg_to_events(&acfg);
    let b = acfg_to_events(&acfg);
    assert_eq!(a, b, "projection must be deterministic");
}

// --------------------------------------------------------------------
// End-to-end: example 02 (split) projected to EventLists
// --------------------------------------------------------------------

fn read_example(relpath: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let full = repo_root.join("nuc-nucleus").join("examples").join(relpath);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", full.display(), e))
}

fn pipeline_to_events(
    algo_rel: &str,
    sched_rel: &str,
) -> (BTreeMap<WorkerId, Vec<Event>>, BTreeMap<String, WorkerId>) {
    let algo_ast = parse_algo(&read_example(algo_rel)).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(sched_rel)).expect("sched parse");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    let name_workers = acfg.name_workers.clone();
    // Route through `petri_to_events` so the wrapper is exercised on
    // a real input too.
    let net = acfg_to_net(&acfg);
    (petri_to_events(&acfg, &net), name_workers)
}

#[test]
fn e2e_example_02_split_one_eventlist_per_declared_worker() {
    let (events, names) = pipeline_to_events(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );

    // Schedule names exactly two workers: host, w0.
    assert_eq!(
        names.len(),
        2,
        "split.sched.nuc declares two workers; got {:?}",
        names.keys().collect::<Vec<_>>()
    );
    assert_eq!(events.len(), names.len(), "one EventList per worker");
    for wid in names.values() {
        assert!(events.contains_key(wid), "missing EventList for {:?}", wid);
    }

    // Both workers do *something*: host loads + saves, w0 fires the add.
    for (wid, list) in &events {
        assert!(
            !list.is_empty(),
            "worker {:?} has empty EventList in split schedule",
            wid
        );
    }

    // Every Push on some worker has a matching Wait on the
    // corresponding destination worker carrying the same seq + data.
    //
    // TASK-0136 / TASK-0139: `transfer_inject` now splices Pushes
    // across Sequence/Repeat scope boundaries (Pass A whole-symbol
    // hoist + Pass B global Push finaliser). For example 02 the
    // producer `load_input` lives at the top-level sequence while the
    // consumer `add` lives inside a `for` loop; the cross-scope
    // finaliser pairs them. We therefore now assert the *strong*
    // property: at least one Push is present, and every Push has a
    // matching Wait on its declared dst.
    // Recurse into Loop bodies: the consumer `add` lives inside a
    // `for`, so its Wait (and any spliced Push) now nests in an
    // `Event::Loop` (TASK-0159). A flat walk would miss them.
    let mut pushes: Vec<(WorkerId, &Event)> = Vec::new();
    let mut waits: Vec<(WorkerId, &Event)> = Vec::new();
    for (wid, list) in &events {
        for ev in flatten_all(list) {
            match ev {
                Event::Push { .. } => pushes.push((*wid, ev)),
                Event::Wait { .. } => waits.push((*wid, ev)),
                _ => {}
            }
        }
    }
    // Waits should at least be present (consumer side of the split).
    assert!(
        !waits.is_empty(),
        "split schedule consumer should produce at least one Wait"
    );
    // TASK-0136 AC#2: Pushes must now be present (producer side of
    // the split — the cross-scope finaliser pairs every Wait).
    assert!(
        !pushes.is_empty(),
        "split schedule must produce at least one Push after \
         TASK-0136/0139 cross-scope finalisation"
    );
    for (push_owner, push) in &pushes {
        let (push_dst, push_data, push_seq) = match push {
            Event::Push { dst, data, seq, .. } => (*dst, *data, *seq),
            _ => unreachable!(),
        };
        let mate = waits.iter().find(|(wait_owner, wait)| {
            *wait_owner == push_dst
                && match wait {
                    Event::Wait { src, data, seq, .. } => {
                        *src == *push_owner && *data == push_data && *seq == push_seq
                    }
                    _ => false,
                }
        });
        assert!(
            mate.is_some(),
            "Push on {:?} (data={:?} seq={:?}) has no matching Wait on {:?}",
            push_owner,
            push_data,
            push_seq,
            push_dst
        );
    }
}

#[test]
fn e2e_example_02_split_determinism() {
    let (a, _) = pipeline_to_events(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    let (b, _) = pipeline_to_events(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    assert_eq!(
        a, b,
        "two full-pipeline projections of the same input must be bit-identical"
    );
}

#[test]
fn e2e_example_01_naive_single_worker_no_transfers() {
    // Sanity check on the easier example: naive schedule, one worker
    // declared, no Push/Wait/Sync should appear in the EventLists.
    let (events, names) = pipeline_to_events(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    assert_eq!(names.len(), events.len());
    for list in events.values() {
        // Recurse into Loop bodies (TASK-0159): a stray Push/Wait
        // inside the elementwise loop must still be caught.
        for ev in flatten_all(list) {
            assert!(
                !matches!(ev, Event::Push { .. } | Event::Wait { .. }),
                "single-worker schedule must produce no Push/Wait events; got {:?}",
                ev
            );
        }
    }
}

// --------------------------------------------------------------------
// TASK-0160 — the NameSidecar (+ EventList + name tables) ALONE
// carries enough to: size the pre-init allocation `vec![<zero>; N]`,
// pick the Rust element/slot type, and render a rolled loop's bound
// in SOURCE form (e.g. `(16_i64 - 1_i64)`), WITHOUT walking the
// AlgoIR. This proves the codegen contract is sufficient; it does
// NOT switch the pthreads-sync backend (that is TASK-0124). Mirrors
// TASK-0156's `eventlist_alone_reconstructs_stencil_kernel_call`
// pattern: reconstruct from the contract, assert == the literal the
// AlgoIR-walking backend emits today.
// --------------------------------------------------------------------

use nucleus_compiler::sidecar::{build_sidecar, ConstValue, NameSidecar};

/// Like `full_pipeline_acfg` but also returns the `LinkedIR`, which
/// `build_sidecar` needs (it reads `linked.algo` for the unevaluated
/// `for` bounds + consts that `build_acfg` folds away).
fn full_pipeline_with_linked(algo_rel: &str, sched_rel: &str) -> (link::LinkedIR, ACFG) {
    let algo =
        lower_algo(&parse_algo(&read_example(algo_rel)).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(&read_example(sched_rel)).expect("sched parse"))
        .expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    (linked, acfg)
}

/// The Rust element type for a scalar, exactly as the pthreads-sync
/// backend's `rust_scalar_type` spells it. Reproduced here from the
/// SIDECAR's `ScalarType` (NOT the AlgoIR) — this is the join a
/// TASK-0124 backend performs.
fn elem_type_from_sidecar(s: &nucleus_compiler::algo::ScalarType) -> &'static str {
    use nucleus_compiler::algo::ScalarType::*;
    match s {
        Usize => "usize",
        Isize => "isize",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        F32 => "f32",
        F64 => "f64",
        Bool => "bool",
    }
}

/// The Rust "zero" literal for a scalar, exactly as the backend's
/// `rust_scalar_zero` spells it.
fn zero_lit_from_sidecar(s: &nucleus_compiler::algo::ScalarType) -> &'static str {
    use nucleus_compiler::algo::ScalarType::*;
    match s {
        F32 | F64 => "0.0",
        Bool => "false",
        _ => "0",
    }
}

/// Render a loop-bound `IrExpr` into the SAME source string the
/// pthreads-sync `render_const_expr` produces — but resolving const
/// idents through the SIDECAR's const table, NOT `ctx.algo.consts`.
/// This is exactly what a TASK-0124 EventList-only backend would do.
///
/// DRIFT NOTE (review finding (7), reconciled by TASK-0124): this is
/// a HAND-MIRROR of `pthreads-sync::render_const_expr` (the
/// `compiler` crate cannot depend on the backend, so it cannot call
/// the real fn). Since TASK-0124 the backend ACTUALLY consumes the
/// sidecar, so the real spelling is now pinned independently by
/// `pthreads-sync/tests/emit.rs::
/// golden_real_codegen_strings_pin_sidecar_consumption`, which
/// asserts the exact emitted `for y in (1_i64)..((16_i64 - 1_i64))`
/// / `vec![0; 256]` against REAL `emit()` output. If this mirror
/// ever drifts from the backend, that golden test fails loudly — the
/// mirror can no longer silently agree-while-both-wrong.
fn render_bound_from_sidecar(
    e: &nucleus_compiler::algo::IrExpr,
    consts: &BTreeMap<String, ConstValue>,
) -> String {
    use nucleus_compiler::algo::{IrBinOp, IrExpr};
    match e {
        IrExpr::IntLit(v) => format!("{v}_i64"),
        IrExpr::Ident(n) => match consts.get(n) {
            Some(c) => format!("{}_i64", c.value),
            // An outer loop var: render as-is (mirrors the backend).
            None => n.clone(),
        },
        IrExpr::Neg(i) => format!("-({})", render_bound_from_sidecar(i, consts)),
        IrExpr::BinOp(op, l, r) => {
            let o = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
            };
            format!(
                "({} {o} {})",
                render_bound_from_sidecar(l, consts),
                render_bound_from_sidecar(r, consts)
            )
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => {
            unreachable!("loop bounds are integer-only (PRD §6.2.3)")
        }
    }
}

#[test]
fn sidecar_alone_sizes_preinit_and_types_slots_for_all_e2e_examples() {
    // For each example the backend pre-inits indexed-LHS data with
    // `vec![<zero>; product(dims)]` and types it `Vec<elem>`. Prove
    // the SIDECAR alone yields the exact length + element type the
    // AlgoIR-walking backend computes today.
    //
    // (data symbol, expected vec! length, expected elem type)
    let expectations: &[SidecarExpectation] = &[
        (
            "01-elementwise-add/prog.algo.nuc",
            "01-elementwise-add/schedules/naive.sched.nuc",
            &[("a", 256, "i32"), ("b", 256, "i32"), ("c", 256, "i32")],
        ),
        (
            "02-split-add/prog.algo.nuc",
            "02-split-add/schedules/split.sched.nuc",
            &[("a", 256, "i32"), ("b", 256, "i32"), ("c", 256, "i32")],
        ),
        (
            "03-reduction/prog.algo.nuc",
            "03-reduction/schedules/naive.sched.nuc",
            &[],
        ),
        (
            "05-stencil/prog.algo.nuc",
            "05-stencil/schedules/naive.sched.nuc",
            // i32[H][W] = i32[16][16] -> product = 256.
            &[("img_in", 256, "i32"), ("img_out", 256, "i32")],
        ),
        (
            "07-matmul/prog.algo.nuc",
            "07-matmul/schedules/naive.sched.nuc",
            &[],
        ),
    ];

    for (algo, sched, data_checks) in expectations {
        let (linked, acfg) = full_pipeline_with_linked(algo, sched);
        let sidecar = build_sidecar(&linked, &acfg)
            .expect("build_sidecar: e2e examples reuse no loop var with differing bounds");

        // The name table the backend also receives (DataId -> name)
        // — same join key as the EventList's DataSlice.data.
        let did_of = |name: &str| *acfg.name_data.get(name).expect("data declared");

        for (dname, want_len, want_elem) in *data_checks {
            let did = did_of(dname);
            // Length: from the sidecar's dims product. NO AlgoIR.
            let got_len = sidecar
                .alloc_len(did)
                .expect("sidecar carries this DataId's type");
            assert_eq!(
                got_len, *want_len,
                "{algo}: vec! length for `{dname}` from sidecar"
            );
            // Element type: from the sidecar's ScalarType. NO AlgoIR.
            let ty = sidecar.data_type(did).expect("sidecar carries type");
            assert_eq!(
                elem_type_from_sidecar(&ty.scalar),
                *want_elem,
                "{algo}: element type for `{dname}` from sidecar"
            );
            // The full pre-init line a TASK-0124 backend emits,
            // reconstructed from the sidecar ALONE, must equal what
            // the AlgoIR-walking backend's `render_array_init`
            // produces today.
            let zero = zero_lit_from_sidecar(&ty.scalar);
            let reconstructed = format!("vec![{zero}; {got_len}]");
            assert_eq!(
                reconstructed,
                format!("vec![0; {want_len}]"),
                "{algo}: reconstructed pre-init for `{dname}`"
            );
        }

        // Determinism: a second sidecar build is byte-identical.
        assert_eq!(
            sidecar,
            build_sidecar(&linked, &acfg)
                .expect("build_sidecar: deterministic, no same-name-diff-bounds loop"),
            "{algo}: build_sidecar must be deterministic"
        );
    }
}

#[test]
fn sidecar_renders_stencil_symbolic_loop_bound_in_source_form() {
    // The crux of AC#2. 05-stencil's `for y : 1 .. H-1` (H=16) is
    // folded by build_acfg to a concrete `1..15` — the AlgoIR-walking
    // backend instead emits `for y in (1_i64)..((16_i64 - 1_i64))`
    // from the SOURCE bounds. Prove: from `Event::Loop`'s iter_var +
    // the SIDECAR's loop_bounds + consts ALONE (no AlgoIR), a backend
    // reconstructs that exact source-form bound.
    let (linked, acfg) = full_pipeline_with_linked(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/naive.sched.nuc",
    );
    let sidecar = build_sidecar(&linked, &acfg)
        .expect("build_sidecar: e2e examples reuse no loop var with differing bounds");
    let events = acfg_to_events(&acfg);

    // Find the OUTER stencil loop in the EventList (the `y` loop). It
    // is the top-level Event::Loop in the single host worker's list.
    let host = *acfg.name_workers.get("host").expect("host worker");
    let list = events.get(&host).expect("host EventList");
    let outer = list
        .iter()
        .find_map(|e| match e {
            Event::Loop {
                iter_var, range, ..
            } => Some((*iter_var, range.clone())),
            _ => None,
        })
        .expect("a top-level Event::Loop (the y loop) must exist");
    let (iter_var, concrete_range) = outer;

    // Sanity: the EventList alone only gives the FOLDED range.
    assert_eq!(
        concrete_range,
        1..15,
        "Event::Loop carries the concrete folded range (TASK-0159)"
    );

    // The sidecar pairs the SAME IterVar to the unevaluated source
    // bounds. This is the join TASK-0124 performs.
    let bound = sidecar
        .loop_bounds
        .get(&iter_var)
        .expect("sidecar carries the symbolic bound for this loop var");

    let lo_s = render_bound_from_sidecar(&bound.lo, &sidecar.consts);
    let hi_s = render_bound_from_sidecar(&bound.hi, &sidecar.consts);
    // Exactly the string pthreads-sync emits today (lib.rs ~538:
    // `for {var} in ({lo_s})..({hi_s})`), but built from the
    // CONTRACT, not the AlgoIR.
    assert_eq!(lo_s, "1_i64", "lower bound source form from sidecar");
    assert_eq!(
        hi_s, "(16_i64 - 1_i64)",
        "upper bound `H-1` (H=16) re-rendered in SOURCE form from \
         sidecar.loop_bounds + sidecar.consts — NOT the folded 15"
    );

    // And the inner `x` loop (`1 .. W-1`, W=16) likewise. Collect
    // every Event::Loop's iter_var recursively (flatten_all only
    // yields non-Loop leaves, never the Loop nodes themselves).
    fn collect_loop_vars(events: &[Event], out: &mut Vec<nucleus_compiler::event::IterVar>) {
        for e in events {
            if let Event::Loop { iter_var, body, .. } = e {
                out.push(*iter_var);
                collect_loop_vars(body, out);
            }
        }
    }
    let mut loop_vars = Vec::new();
    collect_loop_vars(list, &mut loop_vars);
    let inner = loop_vars
        .into_iter()
        .find(|iv| *iv != iter_var)
        .expect("an inner Event::Loop (the x loop)");
    let inner_bound = sidecar.loop_bounds.get(&inner).expect("inner bound");
    assert_eq!(
        render_bound_from_sidecar(&inner_bound.hi, &sidecar.consts),
        "(16_i64 - 1_i64)",
        "inner `W-1` (W=16) also re-renders to source form from sidecar"
    );
}

#[test]
fn sidecar_const_table_matches_resolved_consts() {
    // The sidecar const table must carry every AlgoIR const verbatim
    // (value + scalar type) so the backend never reaches for
    // `algo.consts`.
    let (linked, acfg) = full_pipeline_with_linked(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let sidecar = build_sidecar(&linked, &acfg)
        .expect("build_sidecar: e2e examples reuse no loop var with differing bounds");
    assert_eq!(
        sidecar.consts.get("N"),
        Some(&ConstValue {
            ty: nucleus_compiler::algo::ScalarType::Usize,
            value: 256,
        }),
        "const N=256:usize must reach the sidecar"
    );
    // Every AlgoIR const is present with the same value/type.
    for (name, rc) in &linked.algo.consts {
        let cv = sidecar.consts.get(name).expect("const in sidecar");
        assert_eq!(cv.value, rc.value, "const `{name}` value");
        assert_eq!(cv.ty, rc.ty, "const `{name}` scalar type");
    }
}

#[cfg(feature = "serde")]
#[test]
fn sidecar_serde_roundtrip_is_byte_identical() {
    // The sidecar is a committable codegen artifact (like the
    // contract types) — JSON roundtrip must be lossless and stable.
    let (linked, acfg) = full_pipeline_with_linked(
        "05-stencil/prog.algo.nuc",
        "05-stencil/schedules/naive.sched.nuc",
    );
    let sidecar = build_sidecar(&linked, &acfg)
        .expect("build_sidecar: e2e examples reuse no loop var with differing bounds");
    let json = serde_json::to_string(&sidecar).expect("serialize");
    let back: NameSidecar = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(sidecar, back, "sidecar serde roundtrip must be lossless");
    let json2 = serde_json::to_string(&back).expect("reserialize");
    assert_eq!(json, json2, "serde wire form must be stable");
}

// --------------------------------------------------------------------
// TASK-0169 — the NameSidecar's `kernel_sigs` table (+ Event::Fire's
// `kernel` KernelId + `bindings`) ALONE carries enough to reproduce
// the shared `render_fire_arg`'s scalar-argument cast decision
// (`(expr) as usize` when the kernel param is scalar), WITHOUT
// walking `ctx.algo.kernels`. This closes the last AlgoIR read in
// pthreads-sync codegen — the contract is now fully AlgoIR-free.
//
// RESOLVED FINDING (forward-carried to TASK-0124): NONE of the e2e
// set 01/02/03/05/07 feeds an `ArgBinding::Scalar` (iter-var/const
// arithmetic) to a scalar kernel param — every kernel argument in
// those 5 is an `ArgBinding::Data` element/whole-array read. So
// `render_fire_arg`'s `param_ty.is_scalar()` *cast* branch is reached
// only from the `IrExpr::IntLit|Ident|Neg|BinOp` arm, which the e2e
// set never hits with a scalar param. The two assertions below prove
// (a) that finding holds across all 5, and (b) that the cast is
// nevertheless faithfully reconstructible from `kernel_sigs` alone
// via a synthetic scalar-param kernel + a synthetic `Scalar` arg.
// --------------------------------------------------------------------

use nucleus_compiler::sidecar::KernelSig;
use nucleus_compiler::ArgBinding;

/// Faithful reproduction of pthreads-sync `render_int_expr`
/// (lib.rs ~708): how a *scalar* (`ArgBinding::Scalar`) argument's
/// integer expression is spelled before the cast is applied.
fn render_int_expr_mirror(e: &nucleus_compiler::algo::IrExpr) -> String {
    use nucleus_compiler::algo::{IrBinOp, IrExpr};
    match e {
        IrExpr::IntLit(v) => format!("{v}"),
        IrExpr::Ident(n) => n.clone(),
        IrExpr::Neg(i) => format!("-({})", render_int_expr_mirror(i)),
        IrExpr::BinOp(op, l, r) => {
            let o = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
            };
            format!(
                "({} {o} {})",
                render_int_expr_mirror(l),
                render_int_expr_mirror(r)
            )
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => {
            unreachable!("a Scalar ArgBinding never contains a DataRef/Call")
        }
    }
}

/// Reproduce the shared `render_fire_arg` for ONE scalar
/// (`ArgBinding::Scalar`) argument, sourcing the parameter type from
/// the SIDECAR's `kernel_sigs[kid].params[i]` joined via the same
/// `KernelId` `Event::Fire` carries — NEVER `ctx.algo.kernels`. This
/// is exactly the join a TASK-0124 EventList-only backend performs.
fn render_scalar_arg_from_sidecar(
    arg_expr: &nucleus_compiler::algo::IrExpr,
    param_ty: Option<&nucleus_compiler::algo::ResolvedType>,
) -> String {
    let rendered = render_int_expr_mirror(arg_expr);
    // The exact branch from backend-common's render/fire.rs
    // `render_fire_arg` scalar-cast arm:
    if let Some(pty) = param_ty {
        if pty.is_scalar() {
            return format!("({rendered}) as {}", elem_type_from_sidecar(&pty.scalar));
        }
    }
    rendered
}

#[test]
fn sidecar_kernel_sigs_match_algoir_for_all_e2e_examples() {
    // `kernel_sigs` must carry every AlgoIR kernel's params + ret
    // verbatim, keyed by the SAME KernelId Event::Fire carries
    // (acfg.name_kernels), so a backend never reaches for
    // `algo.kernels`. Also records the RESOLVED FINDING: across the
    // e2e set, no Fire feeds a Scalar arg to a scalar param.
    let examples: &[(&str, &str)] = &[
        (
            "01-elementwise-add/prog.algo.nuc",
            "01-elementwise-add/schedules/naive.sched.nuc",
        ),
        (
            "02-split-add/prog.algo.nuc",
            "02-split-add/schedules/split.sched.nuc",
        ),
        (
            "03-reduction/prog.algo.nuc",
            "03-reduction/schedules/naive.sched.nuc",
        ),
        (
            "05-stencil/prog.algo.nuc",
            "05-stencil/schedules/naive.sched.nuc",
        ),
        (
            "07-matmul/prog.algo.nuc",
            "07-matmul/schedules/naive.sched.nuc",
        ),
    ];

    for (algo, sched) in examples {
        let (linked, acfg) = full_pipeline_with_linked(algo, sched);
        let sidecar = build_sidecar(&linked, &acfg)
            .expect("build_sidecar: e2e examples reuse no loop var with differing bounds");

        // Every AlgoIR kernel reachable via the canonical KernelId
        // must be in kernel_sigs with identical params + ret.
        for (name, kid) in &acfg.name_kernels {
            let rk = linked
                .algo
                .kernels
                .get(name)
                .expect("kernel declared in AlgoIR");
            let sig = sidecar
                .kernel_sig(*kid)
                .unwrap_or_else(|| panic!("{algo}: kernel `{name}` missing from kernel_sigs"));
            assert_eq!(
                sig.params, rk.params,
                "{algo}: `{name}` params must match AlgoIR verbatim"
            );
            assert_eq!(
                sig.ret, rk.ret,
                "{algo}: `{name}` ret must match AlgoIR verbatim"
            );
        }

        // RESOLVED FINDING: walk every Event::Fire's bindings; assert
        // no Scalar arg lands on a scalar param in this example. This
        // is what makes TASK-0124 byte-identical for these 5 WITHOUT
        // depending on kernel_sigs at runtime — yet the contract is
        // only fully AlgoIR-free WITH it (proved by the next test).
        fn walk(events: &[Event], sidecar: &NameSidecar, algo: &str) {
            for e in events {
                match e {
                    Event::Fire {
                        kernel, bindings, ..
                    } => {
                        if let Some(sig) = sidecar.kernel_sig(*kernel) {
                            for (i, ab) in bindings.inputs.iter().enumerate() {
                                if let ArgBinding::Scalar(_) = ab {
                                    let is_scalar_param =
                                        sig.params.get(i).map(|p| p.is_scalar()).unwrap_or(false);
                                    assert!(
                                        !is_scalar_param,
                                        "{algo}: UNEXPECTED Scalar arg #{i} fed to a \
                                         scalar param — the e2e set was believed to \
                                         have NO such call. Update the TASK-0169 / \
                                         TASK-0124 finding."
                                    );
                                }
                            }
                        }
                        // FireBinding.inputs is flat (Nested args are
                        // rejected by the shared render_fire_arg
                        // anyway), so no recursion into Nested needed.
                    }
                    Event::Loop { body, .. } => walk(body, sidecar, algo),
                    _ => {}
                }
            }
        }
        let events = acfg_to_events(&acfg);
        for list in events.values() {
            walk(list, &sidecar, algo);
        }

        // Determinism: second build byte-identical (covers kernel_sigs).
        assert_eq!(
            sidecar,
            build_sidecar(&linked, &acfg)
                .expect("build_sidecar: deterministic, no same-name-diff-bounds loop"),
            "{algo}: build_sidecar (incl. kernel_sigs) must be deterministic"
        );
    }
}

#[test]
fn sidecar_alone_reconstructs_scalar_arg_cast_no_algoir_walk() {
    // The crux of AC#3. Since (resolved finding above) no e2e example
    // feeds a Scalar arg to a scalar param, we prove the cast is
    // reconstructible from `kernel_sigs` ALONE with a SYNTHETIC
    // scalar-param kernel signature + a SYNTHETIC `Scalar` arg —
    // mirroring exactly what TASK-0124's EventList-only backend will
    // do for a program that DOES (e.g. a future `shift(x, n)` kernel
    // called `shift(buf[i], i + 1)`).
    use nucleus_compiler::algo::{IrBinOp, IrExpr, ResolvedType, ScalarType};

    // Synthetic kernel: `dilate : (i32[256], usize) -> i32` — arg #0
    // is a whole-array param (NOT scalar -> no cast), arg #1 is a
    // scalar `usize` param (scalar -> cast). Built as a KernelSig
    // exactly as build_sidecar copies it out of a ResolvedKernel.
    let kid = KernelId(99);
    let mut kernel_sigs: BTreeMap<KernelId, KernelSig> = BTreeMap::new();
    kernel_sigs.insert(
        kid,
        KernelSig {
            params: vec![
                ResolvedType {
                    scalar: ScalarType::I32,
                    dims: vec![256],
                }, // aggregate param: is_scalar() == false
                ResolvedType {
                    scalar: ScalarType::Usize,
                    dims: vec![],
                }, // scalar param: is_scalar() == true
            ],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
        },
    );
    let sidecar = NameSidecar {
        kernel_sigs,
        ..Default::default()
    };

    // The Fire carries the KernelId; the backend joins it to the
    // sidecar with NO AlgoIR. (We do not even construct an AlgoIR.)
    let sig = sidecar
        .kernel_sig(kid)
        .expect("kernel_sigs join via Event::Fire.kernel KernelId");

    // Arg #1: scalar expression `i + 1` (an iter-var-derived value) —
    // the shape `render_fire_arg` casts. Reconstruct purely from the
    // sidecar's param type.
    let scalar_arg = IrExpr::BinOp(
        IrBinOp::Add,
        Box::new(IrExpr::Ident("i".into())),
        Box::new(IrExpr::IntLit(1)),
    );
    let rendered_scalar = render_scalar_arg_from_sidecar(&scalar_arg, sig.params.get(1));
    // Exactly what pthreads-sync emits today: render_int_expr already
    // parenthesises a BinOp (`(i + 1)`), then render_fire_arg's cast
    // wraps the whole thing again (`(<rendered>) as usize`) — hence
    // the double parens. We assert the REAL backend string, not a
    // tidied one (the contract must reproduce it byte-for-byte).
    assert_eq!(
        rendered_scalar, "((i + 1)) as usize",
        "scalar arg fed to a `usize` param must cast — reconstructed \
         from NameSidecar.kernel_sigs ALONE, no ctx.algo.kernels walk"
    );

    // Arg #0's param is aggregate (i32[256]) — render_fire_arg's
    // DataRef arm emits the bare name and NEVER the scalar-cast
    // branch. A scalar expr against that aggregate param #0 must NOT
    // cast — proving the dispatch decision is param-type-driven, read
    // from the sidecar alone.
    assert!(
        !sig.params[0].is_scalar(),
        "arg #0's param is aggregate (i32[256]) — no scalar cast"
    );
    let no_cast = render_scalar_arg_from_sidecar(&IrExpr::IntLit(7), sig.params.first());
    assert_eq!(
        no_cast, "7",
        "scalar expr against an aggregate param is NOT cast — the \
         dispatch decision is driven by kernel_sigs param type alone"
    );
}

// --------------------------------------------------------------------
// TASK-0170: same-name-loop differing-bounds is a TYPED error, never a
// bare panic, on VALID Nuc input.
//
// Reachability finding (recorded in TASK-0170): two sequential sibling
// loops reusing one variable name with DIFFERENT bounds, writing
// DISTINCT data arrays so single-assignment holds, is accepted by
// parse_algo + lower_algo + link + build_acfg. `ACFG::name_iter_vars`
// then collapses both onto ONE `IterVar`, so `build_sidecar` cannot
// represent two bound pairs. This pins that it returns
// `SidecarError::SameNameLoopBoundConflict` (clean, driver-surfacable)
// rather than `panic!`-ing — which is exactly what makes the
// TASK-0124 EventList-only path panic-safe (AC#3). The proper
// distinct-identity fix that would let this COMPILE is TASK-0171; when
// that lands, the negative test below flips to expect success.
// --------------------------------------------------------------------

/// Lower an inline algo + an inline single-`host` schedule and link
/// them, returning the `(LinkedIR, ACFG)` `build_sidecar` consumes.
/// Inline (not a fixture file) so the witness program lives next to
/// the assertion that depends on it.
fn linked_acfg_from_src(algo_src: &str, sched_src: &str) -> (link::LinkedIR, ACFG) {
    let algo = lower_algo(&parse_algo(algo_src).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(sched_src).expect("sched parse")).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    (linked, acfg)
}

const SAME_NAME_SCHED: &str = r#"
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input  on host;
    place id          on host;
    place save_output on host;
}
"#;

#[test]
fn sidecar_same_name_loop_differing_bounds_is_typed_error_not_panic() {
    // Two sibling `for i` loops, bounds 0..N vs 0..M (N != M), each
    // writing a DISTINCT array (single-assignment holds). This is a
    // valid Nuc program (parse/lower/link/build_acfg all accept it).
    let algo = r#"
const N : usize = 8;
const M : usize = 4;

data a : i32[N];
data c : i32[N];
data d : i32[N];

kernel load_input  : ()       -> i32[N] effectful;
kernel id          : (i32)    -> i32    pure;
kernel save_output : (i32[N])  -> ()     effectful;

a <-- load_input();

for i : 0 .. N { c[i] <-- id(a[i]); }
for i : 0 .. M { d[i] <-- id(a[i]); }

save_output(c);
save_output(d);
"#;

    let (linked, acfg) = linked_acfg_from_src(algo, SAME_NAME_SCHED);

    // The reachability invariant this test pins: ONE IterVar for the
    // reused name `i` (if this ever changes — e.g. TASK-0171 lands —
    // this assertion fails first, signalling the contract moved).
    assert_eq!(
        acfg.name_iter_vars.len(),
        1,
        "precondition: reused loop name `i` collapses to one IterVar \
         (the TASK-0170 root cause); if this changed see TASK-0171"
    );

    match build_sidecar(&linked, &acfg) {
        Err(nucleus_compiler::sidecar::SidecarError::SameNameLoopBoundConflict {
            var,
            first,
            second,
        }) => {
            assert_eq!(var, "i");
            // First occurrence (0..N) kept; second (0..M) is the
            // conflicting one — verbatim source exprs, fail-fast +
            // verbose.
            assert_eq!(first.lo, nucleus_compiler::algo::IrExpr::IntLit(0));
            assert_eq!(first.hi, nucleus_compiler::algo::IrExpr::Ident("N".into()));
            assert_eq!(second.lo, nucleus_compiler::algo::IrExpr::IntLit(0));
            assert_eq!(second.hi, nucleus_compiler::algo::IrExpr::Ident("M".into()));
            // The Display string carries the actionable diagnostic.
            let msg = nucleus_compiler::sidecar::SidecarError::SameNameLoopBoundConflict {
                var,
                first,
                second,
            }
            .to_string();
            assert!(msg.contains("loop variable `i`"), "msg: {msg}");
            assert!(msg.contains("DIFFERENT bounds"), "msg: {msg}");
        }
        Ok(_) => panic!(
            "build_sidecar must reject same-name-differing-bounds with \
             a typed error, not silently succeed (would drop one \
             loop's bounds for the TASK-0124 EventList backend)"
        ),
    }
}

#[test]
fn sidecar_same_name_loop_identical_bounds_is_idempotent_ok() {
    // Same reused name `i`, but IDENTICAL bounds (0..N both): the
    // shared IterVar CAN represent this, so it must succeed (not a
    // false positive) — pins the idempotent branch.
    let algo = r#"
const N : usize = 8;

data a : i32[N];
data c : i32[N];
data d : i32[N];

kernel load_input  : ()       -> i32[N] effectful;
kernel id          : (i32)    -> i32    pure;
kernel save_output : (i32[N])  -> ()     effectful;

a <-- load_input();

for i : 0 .. N { c[i] <-- id(a[i]); }
for i : 0 .. N { d[i] <-- id(a[i]); }

save_output(c);
save_output(d);
"#;

    let (linked, acfg) = linked_acfg_from_src(algo, SAME_NAME_SCHED);
    let sidecar = build_sidecar(&linked, &acfg)
        .expect("identical bounds: shared IterVar represents both, must be Ok");
    // One IterVar -> exactly one loop_bounds entry, the shared 0..N.
    assert_eq!(sidecar.loop_bounds.len(), 1);
    let lb = sidecar.loop_bounds.values().next().unwrap();
    assert_eq!(lb.lo, nucleus_compiler::algo::IrExpr::IntLit(0));
    assert_eq!(lb.hi, nucleus_compiler::algo::IrExpr::Ident("N".into()));
}
