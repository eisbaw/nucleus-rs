//! mp-uds-event multi-worker codegen — thin shim over the shared
//! [`backend_common::event_plan`] substrate (TASK-0044.03.02 lift).
//!
//! # Status
//!
//! The multi-worker arm emits one `src/bin/<worker>.rs` per used
//! worker, each containing the mio reactor wired to per-(seq, peer)
//! bounded outbound queues plus per-seq inbound queues. The shared
//! [`backend_common::multi_worker_walker::render_worker_events`] drives
//! the per-event walk with `rendezvous_prefix = "chan"`, so Push/Wait
//! sites lower to `chan_<rid>.push(name.clone())` / `chan_<rid>.wait()`.
//!
//! # Where the code lives
//!
//! The Plan substrate (host election, chan registry, per-pair capacity,
//! peer-index routing, barrier analysis, host-relay codegen, the entire
//! event walk + worker-program assembly) is shared with mp-tcp-event
//! and lives in [`backend_common::event_plan`], parameterised over the
//! [`backend_common::event_plan::EventTransport`] trait. mp-uds-event's
//! Unix-domain-socket `EventTransport` impl + the `Plan` type alias +
//! `render_run_sh` wrapper live in [`crate::plan`]. This module just
//! re-exports them so `lib.rs` (which imports `multi_worker::Plan` +
//! calls `multi_worker::render_run_sh`) resolves unchanged across the
//! lift. Before the lift this was a verbatim copy of mp-tcp-event's
//! `multi_worker/` subtree with the TCP→UDS transport swap; the lift
//! retires that ~3000 LoC duplication (and the silent-sibling-defect
//! surface it carried).
//!
//! # Design notes (transport behaviour, for reviewers)
//!
//! - **Two sockets per (host, worker) pair**: DATA (mio-managed,
//!   non-blocking) for Push/Wait; CTRL (`std::os::unix::net::UnixStream`,
//!   blocking) for barriers via `wire::barrier_cross`. mp-uds-event's
//!   data-channel demultiplex happens by `seq` instead of arrival
//!   order, but barriers still need their own ordered channel.
//! - **Path-as-rendezvous handshake** (cycle 197): host binds a
//!   `UnixListener` per non-host worker at the well-known paths
//!   `$NUC_RENDEZVOUS_DIR/<wname>.{data,ctrl}.sock`. Non-host worker
//!   `connect_retry`s the same paths directly (no port file, no
//!   port-binding race — the path itself IS the rendezvous, which is
//!   structurally SIMPLER than mp-tcp-event's TCP-port rendezvous). The
//!   shared multi_binary run.sh template's `$here`-rooted rendezvous dir
//!   would bust the UDS sun_path 104 byte cap on the e2e harness's deep
//!   scratch hierarchy; the UDS `EventTransport::render_run_sh_post`
//!   impl post-processes run.sh to use a `/tmp`-rooted
//!   `mktemp -d -t nuc-uds-XXXXXXXX` instead (see
//!   [`crate::plan::UdsEventTransport`] + memory
//!   `project-uds-path-cap-rendezvous`). The exact emitted handshake
//!   block lives in [`crate::plan::UdsEventTransport::emit_handshake`].
//! - **Host-excluding barriers**: one CTRL stream per `(host, worker)`
//!   pair, so a barrier whose participants exclude host cannot be
//!   lowered AT THE BACKEND directly. The driver's
//!   `apply_host_mediation_inject` pass (cycle-197-widened to include
//!   mp-uds-event) adds host as a participant to every host-excluding
//!   `Sync` before emit; the `Plan::build` `ContractGap` rejection is
//!   defense-in-depth only. Not integration-pinned in mp-uds-event (the
//!   e2e gate is the behavioural witness); the sibling mp-tcp-event has
//!   a `tests/multi_worker_emit.rs::host_excluding_barrier_is_typed_contract_gap`
//!   pin. Porting it to mp-uds-event is left as a defensive-coverage
//!   follow-up if the upstream mediation gate is ever loosened.
//! - **Worker-to-worker `Push`/`Wait`** (TASK-0327, cycle 149):
//!   DATA-side w↔w lifted via HOST-RELAY (`Plan::render_relay_phase` +
//!   `Reactor::relay_one`).

pub(crate) use crate::plan::{render_run_sh, Plan};

// --------------------------------------------------------------------
// Branch A (used_workers.len() < 2) unit test — Plan::build is
// reachable from inside this crate. Branch A is unreachable from the
// public `emit()` because the lib.rs dispatch routes
// `used_workers.len() <= 1` to the single-worker arm BEFORE Plan::build
// is ever called. The only way to exercise Branch A is to call
// Plan::build directly from inside this crate.
//
// Branches B/C/D have e2e coverage on 2+ workers (the integration
// witness for them lives in the e2e harness, not a dedicated unit test
// in mp-uds-event/tests/). Porting the
// `host_excluding_barrier_is_typed_contract_gap` pattern from
// mp-tcp-event/tests/multi_worker_emit.rs is left as a
// defensive-coverage follow-up if the upstream driver mediation gates
// are ever loosened.
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::Plan;
    use crate::{EmitError, NameTables};
    use nucleus_compiler::event::{Event, SyncKind, SyncTag, WorkerId};
    use nucleus_compiler::sidecar::NameSidecar;
    use std::collections::BTreeMap;

    /// Branch A — `Plan::build` must reject single-worker input
    /// (used_workers.len() < 2) with a typed ContractGap naming the
    /// `len() >= 2` invariant. This branch is the gatekeeper that
    /// catches a regression where the lib.rs dispatch arm accidentally
    /// routed single-worker input to `Plan::build` instead of the
    /// single-worker emitter.
    ///
    /// Reachability: only from inside the crate. `emit()` routes
    /// `<=1` worker input to `render_single_worker_main` BEFORE
    /// `Plan::build` is called.
    #[test]
    fn single_worker_input_is_typed_contract_gap() {
        let w_host = WorkerId(0);

        // ONE non-empty worker. used_workers will be `[w_host]`,
        // len 1, < 2 — Branch A fires.
        let host_marker = Event::Sync {
            participants: [w_host].into_iter().collect(),
            kind: SyncKind::Barrier,
            sync: SyncTag(0),
        };
        let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
        per_worker.insert(w_host, vec![host_marker]);

        let mut names = NameTables::default();
        names.worker.insert(w_host, "host".to_string());
        let sidecar = NameSidecar::default();

        let r = Plan::build(&per_worker, &names, &sidecar);
        match r {
            Err(EmitError::ContractGap(msg)) => {
                assert!(
                    msg.contains("used_workers.len() >= 2"),
                    "ContractGap must name the >= 2 invariant: {msg}"
                );
                assert!(
                    msg.contains("Single-worker is handled by emit()"),
                    "ContractGap must point to the single-worker arm as the correct route: {msg}"
                );
                // Backend-prefix pin (mp-uds-event vs mp-tcp-event) —
                // the routed BACKEND_NAME must be the UDS one.
                assert!(
                    msg.contains("mp-uds-event"),
                    "ContractGap must carry the mp-uds-event backend prefix: {msg}"
                );
            }
            Err(other) => {
                panic!("expected ContractGap on single-worker Plan::build; got Err({other:?})")
            }
            Ok(_) => panic!("expected ContractGap on single-worker Plan::build; got Ok(Plan)"),
        }
    }

    /// Edge: ZERO non-empty workers (every Vec is empty). Still
    /// triggers Branch A — used_workers.len() == 0 < 2. The
    /// message's `n=` placeholder must reflect that.
    #[test]
    fn zero_worker_input_is_typed_contract_gap() {
        let w_host = WorkerId(0);
        let w1 = WorkerId(1);
        let mut per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
        // Both entries present but EMPTY — used_workers filter drops
        // empties, leaving 0.
        per_worker.insert(w_host, vec![]);
        per_worker.insert(w1, vec![]);

        let names = NameTables::default();
        let sidecar = NameSidecar::default();

        let r = Plan::build(&per_worker, &names, &sidecar);
        match r {
            Err(EmitError::ContractGap(msg)) => {
                assert!(
                    msg.contains("used_workers.len() >= 2"),
                    "ContractGap must name the >= 2 invariant: {msg}"
                );
                assert!(
                    msg.contains("got 0"),
                    "ContractGap must report the actual count (0 here): {msg}"
                );
            }
            Err(other) => {
                panic!("expected ContractGap on zero-worker Plan::build; got Err({other:?})")
            }
            Ok(_) => panic!("expected ContractGap on zero-worker Plan::build; got Ok(Plan)"),
        }
    }
}
