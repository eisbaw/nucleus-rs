//! TASK-0329 cycle 160 — host-mediation injection.
//!
//! Add `host` as a participant to every [`ACFGNode::Sync`] whose
//! participant set excludes it. Backend-local: invoked by
//! `mp-tcp-bufsync` / `mp-tcp-event` whose transport topology
//! (one-CTRL-stream-per-`(host, worker)` star) cannot lower a
//! host-excluding barrier without a worker-to-worker mesh. Adding
//! host as a mediating hub turns each host-excluding barrier into a
//! star-shaped N+1-party rendezvous through host, which the existing
//! barrier-shim emitter ([`wire::barrier_cross`]) handles transparently
//! with no per-cell code changes.
//!
//! ## Backends that DO NOT use this pass
//!
//! `pthreads-sync` and `pthreads-async` use `std::sync::Barrier::new(N)`
//! which handles host-excluding barriers natively (shared-memory
//! primitive coordinates only the listed participants). The driver
//! conditionally applies this pass only for the TCP backends.
//!
//! ## Cross-reference
//!
//! - TASK-0327 (cycle 148/149) lifted the DATA arm of the original
//!   combined TASK-0175 filing (worker-to-worker `Push`/`Wait` via
//!   synchronous host-relay). This pass lifts the CTRL arm (the
//!   host-excluding barrier rejection at each TCP backend's
//!   `Plan::build`). The two arms are independently lifted because
//!   they have independent runtime mechanisms.
//! - TASK-0175 — original combined filing; the ContractGap message
//!   text at the backends still cites TASK-0175 (test-pinned).
//! - PRD §6.3.3 / §8.3 — barrier semantics; host participation is a
//!   transport-level concern, not a schedule-level one.
//!
//! ## Correctness sketch
//!
//! After this pass, every barrier in the ACFG includes host. The
//! barrier's logical semantics — "all named participants synchronise"
//! — is preserved: the named participants still rendezvous through
//! host, and host's release happens only after every original
//! participant has crossed. Host adds an extra wait on its own event
//! stream but does no other observable work (no kernel call, no data
//! mutation), so program output is bit-identical.
//!
//! Idempotence: a second application is a no-op (host already in
//! participants after first application).
//!
//! ## Honest limitation
//!
//! This is a transport-level "always-mediate" lift. It does NOT
//! reduce the barrier count, does NOT collapse adjacent mediated
//! barriers, and does NOT optimise for the case where host's
//! mediation is logically redundant (e.g. an internal worker-to-worker
//! barrier whose semantics could be expressed via a different
//! mechanism). All of those are future optimisations.

use crate::acfg::{ACFGNode, SyncPlaceholder, ACFG};
use crate::event::WorkerId;

/// Apply host-mediation injection to the given ACFG.
///
/// For every `Sync` node in the ACFG whose `participants` set does not
/// contain `host`, insert `host` into the set. Returns the modified
/// ACFG; the operation is structurally a no-op if no barrier excludes
/// host.
///
/// Callers (the driver) must invoke this only for backends that
/// require host-mediated barrier topology (mp-tcp-bufsync,
/// mp-tcp-event). pthreads-sync / pthreads-async must NOT apply this
/// pass — their shared-memory barrier primitives handle host-excluding
/// barriers natively.
pub fn apply_host_mediation_inject(mut acfg: ACFG, host: WorkerId) -> ACFG {
    inject_at(&mut acfg.root, host);
    acfg
}

fn inject_at(node: &mut ACFGNode, host: WorkerId) {
    // EXHAUSTIVE: every new `ACFGNode` variant that can transitively
    // contain a `Sync` MUST be added to this match. The compiler
    // catches additions structurally; this comment is the manual
    // reminder when a future variant lands (e.g., an Async / Parallel
    // / Race block) — adding it without mediation would silently let
    // a host-excluding barrier slip past the lift and re-trigger the
    // TASK-0329 ContractGap downstream. See cycle-160 architect P2.2.
    match node {
        ACFGNode::Sync(SyncPlaceholder { participants, .. }) => {
            // BTreeSet::insert is a no-op when the key is already
            // present, so this is structurally idempotent.
            participants.insert(host);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                inject_at(c, host);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            inject_at(body, host);
        }
        // Operations and Xfers have no participant set to mediate.
        ACFGNode::Operation(_) | ACFGNode::Xfer(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acfg::{ACFGNode, SyncPlaceholder};
    use crate::event::{SyncTag, WorkerId};
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

    fn sync_excluding_host() -> SyncPlaceholder {
        let mut parts = BTreeSet::new();
        parts.insert(w1());
        parts.insert(w2());
        SyncPlaceholder {
            participants: parts,
            sync: SyncTag(7),
        }
    }

    fn sync_including_host() -> SyncPlaceholder {
        let mut parts = BTreeSet::new();
        parts.insert(host());
        parts.insert(w1());
        SyncPlaceholder {
            participants: parts,
            sync: SyncTag(3),
        }
    }

    fn empty_acfg(root: ACFGNode) -> ACFG {
        ACFG {
            root,
            name_kernels: Default::default(),
            name_data: Default::default(),
            name_workers: Default::default(),
            name_iter_vars: Default::default(),
            inner_block_iter_vars: Default::default(),
            partition_worker_ranges: Default::default(),
            pipeline_depth_for_seq: Default::default(),
            halo_widths: Default::default(),
            reuse_widths: Default::default(),
            partition_pairs: Default::default(),
            grid_shape_for_outer_iv: Default::default(),
        }
    }

    #[test]
    fn host_excluding_sync_at_top_level_is_mediated() {
        let acfg = empty_acfg(ACFGNode::Sync(sync_excluding_host()));
        let out = apply_host_mediation_inject(acfg, host());
        match &out.root {
            ACFGNode::Sync(s) => {
                assert!(s.participants.contains(&host()), "host must be inserted");
                assert!(s.participants.contains(&w1()), "w1 must stay");
                assert!(s.participants.contains(&w2()), "w2 must stay");
                assert_eq!(s.sync, SyncTag(7), "tag must be unchanged");
            }
            _ => panic!("expected Sync at root"),
        }
    }

    #[test]
    fn host_including_sync_is_unchanged() {
        let original = sync_including_host();
        let original_parts = original.participants.clone();
        let acfg = empty_acfg(ACFGNode::Sync(original));
        let out = apply_host_mediation_inject(acfg, host());
        match &out.root {
            ACFGNode::Sync(s) => {
                assert_eq!(
                    s.participants, original_parts,
                    "host-including sync's participants must be byte-identical"
                );
            }
            _ => panic!("expected Sync at root"),
        }
    }

    #[test]
    fn sync_inside_sequence_is_mediated() {
        let root = ACFGNode::Sequence(vec![
            ACFGNode::Sync(sync_excluding_host()),
            ACFGNode::Sync(sync_including_host()),
        ]);
        let out = apply_host_mediation_inject(empty_acfg(root), host());
        match &out.root {
            ACFGNode::Sequence(children) => {
                assert_eq!(children.len(), 2);
                if let ACFGNode::Sync(s) = &children[0] {
                    assert!(s.participants.contains(&host()), "first sync gains host");
                } else {
                    panic!("first child must be Sync");
                }
            }
            _ => panic!("expected Sequence at root"),
        }
    }

    #[test]
    fn sync_inside_repeat_body_is_mediated() {
        use crate::event::IterVar;
        let body = ACFGNode::Sync(sync_excluding_host());
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(body),
            block_tag: None,
        };
        let out = apply_host_mediation_inject(empty_acfg(root), host());
        match &out.root {
            ACFGNode::Repeat { body, .. } => {
                if let ACFGNode::Sync(s) = body.as_ref() {
                    assert!(
                        s.participants.contains(&host()),
                        "host must be inserted into Sync inside Repeat body"
                    );
                } else {
                    panic!("Repeat body must be Sync");
                }
            }
            _ => panic!("expected Repeat at root"),
        }
    }

    #[test]
    fn idempotence_one_pass_equals_two_passes() {
        let root = ACFGNode::Sequence(vec![
            ACFGNode::Sync(sync_excluding_host()),
            ACFGNode::Sync(sync_including_host()),
        ]);
        let once = apply_host_mediation_inject(empty_acfg(root.clone()), host());
        let twice = apply_host_mediation_inject(once.clone(), host());
        assert_eq!(once.root, twice.root, "second pass must be a no-op");
    }

    #[test]
    fn idempotence_with_projection_acfg_to_events_is_stable() {
        // Compositional idempotence: the pass + acfg_to_events
        // together are stable under re-application of the pass. Guards
        // against a future projection change that interacts with the
        // mediation (e.g., a new ACFGNode variant whose projection
        // would otherwise re-introduce a host-excluding Sync into
        // per_worker). See cycle-160 architect P2.1.
        use crate::passes::petri_to_events::acfg_to_events;
        let root = ACFGNode::Sequence(vec![
            ACFGNode::Sync(sync_excluding_host()),
            ACFGNode::Sync(sync_including_host()),
        ]);
        let once = apply_host_mediation_inject(empty_acfg(root), host());
        let projected_once = acfg_to_events(&once);
        let twice = apply_host_mediation_inject(once, host());
        let projected_twice = acfg_to_events(&twice);
        assert_eq!(
            projected_once, projected_twice,
            "acfg_to_events(apply) == acfg_to_events(apply ∘ apply)"
        );
    }

    #[test]
    fn composed_with_petri_to_events_projects_sync_to_host() {
        // End-to-end pipeline sanity: host_mediation_inject + then
        // acfg_to_events together must place a Sync on host's
        // projected event list for every formerly-host-excluding
        // barrier. This is the contract the mp-tcp-bufsync /
        // mp-tcp-event Plan::build relies on (rejection guard checks
        // every Sync includes host; after this pass, every Sync does).
        use crate::event::Event;
        use crate::passes::petri_to_events::acfg_to_events;

        let root = ACFGNode::Sync(sync_excluding_host());
        let acfg = empty_acfg(root);
        let mediated = apply_host_mediation_inject(acfg, host());
        let per_worker = acfg_to_events(&mediated);

        // Host now has an event list, and its single Event::Sync
        // names host in its participants.
        let host_events = per_worker
            .get(&host())
            .expect("host must have a projected event list after mediation");
        assert_eq!(
            host_events.len(),
            1,
            "host's projected event list must carry exactly the mediated Sync"
        );
        match &host_events[0] {
            Event::Sync { participants, .. } => {
                assert!(
                    participants.contains(&host()),
                    "host's projected Sync must include host as a participant"
                );
                assert!(
                    participants.contains(&w1()) && participants.contains(&w2()),
                    "original participants must be preserved"
                );
            }
            other => panic!("expected Event::Sync, got {other:?}"),
        }
        // Symmetrically, the non-host workers' Syncs also carry host.
        for w in [w1(), w2()] {
            let evs = per_worker.get(&w).expect("worker must have events");
            match &evs[0] {
                Event::Sync { participants, .. } => {
                    assert!(
                        participants.contains(&host()),
                        "worker {w:?}'s Sync must include host (the mutation is shared)"
                    );
                }
                other => panic!("expected Event::Sync on worker {w:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn no_sync_nodes_is_structural_noop() {
        // A leaf Xfer / Operation: the pass walks past it without
        // mutating anything (and without panicking).
        use crate::acfg::{XferPlaceholder, XferRole};
        use crate::event::{DataId, IterTile, SeqTag};
        let xfer = ACFGNode::Xfer(XferPlaceholder {
            src: host(),
            dst: w1(),
            data: DataId(0),
            role: XferRole::Push,
            seq: SeqTag(0),
            tile: IterTile::new(vec![]),
            policy: Default::default(),
        });
        let out = apply_host_mediation_inject(empty_acfg(xfer.clone()), host());
        assert_eq!(out.root, xfer, "Xfer-only ACFG must be unchanged");
    }
}
