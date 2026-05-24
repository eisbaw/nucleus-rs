//! Tests for `NameSidecar::transfer_buffer_for_seq` (TASK-0233).
//!
//! The new sidecar field carries per-SeqTag buffer sizes from the
//! schedule's `transfer DATA : buffer=N` directives, so the
//! pthreads-async multi-worker codegen (TASK-0228 Wave B) can size
//! Arc<Ring<T>> instances without re-reading ACFG or LinkedIR
//! (preserving the AlgoIR/LinkedIR/ACFG-free EventList contract).
//!
//! These tests pin both halves of the invariant:
//!
//! 1. Async schedule with `buffer=N` (example 13 / pipeline_parallel,
//!    `buffer=3`) populates the map with the expected N for every
//!    cross-worker Push/Wait pair.
//! 2. Sync-only schedule (example 01 / naive) produces a map with no
//!    entries (no cross-worker transfers — example 01 runs on one
//!    worker).

use std::fs;
use std::path::PathBuf;

use nucleus_compiler::{
    acfg::ACFGNode,
    algo::{lower_algo, parse_algo},
    apply_block_transforms, build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("two ancestors above compiler crate")
}

/// Run the full lower-link-inject pipeline for a given example/schedule.
fn lower(
    ex_rel: &str,
    sched_rel: &str,
) -> (
    nucleus_compiler::link::LinkedIR,
    nucleus_compiler::ACFG,
    PathBuf,
) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(ex_rel);
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).expect("read algo");
    let sched_src = fs::read_to_string(ex.join(sched_rel)).expect("read sched");

    let algo_ir = lower_algo(&parse_algo(&algo_src).expect("parse_algo")).expect("lower_algo");
    let sched_ir =
        lower_sched(&parse_sched(&sched_src).expect("parse_sched")).expect("lower_sched");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block_transforms");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    (linked, acfg, ex.join("kernels.rs"))
}

#[test]
fn async_pipeline_parallel_populates_buffer_for_each_cross_worker_pair() {
    // Example 13 / pipeline_parallel: three inter-stage `transfer ... :
    // async, buffer=3` directives on the cross-worker edges (input,
    // feat1, feat2). The host-side `transfer output : sync` is a SYNC
    // hop with default buffer=1.
    //
    // The new sidecar field MUST contain at least one entry whose
    // value is 3 (the async buffer), and the sync hops produce
    // entries whose value is 1.
    let (linked, acfg, _) = lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");

    // Some entries must exist (this is a multi-worker schedule with
    // cross-worker transfers).
    assert!(
        !sidecar.transfer_buffer_for_seq.is_empty(),
        "pipeline_parallel must produce non-empty transfer_buffer_for_seq; \
         got {:?}",
        sidecar.transfer_buffer_for_seq
    );

    // Count buffer values. pipeline_parallel declares 3 async edges
    // (input, feat1, feat2 — each `transfer ... : async, buffer=3`).
    // Each edge produces one Push/Wait pair sharing ONE SeqTag, so the
    // map has exactly 3 entries whose value is 3. (transfer_inject's
    // per-(src,dst) fan-out preserves seq sharing per pair — see
    // TASK-0216 forward-carry on TASK-0228.) A partial-drop regression
    // that dropped 1 of the 3 entries would slip past a `>= 1`
    // assertion; pin the exact 3 instead. Cycle-19 review-gate C.2.
    let count_3 = sidecar
        .transfer_buffer_for_seq
        .values()
        .filter(|&&v| v == 3)
        .count();
    assert_eq!(
        count_3, 3,
        "pipeline_parallel declares EXACTLY 3 async transfers \
         (input/feat1/feat2 with buffer=3); each Push/Wait pair shares \
         one SeqTag so the map must have exactly 3 entries with value 3. \
         A drop here is a regression in either transfer_inject's pair \
         creation or the sidecar walker. Got: {:?}",
        sidecar.transfer_buffer_for_seq
    );

    // The sync output hop (default buffer=1) must also appear if it
    // produced any cross-worker Push/Wait. Defensive: any value that's
    // not 1 or 3 would mean an unexpected buffer size leaked in.
    for (seq, &cap) in &sidecar.transfer_buffer_for_seq {
        assert!(
            cap == 1 || cap == 3,
            "pipeline_parallel only declares buffer=1 (default sync) or \
             buffer=3 (the 3 async edges); seq {seq:?} carries cap={cap}, \
             which is neither. Full map: {:?}",
            sidecar.transfer_buffer_for_seq
        );
    }
}

#[test]
fn sync_naive_schedule_has_empty_transfer_buffer_map() {
    // Example 01 / naive: single-worker schedule (all kernels on
    // `host`). No cross-worker transfers means transfer_inject
    // generates no XferPlaceholder, so the new sidecar map is empty.
    let (linked, acfg, _) = lower("01-elementwise-add", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert!(
        sidecar.transfer_buffer_for_seq.is_empty(),
        "01-elementwise-add/naive is single-worker; no cross-worker \
         transfers means transfer_buffer_for_seq must be empty. Got: {:?}",
        sidecar.transfer_buffer_for_seq
    );
}

#[test]
fn split_schedule_carries_default_buffer_1_for_sync_transfers() {
    // Example 02 / split: TWO workers, default-buffer (sync) transfers.
    // Map must be populated (multi-worker) AND every entry must be 1
    // (no async, no buffer override).
    let (linked, acfg, _) = lower("02-split-add", "schedules/split.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert!(
        !sidecar.transfer_buffer_for_seq.is_empty(),
        "02-split-add/split is multi-worker; cross-worker transfers \
         must populate the map. Got: {:?}",
        sidecar.transfer_buffer_for_seq
    );
    for (seq, &cap) in &sidecar.transfer_buffer_for_seq {
        assert_eq!(
            cap, 1,
            "split.sched.nuc declares no buffer override; every entry \
             must be the default 1. seq {seq:?} carries cap={cap}. \
             Full map: {:?}",
            sidecar.transfer_buffer_for_seq
        );
    }
}

#[test]
fn collect_walks_repeat_and_sequence_via_pipeline_parallel_fixture() {
    // Defensive coverage: the ACFG for pipeline_parallel nests
    // XferPlaceholder nodes inside Repeat (the pipelined `loop n`).
    // The walker MUST descend into Repeat::body — a regression that
    // stopped descending would silently drop all transfer entries.
    //
    // Verify by counting Xfer nodes via a direct walk vs the sidecar
    // map size. The map collapses Push+Wait endpoints onto one seq,
    // so the count is `xfer_node_count / 2`.
    let (linked, acfg, _) = lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let xfer_count = count_xfer_nodes(&acfg.root);
    let map_count = sidecar.transfer_buffer_for_seq.len();
    // Each Push/Wait pair shares one seq, so map_count == xfer_count / 2.
    assert_eq!(
        map_count * 2,
        xfer_count,
        "transfer_buffer_for_seq has {map_count} entries but the ACFG \
         carries {xfer_count} Xfer nodes (Push + Wait per pair = 2x \
         the map). Walker likely missed nodes inside Repeat or \
         Sequence. Full map: {:?}",
        sidecar.transfer_buffer_for_seq
    );
}

/// Forward-compatibility (cycle-19 review-gate B.1): a wire payload
/// serialised BEFORE TASK-0233 lacks the new `transfer_buffer_for_seq`
/// field. The struct's serde-default attribute on the field means
/// such payloads must deserialise successfully with the new field
/// defaulting to empty. This pins the contract on the wire surface,
/// not just the in-memory roundtrip (the existing
/// `sidecar_serde_roundtrip_is_byte_identical` in petri_to_events.rs
/// only exercises the current schema).
#[cfg(feature = "serde")]
#[test]
fn old_wire_payload_without_transfer_buffer_field_deserializes_with_empty_default() {
    use nucleus_compiler::NameSidecar;

    // A NameSidecar JSON payload missing the new field. Every other
    // field is present at its minimal valid empty shape.
    let old_json = r#"{
        "data_types": {},
        "consts": {},
        "loop_bounds": {},
        "kernel_sigs": {},
        "partition_worker_ranges": {}
    }"#;

    let sidecar: NameSidecar =
        serde_json::from_str(old_json).expect("old-wire payload must deserialize cleanly");
    assert!(
        sidecar.transfer_buffer_for_seq.is_empty(),
        "missing field on the wire must default to empty BTreeMap, \
         not error; got {:?}",
        sidecar.transfer_buffer_for_seq
    );

    // Defensive: a fresh (non-deserialised) NameSidecar::default()
    // also produces an empty map — behavioral symmetry between
    // "field absent on wire" and "field constructed fresh".
    let fresh = NameSidecar::default();
    assert_eq!(
        sidecar.transfer_buffer_for_seq, fresh.transfer_buffer_for_seq,
        "old-wire-deserialized empty must equal fresh-default empty"
    );
}

/// Cross-check helper that mirrors the sidecar's walker — independent
/// implementation so a bug in the sidecar walker is caught by
/// disagreement with this implementation.
///
/// **Why a fresh helper instead of `ACFGNode::count_xfers`** (cycle-19
/// review-gate C.1): `nucleus_compiler::passes::transfer_inject::count_xfers`
/// exists and walks the same shape. Reusing it here would make the
/// cross-check vacuous (one walker calling itself). The point of a
/// cross-check is structural INDEPENDENCE: if both walkers are correct
/// they agree; if either drifts, this test trips. So the duplication
/// is intentional — keep both.
fn count_xfer_nodes(node: &ACFGNode) -> usize {
    match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => 0,
        ACFGNode::Xfer(_) => 1,
        ACFGNode::Sequence(children) => children.iter().map(count_xfer_nodes).sum(),
        ACFGNode::Repeat { body, .. } => count_xfer_nodes(body),
    }
}
