//! `embedded-pattern` backend — the GENERIC tier-3 embedded codegen
//! (PRD §7.3 / §11 M9+M10, TASK-0047 / TASK-0048.01).
//!
//! Two emit modes, sharing the SAME event lowering:
//!
//! - [`emit`] (M9, the default, no `--shim`): a SELF-CONTAINED `no_std`
//!   **library** crate that lowers the per-worker [`Event`] list against
//!   a `NucleusShim` trait. The acceptance is COMPILE-ONLY: the
//!   generated lib must pass `cargo check --target thumbv7em-none-eabihf`
//!   against a do-nothing STUB shim. Run that check via `just
//!   check-embedded` under `nix develop .#embedded`.
//! - [`emit_bin`] (M10, `--shim stm32h7`, TASK-0048.01 / TASK-0048.02): a
//!   Renode-runnable `no_std` **binary** (cortex-m-rt `#[entry]` +
//!   `#[panic_handler]` + STM32H743 `memory.x` + a concrete `Usart1Shim`
//!   whose `alloc_in_region` reads the Renode-injected input region and
//!   whose `dma_push` streams the RAW computed output bytes over USART1).
//!   Runtime acceptance: `just renode-embedded <example>` (the example
//!   dir is a positional arg, default 01-elementwise-add; runs the
//!   GENERATED firmware in Renode on REAL injected input, captures
//!   USART1, and `cmp`s the captured bytes BYTE-EXACT against the
//!   example's `reference.bin` — PRD §10.3 point 3 value-correctness).
//!   Covers the PRD §11 M10 set: examples 1, 5, 9 (TASK-0048.03;
//!   EX defaults to 01-elementwise-add). The two modes share kernel
//!   extraction + the `run<S>` body via `lower_kernels_and_run`; only the
//!   surrounding scaffolding differs.
//!   The real STM32H7 DMA/IRQ shim remains parent TASK-0048 AC#1 work
//!   (the current `Usart1Shim` reads a memory-mapped injected region
//!   synchronously rather than driving a real async DMA controller).
//!
//! # The std-bound-kernel problem and its resolution
//!
//! The tier-1 `kernels.rs` files are heavily std-bound: the I/O kernels
//! (`load_input` / `save_output` / `load_image` / `save_image`) use
//! `std::fs`, `std::env`, and `Vec`, none of which exist under `no_std`.
//! So this backend MUST NOT copy `kernels.rs` wholesale (that is exactly
//! what every tier-1 backend does — `fs::write(&kernels_rs, &kernels_src)`
//! — and it would break the cross-compile).
//!
//! The split is **structural**, read off the EventList (NOT a purity
//! lookup — the Event/sidecar contract deliberately drops purity, see
//! `nucleus_compiler::sidecar::KernelSig`'s DIVERGENCE HAZARD note):
//!
//! - A kernel called by an **indexed-output** `Fire` (`c[i] <-- add(..)`
//!   inside a [`Event::Loop`]) is a PURE compute kernel. Its body is
//!   no_std-clean by inspection (the tier-1 pure kernels use only
//!   `wrapping_add` / integer `/`, both inherent — no `use` needed).
//!   This backend extracts that fn definition VERBATIM from the source
//!   `kernels.rs` (PRD §6.2.2: Nucleus does not interpolate kernel
//!   bodies — we copy the user's text unchanged) into a `mod kernels`
//!   inside the emitted lib.
//! - A kernel called by an **effectful** `Fire` — a top-level
//!   output-less `Fire` (`save_output(c)`) or a top-level whole-array
//!   `Fire` with no data-reading inputs (`a <-- load_input()`) — is an
//!   I/O kernel. It maps to a `NucleusShim` hook (an embedded "input"
//!   is a DMA/sensor fill of a region; an "output" is a DMA push to a
//!   peripheral). The effectful kernel body is NOT extracted; the stub
//!   shim's hook is a no-op.
//!
//! # Data layout: fixed-size arrays, alloc-free
//!
//! Each data symbol becomes a fixed-size `[T; N]` local (`N` =
//! product of the sidecar `ResolvedType.dims`). That is alloc-free and
//! no_std-clean — no `Vec`, no global allocator. 2D data (example 5
//! `i32[16][16]`) flattens to `[i32; 256]` row-major, and the
//! `img[y][x]` index flattens to `img[y*W + x]` exactly as the tier-1
//! backends do (same shared `render_flat_index`; the no_std lowering
//! calls `render_fire_args_nostd`, the fixed-array `[T; N]` sibling of
//! the tier-1 `render_fire_args`, so array-typed pure-kernel sub-array
//! args materialise alloc-free via `try_into` instead of `to_vec` —
//! TASK-0049.06).
//!
//! # Scope: both LIB and BIN paths are multi-worker (M11)
//!
//! The lowering handles single-`host` naive schedules generally, and —
//! since TASK-0049.04 (M11 backend slice A) — the LIB path
//! ([`emit`]) ALSO handles multi-worker schedules: one `no_std` lib
//! project is emitted PER used worker (`out_dir/<worker_name>/`), and
//! the cross-worker `Push` / `Wait` / `Sync` events lower to the
//! `skeleton::NucleusShim` transport hooks (`link_push` / `link_recv` /
//! `irq_barrier`). A single-worker schedule still emits ONE project at
//! `out_dir` root (unchanged observable output — `check-embedded`
//! examples 1 + 5 are byte-stable). The M9 compile-only acceptance set
//! (`check-embedded`) is examples 1 and 5; the M10 bin runtime set
//! (`renode-embedded`) is examples 1, 5, and 9 (TASK-0048.03).
//!
//! The BIN path ([`emit_bin`]) is ALSO multi-worker since TASK-0049.05
//! (M11 backend slice B): a single-worker schedule emits ONE bin at
//! `out_dir` root with the `Usart1Shim` (UNCHANGED M10 shape), while a
//! multi-worker schedule emits one bin per worker under
//! `out_dir/<worker>/` with the CONCRETE `MultiMcuShim` (real inter-MCU
//! UART-hub transport: `link_push` -> USART TX, `link_recv` -> blocking
//! USART RX), PLUS a generated multi-machine `out_dir/multimcu.resc`
//! wiring the bins on a `UARTHub` per CHANNEL (`SeqTag`) — one dedicated
//! byte FIFO per transport channel (TASK-0049.05.02; see [`multimcu`]). The
//! per-worker effectful IO (`save_output` raw USART1 stream,
//! `load_input` from the injected axiSram region) stays on its OWN
//! channel namespace (`dma_push(0)` / `alloc_in_region`), DISJOINT from
//! the cross-worker `link_*` transport, so a real shim routes peripheral
//! IO and inter-MCU transport to different USARTs (TASK-0049.05 trap #1).
//! `Alloc` / `Free` (explicit region management) do not occur in the
//! schedules this backend admits and are still rejected with a forward
//! link.
//!
//! ## What the LIB-path multi-worker lowering is, and is NOT
//!
//! It is COMPILE-ONLY against the do-nothing `skeleton::StubShim`:
//! `dma_push` / `dma_wait` / `irq_barrier` are all no-ops, so a
//! [`Event::Wait`]'s receive local stays zero-filled (the stub does not
//! actually receive). That is honest for this slice: AC#3 is a REAL
//! cross-compile (`cargo check --target thumbv7em-none-eabihf`), NOT
//! value-correctness or a Renode run. The transport channel ids come
//! straight from the events (`SeqTag` -> dma channel, `SyncTag` -> irq
//! barrier tag) — there is no channel allocator.
//!
//! # Honest limitations (AC#6)
//!
//! - **No DMA, no IRQ, no real timing.** The `StubShim` hooks are
//!   no-ops; in particular `alloc_in_region` returns null, so the
//!   null-guarded input-fill copy is skipped and the arrays stay
//!   zero-filled — the generated lib compiles but computes on
//!   zero-filled input arrays. This is
//!   compile-only validation, exactly the M9 bar (PRD §10.3 point 2 /
//!   §11 M9). Real DMA/IRQ + a Renode-runnable bin (panic_handler +
//!   entry point + linker script + `.resc`) is M10 (TASK-0048).
//! - **`irq_barrier` is UNEXERCISED by the single-worker examples**
//!   (1, 5, 9): they have no [`Event::Sync`] barrier. It IS exercised
//!   on the LIB path by a multi-worker schedule that carries an
//!   [`Event::Sync`] (TASK-0049.04): `Sync` -> `shim.irq_barrier(tag)`.
//!   The `StubShim` no-ops it.
//! - **`alloc_in_region` / `dma_push` / `dma_wait` ARE exercised** by
//!   the effectful-kernel hooks, but against the stub they are no-ops.
//!
//! # Why this crate is a NORMAL std crate
//!
//! The backend runs on the HOST (part of the nucleus workspace, built
//! by `just build`). Only the GENERATED lib is `no_std`. This crate
//! writes strings.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

use nucleus_compiler::event::{Event, FireBinding, KernelId, ViolationKind, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

mod kernel_extract;
mod multimcu;
mod render;
mod skeleton;

use render::render_run_body;

#[cfg(test)]
mod tests;

/// Paths to the files emitted for ONE worker's `no_std` LIB project.
/// Unlike the tier-1 backends this is a `no_std` LIB project:
/// `Cargo.toml` + `src/lib.rs` only (no `main.rs`, no `run.sh` — there
/// is nothing to run for a compile-only lib; a runnable Renode bin is
/// M10's job, TASK-0048).
///
/// [`emit`] returns a [`MultiEmitResult`] carrying one of these per
/// USED worker (one for a single-worker schedule, N for a multi-worker
/// schedule — TASK-0049.04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The worker this project was emitted for. `None` for the
    /// single-worker case (emitted at `out_dir` root, no worker
    /// sub-directory — keeps the M9 single-worker output unchanged);
    /// `Some(name)` for a multi-worker schedule (emitted under
    /// `out_dir/<name>/`).
    pub worker_name: Option<String>,
    /// The generated Cargo project root (`out_dir` for the single
    /// worker, `out_dir/<worker_name>` for a multi-worker schedule).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Path to the emitted `src/lib.rs` (the whole no_std lib).
    pub lib_rs: PathBuf,
}

/// The full result of [`emit`]: one [`EmitResult`] per USED worker
/// (TASK-0049.04, M11 backend slice A). A single-worker schedule yields
/// exactly one (emitted at `out_dir` root); a multi-worker schedule
/// yields one per used worker, each under `out_dir/<worker_name>/`.
/// Declared-but-unused workers (empty event list) are skipped, matching
/// the tier-1 `used_workers` convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiEmitResult {
    /// One project per used worker, in deterministic worker-id order.
    /// Never empty for a schedule with at least one firing.
    pub workers: Vec<EmitResult>,
}

/// Paths to the files [`emit_bin`] writes (M10, TASK-0048.01). Unlike
/// [`EmitResult`] (the compile-only LIB), this is a Renode-runnable
/// `no_std` BIN project: a self-contained `src/main.rs` (cortex-m-rt
/// entry + panic handler + USART1 streaming) plus the bare-metal
/// scaffolding (`Cargo.toml` with `[[bin]]`, `memory.x`, `build.rs`,
/// `.cargo/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinEmitResult {
    /// The worker this bin was emitted for. `None` for a single-worker
    /// schedule (emitted at `out_dir` root, unchanged M10 shape);
    /// `Some(name)` for a multi-worker schedule (one bin per worker under
    /// `out_dir/<name>/`, M11 TASK-0049.05).
    pub worker_name: Option<String>,
    /// The generated Cargo project root (`out_dir` for the single worker,
    /// `out_dir/<worker_name>` for a multi-worker schedule).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml` (with `[[bin]]` + cortex-m-rt).
    pub cargo_toml: PathBuf,
    /// Path to the emitted `src/main.rs` (the whole no_std firmware).
    pub main_rs: PathBuf,
    /// Path to the emitted `memory.x` linker fragment.
    pub memory_x: PathBuf,
    /// Path to the emitted `build.rs`.
    pub build_rs: PathBuf,
    /// Path to the emitted `.cargo/config.toml`.
    pub cargo_config: PathBuf,
}

/// The full result of [`emit_bin`]: one [`BinEmitResult`] per USED worker,
/// plus the generated multi-machine Renode `.resc` for a multi-worker
/// schedule (TASK-0049.05, M11 BIN slice B). A single-worker schedule
/// yields exactly one bin (at `out_dir` root) and `resc: None` (the
/// single-machine `tests/renode/embedded/run.resc` already covers it); a
/// multi-worker schedule yields one bin per worker under
/// `out_dir/<worker>/` plus `out_dir/multimcu.resc` wiring them and an
/// ordered `out_dir/output_captures.txt` capture manifest (one saver
/// `file_var` per line, in `TransportPlan.output_captures` decl-order —
/// the recipe reads it for both var-injection and concat order,
/// TASK-0049.10.08). The manifest path is NOT carried on this struct
/// (the recipe finds it by fixed name beside the `.resc`); `resc: Some`
/// is the multi-worker discriminant for both files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiBinEmitResult {
    /// One bin per used worker, in deterministic worker-id order.
    pub workers: Vec<BinEmitResult>,
    /// Path to the generated multi-machine `.resc` (multi-worker only).
    pub resc: Option<PathBuf>,
}

/// Emit one `no_std` lib project per USED worker from the per-worker
/// EventList (TASK-0049.04, M11 backend slice A).
///
/// Wire contract: consumes the per-worker [`Event`] lists + the
/// [`NameTables`] (reverse `name_*`) + the [`NameSidecar`] — the SAME
/// contract every tier-1 backend consumes (PRD §8.3). **No `&ACFG` /
/// `&LinkedIR` access.** `kernels_rs_path` is read to extract the PURE
/// kernel bodies verbatim (see module docs); `out_dir` is the generated
/// project root.
///
/// LAYOUT (TASK-0049.04 N-projects decision): a SINGLE-worker schedule
/// emits ONE project at `out_dir` root — the M9 single-worker output is
/// observationally UNCHANGED (`check-embedded` examples 1 + 5 stay
/// byte-stable, and `just renode-embedded` continues to find the project
/// at `out_dir`). A MULTI-worker schedule emits one project per used
/// worker under `out_dir/<worker_name>/`. Declared-but-unused workers
/// (empty event list) are skipped — `acfg_to_events` seeds every
/// declared worker with an empty list, so the same `used_workers` filter
/// every tier-1 backend uses applies here.
///
/// The cross-worker `Push` / `Wait` / `Sync` events in each worker's list
/// lower to the stub `skeleton::NucleusShim` hooks (see
/// `render::render_event`); they are compile-only no-ops against the
/// `skeleton::StubShim` (AC#3 is a real cross-compile, not a Renode
/// run — see module docs).
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<MultiEmitResult, EmitError> {
    // Same `used_workers` filter as every tier-1 backend: a
    // declared-but-unused worker carries an empty event list and is
    // skipped (no project emitted for it).
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();

    // Read the source kernels ONCE; every per-worker project extracts
    // the PURE kernel bodies it uses from the same source text.
    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    let multi_worker = used_workers.len() > 1;
    let mut results = Vec::with_capacity(used_workers.len().max(1));

    if used_workers.is_empty() {
        // No firing at all (e.g. an --emit-pn-style empty projection):
        // emit a single empty-body project at the root, preserving the
        // historical single-worker behaviour for a degenerate schedule.
        results.push(emit_one_worker_lib(
            &[],
            names,
            sidecar,
            &kernels_src,
            out_dir,
            None,
        )?);
        return Ok(MultiEmitResult { workers: results });
    }

    for w in &used_workers {
        let events: &[Event] = per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]);
        // Single worker -> root; multi-worker -> out_dir/<worker_name>/.
        // The worker NAME comes from NameTables (reverse WorkerId->name),
        // the same source every other backend uses for worker identity.
        let (project_dir, worker_name) = if multi_worker {
            let name = names.worker.get(w).cloned().ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "worker id {w:?} has an event list but no name in NameTables; \
                     the embedded backend names per-worker project directories \
                     from NameTables (TASK-0049.04)"
                ))
            })?;
            (out_dir.join(&name), Some(name))
        } else {
            (out_dir.to_path_buf(), None)
        };
        results.push(emit_one_worker_lib(
            events,
            names,
            sidecar,
            &kernels_src,
            &project_dir,
            worker_name,
        )?);
    }

    Ok(MultiEmitResult { workers: results })
}

/// Emit ONE worker's `no_std` lib project (`Cargo.toml` + `src/lib.rs`)
/// into `project_dir`. Shared by the single- and multi-worker arms of
/// [`emit`] so the per-worker project shape is a single source of truth.
fn emit_one_worker_lib(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_src: &str,
    project_dir: &Path,
    worker_name: Option<String>,
) -> Result<EmitResult, EmitError> {
    // The `on_violation=count` check loops drive module-scope AtomicU32
    // statics emitted into the lib (TASK-0048.08). The lib path has no
    // `main` (compile-only, StubShim), so the statics are never flushed —
    // but they MUST be in scope so the lib still cross-compiles when the
    // schedule carries a count check frame.
    let count_loops = collect_count_check_loops(events);

    let lib_src = render_lib_rs(events, names, sidecar, kernels_src, &count_loops)?;

    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let cargo_toml = project_dir.join("Cargo.toml");
    let lib_rs = src_dir.join("lib.rs");

    write_file(&cargo_toml, &skeleton::render_cargo_toml())?;
    write_file(&lib_rs, &lib_src)?;

    Ok(EmitResult {
        worker_name,
        project_dir: project_dir.to_path_buf(),
        cargo_toml,
        lib_rs,
    })
}

/// Emit one Renode-runnable `no_std` BIN project per used worker from the
/// per-worker EventList (M10 single-worker, TASK-0048.01; M11 multi-MCU,
/// TASK-0049.05).
///
/// ADDITIVE: this is a SEPARATE entry point from [`emit`]. The driver
/// dispatches here when `--shim stm32h7` is passed; the bare `--backend
/// embedded-pattern` (no `--shim`) still goes through [`emit`] (the
/// unchanged M9 compile-only lib path). The two share the SAME lowering
/// (`render_run_body` + verbatim kernel extraction); the bin adds the
/// bare-metal scaffolding (cortex-m-rt entry, panic handler, linker
/// script, USART shim).
///
/// Wire contract is IDENTICAL to [`emit`] (no `&ACFG` / `&LinkedIR`).
/// The no-block / no-check-frame rejections are reused via
/// `render_run_body`, so an unsupported schedule fails loud with the same
/// typed [`EmitError`] here as on the lib path.
///
/// SINGLE-WORKER SCOPE (TASK-0048.01/.02/.03): the PRD §11 M10 naive set —
/// example 1 (01-elementwise-add), example 5 (05-stencil, 2D blur3),
/// example 9 (09-producer-consumer). One bin at `out_dir` root with the
/// `Usart1Shim`: loads REAL input from the Renode-injected region (axiSram
/// @ 0x2400_0000), computes, streams the RAW output bytes over USART1; the
/// `renode-embedded` recipe `cmp`s the captured bytes BYTE-EXACT against
/// `reference.bin` (PRD §10.3 point 3). See `skeleton::USART1_SHIM_SRC`.
///
/// MULTI-WORKER SCOPE (TASK-0049.05): one bin per worker under
/// `out_dir/<worker>/` with the CONCRETE `MultiMcuShim` (`link_push` ->
/// USART TX, `link_recv` -> blocking USART RX), wired by the
/// [`multimcu::TransportPlan`] (one `UARTHub` per CHANNEL/`SeqTag`,
/// staged-release boot order — see `multimcu::compute_boot_order`), PLUS a
/// generated `out_dir/multimcu.resc` AND an
/// ordered `out_dir/output_captures.txt` capture manifest. The
/// `renode-multimcu` recipe co-simulates the bins, captures each saver
/// worker's USART1 to its own file backend, concatenates those files in
/// [`multimcu::TransportPlan::output_captures`] order (the manifest order)
/// and `cmp`s the concatenation against `reference.bin` (TASK-0049.10.08,
/// BLOCKER 3 slice D). Value-correctness is proven for the 02-split-add
/// 2-MCU schedule; ex14 stays gated on TASK-0049.02 (stateful per-frame
/// kernels — slice D wires the transport/capture, not the per-frame
/// compute VALUES).
pub fn emit_bin(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<MultiBinEmitResult, EmitError> {
    // The BIN path is now MULTI-worker (TASK-0049.05 lifted the M10
    // single-worker guard). A single-worker schedule emits ONE bin at
    // `out_dir` root with the `Usart1Shim` (UNCHANGED M10 shape — examples
    // 1/5/9 byte-stable); a multi-worker schedule emits one bin per worker
    // under `out_dir/<worker>/` with the concrete `MultiMcuShim` (real
    // inter-MCU UART-hub transport) PLUS a generated multi-machine
    // `out_dir/multimcu.resc` wiring them. The N-projects layout MIRRORS
    // [`emit`] (the LIB path); the per-worker / `.resc` wiring is the
    // [`multimcu::TransportPlan`].
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

    let multi_worker = used_workers.len() > 1;
    // Fail loud (TASK-0049.05.01) if a control-only `Event::Sync` would be
    // silently miscompiled by the no-op `irq_barrier` — i.e. it orders two
    // workers' external IO with no subsuming data edge. Inert for every
    // shipped schedule (02-split-add routes all IO through `host`); a real
    // tripwire for a future standalone-barrier schedule. Runs before any
    // emit so the rejection precedes side-effecting file writes.
    if multi_worker {
        multimcu::verify_control_sync_subsumed(per_worker, names, sidecar)?;
    }
    // Build the transport plan up front for the multi-worker case; it is
    // the SINGLE source of truth shared by both the per-worker shim codegen
    // and the generated `.resc` (so the wiring cannot drift).
    let plan = if multi_worker {
        Some(multimcu::TransportPlan::build(per_worker, names, sidecar)?)
    } else {
        None
    };

    let mut results = Vec::with_capacity(used_workers.len().max(1));

    if used_workers.is_empty() {
        // Degenerate (no firing): emit one empty-body single-worker bin at
        // the root, preserving the historical single-worker behaviour.
        results.push(emit_one_worker_bin(
            &[], names, sidecar, &kernels_src, out_dir, None, None,
        )?);
        return Ok(MultiBinEmitResult {
            workers: results,
            resc: None,
        });
    }

    for w in &used_workers {
        let events: &[Event] = per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]);
        let (project_dir, worker_name) = if multi_worker {
            let name = names.worker.get(w).cloned().ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "worker id {w:?} has an event list but no name in NameTables; \
                     the embedded multi-MCU bin names per-worker project \
                     directories from NameTables (TASK-0049.05)"
                ))
            })?;
            (out_dir.join(&name), Some(name))
        } else {
            (out_dir.to_path_buf(), None)
        };
        let wplan = plan.as_ref().map(|p| {
            p.workers
                .get(w)
                .expect("TransportPlan covers every used worker")
        });
        results.push(emit_one_worker_bin(
            events,
            names,
            sidecar,
            &kernels_src,
            &project_dir,
            worker_name,
            wplan,
        )?);
    }

    // Multi-worker: emit the multi-machine Renode `.resc` wiring the bins,
    // PLUS the ordered capture manifest `out_dir/output_captures.txt`.
    let resc = if let Some(p) = &plan {
        let resc_src = multimcu::render_multimachine_resc(p);
        let resc_path = out_dir.join("multimcu.resc");
        write_file(&resc_path, &resc_src)?;

        // Ordered capture manifest (TASK-0049.10.08, BLOCKER 3 slice D).
        // ONE `file_var` per line, in `TransportPlan.output_captures` order
        // (i.e. `NameSidecar::data_decl_order` — NOT WorkerId order). This
        // file is the SINGLE source of truth the `renode-multimcu` recipe
        // reads for BOTH which `$<file_var>` vars to inject AND the order in
        // which to concatenate the per-saver capture files before the
        // byte-exact `reference.bin` diff. The recipe MUST NOT grep the
        // `.resc` for ordering: the `.resc` emits its `CreateFileBackend`
        // lines per machine in WorkerId order, which need NOT match decl
        // order (ex14: `bt_out`=DataId(1) < `spk_out`=DataId(4), but decl
        // order is spk_out-before-bt_out). For ex14 this manifest is
        // `feUart\nrfUart`; for the single-saver 02-split-add it is the lone
        // line `uartFile`.
        let manifest_src: String = p
            .output_captures
            .iter()
            .map(|c| c.file_var.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // Trailing newline so the file is a clean N-line list (a final
        // empty line on read is tolerated by the recipe's line-skip).
        let manifest_src = format!("{manifest_src}\n");
        write_file(&out_dir.join("output_captures.txt"), &manifest_src)?;

        Some(resc_path)
    } else {
        None
    };

    Ok(MultiBinEmitResult {
        workers: results,
        resc,
    })
}

/// Emit ONE worker's Renode-runnable `no_std` BIN project into
/// `project_dir`. Shared by the single- and multi-worker arms of
/// [`emit_bin`]. `wplan` is `None` for a single-worker schedule (uses the
/// M10 `Usart1Shim` via [`render_main_rs`]) and `Some` for a multi-worker
/// schedule (uses the concrete `MultiMcuShim` with this worker's transport
/// plan, via [`render_multimcu_main_rs`]). The bare-metal scaffolding
/// (`Cargo.toml` / `memory.x` / `build.rs` / `.cargo/config.toml`) is
/// IDENTICAL for both — only the `main.rs` shim differs.
#[allow(clippy::too_many_arguments)]
fn emit_one_worker_bin(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_src: &str,
    project_dir: &Path,
    worker_name: Option<String>,
    wplan: Option<&multimcu::WorkerPlan>,
) -> Result<BinEmitResult, EmitError> {
    let count_loops = collect_count_check_loops(events);
    let main_src = match wplan {
        Some(plan) => {
            render_multimcu_main_rs(events, names, sidecar, kernels_src, &count_loops, plan)?
        }
        None => render_main_rs(events, names, sidecar, kernels_src, &count_loops)?,
    };

    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;
    let cargo_dir = project_dir.join(".cargo");
    fs::create_dir_all(&cargo_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: cargo_dir.clone(),
        source: e,
    })?;

    let cargo_toml = project_dir.join("Cargo.toml");
    let main_rs = src_dir.join("main.rs");
    let memory_x = project_dir.join("memory.x");
    let build_rs = project_dir.join("build.rs");
    let cargo_config = cargo_dir.join("config.toml");

    write_file(&cargo_toml, &skeleton::render_bin_cargo_toml())?;
    write_file(&main_rs, &main_src)?;
    write_file(&memory_x, &skeleton::render_memory_x())?;
    write_file(&build_rs, &skeleton::render_build_rs())?;
    write_file(&cargo_config, &skeleton::render_cargo_config())?;

    Ok(BinEmitResult {
        worker_name,
        project_dir: project_dir.to_path_buf(),
        cargo_toml,
        main_rs,
        memory_x,
        build_rs,
        cargo_config,
    })
}

/// Render the complete `no_std` lib source: header + `NucleusShim`
/// trait + `StubShim` + `mod kernels` (pure bodies) + `run<S>`.
fn render_lib_rs(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_src: &str,
    count_loops: &[CountCheckLoop],
) -> Result<String, EmitError> {
    let (kernel_defs, run_body) = lower_kernels_and_run(events, names, sidecar, kernels_src)?;
    let count_idents: Vec<&str> = count_loops.iter().map(|c| c.ident.as_str()).collect();
    Ok(skeleton::render_lib(&kernel_defs, &run_body, &count_idents))
}

/// Render the complete Renode-runnable `no_std` BIN source: cortex-m-rt
/// header + `NucleusShim` trait + `Usart1Shim` + `mod kernels` (pure
/// bodies) + `run<S>` + `#[entry] main` (M10, TASK-0048.01).
///
/// Shares the SAME kernel extraction + `run<S>` body lowering as
/// [`render_lib_rs`] (via [`lower_kernels_and_run`]) — only the
/// surrounding scaffolding differs (the bin adds the entry point /
/// panic handler / concrete UART shim). This shared lowering is why a
/// schedule the lib path rejects (multi-worker / block / check-frame)
/// is rejected IDENTICALLY on the bin path.
fn render_main_rs(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_src: &str,
    count_loops: &[CountCheckLoop],
) -> Result<String, EmitError> {
    let (kernel_defs, run_body) = lower_kernels_and_run(events, names, sidecar, kernels_src)?;
    let summaries: Vec<skeleton::CountSummary> = count_loops
        .iter()
        .map(|c| skeleton::CountSummary {
            ident: c.ident.clone(),
            loop_var: c.loop_var.clone(),
            latency_max_ns: c.latency_max_ns,
        })
        .collect();
    Ok(skeleton::render_bin_main(
        &kernel_defs,
        &run_body,
        &summaries,
    ))
}

/// Render ONE worker's multi-MCU Renode-runnable `no_std` BIN source
/// (TASK-0049.05). Same shared kernel extraction + `run<S>` body lowering
/// as [`render_main_rs`] (via [`lower_kernels_and_run`]), but assembled
/// around the concrete `MultiMcuShim` (real inter-MCU UART transport,
/// wired per `wplan`) instead of the single-worker `Usart1Shim`.
fn render_multimcu_main_rs(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_src: &str,
    count_loops: &[CountCheckLoop],
    wplan: &multimcu::WorkerPlan,
) -> Result<String, EmitError> {
    let (kernel_defs, run_body) = lower_kernels_and_run(events, names, sidecar, kernels_src)?;
    let summaries: Vec<skeleton::CountSummary> = count_loops
        .iter()
        .map(|c| skeleton::CountSummary {
            ident: c.ident.clone(),
            loop_var: c.loop_var.clone(),
            latency_max_ns: c.latency_max_ns,
        })
        .collect();
    Ok(skeleton::render_multimcu_bin_main(
        wplan,
        &kernel_defs,
        &run_body,
        &summaries,
    ))
}

/// The shared core of [`render_lib_rs`] and [`render_main_rs`]: extract
/// the PURE kernel bodies verbatim and render the `run<S>` body. Single
/// source of truth so the lib and bin lower IDENTICALLY (the only
/// difference between the two emit modes is the surrounding scaffolding,
/// not the event lowering).
fn lower_kernels_and_run(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_src: &str,
) -> Result<(String, String), EmitError> {
    // 1. Classify which kernels are PURE (called by an indexed-output
    //    Fire). These bodies are extracted verbatim into `mod kernels`.
    let pure_kernels = collect_pure_kernel_names(events, names)?;

    // 2. Extract the pure kernel definitions verbatim from kernels.rs.
    let mut kernel_defs = String::new();
    for kname in &pure_kernels {
        let def = kernel_extract::extract_pub_fn(kernels_src, kname).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "pure kernel `{kname}` is called by an indexed-output Fire \
                 but no `pub fn {kname}` was found in kernels.rs — the \
                 embedded backend extracts pure kernel bodies verbatim and \
                 cannot synthesise one"
            ))
        })?;
        if !kernel_defs.is_empty() {
            kernel_defs.push('\n');
        }
        kernel_defs.push_str(&def);
        kernel_defs.push('\n');
    }

    // 3. Render the `run<S>` body (data decls + event lowering).
    let run_body = render_run_body(events, names, sidecar)?;

    Ok((kernel_defs, run_body))
}

/// One `on_violation=count` check-loop, materialised for the tier-3
/// embedded count sink (TASK-0048.08, PART 1).
///
/// Deliberately a backend-LOCAL type rather than reusing
/// [`backend_common::check_frame::CountCheckLoop`]: the tier-1 type drives
/// an `AtomicU64` + `Drop`-guard summary that DOES NOT port to bare-metal
/// (a) `AtomicU64` is unavailable on `thumbv7em-none-eabihf` — only 32-bit
/// `LDREX`/`STREX` atomics, so the static must be `AtomicU32`; (b) a
/// firmware spins forever in `loop {}`, so a Rust `Drop` at `main` return
/// never fires (docs/check-loop-latency-max.md §3). The shared
/// [`backend_common::check_frame::sanitize_loop_var`] IS reused for the
/// ident (no drift on the sanitisation rule).
#[derive(Debug, Clone, PartialEq, Eq)]
struct CountCheckLoop {
    /// Sanitised identifier — appears in the static name
    /// `NUC_CHECK_COUNT_<ident>`.
    ident: String,
    /// Original loop-variable name, carried verbatim into the bin-path
    /// summary line so the user sees the directive they wrote.
    loop_var: String,
    /// Threshold in nanoseconds (post unit-normalisation, same as
    /// [`nucleus_compiler::event::CheckFrame::latency_max_ns`]).
    latency_max_ns: u64,
}

/// Walk `events` recursively; collect every `on_violation=count` check
/// frame in EventList order (deterministic across builds — the same
/// guarantee the rest of codegen relies on). Mirrors
/// [`backend_common::check_frame::collect_count_check_frames`] in SHAPE,
/// but yields the backend-local [`CountCheckLoop`] (AtomicU32, not the
/// tier-1 AtomicU64 + Drop summary).
fn collect_count_check_loops(events: &[Event]) -> Vec<CountCheckLoop> {
    let mut out = Vec::new();
    fn walk(events: &[Event], out: &mut Vec<CountCheckLoop>) {
        for e in events {
            if let Event::Loop {
                body, check_frame, ..
            } = e
            {
                if let Some(frame) = check_frame {
                    if matches!(frame.on_violation, ViolationKind::Count) {
                        out.push(CountCheckLoop {
                            ident: backend_common::check_frame::sanitize_loop_var(&frame.loop_var),
                            loop_var: frame.loop_var.clone(),
                            latency_max_ns: frame.latency_max_ns,
                        });
                    }
                }
                walk(body, out);
            }
        }
    }
    walk(events, &mut out);
    out
}

/// Walk the events and collect the names of kernels invoked by an
/// **indexed-output** `Fire` (the pure compute kernels). Returns a
/// sorted, deduped list (deterministic emit order).
fn collect_pure_kernel_names(
    events: &[Event],
    names: &NameTables,
) -> Result<Vec<String>, EmitError> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    collect_pure_kernel_names_in(events, names, &mut out)?;
    Ok(out.into_iter().collect())
}

fn collect_pure_kernel_names_in(
    events: &[Event],
    names: &NameTables,
    out: &mut BTreeSet<String>,
) -> Result<(), EmitError> {
    for ev in events {
        match ev {
            Event::Fire {
                kernel, bindings, ..
            } => {
                if is_pure_compute_fire(bindings) {
                    out.insert(kernel_name(*kernel, names)?);
                }
            }
            Event::Loop { body, .. } => collect_pure_kernel_names_in(body, names, out)?,
            _ => {}
        }
    }
    Ok(())
}

/// A `Fire` is a PURE compute firing iff its output is an INDEXED
/// data slice (`c[i] <-- k(..)`). Top-level loads (`a <-- load()`,
/// whole-array output) and saves (`save(c)`, no output) are effectful
/// and lower to shim hooks instead.
fn is_pure_compute_fire(bindings: &FireBinding) -> bool {
    matches!(&bindings.output, Some(o) if !o.indices.is_empty())
}

/// Resolve a `KernelId` to its source name. Used by
/// `collect_pure_kernel_names_in` here and by the sibling [`render`]
/// module's `render_fire`. Crate-root-private suffices: `render` is a
/// descendant module, so a private `fn` is reachable without any
/// visibility widening (TASK-0340.10 split — verified).
fn kernel_name(kid: KernelId, names: &NameTables) -> Result<String, EmitError> {
    names.kernel.get(&kid).cloned().ok_or_else(|| {
        EmitError::ContractGap(format!("kernel id {kid:?} has no name in NameTables"))
    })
}

/// Write `content` to `path`, mapping io errors to
/// [`EmitError::WriteFailed`] with the offending path attached.
fn write_file(path: &Path, content: &str) -> Result<(), EmitError> {
    fs::write(path, content).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
