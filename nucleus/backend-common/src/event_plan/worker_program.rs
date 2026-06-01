//! Per-worker `src/bin/<wname>.rs` body emit + the multi-process
//! `run.sh` renderer, for the shared async event-reactor substrate.
//!
//! Drives the shared
//! [`crate::multi_worker_walker::render_worker_events`] walker with
//! `rendezvous_prefix = "chan"` and emits the surrounding prelude:
//! the transport-specific rendezvous handshake (delegated wholesale to
//! [`EventTransport::emit_handshake`]), the Reactor + per-chan
//! instances (TASK-0233 sized from sidecar), per-barrier CTRL-channel
//! shim structs (TASK-0172 contract-carried SyncTag identity), and the
//! TASK-0327 host-relay phase splice for w2w transfers.
//!
//! Per-backend variation (all via [`EventTransport`]):
//! - the emitted file-header provenance line ([`EventTransport::FILE_HEADER_PREFIX`]);
//! - the role-specific `use` imports ([`EventTransport::emit_role_imports`]);
//! - the rendezvous handshake + socket setup ([`EventTransport::emit_handshake`]);
//! - the CTRL std-stream type in the barrier-shim structs
//!   ([`EventTransport::CTRL_STD_STREAM_TY`]);
//! - the DATA mio-stream type in the reactor's `peers` Vec
//!   ([`EventTransport::DATA_MIO_STREAM_TY`]);
//! - the `run.sh` post-processing + SO_BUF commentary
//!   ([`EventTransport::render_run_sh_post`] / [`EventTransport::SO_BUF_COMMENT`]).
//!
//! Lifted from the two backends' verbatim-duplicate
//! `multi_worker/{worker_program.rs, mod.rs render_run_sh}`
//! (TASK-0044.03.02).

use std::fmt::Write as _;

use nucleus_compiler::event::WorkerId;

use crate::check_frame::{
    collect_count_check_frames, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static,
};
use crate::event_plan::encode::encode_decode_paths;
use crate::event_plan::walkers::relay_phase_insertion_point;
use crate::event_plan::{EventTransport, Plan};
use crate::multi_worker_walker::{self as walker, WalkerCtx};
use crate::project_skeleton::multi_binary::render_run_sh_multi;
use crate::render::{render_array_init_for, rust_type_of};
use crate::EmitError;

impl<T: EventTransport> Plan<'_, T> {
    /// Render one worker's full `src/bin/<wname>.rs`.
    pub fn render_worker_program(&self, worker: WorkerId) -> Result<String, EmitError> {
        let mut out = String::new();
        let wname = self.worker_name(worker);
        let is_host = worker == self.host_worker;

        // ---- File header + modules. ----
        // The backend supplies the header PREFIX (TCP/UDS wording
        // genuinely differs); the worker-name + role suffix is shared
        // and appended structurally here.
        writeln!(
            out,
            "{prefix} Worker `{wname}`{role}.",
            prefix = T::FILE_HEADER_PREFIX,
            role = if is_host {
                " [host/server]"
            } else {
                " [client]"
            },
        )
        .ok();
        writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
        writeln!(out).ok();
        writeln!(out, "#[path = \"../kernels.rs\"]").ok();
        writeln!(out, "#[allow(dead_code)]").ok();
        writeln!(out, "mod kernels;").ok();
        writeln!(out, "#[path = \"../wire.rs\"]").ok();
        writeln!(out, "#[allow(dead_code)]").ok();
        writeln!(out, "mod wire;").ok();
        writeln!(out, "#[path = \"../runtime.rs\"]").ok();
        writeln!(out, "#[allow(dead_code)]").ok();
        writeln!(out, "mod runtime;").ok();
        writeln!(out).ok();

        // ---- Imports (role-specific so warning-clean). ----
        // The transport-specific lines come from the backend; the
        // shared RefCell + Rc pair is emitted here.
        T::emit_role_imports(&mut out, is_host);
        writeln!(out, "use std::cell::RefCell;").ok();
        writeln!(out, "use std::rc::Rc;").ok();
        writeln!(out).ok();

        // ---- Count check_frame substrate (file scope). ----
        let count_frames = collect_count_check_frames(&self.per_worker[&worker]);
        if !count_frames.is_empty() {
            emit_count_reporter_struct(&mut out);
            for cf in &count_frames {
                emit_count_static(&mut out, &cf.ident);
            }
            writeln!(out).ok();
        }

        // ---- main(). ----
        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, \
             unused_assignments, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();

        // Per-Count-loop Drop guard local.
        for cf in &count_frames {
            emit_count_guard_local(&mut out, &cf.ident, &cf.loop_var, cf.latency_max_ns);
        }
        if !count_frames.is_empty() {
            writeln!(out).ok();
        }

        // ---- Rendezvous handshake + DATA/CTRL socket setup. ----
        // Delegated wholesale to the backend (genuine transport
        // divergence: TCP port-in-file vs UDS path-as-rendezvous).
        let non_host_names: Vec<String> = self
            .non_host_workers()
            .iter()
            .map(|w| self.worker_name(*w))
            .collect();
        T::emit_handshake(&mut out, &wname, is_host, &non_host_names);
        writeln!(out).ok();

        // ---- Build the Reactor (mio-managed DATA sockets) + chan
        //      instances. ----
        self.emit_reactor_and_chans(&mut out, worker)?;
        writeln!(out).ok();

        // ---- Pre-init locals (Wait targets + indexed Fire writes). ----
        //
        // TASK-0349 cycle 220: whole-array-recv-only data EXCLUDED
        // from pre-init and emitted as `let <name> = <rhs>;` at recv
        // site (see `collect_pre_init` doc).
        let (pre_init, let_at_wait) = self.collect_pre_init(worker)?;
        for (name, did) in &pre_init {
            let ty = self.sidecar.data_type(*did).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "pre-init data `{name}` ({did:?}) has no ResolvedType in sidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            let init = render_array_init_for(ty);
            writeln!(out, "    let mut {name}: {rty} = {init};").ok();
        }
        if !pre_init.is_empty() {
            writeln!(out).ok();
        }

        // ---- Drive the shared walker. ----
        let walker_ctx = WalkerCtx {
            names: self.names,
            sidecar: self.sidecar,
            rendezvous_prefix: "chan",
            rendezvous_ids: &self.chan_ids,
            pair_tiles: &self.pair_tiles,
            accumulate_waits: &self.accumulate_waits,
            let_at_wait_data: &let_at_wait,
        };
        // The shared walker is reused VERBATIM for ALL event variants,
        // including Event::Sync — there is NO separate event dispatch
        // here (the walker's Event::Sync arm emits `{prefix}bar_<tag>.wait()`
        // exactly as for the thread backends). The only event-backend
        // difference is what `bar_<tag>` resolves to: barriers in the
        // event backends go through a CTRL-channel (wire::barrier_cross)
        // rather than a `std::sync::Barrier`. We bridge that WITHOUT
        // touching the walker by declaring a `bar_<tag>` local on this
        // worker — a small CTRL-channel shim whose `.wait()` calls
        // wire::barrier_cross on the CTRL socket. All event-backend-
        // specific work therefore lives in the emitted prelude, not in
        // any per-event match (so no Event variant can be dropped here).
        //
        // See emit_barrier_shims below.
        self.emit_barrier_shims(&mut out, worker, is_host)?;
        if !self.barriers_used_by(worker).is_empty() {
            writeln!(out).ok();
        }

        // TASK-0327 (cycle 149): for HOST, when the schedule has w2w
        // pushes, splice the synchronous relay phase right BEFORE the
        // LAST top-level `Event::Sync` event (= between pass-1 barrier
        // and pass-2 barrier on the 06/distributed2 reproducer). The
        // relay phase drains all worker-to-worker (seq, dst) hops
        // through host's existing reactor: `wait(seq)` blocks driving
        // the reactor (pulling frames from `data_<src>` mio sockets
        // into `inbound[seq]`); `push(seq, dst_peer, payload, cap)`
        // enqueues to `outbound[(seq, dst_peer)]` (drained on next
        // reactor turn into `data_<dst>` mio socket). Non-host workers
        // render unchanged — their `chan_<rid>` uses peer_idx=0 (host)
        // for both push and wait; host does the forwarding.
        let host_events = &self.per_worker[&worker];
        let host_relay = if is_host {
            self.render_relay_phase(1)?
        } else {
            String::new()
        };
        if is_host && !host_relay.is_empty() {
            let split_at =
                relay_phase_insertion_point(host_events, self.per_worker, self.host_worker);
            let mut pre = String::new();
            walker::render_worker_events(
                &walker_ctx,
                worker,
                &host_events[..split_at],
                &mut pre,
                1,
                "",
            )?;
            out.push_str(&pre);
            out.push_str(&host_relay);
            let mut post = String::new();
            walker::render_worker_events(
                &walker_ctx,
                worker,
                &host_events[split_at..],
                &mut post,
                1,
                "",
            )?;
            out.push_str(&post);
        } else {
            let body = {
                let mut buf = String::new();
                walker::render_worker_events(&walker_ctx, worker, host_events, &mut buf, 1, "")?;
                buf
            };
            out.push_str(&body);
        }

        // ---- Flush outbound + close. ----
        writeln!(out).ok();
        writeln!(out, "    reactor.borrow_mut().flush_outbound();").ok();

        writeln!(out, "}}").ok();
        Ok(out)
    }

    /// Build the Reactor with peer DATA sockets + the per-chan
    /// instances sized from the sidecar. The peer DATA-stream type is
    /// the only per-backend variation ([`EventTransport::DATA_MIO_STREAM_TY`]).
    fn emit_reactor_and_chans(&self, out: &mut String, worker: WorkerId) -> Result<(), EmitError> {
        // Build the peers Vec for Reactor::new in deterministic order.
        let peers: Vec<WorkerId> = if worker == self.host_worker {
            self.non_host_workers()
        } else {
            vec![self.host_worker]
        };
        writeln!(out, "    let reactor = {{").ok();
        writeln!(
            out,
            "        let peers: Vec<({stream}, String)> = vec![",
            stream = T::DATA_MIO_STREAM_TY,
        )
        .ok();
        for p in &peers {
            let pn = self.worker_name(*p);
            // The DATA socket variable name depends on the role:
            // host -> data_<peer_name>, non-host -> data_host.
            let var = if worker == self.host_worker {
                format!("data_{pn}")
            } else {
                "data_host".to_string()
            };
            writeln!(out, "            ({var}, \"{pn}\".to_string()),").ok();
        }
        writeln!(out, "        ];").ok();
        writeln!(
            out,
            "        Rc::new(RefCell::new(runtime::Reactor::new(peers)))"
        )
        .ok();
        writeln!(out, "    }};").ok();
        writeln!(out).ok();

        // Per-chan instances: chan_<rid> = Chan::new(reactor, seq,
        // peer_idx, cap, encode, decode).
        let touched = self.worker_chans(worker);
        for (key, rid) in &self.chan_ids {
            if !touched.contains(rid) {
                continue;
            }
            let (data, seq) = *key;
            let cap =
                self.chan_caps.get(key).copied().ok_or_else(|| {
                    EmitError::ContractGap(format!("missing chan_cap for {key:?}"))
                })?;
            let ty = self.sidecar.data_type(data).ok_or_else(|| {
                EmitError::ContractGap(format!("cross-worker data {data:?} has no ResolvedType"))
            })?;
            let rty = rust_type_of(ty);
            let peer_idx = self.chan_peer_index(worker, *key)?;
            let (enc, dec) = encode_decode_paths(ty);
            writeln!(
                out,
                "    let chan_{rid}: runtime::Chan<{rty}> = runtime::Chan::new(\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20Rc::clone(&reactor), {seq}u64, {peer_idx}usize, \
                 {cap}usize, {enc}, {dec},\n\
                 \x20\x20\x20\x20);",
                seq = seq.0,
                peer_idx = peer_idx,
                cap = cap,
            )
            .ok();
        }
        Ok(())
    }

    /// Emit the per-barrier shim locals. The shared walker emits
    /// `bar_<bid>.wait()`; the event backends map that to a struct
    /// whose `.wait()` method calls `wire::barrier_cross` on the CTRL
    /// socket(s) for this barrier's participants. Host crosses with
    /// every non-host participant in WorkerId order; non-host crosses
    /// with host only. The CTRL std-stream type is the only
    /// per-backend variation ([`EventTransport::CTRL_STD_STREAM_TY`]).
    fn emit_barrier_shims(
        &self,
        out: &mut String,
        worker: WorkerId,
        is_host: bool,
    ) -> Result<(), EmitError> {
        let tags = self.barriers_used_by(worker);
        if tags.is_empty() {
            return Ok(());
        }
        let ctrl_ty = T::CTRL_STD_STREAM_TY;
        // Inline the shim type ONCE (zero-cost; per-barrier instances
        // are tiny structs that borrow the CTRL streams).
        writeln!(
            out,
            "    // Barrier shim — per-barrier .wait() invokes wire::barrier_cross"
        )
        .ok();
        writeln!(
            out,
            "    // on the CTRL stream(s) for this barrier's participants."
        )
        .ok();
        // The shim type is the same shape across all barriers within
        // one worker: a struct holding Rc<RefCell<CTRL_STD_STREAM_TY>>
        // clones for every CTRL peer this barrier crosses. We DECLARE
        // ONE shim type per barrier so the field set matches that
        // barrier's peer set exactly (different barriers can name
        // different participant subsets — see TASK-0172 + the sync-TCP
        // backends' partial/non-uniform barrier proof).
        for tag in &tags {
            let bid = tag.0;
            let parts = self
                .barrier_participants
                .get(tag)
                .expect("tag in tags came from barriers_used_by");
            if is_host {
                let mut peers: Vec<WorkerId> = parts
                    .iter()
                    .copied()
                    .filter(|w| *w != self.host_worker)
                    .collect();
                peers.sort_unstable();
                writeln!(out, "    struct Bar{bid} {{").ok();
                for p in &peers {
                    let pn = self.worker_name(*p);
                    writeln!(out, "        ctrl_{pn}: Rc<RefCell<{ctrl_ty}>>,").ok();
                }
                writeln!(out, "    }}").ok();
                writeln!(out, "    impl Bar{bid} {{").ok();
                writeln!(out, "        fn wait(&self) {{").ok();
                for p in &peers {
                    let pn = self.worker_name(*p);
                    writeln!(
                        out,
                        "            wire::barrier_cross(&mut *self.ctrl_{pn}.borrow_mut(), {bid});"
                    )
                    .ok();
                }
                writeln!(out, "        }}").ok();
                writeln!(out, "    }}").ok();
                writeln!(out, "    let bar_{bid} = Bar{bid} {{").ok();
                for p in &peers {
                    let pn = self.worker_name(*p);
                    writeln!(out, "        ctrl_{pn}: Rc::clone(&ctrl_{pn}),").ok();
                }
                writeln!(out, "    }};").ok();
            } else {
                writeln!(out, "    struct Bar{bid} {{").ok();
                writeln!(out, "        ctrl_host: Rc<RefCell<{ctrl_ty}>>,").ok();
                writeln!(out, "    }}").ok();
                writeln!(out, "    impl Bar{bid} {{").ok();
                writeln!(out, "        fn wait(&self) {{").ok();
                writeln!(
                    out,
                    "            wire::barrier_cross(&mut *self.ctrl_host.borrow_mut(), {bid});"
                )
                .ok();
                writeln!(out, "        }}").ok();
                writeln!(out, "    }}").ok();
                writeln!(
                    out,
                    "    let bar_{bid} = Bar{bid} {{ ctrl_host: Rc::clone(&ctrl_host) }};"
                )
                .ok();
            }
        }
        Ok(())
    }

    /// Multi-process `run.sh`: delegate to the shared
    /// [`crate::project_skeleton::multi_binary::render_run_sh_multi`]
    /// (lifted in TASK-0257 cycle 112), supplying the host-first worker
    /// ordering + the per-backend SO_BUF commentary
    /// ([`EventTransport::SO_BUF_COMMENT`]), then run the per-backend
    /// post-processing ([`EventTransport::render_run_sh_post`]) — TCP is
    /// identity; UDS swaps the rendezvous dir for a `/tmp`-rooted path
    /// to fit the UDS `sun_path` 104-byte cap.
    pub fn render_run_sh(&self) -> Result<String, EmitError> {
        let bufsz = self.max_payload_bytes()?.max(65536);
        let host_name = self.worker_name(self.host_worker);
        let non_host_names: Vec<String> = self
            .non_host_workers()
            .iter()
            .map(|w| self.worker_name(*w))
            .collect();
        let shared = render_run_sh_multi(&host_name, &non_host_names, bufsz, T::SO_BUF_COMMENT);
        T::render_run_sh_post(shared)
    }
}
