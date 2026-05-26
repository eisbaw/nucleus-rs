//! mp-tcp-bufsync backend. PRD §7.1, TASK-0036/0037/0038.
//!
//! The **second** tier-1 backend, and the keystone that makes the
//! project's core thesis falsifiable: the same `(algorithm,
//! schedule)` must produce a **bit-identical** `output.bin` on two
//! independent backends. pthreads-sync (shared-memory threads) is
//! backend #1; this is `mp-tcp-bufsync` — workers are **OS
//! processes**, transport is `std::net::TcpStream` over loopback,
//! sync = blocking recv, buffered.
//!
//! ## Same contract, AlgoIR-free (TASK-0124 carried)
//!
//! Identical `emit()` signature and consumed contract as
//! pthreads-sync: `(per_worker: &BTreeMap<WorkerId,Vec<Event>>,
//! names: &NameTables, sidecar: &NameSidecar, kernels_rs_path,
//! out_dir) -> Result<EmitResult, EmitError>`. It imports neither
//! `nucleus_compiler::algo` (beyond the inert `IrExpr`/type grammar the
//! EventList carries) nor `nucleus_compiler::link`/`acfg`.
//!
//! ## No renderer drift (TASK-0124 flagged risk, addressed)
//!
//! Expression / index / kernel-call / loop-bound / single-worker
//! rendering is **not re-implemented** here — it calls
//! pthreads-sync's `pub` shared shims (`render_fire_args_pub`,
//! `render_fire_output_assign_pub`, `render_const_expr_pub`,
//! `render_single_worker_main`, `rust_type_of`, …). There is exactly
//! ONE implementation; this crate only adds the multi-PROCESS
//! transport. That is why a single-worker example is byte-identical
//! to pthreads-sync's single process, and why the multi-worker
//! arithmetic cannot silently diverge.
//!
//! ## Topology (deterministic; no sleeps-as-sync — AC#3)
//!
//! Exactly one TCP connection per `(host, worker)` ordered pair.
//! `host` is the server: it binds a listener per non-host worker on
//! `127.0.0.1:0` (kernel-assigned ephemeral port) and ATOMICALLY
//! writes the port to `$NUC_RENDEZVOUS_DIR/<worker>.port` (via
//! tmp-file + POSIX rename). Each non-host worker is the client; it
//! polls-with-retry on that rendezvous file (600 × 10 ms = 6 s,
//! symmetric with the connect-retry bound), reads the port, then
//! `connect`s with a bounded retry loop — a refused connect only
//! means the listener is not up yet (a liveness wait, not a data
//! sync; the eventual outcome is deterministic). The worker that
//! *uses* the port also *allocates* it: there is NO close-then-rebind
//! window where some other process could grab the port (TASK-0176;
//! replaces the prior `__nuc_pick_port` helper that bound+closed
//! before the host re-bound). All Push / Wait / Sync between host
//! and a worker travel over that one stream in the schedule's
//! deterministic event order, so the `SeqTag` on each framed message
//! is a fail-loud cross-check, not routing.
//!
//! Barriers are host-mediated: every barrier in the tier-1 set is
//! `{host, w0}` (2-party); the general N-party barrier is a star
//! through host. Barrier identity is the contract-carried
//! [`nucleus_compiler::event::SyncTag`] on `Event::Sync` (TASK-0172): host
//! and every worker key the wire `barrier_cross` token by that tag,
//! which is the same value for every participant by construction —
//! so partial / non-uniform barriers (participant sets that differ
//! between barriers) lower correctly, not just uniform ones. The old
//! per-worker pre-order-index recovery and its non-uniform
//! [`EmitError::ContractGap`] are removed (TASK-0172). A barrier whose
//! participant set excludes `host` still cannot be lowered on the
//! one-stream-per-pair topology — that is a SEPARATE, genuine
//! transport limitation and stays a typed [`EmitError::ContractGap`]
//! (honest limitation, not a wrong binary; worker-to-worker mesh is
//! TASK-0175). For a uniform-barrier program the tags are `0,1,2,…`
//! in pre-order, so generated code stays byte-identical.
//!
//! ## Inherited caveats (identical to pthreads-sync; fail-loud)
//!
//! - `block_transform` defers absolute-index rebinding to codegen;
//!   the divisible case is handled via the shared single-worker
//!   renderer, non-divisible is TASK-0173. No required mp-tcp cell
//!   exercises a blocked *multi*-worker schedule.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nucleus_compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

// Shared codegen primitives (TASK-0244 cycle 37) — the SINGLE
// implementation of expression / index / call / loop-bound rendering,
// plus the check_frame emit templates. Backend-common parents every
// tier-1 backend. The single-worker delegate
// `render_single_worker_main` still lives in pthreads-sync (it is the
// pthreads-sync-specific straight-line main.rs emitter — pthreads-
// async and mp-tcp-bufsync DELEGATE to it for 0/1-used-worker emit so
// the per-backend single-worker code is byte-identical to pthreads-
// sync's, the cross-backend differential invariant on the
// single-worker path).
use backend_common::check_frame::{
    collect_count_check_frames, emit_count_branch, emit_count_guard_local,
    emit_count_reporter_struct, emit_count_static, emit_log_branch,
};
use backend_common::multi_worker_walker::{collect_pair_tiles, render_wait_assign};
use backend_common::project_skeleton::multi_binary;
use backend_common::render::{
    render_array_init_for, render_const_expr_pub, render_fire_args_pub,
    render_reuse_buf_decls_pub, render_reuse_marker_comment, render_reuse_per_iter_update_pub,
    rust_type_of, RenderCtxPub,
};
pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;
use pthreads_sync::render_single_worker_main_with_kernels_attr;

/// Paths to the files [`emit`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Per-worker `src/bin/<worker>.rs` paths (single-process builds
    /// have exactly one entry).
    pub worker_bins: Vec<PathBuf>,
    /// Path to the emitted `src/kernels.rs`.
    pub kernels_rs: PathBuf,
    /// Path to the emitted `src/wire.rs` (the copied protocol v0).
    pub wire_rs: PathBuf,
    /// Path to the emitted `run.sh`.
    pub run_sh: PathBuf,
}

/// Emit a runnable multi-process Cargo project from the per-worker
/// EventList. Same signature/contract as `pthreads_sync::emit`.
///
/// 0/1 used workers → a single binary whose body is the *shared*
/// single-worker renderer (byte-identical arithmetic to
/// pthreads-sync) plus a no-op run.sh. ≥2 used workers → one binary
/// per used worker, TCP-wired, plus a run.sh that launches them.
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();

    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    let src_dir = out_dir.join("src");
    let bin_dir = src_dir.join("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: bin_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let kernels_rs = src_dir.join("kernels.rs");
    let wire_rs = src_dir.join("wire.rs");
    let run_sh = out_dir.join("run.sh");

    // Shared modules every worker binary `#[path]`-includes.
    //
    // `wire.rs` is emitted PRISTINE: it is a byte-for-byte copy of
    // `mp_tcp_common::WIRE_RUNTIME_SRC` with no test-only branch on this
    // (or any) codegen path. The cross-backend negative falsifier
    // (`NUC_XBACKEND_NEGATIVE`) used to corrupt this source inline here;
    // it was relocated entirely harness-side to nucleus-e2e
    // (`maybe_corrupt_wire_for_xbackend`, post-`nucleus build`,
    // pre-compile) in TASK-0183 — parallel to TASK-0157/0187 for the
    // `NUC_NONDET_TEST` seam. Production codegen now carries zero
    // self-corruption / no `NUC_XBACKEND_NEGATIVE` read.
    write_file(&kernels_rs, &kernels_src)?;
    write_file(&wire_rs, mp_tcp_common::WIRE_RUNTIME_SRC)?;

    if used_workers.len() <= 1 {
        // Single process: reuse the SHARED single-worker renderer so
        // the emitted arithmetic is byte-identical to pthreads-sync.
        let events = used_workers
            .first()
            .and_then(|w| per_worker.get(w))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let body = render_single_worker_main_with_kernels_attr(
            events,
            names,
            sidecar,
            KERNELS_MOD_ATTR_FOR_SRC_BIN,
        )?;
        let bin_path = bin_dir.join("nuc-generated.rs");
        write_file(&bin_path, &body)?;
        write_file(
            &cargo_toml,
            &multi_binary::render_cargo_toml(&[String::from("nuc-generated")], None),
        )?;
        write_file(&run_sh, &multi_binary::render_run_sh_single())?;
        mark_executable(&run_sh);
        return Ok(EmitResult {
            project_dir: out_dir.to_path_buf(),
            cargo_toml,
            worker_bins: vec![bin_path],
            kernels_rs,
            wire_rs,
            run_sh,
        });
    }

    // ---- Multi-process path. ----
    let plan = Plan::build(per_worker, names, sidecar)?;
    let mut bin_names: Vec<String> = Vec::new();
    let mut worker_bins: Vec<PathBuf> = Vec::new();
    for w in &plan.used_workers {
        let wname = plan.worker_name(*w);
        let body = plan.render_worker_program(*w)?;
        let bin_path = bin_dir.join(format!("{wname}.rs"));
        write_file(&bin_path, &body)?;
        bin_names.push(wname);
        worker_bins.push(bin_path);
    }
    // No port-picker binary: each host worker binds 127.0.0.1:0
    // itself and rendezvouses the OS-assigned port through a file
    // (see Plan::render_worker_program / Plan::render_run_sh).
    // TASK-0176 deleted the previous `__nuc_pick_port` helper because
    // its bind-print-exit shape opened a close-then-rebind TOCTOU
    // window between the picker exiting and the host re-binding.

    write_file(
        &cargo_toml,
        &multi_binary::render_cargo_toml(&bin_names, None),
    )?;
    write_file(&run_sh, &plan.render_run_sh()?)?;
    mark_executable(&run_sh);

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        worker_bins,
        kernels_rs,
        wire_rs,
        run_sh,
    })
}

// --------------------------------------------------------------------
// Cargo.toml / run.sh renderers
// --------------------------------------------------------------------
//
// TASK-0257 (cycle 112) lifted the inline `render_cargo_toml` +
// `render_run_sh_single` + the bulk of `Plan::render_run_sh` into
// `backend_common::project_skeleton::multi_binary`. The remaining
// surface here is the per-backend SO_BUF commentary block consumed
// by the shared multi-process emitter (see `SO_BUF_COMMENT_BUFSYNC`
// below) and the data-shape glue around `Plan::max_payload_bytes` /
// `Plan::non_host_workers` that the shared emitter needs as
// parameters.

/// Per-backend SO_BUF commentary block interpolated by the shared
/// [`multi_binary::render_run_sh_multi`] before the
/// `export NUC_SO_BUF=...` line. mp-tcp-bufsync is sync (`buffer=1`)
/// so the requirement is one message in flight — the shared call
/// passes the largest cross-worker payload (sum of element bytes,
/// 64 KiB floor) and this comment explains the sync sizing.
const SO_BUF_COMMENT_BUFSYNC: &str =
    "# Socket buffer requirement from the schedule's per-channel\n\
     # buffer needs (largest single transfer payload, sync=1 msg).\n";

/// `#[path]` attribute block that redirects the shared single-worker
/// renderer's `mod kernels;` at the copied sibling `../kernels.rs`,
/// because mp-tcp-bufsync emits the binary under `src/bin/` rather
/// than as a sibling of `src/kernels.rs`. Passed to
/// `pthreads_sync::render_single_worker_main_with_kernels_attr` as a
/// typed parameter (TASK-0177) — replaces the prior `replacen` token-
/// match against the renderer's literal `mod kernels;` spelling.
const KERNELS_MOD_ATTR_FOR_SRC_BIN: &str = "#[path = \"../kernels.rs\"]\n#[allow(dead_code)]\n";

// --------------------------------------------------------------------
// Multi-process plan
// --------------------------------------------------------------------

/// Stable identifier for one cross-worker data symbol, by sorted
/// `DataId` (deterministic; same order pthreads-sync's slot ids use).
type XferId = usize;

struct Plan<'a> {
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    names: &'a NameTables,
    sidecar: &'a NameSidecar,
    used_workers: Vec<WorkerId>,
    host_worker: WorkerId,
    /// Cross-worker data symbols sorted by DataId.
    xfer_ids: BTreeMap<DataId, XferId>,
    /// Per-(DataId,SeqTag) iteration tile from the originating
    /// XferPlaceholder. Drives the receiver-side leading-axis /
    /// 2D row-loop slice-paste in `backend_common::multi_worker_walker
    /// ::render_wait_assign`. Lifted to the shared helper as of TASK-0296
    /// cycle 116 — before that, mp-tcp-bufsync's Event::Wait emit
    /// rendered `{name} = {dec}` (whole-array overwrite), silently
    /// dropping partition-band slicing on the host gather. The
    /// silent-sibling defect surfaced on 06-separable-filter/distributed
    /// × mp-tcp-bufsync (`tmp` row-band gather: each worker's recv
    /// overwrote the whole `tmp` instead of pasting its band).
    /// pthreads-async + mp-tcp-event currently route through the same
    /// helper via their `WalkerCtx`; if either grows a backend-private
    /// Wait emit path, that path must also call `render_wait_assign`
    /// (= the silent-sibling memory pattern that motivated this fix).
    pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>,
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

        // Host election: shared helper. See
        // `backend_common::host_election` module docstring for the
        // canonical rule. IDENTICAL choice across the four shipped
        // (M1-M4) tier-1 backends' `multi_worker::Plan::build`
        // (pthreads-sync, pthreads-async, mp-tcp-event,
        // mp-tcp-bufsync) AND the three compiler-level driver
        // wirings (cycles 160 / 162 / 163). The two M6 skeleton
        // backends (openmp-rs, mp-tcp-poll) do NOT yet exercise this
        // path — their `emit()` ContractGaps before Plan::build is
        // ever called (per TASK-0044.01/0044.02 skeleton scope). The
        // cross-backend bit-identical differential (PRD §10.1) needs
        // every shipped backend to elect the same host given the
        // same input; the helper is the single source of truth
        // (TASK-0336 cycle 164 lift).
        let host_worker = backend_common::elect_host_from_worker_names(&names.worker, &used_workers)
            .ok_or_else(|| {
                EmitError::ContractGap(
                    "multi-worker emit requires at least one used worker".to_string(),
                )
            })?;

        let mut xfer_data: BTreeSet<DataId> = BTreeSet::new();
        for evs in per_worker.values() {
            collect_xfer_data(evs, &mut xfer_data);
        }
        let xfer_ids: BTreeMap<DataId, XferId> =
            xfer_data.iter().enumerate().map(|(i, d)| (*d, i)).collect();

        // Collect per-pair tiles for slice-aware Wait gathers (TASK-0296
        // cycle 116, hoisted to `collect_pair_tiles` in cycle 130 per
        // TASK-0300). The shared helper preserves deterministic first-
        // sighting-wins on `(DataId, SeqTag)`; both endpoints carry the
        // same tile by XferPlaceholder construction (TASK-0018).
        let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> =
            collect_pair_tiles(per_worker.values());

        // Barrier identity by the contract-carried `SyncTag`
        // (TASK-0172). Each Event::Sync names its own barrier; the
        // projection clones the same participant set into every
        // participant's list, so the first sighting of a tag fixes its
        // participants. No pre-order-index heuristic and no
        // uniform-barrier validation: distinct tags are independent
        // barriers, so a partial/non-uniform barrier lowers correctly.
        let mut barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            collect_barriers_by_tag(&per_worker[w], &mut |tag, parts| {
                barrier_participants
                    .entry(tag)
                    .or_insert_with(|| parts.clone());
            });
        }

        // Topology constraint (UNRELATED to TASK-0172): one stream per
        // (host, worker) pair, so every barrier must include host as
        // the mediating hub. True for the tier-1 set (02-split:
        // {host,w0}). A host-excluding barrier needs host-mediated
        // barrier mediation — the CTRL arm of the cycle-148/149 split
        // of the original combined TASK-0175 filing (DATA arm lifted
        // as TASK-0327; CTRL arm tracked as TASK-0329). Fail loud,
        // never a wrong binary.
        //
        // NB: the ContractGap message text below intentionally still
        // says "filed as TASK-0175" — test-pinned by
        // `nucleus/backends/mp-tcp-event/tests/multi_worker_emit.rs`
        // and `host_relay_emit.rs` for cross-backend differential
        // stability. The forward-link in the prose ABOVE supersedes;
        // do not propose updating the literal message string here.
        for (tag, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                let bid = tag.0;
                return Err(EmitError::ContractGap(format!(
                    "barrier #{bid} participants {parts:?} exclude the host \
                     worker; mp-tcp-bufsync's one-connection-per-(host,worker) \
                     topology requires host as the barrier hub. A \
                     host-excluding barrier needs a worker-to-worker mesh \
                     (filed as TASK-0175)."
                )));
            }
        }

        // TASK-0332 (cycle 151 AC#2): defensive ContractGap for the
        // wait-before-push host-relay deadlock. Cycle-148's
        // synchronous host-relay (`Plan::render_relay_phase`) emits
        // a FLAT relay block whose hops `wire::read_msg_expect(
        // data_<src>, seq)` block on the wire-FIFO. If any non-host
        // worker's first top-level w2w event is a Wait (rather than
        // a Push), host's read blocks for that worker's first Push
        // — which the worker can't reach because it's blocked at
        // its initial Wait. The same defect class fires on
        // mp-tcp-event (cycle-150 empirical reproducer:
        // 05-stencil/distributed-2d × mp-tcp-event); cycle 151 adds
        // this defensive check to BOTH backends per the cycle-148/
        // 149 paired-lift discipline (see
        // [[feedback-silent-sibling-defect]] 10th firing).
        //
        // Conservative-but-sound: rejects every schedule whose first
        // top-level w2w event for ANY non-host worker is a Wait.
        //
        // **Cycle-162 update (TASK-0329.01.01 slice 1, Option D):**
        // the landed architectural fix on the sibling backend is
        // `apply_safe_push_reorder` (a driver-side pass), which
        // hoists hoistable w2w Pushes ahead of preceding w2w Waits.
        // The reorder pass is NOT applied on mp-tcp-bufsync — its
        // per-pair FIFO single-stream constraint 3 (cycle-148 design)
        // makes the analogous splice-point lift unsafe (see memory
        // `project-mp-tcp-event-vs-bufsync-safety-profile`). This
        // detector on mp-tcp-bufsync therefore behaves UNCHANGED
        // from cycle 151: it rejects every wait-before-push shape
        // unconditionally.
        //
        // No in-tree mp-tcp-bufsync schedule triggers this today
        // (the only candidate, 05-stencil/distributed-2d, is
        // capability-skipped on TASK-0042: async + buffer + event
        // not supported by mp-tcp-bufsync's sync transport). The
        // guard remains paired-lifted for fail-loud hygiene if a
        // future capability lift exposes a bufsync-compatible
        // wait-before-push schedule.
        detect_wait_before_push_hazard(per_worker, host_worker)?;

        Ok(Plan {
            per_worker,
            names,
            sidecar,
            used_workers,
            host_worker,
            xfer_ids,
            pair_tiles,
        })
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

    /// Non-host used workers that exchange anything with the host
    /// (every used worker, in the tier-1 set), in WorkerId order.
    fn non_host_workers(&self) -> Vec<WorkerId> {
        self.used_workers
            .iter()
            .copied()
            .filter(|w| *w != self.host_worker)
            .collect()
    }

    /// Render one worker's complete `src/bin/<name>.rs`.
    fn render_worker_program(&self, worker: WorkerId) -> Result<String, EmitError> {
        let mut out = String::new();
        let wname = self.worker_name(worker);
        let is_host = worker == self.host_worker;

        writeln!(
            out,
            "//! Generated by the mp-tcp-bufsync backend (TASK-0036, \
             multi-process). Worker `{wname}`{}.",
            if is_host {
                " [host/server]"
            } else {
                " [client]"
            }
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
        writeln!(out).ok();
        // Role-specific imports so the generated file is
        // warning-clean. BOTH roles need PathBuf + fs for the
        // rendezvous-file handshake; host adds TcpListener +
        // io::Write (file write); non-host adds TcpStream + Duration
        // (connect retry + poll sleep).
        writeln!(out, "use std::fs;").ok();
        writeln!(out, "use std::path::PathBuf;").ok();
        if is_host {
            writeln!(out, "use std::net::TcpListener;").ok();
            writeln!(out, "use std::io::Write as _;").ok();
        } else {
            writeln!(out, "use std::net::TcpStream;").ok();
            writeln!(out, "use std::time::Duration;").ok();
        }
        writeln!(out).ok();

        // TASK-0052.04: per-worker file-scope items for every Count
        // check loop on THIS worker. The collector + reporter struct +
        // static `AtomicU64` shape are shared with pthreads-sync — one
        // implementation, no codegen drift (the cross-backend
        // bit-identical differential per PRD §10.1 depends on this).
        let count_frames = collect_count_check_frames(&self.per_worker[&worker]);
        if !count_frames.is_empty() {
            emit_count_reporter_struct(&mut out);
            for cf in &count_frames {
                // TASK-0222: shared template — see emit_count_static.
                emit_count_static(&mut out, &cf.ident);
            }
            writeln!(out).ok();
        }

        // Connection setup. TWO connections per (host, worker) pair:
        //
        //   - DATA  channel: Push/Wait framed messages.
        //   - CTRL  channel: barrier tokens.
        //
        // WHY TWO: all traffic on ONE stream is a single FIFO whose
        // byte order is whatever the sender emitted. The relative
        // order of a barrier vs a data transfer differs between the
        // producer and the consumer (the host emits `a,b` then a
        // barrier; the worker reaches the barrier first). On one
        // stream the worker's first read would consume the `a` frame
        // as if it were the barrier token — a framing collision.
        // pthreads-sync avoids this only because its `Slot` and
        // `Barrier` are *separate* objects (no shared FIFO). Splitting
        // data from control restores that separation: within the data
        // channel the projection guarantees Push order == Wait order;
        // within the control channel the uniform-barrier pre-order
        // index aligns on both sides. Each channel is independently
        // order-consistent; the cross-channel order never matters.
        //
        // Host is the server: it binds ONE listener per non-host
        // worker and `accept()`s exactly twice — first stream = DATA,
        // second = CTRL (deterministic role by accept order, no
        // handshake bytes). The worker connects twice in the same
        // order (DATA then CTRL), each with a bounded connect-retry
        // (a refused connect only means the listener is not up YET —
        // a liveness wait, not a data sync; deterministic eventual
        // outcome, fail-loud if the host never appears).
        // `unused_assignments`: pre-init declares `let mut d = vec![..]`
        // which is immediately overwritten by the matching Wait recv —
        // intentional (pre-init sizes the slot; the value is the
        // received one). Same shape pthreads-sync's multi-worker
        // pre-init produces; harmless and expected.
        writeln!(
            out,
            "#[allow(unused_mut, dead_code, unused_variables, \
             unused_assignments, clippy::needless_late_init)]"
        )
        .ok();
        writeln!(out, "fn main() {{").ok();

        if is_host {
            // Host: bind 127.0.0.1:0 for EACH non-host worker, then
            // ATOMICALLY publish the OS-assigned port via a rendezvous
            // file. Tmp-file + POSIX rename = atomic on the same FS so
            // a polling worker never reads a partial integer.
            // TASK-0176: replaces the prior env-var handshake whose
            // `__nuc_pick_port` helper had a close-then-rebind TOCTOU.
            writeln!(
                out,
                "    let rendezvous_dir: PathBuf = std::env::var_os(\"NUC_RENDEZVOUS_DIR\")\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.map(PathBuf::from)\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|| panic!(\"host: NUC_RENDEZVOUS_DIR not set (run.sh must export it)\"));"
            )
            .ok();
            writeln!(
                out,
                "    let _ = &rendezvous_dir; // referenced below per-worker"
            )
            .ok();
            // Sequential per-worker bind → publish → accept. For
            // every non-host worker this host serves we bind first,
            // then accept(). The order means worker N+1's port file
            // is not published until worker N has completed BOTH its
            // accepts, so worker N+1 is blocked on the rendezvous
            // poll while host accepts N — the 6s poll bound is shared
            // by all workers, not budgeted per-worker. The cycle-148
            // 06/distributed2 promotion (host + 4 workers; TASK-0327
            // host-relay) confirms this is fine in practice: per-
            // worker accept latency on loopback is ~ms, so 4 workers
            // comfortably finish within the 6s budget. A future
            // schedule with substantially more workers (or slower
            // accept) would need either parallel accepts (per-worker
            // thread) or publish-all-port-files-first, then accept-
            // all in a second pass. Filed: TASK-0176 closure notes
            // carry this forward.
            for nw in self.non_host_workers() {
                let nwn = self.worker_name(nw);
                writeln!(
                    out,
                    "    let listener_{nwn} = TcpListener::bind(\"127.0.0.1:0\")\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: bind 127.0.0.1:0 for {nwn} failed: {{e}}\"));"
                )
                .ok();
                writeln!(
                    out,
                    "    let port_{nwn}: u16 = listener_{nwn}.local_addr()\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: local_addr for {nwn} listener failed: {{e}}\"))\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.port();"
                )
                .ok();
                // Atomic publish: write to <name>.port.tmp then rename
                // to <name>.port. POSIX rename within the same dir is
                // atomic, so a polling worker either sees nothing or
                // sees the full integer — never a partial write.
                // Atomic publish: write the port to <name>.port.tmp,
                // drop the file, rename to <name>.port. POSIX rename
                // within a single directory is atomic — the polling
                // worker either sees nothing or sees the full integer,
                // never a partial write. No `sync_all`: the rendezvous
                // file is process-local-loopback ephemeral state, not
                // durable storage; the page cache is visible across
                // processes immediately.
                writeln!(
                    out,
                    "    {{\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20let rdv_final = rendezvous_dir.join(\"{nwn}.port\");\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20let rdv_tmp = rendezvous_dir.join(\"{nwn}.port.tmp\");\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20let mut f = fs::File::create(&rdv_tmp)\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: create rendezvous tmp `{{}}` for {nwn} failed: {{e}}\", rdv_tmp.display()));\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20write!(f, \"{{}}\", port_{nwn})\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: write port {{}} to rendezvous tmp `{{}}` for {nwn} failed: {{e}}\", port_{nwn}, rdv_tmp.display()));\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20drop(f);\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20fs::rename(&rdv_tmp, &rdv_final)\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: rename rendezvous `{{}}` -> `{{}}` for {nwn} failed: {{e}}\", rdv_tmp.display(), rdv_final.display()));\n\
                     \x20\x20\x20\x20}}"
                )
                .ok();
                writeln!(
                    out,
                    "    let (mut data_{nwn}, _) = listener_{nwn}.accept()\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: accept DATA from {nwn} failed: {{e}}\"));"
                )
                .ok();
                writeln!(
                    out,
                    "    let (mut ctrl_{nwn}, _) = listener_{nwn}.accept()\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: accept CTRL from {nwn} failed: {{e}}\"));"
                )
                .ok();
                writeln!(out, "    data_{nwn}.set_nodelay(true).ok();").ok();
                writeln!(out, "    ctrl_{nwn}.set_nodelay(true).ok();").ok();
                writeln!(out, "    wire::apply_sock_buf(&data_{nwn});").ok();
                writeln!(out, "    wire::apply_sock_buf(&ctrl_{nwn});").ok();
            }
        } else {
            let wn = wname.clone();
            // Non-host worker: poll the rendezvous file the HOST writes
            // after it binds 127.0.0.1:0. Bound: 600 x 10 ms = 6 s
            // (symmetric with `connect_retry` so both liveness waits
            // share one mental budget). If the file never appears the
            // host did not start or failed to bind — fail LOUD naming
            // the path (TASK-0176).
            writeln!(
                out,
                "    let rendezvous_dir: PathBuf = std::env::var_os(\"NUC_RENDEZVOUS_DIR\")\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.map(PathBuf::from)\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|| panic!(\"{wn}: NUC_RENDEZVOUS_DIR not set (run.sh must export it)\"));"
            )
            .ok();
            writeln!(
                out,
                "    let rdv_path = rendezvous_dir.join(\"{wn}.port\");"
            )
            .ok();
            writeln!(
                out,
                "    fn read_rendezvous_port(path: &std::path::Path, who: &str) -> u16 {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20let mut attempt = 0u32;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20loop {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20match fs::read_to_string(path) {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(s) => {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20let trimmed = s.trim();\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20return trimmed.parse::<u16>().unwrap_or_else(|e| panic!(\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\"{{}}: rendezvous file `{{}}` contained `{{}}` which is not a u16: {{}}\",\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20who, path.display(), trimmed, e\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20));\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(_e) => {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20attempt += 1;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if attempt > 600 {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20panic!(\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\"{{}}: rendezvous file `{{}}` did not appear within 6s ({{}} attempts x 10ms) — host worker did not start or failed to bind\",\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20who, path.display(), attempt\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20);\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20std::thread::sleep(Duration::from_millis(10));\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20}}"
            )
            .ok();
            writeln!(
                out,
                "    let port: u16 = read_rendezvous_port(&rdv_path, \"{wn}\");"
            )
            .ok();
            writeln!(
                out,
                "    fn connect_retry(port: u16, role: &str) -> TcpStream {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20let mut attempt = 0u32;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20loop {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20match TcpStream::connect((\"127.0.0.1\", port)) {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Ok(s) => return s,\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20Err(e) => {{\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20attempt += 1;\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if attempt > 600 {{ panic!(\"{wn}: cannot connect {{role}} to host 127.0.0.1:{{port}} after {{attempt}} tries: {{e}}\"); }}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20std::thread::sleep(Duration::from_millis(10));\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
                 \x20\x20\x20\x20}}"
            )
            .ok();
            writeln!(
                out,
                "    let mut data_host = connect_retry(port, \"DATA\");"
            )
            .ok();
            writeln!(
                out,
                "    let mut ctrl_host = connect_retry(port, \"CTRL\");"
            )
            .ok();
            writeln!(out, "    data_host.set_nodelay(true).ok();").ok();
            writeln!(out, "    ctrl_host.set_nodelay(true).ok();").ok();
            writeln!(out, "    wire::apply_sock_buf(&data_host);").ok();
            writeln!(out, "    wire::apply_sock_buf(&ctrl_host);").ok();
        }
        writeln!(out).ok();

        // TASK-0052.04: per-Count-check-loop Drop guard local. The
        // guard's Drop fires when `fn main` returns, printing a
        // stderr summary line. SAME shape as pthreads-sync — single
        // codegen implementation.
        for cf in &count_frames {
            // TASK-0222: shared template — see emit_count_guard_local.
            emit_count_guard_local(&mut out, &cf.ident, &cf.loop_var, cf.latency_max_ns);
        }
        if !count_frames.is_empty() {
            writeln!(out).ok();
        }

        // Pre-init: data this worker Waits on (overwritten on recv)
        // OR writes via an indexed Fire output and never whole-array.
        // Sorted by name. Sized/typed from the sidecar — SAME logic
        // and spelling as pthreads-sync's multi-worker pre-init.
        let pre_init = self.collect_pre_init(worker)?;
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

        let ctx = RenderCtxPub::new(self.names, self.sidecar);

        // TASK-0327 (cycle 148): for HOST, when the schedule has w2w
        // pushes, splice the synchronous relay phase right BEFORE the
        // first top-level Wait event (the gather start). The relay
        // phase drains all worker-to-worker (seq, dst) hops through
        // host's existing data sockets. Insertion-point heuristic
        // (acceptable for the cycle-148 06/distributed2 reproducer
        // shape — scatter → cross-relay → gather): host has no data
        // reads BETWEEN scatter Pushes and gather Waits, so relay-
        // reads on data_<src> don't race host's own reads. If no top-
        // level Wait exists, the relay goes at the END (just before
        // main returns). Non-host workers render unchanged — they
        // just see `data_host` for both directions, host does the
        // forwarding.
        let host_events = &self.per_worker[&worker];
        let host_relay = if is_host {
            self.render_relay_phase(1)?
        } else {
            String::new()
        };
        if is_host && !host_relay.is_empty() {
            let split_at = relay_phase_insertion_point(host_events);
            self.render_events(
                &host_events[..split_at],
                &mut out,
                1,
                worker,
                is_host,
                &ctx,
                None,
            )?;
            out.push_str(&host_relay);
            self.render_events(
                &host_events[split_at..],
                &mut out,
                1,
                worker,
                is_host,
                &ctx,
                None,
            )?;
        } else {
            self.render_events(host_events, &mut out, 1, worker, is_host, &ctx, None)?;
        }

        writeln!(out, "}}").ok();
        Ok(out)
    }

    /// Control-channel variable a given worker uses to barrier with
    /// `peer`. Host: `ctrl_<peer>`; non-host worker: `ctrl_host`.
    fn ctrl_var(&self, self_is_host: bool, peer: WorkerId) -> String {
        if self_is_host {
            format!("ctrl_{}", self.worker_name(peer))
        } else {
            "ctrl_host".to_string()
        }
    }

    /// `enclosing` is the iter-var of the immediately-enclosing
    /// `Event::Loop` (the tile loop, when the child is a strip-mined
    /// inner-block loop with `block_tag.is_partial == false`). `None`
    /// at top level. Mirrors the pthreads-sync single-worker
    /// `render_events_in` parameter (TASK-0180 / TASK-0181).
    ///
    /// Eight params is one over clippy's `too_many_arguments`
    /// threshold; bundling them into a struct would be synthetic
    /// container ceremony for what is a stateless event-walk step
    /// with genuine per-call inputs. Local allow (same rationale as
    /// the shared `multi_worker_walker::render_worker_events_inner`).
    #[allow(clippy::too_many_arguments)]
    fn render_events(
        &self,
        events: &[Event],
        out: &mut String,
        indent: usize,
        worker: WorkerId,
        is_host: bool,
        ctx: &RenderCtxPub<'_>,
        enclosing: Option<IterVar>,
    ) -> Result<(), EmitError> {
        let pad = "    ".repeat(indent);
        for e in events {
            match e {
                Event::Fire {
                    kernel, bindings, ..
                } => {
                    let callee = self.names.kernel.get(kernel).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "kernel id {kernel:?} in a Fire has no name in NameTables"
                        ))
                    })?;
                    // SHARED renderer — no drift from pthreads-sync.
                    let args = render_fire_args_pub(*kernel, &bindings.inputs, ctx)?;
                    match &bindings.output {
                        None => {
                            writeln!(out, "{pad}kernels::{callee}({args});").ok();
                        }
                        Some(o) if o.indices.is_empty() => {
                            let name = self.data_name(o.data)?;
                            writeln!(out, "{pad}let mut {name} = kernels::{callee}({args});").ok();
                        }
                        Some(o) => {
                            // TASK-0209: shared scalar-vs-sub-array
                            // classifier via the pthreads-sync helper.
                            // Same impl as the pthreads-sync single-
                            // and multi-worker Fire-output sites — no
                            // codegen drift between backends, which
                            // the cross-backend bit-identical
                            // differential (PRD §10.1) depends on.
                            let rhs = format!("kernels::{callee}({args})");
                            let stmt = backend_common::render::render_fire_output_assign_pub(
                                o, &rhs, ctx,
                            )?;
                            writeln!(out, "{pad}{stmt}").ok();
                        }
                    }
                }
                Event::Loop {
                    iter_var,
                    range,
                    body,
                    block_tag,
                    check_frame,
                } => {
                    let var = self.names.iter_var.get(iter_var).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "iter var {iter_var:?} in Event::Loop has no name in NameTables"
                        ))
                    })?;

                    // Per-occurrence absolute-index rebinding (TASK-0181;
                    // mirrors pthreads-sync single-worker TASK-0180). Now
                    // delegates to the SHARED
                    // `backend_common::multi_worker_walker::
                    // render_block_tag_loop_header` (TASK-0253) — the same
                    // helper the pthreads-sync / pthreads-async multi-worker
                    // walker calls. The strip-mined inner loop HEADER and
                    // the rebound child `RenderCtxPub` (with `abs_subst`
                    // extended so every Fire arg / index / inner-bound site
                    // substitutes — NOT just the header; the load-bearing
                    // TASK-0181 review-gate finding) are emitted by the
                    // helper. This arm owns only the body recursion through
                    // mp-tcp-bufsync's per-backend substrate (TCP
                    // `ctrl_<peer>` / `sock_<peer>` barriers + host-vs-
                    // worker dispatch in `render_worker_program`) and the
                    // closing `}`. The previous arrangement (TASK-0181
                    // cycle 73) duplicated the rebinding arithmetic
                    // across two files; with this delegation, the
                    // arithmetic lives in exactly one place across all
                    // MULTI-worker backends (cycle-75 review hardening:
                    // a separate sibling copy persists on the
                    // pthreads-sync SINGLE-worker render path, which
                    // uses backend-private RenderCtx — see the helper's
                    // doc-comment for the RenderCtx <-> RenderCtxPub
                    // unification note).
                    if let Some(tag) = block_tag {
                        // TASK-0284 cycle 107: parity with the shared
                        // `multi_worker_walker` strip-mine arm reuse
                        // codegen (TASK-0270 cycle 104). Buffer decls +
                        // prologue MUST live OUTSIDE the for-header (the
                        // buffer must persist across the inner loop's
                        // iterations), so the previous wholesale
                        // delegation to `render_block_tag_loop_header`
                        // (which writes the header itself) is split: use
                        // `compute_block_tag_abs_exprs` for the pure
                        // expressions (returns abs + structurally-built
                        // strip_lo_expr — NO textual replace, mirrors
                        // the cycle-103 fix in pthreads-sync per
                        // `feedback-textual-replace-codegen-unsafe`),
                        // emit buf decls at the OUTER pad, write the
                        // for-header inline, then emit per-iter update
                        // + recurse into body with the child context
                        // carrying BOTH abs_subst AND reuse_active.
                        // `render_block_tag_loop_header` is still
                        // used by callers that don't want reuse codegen
                        // (none currently — pthreads-async / mp-tcp-event
                        // moved to the same pattern in cycle 104).
                        let var_string = var.clone();
                        let (abs, strip_lo_expr) =
                            backend_common::multi_worker_walker::compute_block_tag_abs_exprs(
                                *iter_var, tag, enclosing, ctx,
                            )?;
                        let reuse_groups = render_reuse_buf_decls_pub(
                            out,
                            indent,
                            *iter_var,
                            &var_string,
                            &strip_lo_expr,
                            body,
                            ctx,
                        )?;
                        let mut child_subst = ctx.abs_subst.clone();
                        child_subst.insert(var_string.clone(), abs.clone());
                        let mut child_reuse = ctx.reuse_active.clone();
                        for (data_id, gs) in reuse_groups.clone() {
                            child_reuse.insert(data_id, gs);
                        }
                        let child_ctx =
                            ctx.with_abs_subst_and_reuse_active(child_subst, child_reuse);
                        // Header line: concrete folded range
                        // (`{start}_i64..{end}_i64`) — NOT the source-form
                        // bound (would re-introduce the full range) and
                        // NOT the partition slice (the strip-mined inner
                        // loop iterates over the tile, not the worker's
                        // partition slice).
                        writeln!(
                            out,
                            "{pad}for {var_string} in ({}_i64)..({}_i64) {{",
                            range.start, range.end
                        )
                        .ok();
                        // Marker (preserved; substring `reuse_widths_pending`
                        // is load-bearing for the cross-backend grep tests).
                        render_reuse_marker_comment(
                            out,
                            indent + 1,
                            *iter_var,
                            &var_string,
                            ctx.sidecar,
                            ctx.names,
                        );
                        // Per-iter update: iv expression is the rebound
                        // ABSOLUTE expression so the source-array index
                        // reflects the strip-mined coordinate.
                        render_reuse_per_iter_update_pub(
                            out,
                            indent + 1,
                            &reuse_groups,
                            &abs,
                            &child_ctx,
                        )?;
                        self.render_events(
                            body,
                            out,
                            indent + 1,
                            worker,
                            is_host,
                            &child_ctx,
                            Some(*iter_var),
                        )?;
                        writeln!(out, "{pad}}}").ok();
                        continue;
                    }
                    // Per-worker partition override (TASK-0212): if the
                    // partition pass recorded a slice for THIS worker on
                    // this iter var, render the concrete literal range.
                    // See pthreads-sync multi_worker for the precedence
                    // rationale (concrete-per-worker > symbolic-source-
                    // form > concrete-folded fallback).
                    let partition_slice = self
                        .sidecar
                        .partition_worker_ranges
                        .get(iter_var)
                        .and_then(|m| m.get(&worker));
                    let (lo, hi) = match partition_slice {
                        Some(r) => (format!("{}_i64", r.start), format!("{}_i64", r.end)),
                        None => match self.sidecar.loop_bounds.get(iter_var) {
                            Some(b) => (
                                render_const_expr_pub(&b.lo, ctx)?,
                                render_const_expr_pub(&b.hi, ctx)?,
                            ),
                            None => (format!("{}_i64", range.start), format!("{}_i64", range.end)),
                        },
                    };
                    // TASK-0284 cycle 107: regular arm reuse codegen
                    // parity with the shared walker (TASK-0270 cycle
                    // 104). Buffer decls + prologue at OUTER pad BEFORE
                    // the for-header; per-iter update + body recursion
                    // inside the loop with a child context carrying the
                    // reuse_active map. NO-OP when the iv carries no
                    // reuse (preserves byte-identicality on every
                    // mp-tcp-bufsync cell shipped pre-TASK-0284).
                    let reuse_groups = render_reuse_buf_decls_pub(
                        out, indent, *iter_var, var, &lo, body, ctx,
                    )?;
                    writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                    // Real-time `check loop V : latency_max=T` codegen
                    // (TASK-0052.02). Mirrors the pthreads-sync
                    // single-worker emit: `Instant::now()` at iter
                    // start, comparison + panic at iter end. Determinism
                    // preserved: the emitted bytes on the success path
                    // are unchanged (the instant is consumed locally,
                    // never written to wire / stdout), and panic exits
                    // with rustc's standard code 101 — the cross-backend
                    // differential treats "exit 101 + empty stdout" as
                    // an assertion signal, NOT a corrupt-output false
                    // positive.
                    //
                    // Test coverage: the emit-string pattern is pinned
                    // by `mp_tcp_bufsync_emit_includes_panic_instrumentation_on_check_loop`
                    // (TASK-0052.02 review-gate finding #2). No tier-1
                    // e2e cell uses `check loop` today; the
                    // string-assertion test is the lower-bound
                    // verification that this backend emits the
                    // contracted shape.
                    let body_indent = indent + 1;
                    let body_pad = "    ".repeat(body_indent);
                    // TASK-0284 cycle 107: marker + per-iter update at
                    // body entry (mirrors the shared walker regular
                    // arm). Marker substring `reuse_widths_pending`
                    // preserved as cross-backend canary. Both the
                    // check_frame and non-check_frame body-recursion
                    // arms below use `body_ctx` (the child ctx carrying
                    // the new `reuse_active` map) so any DataRef
                    // rewrite reaches the body.
                    render_reuse_marker_comment(
                        out,
                        body_indent,
                        *iter_var,
                        var,
                        ctx.sidecar,
                        ctx.names,
                    );
                    let mut child_reuse = ctx.reuse_active.clone();
                    for (data_id, gs) in reuse_groups.clone() {
                        child_reuse.insert(data_id, gs);
                    }
                    let body_ctx = ctx.with_reuse_active(child_reuse);
                    render_reuse_per_iter_update_pub(
                        out,
                        body_indent,
                        &reuse_groups,
                        var,
                        &body_ctx,
                    )?;
                    if let Some(frame) = check_frame {
                        // TASK-0221 (a): defensive — `var` (NameTables)
                        // and `frame.loop_var` (CheckFrame) must name
                        // the same user-source loop variable. Dev-only
                        // assert catches future projection divergence.
                        debug_assert_eq!(
                            var.as_str(),
                            frame.loop_var.as_str(),
                            "CheckFrame.loop_var diverged from NameTables.iter_var \
                             (projection-layer bug; TASK-0221)"
                        );
                        writeln!(
                            out,
                            "{body_pad}let _check_start = std::time::Instant::now();"
                        )
                        .ok();
                        self.render_events(
                            body,
                            out,
                            body_indent,
                            worker,
                            is_host,
                            &body_ctx,
                            Some(*iter_var),
                        )?;
                        writeln!(
                            out,
                            "{body_pad}let _check_elapsed = _check_start.elapsed().as_nanos();"
                        )
                        .ok();
                        match frame.on_violation {
                            nucleus_compiler::event::ViolationKind::Panic => {
                                writeln!(
                                    out,
                                    "{body_pad}if _check_elapsed > {ns}_u128 {{ panic!(\"latency budget violated on `check loop {lv}`: iteration took {{}} ns, max {ns} ns\", _check_elapsed); }}",
                                    ns = frame.latency_max_ns,
                                    lv = frame.loop_var,
                                )
                                .ok();
                            }
                            nucleus_compiler::event::ViolationKind::Log => {
                                // TASK-0052.04. eprintln per violation;
                                // execution continues. Mirrors the
                                // pthreads-sync emit verbatim — the
                                // cross-backend differential test pins
                                // this in
                                // `mp_tcp_bufsync_emit_includes_log_eprintln_on_check_loop`.
                                // TASK-0222: shared template — see emit_log_branch.
                                emit_log_branch(
                                    out,
                                    &body_pad,
                                    &frame.loop_var,
                                    frame.latency_max_ns,
                                );
                            }
                            nucleus_compiler::event::ViolationKind::Count => {
                                // TASK-0052.04. The static counter +
                                // Drop guard are emitted at file scope
                                // by `render_worker_program` above
                                // (`collect_count_check_frames` walks
                                // the SAME events). Relaxed ordering is
                                // sufficient: the fetch_add and the
                                // Drop-time load both happen on the
                                // worker process's main thread.
                                // TASK-0222: shared template — see emit_count_branch.
                                let id =
                                    backend_common::check_frame::sanitize_loop_var(&frame.loop_var);
                                emit_count_branch(out, &body_pad, &id, frame.latency_max_ns);
                            }
                        }
                    } else {
                        self.render_events(
                            body,
                            out,
                            body_indent,
                            worker,
                            is_host,
                            &body_ctx,
                            Some(*iter_var),
                        )?;
                    }
                    writeln!(out, "{pad}}}").ok();
                }
                Event::Sync {
                    participants, sync, ..
                } => {
                    // Barrier identity is the contract-carried SyncTag
                    // (TASK-0172). It is the wire `barrier_cross`
                    // token, so host and worker must agree on it:
                    // every participant of this barrier carries the
                    // SAME tag by construction, so they do — including
                    // for partial/non-uniform barriers where the old
                    // per-worker pre-order index would have diverged.
                    let bid = sync.0;
                    // Host-mediated star barrier. Host crosses with
                    // every non-host participant (deterministic
                    // WorkerId order); a non-host worker crosses with
                    // host only. 2-party (tier-1) is the trivial
                    // case. The `barrier_cross` helper is
                    // send-then-recv on both ends — safe over a
                    // duplex stream for a 16-byte token.
                    if is_host {
                        let mut peers: Vec<WorkerId> = participants
                            .iter()
                            .copied()
                            .filter(|p| *p != self.host_worker)
                            .collect();
                        peers.sort_unstable();
                        for p in peers {
                            let cv = self.ctrl_var(true, p);
                            writeln!(out, "{pad}wire::barrier_cross(&mut {cv}, {bid});").ok();
                        }
                    } else {
                        writeln!(out, "{pad}wire::barrier_cross(&mut ctrl_host, {bid});").ok();
                    }
                }
                Event::Push { data, dst, seq, .. } => {
                    let _xid = self.xfer_ids.get(data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Push of data {data:?} not collected as cross-worker"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let ty = self.sidecar.data_type(*data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "cross-worker data `{name}` ({data:?}) has no ResolvedType"
                        ))
                    })?;
                    let enc = encode_expr(&name, ty)?;
                    // The connection to the *destination* worker. The
                    // dst must be a peer of this worker on the
                    // (data,ctrl)-pair-per-(host,worker) topology.
                    let cv = self.data_conn_var(worker, is_host, *dst)?;
                    let to = self.worker_name(*dst);
                    writeln!(
                        out,
                        "{pad}wire::write_msg(&mut {cv}, {}, &{enc}); // send `{name}` to {to}",
                        seq.0
                    )
                    .ok();
                }
                Event::Wait { data, src, seq, .. } => {
                    let _xid = self.xfer_ids.get(data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Wait of data {data:?} not collected as cross-worker"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let ty = self.sidecar.data_type(*data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "cross-worker data `{name}` ({data:?}) has no ResolvedType"
                        ))
                    })?;
                    let cv = self.data_conn_var(worker, is_host, *src)?;
                    let from = self.worker_name(*src);
                    let dec = decode_expr(ty)?;
                    // TASK-0296 cycle 116: route Wait gather through the
                    // shared backend-common slice-paste helper. Before
                    // this, the host-side emit was `{name} = {dec};`
                    // (whole-array overwrite) regardless of the pair's
                    // tile — partition-band gathers silently lost their
                    // slice, e.g. 06-separable-filter/distributed × mp-
                    // tcp-bufsync overwrote `tmp` per recv instead of
                    // pasting each worker's hy row-band. The shared
                    // helper dispatches whole-array vs 1D leading-axis
                    // vs 2D row-loop slice-paste from the IterTile;
                    // pthreads-async + mp-tcp-event already went via
                    // this helper (silent-sibling defect closure for
                    // mp-tcp-bufsync).
                    let assign = render_wait_assign(
                        self.sidecar,
                        &self.pair_tiles,
                        &name,
                        *data,
                        *seq,
                        &dec,
                    )?;
                    writeln!(
                        out,
                        "{pad}{{ let __buf = wire::read_msg_expect(&mut {cv}, {}); \
                         {assign} }} // recv `{name}` from {from}",
                        seq.0
                    )
                    .ok();
                }
                Event::Alloc { .. } | Event::Free { .. } => {
                    // RAII Vec storage; no explicit reservation (same
                    // as pthreads-sync).
                }
            }
        }
        Ok(())
    }

    /// The DATA-channel variable for a Push/Wait whose peer is
    /// `peer`. On the star topology a non-host worker only ever owns
    /// a single TCP connection — to host. TASK-0327 (cycle 148) lifts
    /// the prior fail-loud rejection of non-host peers: when a non-
    /// host worker's Push/Wait names another non-host worker, it
    /// writes/reads on its existing `data_host` connection and HOST
    /// runs a SYNCHRONOUS RELAY PHASE (see [`Plan::relay_schedule`] +
    /// the emit in [`render_relay_phase`]) that drains the matching
    /// (seq, dst) entry from `data_<src>` and forwards it verbatim to
    /// `data_<dst>`. Host stays the sole party that owns the (data,
    /// ctrl)-pair-per-(host, worker) topology; no worker-to-worker
    /// socket is added. (Filed forward as TASK-0327 sibling for mp-
    /// tcp-event; TASK-0175 for the eventual full-mesh path.)
    fn data_conn_var(
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
            // TASK-0327 (cycle 148): non-host peer is now routed via
            // host-relay — the worker uses its existing `data_host`
            // connection for both directions; HOST relays bytes
            // through to/from the actual peer. See module-doc + the
            // relay phase emit in `render_relay_phase`.
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
    fn relay_schedule(&self) -> Result<BTreeMap<WorkerId, Vec<RelayHop>>, EmitError> {
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

    /// TASK-0327 (cycle 148): emit host's synchronous relay phase as
    /// a String — for each src in BTreeMap (sorted WorkerId) order,
    /// for each hop in src's event-list order, read `expect_seq` from
    /// `data_<src>` and forward to `data_<dst>`. The seq cross-check
    /// (`read_msg_expect`) preserves the wire-protocol-v0 fail-loud
    /// contract: a mismatch means the deterministic event order
    /// diverged across the three endpoints (src worker, host relay,
    /// dst worker) — a codegen regression, never silently tolerated.
    ///
    /// Returns `EmitError::ContractGap` if any hop's `DataId` lacks
    /// a name in `NameTables` (a contract violation the existing
    /// Push/Wait emit also fails-loud on — cycle-148 architect P2.2
    /// fold-back replaced an earlier silent comment-fallback).
    fn render_relay_phase(&self, indent: usize) -> Result<String, EmitError> {
        let pad = "    ".repeat(indent);
        let schedule = self.relay_schedule()?;
        if schedule.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        writeln!(
            out,
            "{pad}// TASK-0327 host-relay phase: forward worker-to-worker Push/Wait\n\
             {pad}// pairs through host's existing (data, ctrl)-pair-per-(host, worker)\n\
             {pad}// star topology. SYNCHRONOUS: read from data_<src>, write to data_<dst>,\n\
             {pad}// one (seq, dst) hop at a time, srcs iterated in sorted-WorkerId order."
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
                     let __relay_payload = wire::read_msg_expect(&mut data_{src_name}, {}); \
                     wire::write_msg(&mut data_{dst_name}, {}, &__relay_payload); \
                     }} // relay `{data_name}` from {src_name} to {dst_name}",
                    hop.seq.0, hop.seq.0
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
    fn collect_pre_init(&self, worker: WorkerId) -> Result<Vec<(String, DataId)>, EmitError> {
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
    /// cycle 112), supplying the per-backend SO_BUF commentary +
    /// the host-first worker ordering.
    ///
    /// Socket buffer sizing rationale: mp-tcp-bufsync is sync
    /// (`buffer=1`) so the requirement is one message; size
    /// SO_*BUF from the largest cross-worker payload (sum of element
    /// bytes) with a 64 KiB floor. Per-worker `setsockopt` follows
    /// the capabilities.toml contract. v2 uses the single highest
    /// requirement (AC#7 limitation: no per-channel granularity if an
    /// OS-level cap binds).
    fn render_run_sh(&self) -> Result<String, EmitError> {
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
            SO_BUF_COMMENT_BUFSYNC,
        ))
    }

    /// Largest single cross-worker payload in bytes (sum of element
    /// byte widths). Drives SO_*BUF sizing in run.sh. Sized from the
    /// sidecar `ResolvedType` — no AlgoIR.
    fn max_payload_bytes(&self) -> Result<usize, EmitError> {
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

fn scalar_width(t: &nucleus_compiler::algo::ScalarType) -> usize {
    use nucleus_compiler::algo::ScalarType::*;
    match t {
        I8 | U8 | Bool => 1,
        I16 | U16 => 2,
        I32 | U32 | F32 => 4,
        I64 | U64 | F64 => 8,
        // usize/isize: 8 on every supported (x86-64 loopback) target.
        Usize | Isize => 8,
    }
}

/// `wire::enc_*` / `wire::enc_vec(...)` call for a value named `name`
/// of resolved type `ty`. Encoder is fixed at compile time from the
/// sidecar (TASK-0037 AC#3) — sender/receiver agree by construction.
fn encode_expr(name: &str, ty: &nucleus_compiler::algo::ResolvedType) -> Result<String, EmitError> {
    use nucleus_compiler::algo::ScalarType::Bool;
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        Ok(format!("wire::enc_{s}({name})"))
    } else if ty.scalar == Bool {
        // `bool` has no `to_le_bytes`; dedicated 1-byte-per-element.
        Ok(format!("wire::enc_vec_bool(&{name})"))
    } else {
        let rs = backend_common::render::rust_scalar_type_pub(&ty.scalar);
        Ok(format!("wire::enc_vec(&{name}, {rs}::to_le_bytes)"))
    }
}

/// Expression that decodes `__buf` back into the value's Rust type.
fn decode_expr(ty: &nucleus_compiler::algo::ResolvedType) -> Result<String, EmitError> {
    use nucleus_compiler::algo::ScalarType::Bool;
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        Ok(format!("wire::dec_{s}(&__buf)"))
    } else if ty.scalar == Bool {
        Ok("wire::dec_vec_bool(&__buf)".to_string())
    } else {
        let rs = backend_common::render::rust_scalar_type_pub(&ty.scalar);
        Ok(format!("wire::dec_vec(&__buf, {rs}::from_le_bytes)"))
    }
}

fn scalar_fn_suffix(t: &nucleus_compiler::algo::ScalarType) -> &'static str {
    use nucleus_compiler::algo::ScalarType::*;
    match t {
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        F32 => "f32",
        F64 => "f64",
        Bool => "bool",
        // usize/isize encoded as their 8-byte counterparts on the
        // only supported target (x86-64 loopback). A mixed-width
        // target would bump the protocol version (which v0 lacks).
        Usize => "u64",
        Isize => "i64",
    }
}

// --------------------------------------------------------------------
// Event-walk helpers (recurse into Loop bodies) — same shapes as
// pthreads-sync's multi_worker walkers (kept here because the
// transport-specific Plan differs; the *expression* rendering is the
// shared part and is NOT re-implemented).
// --------------------------------------------------------------------

/// **Loop-body interaction with TASK-0330**: this walker recurses into
/// `Event::Loop` bodies and accumulates DataIds via a set (`insert`),
/// so a Loop-body Push/Wait is idempotent across iterations and benign
/// here regardless of TASK-0330's guard. The TASK-0330 guard rejects
/// the w2w-Push-in-Loop shape upstream in `collect_w2w_pushes` before
/// it would matter; this walker's set-union shape is incidentally
/// robust independently.
fn collect_xfer_data(events: &[Event], out: &mut BTreeSet<DataId>) {
    for e in events {
        match e {
            Event::Push { data, .. } | Event::Wait { data, .. } => {
                out.insert(*data);
            }
            Event::Loop { body, .. } => collect_xfer_data(body, out),
            _ => {}
        }
    }
}

/// TASK-0327 (cycle 148): one host-relay hop = "read seq N from
/// data_<src>, write seq N to data_<dst>". `data` is the DataId for
/// codegen comment only; the wire pass-through is bytes-verbatim.
#[derive(Debug, Clone, Copy)]
struct RelayHop {
    seq: SeqTag,
    dst: WorkerId,
    data: DataId,
}

/// TASK-0327 (cycle 148): pick the position in HOST's top-level
/// event list at which the host-relay phase should splice in.
///
/// Constraints driving the choice (the 06/distributed2 shape):
///
/// 1. Workers reach their pass-2-end barrier only AFTER receiving
///    their cross-tmps (which require relay) and computing pass 2.
///    So relay must happen BEFORE host's LAST top-level
///    `Event::Sync` — otherwise host blocks at that barrier waiting
///    for workers whose progress is gated on the relay we haven't
///    run yet (circular wait = deadlock).
///
/// 2. Workers reach their pass-1-end barrier (typically the FIRST
///    `Event::Sync` on workers) BEFORE pushing their tmps; so relay
///    needs the workers to have crossed that barrier, which means
///    host must have crossed it too — i.e. relay AFTER host's first
///    Sync (if any) is OK and required.
///
/// 3. (Sub-fallback constraint, only when no top-level Sync exists)
///    Relay reads from `data_<src>` would race host's own reads on
///    the same socket, so relay must happen BEFORE any host Wait
///    on a worker that also has w2w pushes. In practice (with no
///    Sync to anchor on): before the first top-level Wait.
///
/// Heuristic resolution — priority order:
///
/// - **Primary**: insert just BEFORE the LAST top-level
///   `Event::Sync` (= relay happens between the pass-1 barrier and
///   the pass-2 barrier — satisfies constraints 1 + 2). Picked
///   for any schedule whose host events contain >= 1 top-level
///   `Sync`; 06/distributed2 lands here (two top-level Syncs).
/// - **Fallback** (no top-level Sync exists): insert just BEFORE
///   the first top-level `Event::Wait` (the gather start —
///   satisfies constraint 3 alone, which is sufficient when there
///   is no barrier to consider).
/// - **Last resort** (no Sync, no Wait): insert at end.
///
/// "Top-level" = not nested in an `Event::Loop`. The implementation
/// uses `rposition` for the primary (last-Sync) and `position` for
/// the fallback (first-Wait), reflecting the priority above.
///
/// Acceptable cycle-148 limitation: any schedule with a host
/// `Sync`-or-`Wait`-AFTER-the-w2w-relay-window structure that does
/// not match this heuristic would deadlock or race; the 06/
/// distributed2 reproducer + the existing 02-split (no w2w) cell +
/// the 03-reduction/distributed cell (no w2w — blocked on
/// host-excluding-barrier, separate gap) all satisfy it.
///
/// ## TASK-0329.01.01 (slice 1) — backend asymmetry NOT applied here
///
/// The sibling `nucleus/backends/mp-tcp-event/src/multi_worker.rs`
/// `relay_phase_insertion_point` was updated in slice 1 of
/// TASK-0329.01.01 to walk worker events by `SyncTag` and return the
/// FIRST Sync after which every non-host worker has finished w2w
/// activity. That change is mp-tcp-event-only: on mp-tcp-event,
/// constraint 3 above is INERT (per-seq demux removes the
/// stream-race hazard), so the relay can splice before host's own
/// w2w Waits without a race. Bufsync uses one ordered DATA stream
/// per `(host, worker)` pair — moving the relay earlier here would
/// race host's own reads on `data_<src>` (constraint 3 ACTIVE). The
/// 05/distributed-2d wait-before-push hazard would, on bufsync, need
/// either a threaded relay or a per-pair-multiplex change to the
/// wire codec — neither in scope for slice 1. Per memory
/// `project-mp-tcp-event-vs-bufsync-safety-profile` the per-seq vs
/// FIFO distinction is load-bearing for this asymmetry.
fn relay_phase_insertion_point(events: &[Event]) -> usize {
    if let Some(idx) = events
        .iter()
        .rposition(|e| matches!(e, Event::Sync { .. }))
    {
        return idx;
    }
    if let Some(idx) = events.iter().position(|e| matches!(e, Event::Wait { .. })) {
        return idx;
    }
    events.len()
}

/// TASK-0327 (cycle 148): collect every Push event where the dst is
/// a non-host worker — these are the w2w pushes that host must relay.
/// Recurses into Loop bodies in event-list order; the relay block is
/// emitted FLAT outside any loop.
///
/// **TASK-0330 active guard** (defensive ContractGap):
/// when the recursion is INSIDE an `Event::Loop` body and encounters a
/// w2w `Push`, returns [`EmitError::ContractGap`] forward-linking
/// TASK-0330. The flat relay block would either over-count (host reads
/// once per (seq, dst) but the worker pushes N times around the loop)
/// or mis-order (the flat read order would not align with the loop's
/// nested iteration order). Fail-loud at codegen > silent miscompile or
/// runtime deadlock, per [[feedback-panic-not-diagnostic-recurring]].
///
/// In-tree schedules today have all w2w Pushes at TOP LEVEL — verified
/// by `host_relay_emit` and the cycle-148 reviewer audit — so this
/// guard is dormant on the current matrix; it pins the contract for a
/// future schedule shape, with test pins in
/// `nucleus/backends/mp-tcp-bufsync/tests/loop_body_w2w_push.rs`.
///
/// **TASK-0329.01.02 cycle 163 (slice 2) AC#5 bufsync audit + cycle-166
/// reframe — guard stays as-is on bufsync; pass NOT mirrored:**
/// the compiler-level `apply_host_data_relay_inject` pass that lifts
/// this guard on mp-tcp-event (sibling backend) is intentionally NOT
/// wired into the driver for mp-tcp-bufsync. Reasoning:
/// (a) mp-tcp-bufsync's 09/13 cells are capability-gated on
///     async/buffer/event so behavioral verification of any pass
///     effect on bufsync would be impossible (capability-skip happens
///     BEFORE codegen);
/// (b) bufsync's per-pair FIFO single stream + `wire::read_msg_expect`
///     panic-on-seq-mismatch (memory
///     `project-mp-tcp-event-vs-bufsync-safety-profile`) has a
///     different failure profile than mp-tcp-event's per-seq-demux;
///     enabling the pass on bufsync without a runtime verification
///     path is a defensible-gain-of-zero risk.
///
/// **Residual safety-net scope (cycle 166 paired with mp-tcp-event
/// sibling).** Because the pass is NOT enabled on bufsync, this
/// guard's reachable shape set is BROADER than the sibling's: every
/// Loop-body w2w `Push` reaches this guard, whereas on mp-tcp-event
/// only the cycle-163b residual classes do. For cross-backend
/// vocabulary parity (so a future reviewer can grep both sibling
/// docstrings consistently), those classes are:
/// - **(R-bare)** A bare `Xfer` outside any parent `Sequence` (would
///   only matter on this backend if the pass were eventually enabled
///   here AND `transfer_inject`'s contract weakened).
/// - **(R-singleton)** A `Push`/`Wait` without its matching sibling
///   endpoint in the same `Sequence` (same conditional applies).
///
/// On bufsync today the operative class is simply "any Loop-body w2w
/// Push" — the residuals (R-bare)/(R-singleton) become operative only
/// if a future cycle enables the pass here.
///
/// **Affirmative structural finding (cycle-163b architect P2.1
/// fold-back):** the B2 rewrite splits one non-host pair `(w_src,
/// w_dst)` into two pairs `(w_src, host)` and `(host, w_dst)`. Each
/// resulting hop is a single-pair stream with its own monotonically-
/// allocated `seq` (from `max_existing_seq + 1`). The per-pair
/// FIFO invariant `wire::read_msg_expect` relies on is therefore
/// preserved per resulting hop — the pass does NOT introduce a
/// latent seq-mismatch panic surface on future capability-compatible
/// schedules. Skipping the pass on bufsync today is a
/// gain-of-zero-for-cells-that-can't-run risk-mitigation choice, not
/// a "the pass would corrupt bufsync" structural barrier.
///
/// If a future cycle relaxes bufsync's capability gate (or the
/// async/buffer/event semantics are mirrored to a poll/sync transport),
/// re-evaluate whether to enable `apply_host_data_relay_inject` on
/// bufsync. The pass itself is backend-agnostic — only the driver
/// wiring is conditional.
fn collect_w2w_pushes(
    events: &[Event],
    host: WorkerId,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    collect_w2w_pushes_inner(events, host, false, out)
}

fn collect_w2w_pushes_inner(
    events: &[Event],
    host: WorkerId,
    inside_loop: bool,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    for e in events {
        match e {
            Event::Push { dst, data, seq, .. } if *dst != host => {
                if inside_loop {
                    return Err(EmitError::ContractGap(format!(
                        "mp-tcp-bufsync: TASK-0330 defensive guard — \
                         worker-to-worker Push (data={data:?}, dst={dst:?}, \
                         seq={seq:?}) found INSIDE an Event::Loop body. The \
                         cycle-148 host-relay (TASK-0327) emits the relay \
                         block FLAT outside any loop, so a nested w2w Push \
                         would either over-count (host reads once per \
                         (seq, dst) but the worker pushes N times around \
                         the loop) or mis-order (the flat read order would \
                         not align with the loop's nested iteration order). \
                         No in-tree schedule trips this today. The \
                         mp-tcp-event sibling carries a compiler-pass \
                         remediation (`apply_host_data_relay_inject`, \
                         TASK-0329.01.02 cycle 163 + TASK-0329.01.02.01 \
                         cycle 165) wired only for mp-tcp-event per the \
                         per-pair FIFO constraint that makes splice-point \
                         lift unsafe on bufsync (see memory \
                         `project-mp-tcp-event-vs-bufsync-safety-profile`). \
                         If a future bufsync-capable schedule needs the \
                         equivalent, file a follow-up; the pass itself is \
                         backend-agnostic at the ACFG layer and would only \
                         require driver-side wiring + a fresh FIFO audit."
                    )));
                }
                out.push(RelayHop {
                    seq: *seq,
                    dst: *dst,
                    data: *data,
                });
            }
            Event::Loop { body, .. } => {
                collect_w2w_pushes_inner(body, host, true, out)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// TASK-0332 (cycle 151 AC#2): detect the wait-before-push hazard at
/// codegen time so the synchronous host-relay's circular-seq-dependency
/// deadlock surfaces as a typed `EmitError::ContractGap` instead of a
/// runtime timeout. See the call site in `Plan::build` for the full
/// design narrative; this is the conservative-but-sound implementation.
///
/// Sibling: the same function exists in
/// `nucleus/backends/mp-tcp-event/src/multi_worker.rs` with the same
/// shape + a backend-specific message prefix. Per the cycle-148/149
/// paired-lift discipline ([[feedback-silent-sibling-defect]] 10th
/// firing), the two implementations were added in the same cycle.
///
/// Cycle-151 architect P1 fold-back: this function was originally
/// placed BEFORE `collect_w2w_pushes` with no blank-line separator,
/// which caused `collect_w2w_pushes`'s cycle-148 docstring to be
/// silently absorbed into this docstring (a paired-lift sibling-
/// defect — the mp-tcp-event sibling avoided it by accident of file
/// structure). Folded back by moving this function AFTER
/// `collect_w2w_pushes`, restoring the docstring boundary.
fn detect_wait_before_push_hazard(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> Result<(), EmitError> {
    for (&w, events) in per_worker {
        if w == host {
            continue;
        }
        // Precondition: this worker must have at least one w2w Push
        // for the deadlock cycle to involve it. A "pure consumer"
        // worker (only w2w Waits, no w2w Pushes) is NOT a src in
        // `Plan::relay_schedule`, so host's relay does not wait FOR
        // it, so this worker's wait-before-anything pattern cannot
        // close a deadlock cycle from its own side. Pure-consumer
        // workers are SAFE under host-relay.
        //
        // Cycle-151 architect P2 note (RESOLVED by TASK-0330): this
        // precondition scans only TOP-LEVEL events for w2w Pushes; the
        // `collect_w2w_pushes` helper recurses into Loop bodies. A
        // worker with Wait at top level + Push inside a Loop body
        // would be a false-negative for THIS detector — but TASK-0330
        // now fires a fail-loud ContractGap in `collect_w2w_pushes` for
        // any w2w Push found INSIDE a Loop body, so the Loop-body
        // hazard is rejected later in the pipeline (in
        // `render_relay_phase` rather than here in `Plan::build`). The
        // composition is sound: a Loop-body w2w Push CANNOT silently
        // reach codegen on either backend.
        let has_w2w_push = events
            .iter()
            .any(|e| matches!(e, Event::Push { dst, .. } if *dst != host));
        if !has_w2w_push {
            continue;
        }
        for e in events {
            match e {
                // First top-level w2w event is a Push — safe shape;
                // host's relay can drain this worker's outbound first.
                Event::Push { dst, .. } if *dst != host => break,
                // First top-level w2w event is a Wait — hazard shape;
                // host's relay would deadlock waiting for THIS worker's
                // Push (which the worker can't reach because it's
                // blocked at this Wait).
                Event::Wait { src, .. } if *src != host => {
                    return Err(EmitError::ContractGap(format!(
                        "mp-tcp-bufsync: worker {w:?} has a worker-to-worker \
                         Wait (from src {src:?}) at top level before any \
                         worker-to-worker Push. Cycle-148's synchronous \
                         host-relay would deadlock on the circular seq \
                         dependency: host's wire::read_msg_expect blocks \
                         for this worker's first Push; this worker blocks \
                         at this Wait for host's relay of the seq from \
                         {src:?}. TASK-0332 cycle 151 filed this defensive \
                         guard. Note: TASK-0329.01.01 (slice-1 Option D \
                         push-before-wait reorder) is wired on mp-tcp-event \
                         ONLY — bufsync's per-pair FIFO constraint 3 (per \
                         cycle-148 design + memory \
                         `project-mp-tcp-event-vs-bufsync-safety-profile`) \
                         makes the splice-point lift unsafe on this \
                         backend, so the reorder pass cannot be enabled \
                         here. If a future capability lift exposes a \
                         bufsync-compatible wait-before-push schedule, a \
                         backend-specific architectural fix would be \
                         needed."
                    )));
                }
                // Non-w2w events (Push/Wait with host as the other
                // endpoint, Fire, Sync, Loop, Alloc, Free) don't
                // affect the hazard. Loop bodies are intentionally
                // NOT walked — the hazard is about top-level event
                // order, not nested.
                _ => continue,
            }
        }
    }
    Ok(())
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index,
/// no fallibility (every tag is an independent barrier; nothing to
/// validate / reject here any more).
fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
where
    F: FnMut(SyncTag, &BTreeSet<WorkerId>),
{
    for e in events {
        match e {
            Event::Sync {
                participants, sync, ..
            } => f(*sync, participants),
            Event::Loop { body, .. } => collect_barriers_by_tag(body, f),
            _ => {}
        }
    }
}

fn collect_pre_init_sets(
    events: &[Event],
    waited: &mut BTreeSet<DataId>,
    whole: &mut BTreeSet<DataId>,
    indexed: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, .. } => {
                waited.insert(*data);
            }
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    if o.indices.is_empty() {
                        whole.insert(o.data);
                    } else {
                        indexed.insert(o.data);
                    }
                }
            }
            Event::Loop { body, .. } => collect_pre_init_sets(body, waited, whole, indexed),
            _ => {}
        }
    }
}

// --------------------------------------------------------------------
// File helpers
// --------------------------------------------------------------------

// NUC_XBACKEND_NEGATIVE seam (TASK-0178) — RELOCATED OUT of this crate.
//
// The cross-backend negative falsifier used to corrupt the emitted
// `wire.rs` here, inline on the production codegen path
// (`maybe_corrupt_wire`, deleted in TASK-0183). It was a safe seam
// (gated / deterministic / loud / anchor-guarded) but production
// codegen carrying a self-corruption branch is not a clean seam.
//
// The e2e harness is the SOLE consumer of `NUC_XBACKEND_NEGATIVE`
// (only `just xbackend-check-negative` sets it), so — exactly as
// TASK-0157/0187 did for the sibling `NUC_NONDET_TEST` seam — the
// whole branch now lives harness-side: nucleus-e2e applies the
// `wire.rs` rewrite as a post-`nucleus build`, pre-compile
// post-process of the emitted mp-tcp tree
// (`maybe_corrupt_wire_for_xbackend` in `e2e/src/main.rs`). This crate
// now does ZERO `std::env::var("NUC_XBACKEND_NEGATIVE")` and has no
// corruption branch on any codegen path; `emit()` writes `wire.rs`
// byte-identical to `mp_tcp_common::WIRE_RUNTIME_SRC`. The exact-`"1"`
// gate, loud banner, and anchor-drift hard-failure all moved verbatim
// to the harness with behaviour preserved.

fn write_file(path: &Path, contents: &str) -> Result<(), EmitError> {
    fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}

fn mark_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(path, perms);
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

// `io` is referenced via `EmitError` (re-exported from pthreads-sync)
// for the `KernelsReadFailed`/`WriteFailed` variants; the alias keeps
// the use explicit for readers.
#[allow(unused_imports)]
use io as _io;
