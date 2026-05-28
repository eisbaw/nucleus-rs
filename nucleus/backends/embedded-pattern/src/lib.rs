//! `embedded-pattern` backend — the GENERIC tier-3 embedded codegen
//! (PRD §7.3 / §11 M9+M10, TASK-0047 / TASK-0048.01).
//!
//! Two emit modes, sharing the SAME event lowering:
//!
//! - [`emit`] (M9, the default, no `--shim`): a SELF-CONTAINED `no_std`
//!   **library** crate that lowers the per-worker [`Event`] list against
//!   a [`NucleusShim`] trait. The acceptance is COMPILE-ONLY: the
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
//!   I/O kernel. It maps to a [`NucleusShim`] hook (an embedded "input"
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
//! backends do (same shared `render_fire_args` / `render_flat_index`).
//!
//! # Scope: single-worker
//!
//! The lowering handles single-`host` naive schedules generally. The
//! M9 compile-only acceptance set (`check-embedded`) is examples 1 and
//! 5; the M10 bin runtime set (`renode-embedded`) is examples 1, 5, and
//! 9 (TASK-0048.03). Multi-worker embedded (workers on different MCUs
//! over SPI / Ethernet) is M11 (TASK-0049) and is REJECTED here with a
//! forward link. `Push` / `Wait` / `Sync` (cross-worker) and `Alloc` / `Free`
//! (explicit region management) do not occur in the naive single-worker
//! event lists and are likewise rejected with forward links — they
//! arrive with the M10 shim (TASK-0048) / M11.
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
//! - **`irq_barrier` is defined but UNEXERCISED** by the naive
//!   single-worker examples (1, 5, 9): they have no [`Event::Sync`]
//!   barrier.
//!   The method is declared on the trait for M10/M11 (where partitioned
//!   multi-MCU schedules emit barriers); declaring it now fixes the
//!   trait shape early so the M10 shim implements a stable surface.
//! - **`alloc_in_region` / `dma_push` / `dma_wait` ARE exercised** by
//!   the effectful-kernel hooks, but against the stub they are no-ops.
//!
//! # Why this crate is a NORMAL std crate
//!
//! The backend runs on the HOST (part of the nucleus workspace, built
//! by `just build`). Only the GENERATED lib is `no_std`. This crate
//! writes strings.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub use backend_common::EmitError;
pub use nucleus_compiler::NameTables;

use backend_common::render::{data_name, render_fire_args, render_loop_bounds, RenderCtx};
use nucleus_compiler::event::{DataId, Event, FireBinding, KernelId, ViolationKind, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

mod kernel_extract;
mod skeleton;

#[cfg(test)]
mod tests;

/// Paths to the files [`emit`] writes. Unlike the tier-1 backends this
/// is a `no_std` LIB project: `Cargo.toml` + `src/lib.rs` only (no
/// `main.rs`, no `run.sh` — there is nothing to run for a compile-only
/// lib; a runnable Renode bin is M10's job, TASK-0048).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Path to the emitted `src/lib.rs` (the whole no_std lib).
    pub lib_rs: PathBuf,
}

/// Paths to the files [`emit_bin`] writes (M10, TASK-0048.01). Unlike
/// [`EmitResult`] (the compile-only LIB), this is a Renode-runnable
/// `no_std` BIN project: a self-contained `src/main.rs` (cortex-m-rt
/// entry + panic handler + USART1 streaming) plus the bare-metal
/// scaffolding (`Cargo.toml` with `[[bin]]`, `memory.x`, `build.rs`,
/// `.cargo/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinEmitResult {
    /// The generated Cargo project root (== input `out_dir`).
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

/// Emit a `no_std` lib project from the per-worker EventList.
///
/// Wire contract: consumes the per-worker [`Event`] lists + the
/// [`NameTables`] (reverse `name_*`) + the [`NameSidecar`] — the SAME
/// contract every tier-1 backend consumes (PRD §8.3). **No `&ACFG` /
/// `&LinkedIR` access.** `kernels_rs_path` is read to extract the PURE
/// kernel bodies verbatim (see module docs); `out_dir` is the generated
/// project root.
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    // M9 is single-worker. `acfg_to_events` seeds every declared worker
    // with an empty list, so a declared-but-unused worker must not trip
    // the multi-worker rejection — same `used_workers` filter as every
    // tier-1 backend.
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    if used_workers.len() > 1 {
        return Err(EmitError::UnsupportedFeature(format!(
            "embedded-pattern (M9) is single-worker compile-only; this \
             schedule uses {} workers. Multi-MCU embedded codegen (workers \
             on co-simulated MCUs over SPI / Ethernet) is M11 — TASK-0049.",
            used_workers.len()
        )));
    }

    let events: &[Event] = used_workers
        .first()
        .and_then(|w| per_worker.get(w))
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    let lib_src = render_lib_rs(events, names, sidecar, &kernels_src)?;

    let src_dir = out_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let lib_rs = src_dir.join("lib.rs");

    write_file(&cargo_toml, &skeleton::render_cargo_toml())?;
    write_file(&lib_rs, &lib_src)?;

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        lib_rs,
    })
}

/// Emit a Renode-runnable `no_std` BIN project from the per-worker
/// EventList (M10, TASK-0048.01 — the lib->bin transition).
///
/// ADDITIVE: this is a SEPARATE entry point from [`emit`]. The driver
/// dispatches here when `--shim stm32h7` is passed; the bare `--backend
/// embedded-pattern` (no `--shim`) still goes through [`emit`] (the
/// unchanged M9 compile-only lib path). The two share the SAME lowering
/// ([`render_run_body`] + verbatim kernel extraction); the bin adds the
/// bare-metal scaffolding (cortex-m-rt entry, panic handler, linker
/// script, USART1 streaming shim).
///
/// Wire contract is IDENTICAL to [`emit`] (no `&ACFG` / `&LinkedIR`).
/// The single-worker / no-block / no-check-frame rejections are reused
/// via [`render_run_body`], so an unsupported schedule fails loud with
/// the same typed [`EmitError`] here as on the lib path.
///
/// SCOPE (TASK-0048.01/.02/.03): the PRD §11 M10 single-worker naive
/// set — example 1 (01-elementwise-add), example 5 (05-stencil, 2D
/// blur3), example 9 (09-producer-consumer, two-stage produce/transform
/// pipe). The firmware loads REAL input from the Renode-injected region
/// (axiSram @ 0x2400_0000), computes the example's kernels, and streams
/// the RAW output bytes over USART1; the `renode-embedded` recipe
/// (parameterised over the example dir as a positional arg) `cmp`s the
/// captured bytes BYTE-EXACT against the example's `reference.bin` (PRD §10.3 point 3
/// value-correctness). Nothing here is example-specific — the lowering
/// reads the EventList — so generalising across the set was recipe
/// parameterisation only. See [`skeleton::USART1_SHIM_SRC`] for the
/// input-region / streaming mechanism.
pub fn emit_bin(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<BinEmitResult, EmitError> {
    // Reuse the EXACT single-worker projection + rejection of `emit`.
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    if used_workers.len() > 1 {
        return Err(EmitError::UnsupportedFeature(format!(
            "embedded-pattern (M10 bin) is single-worker; this schedule \
             uses {} workers. Multi-MCU embedded codegen (workers on \
             co-simulated MCUs over SPI / Ethernet) is M11 — TASK-0049.",
            used_workers.len()
        )));
    }

    let events: &[Event] = used_workers
        .first()
        .and_then(|w| per_worker.get(w))
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    let main_src = render_main_rs(events, names, sidecar, &kernels_src)?;

    let src_dir = out_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;
    let cargo_dir = out_dir.join(".cargo");
    fs::create_dir_all(&cargo_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: cargo_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let main_rs = src_dir.join("main.rs");
    let memory_x = out_dir.join("memory.x");
    let build_rs = out_dir.join("build.rs");
    let cargo_config = cargo_dir.join("config.toml");

    write_file(&cargo_toml, &skeleton::render_bin_cargo_toml())?;
    write_file(&main_rs, &main_src)?;
    write_file(&memory_x, &skeleton::render_memory_x())?;
    write_file(&build_rs, &skeleton::render_build_rs())?;
    write_file(&cargo_config, &skeleton::render_cargo_config())?;

    Ok(BinEmitResult {
        project_dir: out_dir.to_path_buf(),
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
) -> Result<String, EmitError> {
    let (kernel_defs, run_body) = lower_kernels_and_run(events, names, sidecar, kernels_src)?;
    Ok(skeleton::render_lib(&kernel_defs, &run_body))
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
) -> Result<String, EmitError> {
    let (kernel_defs, run_body) = lower_kernels_and_run(events, names, sidecar, kernels_src)?;
    Ok(skeleton::render_bin_main(&kernel_defs, &run_body))
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

fn kernel_name(kid: KernelId, names: &NameTables) -> Result<String, EmitError> {
    names.kernel.get(&kid).cloned().ok_or_else(|| {
        EmitError::ContractGap(format!("kernel id {kid:?} has no name in NameTables"))
    })
}

// --------------------------------------------------------------------
// `run<S>` body rendering
// --------------------------------------------------------------------

/// Render the body of `pub fn run<S: NucleusShim>(shim: &mut S)`:
/// fixed-array data declarations followed by the lowered event list.
fn render_run_body(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    let mut out = String::new();

    // Data declarations: every data symbol touched (read or written) by
    // the event list becomes a fixed `[T; N]` local. Sorted by name for
    // deterministic emit. We allocate ALL declared data referenced in
    // the events — both inputs (filled by shim hooks) and outputs
    // (written by compute loops).
    let decls = collect_data_decls(events, names, sidecar)?;
    for (name, decl_rhs) in &decls {
        writeln!(out, "    let mut {name}: {decl_rhs};").ok();
    }
    if !decls.is_empty() {
        writeln!(out).ok();
    }

    // The RenderCtx the shared Fire/index/loop-bound renderers consume.
    // `abs_subst` stays empty (no block-transform in the naive single-
    // worker schedules); the shared renderers degrade to the bare-name
    // behaviour, byte-equivalent to the tier-1 single-worker emit modulo
    // the array-vs-Vec backing store (irrelevant to index syntax).
    let ctx = RenderCtx::new(names, sidecar);
    render_events(events, &mut out, 1, &ctx)?;

    Ok(out)
}

/// Collect `(name, "[T; N]" + " = [zero; N]")` declarations for every
/// data symbol referenced by the events. Deterministic (BTreeMap → name
/// order).
fn collect_data_decls(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<Vec<(String, String)>, EmitError> {
    let mut data_ids: BTreeSet<DataId> = BTreeSet::new();
    collect_referenced_data(events, &mut data_ids);

    let mut out: Vec<(String, String)> = Vec::new();
    for did in &data_ids {
        let name = names.data.get(did).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {did:?} has no name in NameTables"))
        })?;
        let ty = sidecar.data_types.get(did).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "data `{name}` ({did:?}) has no ResolvedType in the sidecar; \
                 the embedded backend sizes fixed arrays from `dims`"
            ))
        })?;
        if ty.is_scalar() {
            // A scalar datum (dims == []) → a single mutable binding.
            let scalar = backend_common::render::rust_scalar_type(&ty.scalar);
            let zero = backend_common::render::rust_scalar_zero(&ty.scalar);
            out.push((name, format!("{scalar} = {zero}")));
        } else {
            let total: usize = ty.dims.iter().copied().product();
            let scalar = backend_common::render::rust_scalar_type(&ty.scalar);
            let zero = backend_common::render::rust_scalar_zero(&ty.scalar);
            out.push((name, format!("[{scalar}; {total}] = [{zero}; {total}]")));
        }
    }
    Ok(out)
}

/// Gather every `DataId` referenced (input or output) by any `Fire` in
/// the event tree.
fn collect_referenced_data(events: &[Event], out: &mut BTreeSet<DataId>) {
    for ev in events {
        match ev {
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    out.insert(o.data);
                }
                for inp in &bindings.inputs {
                    collect_arg_data(inp, out);
                }
            }
            Event::Loop { body, .. } => collect_referenced_data(body, out),
            _ => {}
        }
    }
}

fn collect_arg_data(arg: &nucleus_compiler::event::ArgBinding, out: &mut BTreeSet<DataId>) {
    use nucleus_compiler::event::ArgBinding;
    match arg {
        ArgBinding::Data(slice) => {
            out.insert(slice.data);
        }
        ArgBinding::Scalar(_) => {}
        ArgBinding::Nested { args, .. } => {
            for a in args {
                collect_arg_data(a, out);
            }
        }
    }
}

/// Lower a slice of events into the `run<S>` body at `indent`.
fn render_events(
    events: &[Event],
    out: &mut String,
    indent: usize,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    for ev in events {
        render_event(ev, out, indent, ctx)?;
    }
    Ok(())
}

fn render_event(
    event: &Event,
    out: &mut String,
    indent: usize,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    let pad = "    ".repeat(indent);
    match event {
        Event::Fire {
            kernel, bindings, ..
        } => render_fire(*kernel, bindings, out, &pad, ctx),
        Event::Loop {
            iter_var,
            range,
            body,
            block_tag,
            check_frame,
        } => {
            // M9 scope: the naive single-worker schedules carry no
            // block-transform (no `block=`) and no real-time check
            // frames. Reject them with a forward link rather than
            // silently mis-lowering — the strip-mine absolute-index
            // rebinding + Drop-guard check reporters are tier-1 std
            // machinery not yet ported to the embedded lowering.
            if block_tag.is_some() {
                return Err(EmitError::UnsupportedFeature(
                    "embedded-pattern (M9) does not lower strip-mined \
                     (`block=`) loops; naive schedules carry none. Blocked \
                     embedded loops are future work (M10+ TASK-0048)."
                        .to_string(),
                ));
            }
            let var = ctx.names.iter_var.get(iter_var).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "iter var {iter_var:?} in an Event::Loop has no name in NameTables"
                ))
            })?;
            // Real-time `check loop V : latency_max=T` frame (TASK-0048.04).
            // Shared by the M9 lib (StubShim) and M10 bin (Usart1Shim) paths
            // — both consume only the trait methods `shim.monotonic_ns()`
            // (PRD §6.3.5 tier-3 backend-specified clock) and
            // `shim.report_violation()` (the `on_violation=log` sink), so
            // the lowering is identical on both. The tier-1 std Drop-guard
            // reporter (`std::time::Instant` + `AtomicU64`) is NOT ported:
            // neither exists on no_std Cortex-M (and `AtomicU64` is absent on
            // thumbv7em — only 32-bit atomics). We render against the trait
            // instead, keeping Cortex-M register details inside the shim.
            if let Some(frame) = check_frame {
                // CheckFrame::loop_var (carried by value) MUST name the same
                // identifier as NameTables.iter_var[iter_var] — see the
                // CheckFrame docstring (TASK-0221). Dev builds catch
                // projection-layer divergence loudly; release builds skip it.
                debug_assert_eq!(
                    var.as_str(),
                    frame.loop_var.as_str(),
                    "check-frame loop_var diverges from the Event::Loop iter_var name"
                );
                match frame.on_violation {
                    // tier-3 on_violation policy (AC#2, PRD §6.3.5):
                    // panic BRICKS the MCU — reject loudly rather than
                    // silently remap to log (a banned silent semantic
                    // change). The default ViolationKind is Panic
                    // (materialised at sched-lower), so an embedded check
                    // loop MUST explicitly pick log.
                    ViolationKind::Panic => {
                        return Err(EmitError::UnsupportedFeature(format!(
                            "embedded-pattern rejects `check loop {lv} : \
                             on_violation=panic` on tier-3: a panic on a \
                             bare-metal MCU bricks the device (PRD §6.3.5). \
                             `on_violation` defaults to `panic`, so add an \
                             explicit `on_violation = log` to the `check loop \
                             {lv}` directive. (`count` is a filed follow-up — \
                             TASK-0048.08 — pending a bare-metal summary sink; \
                             the tier-1 Drop-guard summary does not fire on an \
                             MCU that spins forever.)",
                            lv = frame.loop_var,
                        )));
                    }
                    // count needs an end-of-run summary sink; on a bare-metal
                    // firmware that spins in `loop {}` forever, Rust Drops at
                    // `main` return never fire (docs/check-loop-latency-max.md
                    // §3). Reject loudly (AC#4) and direct to `log`; wiring a
                    // real tier-3 count sink is TASK-0048.08.
                    ViolationKind::Count => {
                        return Err(EmitError::UnsupportedFeature(format!(
                            "embedded-pattern does not yet lower `check loop \
                             {lv} : on_violation=count` on tier-3: the tier-1 \
                             Drop-guard summary fires at `main` return, but a \
                             bare-metal firmware spins forever (no Drop). A \
                             tier-3 count sink (per-N UART summary / in-flash \
                             counter dumped on watchdog reset) is TASK-0048.08. \
                             Use `on_violation = log` for a per-violation UART \
                             line.",
                            lv = frame.loop_var,
                        )));
                    }
                    // log: fully lowered. Per-iteration wall-clock via the
                    // tier-3 monotonic clock; on violation, one UART line.
                    ViolationKind::Log => {
                        let (lo_s, hi_s) = render_loop_bounds(*iter_var, range, ctx)?;
                        let body_pad = "    ".repeat(indent + 1);
                        writeln!(out, "{pad}for {var} in ({lo_s})..({hi_s}) {{").ok();
                        writeln!(out, "{body_pad}let _check_start = shim.monotonic_ns();").ok();
                        render_events(body, out, indent + 1, ctx)?;
                        writeln!(
                            out,
                            "{body_pad}let _check_elapsed = \
                             shim.monotonic_ns().wrapping_sub(_check_start);"
                        )
                        .ok();
                        // The loop_var bytes are emitted as a byte-string
                        // literal so the no_std `report_violation` can write
                        // them over UART without `core::fmt`. Sanitisation:
                        // the iter-var name is a parsed identifier
                        // ([A-Za-z0-9_]), so it is already a valid byte-string
                        // literal body; no escaping required.
                        writeln!(
                            out,
                            "{body_pad}if _check_elapsed > {ns}_u64 {{ \
                             shim.report_violation(b\"{lv}\", _check_elapsed, {ns}_u64); }}",
                            ns = frame.latency_max_ns,
                            lv = frame.loop_var,
                        )
                        .ok();
                        writeln!(out, "{pad}}}").ok();
                        return Ok(());
                    }
                }
            }
            let (lo_s, hi_s) = render_loop_bounds(*iter_var, range, ctx)?;
            writeln!(out, "{pad}for {var} in ({lo_s})..({hi_s}) {{").ok();
            render_events(body, out, indent + 1, ctx)?;
            writeln!(out, "{pad}}}").ok();
            Ok(())
        }
        Event::Push { .. } | Event::Wait { .. } | Event::Sync { .. } => {
            Err(EmitError::UnsupportedFeature(
                "embedded-pattern (M9) is single-worker; cross-worker \
                 Push/Wait/Sync events do not occur in the naive single-\
                 worker event list. Multi-MCU transfers are M11 (TASK-0049)."
                    .to_string(),
            ))
        }
        Event::Alloc { .. } | Event::Free { .. } => Err(EmitError::UnsupportedFeature(
            "embedded-pattern (M9) lays data out as fixed `[T; N]` locals; \
             explicit Alloc/Free region events do not occur in the naive \
             single-worker event list. Region-placed data (`place_data D in \
             tcm_per_core`) is M10 shim work (TASK-0048)."
                .to_string(),
        )),
    }
}

/// Lower a single `Fire`. PURE (indexed-output) firings call the
/// extracted kernel; EFFECTFUL firings (top-level load / save) map to
/// [`NucleusShim`] hooks.
fn render_fire(
    kernel: KernelId,
    bindings: &FireBinding,
    out: &mut String,
    pad: &str,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    let callee = kernel_name(kernel, ctx.names)?;
    match &bindings.output {
        // ---- Effectful OUTPUT (save): `save_output(c)` ----
        None => {
            // The effect's data argument is the region drained to the
            // peripheral. There is exactly one aggregate data input on
            // the tier-1 save kernels; route it through `dma_push` then
            // `dma_wait`. The stub shim no-ops both.
            let drained = first_data_input(bindings, ctx)?;
            writeln!(
                out,
                "{pad}// effectful output `{callee}`: drain region to peripheral via shim."
            )
            .ok();
            writeln!(
                out,
                "{pad}shim.dma_push(0, {drained}.as_ptr() as *const u8, core::mem::size_of_val(&{drained}));"
            )
            .ok();
            writeln!(out, "{pad}shim.dma_wait(0);").ok();
            Ok(())
        }
        // ---- Whole-array OUTPUT ----
        Some(o) if o.indices.is_empty() => {
            let name = data_name(o.data, ctx)?;
            if bindings.inputs.is_empty() {
                // Effectful INPUT (load): `a <-- load_input()`. Fill the
                // region `name` from the shim's input source. The shim's
                // `alloc_in_region` hands back a pointer into the
                // Renode-injected input region (advancing an internal
                // cursor by the array's byte length); we copy those bytes
                // into the array. Sequential loads consume the region in
                // order, matching input.bin's array-concatenation layout
                // (a's N words, then b's N words) — exactly the advancing
                // offsets kernels.rs::read_i32_le_slice uses, WITHOUT this
                // backend parsing kernel bodies (PRD §6.2.2). The stub
                // shim returns null + the copy is guarded by it, so the
                // M9 compile-only lib still compiles (array stays zero-
                // filled there; honest compile-only limit). TASK-0048.02.
                writeln!(
                    out,
                    "{pad}// effectful input `{callee}`: fill region `{name}` from the shim's input source."
                )
                .ok();
                writeln!(
                    out,
                    "{pad}let __src = shim.alloc_in_region(0, core::mem::size_of_val(&{name}));"
                )
                .ok();
                writeln!(out, "{pad}shim.dma_wait(0);").ok();
                // Copy the loaded bytes into the array IFF the shim handed
                // back a non-null source (the stub returns null => no copy,
                // so the compile-only lib's zero-fill is preserved).
                writeln!(out, "{pad}if !__src.is_null() {{").ok();
                writeln!(
                    out,
                    "{pad}    unsafe {{ core::ptr::copy_nonoverlapping(__src, {name}.as_mut_ptr() as *mut u8, core::mem::size_of_val(&{name})); }}"
                )
                .ok();
                writeln!(out, "{pad}}}").ok();
                Ok(())
            } else {
                // A whole-array PURE compute (no tier-1 example hits this
                // in single-worker naive, but lower it faithfully rather
                // than mis-classify). The kernel returns a whole array;
                // M9's fixed-array layout cannot bind a freshly-returned
                // aggregate without an allocation contract. Reject with
                // a precise forward link instead of emitting wrong code.
                Err(EmitError::UnsupportedFeature(format!(
                    "embedded-pattern (M9): kernel `{callee}` is a whole-array \
                     compute firing (non-indexed output WITH inputs). The naive \
                     single-worker schedules of examples 1+5 do not produce \
                     this shape; lowering it needs an aggregate-return binding \
                     contract (future work)."
                )))
            }
        }
        // ---- Indexed OUTPUT (pure compute): `c[i] <-- add(..)` ----
        Some(o) => {
            let rendered_args = render_fire_args(kernel, &bindings.inputs, ctx)?;
            let rhs = format!("kernels::{callee}({rendered_args})");
            let stmt = backend_common::render::render_fire_output_assign(o, &rhs, ctx)?;
            writeln!(out, "{pad}{stmt}").ok();
            Ok(())
        }
    }
}

/// The Rust expression for the first `Data` input of an effect firing
/// (the array a `save_*` kernel drains). Tier-1 save kernels take
/// exactly one aggregate argument.
fn first_data_input(bindings: &FireBinding, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    use nucleus_compiler::event::ArgBinding;
    for inp in &bindings.inputs {
        if let ArgBinding::Data(slice) = inp {
            // Whole-array reference (a save kernel takes the whole
            // array): emit the bare data name. (An indexed save is not a
            // tier-1 shape; if it ever appears the bare name is still the
            // backing array, which is the region to drain.)
            return data_name(slice.data, ctx);
        }
    }
    Err(EmitError::ContractGap(
        "effectful output firing has no data input to drain to the \
         peripheral; the embedded backend expected a `save_*(array)` shape"
            .to_string(),
    ))
}

/// Write `content` to `path`, mapping io errors to
/// [`EmitError::WriteFailed`] with the offending path attached.
fn write_file(path: &Path, content: &str) -> Result<(), EmitError> {
    fs::write(path, content).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
