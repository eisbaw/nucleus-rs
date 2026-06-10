//! Tests for the unified per-`SeqTag` `NameSidecar::xfer_facts` map
//! (TASK-0455.08).
//!
//! `xfer_facts` replaces the former parallel `transfer_buffer_for_seq`
//! (TASK-0233) and `transfer_transport_for_seq` (TASK-0438.02) maps with
//! ONE [`XferFacts`] value per cross-worker Push/Wait `seq`, carrying:
//!
//! - `buffer` (was `transfer_buffer_for_seq`);
//! - `transport` (was `transfer_transport_for_seq`);
//! - `notify` (NEW backend-facing surface — TASK-0455.08; the schedule's
//!   `transfer DATA : notify=event|poll` directive previously had no
//!   sidecar mirror at all);
//! - `pipeline_depth` (a backend-facing MIRROR of the ACFG's
//!   `pipeline_depth_for_seq`, which stays the Petri/initial-marking
//!   source of truth — see the `XferFacts::pipeline_depth` docs).
//!
//! These tests pin the END-TO-END threading (schedule directive →
//! `policy` → `xfer_facts`) for `notify` (AC#2) and the buffer/pipeline
//! mirrors, plus the serde missing-field default (AC#3).

use std::fs;
use std::path::PathBuf;

use nucleus_compiler::{
    acfg::NotifyMode,
    algo::{lower_algo, parse_algo},
    apply_block_transforms, build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched, TransportMode},
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
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    (linked, acfg)
}

/// AC#2: `notify` is carried per-seq end-to-end and readable by the
/// backend-facing surface. Example 13 / pipeline_parallel declares THREE
/// `transfer ... : async, buffer=3, notify=event` edges; the `notify=event`
/// directive must reach `XferFacts::notify == NotifyMode::Event` for every
/// such edge (previously it had no sidecar mirror at all).
#[test]
fn notify_event_directive_threads_into_xfer_facts() {
    let (linked, acfg) = lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");

    assert!(
        !sidecar.xfer_facts.is_empty(),
        "pipeline_parallel is multi-worker; xfer_facts must be populated. Got: {:?}",
        sidecar.xfer_facts
    );

    // The three async edges (input/feat1/feat2) all declare notify=event.
    // Each produces one Push/Wait pair sharing one SeqTag, so AT LEAST 3
    // entries must carry NotifyMode::Event. (The sync output hop carries
    // the default notify.)
    let event_count = sidecar
        .xfer_facts
        .values()
        .filter(|f| f.notify == NotifyMode::Event)
        .count();
    assert!(
        event_count >= 3,
        "the 3 `notify=event` async edges must each thread into \
         XferFacts::notify == Event; found {event_count}. Full map: {:?}",
        sidecar.xfer_facts
    );

    // Every entry's notify must be a value the schedule actually declared:
    // Event (the 3 async edges) or Default (the sync output hop, which
    // states no notify=). A Poll here would mean a leak.
    for (seq, f) in &sidecar.xfer_facts {
        assert!(
            matches!(f.notify, NotifyMode::Event | NotifyMode::Default),
            "pipeline_parallel declares only notify=event or no notify; \
             seq {seq:?} carries notify={:?}. Full map: {:?}",
            f.notify,
            sidecar.xfer_facts
        );
    }

    // The same fixture also proves the buffer and pipeline_depth facts
    // ride on the SAME unified value (no parallel maps).
    let buffer_3 = sidecar.xfer_facts.values().filter(|f| f.buffer == 3).count();
    assert!(
        buffer_3 >= 3,
        "the 3 `buffer=3` edges must thread into XferFacts::buffer; \
         found {buffer_3}. Full map: {:?}",
        sidecar.xfer_facts
    );
}

/// The `pipeline=D` directive's depth is MIRRORED onto `XferFacts::pipeline_depth`
/// (the backend-facing copy of `ACFG::pipeline_depth_for_seq`). Example 13's
/// `loop n : pipeline=3` wraps the cross-worker transfers, so at least one
/// xfer_facts entry must carry `pipeline_depth == Some(3)`, and that depth
/// must EQUAL the ACFG's source-of-truth entry for the same seq (the mirror
/// is consistent, not independently set).
#[test]
fn pipeline_depth_is_mirrored_from_acfg_consistently() {
    let (linked, acfg) = lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");

    // At least one seq must carry the mirrored depth.
    let with_depth: Vec<_> = sidecar
        .xfer_facts
        .iter()
        .filter(|(_, f)| f.pipeline_depth.is_some())
        .collect();
    assert!(
        !with_depth.is_empty(),
        "loop n : pipeline=3 wraps the cross-worker transfers; at least \
         one XferFacts must mirror a Some(depth). Full map: {:?}",
        sidecar.xfer_facts
    );

    // CONSISTENCY: every seq's mirror must equal the ACFG source of truth
    // (`pipeline_depth_for_seq`). This is what guards against the mirror
    // drifting from the single source — the DIVERGENCE HAZARD the unify
    // was meant to remove.
    for (seq, f) in &sidecar.xfer_facts {
        let acfg_depth = acfg.pipeline_depth_for_seq.get(seq).copied();
        assert_eq!(
            f.pipeline_depth, acfg_depth,
            "XferFacts::pipeline_depth for seq {seq:?} ({:?}) must mirror \
             ACFG::pipeline_depth_for_seq ({acfg_depth:?}) EXACTLY",
            f.pipeline_depth
        );
    }

    // At least one mirrored depth must be the declared 3.
    assert!(
        with_depth
            .iter()
            .any(|(_, f)| f.pipeline_depth.map(|d| d.get()) == Some(3)),
        "the declared pipeline=3 depth must appear as Some(3); got {:?}",
        with_depth
    );
}

/// A single-worker schedule produces NO cross-worker transfers, so
/// `xfer_facts` is empty (mirrors the `sidecar_buffer` empty-map pin but
/// over the unified surface).
#[test]
fn single_worker_schedule_has_empty_xfer_facts() {
    let (linked, acfg) = lower("01-elementwise-add", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert!(
        sidecar.xfer_facts.is_empty(),
        "single-worker schedule has no cross-worker transfers; xfer_facts \
         must be empty. Got: {:?}",
        sidecar.xfer_facts
    );
}

/// The accessor helpers (`xfer_buffer`/`xfer_transport`/`xfer_notify`)
/// agree with direct map reads, and apply the documented absent-seq
/// defaults (transport→PIO, notify→Default, buffer→None).
#[test]
fn accessor_helpers_agree_and_default_for_absent_seq() {
    use nucleus_compiler::event::SeqTag;

    let (linked, acfg) = lower("13-cnn-inference", "schedules/pipeline_parallel.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");

    for (seq, f) in &sidecar.xfer_facts {
        assert_eq!(sidecar.xfer_buffer(*seq), Some(f.buffer));
        assert_eq!(sidecar.xfer_transport(*seq), f.transport);
        assert_eq!(sidecar.xfer_notify(*seq), f.notify);
    }

    // An absent seq (way past any real one) defaults.
    let absent = SeqTag(u64::MAX);
    assert_eq!(sidecar.xfer_buffer(absent), None, "absent buffer is None");
    assert_eq!(
        sidecar.xfer_transport(absent),
        TransportMode::Pio,
        "absent transport defaults to PIO (byte-identity contract)"
    );
    assert_eq!(
        sidecar.xfer_notify(absent),
        NotifyMode::Default,
        "absent notify defaults to Default"
    );
}

/// AC#3: serde round-trip over a populated `XferFacts` (all four fields,
/// incl. notify and the pipeline_depth NonZeroU64), AND the missing-field
/// default — a wire payload with NO `xfer_facts` key deserialises to an
/// empty map (additive `serde(default)`).
#[cfg(feature = "serde")]
#[test]
fn xfer_facts_serde_roundtrip_and_missing_field_default() {
    use nucleus_compiler::event::SeqTag;
    use nucleus_compiler::sidecar::{NameSidecar, XferFacts};

    // (1) Round-trip a populated map. Cover all four fields, both
    // transport modes, all three notify modes, and Some/None depth.
    let mut s = NameSidecar::default();
    s.xfer_facts.insert(
        SeqTag(0),
        XferFacts {
            buffer: 3,
            transport: TransportMode::Dma,
            notify: NotifyMode::Event,
            pipeline_depth: std::num::NonZeroU64::new(3),
        },
    );
    s.xfer_facts.insert(
        SeqTag(1),
        XferFacts {
            buffer: 1,
            transport: TransportMode::Pio,
            notify: NotifyMode::Poll,
            pipeline_depth: None,
        },
    );

    let json = serde_json::to_string(&s).expect("serialize");
    let back: NameSidecar = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        s, back,
        "NameSidecar with populated xfer_facts must round-trip byte-identically"
    );

    // (2) Missing-field default: a wire payload with NO xfer_facts key
    // deserialises to an empty map (serde(default)).
    let old_json = r#"{
        "data_types": {},
        "consts": {},
        "loop_bounds": {},
        "kernel_sigs": {},
        "partition_worker_ranges": {}
    }"#;
    let defaulted: NameSidecar =
        serde_json::from_str(old_json).expect("missing xfer_facts must default, not error");
    assert!(
        defaulted.xfer_facts.is_empty(),
        "missing xfer_facts field must default to empty map; got {:?}",
        defaulted.xfer_facts
    );

    // (3) Within XferFacts, a missing `pipeline_depth` key defaults to
    // None (serde(default) on the field), so an emit produced before the
    // mirror landed still deserialises.
    let facts_missing_depth = r#"{
        "data_types": {},
        "consts": {},
        "loop_bounds": {},
        "kernel_sigs": {},
        "partition_worker_ranges": {},
        "xfer_facts": {"7": {"buffer": 2, "transport": "Pio", "notify": "Default"}}
    }"#;
    let m: NameSidecar = serde_json::from_str(facts_missing_depth)
        .expect("XferFacts with no pipeline_depth must default the field, not error");
    let f = m.xfer_facts.get(&SeqTag(7)).expect("seq 7 present");
    assert_eq!(f.buffer, 2);
    assert_eq!(f.transport, TransportMode::Pio);
    assert_eq!(f.notify, NotifyMode::Default);
    assert_eq!(
        f.pipeline_depth, None,
        "missing pipeline_depth in a wire XferFacts must default to None"
    );
}
