//! Host-relay codegen (mp-tcp-poll) + run.sh / pre-init /
//! max_payload_bytes glue. Sibling of
//! `nucleus/backends/mp-tcp-bufsync/src/plan/relay.rs`. The relay
//! emit swaps `wire::read_msg_expect` for `wire::read_msg_expect_poll`
//! and `wire::write_msg` for `wire::write_msg_poll`; everything else
//! is structurally identical.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use backend_common::multi_worker_walker::collect_pre_init_sets;
use backend_common::project_skeleton::multi_binary;
use nucleus_compiler::event::{DataId, WorkerId};

use crate::encode::scalar_width;
use crate::walkers::{collect_w2w_pushes, RelayHop};
use crate::EmitError;
use crate::SO_BUF_COMMENT_POLL;

use super::Plan;

impl Plan<'_> {
    /// The DATA-channel variable for a Push/Wait whose peer is `peer`.
    /// Same star topology as mp-tcp-bufsync (TASK-0327 cycle 148): a
    /// non-host worker only owns one TCP connection (to host); a w2w
    /// Push/Wait writes/reads on its existing `data_host` connection
    /// and HOST runs a synchronous relay phase.
    pub(crate) fn data_conn_var(
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
            Ok("data_host".to_string())
        }
    }

    /// Per non-host src worker, the ordered list of (seq, dst, data)
    /// for every w2w Push event in src's event list. Event-list order
    /// equals TCP wire order on src's `data_host` stream — host's
    /// relay reads in this order.
    pub(crate) fn relay_schedule(
        &self,
    ) -> Result<BTreeMap<WorkerId, Vec<RelayHop>>, EmitError> {
        let mut out: BTreeMap<WorkerId, Vec<RelayHop>> = BTreeMap::new();
        for (src, events) in self.per_worker.iter() {
            if *src == self.host_worker {
                continue;
            }
            let mut hops: Vec<RelayHop> = Vec::new();
            collect_w2w_pushes(events, self.host_worker, &mut hops)?;
            if !hops.is_empty() {
                out.insert(*src, hops);
            }
        }
        Ok(out)
    }

    /// Emit host's synchronous relay phase. POLL variant uses
    /// `wire::read_msg_expect_poll` + `wire::write_msg_poll` so the
    /// nonblocking-socket contract holds across the relay hop too.
    pub(crate) fn render_relay_phase(&self, indent: usize) -> Result<String, EmitError> {
        let pad = "    ".repeat(indent);
        let schedule = self.relay_schedule()?;
        if schedule.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        writeln!(
            out,
            "{pad}// mp-tcp-poll host-relay phase (sibling of bufsync TASK-0327): forward\n\
             {pad}// worker-to-worker Push/Wait pairs through host's existing star topology.\n\
             {pad}// SYNCHRONOUS with poll-variant wire helpers (nonblocking socket: poll-read +\n\
             {pad}// poll-write); one (seq, dst) hop at a time, srcs iterated in sorted-WorkerId\n\
             {pad}// order. seq cross-check via wire::read_msg_expect_poll preserves the\n\
             {pad}// fail-loud contract on Push/Wait pairing divergence."
        )
        .ok();
        for (src, hops) in &schedule {
            let src_name = self.worker_name(*src);
            for hop in hops {
                let dst_name = self.worker_name(hop.dst);
                let data_name = self.data_name(hop.data)?;
                writeln!(
                    out,
                    "{pad}{{ \
                     let __relay_payload = wire::read_msg_expect_poll(&mut data_{src_name}, {}); \
                     wire::write_msg_poll(&mut data_{dst_name}, {}, &__relay_payload); \
                     }} // relay `{data_name}` from {src_name} to {dst_name}",
                    hop.seq.0, hop.seq.0
                )
                .ok();
            }
        }
        Ok(out)
    }

    /// Pre-init set for a worker (same definition as mp-tcp-bufsync).
    pub(crate) fn collect_pre_init(
        &self,
        worker: WorkerId,
    ) -> Result<Vec<(String, DataId)>, EmitError> {
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

    /// run.sh: delegate to the shared
    /// [`multi_binary::render_run_sh_multi`], supplying the per-backend
    /// SO_BUF commentary + host-first worker ordering.
    ///
    /// Socket buffer sizing: mp-tcp-poll has the SAME capability
    /// surface as mp-tcp-bufsync (`buffer=1`, single message in
    /// flight). The largest cross-worker payload sets the SO_BUF
    /// floor (64 KiB minimum).
    pub(crate) fn render_run_sh(&self) -> Result<String, EmitError> {
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
            SO_BUF_COMMENT_POLL,
        ))
    }

    /// Largest single cross-worker payload in bytes. Drives SO_*BUF
    /// sizing in run.sh.
    pub(crate) fn max_payload_bytes(&self) -> Result<usize, EmitError> {
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
