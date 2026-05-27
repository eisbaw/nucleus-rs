//! TASK-0329.01.01 — safe push-before-wait reordering.
//!
//! Per-worker event-list reordering. For each non-host worker's event
//! list, hoist hoistable worker-to-worker `Push` events above preceding
//! worker-to-worker `Wait` events within a top-level boundary (between
//! adjacent `Event::Sync` events at depth 0). Backend-local: invoked by
//! the driver for the per-seq-demux event backends — `mp-tcp-event`
//! (cycle 162, the trigger backend per TASK-0332 cycle-150 finding)
//! and `mp-uds-event` (cycle 197 widening for TASK-0044.03.01,
//! structural twin of mp-tcp-event with UDS transport). Other
//! backends do not need it because their transport / sync model does
//! not deadlock under the cycle-149 synchronous host-relay's
//! wait-before-push hazard. Specifically excluded: pthreads-sync /
//! pthreads-async / openmp-rs (shared-memory `Slot<T>` channels — no
//! synchronous host-relay, no wait-before-push deadlock surface);
//! mp-tcp-bufsync (blocking-read framing, single in-flight per
//! channel — no reorder window opens); mp-tcp-poll (inherits
//! mp-tcp-bufsync's FIFO shape per memory
//! `project-mp-tcp-event-vs-bufsync-safety-profile`).
//!
//! ## Hoistable predicate (TASK-0329.01.01 AC#1)
//!
//! A worker-to-worker `Event::Push` (`dst != host`) at position p in a
//! worker's top-level boundary is *hoistable* iff NO preceding
//! `Event::Wait` at position q < p in the same boundary satisfies BOTH:
//!
//! 1. `Wait.src != host` (i.e. the Wait is itself a w2w transfer, not
//!    host's broadcast / scatter of input data), AND
//! 2. `Wait.data == Push.data` AND `Wait.tile` may overlap `Push.tile`
//!    on any shared iteration-variable axis (per [`tiles_may_overlap`]).
//!
//! Condition 1 is the load-bearing distinction from cycle-161's first
//! design draft (cycle-161b architect P1.1): the 05/distributed-2d
//! halo Push payload reads from `img_in`, which the worker first
//! received from host via a host->worker Wait (the broadcast). Treating
//! that host->worker Wait as a dependency would have made every
//! halo-strip Push not-hoistable, and the reorder pass would have been
//! a no-op on its sole target cell. host->worker Waits are NOT
//! load-bearing here because host's preceding events have completed
//! (the relay phase splices after host's broadcast and before the
//! first wait-bearing Sync; host's data is already on every worker
//! before the reorder window opens).
//!
//! Condition 2 is conservative (false-positive on dependency = false-
//! negative on hoistability = safe). Tile-overlap is computed per
//! shared IterVar axis: if any shared axis is definitively disjoint,
//! tiles are disjoint. Missing-axis-in-one-tile is treated as
//! non-restrictive (overlap by default).
//!
//! ## Top-level boundary
//!
//! A boundary is the maximal contiguous run of events at depth 0
//! between adjacent `Event::Sync` events (or between the start of the
//! list and the first Sync, or between the last Sync and the end of
//! the list). Reordering is per-boundary: an event in one boundary is
//! never moved into another. `Event::Sync` events themselves stay in
//! place.
//!
//! `Event::Loop` bodies are NOT recursed into (the loop event itself
//! is one event at depth 0 and is treated as opaque by this pass).
//! Loop-body w2w Pushes are TASK-0330's surface and were handled by
//! the now-landed slice 2 (TASK-0329.01.02 cycle 163 +
//! TASK-0329.01.02.01 cycle 165): `apply_host_data_relay_inject`
//! routes in-Repeat-body non-host-pair Push/Wait through host at
//! the ACFG layer BEFORE this pass runs, so by the time the worker
//! event list reaches `apply_safe_push_reorder` the Loop-body w2w
//! Push shape is structurally absent for any pair the upstream pass
//! covers (cycle-163b residual class (R-singleton) is the
//! exception). Cycle-166b: this docstring was originally
//! "slice 2 ... is the architectural fix" — a predictive claim that
//! became hostage to fortune; reframed to past-tense + named
//! landed cycles per `feedback-comment-doc-lie-recurring` L3.
//!
//! ## Backend asymmetry (TASK-0329.01.01 AC#3)
//!
//! This pass mutates only the *worker's event list*. The splice-point
//! adjustment that pairs with this reorder lives in `mp-tcp-event`'s
//! `relay_phase_insertion_point` and is NOT mirrored on
//! `mp-tcp-bufsync`. Bufsync's constraint 3 (per cycle-148 design:
//! "relay reads from data_<src> would race host's own reads on the
//! same socket") makes moving its splice point unsafe even though
//! the workers' event reordering would itself be sound on bufsync.
//! See memory `project-mp-tcp-event-vs-bufsync-safety-profile` for
//! the per-seq-demux vs FIFO distinction.
//!
//! ## Semantic preservation argument
//!
//! Within a top-level boundary, reordering events is semantics-
//! preserving iff no event's outputs are required as inputs by any
//! event that originally preceded it. This pass enforces that by
//! moving Push only above Waits that DON'T write to overlapping
//! `(data, tile)`. Other event reorderings the pass introduces are
//! Push-above-Push or Push-above-non-Wait events; Push has no
//! observable effect on the worker's local state (it sends bytes to
//! another worker), so its position among other non-conflicting
//! events is observationally equivalent.

use crate::event::{DataId, Event, IterTile, IterVar, WorkerId};
use std::collections::BTreeMap;
use std::ops::Range;

/// Apply safe-push reorder to every non-host worker's event list.
///
/// Returns a new map. Host's events are unchanged. Each non-host
/// worker's event list is rewritten per the module-level invariants.
/// Idempotent: a second application is a no-op (after one pass, every
/// hoistable Push is already above its preceding w2w Waits).
pub fn apply_safe_push_reorder(
    per_worker: BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> BTreeMap<WorkerId, Vec<Event>> {
    per_worker
        .into_iter()
        .map(|(w, events)| {
            if w == host {
                (w, events)
            } else {
                (w, reorder_worker_events(events, host))
            }
        })
        .collect()
}

fn reorder_worker_events(events: Vec<Event>, host: WorkerId) -> Vec<Event> {
    let mut result = Vec::with_capacity(events.len());
    let mut start = 0;
    for i in 0..events.len() {
        if matches!(events[i], Event::Sync { .. }) {
            reorder_boundary(&events[start..i], host, &mut result);
            result.push(events[i].clone());
            start = i + 1;
        }
    }
    reorder_boundary(&events[start..], host, &mut result);
    result
}

fn reorder_boundary(events: &[Event], host: WorkerId, out: &mut Vec<Event>) {
    let mut tainted: Vec<(DataId, IterTile)> = Vec::new();
    let mut hoistable_idx: Vec<usize> = Vec::new();
    let mut others_idx: Vec<usize> = Vec::new();

    for (i, e) in events.iter().enumerate() {
        match e {
            Event::Wait { src, data, tile, .. } if *src != host => {
                tainted.push((*data, tile.clone()));
                others_idx.push(i);
            }
            // host->worker Wait: per AC#1 P1.1, the FIRST design
            // draft (cycle 162) excluded these from tainting on the
            // argument that "host's broadcast precedes the relay
            // window". Empirical cycle-162a finding refined this:
            // a host->worker Wait CAN appear in the same boundary as
            // a w2w Push that reads the data (the schedule may
            // project Wait_host AFTER bar_0 OR in the same boundary
            // as the halo exchange — depends on transfer_inject
            // placement). For safety, we DO taint here with an
            // unbounded tile sentinel: any subsequent w2w Push of
            // the same data is marked not-hoistable.
            //
            // For the cycle-161/161b target cell (05/distributed-2d),
            // the projection places Wait_host AFTER bar_0 (different
            // boundary from the halo Push/Wait pre-bar_0), so this
            // taint is dormant — the Push is hoisted because there
            // is no host->worker Wait in the same boundary. The
            // taint matters for schedules where they share a
            // boundary; in those, conservative no-hoist is sound
            // (Push depends on host-broadcast data).
            Event::Wait { src: _, data, .. } => {
                tainted.push((*data, IterTile::empty()));
                others_idx.push(i);
            }
            // Fire writes: a kernel firing whose `bindings.output`
            // is `Some(DataSlice { data, .. })` writes that data.
            // Tile is conservatively unbounded (empty IterTile sentinel
            // — `tiles_may_overlap` returns true on empty), so any
            // subsequent w2w Push of the same data is considered
            // dependent and not hoisted.
            //
            // This case is load-bearing for schedules like
            // 06-separable-filter/distributed2: the worker's `tmp`
            // is initialized to zero, then a Fire (pass-1 blur)
            // writes the real `tmp`, then a w2w Push sends `tmp` to
            // neighbours. Without this taint, the Push would be
            // hoisted above the Fire and send the zero buffer
            // (cycle-162a empirical bug from the first slice-1 draft).
            Event::Fire { bindings, .. } => {
                if let Some(out_slice) = &bindings.output {
                    tainted.push((out_slice.data, IterTile::empty()));
                }
                others_idx.push(i);
            }
            // Loop bodies may contain Fire or Wait events that write
            // data. We conservatively scan the body recursively and
            // taint every written DataId with an unbounded tile —
            // the Loop event itself stays in others_idx (the body
            // is opaque w.r.t. reordering at this layer; slice 2
            // TASK-0329.01.02 handles in-loop w2w transfers via the
            // ACFG-layer relay-inject pass).
            Event::Loop { body, .. } => {
                collect_loop_body_writes(body, &mut tainted);
                others_idx.push(i);
            }
            Event::Push { dst, data, tile, .. } if *dst != host => {
                let is_dependent = tainted
                    .iter()
                    .any(|(d, t)| *d == *data && tiles_may_overlap(t, tile));
                if is_dependent {
                    others_idx.push(i);
                } else {
                    hoistable_idx.push(i);
                }
            }
            _ => others_idx.push(i),
        }
    }

    for &idx in &hoistable_idx {
        out.push(events[idx].clone());
    }
    for &idx in &others_idx {
        out.push(events[idx].clone());
    }
}

/// Recursively scan a `Loop` body to collect every written `(DataId,
/// IterTile)`. Used by [`reorder_boundary`] to conservatively taint
/// a Loop event with its body's writes so subsequent w2w Pushes of
/// any written data are correctly marked not-hoistable.
///
/// Both w2w `Wait` AND host->worker `Wait` taint here — mirrors the
/// cycle-162a refinement in `reorder_boundary` (cycle-162b architect
/// P2.1 fold-back). A host->worker Wait inside a Loop body writes
/// data at runtime when the Loop executes; a subsequent top-level
/// w2w Push of the same data would be unsound if hoisted above the
/// Loop. The asymmetry where the top-level handler tainted but the
/// Loop-body collector did not was the kind of silent-sibling gap
/// memory `feedback-silent-sibling-defect` catalogues.
fn collect_loop_body_writes(body: &[Event], tainted: &mut Vec<(DataId, IterTile)>) {
    for e in body {
        match e {
            Event::Fire { bindings, .. } => {
                if let Some(out_slice) = &bindings.output {
                    tainted.push((out_slice.data, IterTile::empty()));
                }
            }
            // Both w2w Wait and host->worker Wait inside a Loop body
            // taint — see docstring rationale. The `src` field is
            // intentionally not matched on; both arms taint identically.
            Event::Wait { data, .. } => {
                tainted.push((*data, IterTile::empty()));
            }
            Event::Loop { body: inner, .. } => {
                collect_loop_body_writes(inner, tainted);
            }
            _ => {}
        }
    }
}

/// Conservative overlap test: true iff two tiles MAY overlap. Returns
/// false only when there is some shared IterVar axis on which the
/// ranges are definitively disjoint. Empty tiles (top-level, non-
/// iterated firings) are treated as may-overlap-with-anything.
fn tiles_may_overlap(a: &IterTile, b: &IterTile) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    let a_map: BTreeMap<IterVar, &Range<i64>> =
        a.bounds.iter().map(|(v, r)| (*v, r)).collect();
    let b_map: BTreeMap<IterVar, &Range<i64>> =
        b.bounds.iter().map(|(v, r)| (*v, r)).collect();
    for (v, ra) in &a_map {
        if let Some(rb) = b_map.get(v) {
            if ra.end <= rb.start || rb.end <= ra.start {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{FireBinding, KernelId, SeqTag, SyncKind, SyncTag};
    use std::collections::BTreeSet;

    fn host() -> WorkerId {
        WorkerId(0)
    }
    fn w1() -> WorkerId {
        WorkerId(1)
    }
    fn w2() -> WorkerId {
        WorkerId(2)
    }
    fn iv(i: u64) -> IterVar {
        IterVar(i)
    }
    fn d(i: u64) -> DataId {
        DataId(i)
    }
    fn seq(i: u64) -> SeqTag {
        SeqTag(i)
    }
    fn tile_1d(v: IterVar, lo: i64, hi: i64) -> IterTile {
        IterTile::new(vec![(v, lo..hi)])
    }
    fn push(dst: WorkerId, data: DataId, tile: IterTile, s: SeqTag) -> Event {
        Event::Push {
            dst,
            data,
            tile,
            seq: s,
        }
    }
    fn wait(src: WorkerId, data: DataId, tile: IterTile, s: SeqTag) -> Event {
        Event::Wait {
            src,
            data,
            tile,
            seq: s,
        }
    }
    fn sync_at(tag: u64, parts: &[WorkerId]) -> Event {
        let mut p = BTreeSet::new();
        for w in parts {
            p.insert(*w);
        }
        Event::Sync {
            participants: p,
            kind: SyncKind::Barrier,
            sync: SyncTag(tag),
        }
    }
    fn fire(k: u64) -> Event {
        Event::Fire {
            kernel: KernelId(k),
            tile: IterTile::empty(),
            bindings: FireBinding::default(),
        }
    }

    fn run_one(events: Vec<Event>) -> Vec<Event> {
        let mut pw = BTreeMap::new();
        pw.insert(w1(), events);
        let out = apply_safe_push_reorder(pw, host());
        out.into_iter().next().unwrap().1
    }

    #[test]
    fn host_events_unchanged() {
        // Host events are not reordered even if they contain Push/Wait.
        let mut pw = BTreeMap::new();
        pw.insert(
            host(),
            vec![
                wait(w1(), d(1), tile_1d(iv(0), 0, 4), seq(1)),
                push(w2(), d(2), tile_1d(iv(0), 0, 4), seq(2)),
            ],
        );
        let before = pw.clone();
        let after = apply_safe_push_reorder(pw, host());
        assert_eq!(before, after, "host's events must be unchanged");
    }

    #[test]
    fn no_w2w_events_is_noop() {
        let events = vec![
            wait(host(), d(1), tile_1d(iv(0), 0, 4), seq(1)),
            fire(0),
            sync_at(0, &[host(), w1()]),
            fire(1),
        ];
        let out = run_one(events.clone());
        assert_eq!(out, events, "no w2w events → unchanged");
    }

    #[test]
    fn w2w_wait_then_w2w_push_same_data_disjoint_tiles_hoists() {
        // 05/distributed-2d shape: halo wait from w2 then halo push to w2,
        // same data (`img_in`) but disjoint tiles (own_strip vs neighbor_strip).
        let neighbor_strip = tile_1d(iv(0), 0, 1); // first column of neighbor
        let own_strip = tile_1d(iv(0), 3, 4); // last column of own tile (disjoint from neighbor_strip)
        let events = vec![
            wait(w2(), d(1), neighbor_strip.clone(), seq(1)),
            push(w2(), d(1), own_strip.clone(), seq(2)),
        ];
        let out = run_one(events);
        assert!(matches!(out[0], Event::Push { .. }), "Push must be hoisted");
        assert!(matches!(out[1], Event::Wait { .. }), "Wait stays second");
        if let Event::Push { tile, .. } = &out[0] {
            assert_eq!(tile, &own_strip);
        }
    }

    #[test]
    fn w2w_wait_then_w2w_push_same_data_overlapping_tiles_does_not_hoist() {
        // Chained transfer: Push payload IS the Wait result. Disjoint
        // would have been [0,4) wait then [0,4) push reusing — same tile
        // → may-overlap → not hoistable.
        let tile = tile_1d(iv(0), 0, 4);
        let events = vec![
            wait(w2(), d(1), tile.clone(), seq(1)),
            push(w2(), d(1), tile.clone(), seq(2)),
        ];
        let out = run_one(events.clone());
        assert_eq!(out, events, "overlapping tile → Push not hoisted");
    }

    #[test]
    fn host_wait_then_w2w_push_same_boundary_does_not_hoist() {
        // Cycle-162 design draft (AC#1 P1.1) excluded host->worker Wait
        // from tainting. Empirical cycle-162a investigation revised:
        // a host->worker Wait in the SAME boundary as a w2w Push of
        // the same data IS a true dependency (the Push reads memory
        // the Wait wrote). Conservative-but-sound: taint on
        // host->worker Wait too.
        //
        // For the cycle-161/161b target cell (05/distributed-2d), the
        // projection places host->worker Wait of img_in AFTER bar_0
        // (different boundary from the halo Push/Wait pre-bar_0), so
        // this taint is DORMANT — boundary isolation (see
        // `boundary_isolation_sync_separates`) prevents the cross-
        // boundary taint. The hoist still happens because no
        // host->worker Wait is in the same boundary.
        let tile = tile_1d(iv(0), 0, 4);
        let events = vec![
            wait(host(), d(1), tile.clone(), seq(1)),
            push(w2(), d(1), tile.clone(), seq(2)),
        ];
        let out = run_one(events.clone());
        assert_eq!(
            out, events,
            "host->worker Wait in same boundary taints — Push NOT hoisted (conservative)"
        );
    }

    #[test]
    fn host_wait_in_previous_boundary_does_not_block_hoist() {
        // The 05/distributed-2d shape: host->worker Wait of img_in
        // happens AFTER bar_0; the halo Push/Wait pre-bar_0 is in a
        // DIFFERENT boundary. Boundary isolation (Sync events
        // discharge cross-boundary dependencies) lets the halo Push
        // hoist above the w2w halo Wait without being blocked by the
        // post-bar_0 host->worker Wait.
        //
        // Pre-bar_0 boundary: [Wait halo w2w, Push halo w2w] with
        // disjoint tiles (own_strip vs neighbor_strip).
        // bar_0
        // Post-bar_0 boundary: [Wait host img_in].
        //
        // Expected post-reorder: [Push, Wait, bar_0, Wait host].
        let neighbor = tile_1d(iv(0), 0, 1);
        let own = tile_1d(iv(0), 3, 4);
        let events = vec![
            wait(w2(), d(1), neighbor, seq(1)),
            push(w2(), d(1), own.clone(), seq(2)),
            sync_at(0, &[host(), w1(), w2()]),
            wait(host(), d(1), tile_1d(iv(0), 0, 4), seq(3)),
        ];
        let out = run_one(events);
        assert_eq!(out.len(), 4);
        if let Event::Push { tile, .. } = &out[0] {
            assert_eq!(tile, &own, "Push hoisted to position 0");
        } else {
            panic!("expected Push first");
        }
        assert!(matches!(out[1], Event::Wait { .. }));
        assert!(matches!(out[2], Event::Sync { .. }));
        assert!(matches!(out[3], Event::Wait { .. }));
    }

    #[test]
    fn boundary_isolation_sync_separates() {
        // Different boundaries: a Wait in boundary 1 does NOT taint a
        // Push in boundary 2 (the Sync barrier discharges the dependency).
        let tile = tile_1d(iv(0), 0, 4);
        let events = vec![
            wait(w2(), d(1), tile.clone(), seq(1)),
            sync_at(0, &[host(), w1(), w2()]),
            push(w2(), d(1), tile.clone(), seq(2)),
        ];
        let out = run_one(events);
        // Boundary 1: [Wait] → unchanged.
        // Sync: stays.
        // Boundary 2: [Push] → hoistable (no preceding Wait in this boundary).
        // But there are no other events in boundary 2 to move past.
        // Order should be preserved.
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Event::Wait { .. }));
        assert!(matches!(out[1], Event::Sync { .. }));
        assert!(matches!(out[2], Event::Push { .. }));
    }

    #[test]
    fn full_05_distributed_2d_shape() {
        // Pre-reorder: Wait, Wait, Push, Push, Sync.
        // The two pushes' tiles are own-strips; the two waits' tiles
        // are neighbor-strips — same DataId (img_in), disjoint tiles.
        let neighbor1 = tile_1d(iv(0), 0, 1);
        let neighbor2 = tile_1d(iv(0), 7, 8);
        let own1 = tile_1d(iv(0), 3, 4);
        let own2 = tile_1d(iv(0), 4, 5);
        let events = vec![
            wait(w2(), d(1), neighbor1.clone(), seq(1)),
            wait(w2(), d(1), neighbor2.clone(), seq(2)),
            push(w2(), d(1), own1.clone(), seq(3)),
            push(w2(), d(1), own2.clone(), seq(4)),
            sync_at(0, &[host(), w1(), w2()]),
        ];
        let out = run_one(events);
        // Expected: [Push own1, Push own2, Wait n1, Wait n2, Sync].
        assert_eq!(out.len(), 5);
        if let Event::Push { tile, .. } = &out[0] {
            assert_eq!(tile, &own1);
        } else {
            panic!("expected Push first, got {:?}", out[0]);
        }
        if let Event::Push { tile, .. } = &out[1] {
            assert_eq!(tile, &own2);
        } else {
            panic!("expected Push second, got {:?}", out[1]);
        }
        assert!(matches!(out[2], Event::Wait { .. }));
        assert!(matches!(out[3], Event::Wait { .. }));
        assert!(matches!(out[4], Event::Sync { .. }));
    }

    #[test]
    fn idempotence_apply_twice_equals_once() {
        // After one pass, the reorder is a fixpoint.
        let neighbor = tile_1d(iv(0), 0, 1);
        let own = tile_1d(iv(0), 3, 4);
        let events = vec![
            wait(w2(), d(1), neighbor.clone(), seq(1)),
            push(w2(), d(1), own.clone(), seq(2)),
        ];
        let mut pw = BTreeMap::new();
        pw.insert(w1(), events);
        let once = apply_safe_push_reorder(pw, host());
        let twice = apply_safe_push_reorder(once.clone(), host());
        assert_eq!(once, twice, "second apply must be a no-op");
    }

    #[test]
    fn loop_body_is_opaque() {
        // Event::Loop is treated as one opaque event at depth 0; not
        // recursed into. A Push inside a Loop body stays inside.
        let tile = tile_1d(iv(0), 0, 4);
        let body_push = push(w2(), d(1), tile.clone(), seq(1));
        let outer = Event::Loop {
            iter_var: iv(1),
            range: 0..16,
            body: vec![body_push.clone()],
            block_tag: None,
            check_frame: None,
        };
        let events = vec![wait(w2(), d(1), tile.clone(), seq(2)), outer.clone()];
        let out = run_one(events);
        // The Wait taints d(1) on tile=[0,4). The Loop event is "other"
        // (not a w2w Push at depth 0), so it stays in original position.
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], Event::Wait { .. }));
        match &out[1] {
            Event::Loop { body, .. } => {
                assert_eq!(body, &vec![body_push]);
            }
            _ => panic!("expected Loop at position 1"),
        }
    }

    #[test]
    fn pure_consumer_worker_unchanged() {
        // A worker with only w2w Waits (no w2w Pushes) is hazard-safe
        // per detect_wait_before_push_hazard's precondition: not a src
        // in the relay schedule. The reorder pass touches its events
        // but produces no change because there are no hoistable Pushes.
        let events = vec![
            wait(w2(), d(1), tile_1d(iv(0), 0, 4), seq(1)),
            fire(0),
            sync_at(0, &[host(), w1()]),
        ];
        let out = run_one(events.clone());
        assert_eq!(out, events, "pure-consumer worker → unchanged");
    }

    #[test]
    fn multiple_boundaries_each_reordered_independently() {
        let neighbor = tile_1d(iv(0), 0, 1);
        let own = tile_1d(iv(0), 3, 4);
        // Boundary 0: Wait then Push (both w2w, disjoint tiles).
        // Sync
        // Boundary 1: Wait then Push (same shape).
        // Sync
        let events = vec![
            wait(w2(), d(1), neighbor.clone(), seq(1)),
            push(w2(), d(1), own.clone(), seq(2)),
            sync_at(0, &[host(), w1(), w2()]),
            wait(w2(), d(2), neighbor.clone(), seq(3)),
            push(w2(), d(2), own.clone(), seq(4)),
            sync_at(1, &[host(), w1(), w2()]),
        ];
        let out = run_one(events);
        // Expected: Push d(1), Wait d(1), Sync, Push d(2), Wait d(2), Sync.
        assert_eq!(out.len(), 6);
        if let Event::Push { data, .. } = &out[0] {
            assert_eq!(*data, d(1));
        } else {
            panic!("expected Push d(1)");
        }
        assert!(matches!(out[1], Event::Wait { .. }));
        assert!(matches!(out[2], Event::Sync { .. }));
        if let Event::Push { data, .. } = &out[3] {
            assert_eq!(*data, d(2));
        } else {
            panic!("expected Push d(2)");
        }
        assert!(matches!(out[4], Event::Wait { .. }));
        assert!(matches!(out[5], Event::Sync { .. }));
    }

    #[test]
    fn tiles_may_overlap_disjoint_axes() {
        let a = tile_1d(iv(0), 0, 5);
        let b = tile_1d(iv(0), 5, 10); // ra.end=5 <= rb.start=5
        assert!(!tiles_may_overlap(&a, &b));
        let c = tile_1d(iv(0), 4, 6); // overlaps a on [4,5)
        assert!(tiles_may_overlap(&a, &c));
    }

    #[test]
    fn tiles_may_overlap_no_shared_axis() {
        let a = tile_1d(iv(0), 0, 5);
        let b = tile_1d(iv(1), 5, 10);
        // No shared axis → conservative may-overlap (true).
        assert!(tiles_may_overlap(&a, &b));
    }

    #[test]
    fn fire_write_taints_subsequent_w2w_push_same_data() {
        // 06-separable-filter/distributed2 shape: worker initialises
        // tmp to zero, Fire (pass-1) writes tmp, then Push tmp w2w.
        // The first slice-1 draft (cycle 162) lacked the Fire-taint
        // and hoisted the Push above the Fire — sending zero `tmp`
        // and breaking the cell. Test guards against the regression.
        use crate::event::DataSlice;
        let tile = tile_1d(iv(0), 0, 4);
        let fire = Event::Fire {
            kernel: KernelId(0),
            tile: tile.clone(),
            bindings: FireBinding {
                inputs: vec![],
                output: Some(DataSlice {
                    data: d(1),
                    indices: vec![],
                }),
            },
        };
        let events = vec![fire, push(w2(), d(1), tile.clone(), seq(1))];
        let out = run_one(events.clone());
        assert_eq!(out, events, "Fire writes d(1) → Push d(1) not hoistable");
    }

    #[test]
    fn fire_write_to_different_data_does_not_taint() {
        // Fire writes `out`, Push pushes `tmp` — different DataIds.
        // Push IS hoistable.
        use crate::event::DataSlice;
        let tile = tile_1d(iv(0), 0, 4);
        let fire = Event::Fire {
            kernel: KernelId(0),
            tile: tile.clone(),
            bindings: FireBinding {
                inputs: vec![],
                output: Some(DataSlice {
                    data: d(2),
                    indices: vec![],
                }),
            },
        };
        let events = vec![
            wait(w2(), d(1), tile_1d(iv(0), 5, 6), seq(2)), // Wait w2w on d(1) tile [5,6)
            fire,                                            // Fire writes d(2)
            push(w2(), d(1), tile.clone(), seq(1)),         // Push d(1) tile [0,4)
        ];
        // Push d(1) tile [0,4) vs Wait d(1) tile [5,6) — disjoint.
        // Fire writes d(2), not d(1) — no taint of d(1).
        // → hoistable.
        let out = run_one(events);
        assert!(
            matches!(out[0], Event::Push { .. }),
            "Push hoists above Wait and Fire when no overlap"
        );
    }

    #[test]
    fn loop_body_fire_taints_subsequent_push() {
        // A Loop whose body writes data D must taint D so a subsequent
        // w2w Push of D is not hoisted above the Loop.
        use crate::event::DataSlice;
        let tile = tile_1d(iv(0), 0, 4);
        let inner_fire = Event::Fire {
            kernel: KernelId(0),
            tile: tile.clone(),
            bindings: FireBinding {
                inputs: vec![],
                output: Some(DataSlice {
                    data: d(1),
                    indices: vec![],
                }),
            },
        };
        let outer_loop = Event::Loop {
            iter_var: iv(1),
            range: 0..8,
            body: vec![inner_fire],
            block_tag: None,
            check_frame: None,
        };
        let events = vec![outer_loop, push(w2(), d(1), tile.clone(), seq(1))];
        let out = run_one(events.clone());
        assert_eq!(out, events, "Loop body writes d(1) → Push d(1) not hoistable");
    }

    #[test]
    fn tiles_may_overlap_empty() {
        let a = IterTile::empty();
        let b = tile_1d(iv(0), 0, 5);
        assert!(tiles_may_overlap(&a, &b));
        assert!(tiles_may_overlap(&b, &a));
    }

    // ---- 2D tile-overlap tests (cycle-162b architect P2.2 fold-back) ----
    //
    // The 05/distributed-2d production cell uses 2D (y, x) iteration;
    // the pass's `tiles_may_overlap` helper is exercised on 2D tiles in
    // practice but the test fixtures pre-cycle-162b were 1D only. The
    // tests below pin 2D behaviour: disjoint-on-x-overlap-on-y,
    // overlap-on-both, missing-axis-in-one.

    fn tile_2d(va: IterVar, ra: Range<i64>, vb: IterVar, rb: Range<i64>) -> IterTile {
        IterTile::new(vec![(va, ra), (vb, rb)])
    }

    #[test]
    fn tiles_may_overlap_2d_disjoint_on_one_axis_overlap_on_other() {
        // 05/distributed-2d halo strip shape: w0's east column tile vs
        // w1's west column tile. y overlaps (both span the same rows);
        // x disjoint (col 7..8 vs col 8..9). → disjoint.
        let own_east = tile_2d(iv(0), 1..8, iv(1), 7..8);
        let neighbor_west = tile_2d(iv(0), 1..8, iv(1), 8..9);
        assert!(!tiles_may_overlap(&own_east, &neighbor_west));
        assert!(!tiles_may_overlap(&neighbor_west, &own_east));
    }

    #[test]
    fn tiles_may_overlap_2d_overlap_on_both_axes() {
        let a = tile_2d(iv(0), 0..4, iv(1), 0..4);
        let b = tile_2d(iv(0), 2..6, iv(1), 2..6); // overlaps a on [2,4) × [2,4)
        assert!(tiles_may_overlap(&a, &b));
    }

    #[test]
    fn tiles_may_overlap_2d_missing_axis_in_one_is_non_restrictive() {
        // 2D tile vs 1D tile that only constrains one axis. The 1D
        // tile's missing axis is treated as non-restrictive (the 2D
        // tile's other axis range is unconstrained from the 1D tile's
        // perspective). Result depends only on the shared axis.
        let a = tile_2d(iv(0), 0..4, iv(1), 0..4); // 2D
        let b = tile_1d(iv(0), 5, 10); // 1D on axis 0, disjoint
        assert!(!tiles_may_overlap(&a, &b)); // disjoint on shared axis 0
        let c = tile_1d(iv(0), 2, 3); // 1D on axis 0, overlapping
        assert!(tiles_may_overlap(&a, &c));
    }

    #[test]
    fn loop_body_host_wait_taints_subsequent_push() {
        // Cycle-162b architect P2.1: a host->worker Wait inside a Loop
        // body writes data at runtime when the Loop executes; a
        // subsequent top-level w2w Push of the same data must not be
        // hoisted above the Loop.
        let tile = tile_1d(iv(0), 0, 4);
        let inner_wait = wait(host(), d(1), tile.clone(), seq(5));
        let outer_loop = Event::Loop {
            iter_var: iv(1),
            range: 0..8,
            body: vec![inner_wait],
            block_tag: None,
            check_frame: None,
        };
        let events = vec![outer_loop, push(w2(), d(1), tile.clone(), seq(1))];
        let out = run_one(events.clone());
        assert_eq!(
            out, events,
            "Loop body contains host->worker Wait of d(1) → Push d(1) not hoistable"
        );
    }
}
