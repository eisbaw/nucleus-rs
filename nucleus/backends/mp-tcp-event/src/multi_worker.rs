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

    /// TASK-0044.03.02.02: golden pin of the transport-divergent
    /// `emit_handshake` block (the most divergent emitted surface).
    /// A behavior-preserving edit to `emit_handshake` is otherwise
    /// caught by NOTHING: e2e checks runtime output not emitted
    /// source, and the integration emit-pinning tests deliberately
    /// strip this prelude (`strip_transport_prelude`). Inputs match
    /// the `worker_program.rs` call site: `non_host_names` is the full
    /// list of non-host workers (the host branch loops it once per
    /// peer; the non-host branch ignores it and uses only `wname`), so
    /// a single-element list pins the whole emitted template.
    /// To regenerate after an INTENTIONAL `emit_handshake` change:
    /// temporarily swap an `assert_eq!` for
    /// `std::fs::write("/tmp/g.txt", &host).unwrap();`, run this test,
    /// and paste `/tmp/g.txt`'s exact bytes back into the matching
    /// `GOLDEN_*` raw-string const.
    #[test]
    fn emit_handshake_golden() {
        use backend_common::event_plan::EventTransport;
        const GOLDEN_HOST: &str = r#"    let rendezvous_dir: PathBuf = std::env::var_os("NUC_RENDEZVOUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("host: NUC_RENDEZVOUS_DIR not set (run.sh must export it)"));
    let _ = &rendezvous_dir;
    let listener_w0 = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|e| panic!("host: bind 127.0.0.1:0 for w0 failed: {e}"));
    let port_w0: u16 = listener_w0.local_addr()
        .unwrap_or_else(|e| panic!("host: local_addr for w0 listener failed: {e}"))
        .port();
    {
        let rdv_final = rendezvous_dir.join("w0.port");
        let rdv_tmp = rendezvous_dir.join("w0.port.tmp");
        let mut f = fs::File::create(&rdv_tmp)
            .unwrap_or_else(|e| panic!("host: create rendezvous tmp `{}` for w0 failed: {e}", rdv_tmp.display()));
        write!(f, "{}", port_w0)
            .unwrap_or_else(|e| panic!("host: write port {} to rendezvous tmp `{}` for w0 failed: {e}", port_w0, rdv_tmp.display()));
        drop(f);
        fs::rename(&rdv_tmp, &rdv_final)
            .unwrap_or_else(|e| panic!("host: rename rendezvous `{}` -> `{}` for w0 failed: {e}", rdv_tmp.display(), rdv_final.display()));
    }
    let (data_w0_std, _) = listener_w0.accept()
        .unwrap_or_else(|e| panic!("host: accept DATA from w0 failed: {e}"));
    let (ctrl_w0_raw, _) = listener_w0.accept()
        .unwrap_or_else(|e| panic!("host: accept CTRL from w0 failed: {e}"));
    data_w0_std.set_nodelay(true).ok();
    ctrl_w0_raw.set_nodelay(true).ok();
    wire::apply_sock_buf(&data_w0_std);
    wire::apply_sock_buf(&ctrl_w0_raw);
    let ctrl_w0: Rc<RefCell<std::net::TcpStream>> = Rc::new(RefCell::new(ctrl_w0_raw));
    data_w0_std.set_nonblocking(true)
        .unwrap_or_else(|e| panic!("host: set_nonblocking on DATA to w0 failed: {e}"));
    let data_w0 = mio::net::TcpStream::from_std(data_w0_std);
"#;
        const GOLDEN_NONHOST: &str = r#"    let rendezvous_dir: PathBuf = std::env::var_os("NUC_RENDEZVOUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("w0: NUC_RENDEZVOUS_DIR not set (run.sh must export it)"));
    let rdv_path = rendezvous_dir.join("w0.port");
    fn read_rendezvous_port(path: &std::path::Path, who: &str) -> u16 {
        let mut attempt = 0u32;
        loop {
            match fs::read_to_string(path) {
              Ok(s) => {
                let trimmed = s.trim();
                return trimmed.parse::<u16>().unwrap_or_else(|e| panic!(
                  "{}: rendezvous file `{}` contained `{}` which is not a u16: {}",
                  who, path.display(), trimmed, e
                ));
              }
              Err(_e) => {
                attempt += 1;
                if attempt > 600 {
                  panic!(
                    "{}: rendezvous file `{}` did not appear within 6s ({} attempts x 10ms) — host worker did not start or failed to bind",
                    who, path.display(), attempt
                  );
                }
                std::thread::sleep(Duration::from_millis(10));
              }
            }
        }
    }
    let port: u16 = read_rendezvous_port(&rdv_path, "w0");
    fn connect_retry(port: u16, role: &str) -> StdTcpStream {
        let mut attempt = 0u32;
        loop {
            match StdTcpStream::connect(("127.0.0.1", port)) {
              Ok(s) => return s,
              Err(e) => {
                attempt += 1;
                if attempt > 600 { panic!("w0: cannot connect {role} to host 127.0.0.1:{port} after {attempt} tries: {e}"); }
                std::thread::sleep(Duration::from_millis(10));
            }
            }
        }
    }
    let data_host_std = connect_retry(port, "DATA");
    let ctrl_host_raw = connect_retry(port, "CTRL");
    data_host_std.set_nodelay(true).ok();
    ctrl_host_raw.set_nodelay(true).ok();
    wire::apply_sock_buf(&data_host_std);
    wire::apply_sock_buf(&ctrl_host_raw);
    data_host_std.set_nonblocking(true)
        .unwrap_or_else(|e| panic!("w0: set_nonblocking on DATA to host failed: {e}"));
    let data_host = mio::net::TcpStream::from_std(data_host_std);
    let ctrl_host: Rc<RefCell<std::net::TcpStream>> = Rc::new(RefCell::new(ctrl_host_raw));
"#;
        let mut host = String::new();
        crate::plan::TcpEventTransport::emit_handshake(&mut host, "host", true, &["w0".to_string()]);
        assert_eq!(host, GOLDEN_HOST, "mp-tcp-event host emit_handshake drifted from golden");
        let mut nonhost = String::new();
        crate::plan::TcpEventTransport::emit_handshake(&mut nonhost, "w0", false, &["w0".to_string()]);
        assert_eq!(
            nonhost, GOLDEN_NONHOST,
            "mp-tcp-event non-host emit_handshake drifted from golden"
        );
    }
}
