//! mp-tcp-event multi-worker codegen — thin shim over the shared
//! [`backend_common::event_plan`] substrate (TASK-0044.03.02 lift).
//!
//! # Status
//!
//! The multi-worker arm emits one `src/bin/<worker>.rs` per used
//! worker, each containing the mio reactor wired to per-(seq, peer)
//! bounded outbound queues plus per-seq inbound queues. The shared
//! [`backend_common::multi_worker_walker::render_worker_events`] drives
//! the per-event walk with `rendezvous_prefix = "chan"`, so Push/Wait
//! sites lower to `chan_<rid>.push(name.clone())` / `chan_<rid>.wait()`
//! — the same surface pthreads-async uses for `ring_<id>`.
//!
//! # Where the code lives
//!
//! The Plan substrate (host election, chan registry, per-pair capacity,
//! peer-index routing, barrier analysis, host-relay codegen, the entire
//! event walk + worker-program assembly) is shared with mp-uds-event
//! and lives in [`backend_common::event_plan`], parameterised over the
//! [`backend_common::event_plan::EventTransport`] trait. mp-tcp-event's
//! TCP-loopback `EventTransport` impl + the `Plan` type alias +
//! `render_run_sh` wrapper live in [`crate::plan`]. This module just
//! re-exports them so `lib.rs` (which imports `multi_worker::Plan` +
//! calls `multi_worker::render_run_sh`) resolves unchanged across the
//! lift.
//!
//! # Design notes (transport behaviour, for reviewers)
//!
//! - **Two sockets per (host, worker) pair**: DATA (mio-managed,
//!   non-blocking) for Push/Wait; CTRL (`std::net::TcpStream`,
//!   blocking) for barriers via `wire::barrier_cross`. Producer/consumer
//!   barrier-vs-data ordering can differ on each side of a
//!   `(host,worker)` pair, and a single FIFO would corrupt frame
//!   demuxing. mp-tcp-event's data-channel demultiplex happens by `seq`
//!   instead of arrival order, but barriers still need their own
//!   ordered channel.
//! - **Rendezvous-file handshake** (TASK-0176): host binds
//!   `127.0.0.1:0` ITSELF per non-host worker and atomically publishes
//!   the OS-assigned port to `$NUC_RENDEZVOUS_DIR/<wname>.port`
//!   (tmp + rename). Non-host worker polls the file (600 × 10 ms =
//!   6 s) then `connect_retry`s. NEVER use the deleted
//!   `__nuc_pick_port` helper — its close-then-rebind shape opened a
//!   TOCTOU window that TASK-0176 closed. The exact emitted block lives
//!   in [`crate::plan::TcpEventTransport::emit_handshake`].
//! - **Host-excluding barriers**: one CTRL stream per `(host, worker)`
//!   pair, so a barrier whose participants exclude host cannot be
//!   lowered AT THE BACKEND directly. The driver's
//!   `apply_host_mediation_inject` pass (TASK-0329, Done cycle 160)
//!   adds host as a participant to every host-excluding `Sync` before
//!   emit; the `Plan::build` `ContractGap` rejection is now
//!   defense-in-depth. (Wire-message text still cites TASK-0175 —
//!   test-pinned by `multi_worker_emit::host_excluding_barrier_is_typed_contract_gap`.)
//! - **Worker-to-worker `Push`/`Wait`** (TASK-0327, cycle 149):
//!   DATA-side w↔w lifted via HOST-RELAY (`Plan::render_relay_phase` +
//!   `Reactor::relay_one`).

pub(crate) use crate::plan::{render_run_sh, Plan};

// --------------------------------------------------------------------
// TASK-0255 — Branch A (used_workers.len() < 2) unit test.
//
// `Plan::build` is reachable from inside this crate. Branch A is
// unreachable from the public `emit()` because the lib.rs dispatch
// routes `used_workers.len() <= 1` to the single-worker arm BEFORE
// Plan::build is ever called. The only way to exercise Branch A is to
// call Plan::build directly from inside this crate — hence this
// in-module test.
//
// Branches B/C/D have integration tests in `tests/multi_worker_emit.rs`
// (they ARE reachable from `emit()` on 2+ workers).
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
                // Backend-prefix pin (mp-tcp-event vs mp-uds-event) —
                // the routed BACKEND_NAME must be the TCP one.
                assert!(
                    msg.contains("mp-tcp-event"),
                    "ContractGap must carry the mp-tcp-event backend prefix: {msg}"
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
