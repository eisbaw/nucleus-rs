//! CLI argument parsing for the `nucleus build` subcommand.
//!
//! Extracted from `main.rs` (TASK-0388) to keep the driver entry file
//! below the 1000-LoC mega-file fence (`just check-mega-files`). This
//! is the "arg-parse" seam named in the driver module docstring; the
//! "per-backend dispatch" seam lives in `dispatch.rs`. Pure parsing of
//! the `argv` vector into a [`BuildArgs`]; no I/O beyond `--help`.

use std::path::PathBuf;

pub(crate) fn print_help() {
    eprintln!(
        "nucleus — Nuc v2 pre-compiler\n\
         \n\
         USAGE:\n    \
             nucleus build --algo FILE --sched FILE --backend NAME \\\n    \
                           [--out DIR] [--kernels FILE] [--capabilities FILE] \\\n    \
                           [--emit-pn FILE] [--shim NAME]\n\
         \n\
         FLAGS:\n    \
             --emit-pn FILE   Write the global Petri net to FILE as Graphviz DOT.\n    \
                              Makes --out optional (inspection-only build).\n    \
             --shim NAME      Tier-3 target shim (embedded-pattern only). `stm32h7`\n    \
                              or `nrf52840` emits a Renode-runnable no_std BIN; omit\n    \
                              for the M9 compile-only no_std LIB.\n\
         \n\
         BACKENDS:\n    \
             pthreads-sync   shared-memory threads (tier 1)\n    \
             mp-tcp-bufsync  OS processes over TCP loopback (tier 1)\n    \
             pthreads-async  shared-memory + ring buffer (tier 1)\n    \
             mp-tcp-event    OS processes + TCP loopback + mio (tier 1)\n    \
             openmp-rs       rayon threads (tier 1, single-worker + multi-worker landed cycles 191/196)\n    \
             mp-tcp-poll     OS processes + TCP loopback + nonblocking poll (tier 1, single-worker + multi-worker landed cycles 192/195)\n    \
             mp-uds-event    OS processes + Unix domain sockets + mio (tier 1, single-worker + multi-worker landed cycles 194/197)\n    \
             embedded-pattern  no_std lib + NucleusShim trait (tier 3, compile-only; check via `just check-embedded`).\n    \
                               LIB path: single-worker (M9) OR multi-worker (M11 slice A, TASK-0049.04 — one lib per worker, Push/Wait/Sync -> stub-shim hooks).\n    \
                               With `--shim stm32h7`: Renode-runnable no_std bin (M10, single-worker only; `just renode-embedded <example>`; examples 1/5/9)\n    \
                               With `--shim nrf52840`: SECOND MCU family (P10, TASK-0453.10) — nRF52840 Cortex-M4F UARTE-EasyDMA bin, single-worker; `just renode-embedded-nrf <example>`; examples 1/5/9 byte-exact\n    \
             mpi-blocking    SPMD MPI (tier 2, M7); one rank-dispatched binary + rsmpi. Builds/runs under `nix develop .#mpi` via `just check-mpi`.\n    \
                               Single-worker SPMD arm landed; multi-worker (rank Send/Recv + MPI_Barrier) is TASK-0045.01.\n    \
             mpi-nonblocking SPMD MPI (tier 2, M8); non-blocking BUFFERED MPI_Ibsend + MPI_Imrecv/Irecv + MPI_Wait. Builds/runs under `nix develop .#mpi` via `just check-mpi-nonblocking`.\n    \
                               Admits the async/buffered schedules mpi-blocking rejects (05-stencil distributed + distributed-2d, 11-game-of-life pipelined, 09-producer-consumer pipelined); deadlock-immune (TASK-0046).\n"
    );
}

#[derive(Default)]
pub(crate) struct BuildArgs {
    pub(crate) algo: Option<PathBuf>,
    pub(crate) sched: Option<PathBuf>,
    pub(crate) backend: Option<String>,
    pub(crate) out: Option<PathBuf>,
    pub(crate) kernels: Option<PathBuf>,
    pub(crate) capabilities: Option<PathBuf>,
    pub(crate) emit_pn: Option<PathBuf>,
    // Target shim selector (PRD §10.3 quad: algorithm, schedule, tier-3
    // backend, target shim). M10 (TASK-0048.01): `--shim stm32h7` makes
    // the embedded-pattern backend emit a Renode-runnable no_std BIN
    // instead of the M9 compile-only LIB. No `--shim` => the M9 lib.
    pub(crate) shim: Option<String>,
}

pub(crate) fn parse_build_args(argv: &[String]) -> Result<BuildArgs, String> {
    let mut a = BuildArgs::default();
    let mut i = 0;
    while i < argv.len() {
        let cur = argv[i].as_str();
        let val = || -> Result<&String, String> {
            argv.get(i + 1)
                .ok_or_else(|| format!("flag `{cur}` requires a value"))
        };
        match cur {
            "--algo" => {
                a.algo = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--sched" => {
                a.sched = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--backend" => {
                a.backend = Some(val()?.clone());
                i += 2;
            }
            "--out" => {
                a.out = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--kernels" => {
                a.kernels = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--capabilities" => {
                a.capabilities = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--emit-pn" => {
                a.emit_pn = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--shim" => {
                a.shim = Some(val()?.clone());
                i += 2;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    Ok(a)
}
