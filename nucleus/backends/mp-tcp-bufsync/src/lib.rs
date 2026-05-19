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
//! `compiler::algo` (beyond the inert `IrExpr`/type grammar the
//! EventList carries) nor `compiler::link`/`acfg`.
//!
//! ## No renderer drift (TASK-0124 flagged risk, addressed)
//!
//! Expression / index / kernel-call / loop-bound / single-worker
//! rendering is **not re-implemented** here — it calls
//! pthreads-sync's `pub` shared shims (`render_fire_args_pub`,
//! `render_flat_index_pub`, `render_const_expr_pub`,
//! `render_single_worker_main`, `rust_type_of`, …). There is exactly
//! ONE implementation; this crate only adds the multi-PROCESS
//! transport. That is why a single-worker example is byte-identical
//! to pthreads-sync's single process, and why the multi-worker
//! arithmetic cannot silently diverge.
//!
//! ## Topology (deterministic; no sleeps-as-sync — AC#3)
//!
//! Exactly one TCP connection per `(host, worker)` ordered pair.
//! `host` is the server: it binds a listener per non-host worker on a
//! port passed in `NUC_TCP_PORT_<worker>` (run.sh allocates a free
//! port and exports it). Each non-host worker is the client and
//! `connect`s with a bounded retry loop — a refused connect only
//! means the listener is not up yet (a liveness wait, not a data
//! sync; the eventual outcome is deterministic). All Push / Wait /
//! Sync between host and a worker travel over that one stream in the
//! schedule's deterministic event order, so the `SeqTag` on each
//! framed message is a fail-loud cross-check, not routing.
//!
//! Barriers are host-mediated: every barrier in the tier-1 set is
//! `{host, w0}` (2-party); the general N-party barrier is a star
//! through host. A barrier whose participant set excludes `host`
//! cannot be lowered on the one-stream-per-pair topology — that is a
//! typed [`EmitError::ContractGap`] (honest limitation, not a wrong
//! binary; see TASK-0036 notes / filed follow-up).
//!
//! ## Inherited caveats (identical to pthreads-sync; fail-loud)
//!
//! - `Event::Sync` carries no stable cross-worker barrier id —
//!   recovered by per-worker pre-order Sync index, valid only for
//!   UNIFORM barriers, [`EmitError::ContractGap`] otherwise
//!   (TASK-0172).
//! - `block_transform` defers absolute-index rebinding to codegen;
//!   the divisible case is handled via the shared single-worker
//!   renderer, non-divisible is TASK-0173. No required mp-tcp cell
//!   exercises a blocked *multi*-worker schedule.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use compiler::event::{DataId, Event, WorkerId};
use compiler::sidecar::NameSidecar;

// Shared codegen — the SINGLE implementation of expression/index/
// call/single-worker rendering. Re-exported so callers (driver,
// tests) build `NameTables` once and feed both backends.
pub use pthreads_sync::{EmitError, NameTables};
use pthreads_sync::{
    render_array_init_for, render_const_expr_pub, render_fire_args_pub, render_flat_index_pub,
    render_single_worker_main, rust_type_of, RenderCtxPub,
};

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
        let body = render_single_worker_main(events, names, sidecar)?;
        let bin_path = bin_dir.join("nuc-generated.rs");
        write_file(&bin_path, &wrap_single_worker(&body))?;
        write_file(&cargo_toml, &render_cargo_toml(&[String::from("nuc-generated")]))?;
        write_file(&run_sh, &render_run_sh_single())?;
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
    // Reproducible port picker (std-only Rust, no python3 / external
    // runtime — built by the same cargo build, works under
    // `nix develop`). run.sh calls it once per non-host worker.
    let pick_bin = "__nuc_pick_port".to_string();
    write_file(&bin_dir.join(format!("{pick_bin}.rs")), PICK_PORT_SRC)?;
    bin_names.push(pick_bin);

    write_file(&cargo_toml, &render_cargo_toml(&bin_names))?;
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

/// A std-only port picker: bind 127.0.0.1:0, print the
/// kernel-assigned ephemeral port, exit. Reproducible (no python3 /
/// external tool); built by the same cargo build as the workers.
const PICK_PORT_SRC: &str = "\
// Generated by the mp-tcp-bufsync backend (TASK-0038). Reproducible
// free-port picker — std only, no external runtime dependency.
fn main() {
    let l = std::net::TcpListener::bind(\"127.0.0.1:0\")
        .unwrap_or_else(|e| panic!(\"__nuc_pick_port: bind failed: {e}\"));
    println!(\"{}\", l.local_addr().unwrap().port());
}
";

fn render_cargo_toml(bin_names: &[String]) -> String {
    let mut s = String::from(
        "# Generated by the mp-tcp-bufsync backend. Do not edit; rerun \
         `nucleus build` to regenerate.\n\
         [package]\n\
         name        = \"nuc-generated\"\n\
         version     = \"0.0.0\"\n\
         edition     = \"2021\"\n\
         publish     = false\n\
         \n\
         [workspace]\n\
         # Empty: standalone, not part of any parent workspace.\n\
         \n",
    );
    for b in bin_names {
        // Each worker is its own binary target. `src/bin/<name>.rs`
        // is the conventional auto-discovered location but we declare
        // it explicitly so the manifest is the single source of truth
        // for which binaries exist (deterministic, greppable).
        writeln!(s, "[[bin]]").ok();
        writeln!(s, "name = \"{b}\"").ok();
        writeln!(s, "path = \"src/bin/{b}.rs\"").ok();
        writeln!(s).ok();
    }
    s.push_str("[profile.release]\npanic = \"abort\"\n");
    s
}

/// Single-process run.sh: build + run the one binary. Mirrors the
/// pthreads-sync run.sh contract (INPUT_BIN OUTPUT_BIN positional).
fn render_run_sh_single() -> String {
    String::from(
        "#!/usr/bin/env bash\n\
         # Generated by the mp-tcp-bufsync backend (single-process: \
         0/1 used workers). Rerun `nucleus build` to regenerate.\n\
         # Usage: bash run.sh INPUT_BIN OUTPUT_BIN\n\
         set -euo pipefail\n\
         \n\
         here=\"$(cd -- \"$(dirname -- \"${BASH_SOURCE[0]}\")\" && pwd)\"\n\
         input_bin=\"${1:-input.bin}\"\n\
         output_bin=\"${2:-output.bin}\"\n\
         \n\
         (cd \"$here\" && cargo build --release --quiet)\n\
         \n\
         NUC_INPUT_PATH=\"$input_bin\" \\\n\
         NUC_OUTPUT_PATH=\"$output_bin\" \\\n\
         \"$here/target/release/nuc-generated\"\n",
    )
}

/// Wrap a shared single-worker `main.rs` body. The shared renderer
/// emits `mod kernels;` + `fn main() {...}`; for the
/// `src/bin/<name>.rs` layout we redirect `mod kernels` at the copied
/// `../kernels.rs` via `#[path]`.
fn wrap_single_worker(shared_body: &str) -> String {
    // The shared body starts with doc comments then `mod kernels;`.
    // Replace the bare `mod kernels;` with a `#[path]` form so the
    // binary in `src/bin/` finds the sibling `src/kernels.rs`.
    shared_body.replacen(
        "mod kernels;",
        "#[path = \"../kernels.rs\"]\n#[allow(dead_code)]\nmod kernels;",
        1,
    )
}

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

        // Host: the worker literally named "host", else the smallest
        // used WorkerId — IDENTICAL choice to pthreads-sync's
        // multi_worker::Plan::build (the differential needs the same
        // host election under both backends).
        let host_named = names
            .worker
            .iter()
            .find(|(_, n)| n.as_str() == "host")
            .map(|(w, _)| *w)
            .filter(|w| used_workers.contains(w));
        let host_worker = host_named
            .or_else(|| used_workers.first().copied())
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

        // Barrier identity by per-worker pre-order Sync index, with
        // the SAME uniform-barrier validation pthreads-sync performs
        // (TASK-0172). A non-uniform barrier is a typed ContractGap,
        // never a silently mismatched barrier graph.
        let mut barrier_participants: BTreeMap<usize, BTreeSet<WorkerId>> = BTreeMap::new();
        for w in &used_workers {
            let mut idx = 0usize;
            collect_barriers_preorder(&per_worker[w], &mut idx, &mut |bid, parts| {
                match barrier_participants.get(&bid) {
                    None => {
                        barrier_participants.insert(bid, parts.clone());
                        Ok(())
                    }
                    Some(existing) if existing == parts => Ok(()),
                    Some(existing) => Err(EmitError::ContractGap(format!(
                        "barrier #{bid} has inconsistent participant sets across \
                         workers ({existing:?} vs {parts:?}); Event::Sync carries \
                         no stable cross-worker identity and the pre-order-index \
                         recovery only holds for uniform barriers (TASK-0172). \
                         mp-tcp-bufsync will not byte-identically lower a \
                         partial-barrier schedule."
                    ))),
                }
            })?;
        }

        // Topology constraint: one stream per (host, worker) pair, so
        // every barrier must include host as the mediating hub. True
        // for the tier-1 set (02-split: {host,w0}). Fail loud
        // otherwise — an honest limitation, not a wrong binary.
        for (bid, parts) in &barrier_participants {
            if !parts.contains(&host_worker) {
                return Err(EmitError::ContractGap(format!(
                    "barrier #{bid} participants {parts:?} exclude the host \
                     worker; mp-tcp-bufsync's one-connection-per-(host,worker) \
                     topology requires host as the barrier hub. A \
                     host-excluding barrier needs a worker-to-worker mesh \
                     (filed as TASK-0175)."
                )));
            }
        }

        Ok(Plan {
            per_worker,
            names,
            sidecar,
            used_workers,
            host_worker,
            xfer_ids,
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
            if is_host { " [host/server]" } else { " [client]" }
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
        // warning-clean (host = server: only TcpListener; non-host =
        // client: TcpStream + Duration for the connect-retry).
        if is_host {
            writeln!(out, "use std::net::TcpListener;").ok();
        } else {
            writeln!(out, "use std::net::TcpStream;").ok();
            writeln!(out, "use std::time::Duration;").ok();
        }
        writeln!(out).ok();

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
            for nw in self.non_host_workers() {
                let nwn = self.worker_name(nw);
                writeln!(
                    out,
                    "    let port_{nwn}: u16 = std::env::var(\"NUC_TCP_PORT_{nwn}\")\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.expect(\"host: NUC_TCP_PORT_{nwn} not set (run.sh must export it)\")\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.parse().expect(\"host: NUC_TCP_PORT_{nwn} not a u16\");"
                )
                .ok();
                writeln!(
                    out,
                    "    let listener_{nwn} = TcpListener::bind((\"127.0.0.1\", port_{nwn}))\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20.unwrap_or_else(|e| panic!(\"host: bind 127.0.0.1:{{port_{nwn}}} failed: {{e}}\"));"
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
            writeln!(
                out,
                "    let port: u16 = std::env::var(\"NUC_TCP_PORT_{wn}\")\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.expect(\"{wn}: NUC_TCP_PORT_{wn} not set (run.sh must export it)\")\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20.parse().expect(\"{wn}: NUC_TCP_PORT_{wn} not a u16\");"
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
            writeln!(out, "    let mut data_host = connect_retry(port, \"DATA\");").ok();
            writeln!(out, "    let mut ctrl_host = connect_retry(port, \"CTRL\");").ok();
            writeln!(out, "    data_host.set_nodelay(true).ok();").ok();
            writeln!(out, "    ctrl_host.set_nodelay(true).ok();").ok();
            writeln!(out, "    wire::apply_sock_buf(&data_host);").ok();
            writeln!(out, "    wire::apply_sock_buf(&ctrl_host);").ok();
        }
        writeln!(out).ok();

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
        self.render_events(
            &self.per_worker[&worker],
            &mut out,
            1,
            worker,
            is_host,
            &ctx,
            &mut 0,
        )?;

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

    #[allow(clippy::too_many_arguments)]
    fn render_events(
        &self,
        events: &[Event],
        out: &mut String,
        indent: usize,
        worker: WorkerId,
        is_host: bool,
        ctx: &RenderCtxPub<'_>,
        sync_idx: &mut usize,
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
                            writeln!(
                                out,
                                "{pad}let mut {name} = kernels::{callee}({args});"
                            )
                            .ok();
                        }
                        Some(o) => {
                            let name = self.data_name(o.data)?;
                            let idx = render_flat_index_pub(o, ctx)?;
                            writeln!(
                                out,
                                "{pad}{name}[{idx}] = kernels::{callee}({args});"
                            )
                            .ok();
                        }
                    }
                }
                Event::Loop {
                    iter_var,
                    range,
                    body,
                    block_tag,
                } => {
                    let var = self.names.iter_var.get(iter_var).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "iter var {iter_var:?} in Event::Loop has no name in NameTables"
                        ))
                    })?;
                    // A strip-mine `block_tag` in this MULTI-process
                    // renderer means a blocked multi-worker schedule.
                    // Per-occurrence absolute-index rebinding (TASK-0180)
                    // is implemented only on the shared single-worker
                    // path (which a 0/1-worker schedule — incl. all
                    // tier-1 blocked schedules 04/05/06/07 — already
                    // routes through). No tier-1 schedule blocks a
                    // multi-worker loop; fail LOUD rather than silently
                    // emit the un-rebound loop (accumulator
                    // double-count, the exact TASK-0180 defect class).
                    if block_tag.is_some() {
                        return Err(EmitError::ContractGap(format!(
                            "Event::Loop for iter var `{var}` carries a strip-mine \
                             block_tag inside a MULTI-process schedule; per-occurrence \
                             rebinding is single-worker-path only (TASK-0180). No \
                             tier-1 schedule blocks a multi-worker loop; refusing to \
                             emit un-rebound. Tracked as TASK-0181."
                        )));
                    }
                    // SHARED bound renderer (source-form via sidecar,
                    // else concrete folded range) — identical to
                    // pthreads-sync multi-worker.
                    let (lo, hi) = match self.sidecar.loop_bounds.get(iter_var) {
                        Some(b) => (
                            render_const_expr_pub(&b.lo, ctx)?,
                            render_const_expr_pub(&b.hi, ctx)?,
                        ),
                        None => (
                            format!("{}_i64", range.start),
                            format!("{}_i64", range.end),
                        ),
                    };
                    writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                    self.render_events(body, out, indent + 1, worker, is_host, ctx, sync_idx)?;
                    writeln!(out, "{pad}}}").ok();
                }
                Event::Sync { participants, .. } => {
                    let bid = *sync_idx;
                    *sync_idx += 1;
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
                            writeln!(
                                out,
                                "{pad}wire::barrier_cross(&mut {cv}, {bid});"
                            )
                            .ok();
                        }
                    } else {
                        writeln!(
                            out,
                            "{pad}wire::barrier_cross(&mut ctrl_host, {bid});"
                        )
                        .ok();
                    }
                }
                Event::Push {
                    data, dst, seq, ..
                } => {
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
                Event::Wait {
                    data, src, seq, ..
                } => {
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
                    writeln!(
                        out,
                        "{pad}{{ let __buf = wire::read_msg_expect(&mut {cv}, {}); \
                         {name} = {dec}; }} // recv `{name}` from {from}",
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
    /// `peer`. On the star topology a non-host worker only ever talks
    /// to host; host talks to the specific peer. A non-host worker
    /// whose Push/Wait peer is not host cannot be lowered here
    /// (fail-loud, not mis-routed).
    fn data_conn_var(
        &self,
        worker: WorkerId,
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
            if peer != self.host_worker {
                return Err(EmitError::ContractGap(format!(
                    "worker {:?} Push/Wait peer {peer:?} is not the host; \
                     mp-tcp-bufsync's one-(data,ctrl)-pair-per-(host,worker) \
                     topology has no worker-to-worker channel (filed as \
                     TASK-0175). Not silently mis-routed.",
                    worker
                )));
            }
            Ok("data_host".to_string())
        }
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

    /// run.sh (TASK-0038): launch one process per used worker, wire
    /// ports + socket buffer sizes via env, wait, non-zero on any
    /// worker failure naming the worker.
    fn render_run_sh(&self) -> Result<String, EmitError> {
        let mut s = String::new();
        writeln!(s, "#!/usr/bin/env bash").ok();
        writeln!(
            s,
            "# Generated by the mp-tcp-bufsync backend (TASK-0038, multi-process)."
        )
        .ok();
        writeln!(s, "# Usage: bash run.sh INPUT_BIN OUTPUT_BIN").ok();
        writeln!(s, "set -uo pipefail").ok();
        writeln!(s).ok();
        writeln!(
            s,
            "here=\"$(cd -- \"$(dirname -- \"${{BASH_SOURCE[0]}}\")\" && pwd)\""
        )
        .ok();
        writeln!(s, "input_bin=\"${{1:-input.bin}}\"").ok();
        writeln!(s, "output_bin=\"${{2:-output.bin}}\"").ok();
        writeln!(s).ok();
        writeln!(s, "(cd \"$here\" && cargo build --release --quiet)").ok();
        writeln!(s).ok();
        // Port allocation (AC#3): a tiny Rust helper binary
        // (`__nuc_pick_port`, emitted in src/bin/) binds 127.0.0.1:0,
        // lets the OS assign a free ephemeral port, prints it, exits.
        // Rust-std-only — NO python3 / external runtime (reproducible:
        // it is built by the SAME `cargo build` as the workers, so it
        // works under `nix develop` with nothing extra on PATH). No
        // fixed ports ⇒ no clashes across concurrent matrix cells.
        // There is an unavoidable but vanishing TOCTOU window between
        // "helper closes the socket" and "host binds it"; on loopback
        // with ephemeral ports under a single test run this does not
        // bite, and a bind clash fails LOUD (host panics naming the
        // port) rather than silently mis-connecting.
        writeln!(s, "pick_port() {{ \"$here/target/release/__nuc_pick_port\"; }}").ok();
        writeln!(s).ok();
        // Socket buffer sizing (TASK-0038 AC#2): derived from the
        // schedule's per-channel buffer requirement. mp-tcp-bufsync
        // is sync (buffer=1) so the requirement is one message; we
        // size SO_*BUF from the largest cross-worker payload (sum of
        // element bytes) with a 64 KiB floor. Passed via env to each
        // worker, which calls setsockopt (capabilities.toml contract).
        // v2 uses the single highest requirement (AC#7 limitation: no
        // per-channel granularity if an OS-level cap binds).
        let bufsz = self.max_payload_bytes()?.max(65536);
        writeln!(
            s,
            "# Socket buffer requirement from the schedule's per-channel\n\
             # buffer needs (largest single transfer payload, sync=1 msg).\n\
             export NUC_SO_BUF={bufsz}"
        )
        .ok();
        writeln!(s).ok();

        let non_host = self.non_host_workers();
        for nw in &non_host {
            let nwn = self.worker_name(*nw);
            writeln!(s, "PORT_{nwn}=\"$(pick_port)\"").ok();
            writeln!(s, "export NUC_TCP_PORT_{nwn}=\"$PORT_{nwn}\"").ok();
        }
        writeln!(s).ok();

        let host_name = self.worker_name(self.host_worker);
        // Host first so its listener is binding while workers spin up
        // (workers retry-connect anyway; this just minimises retries).
        writeln!(
            s,
            "NUC_INPUT_PATH=\"$input_bin\" NUC_OUTPUT_PATH=\"$output_bin\" \\\n\
             \x20\x20\"$here/target/release/{host_name}\" &\n\
             PID_{host_name}=$!"
        )
        .ok();
        for nw in &non_host {
            let nwn = self.worker_name(*nw);
            writeln!(
                s,
                "NUC_INPUT_PATH=\"$input_bin\" NUC_OUTPUT_PATH=\"$output_bin\" \\\n\
                 \x20\x20\"$here/target/release/{nwn}\" &\n\
                 PID_{nwn}=$!"
            )
            .ok();
        }
        writeln!(s).ok();

        // Wait for every worker; non-zero (naming the worker) if any
        // fails (AC#3). `wait <pid>` returns that child's status.
        writeln!(s, "rc=0").ok();
        let mut all = vec![host_name.clone()];
        all.extend(non_host.iter().map(|w| self.worker_name(*w)));
        for n in &all {
            writeln!(
                s,
                "if ! wait \"$PID_{n}\"; then echo \"run.sh: worker '{n}' failed (exit $?)\" >&2; rc=1; fi"
            )
            .ok();
        }
        writeln!(s, "exit \"$rc\"").ok();
        Ok(s)
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

fn scalar_width(t: &compiler::algo::ScalarType) -> usize {
    use compiler::algo::ScalarType::*;
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
fn encode_expr(name: &str, ty: &compiler::algo::ResolvedType) -> Result<String, EmitError> {
    use compiler::algo::ScalarType::Bool;
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        Ok(format!("wire::enc_{s}({name})"))
    } else if ty.scalar == Bool {
        // `bool` has no `to_le_bytes`; dedicated 1-byte-per-element.
        Ok(format!("wire::enc_vec_bool(&{name})"))
    } else {
        let rs = pthreads_sync::rust_scalar_type_pub(&ty.scalar);
        Ok(format!("wire::enc_vec(&{name}, {rs}::to_le_bytes)"))
    }
}

/// Expression that decodes `__buf` back into the value's Rust type.
fn decode_expr(ty: &compiler::algo::ResolvedType) -> Result<String, EmitError> {
    use compiler::algo::ScalarType::Bool;
    let s = scalar_fn_suffix(&ty.scalar);
    if ty.is_scalar() {
        Ok(format!("wire::dec_{s}(&__buf)"))
    } else if ty.scalar == Bool {
        Ok("wire::dec_vec_bool(&__buf)".to_string())
    } else {
        let rs = pthreads_sync::rust_scalar_type_pub(&ty.scalar);
        Ok(format!("wire::dec_vec(&__buf, {rs}::from_le_bytes)"))
    }
}

fn scalar_fn_suffix(t: &compiler::algo::ScalarType) -> &'static str {
    use compiler::algo::ScalarType::*;
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

fn collect_barriers_preorder<F>(
    events: &[Event],
    idx: &mut usize,
    f: &mut F,
) -> Result<(), EmitError>
where
    F: FnMut(usize, &BTreeSet<WorkerId>) -> Result<(), EmitError>,
{
    for e in events {
        match e {
            Event::Sync { participants, .. } => {
                let bid = *idx;
                *idx += 1;
                f(bid, participants)?;
            }
            Event::Loop { body, .. } => collect_barriers_preorder(body, idx, f)?,
            _ => {}
        }
    }
    Ok(())
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
