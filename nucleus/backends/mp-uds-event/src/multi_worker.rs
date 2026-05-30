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
    fn check_uds_path_len(p: &std::path::Path, who: &str) {
        // AC#9 (cycle-197 TASK-0044.03.01): UDS sun_path is
        // capped at 108 bytes on Linux glibc and 104 on
        // musl/macOS. Use the smaller cap to defend against
        // silent breakage when a generated project is run
        // on a musl distro after development on glibc.
        const UDS_PATH_CAP: usize = 104;
        let bytes = p.as_os_str().len();
        if bytes >= UDS_PATH_CAP {
            panic!(
                "{who}: UDS rendezvous path `{path}` is {bytes} bytes long, which meets-or-exceeds the {cap}-byte UDS sun_path cap (musl/macOS limit; Linux glibc is 108 but we take the smaller for portability + margin). Shorten NUC_RENDEZVOUS_DIR or move scratch to /tmp.",
                who = who, path = p.display(), bytes = bytes, cap = UDS_PATH_CAP
            );
        }
    }
    let data_path_w0 = rendezvous_dir.join("w0.data.sock");
    let ctrl_path_w0 = rendezvous_dir.join("w0.ctrl.sock");
    check_uds_path_len(&data_path_w0, "host: bind DATA to w0");
    check_uds_path_len(&ctrl_path_w0, "host: bind CTRL to w0");
    let data_listener_w0 = UnixListener::bind(&data_path_w0)
        .unwrap_or_else(|e| panic!("host: bind DATA UDS `{}` for w0 failed: {e}", data_path_w0.display()));
    let ctrl_listener_w0 = UnixListener::bind(&ctrl_path_w0)
        .unwrap_or_else(|e| panic!("host: bind CTRL UDS `{}` for w0 failed: {e}", ctrl_path_w0.display()));
    let (data_w0_std, _) = data_listener_w0.accept()
        .unwrap_or_else(|e| panic!("host: accept DATA from w0 failed: {e}"));
    let (ctrl_w0_raw, _) = ctrl_listener_w0.accept()
        .unwrap_or_else(|e| panic!("host: accept CTRL from w0 failed: {e}"));
    wire::apply_sock_buf(&data_w0_std);
    wire::apply_sock_buf(&ctrl_w0_raw);
    let ctrl_w0: Rc<RefCell<std::os::unix::net::UnixStream>> = Rc::new(RefCell::new(ctrl_w0_raw));
    data_w0_std.set_nonblocking(true)
        .unwrap_or_else(|e| panic!("host: set_nonblocking on DATA to w0 failed: {e}"));
    let data_w0 = mio::net::UnixStream::from_std(data_w0_std);
"#;
        const GOLDEN_NONHOST: &str = r#"    let rendezvous_dir: PathBuf = std::env::var_os("NUC_RENDEZVOUS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("w0: NUC_RENDEZVOUS_DIR not set (run.sh must export it)"));
    let data_path = rendezvous_dir.join("w0.data.sock");
    let ctrl_path = rendezvous_dir.join("w0.ctrl.sock");
    fn check_uds_path_len(p: &std::path::Path, who: &str) {
        const UDS_PATH_CAP: usize = 104;
        let bytes = p.as_os_str().len();
        if bytes >= UDS_PATH_CAP {
            panic!(
                "{who}: UDS rendezvous path `{path}` is {bytes} bytes long, which meets-or-exceeds the {cap}-byte UDS sun_path cap (musl/macOS limit). Shorten NUC_RENDEZVOUS_DIR.",
                who = who, path = p.display(), bytes = bytes, cap = UDS_PATH_CAP
            );
        }
    }
    check_uds_path_len(&data_path, "w0: connect DATA");
    check_uds_path_len(&ctrl_path, "w0: connect CTRL");
    fn connect_retry(path: &std::path::Path, who: &str, role: &str) -> StdUnixStream {
        let mut attempt = 0u32;
        loop {
            match StdUnixStream::connect(path) {
              Ok(s) => return s,
              Err(e) => {
                attempt += 1;
                if attempt > 600 { panic!("{who}: cannot connect {role} UDS `{path}` after {attempt} tries: {err}", who = who, role = role, path = path.display(), attempt = attempt, err = e); }
                std::thread::sleep(Duration::from_millis(10));
            }
            }
        }
    }
    let data_host_std = connect_retry(&data_path, "w0", "DATA");
    let ctrl_host_raw = connect_retry(&ctrl_path, "w0", "CTRL");
    wire::apply_sock_buf(&data_host_std);
    wire::apply_sock_buf(&ctrl_host_raw);
    data_host_std.set_nonblocking(true)
        .unwrap_or_else(|e| panic!("w0: set_nonblocking on DATA to host failed: {e}"));
    let data_host = mio::net::UnixStream::from_std(data_host_std);
    let ctrl_host: Rc<RefCell<std::os::unix::net::UnixStream>> = Rc::new(RefCell::new(ctrl_host_raw));
"#;
        let mut host = String::new();
        crate::plan::UdsEventTransport::emit_handshake(
            &mut host,
            "host",
            true,
            &["w0".to_string()],
        );
        assert_eq!(
            host, GOLDEN_HOST,
            "mp-uds-event host emit_handshake drifted from golden"
        );
        let mut nonhost = String::new();
        crate::plan::UdsEventTransport::emit_handshake(
            &mut nonhost,
            "w0",
            false,
            &["w0".to_string()],
        );
        assert_eq!(
            nonhost, GOLDEN_NONHOST,
            "mp-uds-event non-host emit_handshake drifted from golden"
        );
    }
}
