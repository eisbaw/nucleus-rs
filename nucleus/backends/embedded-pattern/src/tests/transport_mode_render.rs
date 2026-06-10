//! TASK-0438.02 — emit-oracle pins for the per-seq DMA-async vs PIO-sync
//! transport render divergence (AC#1).
//!
//! `render_run_body` lowers `Event::Push` / `Event::Wait` to either the
//! existing PIO byte-loop hooks (`shim.link_push` / `shim.link_recv`) or a
//! STRUCTURALLY DISTINCT modelled-DMA shape (`shim.dma_link_arm` +
//! `while !shim.dma_link_poll(..) { core::hint::spin_loop(); }`), chosen
//! per-`SeqTag` from the unified `NameSidecar::xfer_facts` map (the
//! `XferFacts::transport` field, read via `NameSidecar::xfer_transport`;
//! TASK-0455.08 unified the former `transfer_transport_for_seq` map). These
//! tests pin BOTH arms:
//!   - a seq mapped to `TransportMode::Dma` emits the arm + completion-spin
//!     and NOT the plain `link_push`/`link_recv`;
//!   - a seq ABSENT from the map (the default) emits the unchanged PIO path
//!     and NOT any `dma_link_*` — the load-bearing byte-identity guarantee
//!     for schedules with no `mode=` directive (02-split-add, 14-hearing-aid).

use nucleus_compiler::algo::{ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, WorkerId};
use nucleus_compiler::sched::TransportMode;
use nucleus_compiler::sidecar::{NameSidecar, XferFacts};
use nucleus_compiler::NameTables;

use crate::render::render_run_body;

const PEER: WorkerId = WorkerId(2);
const X: DataId = DataId(0); // "x"
const SEQ: u64 = 0;

fn push(seq: u64) -> Event {
    Event::Push {
        dst: PEER,
        data: X,
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

fn wait(seq: u64) -> Event {
    Event::Wait {
        src: PEER,
        data: X,
        tile: IterTile::empty(),
        seq: SeqTag(seq),
    }
}

/// NameTables with the data symbol `x` named (so `data_name` resolves) and a
/// peer worker present.
fn names() -> NameTables {
    let mut n = NameTables::default();
    n.data.insert(X, "x".to_string());
    n.worker.insert(PEER, "b".to_string());
    n
}

/// Sidecar typing `x` as a small `i32[4]` local (so the run-body data decl
/// emits). `transport` maps SEQ to `mode`; pass `None` to leave it absent
/// (the default-PIO case).
fn sidecar(mode: Option<TransportMode>) -> NameSidecar {
    let mut s = NameSidecar::default();
    s.data_types.insert(
        X,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![4],
        },
    );
    if let Some(m) = mode {
        s.xfer_facts.insert(
            SeqTag(SEQ),
            XferFacts {
                transport: m,
                ..Default::default()
            },
        );
    }
    s
}

#[test]
fn dma_seq_emits_arm_and_completion_spin_not_link_push() {
    let events = vec![push(SEQ), wait(SEQ)];
    let body = render_run_body(&events, &names(), &sidecar(Some(TransportMode::Dma)))
        .expect("DMA-mode run body renders");

    // Push arm: descriptor-arm + completion-spin (AC#1 structural shape).
    assert!(
        body.contains("shim.dma_link_arm(0, x.as_ptr() as *const u8, core::mem::size_of_val(&x));"),
        "DMA push must arm the descriptor:\n{body}"
    );
    // Wait arm: receive-arm + completion-spin.
    assert!(
        body.contains(
            "shim.dma_link_recv_arm(0, x.as_mut_ptr() as *mut u8, core::mem::size_of_val(&x));"
        ),
        "DMA wait must arm the receive descriptor:\n{body}"
    );
    // The completion-spin (AC#2: SPIN, not wfi) appears for BOTH endpoints.
    assert_eq!(
        body.matches("while !shim.dma_link_poll(0) { core::hint::spin_loop(); }")
            .count(),
        2,
        "both DMA endpoints must spin on completion (no wfi):\n{body}"
    );
    // The PIO hooks must NOT appear for a DMA-mode seq (true divergence).
    assert!(
        !body.contains("shim.link_push("),
        "DMA mode must NOT fall through to the PIO link_push:\n{body}"
    );
    assert!(
        !body.contains("shim.link_recv("),
        "DMA mode must NOT fall through to the PIO link_recv:\n{body}"
    );
}

#[test]
fn absent_seq_emits_plain_pio_not_dma() {
    // No `mode=` -> seq absent from xfer_facts -> default PIO.
    let events = vec![push(SEQ), wait(SEQ)];
    let body =
        render_run_body(&events, &names(), &sidecar(None)).expect("PIO-default run body renders");

    assert!(
        body.contains("shim.link_push(0, x.as_ptr() as *const u8, core::mem::size_of_val(&x));"),
        "absent seq must emit the unchanged PIO link_push:\n{body}"
    );
    assert!(
        body.contains("shim.link_recv(0, x.as_mut_ptr() as *mut u8, core::mem::size_of_val(&x));"),
        "absent seq must emit the unchanged PIO link_recv:\n{body}"
    );
    // No DMA shape leaks into the default path.
    assert!(
        !body.contains("dma_link_arm")
            && !body.contains("dma_link_recv_arm")
            && !body.contains("dma_link_poll"),
        "default-PIO path must emit NO dma_link_* hooks:\n{body}"
    );
}

#[test]
fn explicit_pio_mode_matches_the_absent_default_byte_for_byte() {
    // An explicit `mode=pio` and an absent seq must render IDENTICALLY — the
    // contract that keeps a pio-tagged edge byte-exact with pre-TASK-0438.02.
    let events = vec![push(SEQ), wait(SEQ)];
    let explicit = render_run_body(&events, &names(), &sidecar(Some(TransportMode::Pio)))
        .expect("explicit-PIO run body renders");
    let absent =
        render_run_body(&events, &names(), &sidecar(None)).expect("absent-default run body renders");
    assert_eq!(
        explicit, absent,
        "explicit mode=pio must be byte-identical to the absent default"
    );
}
