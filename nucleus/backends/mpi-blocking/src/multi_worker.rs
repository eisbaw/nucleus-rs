//! Multi-worker SPMD codegen for the `mpi-blocking` backend
//! (TASK-0045.01). Lowers the per-worker [`Event`] lists of a ≥2-worker
//! schedule into ONE rank-dispatched Rust binary (`mpiexec -n N`).
//!
//! # Why this is NOT the pthreads-sync `Plan`
//!
//! `pthreads-sync::multi_worker` emits ONE process that `thread::spawn`s
//! one OS thread per non-host worker and exchanges data through
//! shared-memory `Slot<T>` rendezvous objects (a `Mutex<Option<T>>` +
//! `Condvar`). MPI is SPMD: there is ONE binary, every rank runs `main`,
//! and behaviour branches on `world.rank()` (PRD §7.2). Workers map to
//! ranks; there is no shared memory — cross-worker data crosses the
//! network via blocking `MPI_Send`/`MPI_Recv`.
//!
//! # What IS reused (no arithmetic drift)
//!
//! The compute arithmetic (Fire/Loop/partition/reuse/check-frame walk)
//! is the SHARED [`backend_common::multi_worker_walker::render_worker_events`]
//! — the same walker pthreads-sync / pthreads-async / mp-tcp-event use.
//! This backend emits a per-rank PRELUDE of MPI rendezvous + barrier
//! wrapper types whose `.push()` / `.wait()` methods the walker targets
//! (`rendezvous_prefix = "mpi"`), so the Fire/Loop codegen cannot drift
//! from the in-process backends. The only MPI-specific surface is the
//! rendezvous prelude + the `match world.rank()` dispatch.
//!
//! # Lowering map
//!
//! - `Push{data, dst, seq}` -> `mpi_<rid>.push(<data>.clone())` where
//!   `mpi_<rid>` is a [`VecChan`]/[`ScalarChan`] bound to peer `rank(dst)`
//!   and **MPI tag = `<rid>`**. The per-rid tag is LOAD-BEARING: without
//!   it, two messages with the same `(src, dst)` would match in MPI
//!   send-order, so a consumer that posts its receives in a different
//!   order than the producer sent would silently swap payloads — the
//!   exact value-bug a single `mpiexec -n 1` run hides and an all-ranks-
//!   live `-n N` run exposes (TASK-0045.01 AC#4; memory `16-jacobi`:
//!   deadlock-free != value-correct).
//! - `Wait{data, src, seq}` -> `let <data> = mpi_<rid>.wait();` (whole-
//!   array) or the gather slice-paste the shared walker emits, with the
//!   same tag = `<rid>` so it matches the producer's `Push`.
//! - `Sync{tag}` -> `bar_<tag>.wait()` == a whole-world `MPI_Barrier`.
//!
//! # Scope limits (rejected loud, NOT silently mis-emitted)
//!
//! - **Non-whole-world barriers.** A barrier whose participants are a
//!   strict subset of the used workers needs `Comm_split` (a collective
//!   the M7 foundation did not prove). Rejected with a typed
//!   [`EmitError::UnsupportedFeature`] forward-linked to TASK-0045.02.
//! - **Multi-worker check-loop frames.** `check loop` under MPI has
//!   per-rank-vs-aggregate reporter semantics (no shared memory across
//!   processes) that need design; rejected loud, forward-linked to
//!   TASK-0045.03. No shipped multi-worker schedule carries a check
//!   frame, so this never fires on the example set.
//! - **Standard-mode `MPI_Send`** may block for messages above the eager
//!   limit (see the crate-root limitation note). Documented, not a
//!   codegen reject.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use nucleus_compiler::event::{DataId, Event, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use crate::NameTables;
use backend_common::elect_host_from_worker_names;
use backend_common::multi_worker_walker::{self as walker, RendezvousId, WalkerCtx};
use backend_common::render::{rust_scalar_type, EmitError};

/// Stable id of one MPI rendezvous channel (one per cross-worker
/// Push/Wait pair). Shared alias from the walker; doubles as the MPI
/// message TAG (see module docstring on why per-rid tags are load-
/// bearing for value-correctness).
///
/// Tag-space note: this `usize` rid is emitted as a bare `Tag`
/// (`c_int`) literal. MPI guarantees `MPI_TAG_UB >= 32767`, so a
/// schedule with more than ~32k distinct cross-worker transfers could
/// emit a tag outside the portable range. No shipped schedule emits
/// more than a handful of rids; if that ever changes the fix is a
/// modular tag scheme keyed on (peer, generation) rather than a global
/// rid counter (review-gate P3, cycle M7-multiworker).
type ChanId = RendezvousId;
/// Stable id of one barrier — the contract-carried [`SyncTag`]
/// (TASK-0172). Same value for every participant of the barrier.
type BarrierId = SyncTag;

/// Render the full `main.rs` of the multi-worker SPMD MPI program.
pub(crate) fn render_main_rs_multi(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    Plan::build(per_worker, names, sidecar)?.emit()
}

struct Plan<'a> {
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    names: &'a NameTables,
    sidecar: &'a NameSidecar,
    /// Workers with a non-empty EventList, in WorkerId order.
    used_workers: Vec<WorkerId>,
    /// MPI rank assigned to each used worker. The elected host gets rank
    /// 0 (mirrors the single-worker arm's `rank == 0` guard and the
    /// `elect_host_from_worker_names` rule — memory
    /// `feedback-driver-must-mirror-backend-election-exactly`); the
    /// remaining used workers take ranks 1..N in WorkerId order.
    rank_of: BTreeMap<WorkerId, i32>,
    /// Cross-worker Push/Wait pairs, `(DataId, SeqTag)` -> ChanId. Same
    /// construction as pthreads-sync's `slot_ids` (so the rid numbering
    /// is identical), but each rid here is ALSO the MPI message tag.
    chan_ids: BTreeMap<(DataId, SeqTag), ChanId>,
    /// Per-pair tile (the gather slice-paste axis), threaded into the
    /// shared walker's `Wait` codegen verbatim.
    pair_tiles: BTreeMap<(DataId, SeqTag), nucleus_compiler::event::IterTile>,
    /// Per-(worker, data, seq) overlapping-write accumulator
    /// classification (TASK-0343), threaded into the walker verbatim.
    accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)>,
    /// `SyncTag` -> participants. Keyed by the contract barrier identity.
    barrier_participants: BTreeMap<BarrierId, BTreeSet<WorkerId>>,
}

impl<'a> Plan<'a> {
    fn build(
        per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
        names: &'a NameTables,
        sidecar: &'a NameSidecar,
    ) -> Result<Self, EmitError> {
        let used_workers: Vec<WorkerId> = per_worker
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(w, _)| *w)
            .collect();

        // Host election: the SHARED helper — identical rule to every
        // other multi-worker backend, so the rank-0 = host mapping cannot
        // skew from the backend election (memory
        // `feedback-driver-must-mirror-backend-election-exactly`).
        let host_worker =
            elect_host_from_worker_names(&names.worker, &used_workers).ok_or_else(|| {
                EmitError::ContractGap(
                    "multi-worker mpi-blocking emit requires at least one used worker".to_string(),
                )
            })?;

        // Rank assignment: host -> 0, remaining used workers -> 1..N in
        // WorkerId order (deterministic).
        let mut rank_of: BTreeMap<WorkerId, i32> = BTreeMap::new();
        rank_of.insert(host_worker, 0);
        let mut next: i32 = 1;
        for w in &used_workers {
            if *w == host_worker {
                continue;
            }
            rank_of.insert(*w, next);
            next += 1;
        }

        // Reject multi-worker check-loop frames loud (no shipped schedule
        // carries one; the per-rank-vs-aggregate reporter semantics need
        // design — TASK-0045.03). See module docstring.
        for w in &used_workers {
            if has_check_frame(&per_worker[w]) {
                return Err(EmitError::UnsupportedFeature(
                    "mpi-blocking: a `check loop` frame on a multi-worker schedule is not yet \
                     supported — under MPI the latency tally has per-rank-vs-aggregate semantics \
                     (no shared memory across processes; an aggregate would need MPI_Reduce). \
                     Forward-linked to TASK-0045.03. This is a loud reject, not a silent mis-emit."
                        .to_string(),
                ));
            }
        }

        // Channel ids: identical construction to pthreads-sync (one per
        // `(DataId, SeqTag)` Push/Wait pair, ascending) so the rid
        // numbering matches; here each rid is also the MPI tag.
        let pair_tiles: BTreeMap<(DataId, SeqTag), nucleus_compiler::event::IterTile> =
            walker::collect_pair_tiles(per_worker.values());
        let chan_ids: BTreeMap<(DataId, SeqTag), ChanId> = pair_tiles
            .keys()
            .enumerate()
            .map(|(i, k)| (*k, i))
            .collect();

        // Barrier participants, keyed by the contract-carried SyncTag.
        let mut barrier_participants: BTreeMap<BarrierId, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            walker::collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
                barrier_participants
                    .entry(tag)
                    .or_insert_with(|| parts.clone());
            });
        }

        // Reject non-whole-world barriers loud (Comm_split is unproven —
        // TASK-0045.02). A whole-world barrier requires EVERY used worker
        // to participate; `mpiexec -n N` launches exactly the used-worker
        // count, so a subset participant set would make the participating
        // ranks' `world.barrier()` block on a rank that never calls it.
        //
        // This guard checks the participant SET, not the per-rank barrier
        // CALL COUNT. Equal counts are guaranteed UPSTREAM: `inject_syncs`
        // refuses to emit a barrier inside a `partition=workers` scope (the
        // one place per-rank loop trip-counts could diverge), so every
        // whole-world barrier sits at a uniform-iteration point and all
        // ranks reach it the same number of times. If that upstream
        // invariant ever regressed, `check-mpi`'s `timeout` wrapper would
        // catch the resulting deadlock loud (review-gate P3.1).
        let all_used: BTreeSet<WorkerId> = used_workers.iter().copied().collect();
        for (tag, parts) in &barrier_participants {
            if *parts != all_used {
                let names_in: Vec<&str> = parts
                    .iter()
                    .map(|w| names.worker.get(w).map(String::as_str).unwrap_or("?"))
                    .collect();
                return Err(EmitError::UnsupportedFeature(format!(
                    "mpi-blocking: barrier (SyncTag {}) has a non-whole-world participant set \
                     {{{}}} (a strict subset of the {} used workers). A whole-world MPI_Barrier \
                     would deadlock; the host-excluding / non-uniform case needs Comm_split + a \
                     sub-communicator barrier, a collective the M7 foundation did not prove. \
                     Forward-linked to TASK-0045.02. This is a loud reject, not a silent mis-emit.",
                    tag.0,
                    names_in.join(","),
                    all_used.len(),
                )));
            }
        }

        // Per-worker overlapping-write accumulator classification.
        let mut accumulate_waits: BTreeSet<(WorkerId, DataId, SeqTag)> = BTreeSet::new();
        for w in &used_workers {
            for (d, s) in walker::collect_accumulate_waits(&per_worker[w], sidecar, &pair_tiles) {
                accumulate_waits.insert((*w, d, s));
            }
        }

        Ok(Plan {
            per_worker,
            names,
            sidecar,
            used_workers,
            rank_of,
            chan_ids,
            pair_tiles,
            accumulate_waits,
            barrier_participants,
        })
    }

    fn emit(&self) -> Result<String, EmitError> {
        let mut out = String::new();
        let n = self.used_workers.len();

        // ---- Header + kernels mod. ----
        writeln!(
            out,
            "//! Generated by the nucleus pre-compiler (mpi-blocking backend, SPMD multi-worker)."
        )
        .ok();
        writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
        writeln!(out, "//!").ok();
        writeln!(
            out,
            "//! SPMD: one binary, every MPI rank runs `main`; behaviour branches on"
        )
        .ok();
        writeln!(
            out,
            "//! `world.rank()`. Push/Wait => blocking MPI Send/Recv (tag = rendezvous id);"
        )
        .ok();
        writeln!(out, "//! Sync => whole-world MPI_Barrier. Launch: `mpiexec -n {n}`.").ok();
        writeln!(out).ok();
        writeln!(out, "// The user's kernel bodies live in kernels.rs.").ok();
        writeln!(out, "mod kernels;").ok();
        writeln!(out).ok();

        // ---- MPI rendezvous + barrier prelude. ----
        self.emit_prelude(&mut out);

        // ---- fn main: MPI_Init, rank guard, dispatch. ----
        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();
        writeln!(
            out,
            "    // MPI_Init. The `Universe` guard runs MPI_Finalize on drop on EVERY rank."
        )
        .ok();
        writeln!(out, "    let universe = mpi::initialize().expect(\"MPI_Init failed\");").ok();
        writeln!(out, "    let world = universe.world();").ok();
        writeln!(out, "    let size = world.size();").ok();
        writeln!(out, "    // This schedule uses exactly {n} workers => exactly {n} ranks.").ok();
        writeln!(
            out,
            "    // `size` is identical on every rank, so all ranks take this branch together"
        )
        .ok();
        writeln!(
            out,
            "    // (no rank returns alone before a collective => no deadlock)."
        )
        .ok();
        writeln!(out, "    if size != {n} {{").ok();
        writeln!(out, "        if world.rank() == 0 {{").ok();
        writeln!(
            out,
            "            eprintln!(\"mpi-blocking: this schedule needs exactly {n} ranks (got {{size}}); launch `mpiexec -n {n}`\");"
        )
        .ok();
        writeln!(out, "        }}").ok();
        writeln!(out, "        return;").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "    match world.rank() {{").ok();

        for w in &self.used_workers {
            let rank = self.rank_of[w];
            let wname = self.worker_name(*w);
            writeln!(out, "        {rank} => {{ // worker `{wname}`").ok();
            let arm = self.render_worker_arm(*w)?;
            out.push_str(&arm);
            writeln!(out, "        }}").ok();
        }
        // Defensive: `size == n` was checked above, so ranks 0..n are the
        // only ones reachable. The wildcard keeps the match exhaustive.
        writeln!(out, "        _ => {{ unreachable!(\"rank >= worker count despite the size guard above\") }}").ok();
        writeln!(out, "    }}").ok();
        writeln!(out, "}}").ok();
        Ok(out)
    }

    /// Emit the MPI rendezvous + barrier wrapper types. These give the
    /// shared walker the `.push()` / `.wait()` method surface it targets,
    /// so the Fire/Loop arithmetic stays byte-shared with the in-process
    /// backends and only the transport is MPI-specific.
    fn emit_prelude(&self, out: &mut String) {
        let prelude = "\
use mpi::traits::*;
use mpi::topology::Process;
use mpi::{Rank, Tag};
use core::marker::PhantomData;

/// One whole-array (`Vec<T>`) cross-worker rendezvous over MPI. `push`
/// is a blocking standard-mode `MPI_Send` of the slice; `wait` is a
/// blocking `MPI_Recv` (count-probing `receive_vec`). Both pin the MPI
/// TAG to the rendezvous id so concurrent transfers between the same
/// rank pair cannot cross-match (see backend module docstring).
//
// `dead_code`: a given schedule uses VecChan (array transfers) and/or
// ScalarChan (scalar transfers); the unused one is dead in that project.
#[allow(dead_code)]
struct VecChan<'a, T: Equivalence> { proc_: Process<'a>, tag: Tag, _pd: PhantomData<T> }
#[allow(dead_code)]
impl<'a, T: Equivalence> VecChan<'a, T> {
    fn new<C: Communicator>(comm: &'a C, peer: Rank, tag: Tag) -> Self {
        VecChan { proc_: comm.process_at_rank(peer), tag, _pd: PhantomData }
    }
    fn push(&self, v: Vec<T>) { self.proc_.send_with_tag(&v[..], self.tag); }
    fn wait(&self) -> Vec<T> { self.proc_.receive_vec_with_tag::<T>(self.tag).0 }
}

/// Scalar (`T`) cross-worker rendezvous over MPI. Same tag discipline as
/// [`VecChan`]; sends/receives a single element.
#[allow(dead_code)]
struct ScalarChan<'a, T: Equivalence> { proc_: Process<'a>, tag: Tag, _pd: PhantomData<T> }
#[allow(dead_code)]
impl<'a, T: Equivalence> ScalarChan<'a, T> {
    fn new<C: Communicator>(comm: &'a C, peer: Rank, tag: Tag) -> Self {
        ScalarChan { proc_: comm.process_at_rank(peer), tag, _pd: PhantomData }
    }
    fn push(&self, v: T) { self.proc_.send_with_tag(&v, self.tag); }
    fn wait(&self) -> T { self.proc_.receive_with_tag::<T>(self.tag).0 }
}

/// Whole-world barrier wrapper. `wait` is `MPI_Barrier` over COMM_WORLD;
/// every rank participates (the emit rejects non-whole-world barriers).
struct WorldBar<'a, C: Communicator> { comm: &'a C }
impl<'a, C: Communicator> WorldBar<'a, C> {
    fn new(comm: &'a C) -> Self { WorldBar { comm } }
    fn wait(&self) { self.comm.barrier(); }
}
";
        out.push_str(prelude);
        out.push('\n');
    }

    /// Render one rank's match arm: pre-init, channel bindings, barrier
    /// bindings, then the shared event walk (prefix `""` — each arm is
    /// its own scope, no closure-capture clones like the pthreads path).
    fn render_worker_arm(&self, worker: WorkerId) -> Result<String, EmitError> {
        let mut out = String::new();
        let indent = 3; // inside fn main { match { <rank> => { .. } } }
        let pad = "    ".repeat(indent);
        let evs = &self.per_worker[&worker];

        // ---- Pre-init (cross-worker WAITed data + indexed-Fire outputs). ----
        let (pre_init, let_at_wait) = self.collect_pre_init(worker)?;
        for (name, did) in &pre_init {
            let ty = self.sidecar.data_type(*did).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "pre-init data `{name}` ({did:?}) has no ResolvedType in sidecar"
                ))
            })?;
            let rty = rust_type_of(ty);
            let init = render_array_init(ty);
            writeln!(out, "{pad}let mut {name}: {rty} = {init};").ok();
        }

        // ---- Channel bindings (one per rid this worker uses). ----
        // Peer rank comes from the OTHER end of the pair: a Push targets
        // `rank(dst)`, a Wait sources `rank(src)`. Each rid is used by a
        // worker for EITHER push OR wait (single-producer/single-consumer
        // per pair), so the peer is unambiguous.
        let chans = self.chans_used_by(worker)?;
        for c in &chans {
            let ty = self.sidecar.data_type(c.data).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "cross-worker data `{}` ({:?}) has no ResolvedType in sidecar",
                    c.name, c.data
                ))
            })?;
            // All v2 scalar types have `mpi::Equivalence` (rsmpi 0.8
            // covers i8..i64, u8..u64, usize/isize, f32/f64, bool), so
            // the typed-error arm (AC#2) is currently unreachable; the
            // substantive typing decision is scalar-vs-Vec dispatch.
            mpi_equivalence_scalar(ty)?;
            let elem = rust_scalar_type(&ty.scalar);
            let kind = if ty.is_scalar() { "ScalarChan" } else { "VecChan" };
            writeln!(
                out,
                "{pad}let mpi_{rid} = {kind}::<{elem}>::new(&world, {peer}, {rid}); // data `{name}` <-> rank {peer}",
                rid = c.id,
                peer = c.peer_rank,
                name = c.name,
            )
            .ok();
        }

        // ---- Barrier bindings (one per SyncTag this worker uses). ----
        for tag in self.barriers_used_by(worker) {
            writeln!(out, "{pad}let bar_{} = WorldBar::new(&world);", tag.0).ok();
        }

        if !pre_init.is_empty() || !chans.is_empty() || !self.barriers_used_by(worker).is_empty() {
            writeln!(out).ok();
        }

        // ---- Shared event walk (rendezvous_prefix "mpi", prefix ""). ----
        let walker_ctx = WalkerCtx {
            names: self.names,
            sidecar: self.sidecar,
            rendezvous_prefix: "mpi",
            rendezvous_ids: &self.chan_ids,
            pair_tiles: &self.pair_tiles,
            accumulate_waits: &self.accumulate_waits,
            let_at_wait_data: &let_at_wait,
        };
        walker::render_worker_events(&walker_ctx, worker, evs, &mut out, indent, "")?;
        Ok(out)
    }

    /// Channel descriptors a worker touches, in ChanId order.
    fn chans_used_by(&self, worker: WorkerId) -> Result<Vec<ChanDecl>, EmitError> {
        // rid -> (peer rank, direction), collected by scanning Push.dst /
        // Wait.src. The direction enforces the push-XOR-wait-per-rid
        // invariant (see `record_peer`).
        let mut peer_of: BTreeMap<ChanId, (i32, Dir)> = BTreeMap::new();
        self.collect_chan_peers(&self.per_worker[&worker], &mut peer_of)?;
        let mut out: Vec<ChanDecl> = Vec::new();
        for (id, (peer_rank, _dir)) in peer_of {
            // Recover the (data, seq) for this rid to name it + type it.
            let (data, _seq) = self
                .chan_ids
                .iter()
                .find(|(_, v)| **v == id)
                .map(|(k, _)| *k)
                .ok_or_else(|| {
                    EmitError::ContractGap(format!("chan id {id} has no (DataId, SeqTag) key"))
                })?;
            out.push(ChanDecl {
                id,
                peer_rank,
                data,
                name: self.data_name(data)?,
            });
        }
        Ok(out)
    }

    /// Walk a worker's events (recursively into loops) recording, per
    /// rid, the peer rank AND direction of the matching Push/Wait. Fails
    /// loud if the same rid is used for BOTH a push and a wait in one
    /// worker (a single worker as both producer and consumer of one pair
    /// — a projection-layer contract violation) or resolves to two peer
    /// ranks. The direction check makes the "each rid is used for EITHER
    /// push OR wait" invariant (relied on when emitting ONE chan binding
    /// per rid) a fail-loud guard rather than an unchecked comment
    /// (review-gate P3, cycle M7-multiworker).
    fn collect_chan_peers(
        &self,
        events: &[Event],
        out: &mut BTreeMap<ChanId, (i32, Dir)>,
    ) -> Result<(), EmitError> {
        for e in events {
            match e {
                Event::Push { data, dst, seq, .. } => {
                    let id = self.chan_id(*data, *seq)?;
                    let peer = self.rank_for(*dst)?;
                    self.record_peer(out, id, peer, Dir::Push)?;
                }
                Event::Wait { data, src, seq, .. } => {
                    let id = self.chan_id(*data, *seq)?;
                    let peer = self.rank_for(*src)?;
                    self.record_peer(out, id, peer, Dir::Wait)?;
                }
                Event::Loop { body, .. } => self.collect_chan_peers(body, out)?,
                Event::Fire { .. }
                | Event::Sync { .. }
                | Event::Alloc { .. }
                | Event::Free { .. } => {}
            }
        }
        Ok(())
    }

    fn record_peer(
        &self,
        out: &mut BTreeMap<ChanId, (i32, Dir)>,
        id: ChanId,
        peer: i32,
        dir: Dir,
    ) -> Result<(), EmitError> {
        match out.insert(id, (peer, dir)) {
            Some((prev_peer, _)) if prev_peer != peer => Err(EmitError::ContractGap(format!(
                "rendezvous id {id} resolves to two peer ranks ({prev_peer} and {peer}) within \
                 one worker — the single-producer/single-consumer-per-pair invariant is violated \
                 (projection-layer bug)"
            ))),
            Some((_, prev_dir)) if prev_dir != dir => Err(EmitError::ContractGap(format!(
                "rendezvous id {id} is used for BOTH a Push and a Wait within one worker — a \
                 worker cannot be both producer and consumer of the same (data, seq) pair \
                 (projection-layer contract violation; would make the single chan binding's \
                 send/recv direction ambiguous)"
            ))),
            _ => Ok(()),
        }
    }

    fn chan_id(&self, data: DataId, seq: SeqTag) -> Result<ChanId, EmitError> {
        self.chan_ids.get(&(data, seq)).copied().ok_or_else(|| {
            EmitError::ContractGap(format!(
                "Push/Wait of data {data:?} (seq {seq:?}) has no rendezvous id (not collected \
                 as cross-worker)"
            ))
        })
    }

    fn rank_for(&self, w: WorkerId) -> Result<i32, EmitError> {
        self.rank_of.get(&w).copied().ok_or_else(|| {
            EmitError::ContractGap(format!(
                "worker {w:?} (a Push.dst / Wait.src peer) is not a used worker with a rank"
            ))
        })
    }

    /// SyncTags a worker participates in, ascending.
    fn barriers_used_by(&self, w: WorkerId) -> Vec<BarrierId> {
        let mut out: Vec<BarrierId> = self
            .barrier_participants
            .iter()
            .filter(|(_, parts)| parts.contains(&w))
            .map(|(id, _)| *id)
            .collect();
        out.sort_unstable();
        out
    }

    fn worker_name(&self, w: WorkerId) -> String {
        self.names
            .worker
            .get(&w)
            .cloned()
            .unwrap_or_else(|| format!("w{}", w.0))
    }

    fn data_name(&self, d: DataId) -> Result<String, EmitError> {
        self.names.data.get(&d).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {d:?} has no name in NameTables"))
        })
    }

    /// Pre-init set (identical policy to pthreads-sync): cross-worker
    /// inputs the worker Waits on + data it writes via an indexed Fire
    /// output and never whole-array; whole-array-recv-only data is
    /// excluded and declared at its `.wait()` site instead (TASK-0349).
    #[allow(clippy::type_complexity)]
    fn collect_pre_init(
        &self,
        worker: WorkerId,
    ) -> Result<(Vec<(String, DataId)>, BTreeSet<DataId>), EmitError> {
        let evs = &self.per_worker[&worker];
        let mut waited: BTreeSet<DataId> = BTreeSet::new();
        let mut whole: BTreeSet<DataId> = BTreeSet::new();
        let mut indexed: BTreeSet<DataId> = BTreeSet::new();
        walker::collect_pre_init_sets(evs, &mut waited, &mut whole, &mut indexed);

        let accumulate_data: BTreeSet<DataId> = self
            .accumulate_waits
            .iter()
            .filter_map(|(w, d, _)| if *w == worker { Some(*d) } else { None })
            .collect();
        let let_at_wait = walker::collect_let_at_wait_data(
            evs,
            &self.pair_tiles,
            self.sidecar,
            &accumulate_data,
            &indexed,
        );

        let mut ids: BTreeSet<DataId> = BTreeSet::new();
        for d in &waited {
            if let_at_wait.contains(d) {
                continue;
            }
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
        Ok((out, let_at_wait))
    }
}

/// One emitted MPI channel binding.
struct ChanDecl {
    id: ChanId,
    peer_rank: i32,
    data: DataId,
    name: String,
}

/// Direction a worker uses a rendezvous id (used to fail loud if one
/// worker uses the same rid for both — see `record_peer`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    Push,
    Wait,
}

/// True if any event (recursively) carries a `check_frame`.
fn has_check_frame(events: &[Event]) -> bool {
    events.iter().any(|e| match e {
        Event::Loop {
            check_frame, body, ..
        } => check_frame.is_some() || has_check_frame(body),
        _ => false,
    })
}

/// Verify a transferred scalar type maps to an `mpi::Equivalence`
/// payload (AC#2). Every v2 [`ScalarType`] currently does (rsmpi 0.8
/// covers all of them, including `bool`), so this returns `Ok` for the
/// whole closed enum; the typed-error arm exists so a future non-MPI
/// scalar fails LOUD here rather than producing a `trait bound not
/// satisfied` rustc error in the generated crate (panic-not-diagnostic
/// discipline). The substantive AC#2 work is the scalar-vs-Vec dispatch
/// in `render_worker_arm`.
fn mpi_equivalence_scalar(
    ty: &nucleus_compiler::algo::ResolvedType,
) -> Result<(), EmitError> {
    use nucleus_compiler::algo::ScalarType::*;
    match ty.scalar {
        Usize | Isize | U8 | U16 | U32 | U64 | I8 | I16 | I32 | I64 | F32 | F64 | Bool => Ok(()),
    }
}

// Type rendering helpers (sidecar-driven; mirror pthreads-sync exactly).

fn rust_type_of(ty: &nucleus_compiler::algo::ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_type(&ty.scalar).to_string()
    } else {
        format!("Vec<{}>", rust_scalar_type(&ty.scalar))
    }
}

fn render_array_init(ty: &nucleus_compiler::algo::ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_zero(&ty.scalar).to_string()
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        format!("vec![{zero}; {total}]")
    }
}

fn rust_scalar_zero(t: &nucleus_compiler::algo::ScalarType) -> &'static str {
    use nucleus_compiler::algo::ScalarType::*;
    match t {
        F32 | F64 => "0.0",
        Bool => "false",
        _ => "0",
    }
}
