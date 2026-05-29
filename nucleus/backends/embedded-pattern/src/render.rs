//! `run<S>` body rendering for the embedded-pattern backend
//! (TASK-0340.10 file-hygiene split out of `lib.rs`).
//!
//! This module is the shared event-lowering core of BOTH emit modes:
//! [`crate::emit`] (the M9 compile-only LIB) and [`crate::emit_bin`] (the
//! M10 Renode-runnable BIN) reach it via `lower_kernels_and_run` in the
//! crate root, so the lib and bin lower IDENTICALLY — the only difference
//! between the two modes is the surrounding scaffolding (see the crate
//! root module docs + `skeleton`), NOT the event lowering here.
//!
//! The entry point is [`render_run_body`]; everything else is its
//! recursive helper set. Nothing here is exported beyond the crate
//! (`pub(crate)` on the entry point; the helpers are module-private).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use backend_common::render::{data_name, render_fire_args_nostd, render_loop_bounds, RenderCtx};
use backend_common::EmitError;
use nucleus_compiler::event::{DataId, Event, FireBinding, KernelId, ViolationKind};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::kernel_name;

/// Render the body of `pub fn run<S: NucleusShim>(shim: &mut S)`:
/// fixed-array data declarations followed by the lowered event list.
pub(crate) fn render_run_body(
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
/// the event tree, PLUS every data symbol crossing a worker boundary via
/// `Push` / `Wait` (TASK-0049.04). A `Push` drains a local buffer and a
/// `Wait` receives into one; both need the symbol declared as a fixed
/// `[T; N]` local in the run body (the `Wait` receive local stays
/// zero-initialised — the StubShim no-ops the receive, honest for a
/// compile-only slice).
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
            Event::Push { data, .. } | Event::Wait { data, .. } => {
                out.insert(*data);
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
                             explicit `on_violation = log` (per-violation UART \
                             line) or `on_violation = count` (a violation \
                             counter summarised over USART1 at program exit, \
                             TASK-0048.08) to the `check loop {lv}` directive.",
                            lv = frame.loop_var,
                        )));
                    }
                    // count: fully lowered on tier-3 (TASK-0048.08, PART 1).
                    // SAME SysTick per-iteration timing as the Log arm; on
                    // violation, atomically increment a MODULE-scope
                    // `static NUC_CHECK_COUNT_<ident>: AtomicU32` instead of
                    // calling report_violation. AtomicU32 (not the tier-1
                    // AtomicU64) because AtomicU64 is unavailable on
                    // thumbv7em-none-eabihf; Relaxed because the firmware is
                    // single-core and the counter is only read after run()
                    // returns on the SAME core. The bare-metal summary sink
                    // (the tier-1 Drop-guard does not fire — firmware spins
                    // forever) is emitted by `render_bin_main` AFTER
                    // `run(&mut shim)` and BEFORE `loop {}`. This arm is
                    // SHARED lib+bin: the lib path emits the same static (so
                    // it cross-compiles) but, being a `StubShim` lib with no
                    // `main`, never flushes a summary — fine for compile-only.
                    ViolationKind::Count => {
                        let ident =
                            backend_common::check_frame::sanitize_loop_var(&frame.loop_var);
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
                        // `ident` is built structurally from the sanitised
                        // loop_var (no textual replace on a rendered
                        // expression — feedback-textual-replace-codegen-unsafe).
                        writeln!(
                            out,
                            "{body_pad}if _check_elapsed > {ns}_u64 {{ \
                             NUC_CHECK_COUNT_{id}.fetch_add(1, \
                             core::sync::atomic::Ordering::Relaxed); }}",
                            ns = frame.latency_max_ns,
                            id = ident,
                        )
                        .ok();
                        writeln!(out, "{pad}}}").ok();
                        return Ok(());
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
        // Cross-worker transport (TASK-0049.04, M11 backend slice A).
        // Lowered to the stub NucleusShim hooks; compile-only no-ops
        // against StubShim (this slice is a real cross-compile, NOT a
        // Renode run — see module docs). Channel/tag come STRAIGHT from
        // the events: SeqTag is the dma channel id (a Push/Wait pair
        // shares its `seq`), SyncTag is the irq_barrier tag — there is no
        // channel allocator.
        Event::Push { data, seq, .. } => {
            // Drain the local data buffer to the peer over the DMA
            // channel. Mirrors the effectful-save `dma_push` template
            // (the data symbol is a fixed `[T; N]` local, sized by the
            // run-body data decls; `collect_referenced_data` includes
            // Push/Wait data so the symbol is in scope).
            let name = data_name(*data, ctx)?;
            let chan = seq.0;
            writeln!(
                out,
                "{pad}// Push `{name}` (seq {chan}) to peer worker via the inter-MCU transport."
            )
            .ok();
            writeln!(
                out,
                "{pad}shim.dma_push({chan}, {name}.as_ptr() as *const u8, \
                 core::mem::size_of_val(&{name}));"
            )
            .ok();
            Ok(())
        }
        Event::Wait { data, seq, .. } => {
            // Block until the peer's matching Push (same `seq`) lands in
            // the receive buffer `name`. The receive local is declared
            // zero-initialised by the run-body data decls (the StubShim
            // no-ops dma_wait, so zero-init is correct for a compile-only
            // slice — see module docs). We do NOT re-declare it here;
            // declaring it at the top of `run` keeps a single home for
            // every data symbol (loads, computes, and now Waits).
            let name = data_name(*data, ctx)?;
            let chan = seq.0;
            writeln!(
                out,
                "{pad}// Wait for `{name}` (seq {chan}) from peer worker; receive into the local."
            )
            .ok();
            writeln!(out, "{pad}shim.dma_wait({chan});").ok();
            Ok(())
        }
        Event::Sync { sync, .. } => {
            // Cross-worker control barrier -> the IRQ-completion barrier
            // hook. The SyncTag is the stable cross-worker barrier
            // identity (every participant carries the same tag), used
            // verbatim as the irq_barrier tag.
            let tag = sync.0;
            writeln!(
                out,
                "{pad}// Cross-worker barrier (sync tag {tag}) -> irq_barrier."
            )
            .ok();
            writeln!(out, "{pad}shim.irq_barrier({tag} as u32);").ok();
            Ok(())
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
/// `NucleusShim` hooks.
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
            // no_std arg materialisation: a contiguous-prefix sub-array
            // argument (e.g. `mic_in[frame]` of an `i32[N_FRAMES][16]`
            // datum, an array-typed pure kernel param) renders as a
            // fixed `[T; N]` via `.try_into().unwrap()` — NOT the tier-1
            // `.to_vec()`, which needs `alloc`/`Vec`. Scalar (full-rank)
            // args render identically to tier-1. This is what makes the
            // real example-14 array-typed mix2/denoise pure kernels
            // cross-compile under `no_std` (TASK-0049.06; the lowering
            // gap the M11 cross-compile surfaced).
            let rendered_args = render_fire_args_nostd(kernel, &bindings.inputs, ctx)?;
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
