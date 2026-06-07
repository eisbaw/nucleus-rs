//! Per-backend codegen dispatch for `nucleus build`.
//!
//! Extracted from `main.rs` (TASK-0388) to keep the driver entry file
//! below the 1000-LoC mega-file fence (`just check-mega-files`). This
//! is the "per-backend dispatch" seam named in the driver module
//! docstring; the "arg-parse" seam lives in `args.rs`.
//!
//! [`dispatch_backend`] is the final step of `cmd_build`: it takes the
//! fully-projected per-worker EventList contract (as a
//! [`crate::gate::GatedPerWorker`] witness — the SAME inputs every
//! EventList-consuming backend receives, but only OBTAINABLE by passing
//! the post-projection gate, TASK-0440 — plus reverse name tables +
//! codegen sidecar) and routes to the elected backend's `emit(...)`,
//! printing the deterministic machine-parseable summary the e2e harness
//! parses. The `--shim` validity check (a tier-3 selector meaningful
//! only for embedded-pattern) is gated here, immediately before the
//! dispatch, because the embedded arm is its only consumer.

use std::path::Path;

use nucleus_compiler::{NameSidecar, NameTables};

/// Route the projected EventList contract to the elected backend's
/// `emit(...)` and print the success summary. The EventList arrives as a
/// [`crate::gate::GatedPerWorker`] witness, so this fn is unreachable
/// unless the post-projection gate ran and accepted it (TASK-0440).
/// `shim` is the validated `--shim` selector (tier-3 / embedded-pattern
/// only). Returns the String-channel error of any codegen failure
/// (panic-safe — the EventList path never aborts the process).
pub(crate) fn dispatch_backend(
    backend: &str,
    gated: crate::gate::GatedPerWorker<'_>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_path: &Path,
    out_dir: &Path,
    shim: Option<&str>,
) -> Result<(), String> {
    // The witness is the SOLE source of `per_worker`; the rest of the
    // function is unchanged. Holding `gated` is proof the gate ran.
    let per_worker = gated.events();
    // `--shim` is a tier-3 selector (M10, TASK-0048.01). A `--shim` on a
    // backend that has no shim concept is a user error — reject it loudly
    // rather than silently ignore it (PRD fail-fast rule). Only the
    // embedded-pattern backend consults `shim` below.
    if shim.is_some() && backend != "embedded-pattern" {
        return Err(format!(
            "--shim is only meaningful for the embedded-pattern backend \
             (a tier-3 target shim, PRD \u{00A7}10.3); backend `{backend}` does \
             not take a shim"
        ));
    }

    match backend {
        "pthreads-sync" => {
            let result =
                pthreads_sync::emit(per_worker, names, sidecar, kernels_path, out_dir)
                    .map_err(|e| format!("pthreads-sync codegen error: {e}"))?;
            // Print a deterministic, machine-parseable summary so the
            // e2e harness can pick up the run.sh path.
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Second tier-1 backend (TASK-0036/0037/0038): multi-process
        // over TCP loopback. Same contract; the only difference is
        // the transport. `run_sh` is the always-present entry point
        // (single-process AND multi-process) — the e2e harness keys
        // its multi-process run path off the backend's
        // `transport = "tcp"` capability + this run.sh.
        "mp-tcp-bufsync" => {
            let result =
                mp_tcp_bufsync::emit(per_worker, names, sidecar, kernels_path, out_dir)
                    .map_err(|e| format!("mp-tcp-bufsync codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            for (i, b) in result.worker_bins.iter().enumerate() {
                println!("worker_bin{i} = {}", b.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("wire_rs     = {}", result.wire_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Third tier-1 backend (TASK-0042.01): shared-memory + ring
        // buffer per (DataId, SeqTag). SKELETON in this cycle (16) —
        // `pthreads_async::emit` returns ContractGap until TASK-0226
        // lands the ring-buffer + Condvar codegen. The capability
        // matrix + dispatch wiring are real so that schedule authoring
        // can target this backend now; the user-facing error from this
        // arm carries the precise forward-link.
        "pthreads-async" => {
            let result =
                pthreads_async::emit(per_worker, names, sidecar, kernels_path, out_dir)
                    .map_err(|e| format!("pthreads-async codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Fourth tier-1 backend (TASK-0042.02 / TASK-0042.05): OS
        // processes + TCP loopback + mio reactor + per-(seq, peer)
        // outbound queue + per-seq inbound queue. SYNC -> ASYNC
        // upgrade of mp-tcp-bufsync (same shape pthreads-async is to
        // pthreads-sync). Stages 1+2 (skeleton + single-worker
        // delegation) landed cycle 41; Stage 3 (multi-worker mio
        // reactor + Chan<T>) landed cycle 79 — verified bit-identical
        // against reference.bin on 3 cells.
        //
        // Worker-to-worker Pushes were LIFTED in TASK-0327 cycle 149
        // via host-relay (Reactor::relay_one + Plan::render_relay_phase),
        // plus TASK-0329.01.02 cycles 163-164b for in-`Repeat`-body
        // pairs. Host-excluding barriers were lifted cycle 160 via
        // TASK-0329's `apply_host_mediation_inject` pass — marked Done.
        // The backend's ContractGap rejection in `Plan::build` (wire
        // text still cites TASK-0175 for test-pin compatibility) is
        // now defense-in-depth — should not fire for ACFGs that came
        // through the driver's pipeline.
        "mp-tcp-event" => {
            let result = mp_tcp_event::emit(per_worker, names, sidecar, kernels_path, out_dir)
                .map_err(|e| format!("mp-tcp-event codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            for (i, b) in result.worker_bins.iter().enumerate() {
                println!("worker_bin{i} = {}", b.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("wire_rs     = {}", result.wire_rs.display());
            if let Some(r) = &result.runtime_rs {
                println!("runtime_rs  = {}", r.display());
            }
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Fifth tier-1 backend (TASK-0044.01, M6): rayon threads +
        // shared memory + barrier + sync — same capability surface as
        // pthreads-sync (sync + shared-memory + barrier/blocking
        // notify), differing only in runtime substrate (rayon scope
        // instead of std::thread). Status as of cycle 197 (M6 matrix
        // complete): single-worker arm landed cycle 191 (delegates to
        // pthreads-sync's `render_single_worker_main` + backend-common's
        // project-skeleton, byte-identical to pthreads-sync /
        // pthreads-async); multi-worker arm landed cycle 196 via
        // TASK-0044.01.01 (rayon-scope codegen + 8 [[required]] cells
        // bit-identical vs pthreads-sync template).
        "openmp-rs" => {
            let result = openmp_rs::emit(per_worker, names, sidecar, kernels_path, out_dir)
                .map_err(|e| format!("openmp-rs codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Sixth tier-1 backend (TASK-0044.02, M6): OS processes + TCP
        // loopback + nonblocking poll + sync — same capability surface
        // as mp-tcp-bufsync, differing only in the wait primitive
        // (nonblocking-read poll loop instead of blocking recv). Status
        // as of cycle 197 (M6 matrix complete): single-worker arm landed
        // cycle 192 (delegates to pthreads-sync's
        // `render_single_worker_main_with_kernels_attr` +
        // backend-common's multi_binary skeleton, byte-identical to
        // mp-tcp-bufsync's single-process output); multi-worker arm
        // landed cycle 195 via TASK-0044.02.02 (nonblocking-poll
        // codegen + 8 [[required]] cells bit-identical vs mp-tcp-bufsync
        // template). Multi-binary shape (same dispatch fields as
        // mp-tcp-bufsync / mp-tcp-event).
        "mp-tcp-poll" => {
            let result = mp_tcp_poll::emit(per_worker, names, sidecar, kernels_path, out_dir)
                .map_err(|e| format!("mp-tcp-poll codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            for (i, b) in result.worker_bins.iter().enumerate() {
                println!("worker_bin{i} = {}", b.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("wire_rs     = {}", result.wire_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Seventh tier-1 backend (TASK-0044.03, M6): OS processes +
        // Unix domain sockets + mio/epoll + async + buffer — same
        // capability surface as mp-tcp-event, differing only in
        // transport (UDS instead of TCP loopback). Status as of cycle
        // 197 (M6 matrix complete): single-worker arm landed cycle 194
        // (delegates to pthreads-sync's
        // `render_single_worker_main_with_kernels_attr` +
        // backend-common's multi_binary skeleton, byte-identical to
        // mp-tcp-event's single-process output; wire.rs emitted from
        // mp_tcp_common::WIRE_RUNTIME_SRC verbatim for shape uniformity,
        // unused because the single-process bin does not `mod wire;`);
        // multi-worker arm landed cycle 197 via TASK-0044.03.01
        // (UDS-reactor codegen + 13 [[required]] cells bit-identical
        // vs mp-tcp-event template; transport-layer lift filed as
        // TASK-0044.03.02 follow-up). Multi-binary shape with optional
        // runtime_rs (same as mp-tcp-event).
        "mp-uds-event" => {
            let result = mp_uds_event::emit(per_worker, names, sidecar, kernels_path, out_dir)
                .map_err(|e| format!("mp-uds-event codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            for (i, b) in result.worker_bins.iter().enumerate() {
                println!("worker_bin{i} = {}", b.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("wire_rs     = {}", result.wire_rs.display());
            if let Some(r) = &result.runtime_rs {
                println!("runtime_rs  = {}", r.display());
            }
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // First tier-3 backend (TASK-0047, M9): the generic
        // `embedded-pattern` backend. The LIB path emits COMPILE-ONLY
        // `no_std` lib project(s) (Cargo.toml + src/lib.rs only — no
        // main.rs, no run.sh: there is nothing to RUN for a compile-only
        // lib; a Renode-runnable bin is M10's job, TASK-0048). It lowers
        // the per-worker EventList against a `NucleusShim` trait +
        // do-nothing stub shim. As of TASK-0049.04 (M11 backend slice A)
        // the LIB path is MULTI-worker: a multi-worker schedule emits ONE
        // lib per used worker (under out_dir/<worker>/) and lowers the
        // cross-worker Push/Wait/Sync to the stub-shim hooks; a
        // single-worker schedule still emits ONE project at out_dir root.
        // Acceptance: `cargo check --target thumbv7em-none-eabihf` (run
        // under `nix develop .#embedded` via `just check-embedded`). This
        // backend is NOT in the e2e-matrix.toml backends list — the
        // tier-1 runtime differential runs+diffs host binaries, which is
        // wrong for a compile-only no_std backend. The Renode MULTI-MCU
        // bin (`--shim stm32h7` multi-worker) is M11 slice B (TASK-0049.05).
        "embedded-pattern" => {
            // --shim selects the emit mode (PRD §10.3 quad's "target
            // shim", M10 TASK-0048.01):
            //   --shim stm32h7  -> Renode-runnable no_std BIN (cortex-m-rt
            //                      entry + panic handler + memory.x +
            //                      real-input load from the injected region
            //                      + raw USART1 output streaming).
            //   (no --shim)     -> the M9 compile-only no_std LIB
            //                      (UNCHANGED — `just check-embedded`).
            // An unrecognised shim name is a typed (not panicking) error.
            match shim {
                None => {
                    // TASK-0049.04: emit() now returns ONE lib project
                    // per used worker (single-worker -> one project at
                    // out_dir root; multi-worker -> one under
                    // out_dir/<worker_name>/ each). Print every project
                    // so a caller (e.g. `just check-embedded`) can locate
                    // and cross-compile each one.
                    let result = embedded_pattern::emit(
                        per_worker,
                        names,
                        sidecar,
                        kernels_path,
                        out_dir,
                    )
                    .map_err(|e| format!("embedded-pattern codegen error: {e}"))?;
                    println!("nucleus: ok");
                    println!("worker_projects = {}", result.workers.len());
                    for w in &result.workers {
                        match &w.worker_name {
                            Some(name) => println!("worker      = {name}"),
                            None => println!("worker      = (single)"),
                        }
                        println!("project_dir = {}", w.project_dir.display());
                        println!("cargo_toml  = {}", w.cargo_toml.display());
                        println!("lib_rs      = {}", w.lib_rs.display());
                    }
                    Ok(())
                }
                Some(name) => {
                    // Map the `--shim` flag to a target. TASK-0048.01 wired
                    // `stm32h7`; P10 (TASK-0453.10) adds the SECOND family
                    // `nrf52840` (single-worker BIN). An unknown name is a
                    // typed (non-panicking) user error.
                    let target = embedded_pattern::ShimTarget::from_flag(name).ok_or_else(|| {
                        format!(
                            "unknown --shim `{name}` for backend embedded-pattern; \
                             registered shims: `stm32h7` (STM32H7 Cortex-M7, M10 \
                             TASK-0048.01) and `nrf52840` (nRF52840 Cortex-M4F \
                             single-worker, P10 TASK-0453.10). Omit --shim for the \
                             M9 compile-only lib."
                        )
                    })?;
                    // TASK-0049.05: emit_bin returns ONE bin per used worker
                    // (single-worker -> one at out_dir root; multi-worker ->
                    // one under out_dir/<worker>/ each, STM32-only M11
                    // multi-MCU) plus a generated multi-machine .resc for the
                    // multi-worker case. Print every bin so a caller (e.g.
                    // `just renode-multimcu` / `just renode-embedded-nrf`) can
                    // locate and cross-compile + co-simulate each one.
                    let result = embedded_pattern::emit_bin(
                        per_worker,
                        names,
                        sidecar,
                        kernels_path,
                        out_dir,
                        target,
                    )
                    .map_err(|e| {
                        format!("embedded-pattern ({} bin) codegen error: {e}", target.flag())
                    })?;
                    println!("nucleus: ok");
                    println!("worker_bins = {}", result.workers.len());
                    for w in &result.workers {
                        match &w.worker_name {
                            Some(name) => println!("worker       = {name}"),
                            None => println!("worker       = (single)"),
                        }
                        println!("project_dir  = {}", w.project_dir.display());
                        println!("cargo_toml   = {}", w.cargo_toml.display());
                        println!("main_rs      = {}", w.main_rs.display());
                        println!("memory_x     = {}", w.memory_x.display());
                        println!("build_rs     = {}", w.build_rs.display());
                        println!("cargo_config = {}", w.cargo_config.display());
                    }
                    if let Some(resc) = &result.resc {
                        println!("resc         = {}", resc.display());
                    }
                    Ok(())
                }
            }
        }
        // FIRST tier-2 backend (TASK-0045, M7): SPMD MPI. ONE
        // rank-dispatched binary (MPI_Comm_rank), output is hosted Rust +
        // rsmpi. Single-worker SPMD arm landed cycle M7-entry (reuses the
        // shared single-worker renderer for byte-identical compute,
        // wrapped in MPI_Init/Finalize + a rank==0 guard); the
        // multi-worker arm (rank-dispatched Send/Recv + MPI_Barrier)
        // returns ContractGap forward-linking TASK-0045.01. The generated
        // project builds + runs only under `nix develop .#mpi`
        // (`just check-mpi`), NOT the tier-1 runtime differential.
        "mpi-blocking" => {
            let result =
                mpi_blocking::emit(per_worker, names, sidecar, kernels_path, out_dir)
                    .map_err(|e| format!("mpi-blocking codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            if let Some(compute_rs) = &result.compute_rs {
                // Single-worker arm only; the multi-worker arm (TASK-0045.01)
                // emits the whole rank-dispatched program in main.rs.
                println!("compute_rs  = {}", compute_rs.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // mpi-nonblocking (TASK-0046, M8). SPMD MPI like mpi-blocking but
        // Push => non-blocking BUFFERED MPI_Ibsend (local completion,
        // deadlock-immune) and Wait => MPI_Imrecv/Irecv + explicit
        // MPI_Wait. Admits the async/buffered schedules mpi-blocking
        // rejects (05-stencil/distributed{,-2d}, 11-game-of-life/pipelined,
        // 09-producer-consumer/pipelined). Builds + runs only under `nix
        // develop .#mpi` (`just
        // check-mpi-nonblocking`), NOT the tier-1 runtime differential.
        "mpi-nonblocking" => {
            let result =
                mpi_nonblocking::emit(per_worker, names, sidecar, kernels_path, out_dir)
                    .map_err(|e| format!("mpi-nonblocking codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            if let Some(compute_rs) = &result.compute_rs {
                // Single-worker arm only; the multi-worker arm emits the
                // whole rank-dispatched program in main.rs.
                println!("compute_rs  = {}", compute_rs.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        other => Err(format!(
            "unknown backend `{other}`; registered: `pthreads-sync`, \
             `mp-tcp-bufsync`, `pthreads-async`, `mp-tcp-event`, \
             `openmp-rs`, `mp-tcp-poll`, `mp-uds-event`, `embedded-pattern`, \
             `mpi-blocking`, `mpi-nonblocking`"
        )),
    }
}
