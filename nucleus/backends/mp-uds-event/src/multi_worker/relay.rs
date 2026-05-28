//! TASK-0327 (cycle 149) host-relay codegen for worker-to-worker
//! Push/Wait pairs. Host runs a synchronous relay phase that drains
//! `inbound[seq]` from src and re-pushes to
//! `outbound[(seq, dst_peer_idx_at_host)]` toward dst.
//!
//! The schedule of hops is computed by [`Plan::relay_schedule`]; the
//! emitted code is produced by [`Plan::render_relay_phase`]. Both
//! methods live on [`super::Plan`].

use std::collections::BTreeMap;
use std::fmt::Write as _;

use nucleus_compiler::event::WorkerId;

use super::walkers::{collect_w2w_pushes, RelayHop};
use super::Plan;
use crate::EmitError;

impl Plan<'_> {
    /// TASK-0327 (cycle 149): per non-host src worker, the ordered
    /// list of (seq, dst, data, cap) for every w2w Push event in src's
    /// event list (src != host && dst != host). Event-list order
    /// equals the order in which the host's relay should drain
    /// `inbound[seq]` for that src — though since `wait(seq)` is
    /// per-seq-demuxed, ordering across hops only affects latency,
    /// not correctness.
    ///
    /// Empty for any src with no w2w pushes. Empty overall if the
    /// schedule has no w2w transfers (the common host↔worker-only
    /// case), and then `render_relay_phase` is a no-op.
    pub(super) fn relay_schedule(&self) -> Result<BTreeMap<WorkerId, Vec<RelayHop>>, EmitError> {
        let mut out: BTreeMap<WorkerId, Vec<RelayHop>> = BTreeMap::new();
        for (src, events) in self.per_worker.iter() {
            if *src == self.host_worker {
                continue;
            }
            let mut hops: Vec<RelayHop> = Vec::new();
            collect_w2w_pushes(events, self.host_worker, &self.chan_caps, &mut hops)?;
            if !hops.is_empty() {
                out.insert(*src, hops);
            }
        }
        Ok(out)
    }

    /// TASK-0327 (cycle 149): emit host's synchronous relay phase as a
    /// String — for each src in BTreeMap (sorted WorkerId) order, for
    /// each hop in src's event-list order, call
    /// `reactor.borrow_mut().relay_one(seq, dst_peer_idx_at_host, cap)`.
    /// `relay_one` (defined in `runtime.rs`) does `wait(seq)` then
    /// `push(seq, dst_peer, payload, cap)` — bytes-verbatim forwarding,
    /// no re-encode. The whole batch runs inside a single
    /// `reactor.borrow_mut()` scope so no other reactor borrow can
    /// interleave (single-threaded RefCell on host).
    ///
    /// Returns `EmitError::ContractGap` if any hop's `DataId` lacks a
    /// name in `NameTables` — same fail-loud contract as the Push/Wait
    /// emit path. Cycle-148 architect P2.2 lesson applies: bubble
    /// data_name errors rather than silently inlining a `{DataId:?}`
    /// fallback in the comment.
    pub(super) fn render_relay_phase(&self, indent: usize) -> Result<String, EmitError> {
        let pad = "    ".repeat(indent);
        let schedule = self.relay_schedule()?;
        if schedule.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        writeln!(
            out,
            "{pad}// TASK-0327 host-relay phase: forward worker-to-worker Push/Wait\n\
             {pad}// pairs through host's existing per-(host,worker) star-topology\n\
             {pad}// reactor. SYNCHRONOUS: read inbound[seq] (from data_<src>),\n\
             {pad}// then re-push to outbound[(seq, dst_peer)] (toward data_<dst>),\n\
             {pad}// one (seq, dst) hop at a time, srcs iterated in sorted-WorkerId order."
        )
        .ok();
        writeln!(out, "{pad}{{").ok();
        writeln!(out, "{pad}    let mut __relay = reactor.borrow_mut();").ok();
        for (src, hops) in &schedule {
            let src_name = self.worker_name(*src);
            for hop in hops {
                let dst_name = self.worker_name(hop.dst);
                let data_name = self.data_name(hop.data)?;
                let dst_peer = self
                    .peer_index_for(self.host_worker, hop.dst)
                    .ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "mp-uds-event relay: host has no peer index for dst {:?} \
                         on hop seq={:?} data={:?}",
                            hop.dst, hop.seq, hop.data
                        ))
                    })?;
                writeln!(
                    out,
                    "{pad}    __relay.relay_one({seq}u64, {dst_peer}usize, {cap}usize); \
                     // relay `{data_name}` from {src_name} to {dst_name}",
                    seq = hop.seq.0,
                    dst_peer = dst_peer,
                    cap = hop.cap,
                )
                .ok();
            }
        }
        writeln!(out, "{pad}}}").ok();
        Ok(out)
    }
}
