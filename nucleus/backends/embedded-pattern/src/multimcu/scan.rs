//! Event-tree scan helpers for the multi-MCU transport plan: channel
//! collection, endpoint pairing, effectful-IO/load classification, and
//! saved-symbol collection (TASK-0049.05 / TASK-0450 split).

use std::collections::{BTreeMap, BTreeSet};

use backend_common::EmitError;
use nucleus_compiler::event::{DataId, Event, FireBinding, KernelId, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

/// Collect the distinct CHANNELS (`SeqTag.0`) a worker participates in (as
/// a Push sender or a Wait receiver), recursing into loop bodies. Each
/// distinct seq gets its OWN USART/hub (TASK-0049.05.02) so same-direction
/// multi-seq cannot cross on a shared byte FIFO.
pub(super) fn collect_seqs(events: &[Event], out: &mut BTreeSet<u64>) {
    for e in events {
        match e {
            Event::Push { seq, .. } | Event::Wait { seq, .. } => {
                out.insert(seq.0);
            }
            Event::Loop { body, .. } => collect_seqs(body, out),
            _ => {}
        }
    }
}

/// The two endpoints of one channel: the worker that PUSHes (sends) and the
/// worker that WAITs (receives). A `SeqTag` is unique program-wide with
/// exactly one of each, so both slots fill exactly once; a duplicate sender
/// or receiver is a contract violation and fails loud in
/// [`collect_seq_endpoints`].
#[derive(Default, Clone, Copy)]
pub(super) struct SeqEndpoints {
    pub sender: Option<WorkerId>,
    pub receiver: Option<WorkerId>,
}

/// Record worker `w`'s Push/Wait events into the global per-seq endpoint
/// map, walking the STATIC event tree once (recursing into loop bodies).
/// Fails loud if a seq gets TWO distinct senders or TWO distinct receivers
/// (a `SeqTag` must have exactly one Push and one Wait — see `event.rs`).
/// If the same seq appears more than once in ONE worker's static event
/// list (e.g. a Push inside a loop body), the re-assertion is idempotent
/// because `prev == w` (same worker) — only a DIFFERENT worker claiming the
/// same role trips the guard.
pub(super) fn collect_seq_endpoints(
    w: WorkerId,
    events: &[Event],
    out: &mut BTreeMap<u64, SeqEndpoints>,
) -> Result<(), EmitError> {
    for e in events {
        match e {
            Event::Push { seq, .. } => {
                let ep = out.entry(seq.0).or_default();
                if let Some(prev) = ep.sender {
                    if prev != w {
                        return Err(EmitError::ContractGap(format!(
                            "transport channel seq {} has two distinct Push \
                             senders ({prev:?} and {w:?}); a SeqTag must have \
                             exactly one sender (TASK-0049.05.02)",
                            seq.0
                        )));
                    }
                }
                ep.sender = Some(w);
            }
            Event::Wait { seq, .. } => {
                let ep = out.entry(seq.0).or_default();
                if let Some(prev) = ep.receiver {
                    if prev != w {
                        return Err(EmitError::ContractGap(format!(
                            "transport channel seq {} has two distinct Wait \
                             receivers ({prev:?} and {w:?}); a SeqTag must \
                             have exactly one receiver (TASK-0049.05.02)",
                            seq.0
                        )));
                    }
                }
                ep.receiver = Some(w);
            }
            Event::Loop { body, .. } => collect_seq_endpoints(w, body, out)?,
            _ => {}
        }
    }
    Ok(())
}

/// A worker has an effectful LOAD iff any of its `Fire`s is an effectful
/// load — mirrors `render_fire`'s classification (see [`is_effectful_load`]).
/// Fallible: the indexed-effectful arm consults the kernel's purity in the
/// `NameSidecar`, and a missing `KernelSig` (`nucleus_compiler::sidecar::KernelSig`)
/// fails loud (`ContractGap`) rather than silently mis-classifying.
pub(super) fn has_effectful_load(
    events: &[Event],
    sidecar: &NameSidecar,
) -> Result<bool, EmitError> {
    for e in events {
        let hit = match e {
            Event::Fire {
                kernel, bindings, ..
            } => is_effectful_load(*kernel, bindings, sidecar)?,
            Event::Loop { body, .. } => has_effectful_load(body, sidecar)?,
            _ => false,
        };
        if hit {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Collect the OUTPUT symbol(s) a worker SAVES — the drained datum of each
/// output-less (effectful-save) `Fire`, recursing into loop bodies, in
/// encounter order, deduped (TASK-0049.10.05).
///
/// An effectful save is an output-less `Fire` (`save_output(c)` /
/// `fe_emit(spk_out[frame])`) — the SAME `bindings.output.is_none()` SAVE arm
/// that `render_fire` routes through `effect_drain_place`. (Note this is
/// NARROWER than `is_effectful_io`, which ALSO counts effectful LOADs — those
/// have `output.is_some()` and are not saves.)
/// The drained datum is the FIRST `ArgBinding::Data` input (the symbol whose
/// bytes stream out USART1): `fe_emit(spk_out[frame])` →`spk_out`,
/// `rf_transmit(bt_out[frame])` →`bt_out`. A save whose argument is not a
/// data read (e.g. a bare scalar) drains no capturable symbol and is skipped
/// here — its USART1 still carries whatever `render_fire` streams, but there
/// is no named output symbol to order in the capture layout.
///
/// Mirrors [`collect_loaded_symbols`](super::input_offsets::collect_loaded_symbols)
/// (the INPUT-side sibling) — same recursion, same `!out.contains` dedup,
/// same encounter-order contract (the caller sorts by `data_decl_order` for
/// the global layout).
pub(super) fn collect_saved_symbols(events: &[Event], out: &mut Vec<DataId>) {
    for e in events {
        match e {
            Event::Fire { bindings, .. } if bindings.output.is_none() => {
                if let Some(did) = first_data_input(bindings) {
                    if !out.contains(&did) {
                        out.push(did);
                    }
                }
            }
            Event::Loop { body, .. } => collect_saved_symbols(body, out),
            _ => {}
        }
    }
}

/// The `DataId` of the first `ArgBinding::Data` input of a firing, if any —
/// the symbol an effectful-save drains. A `Scalar`/`Nested` argument is not a
/// capturable data read, so it is skipped.
///
/// SIBLING-CONSISTENCY: this is the SAME first-`Data`-input selection that
/// `render::effect_drain_place` uses to pick the region a save drains to the
/// peripheral. A save with NO `Data` input is rejected there with a typed
/// `ContractGap` (it cannot lower at all), so a `None` here mirrors a firing
/// that produces no capturable bytes — consistent, not a silent drop.
fn first_data_input(bindings: &FireBinding) -> Option<DataId> {
    use nucleus_compiler::event::ArgBinding;
    bindings.inputs.iter().find_map(|a| match a {
        ArgBinding::Data(slice) => Some(slice.data),
        _ => None,
    })
}

/// True iff this `Fire` is an effectful LOAD — the source half of
/// `render_fire`'s INPUT classification. Two shapes, BOTH gated on no
/// inputs:
///
///   * **whole-array** (`a <-- load_input()`): output present with EMPTY
///     indices. STRUCTURAL, purity-independent — UNCHANGED since
///     TASK-0049.05 (backs 02-split-add + the 7 M6 + M10 ex1/5/9 loads).
///   * **indexed effectful** (`mic_in[frame] <-- fe_capture()`,
///     TASK-0049.10.04): output present with NON-EMPTY indices AND the
///     kernel is declared `effectful`. This shape is structurally identical
///     to a PURE indexed compute (`c[i] <-- add(..)` with no data inputs),
///     so it MUST be disambiguated by purity — exactly the silent-sibling
///     of slice A's (TASK-0049.10.01) `render_fire` fix, which this arm
///     mirrors. Without it `fe`/`rf`'s per-frame captures were not
///     recognised as loads, so their machines got NO `$input` injected and
///     the shim read garbage.
///
/// Fallible: a missing `KernelSig` in the indexed arm fails loud
/// (`ContractGap`) via the shared
/// [`backend_common::render::kernel_is_effectful`].
pub(super) fn is_effectful_load(
    kernel: KernelId,
    bindings: &FireBinding,
    sidecar: &NameSidecar,
) -> Result<bool, EmitError> {
    if !bindings.inputs.is_empty() {
        return Ok(false);
    }
    match &bindings.output {
        // whole-array load: STRUCTURAL, unchanged.
        Some(o) if o.indices.is_empty() => Ok(true),
        // indexed effectful load: gated on purity (NEW, additive).
        Some(_) => backend_common::render::kernel_is_effectful(kernel, sidecar),
        None => Ok(false),
    }
}

/// True iff this `Fire` is a GLOBALLY-OBSERVABLE external IO side effect —
/// an effectful load (`a <-- load_input()` / `mic_in[frame] <-- fe_capture()`)
/// or save (`save_output(c)`), the only firings that map to a peripheral
/// hook in `render_fire`. A pure (indexed-output) compute firing is NOT
/// observable across MCUs and an inter-MCU Push/Wait is transport (handled
/// separately), so neither counts here. Fallible for the same reason as
/// [`is_effectful_load`].
pub(super) fn is_effectful_io(
    kernel: KernelId,
    bindings: &FireBinding,
    sidecar: &NameSidecar,
) -> Result<bool, EmitError> {
    Ok(bindings.output.is_none() || is_effectful_load(kernel, bindings, sidecar)?)
}
