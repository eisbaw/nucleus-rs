//! Tests for the EventList contract from PRD §8.3 (TASK-0015).
//!
//! What this file covers:
//! - Constructor smoke tests for each `Event` variant (so a renaming
//!   of a field surfaces here before in real callers).
//! - Serde JSON round-trip for every variant.
//! - `PartialEq` and `Hash` correctness on simple cases.
//! - `IterTile` equality semantics: same bounds + same iter-var
//!   order compare equal; reordering breaks equality.
//!
//! What this file does NOT cover:
//! - Trait surface exhaustiveness (e.g. checking `Send + Sync` or
//!   that every newtype implements `Ord`). Filed as a follow-up.
//! - Semantic validity of events (e.g. `Push.dst != self`, matched
//!   `seq`). The event types are a contract, not a validator.

use std::collections::{BTreeSet, HashSet};

use compiler::event::{
    ArgBinding, DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId, Region, SeqTag,
    SyncKind, SyncTag, WorkerId,
};

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

fn tile_y32_64_x0_256() -> IterTile {
    IterTile::new(vec![(IterVar(0), 32..64), (IterVar(1), 0..256)])
}

fn sample_fire() -> Event {
    Event::fire_bare(KernelId(7), tile_y32_64_x0_256())
}

fn sample_alloc() -> Event {
    Event::Alloc {
        data: DataId(3),
        tile: tile_y32_64_x0_256(),
        region: Region(42),
    }
}

fn sample_push() -> Event {
    Event::Push {
        dst: WorkerId(1),
        data: DataId(3),
        tile: tile_y32_64_x0_256(),
        seq: SeqTag(5),
    }
}

fn sample_wait() -> Event {
    Event::Wait {
        src: WorkerId(0),
        data: DataId(3),
        tile: tile_y32_64_x0_256(),
        seq: SeqTag(5),
    }
}

fn sample_sync() -> Event {
    let mut p = BTreeSet::new();
    p.insert(WorkerId(0));
    p.insert(WorkerId(1));
    p.insert(WorkerId(2));
    Event::Sync {
        participants: p,
        kind: SyncKind::Barrier,
        sync: SyncTag(7),
    }
}

fn sample_free() -> Event {
    Event::Free {
        data: DataId(3),
        tile: tile_y32_64_x0_256(),
    }
}

// --------------------------------------------------------------------
// Constructor smoke tests
// --------------------------------------------------------------------

#[test]
fn fire_constructor_smoke() {
    let e = sample_fire();
    if let Event::Fire { kernel, tile, .. } = e {
        assert_eq!(kernel, KernelId(7));
        assert_eq!(tile.rank(), 2);
        assert!(!tile.is_empty());
    } else {
        panic!("expected Fire");
    }
}

#[test]
fn fire_with_empty_tile_for_non_iterated_firing() {
    let e = Event::fire_bare(KernelId(0), IterTile::empty());
    if let Event::Fire { tile, .. } = e {
        assert!(tile.is_empty());
        assert_eq!(tile.rank(), 0);
    } else {
        panic!("expected Fire");
    }
}

#[test]
fn alloc_constructor_smoke() {
    let e = sample_alloc();
    if let Event::Alloc { data, tile, region } = e {
        assert_eq!(data, DataId(3));
        assert_eq!(region, Region(42));
        assert_eq!(tile.rank(), 2);
    } else {
        panic!("expected Alloc");
    }
}

#[test]
fn push_constructor_smoke() {
    let e = sample_push();
    if let Event::Push {
        dst,
        data,
        tile,
        seq,
    } = e
    {
        assert_eq!(dst, WorkerId(1));
        assert_eq!(data, DataId(3));
        assert_eq!(seq, SeqTag(5));
        assert_eq!(tile.rank(), 2);
    } else {
        panic!("expected Push");
    }
}

#[test]
fn wait_constructor_smoke() {
    let e = sample_wait();
    if let Event::Wait {
        src,
        data,
        tile,
        seq,
    } = e
    {
        assert_eq!(src, WorkerId(0));
        assert_eq!(data, DataId(3));
        assert_eq!(seq, SeqTag(5));
        assert_eq!(tile.rank(), 2);
    } else {
        panic!("expected Wait");
    }
}

#[test]
fn sync_constructor_smoke() {
    let e = sample_sync();
    if let Event::Sync {
        participants,
        kind,
        sync,
    } = e
    {
        assert_eq!(participants.len(), 3);
        assert!(participants.contains(&WorkerId(0)));
        assert!(participants.contains(&WorkerId(1)));
        assert!(participants.contains(&WorkerId(2)));
        assert_eq!(kind, SyncKind::Barrier);
        // TASK-0172: Sync carries a stable cross-worker barrier id.
        assert_eq!(sync, SyncTag(7));
    } else {
        panic!("expected Sync");
    }
}

#[test]
fn free_constructor_smoke() {
    let e = sample_free();
    if let Event::Free { data, tile } = e {
        assert_eq!(data, DataId(3));
        assert_eq!(tile.rank(), 2);
    } else {
        panic!("expected Free");
    }
}

// --------------------------------------------------------------------
// Serde JSON round-trip
// --------------------------------------------------------------------

fn roundtrip(e: &Event) -> Event {
    let s = serde_json::to_string(e).expect("serialise");
    serde_json::from_str(&s).expect("deserialise")
}

#[test]
fn serde_roundtrip_fire() {
    let e = sample_fire();
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn serde_roundtrip_alloc() {
    let e = sample_alloc();
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn serde_roundtrip_push() {
    let e = sample_push();
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn serde_roundtrip_wait() {
    let e = sample_wait();
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn serde_roundtrip_sync() {
    let e = sample_sync();
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn serde_roundtrip_free() {
    let e = sample_free();
    assert_eq!(roundtrip(&e), e);
}

#[test]
fn serde_roundtrip_empty_tile() {
    let e = Event::fire_bare(KernelId(99), IterTile::empty());
    assert_eq!(roundtrip(&e), e);
}

// --------------------------------------------------------------------
// PartialEq correctness
// --------------------------------------------------------------------

#[test]
fn equal_fires_compare_equal() {
    assert_eq!(sample_fire(), sample_fire());
}

#[test]
fn different_kernel_id_compares_unequal() {
    let a = Event::fire_bare(KernelId(1), IterTile::empty());
    let b = Event::fire_bare(KernelId(2), IterTile::empty());
    assert_ne!(a, b);
}

#[test]
fn different_variants_compare_unequal() {
    assert_ne!(sample_fire(), sample_alloc());
    assert_ne!(sample_push(), sample_wait());
}

#[test]
fn itertile_same_bounds_same_order_compare_equal() {
    let a = IterTile::new(vec![(IterVar(0), 0..16), (IterVar(1), 0..32)]);
    let b = IterTile::new(vec![(IterVar(0), 0..16), (IterVar(1), 0..32)]);
    assert_eq!(a, b);
}

#[test]
fn itertile_reordered_bounds_compare_unequal() {
    // The iteration-nest order is semantically meaningful (outer vs
    // inner loop). Reordering breaks equality on purpose; see module
    // docs.
    let a = IterTile::new(vec![(IterVar(0), 0..16), (IterVar(1), 0..32)]);
    let b = IterTile::new(vec![(IterVar(1), 0..32), (IterVar(0), 0..16)]);
    assert_ne!(a, b);
}

#[test]
fn itertile_different_ranges_compare_unequal() {
    let a = IterTile::new(vec![(IterVar(0), 0..16)]);
    let b = IterTile::new(vec![(IterVar(0), 0..17)]);
    assert_ne!(a, b);
}

#[test]
fn sync_participants_order_irrelevant_for_equality() {
    // BTreeSet equality is by membership, not by insertion order.
    let mut p1 = BTreeSet::new();
    p1.insert(WorkerId(2));
    p1.insert(WorkerId(0));
    p1.insert(WorkerId(1));

    let mut p2 = BTreeSet::new();
    p2.insert(WorkerId(0));
    p2.insert(WorkerId(1));
    p2.insert(WorkerId(2));

    let a = Event::Sync {
        participants: p1,
        kind: SyncKind::Barrier,
        sync: SyncTag(0),
    };
    let b = Event::Sync {
        participants: p2,
        kind: SyncKind::Barrier,
        sync: SyncTag(0),
    };
    assert_eq!(a, b);
}

// --------------------------------------------------------------------
// Hash correctness
// --------------------------------------------------------------------

#[test]
fn equal_events_have_same_hash_set_membership() {
    // Hashes don't have to be equal in general (Hash trait permits
    // collisions both ways), but the contract is: a == b => hash(a)
    // == hash(b). The cheapest check is that inserting both into a
    // HashSet collapses them.
    let mut set: HashSet<Event> = HashSet::new();
    set.insert(sample_fire());
    set.insert(sample_fire());
    assert_eq!(set.len(), 1);
}

#[test]
fn distinct_events_kept_distinct_in_hashset() {
    let mut set: HashSet<Event> = HashSet::new();
    set.insert(sample_fire());
    set.insert(sample_alloc());
    set.insert(sample_push());
    set.insert(sample_wait());
    set.insert(sample_sync());
    set.insert(sample_free());
    // Six variants, six entries.
    assert_eq!(set.len(), 6);
}

#[test]
fn itertile_hash_consistent_with_eq() {
    let mut set: HashSet<Event> = HashSet::new();
    let a = Event::Free {
        data: DataId(1),
        tile: IterTile::new(vec![(IterVar(0), 0..16), (IterVar(1), 0..32)]),
    };
    let b = Event::Free {
        data: DataId(1),
        tile: IterTile::new(vec![(IterVar(0), 0..16), (IterVar(1), 0..32)]),
    };
    set.insert(a);
    set.insert(b);
    assert_eq!(set.len(), 1, "equal IterTiles must hash the same");
}

// --------------------------------------------------------------------
// Wire format spot-check (one variant; not a snapshot of the whole
// surface, to keep this from being brittle to serde-naming knobs).
// --------------------------------------------------------------------

#[test]
fn fire_json_contains_expected_top_level_tag() {
    let e = sample_fire();
    let s = serde_json::to_string(&e).expect("serialise");
    // Externally-tagged by default: {"Fire":{...}}.
    assert!(s.starts_with("{\"Fire\""), "unexpected serialisation: {s}");
}

// --------------------------------------------------------------------
// TASK-0156 — Fire carries ordered per-parameter value bindings.
// --------------------------------------------------------------------

fn slice(data: u64, indices: Vec<compiler::algo::IrExpr>) -> DataSlice {
    DataSlice {
        data: DataId(data),
        indices,
    }
}

#[test]
fn fire_binding_preserves_input_order_and_output() {
    use compiler::algo::IrExpr;
    // blur3-like: inputs are data slices in argument order; the
    // output is a distinct (data, slice). Order must be preserved
    // exactly (parameter order is load-bearing for codegen).
    let inputs = vec![
        ArgBinding::Data(slice(0, vec![IrExpr::Ident("a".into())])),
        ArgBinding::Scalar(IrExpr::IntLit(7)),
        ArgBinding::Data(slice(1, vec![IrExpr::Ident("b".into())])),
    ];
    let output = Some(slice(2, vec![IrExpr::Ident("c".into())]));
    let binding = FireBinding {
        inputs: inputs.clone(),
        output: output.clone(),
    };
    let e = Event::fire(KernelId(3), IterTile::empty(), binding);

    let Event::Fire {
        kernel,
        tile,
        bindings,
    } = &e
    else {
        panic!("expected Fire");
    };
    assert_eq!(*kernel, KernelId(3));
    assert!(tile.is_empty());
    // Exact order preserved.
    assert_eq!(bindings.inputs, inputs);
    assert_eq!(bindings.output, output);
    assert!(!bindings.is_empty());
}

#[test]
fn fire_bare_has_empty_binding() {
    let e = Event::fire_bare(KernelId(0), IterTile::empty());
    let Event::Fire { bindings, .. } = &e else {
        panic!("expected Fire");
    };
    assert!(bindings.is_empty());
    assert_eq!(*bindings, FireBinding::none());
}

#[test]
fn serde_roundtrip_fire_with_binding() {
    use compiler::algo::{IrBinOp, IrExpr};
    // A stencil-shaped binding with a compound index expression
    // exercises the IrExpr serde path on Event.
    let y_minus_1 = IrExpr::BinOp(
        IrBinOp::Sub,
        Box::new(IrExpr::Ident("y".into())),
        Box::new(IrExpr::IntLit(1)),
    );
    let binding = FireBinding {
        inputs: vec![ArgBinding::Data(slice(
            0,
            vec![y_minus_1, IrExpr::Ident("x".into())],
        ))],
        output: Some(slice(1, vec![IrExpr::Ident("y".into())])),
    };
    let e = Event::fire(KernelId(9), IterTile::empty(), binding);
    assert_eq!(roundtrip(&e), e, "binding must survive serde round-trip");
}

#[test]
fn fire_with_distinct_bindings_are_unequal_and_hashable() {
    use compiler::algo::IrExpr;
    let a = Event::fire(
        KernelId(1),
        IterTile::empty(),
        FireBinding {
            inputs: vec![ArgBinding::Data(slice(0, vec![]))],
            output: None,
        },
    );
    let b = Event::fire(
        KernelId(1),
        IterTile::empty(),
        FireBinding {
            inputs: vec![ArgBinding::Scalar(IrExpr::IntLit(0))],
            output: None,
        },
    );
    // Same kernel + tile but different bindings ⇒ distinct events.
    assert_ne!(a, b);
    // Event: Hash still holds (FireBinding has a manual Hash).
    let mut set: HashSet<Event> = HashSet::new();
    set.insert(a);
    set.insert(b);
    assert_eq!(set.len(), 2);
}

// --------------------------------------------------------------------
// Event::Loop — structure-preserving rolled loop (TASK-0159)
// --------------------------------------------------------------------

#[test]
fn loop_constructor_carries_iter_var_range_and_body() {
    let body = vec![sample_fire(), sample_push()];
    let e = Event::loop_over(IterVar(3), 1..15, body.clone());
    match &e {
        Event::Loop {
            iter_var,
            range,
            body: b,
            block_tag,
        } => {
            assert_eq!(*iter_var, IterVar(3));
            assert_eq!(*range, 1..15, "concrete bound carried verbatim");
            assert_eq!(*b, body, "body order preserved, not flattened");
            // `loop_over` is the untagged constructor (source loops);
            // strip-mined inner loops use `loop_over_tagged`.
            assert_eq!(*block_tag, None, "loop_over yields no block_tag");
        }
        other => panic!("expected Event::Loop, got {other:?}"),
    }
}

#[test]
fn loop_serde_roundtrip_including_nested_loop() {
    // A nested loop (Repeat-inside-Repeat projects to Loop-inside-Loop).
    let inner = Event::loop_over(IterVar(1), 0..8, vec![sample_fire()]);
    let outer = Event::loop_over(IterVar(0), 0..4, vec![inner, sample_sync()]);
    assert_eq!(roundtrip(&outer), outer, "Loop survives serde verbatim");
}

#[test]
fn loop_distinct_structure_is_unequal_and_hashable() {
    // Same iter-var + same body, different range ⇒ distinct events.
    let a = Event::loop_over(IterVar(0), 0..16, vec![sample_fire()]);
    let b = Event::loop_over(IterVar(0), 0..15, vec![sample_fire()]);
    assert_ne!(a, b, "different bound ⇒ different loop");

    // Same range, different body ⇒ distinct.
    let c = Event::loop_over(IterVar(0), 0..16, vec![sample_push()]);
    assert_ne!(a, c, "different body ⇒ different loop");

    // Event: Hash still holds with the recursive manual Hash.
    let mut set: HashSet<Event> = HashSet::new();
    set.insert(a.clone());
    set.insert(b);
    set.insert(c);
    set.insert(a); // duplicate of the first insert
    assert_eq!(set.len(), 3, "three structurally distinct loops");
}

#[test]
fn equal_loops_hash_the_same() {
    let a = Event::loop_over(IterVar(2), 5..9, vec![sample_fire(), sample_free()]);
    let b = Event::loop_over(IterVar(2), 5..9, vec![sample_fire(), sample_free()]);
    assert_eq!(a, b);
    let mut set: HashSet<Event> = HashSet::new();
    set.insert(a);
    set.insert(b);
    assert_eq!(set.len(), 1, "equal loops must hash the same");
}
