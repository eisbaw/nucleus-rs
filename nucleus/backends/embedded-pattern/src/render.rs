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

use backend_common::multi_worker_walker::WireShape;
use backend_common::render::{
    data_name, render_fire_args_nostd, render_indexed_subarray_place, render_loop_bounds,
    RenderCtx,
};
use backend_common::EmitError;
use nucleus_compiler::event::{DataId, Event, FireBinding, KernelId, ViolationKind};
use nucleus_compiler::sched::TransportMode;
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
        // Zero (NOT combine-identity, TASK-0343.01.02): tier-3 embedded
        // has no multi-worker accumulator fan-in path (single-MCU, no
        // partition partials to host-combine), so no `combine=min|max|
        // and` accumulator reaches here; the zero init is correct.
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
            break_cond,
        } => {
            // `for..until` early-exit break (epic S4,
            // TASK-0341.02.01.05.04) is emitted ONLY by the tier-1
            // single-worker sequential backend (`pthreads-sync`) this
            // slice. The embedded (M9/M10) lowering does not yet port the
            // break emit; rather than silently dropping the predicate
            // (which would mis-lower a convergence loop to a non-
            // terminating full-cap loop), reject loud with a forward link.
            // A naive embedded schedule carries no `for..until` today, so
            // this is inert; cross-backend break emit is later work
            // (S7, TASK-0341.02.01.08).
            if break_cond.is_some() {
                return Err(EmitError::UnsupportedFeature(
                    "embedded-pattern does not yet lower `for..until` early-exit \
                     (break-condition) loops; the runtime break emit is tier-1 \
                     single-worker only this slice (TASK-0341.02.01.05.04). \
                     Cross-backend break emit is future work \
                     (S7, TASK-0341.02.01.08)."
                        .to_string(),
                ));
            }
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
                        let ident = backend_common::check_frame::sanitize_loop_var(&frame.loop_var);
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
        // Cross-worker transport (TASK-0049.04 LIB slice A; TASK-0049.05
        // BIN slice B). Lowered to the DEDICATED transport hooks
        // `link_push` / `link_recv` — DISTINCT from the effectful
        // `dma_push` / `dma_wait` (which drain to a local peripheral) so a
        // real multi-MCU shim can route inter-MCU transport and peripheral
        // IO to different USARTs (TASK-0049.05 trap #1: on the host,
        // `save_output` -> `dma_push(0)` and `Push a` -> `link_push(0)`
        // would otherwise collide on one channel id). Channel/tag come
        // STRAIGHT from the events: `SeqTag` is the transport channel id (a
        // Push/Wait pair shares its `seq`); `SyncTag` is the barrier tag.
        // The per-`seq` -> USART/hub physical mapping is the multi-MCU
        // shim's job (`skeleton::render_multimcu_bin_main`), NOT a channel
        // allocator here. Against the compile-only `StubShim` both link
        // hooks no-op (a real cross-compile, not a Renode run — module docs).
        Event::Push { data, seq, .. } => {
            // Send the local data buffer to the peer over transport channel
            // `seq`. The data symbol is a fixed `[T; N]` local, sized by the
            // run-body data decls (`collect_referenced_data` includes
            // Push/Wait data so the symbol is in scope).
            let name = data_name(*data, ctx)?;
            let chan = seq.0;
            // TASK-0455.07: the transport byte-length is the ONE
            // WireShape sender extent expression (whole-array
            // `size_of_val` today; TASK-0453.22 narrows it to the
            // recv_basis span). The embedded backend has no `pair_tiles`
            // substrate (it lays data out as `[T; N]` locals, single-
            // worker / multi-MCU naive only), so it derives the
            // whole-array shape via `from_tile(.., None)`.
            let wire = WireShape::from_tile(ctx.sidecar, *data, None)?;
            let byte_len = wire.sender_byte_len_expr(&name);
            // Per-seq transport mode (TASK-0438.02). Threaded from the
            // schedule's `transfer DATA : mode=pio|dma` directive through
            // `TransferPolicy.transport` into the unified
            // `NameSidecar.xfer_facts` map (TASK-0455.08) and read via the
            // `xfer_transport` accessor. A seq absent there renders the
            // unchanged PIO path — so a schedule with NO `mode=` directive
            // is byte-identical to pre-TASK-0438.02 (load-bearing for the
            // 02-split-add / 14-hearing-aid byte-exact gate, AC#3).
            let mode = ctx.sidecar.xfer_transport(*seq);
            match mode {
                TransportMode::Pio => {
                    // PIO comment + call kept BYTE-IDENTICAL to pre-TASK-0438.02
                    // so a no-`mode=` schedule (02-split-add / 14-hearing-aid)
                    // emits an unchanged Push (AC#3).
                    writeln!(
                        out,
                        "{pad}// Push `{name}` (seq {chan}) to peer worker via the inter-MCU transport."
                    )
                    .ok();
                    writeln!(
                        out,
                        "{pad}shim.link_push({chan}, {name}.as_ptr() as *const u8, \
                         {byte_len});"
                    )
                    .ok();
                }
                TransportMode::Dma => {
                    // DMA push. A `buffer>=2` edge (TASK-0455.02) renders the
                    // depth-2 descriptor RING (split-phase, occupancy-tracked,
                    // notify-selected completion); a `buffer=1` / no-`buffer=`
                    // edge keeps the single-buffer arm + completion-spin
                    // (byte-identical to pre-TASK-0455.02, load-bearing for the
                    // 22-dma-pio-demo gate whose edges are all single-buffered).
                    if crate::render_dma_ring::is_double_buffered(ctx.sidecar.xfer_buffer(*seq)) {
                        let suf = crate::render_dma_ring::ring_suffix(*seq);
                        let notify = ctx.sidecar.xfer_notify(*seq);
                        crate::render_dma_ring::render_ring_push(
                            out, &pad, &name, &byte_len, chan, &suf, notify,
                        )?;
                    } else {
                        // Single-buffer DMA-async push: arm a transfer
                        // descriptor, then spin on the completion flag. This is
                        // a MODELLED DMA SHAPE, NOT a silicon DMA engine (AC#4)
                        // — the bytes ride the SAME UART fabric (the default
                        // `dma_link_arm` delegates to `link_push`). The real
                        // STM32H7 DMA engine is deferred to TASK-0048.12.
                        //
                        // SPIN, not `wfi` (AC#2): the modelled shim completes
                        // the transfer synchronously inside the arm call (no
                        // real DMA-complete IRQ fires), so `wfi` would deadlock
                        // waiting for an interrupt that never arrives.
                        // `dma_link_poll` returns true immediately, so the spin
                        // loop is the honest structural completion-wait and
                        // terminates on its first iteration.
                        writeln!(
                            out,
                            "{pad}// DMA-async push of `{name}` (seq {chan}): arm descriptor, then spin on completion."
                        )
                        .ok();
                        writeln!(
                            out,
                            "{pad}shim.dma_link_arm({chan}, {name}.as_ptr() as *const u8, \
                             {byte_len});"
                        )
                        .ok();
                        writeln!(
                            out,
                            "{pad}while !shim.dma_link_poll({chan}) {{ core::hint::spin_loop(); }}"
                        )
                        .ok();
                    }
                }
            }
            Ok(())
        }
        Event::Wait { data, seq, .. } => {
            // Block until the peer's matching Push (same `seq`) lands, and
            // RECEIVE the bytes INTO the local `name`. The receive local is
            // declared zero-initialised by the run-body data decls; we pass
            // it as a `*mut u8` + byte length so the concrete multi-MCU shim
            // fills it (the `StubShim` no-ops the receive, so the local
            // stays zero-filled — correct for the compile-only LIB slice;
            // see module docs). We do NOT re-declare it here; declaring it
            // at the top of `run` keeps a single home for every data symbol
            // (loads, computes, and Waits).
            let name = data_name(*data, ctx)?;
            let chan = seq.0;
            // TASK-0455.07: the receive byte-length is the ONE WireShape
            // extent expression — the embedded receiver `_tmp` basis.
            // Whole-array `size_of_val` today (the embedded backend has
            // no slice-paste receive; data is a `[T; N]` local filled
            // whole); on the whole-array path the receive length equals
            // the matching Push's send length by construction, so it
            // reuses `sender_byte_len_expr`. TASK-0453.22 narrows both to
            // the recv_basis span together.
            let wire = WireShape::from_tile(ctx.sidecar, *data, None)?;
            let byte_len = wire.sender_byte_len_expr(&name);
            // Per-seq transport mode (TASK-0438.02) — symmetric to the Push
            // arm. PIO/absent: the existing blocking byte-loop receive; DMA:
            // a descriptor-arm + completion-spin. See the Push arm for the
            // SPIN-not-wfi rationale and the modelled-vs-silicon caveat.
            let mode = ctx.sidecar.xfer_transport(*seq);
            match mode {
                TransportMode::Pio => {
                    writeln!(
                        out,
                        "{pad}// Wait for `{name}` (seq {chan}) from peer worker; receive into the local."
                    )
                    .ok();
                    writeln!(
                        out,
                        "{pad}shim.link_recv({chan}, {name}.as_mut_ptr() as *mut u8, \
                         {byte_len});"
                    )
                    .ok();
                }
                TransportMode::Dma => {
                    // Symmetric to the Push DMA arm: a `buffer>=2` edge renders
                    // the depth-2 ring receive (TASK-0455.02); a single-buffer
                    // edge keeps the arm + completion-spin (byte-identical to
                    // pre-TASK-0455.02).
                    if crate::render_dma_ring::is_double_buffered(ctx.sidecar.xfer_buffer(*seq)) {
                        let suf = crate::render_dma_ring::ring_suffix(*seq);
                        let notify = ctx.sidecar.xfer_notify(*seq);
                        crate::render_dma_ring::render_ring_recv(
                            out, &pad, &name, &byte_len, chan, &suf, notify,
                        )?;
                    } else {
                        writeln!(
                            out,
                            "{pad}// DMA-async receive of `{name}` (seq {chan}): arm descriptor, then spin on completion."
                        )
                        .ok();
                        writeln!(
                            out,
                            "{pad}shim.dma_link_recv_arm({chan}, {name}.as_mut_ptr() as *mut u8, \
                             {byte_len});"
                        )
                        .ok();
                        writeln!(
                            out,
                            "{pad}while !shim.dma_link_poll({chan}) {{ core::hint::spin_loop(); }}"
                        )
                        .ok();
                    }
                }
            }
            Ok(())
        }
        Event::Sync { sync, .. } => {
            // Cross-worker control barrier -> the IRQ-completion barrier
            // hook. The SyncTag is the stable cross-worker barrier identity
            // (every participant carries the same tag), passed verbatim as a
            // `u64` — NOT cast to `u32`, which would silently truncate a
            // large tag (TASK-0049.05 trap #2).
            let tag = sync.0;
            writeln!(
                out,
                "{pad}// Cross-worker barrier (sync tag {tag}) -> irq_barrier."
            )
            .ok();
            writeln!(out, "{pad}shim.irq_barrier({tag}_u64);").ok();
            Ok(())
        }
        Event::Alloc { .. } | Event::Free { .. } => Err(EmitError::UnsupportedFeature(
            "embedded-pattern lays data out as fixed `[T; N]` locals; \
             explicit Alloc/Free region events do not occur in any event \
             list this backend receives. Alloc/Free are a deliberately- \
             reserved contract surface emitted by no pass (TASK-0455.16; see \
             nucleus_compiler::event module-doc \"DELIBERATELY RESERVED\"). A \
             `place_data D in REGION` schedule never reaches codegen as an \
             event: it is consumed as a capability-admission gate, and the \
             only corpus schedule using it (14-hearing-aid embedded_multimcu) \
             is rejected for requesting `sram_shared` on this heap-only \
             backend. This arm fails loud only if a future tier starts \
             emitting Alloc/Free."
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
            // peripheral; route it through `dma_push` then `dma_wait`
            // (the stub shim no-ops both). `effect_drain_place` returns
            // the BARE array name for a whole-array save (tier-1
            // `save_output(c)`, byte-identical to before) and the INDEXED
            // FRAME place `spk_out[start..start + 16usize]` for a
            // per-frame indexed drain (ex14 `fe_emit(spk_out[frame])`),
            // so `size_of_val(&{drained})` sizes the whole array or the
            // one frame row respectively (TASK-0049.10.02, slice B).
            let drained = effect_drain_place(bindings, ctx)?;
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
        // ---- Indexed OUTPUT, EFFECTFUL, zero-input: per-frame
        //      peripheral region-read `mic_in[frame] <-- fe_capture()` ----
        //
        // ADDITIVE branch (TASK-0049.10.01): an effectful kernel with an
        // indexed output AND no inputs is a PER-FRAME peripheral capture
        // (fe_capture/rf_receive in ex14: `mic_in[frame] <-- fe_capture()`,
        // `kernel fe_capture : () -> i32[16] effectful`). Structurally it
        // is indistinguishable from a pure indexed compute (same Fire
        // shape: indexed output, the only inputs being none), so WITHOUT
        // the purity bit it lowered to the verbatim-extracted
        // `kernels::fe_capture()` stub which returns `[0i32; 16]` — the
        // firmware input arrays were all zeros and could never match the
        // real reference output (BLOCKER 1). Here we instead read ONE
        // indexed element's worth of bytes from the shim's input region
        // into `mic_in[frame]`, modelled on the whole-array effectful-load
        // arm above but targeting the indexed sub-slice (sized by the row,
        // NOT the whole array). The shim cursor advances by the row size
        // each frame, so sequential per-frame reads consume the input
        // region in frame order. The stub shim returns null + the copy is
        // null-guarded, so the M9 compile-only lib still compiles (the row
        // stays zero-filled there; honest compile-only limit, same as the
        // whole-array load arm).
        //
        // Per-frame effectful OUTPUT drain (fe_emit/rf_transmit, output =
        // None) is slice B (TASK-0049.10.B) — the `None` arm above still
        // drains the WHOLE array each frame and is intentionally untouched
        // here. Per-worker input partitioning + multi-output .resc capture
        // is slice C; renode-multimcu byte-exact is slice D. Slice A is
        // INPUT-only and does NOT claim byte-exact.
        Some(o)
            if !o.indices.is_empty()
                && bindings.inputs.is_empty()
                && backend_common::render::kernel_is_effectful(kernel, ctx.sidecar)? =>
        {
            // Guarded: a full-rank-indexed scalar place has no
            // `.as_mut_ptr()` and would emit non-compiling firmware —
            // fail loud at codegen instead (TASK-0049.10.03).
            let place = render_indexed_subarray_place(o, ctx)?;
            writeln!(
                out,
                "{pad}// effectful per-frame input `{callee}`: fill indexed slice `{place}` from the shim's input source."
            )
            .ok();
            writeln!(
                out,
                "{pad}let __src = shim.alloc_in_region(0, core::mem::size_of_val(&{place}));"
            )
            .ok();
            writeln!(out, "{pad}shim.dma_wait(0);").ok();
            // Copy the per-frame bytes into the indexed slice IFF the shim
            // handed back a non-null source (the stub returns null => no
            // copy, so the compile-only lib's zero-fill is preserved).
            writeln!(out, "{pad}if !__src.is_null() {{").ok();
            writeln!(
                out,
                "{pad}    unsafe {{ core::ptr::copy_nonoverlapping(__src, {place}.as_mut_ptr() as *mut u8, core::mem::size_of_val(&{place})); }}"
            )
            .ok();
            writeln!(out, "{pad}}}").ok();
            Ok(())
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

/// The Rust place-expression for the first `Data` input of an effect
/// firing — the region a `save_*` / per-frame `*_emit` kernel drains to
/// the peripheral. Returns the drain TARGET sized for the `dma_push` /
/// `size_of_val` shape in the `None` arm of [`render_fire`]:
///
/// - **Empty indices** (the tier-1 `save_output(c)` / `save_image(img)`
///   shape, and every shipped M6 + M10 + 02-split-add cell): the BARE
///   data name `c`. `size_of_val(&c)` is the whole fixed-array — drain
///   the whole array, BYTE-IDENTICAL to the pre-TASK-0049.10.02 emit.
/// - **Non-empty indices** (the ex14 per-frame `fe_emit(spk_out[frame])`
///   / `rf_transmit(bt_out[frame])` shape): the INDEXED frame place via
///   [`render_indexed_subarray_place`] (`spk_out[start..start + 16usize]`).
///   `size_of_val(&place)` is the per-frame row, so the drain targets
///   exactly the one indexed frame slice instead of the whole array each
///   frame (TASK-0049.10.02, slice B — the structural mirror of slice
///   A's per-frame INPUT region read). `.as_ptr()` / `size_of_val` work
///   on a `[T]` sub-array place exactly as on a `[T; N]` array, so the
///   `dma_push` / `dma_wait` shape in the caller is otherwise unchanged.
///   A FULL-rank-indexed scalar place (`D[idx]`, a single `T`) has no
///   `.as_ptr()` and is rejected by the guarded helper at codegen
///   (TASK-0049.10.03) rather than emitting non-compiling firmware.
///
/// `render_indexed_subarray_place` REQUIRES non-empty indices (its
/// documented caller responsibility), so the empty-indices arm is gated
/// to the bare `data_name` path. No textual `String::replace` on a
/// rendered expr — the indexed place is built structurally by the shared
/// helper.
fn effect_drain_place(bindings: &FireBinding, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    use nucleus_compiler::event::ArgBinding;
    for inp in &bindings.inputs {
        if let ArgBinding::Data(slice) = inp {
            if slice.indices.is_empty() {
                // Whole-array reference (a tier-1 save kernel takes the
                // whole array): the bare data name is the region to
                // drain. Unchanged whole-array drain.
                return data_name(slice.data, ctx);
            }
            // Indexed per-frame drain (ex14 fe_emit/rf_transmit): the
            // indexed frame place `D[start..start + sub_len]`, drained
            // sized by the row. The caller applies `.as_ptr()`, which is
            // valid only on a `[T]` slice — so we use the GUARDED helper
            // `render_indexed_subarray_place` (TASK-0049.10.03): a
            // PARTIAL-rank index yields the sub-array place; a FULL-rank
            // index (`SliceForm::Scalar` `D[idx]`, a single `T` with no
            // `.as_ptr()`) is rejected at codegen with a typed
            // `EmitError::UnsupportedFeature` instead of emitting firmware
            // that fails the cross-compile with an opaque rustc error. No
            // current shape hits the Scalar arm (ex14 data is 2-D, an
            // `[frame]` index is partial-rank).
            return render_indexed_subarray_place(slice, ctx);
        }
    }
    Err(EmitError::ContractGap(
        "effectful output firing has no data input to drain to the \
         peripheral; the embedded backend expected a `save_*(array)` shape"
            .to_string(),
    ))
}
