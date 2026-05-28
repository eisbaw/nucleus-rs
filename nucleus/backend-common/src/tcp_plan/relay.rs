//! Host-relay codegen (TASK-0327 cycle 148) + the `run.sh` / pre-init /
//! `max_payload_bytes` glue for the shared sync-TCP substrate.
//!
//! - `Plan::data_conn_var` — DATA-channel variable picker, lifts to
//!   the cycle-148 host-mediated non-host-peer routing.
//! - `Plan::relay_schedule` / `Plan::render_relay_phase` — host's
//!   synchronous w2w relay block, in deterministic (`BTreeMap`
//!   WorkerId, then event-list) order. The relay hop's two wire calls
//!   route through [`WirePrimitives`]; the banner comment block is
//!   supplied wholesale per backend via [`WirePrimitives::relay_banner`].
//! - `Plan::collect_pre_init` — pre-init data set per worker.
//! - `Plan::render_run_sh` / `Plan::max_payload_bytes` — `run.sh`
//!   emission delegating to the shared
//!   [`crate::project_skeleton::multi_binary::render_run_sh_multi`]
//!   plus the per-backend SO_BUF sizing
//!   ([`WirePrimitives::so_buf_comment`]).
//!
//! Lifted from the two backends' verbatim-duplicate `plan/relay.rs`
//! (TASK-0044.02.03).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::multi_worker_walker::collect_pre_init_sets;
use crate::project_skeleton::multi_binary;
use crate::tcp_plan::encode::scalar_width;
use crate::tcp_plan::walkers::{collect_w2w_pushes, RelayHop};
use crate::tcp_plan::{Plan, WirePrimitives};
use crate::EmitError;
use nucleus_compiler::event::{DataId, WorkerId};

impl<W: WirePrimitives> Plan<'_, W> {
    /// The DATA-channel variable for a Push/Wait whose peer is `peer`.
    /// On the star topology a non-host worker only ever owns a single
    /// TCP connection — to host. TASK-0327 (cycle 148) lifts the prior
    /// fail-loud rejection of non-host peers: when a non-host worker's
    /// Push/Wait names another non-host worker, it writes/reads on its
    /// existing `data_host` connection and HOST runs a SYNCHRONOUS
    /// RELAY PHASE (see [`Plan::relay_schedule`] +
    /// [`Plan::render_relay_phase`]) that drains the matching (seq, dst)
    /// entry from `data_<src>` and forwards it verbatim to
    /// `data_<dst>`. Host stays the sole party that owns the (data,
    /// ctrl)-pair-per-(host, worker) topology; no worker-to-worker
    /// socket is added.
    pub fn data_conn_var(
        &self,
        _worker: WorkerId,
        is_host: bool,
        peer: WorkerId,
    ) -> Result<String, EmitError> {
        if is_host {
            if peer == self.host_worker {
                return Err(EmitError::ContractGap(format!(
                    "host Push/Wait names itself ({peer:?}) as the peer — \
                     malformed projection"
                )));
            }
            Ok(format!("data_{}", self.worker_name(peer)))
        } else {
            // TASK-0327 (cycle 148): non-host peer is routed via
            // host-relay — the worker uses its existing `data_host`
            // connection for both directions; HOST relays bytes
            // through to/from the actual peer.
            Ok("data_host".to_string())
        }
    }

    /// TASK-0327 (cycle 148): per non-host src worker, the ordered
    /// list of (seq, dst, data) for every w2w Push event in src's
    /// event list (src != host && dst != host). Event-list order
    /// equals TCP wire order on src's `data_host` stream — host's
    /// relay reads in this order.
    ///
    /// Empty for any src with no w2w pushes. Empty overall if the
    /// schedule has no w2w transfers (the common host↔worker-only
    /// case), and then `render_relay_phase` is a no-op.
    pub fn relay_schedule(&self) -> Result<BTreeMap<WorkerId, Vec<RelayHop>>, EmitError> {
        let mut out: BTreeMap<WorkerId, Vec<RelayHop>> = BTreeMap::new();
        for (src, events) in self.per_worker.iter() {
            if *src == self.host_worker {
                continue;
            }
            let mut hops: Vec<RelayHop> = Vec::new();
            collect_w2w_pushes::<W>(events, self.host_worker, &mut hops)?;
            if !hops.is_empty() {
                out.insert(*src, hops);
            }
        }
        Ok(out)
    }

    /// TASK-0327 (cycle 148): emit host's synchronous relay phase as
    /// a String — for each src in BTreeMap (sorted WorkerId) order,
    /// for each hop in src's event-list order, read `expect_seq` from
    /// `data_<src>` and forward to `data_<dst>`. The seq cross-check
    /// (`read_msg_expect`) preserves the wire-protocol-v0 fail-loud
    /// contract: a mismatch means the deterministic event order
    /// diverged across the three endpoints (src worker, host relay,
    /// dst worker) — a codegen regression, never silently tolerated.
    /// The two wire calls route through `W` (blocking vs `_poll`); the
    /// banner comment block is supplied per backend via
    /// `W::relay_banner`.
    ///
    /// Returns `EmitError::ContractGap` if any hop's `DataId` lacks
    /// a name in `NameTables` (a contract violation the existing
    /// Push/Wait emit also fails-loud on — cycle-148 architect P2.2
    /// fold-back replaced an earlier silent comment-fallback).
    pub fn render_relay_phase(&self, indent: usize) -> Result<String, EmitError> {
        let pad = "    ".repeat(indent);
        let schedule = self.relay_schedule()?;
        if schedule.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        writeln!(out, "{}", W::relay_banner(&pad)).ok();
        for (src, hops) in &schedule {
            let src_name = self.worker_name(*src);
            for hop in hops {
                let dst_name = self.worker_name(hop.dst);
                let data_name = self.data_name(hop.data)?;
                let read_call =
                    W::read_msg_expect_call(&format!("data_{src_name}"), hop.seq.0);
                let write_call =
                    W::write_msg_call(&format!("data_{dst_name}"), hop.seq.0, "__relay_payload");
                writeln!(
                    out,
                    "{pad}{{ \
                     let __relay_payload = {read_call}; \
                     {write_call}; \
                     }} // relay `{data_name}` from {src_name} to {dst_name}",
                )
                .ok();
            }
        }
        Ok(out)
    }

    /// Pre-init set for a worker: cross-worker inputs it Waits on +
    /// data it writes via an indexed Fire output and never
    /// whole-array. Sorted by name. SAME definition as
    /// pthreads-sync's multi-worker `collect_pre_init`.
    pub fn collect_pre_init(&self, worker: WorkerId) -> Result<Vec<(String, DataId)>, EmitError> {
        let evs = &self.per_worker[&worker];
        let mut waited: BTreeSet<DataId> = BTreeSet::new();
        let mut whole: BTreeSet<DataId> = BTreeSet::new();
        let mut indexed: BTreeSet<DataId> = BTreeSet::new();
        collect_pre_init_sets(evs, &mut waited, &mut whole, &mut indexed);

        let mut ids: BTreeSet<DataId> = BTreeSet::new();
        for d in &waited {
            ids.insert(*d);
        }
        for d in &indexed {
            if !whole.contains(d) {
                ids.insert(*d);
            }
        }
        let mut out: Vec<(String, DataId)> = Vec::new();
        for d in &ids {
            out.push((self.data_name(*d)?, *d));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// run.sh (TASK-0038): delegate to the shared
    /// [`multi_binary::render_run_sh_multi`] (lifted in TASK-0257
    /// cycle 112), supplying the per-backend SO_BUF commentary
    /// ([`WirePrimitives::so_buf_comment`]) + the host-first worker
    /// ordering.
    ///
    /// Socket buffer sizing rationale: the sync-TCP backends are sync
    /// (`buffer=1`) so the requirement is one message; size SO_*BUF
    /// from the largest cross-worker payload (sum of element bytes)
    /// with a 64 KiB floor. v2 uses the single highest requirement
    /// (AC#7 limitation: no per-channel granularity if an OS-level cap
    /// binds).
    pub fn render_run_sh(&self) -> Result<String, EmitError> {
        let bufsz = self.max_payload_bytes()?.max(65536);
        let host_name = self.worker_name(self.host_worker);
        let non_host_names: Vec<String> = self
            .non_host_workers()
            .iter()
            .map(|w| self.worker_name(*w))
            .collect();
        Ok(multi_binary::render_run_sh_multi(
            &host_name,
            &non_host_names,
            bufsz,
            W::so_buf_comment(),
        ))
    }

    /// Largest single cross-worker payload in bytes (sum of element
    /// byte widths). Drives SO_*BUF sizing in run.sh. Sized from the
    /// sidecar `ResolvedType` — no AlgoIR.
    pub fn max_payload_bytes(&self) -> Result<usize, EmitError> {
        let mut max = 0usize;
        for d in self.xfer_ids.keys() {
            let name = self.data_name(*d)?;
            let ty = self.sidecar.data_type(*d).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "cross-worker data `{name}` ({d:?}) has no ResolvedType"
                ))
            })?;
            let elems: usize = if ty.is_scalar() {
                1
            } else {
                ty.dims.iter().copied().product()
            };
            let w = scalar_width(&ty.scalar);
            max = max.max(elems * w);
        }
        Ok(max)
    }
}
