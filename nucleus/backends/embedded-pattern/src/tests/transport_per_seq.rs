//! TASK-0049.05.02 — per-seq transport topology tests.
//!
//! The ex14 multi-MCU deadlock was an UNFRAMED shared-FIFO multiplex: the
//! pre-fix `TransportPlan` grouped channels by PEER, so two SAME-DIRECTION
//! channels between one worker pair (ex14 `fe`->`dsp` seq0 + seq2) rode ONE
//! USART. `link_recv` reads raw bytes off that single FIFO with no per-seq
//! framing, so when push order != recv order the two streams interleave and
//! the downstream worker starves (rf stalled at frame 3/4).
//!
//! The fix assigns ONE DEDICATED USART + hub per CHANNEL (`SeqTag`). These
//! tests pin that property at the `TransportPlan` level: same-direction
//! multi-seq get DISTINCT USARTs/hubs (the BITE — the pre-fix per-peer code
//! mapped them to the same slot), the channel pool is bounded (>8 fails
//! loud), and a one-sided channel fails loud rather than silently emitting a
//! deadlocking firmware.

use std::collections::BTreeMap;

use backend_common::EmitError;
use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::multimcu::{render_multimachine_resc, TransportPlan};

const A: WorkerId = WorkerId(1);
const B: WorkerId = WorkerId(2);

fn push(dst: WorkerId, data: u64, seq: u64) -> Event {
    Event::Push {
        dst,
        data: DataId(data),
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

fn wait(src: WorkerId, data: u64, seq: u64) -> Event {
    Event::Wait {
        src,
        data: DataId(data),
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

fn ab_names() -> NameTables {
    let mut n = NameTables::default();
    n.worker.insert(A, "a".to_string());
    n.worker.insert(B, "b".to_string());
    n
}

/// THE BITE: worker `a` pushes TWO same-direction channels (seq0 + seq2) to
/// `b`; each must land on its OWN USART (and its OWN hub), so the two
/// streams can never cross on a shared byte FIFO. The pre-fix per-peer code
/// mapped both to `b`'s single USART — this asserts they are now DISTINCT.
#[test]
fn same_direction_multiseq_get_distinct_usarts_and_hubs() {
    let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    // a -> b on seq0 (data 0) AND seq2 (data 1): SAME direction, one pair.
    pw.insert(A, vec![push(B, 0, 0), push(B, 1, 2)]);
    pw.insert(B, vec![wait(A, 0, 0), wait(A, 1, 2)]);

    let plan = TransportPlan::build(&pw, &ab_names(), &NameSidecar::default())
        .expect("two same-direction channels build");

    // Distinct USART per channel, on BOTH endpoints.
    for w in [A, B] {
        let su = &plan.workers[&w].seq_usart;
        let u0 = su.get(&0).expect("seq0 mapped");
        let u2 = su.get(&2).expect("seq2 mapped");
        assert_ne!(
            u0.base, u2.base,
            "worker {w:?}: same-direction seq0 and seq2 must ride DISTINCT \
             USARTs (got {:?} for both)",
            u0.renode_name
        );
    }

    // One hub per channel (2 hubs), distinct seqs.
    assert_eq!(plan.hubs.len(), 2, "one hub per channel: {:?}", plan.hubs);
    let mut hub_seqs: Vec<u64> = plan.hubs.iter().map(|h| h.seq).collect();
    hub_seqs.sort_unstable();
    assert_eq!(hub_seqs, vec![0, 2], "hubs must carry seq0 + seq2");

    // The generated .resc creates two distinct hubs and `a` Connects two
    // distinct USARTs (one per channel).
    let resc = render_multimachine_resc(&plan);
    assert_eq!(
        resc.matches("emulation CreateUARTHub").count(),
        2,
        "two per-channel hubs in the .resc:\n{resc}"
    );
    // `a`'s two channels are both a->b, so its two connects target two
    // DISTINCT USART peripherals (not one shared usart2).
    assert!(
        resc.contains("connector Connect usart2 link_a_b_s0")
            && resc.contains("connector Connect usart3 link_a_b_s2"),
        "`a` must Connect seq0 + seq2 on DISTINCT USARTs:\n{resc}"
    );
}

/// A worker using more distinct channels than the 8-slot USART pool fails
/// LOUD (typed `UnsupportedFeature`) — never silently truncates the seq
/// table (which would emit a firmware whose `mmcu_link_base` panics on an
/// unmapped seq, a brick on bare metal). Exercises the realistic boundary
/// where the pool is exhausted by a MIX of sends and receives (the
/// per-worker pool counts BOTH `Push` and `Wait` channels via
/// `collect_seqs`), not just one direction.
#[test]
fn over_eight_channels_fails_loud() {
    // `a` SENDS seqs 0..5 to `b` AND RECEIVES seqs 5..9 from `b` — 9 distinct
    // channels touching `a` (5 sends + 4 receives). `b` mirrors so the only
    // failure is the per-worker USART-pool overflow, not a one-sided edge.
    let mut a_evs = Vec::new();
    let mut b_evs = Vec::new();
    for seq in 0..5u64 {
        a_evs.push(push(B, seq, seq));
        b_evs.push(wait(A, seq, seq));
    }
    for seq in 5..9u64 {
        a_evs.push(wait(B, seq, seq));
        b_evs.push(push(A, seq, seq));
    }
    let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    pw.insert(A, a_evs);
    pw.insert(B, b_evs);

    let err = TransportPlan::build(&pw, &ab_names(), &NameSidecar::default())
        .expect_err("9 channels must exceed the 8-USART pool");
    match err {
        EmitError::UnsupportedFeature(m) => {
            assert!(
                m.contains("transport channels") && m.contains('8'),
                "message must name the channel/USART-pool limit: {m}"
            );
        }
        other => panic!("expected UnsupportedFeature, got {other:?}"),
    }
}

/// A one-sided channel — a Push with no matching Wait — fails LOUD
/// (`ContractGap`) instead of emitting a firmware that deadlocks (the
/// receiver never exists / the sender's bytes drop).
#[test]
fn one_sided_channel_fails_loud() {
    let mut pw: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    // `a` pushes seq0 to `b`, but nobody Waits it.
    pw.insert(A, vec![push(B, 0, 0)]);

    let err = TransportPlan::build(&pw, &ab_names(), &NameSidecar::default())
        .expect_err("a Push with no matching Wait must fail loud");
    match err {
        EmitError::ContractGap(m) => assert!(
            m.contains("one-sided") && m.contains("seq 0"),
            "message must name the one-sided channel: {m}"
        ),
        other => panic!("expected ContractGap, got {other:?}"),
    }
}
