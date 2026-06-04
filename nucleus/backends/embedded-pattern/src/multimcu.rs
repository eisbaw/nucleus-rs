//! M11 multi-MCU transport plan (TASK-0049.05, BIN slice B).
//!
//! The single-worker M10 bin ([`crate::emit_bin`] -> one `Usart1Shim`)
//! has no inter-MCU transport. A MULTI-worker schedule emits one firmware
//! bin per worker, co-simulated as N separate Renode STM32H7 machines
//! wired by `UARTHub`s. This module computes the PHYSICAL transport plan
//! that BOTH the per-worker shim (`skeleton::render_multimcu_bin_main`)
//! AND the generated multi-machine `.resc`
//! ([`render_multimachine_resc`]) must agree on:
//!
//!   * which USART each cross-worker channel (`SeqTag`) rides on, per
//!     worker (the shim's `link_push`/`link_recv` seq->USART table) — ONE
//!     dedicated USART per `SeqTag` (TASK-0049.05.02), NOT shared per peer;
//!   * one `UARTHub` per CHANNEL (`SeqTag`) (Renode's UARTHub is a
//!     BROADCAST bus — a dedicated hub per channel keeps every transport
//!     stream on its OWN byte FIFO, so two SAME-DIRECTION channels between
//!     one worker pair can never interleave/cross on a shared FIFO; the 9
//!     USARTs the stm32h743 platform models give ample fan-out for the
//!     channel counts these schedules use — `usart1` minus the 8-slot pool
//!     below). A worker using more than 8 distinct channels fails loud;
//!     scaling past that needs per-seq framing on a shared FIFO
//!     (TASK-0049.05.02 follow-up);
//!   * a receivers-first BOOT ORDER so the
//!     receiver-RX-enabled-before-sender-TX start-gating discipline
//!     (TASK-0049.01: Renode's `UARTBase.WriteChar` DROPS bytes that
//!     arrive before the receiver enables RX) holds BY CONSTRUCTION for a
//!     pipeline DAG, not by scheduler luck.
//!
//! The `SeqTag`/peer come straight from the events: `Event::Push` carries
//! its `dst` worker, `Event::Wait` its `src` worker — so the peer of a
//! channel is read directly, no cross-referencing.

use std::collections::{BTreeMap, BTreeSet};

use backend_common::EmitError;
use nucleus_compiler::algo::Purity;
use nucleus_compiler::event::{DataId, Event, FireBinding, KernelId, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

/// A modelled STM32H743 USART (Renode `platforms/cpus/stm32h743.repl`).
/// `usart1` is RESERVED for the effectful-output capture stream (the M10
/// `save_output` -> raw USART1 path the `renode-embedded` capture relies
/// on), so the cross-worker transport pool starts at `usart2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsartSlot {
    /// Renode peripheral name (used in `connector Connect <name> <hub>`).
    pub renode_name: &'static str,
    /// Memory-mapped base address (the generated shim pokes CR1/ISR/RDR/TDR
    /// relative to this).
    pub base: u32,
}

/// The cross-worker transport USART pool, in deterministic allocation
/// order. `usart1` (0x4001_1000) is DELIBERATELY ABSENT — it is the
/// effectful-output capture channel. Eight slots; a worker needing more
/// than eight DISTINCT CHANNELS (`SeqTag`s) fails loud (a real STM32H743
/// has exactly these UARTs). Each channel gets its OWN slot so two
/// same-direction channels cannot cross on one shared byte FIFO
/// (TASK-0049.05.02).
pub(crate) const TRANSPORT_USARTS: [UsartSlot; 8] = [
    UsartSlot { renode_name: "usart2", base: 0x4000_4400 },
    UsartSlot { renode_name: "usart3", base: 0x4000_4800 },
    UsartSlot { renode_name: "usart6", base: 0x4001_1400 },
    UsartSlot { renode_name: "uart4", base: 0x4000_4C00 },
    UsartSlot { renode_name: "uart5", base: 0x4000_5000 },
    UsartSlot { renode_name: "uart7", base: 0x4000_7800 },
    UsartSlot { renode_name: "uart8", base: 0x4000_7C00 },
    UsartSlot { renode_name: "lpuart1", base: 0x5800_0C00 },
];

/// One worker's view of the multi-MCU transport.
#[derive(Debug, Clone)]
pub(crate) struct WorkerPlan {
    pub worker: WorkerId,
    pub name: String,
    /// `SeqTag.0` -> the USART that channel rides on, for THIS worker. Both
    /// `link_push(seq)` and `link_recv(seq)` consult it, AND it drives the
    /// worker's `connector Connect` lines in the `.resc`. INJECTIVE: every
    /// distinct channel (`SeqTag`) gets its OWN dedicated USART
    /// (TASK-0049.05.02), so two channels — even two in the SAME direction
    /// between one worker pair — never share a byte FIFO and cannot
    /// interleave/cross. (Before TASK-0049.05.02 this grouped channels by
    /// PEER, which let `fe`'s two same-direction sends to `dsp` collide on
    /// one FIFO and starve the downstream worker — the ex14 deadlock.)
    pub seq_usart: BTreeMap<u64, UsartSlot>,
    /// This worker performs an effectful load (`a <-- load_input()` OR the
    /// per-frame indexed `mic_in[frame] <-- fe_capture()`) — its machine
    /// gets `$input` injected into axiSram.
    pub loads_input: bool,
    /// Byte offset into the injected `input.bin` at which THIS worker's
    /// input slice starts — i.e. the value the shim's `input_cursor` must
    /// START at (TASK-0049.10.04, BLOCKER 2). The whole `input.bin` is
    /// injected at axiSram into EVERY loader (the `.resc` is unchanged); each
    /// loader's cursor is pre-seeded here so it reads its OWN slice.
    ///
    /// For a SINGLE-loader schedule (02-split-add `host` reads the whole
    /// input from the front, in firing order) this is `0` — regression-safe,
    /// byte-identical to pre-TASK-0049.10.04. For the cross-worker
    /// multi-loader case (ex14 `fe`→`mic_in`, `rf`→`bt_in`) it is the byte
    /// offset of the worker's FIRST loaded symbol in the reference
    /// generator's DECLARATION-ORDER `input.bin` layout, computed by
    /// [`compute_input_offsets`] off [`NameSidecar::data_decl_order`]
    /// (threaded into the contract by TASK-0049.10.06) — for ex14 that is
    /// `fe`=0, `rf`=256. The shim's `NUC_INPUT_BASE` seam consumes this.
    pub input_base_offset: usize,
    /// This worker performs an effectful save (`save_output(c)` /
    /// `fe_emit(spk_out[frame])`) — its USART1 is captured to a file backend.
    /// Equivalent to `!saved_outputs.is_empty()`; kept as a precomputed bool
    /// because the shim codegen (`skeleton::multimcu`) only needs the
    /// boolean "does this worker enable USART1 TX for capture".
    pub saves_output: bool,
    /// The OUTPUT symbol(s) this worker drains to its capture USART1, in
    /// encounter order — the first `ArgBinding::Data` input datum of each
    /// output-less (effectful-save) `Fire`. For ex14 each saver drains
    /// exactly one (`fe`→`spk_out`, `rf`→`bt_out`); a worker with no save
    /// has this empty (TASK-0049.10.05, BLOCKER 3 slice C2).
    pub saved_outputs: Vec<DataId>,
}

/// One saver worker's deterministic output capture, ordered globally by the
/// drained symbol's position in [`NameSidecar::data_decl_order`] (NOT
/// alphabetical `DataId`). The `.resc` writes this saver's USART1 to
/// `$<file_var>`; slice D concatenates the per-saver files in this list's
/// order to reconstruct the reference output layout (TASK-0049.10.05).
#[derive(Debug, Clone)]
pub(crate) struct OutputCapture {
    /// The saver worker.
    pub worker: WorkerId,
    /// The OUTPUT symbol this saver drains (ex14: `fe`→`spk_out`,
    /// `rf`→`bt_out`).
    pub data: DataId,
    /// The `.resc` file-backend var (`$<file_var>`) this saver's USART1
    /// writes to — the SINGLE SOURCE OF TRUTH for the capture file name (the
    /// `.resc` generator reads it directly; the shim needs only the boolean
    /// [`WorkerPlan::saves_output`]).
    ///
    /// SINGLE-saver / MULTI-saver ASYMMETRY (LOAD-BEARING, READ THIS): with
    /// exactly ONE saver in the whole schedule, the var is `uartFile` —
    /// byte-identical to the pre-TASK-0049.10.05 emit, because the
    /// `just renode-multimcu` recipe injects and reads `$uartFile` for the
    /// single saver (02-split-add `host`). Changing it would break that
    /// recipe (a slice-D concern, out of scope here). With ≥2 savers (ex14
    /// `fe`+`rf`) each saver gets a DISTINCT var `<sanitized-worker>Uart`
    /// (e.g. `feUart`, `rfUart`) so the two no longer collide on one shared
    /// `$uartFile`. Slice D extends the recipe to inject + concatenate these
    /// per-saver files in this list's order. The asymmetry exists SOLELY to
    /// keep the existing single-saver recipe byte-compatible while the
    /// multi-saver recipe is built.
    pub file_var: String,
}

/// One inter-MCU `UARTHub`, dedicated to exactly ONE channel (`SeqTag`).
/// Both endpoints Connect their own `seq_usart[seq]` (the single source of
/// truth) to this hub keyed by `seq`, so the hub only needs the channel
/// `seq` + a name (the name already encodes sender→receiver via the two
/// worker names). One hub per channel (TASK-0049.05.02) keeps each
/// transport stream on its own byte FIFO.
#[derive(Debug, Clone)]
pub(crate) struct Hub {
    pub name: String,
    /// The channel (`SeqTag.0`) this hub carries — its dedicated FIFO. Both
    /// the `.resc` connect lines and the per-worker `seq_usart` join on it.
    pub seq: u64,
}

/// The whole-schedule transport plan, shared by the shim codegen and the
/// `.resc` generator so the two CANNOT drift.
#[derive(Debug, Clone)]
pub(crate) struct TransportPlan {
    /// Per used worker, in `WorkerId` order.
    pub workers: BTreeMap<WorkerId, WorkerPlan>,
    /// One hub per CHANNEL (`SeqTag`), in ascending `seq` order
    /// (TASK-0049.05.02).
    pub hubs: Vec<Hub>,
    /// Receivers-first release order for the `.resc` start-gating.
    pub boot_order: Vec<WorkerId>,
    /// The deterministic per-saver output-capture list, ordered by the
    /// drained symbol's position in [`NameSidecar::data_decl_order`] (NOT
    /// alphabetical `DataId`) so the captured files concatenate in the
    /// reference output layout. For ex14 this is `[spk_out(fe), bt_out(rf)]`
    /// (decl order: `spk_out` line 80 before `bt_out` line 81), matching
    /// `reference.bin` = spk_out@0 ++ bt_out@256 — NOT the DataId reversal
    /// (`bt_out`=DataId(1) < `spk_out`=DataId(4)). Slice D's recipe injects
    /// `$<file_var>` per entry and concatenates IN THIS ORDER before the
    /// byte-exact diff (TASK-0049.10.05, BLOCKER 3 slice C2).
    pub output_captures: Vec<OutputCapture>,
}

impl TransportPlan {
    /// Build the plan from the per-worker event lists. Fails loud
    /// (`EmitError`) on a worker with more distinct peers than the USART
    /// pool can serve, or a worker missing from `NameTables` — never
    /// silently truncates.
    pub(crate) fn build(
        per_worker: &BTreeMap<WorkerId, Vec<Event>>,
        names: &NameTables,
        sidecar: &NameSidecar,
    ) -> Result<TransportPlan, EmitError> {
        let used: Vec<WorkerId> = per_worker
            .iter()
            .filter(|(_, evs)| !evs.is_empty())
            .map(|(w, _)| *w)
            .collect();

        // Per-worker input base offset into the global `input.bin` layout
        // (TASK-0049.10.04). Computed up front from ALL workers' loaded
        // symbols so the cumulative byte offsets are globally consistent.
        let input_offsets = compute_input_offsets(&used, per_worker, sidecar)?;

        let mut workers: BTreeMap<WorkerId, WorkerPlan> = BTreeMap::new();
        for w in &used {
            let evs = per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]);
            let name = names.worker.get(w).cloned().ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "worker id {w:?} has events but no name in NameTables; the \
                     embedded multi-MCU bin names per-worker machines + \
                     directories from NameTables (TASK-0049.05)"
                ))
            })?;

            // Distinct CHANNELS (seqs), in ascending seq order, each get
            // their OWN USART slot (TASK-0049.05.02). A per-seq (not
            // per-peer) assignment is the ONLY correct unit: two channels in
            // the SAME direction between one worker pair (ex14 `fe`→`dsp`
            // seq0 + seq2) MUST land on distinct byte FIFOs or they
            // interleave on the receiver's single RX FIFO and the streams
            // cross (the ex14 deadlock — rf starved at frame 3).
            let mut seqs: BTreeSet<u64> = BTreeSet::new();
            collect_seqs(evs, &mut seqs);
            if seqs.len() > TRANSPORT_USARTS.len() {
                return Err(EmitError::UnsupportedFeature(format!(
                    "worker `{name}` uses {} distinct cross-MCU transport \
                     channels (seqs) but the STM32H743 platform models only \
                     {} transport USARTs (usart1 is reserved for the \
                     output-capture stream). Each channel needs its OWN USART \
                     so same-direction multi-seq cannot cross on a shared byte \
                     FIFO (TASK-0049.05.02); scaling past 8 channels needs \
                     per-seq framing on a shared FIFO (TASK-0049.05.02 \
                     follow-up).",
                    seqs.len(),
                    TRANSPORT_USARTS.len()
                )));
            }
            let mut seq_usart: BTreeMap<u64, UsartSlot> = BTreeMap::new();
            for (i, s) in seqs.iter().enumerate() {
                seq_usart.insert(*s, TRANSPORT_USARTS[i]);
            }

            // OUTPUT-capture symbols this worker drains (TASK-0049.10.05).
            // `saves_output` is derived from this so the two cannot drift.
            let mut saved_outputs: Vec<DataId> = Vec::new();
            collect_saved_symbols(evs, &mut saved_outputs);

            workers.insert(
                *w,
                WorkerPlan {
                    worker: *w,
                    name,
                    seq_usart,
                    loads_input: has_effectful_load(evs, sidecar)?,
                    saves_output: !saved_outputs.is_empty(),
                    saved_outputs,
                    input_base_offset: input_offsets.get(w).copied().unwrap_or(0),
                },
            );
        }

        // Deterministic per-saver output-capture list (ordered by
        // data_decl_order) + the single-/multi-saver file-var asymmetry,
        // computed once the full worker set is known (TASK-0049.10.05). This
        // is the SINGLE SOURCE OF TRUTH for the capture file vars — the
        // `.resc` generator reads it directly.
        let output_captures = compute_output_capture(&workers, sidecar)?;

        // One hub per CHANNEL (seq), wiring its Push sender to its Wait
        // receiver (TASK-0049.05.02). Each endpoint Connects its own
        // `seq_usart[seq]` (they need not be the same index — `connector
        // Connect` joins both to the hub). Endpoints are read GLOBALLY: a
        // `SeqTag` is unique program-wide (event.rs: single monotonic
        // counter) with exactly one Push sender and one Wait receiver, so
        // each seq maps to exactly one (sender, receiver) pair.
        let mut seq_endpoints: BTreeMap<u64, SeqEndpoints> = BTreeMap::new();
        for w in &used {
            let evs = per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]);
            collect_seq_endpoints(*w, evs, &mut seq_endpoints)?;
        }
        let mut hubs: Vec<Hub> = Vec::new();
        for (seq, ep) in &seq_endpoints {
            // Fail loud on a one-sided channel: a Push with no matching Wait
            // (or vice versa) would deadlock the receiver / drop the sender's
            // bytes — never silently emit it.
            let sender = ep.sender.ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "transport channel seq {seq} has a Wait (receiver) but no \
                     matching Push (sender) — one-sided channel \
                     (TASK-0049.05.02)"
                ))
            })?;
            let receiver = ep.receiver.ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "transport channel seq {seq} has a Push (sender) but no \
                     matching Wait (receiver) — one-sided channel \
                     (TASK-0049.05.02)"
                ))
            })?;
            let na = workers.get(&sender).map(|p| p.name.as_str()).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "transport channel seq {seq} sender {sender:?} has no \
                     WorkerPlan/name (TASK-0049.05.02)"
                ))
            })?;
            let nb = workers.get(&receiver).map(|p| p.name.as_str()).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "transport channel seq {seq} receiver {receiver:?} has no \
                     WorkerPlan/name (TASK-0049.05.02)"
                ))
            })?;
            hubs.push(Hub {
                name: format!("link_{na}_{nb}_s{seq}"),
                seq: *seq,
            });
        }

        let boot_order = compute_boot_order(per_worker, &used);

        Ok(TransportPlan {
            workers,
            hubs,
            boot_order,
            output_captures,
        })
    }
}

/// Collect the distinct CHANNELS (`SeqTag.0`) a worker participates in (as
/// a Push sender or a Wait receiver), recursing into loop bodies. Each
/// distinct seq gets its OWN USART/hub (TASK-0049.05.02) so same-direction
/// multi-seq cannot cross on a shared byte FIFO.
fn collect_seqs(events: &[Event], out: &mut BTreeSet<u64>) {
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
struct SeqEndpoints {
    sender: Option<WorkerId>,
    receiver: Option<WorkerId>,
}

/// Record worker `w`'s Push/Wait events into the global per-seq endpoint
/// map, walking the STATIC event tree once (recursing into loop bodies).
/// Fails loud if a seq gets TWO distinct senders or TWO distinct receivers
/// (a `SeqTag` must have exactly one Push and one Wait — see `event.rs`).
/// If the same seq appears more than once in ONE worker's static event
/// list (e.g. a Push inside a loop body), the re-assertion is idempotent
/// because `prev == w` (same worker) — only a DIFFERENT worker claiming the
/// same role trips the guard.
fn collect_seq_endpoints(
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
/// `NameSidecar`, and a missing [`KernelSig`](nucleus_compiler::sidecar::KernelSig)
/// fails loud (`ContractGap`) rather than silently mis-classifying.
fn has_effectful_load(events: &[Event], sidecar: &NameSidecar) -> Result<bool, EmitError> {
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
/// Mirrors [`collect_loaded_symbols`] (the INPUT-side sibling) — same
/// recursion, same `!out.contains` dedup, same encounter-order contract
/// (the caller sorts by `data_decl_order` for the global layout).
fn collect_saved_symbols(events: &[Event], out: &mut Vec<DataId>) {
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
/// (`ContractGap`), mirroring slice A's `kernel_is_effectful`.
fn is_effectful_load(
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
        Some(_) => kernel_is_effectful(kernel, sidecar),
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
fn is_effectful_io(
    kernel: KernelId,
    bindings: &FireBinding,
    sidecar: &NameSidecar,
) -> Result<bool, EmitError> {
    Ok(bindings.output.is_none() || is_effectful_load(kernel, bindings, sidecar)?)
}

/// Is `kernel` declared `effectful`? Read from the codegen-contract
/// `NameSidecar`'s `KernelSig.purity` (mirrored from `ResolvedKernel::purity`
/// in `build_sidecar`; TASK-0049.10.01). A missing sig is a contract gap —
/// fail loud with context rather than silently defaulting to `Pure` and
/// mis-classifying a load/IO. Structural sibling of `render.rs`'s
/// `kernel_is_effectful`.
fn kernel_is_effectful(kernel: KernelId, sidecar: &NameSidecar) -> Result<bool, EmitError> {
    let sig = sidecar.kernel_sig(kernel).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "kernel id {kernel:?} has no KernelSig in the NameSidecar; cannot \
             determine purity for multi-MCU effectful-IO classification \
             (TASK-0049.10.04)"
        ))
    })?;
    Ok(sig.purity == Purity::Effectful)
}

/// Compute each loader worker's byte offset into the GLOBAL `input.bin`
/// layout (TASK-0049.10.04 BLOCKER 2 + TASK-0049.10.06; Mechanism A =
/// partition in codegen, NOT in the recipe).
///
/// # The global layout
///
/// The `.resc` injects the WHOLE `input.bin` at axiSram into EVERY loader
/// (unchanged). Each loader's shim cursor must START at the byte offset of
/// ITS slice so it reads its own bytes. The byte order of that global layout
/// is defined by the REFERENCE GENERATOR's HAND-WRITTEN `input.bin` (ex14
/// `reference/src/main.rs`: the `mic` block first, then the `bt` block — i.e.
/// data-DECLARATION order), so the offsets here are computed by ordering the
/// LOADED input symbols by their position in
/// [`NameSidecar::data_decl_order`] (TASK-0049.10.06 threaded declaration
/// order into the codegen contract). `DataId` order is NOT used: `DataId` is
/// assigned ALPHABETICALLY (`acfg::build`), which for ex14 would put
/// `bt_in`=DataId(0) before `mic_in`=DataId(2) — the REVERSE of the
/// reference layout.
///
/// # Computation
///
/// Only the symbols that are actually loaded participate in the layout.
/// Each loaded symbol's byte size = element count
/// ([`NameSidecar::alloc_len`], the product of its `dims`) × the FIXED
/// element byte width ([`ScalarType::fixed_byte_width`]; i32 ⇒ 4, NOT
/// hardcoded). The cumulative byte offset of a symbol is the sum of the byte
/// sizes of all loaded symbols that PRECEDE it in declaration order. A
/// worker's base offset is the offset of its FIRST loaded symbol.
///
/// # Cases
///
/// - **≤1 loader worker (e.g. 02-split-add `host` loads BOTH `a` and `b`).**
///   The single loader's cursor starts at 0 and reads its symbols
///   sequentially in firing order; there is no cross-worker ordering to get
///   wrong. Early-returns offset 0 — byte-identical to
///   pre-TASK-0049.10.04, and `just renode-multimcu 02-split-add` proves it
///   byte-exact.
///
/// - **≥2 distinct loader workers (ex14 `fe`→`mic_in`, `rf`→`bt_in`).** The
///   cross-worker input partition. Each worker's base offset is the
///   declaration-order cumulative offset of its first loaded symbol — for
///   ex14, `fe`(mic_in)=0, `rf`(bt_in)=256.
///
/// # Fail-loud cases (typed [`EmitError`], never a wrong offset)
///
/// - A loaded symbol absent from `data_decl_order` (contract desync).
/// - A loaded symbol with a platform-dependent scalar width
///   (`usize`/`isize`), whose on-target byte width is ambiguous for a
///   byte-exact layout.
/// - A worker whose loaded symbols are NON-CONTIGUOUS in the global
///   declaration-order layout (some other worker's symbol falls between two
///   of this worker's) — this slice models a worker as one contiguous slice,
///   so a non-contiguous load is a shape it cannot emit a single base offset
///   for.
pub(crate) fn compute_input_offsets(
    used: &[WorkerId],
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    sidecar: &NameSidecar,
) -> Result<BTreeMap<WorkerId, usize>, EmitError> {
    // Which symbol(s) does each loader worker load? (effectful-load Fire
    // output datum(s), in encounter order within the worker).
    let mut per_worker_syms: BTreeMap<WorkerId, Vec<DataId>> = BTreeMap::new();
    for w in used {
        let evs = per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]);
        let mut syms: Vec<DataId> = Vec::new();
        collect_loaded_symbols(evs, sidecar, &mut syms)?;
        if !syms.is_empty() {
            per_worker_syms.insert(*w, syms);
        }
    }

    // ≤1 loader worker: the sound case. Its offset is 0 (it reads the whole
    // injected input from the front, in firing order). 02-split-add `host`
    // is here — byte-identical to pre-TASK-0049.10.04.
    if per_worker_syms.len() <= 1 {
        return Ok(per_worker_syms.keys().map(|w| (*w, 0usize)).collect());
    }

    // ≥2 distinct loader workers: the cross-worker input partition. Build the
    // GLOBAL input.bin layout = the LOADED input symbols ordered by their
    // position in the reference generator's declaration-order layout
    // (sidecar.data_decl_order). Accumulate the per-symbol byte offset, then
    // read each worker's base off its FIRST loaded symbol.

    // The set of symbols actually loaded (any worker), so the layout only
    // includes participating symbols (unloaded data — e.g. ex14 `mixed`,
    // outputs — does NOT appear in input.bin).
    let loaded_syms: BTreeSet<DataId> = per_worker_syms
        .values()
        .flat_map(|v| v.iter().copied())
        .collect();

    // Cumulative byte offset of each loaded symbol, walking declaration order.
    let mut offset_of: BTreeMap<DataId, usize> = BTreeMap::new();
    let mut cursor: usize = 0usize;
    for did in &sidecar.data_decl_order {
        if !loaded_syms.contains(did) {
            continue;
        }
        offset_of.insert(*did, cursor);
        cursor = cursor
            .checked_add(symbol_byte_size(*did, sidecar)?)
            .ok_or_else(|| {
                EmitError::UnsupportedFeature(format!(
                    "input.bin byte layout overflowed usize accumulating \
                     symbol {did:?} (TASK-0049.10.06)"
                ))
            })?;
    }

    // Every loaded symbol MUST appear in declaration order (else the layout
    // is incomplete and offsets would be wrong). A miss means the contract's
    // data_decl_order does not cover a loaded symbol — fail loud.
    for did in &loaded_syms {
        if !offset_of.contains_key(did) {
            return Err(EmitError::UnsupportedFeature(format!(
                "loaded input symbol {did:?} is absent from \
                 NameSidecar.data_decl_order; the global input.bin byte \
                 layout cannot be computed (decl-order contract desync, \
                 TASK-0049.10.06)"
            )));
        }
    }

    // Per worker: base = offset of its FIRST loaded symbol (declaration-order
    // position). Verify the worker's loaded symbols are CONTIGUOUS in the
    // global layout — this slice models a worker as one contiguous input
    // slice. Non-contiguous (another worker's symbol interleaved) is a shape
    // we cannot represent with a single base offset; fail loud.
    let mut out: BTreeMap<WorkerId, usize> = BTreeMap::new();
    for (w, syms) in &per_worker_syms {
        // Order this worker's symbols by their declaration-order offset.
        let mut owned: Vec<(usize, usize)> = syms
            .iter()
            .map(|did| (offset_of[did], symbol_byte_size(*did, sidecar)))
            .map(|(off, sz)| Ok::<_, EmitError>((off, sz?)))
            .collect::<Result<_, _>>()?;
        owned.sort_by_key(|(off, _)| *off);

        // Contiguity: each symbol's offset must equal the running end of the
        // previous one.
        let base = owned[0].0;
        let mut end = base;
        for (off, sz) in &owned {
            if *off != end {
                return Err(EmitError::UnsupportedFeature(format!(
                    "worker {w:?} loads input symbols that are NON-CONTIGUOUS \
                     in the global declaration-order input.bin layout \
                     (expected next symbol at byte {end}, found one at byte \
                     {off}); a single per-worker base offset cannot describe a \
                     non-contiguous slice. This multi-loader interleaving \
                     shape is out of scope for TASK-0049.10.06."
                )));
            }
            end += sz;
        }
        out.insert(*w, base);
    }
    Ok(out)
}

/// Byte size of one data symbol's full allocation in the global
/// `input.bin` layout: element count × fixed element byte width
/// (TASK-0049.10.06). Fails loud (typed [`EmitError`]) if the symbol is
/// absent from the sidecar or has a platform-dependent scalar width
/// (`usize`/`isize`), whose on-target byte size is ambiguous for a
/// byte-exact layout.
fn symbol_byte_size(did: DataId, sidecar: &NameSidecar) -> Result<usize, EmitError> {
    let ty = sidecar.data_type(did).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "input symbol {did:?} has no ResolvedType in NameSidecar.data_types; \
             cannot size its input.bin block (TASK-0049.10.06)"
        ))
    })?;
    let elems = sidecar.alloc_len(did).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "input symbol {did:?} has no alloc_len in NameSidecar; cannot size \
             its input.bin block (TASK-0049.10.06)"
        ))
    })?;
    let width = ty.scalar.fixed_byte_width().ok_or_else(|| {
        EmitError::UnsupportedFeature(format!(
            "input symbol {did:?} has scalar type {:?}, whose byte width is \
             platform-dependent (usize/isize differ between 64-bit host and \
             32-bit embedded target); a byte-exact input.bin layout needs a \
             fixed width. Out of scope for TASK-0049.10.06.",
            ty.scalar
        ))
    })?;
    elems.checked_mul(width).ok_or_else(|| {
        EmitError::UnsupportedFeature(format!(
            "input symbol {did:?} byte size overflowed usize ({elems} elems × \
             {width} bytes) (TASK-0049.10.06)"
        ))
    })
}

/// Collect the input symbol(s) a worker LOADS — the output datum of each
/// effectful-load `Fire`, recursing into loop bodies. Order = encounter
/// order (the caller sorts by `DataId` for the global layout).
fn collect_loaded_symbols(
    events: &[Event],
    sidecar: &NameSidecar,
    out: &mut Vec<DataId>,
) -> Result<(), EmitError> {
    for e in events {
        match e {
            Event::Fire {
                kernel, bindings, ..
            } => {
                if is_effectful_load(*kernel, bindings, sidecar)? {
                    if let Some(o) = &bindings.output {
                        if !out.contains(&o.data) {
                            out.push(o.data);
                        }
                    }
                }
            }
            Event::Loop { body, .. } => collect_loaded_symbols(body, sidecar, out)?,
            _ => {}
        }
    }
    Ok(())
}

/// Compute the deterministic per-saver OUTPUT-capture list, ordered by the
/// drained symbol's position in [`NameSidecar::data_decl_order`] (NOT
/// alphabetical `DataId`) — the OUTPUT-side mirror of [`compute_input_offsets`]
/// (TASK-0049.10.05, BLOCKER 3 slice C2).
///
/// # Why declaration order, not `DataId`
///
/// The reference output layout (ex14 `reference/src/main.rs`) is
/// `spk_out` (256 B) ++ `bt_out` (256 B) — DECLARATION order (`spk_out` line
/// 80 before `bt_out` line 81). But `DataId` is assigned ALPHABETICALLY
/// (`acfg::build`): `bt_out`=DataId(1) < `spk_out`=DataId(4) — the REVERSE.
/// So a capture order derived from `DataId` would be backwards; this orders
/// by `data_decl_order` index, exactly as the INPUT side does, so slice D
/// concatenates the per-saver files into `reference.bin` order.
///
/// # File-var asymmetry (LOAD-BEARING — see [`OutputCapture::file_var`])
///
/// * **Exactly one saver** (02-split-add `host`): var = `uartFile`,
///   byte-identical to the pre-TASK-0049.10.05 `.resc` so the
///   `just renode-multimcu` recipe (which injects/reads `$uartFile`) keeps
///   passing. The single-saver case is regression-pinned by the renode
///   byte-exact gate.
/// * **≥2 savers** (ex14 `fe`+`rf`): each saver gets a DISTINCT var
///   `<sanitized-worker>Uart` (`feUart`, `rfUart`) so the two no longer
///   collide on a shared `$uartFile`. Slice D extends the recipe to inject +
///   concatenate these.
///
/// # Fail-loud (typed [`EmitError`], never a wrong order)
///
/// A saved symbol absent from `data_decl_order` is a contract desync (mirrors
/// the analogous INPUT case in [`compute_input_offsets`]) — fail loud rather
/// than emit an unordered/partial capture list.
fn compute_output_capture(
    workers: &BTreeMap<WorkerId, WorkerPlan>,
    sidecar: &NameSidecar,
) -> Result<Vec<OutputCapture>, EmitError> {
    // (worker, drained DataId) for every saver, in WorkerId then encounter
    // order. ex14 each saver drains exactly one; a (hypothetical) worker
    // draining several contributes one entry per distinct saved symbol.
    let mut saver_syms: Vec<(WorkerId, DataId)> = Vec::new();
    for (w, wp) in workers {
        for did in &wp.saved_outputs {
            saver_syms.push((*w, *did));
        }
    }
    if saver_syms.is_empty() {
        return Ok(Vec::new());
    }

    // Single-saver vs multi-saver var asymmetry (recipe-compat). "Saver" is
    // counted by DISTINCT worker, not by drained symbol: a lone saver
    // draining two symbols still writes one USART1 -> `$uartFile`.
    let distinct_savers: BTreeSet<WorkerId> = saver_syms.iter().map(|(w, _)| *w).collect();
    let single_saver = distinct_savers.len() == 1;

    // Order the saved symbols by their declaration-order index. A symbol
    // absent from `data_decl_order` is a contract desync — fail loud.
    let decl_index = |did: DataId| -> Result<usize, EmitError> {
        sidecar
            .data_decl_order
            .iter()
            .position(|d| *d == did)
            .ok_or_else(|| {
                EmitError::UnsupportedFeature(format!(
                    "saved output symbol {did:?} is absent from \
                     NameSidecar.data_decl_order; the per-saver output-capture \
                     order cannot be computed (decl-order contract desync, \
                     TASK-0049.10.05)"
                ))
            })
    };

    // Sort by decl-order index (tie-break on WorkerId for determinism, though
    // distinct savers drain distinct symbols in the schedules we model).
    let mut keyed: Vec<(usize, WorkerId, DataId)> = saver_syms
        .iter()
        .map(|(w, did)| Ok::<_, EmitError>((decl_index(*did)?, *w, *did)))
        .collect::<Result<_, _>>()?;
    keyed.sort_by(|(ia, wa, _), (ib, wb, _)| ia.cmp(ib).then(wa.cmp(wb)));

    let mut out: Vec<OutputCapture> = Vec::with_capacity(keyed.len());
    for (_, w, did) in keyed {
        let file_var = if single_saver {
            "uartFile".to_string()
        } else {
            format!("{}Uart", sanitize_var(&workers[&w].name))
        };
        out.push(OutputCapture {
            worker: w,
            data: did,
            file_var,
        });
    }
    Ok(out)
}

/// Sanitise a worker name into a Renode/monitor variable token: keep ASCII
/// alphanumerics, map every other char to `_`. The `.resc` var is referenced
/// as `$<name>Uart`; worker names in this repo are already simple tokens
/// (`fe`, `rf`, `host`, `w0`) so this is a defensive normalisation, mirroring
/// how the recipe derives `$<worker>Bin` from worker directory names.
fn sanitize_var(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

// ===================================================================
// TASK-0049.05.01 — fail-loud guard for a control-only `Event::Sync`
// whose ordering is NOT subsumed by the data edges.
// ===================================================================

/// One salient event in a worker's flattened (loop-bodies inlined) event
/// stream, for the control-barrier subsumption analysis. Pure-compute
/// `Fire`s, `Alloc`/`Free` carry no cross-MCU ordering and are dropped.
#[derive(Debug, Clone, Copy)]
enum Salient {
    /// Effectful load/save — a globally-observable external IO side effect.
    Io,
    /// Outgoing inter-MCU transport (the tail of a data edge).
    Push { seq: u64 },
    /// Incoming inter-MCU transport (the head of a data edge).
    Wait { seq: u64 },
    /// A control-only cross-worker barrier (`irq_barrier`, lowered no-op).
    Sync { tag: u64 },
}

/// Flatten one worker's event tree into its ordered salient-event stream,
/// inlining `Loop` bodies in place. In-loop barriers/IO therefore appear
/// at the loop's position (a single linearisation), which is the
/// conservative reading the guard below relies on.
fn flatten_salients(
    events: &[Event],
    sidecar: &NameSidecar,
    out: &mut Vec<Salient>,
) -> Result<(), EmitError> {
    for e in events {
        match e {
            Event::Fire {
                kernel, bindings, ..
            } => {
                if is_effectful_io(*kernel, bindings, sidecar)? {
                    out.push(Salient::Io);
                }
            }
            Event::Push { seq, .. } => out.push(Salient::Push { seq: seq.0 }),
            Event::Wait { seq, .. } => out.push(Salient::Wait { seq: seq.0 }),
            Event::Sync { sync, .. } => out.push(Salient::Sync { tag: sync.0 }),
            Event::Loop { body, .. } => flatten_salients(body, sidecar, out)?,
            Event::Alloc { .. } | Event::Free { .. } => {}
        }
    }
    Ok(())
}

/// Fail loud (`EmitError`) when a control-only `Event::Sync` would be
/// SILENTLY MISCOMPILED by the multi-MCU `MultiMcuShim`'s no-op
/// `irq_barrier` (skeleton/multimcu.rs).
///
/// ## Why a no-op is *usually* correct, and when it is not
///
/// In the multi-MCU model the co-simulated MCUs share NO memory; the ONLY
/// inter-MCU channel is the BLOCKING `link_push`/`link_recv` transport
/// (`Event::Push`/`Wait`), which self-orders (a receive cannot complete
/// before its matching send). Every cross-MCU DATA dependency is therefore
/// already carried by a data edge, so dropping the barrier is value-correct
/// for data — and `just renode-multimcu 02-split-add` proves it byte-exact.
///
/// The ONE ordering a barrier can impose that the data edges cannot is the
/// relative order of two DIFFERENT workers' globally-observable EXTERNAL IO
/// side effects (effectful load/save Fires — [`is_effectful_io`]). Inter-MCU
/// transport self-orders and pure compute is unobservable across MCUs, so
/// only true peripheral IO can be barrier-ordered-but-not-data-ordered. If
/// such a straddling IO pair exists with no connecting data edge, the no-op
/// would silently drop a real ordering — there is no transport to implement
/// a standalone barrier, so we reject loud (a real UART barrier protocol is
/// future work, parent TASK-0049.05).
///
/// ## Why this does NOT reject the shipped path
///
/// `02-split-add/split` routes ALL external IO through one worker (`host`
/// loads `a`,`b` and saves `c`; `w0` only receives/computes/pushes — zero
/// effectful IO). With fewer than two IO-bearing participants in any
/// barrier there is no cross-worker IO pair to order, so the guard is inert
/// — exactly the conservative-tripwire shape of
/// `TransferInjectError::CumulativeWholeArrayFallback`.
pub(crate) fn verify_control_sync_subsumed(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<(), EmitError> {
    // Workers in a stable order; index into this vec is a worker's "lane".
    let lanes: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    if lanes.len() < 2 {
        return Ok(()); // single-worker bin carries no cross-worker barrier.
    }

    // Flatten each lane and lay every salient out on one global node index
    // line: node `base[lane] + local` is lane `lane`'s `local`-th salient.
    let mut salients: Vec<Vec<Salient>> = Vec::with_capacity(lanes.len());
    let mut base: Vec<usize> = Vec::with_capacity(lanes.len());
    let mut total = 0usize;
    for w in &lanes {
        let mut s = Vec::new();
        flatten_salients(
            per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]),
            sidecar,
            &mut s,
        )?;
        base.push(total);
        total += s.len();
        salients.push(s);
    }

    // Happens-before adjacency: program order within a lane, plus a
    // Push{seq} -> matching Wait{seq} edge across lanes (a unique pair per
    // `SeqTag`). `node(l, i)` is the global id of lane `l`'s `i`-th salient.
    let node = |l: usize, i: usize| base[l] + i;
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut push_of: BTreeMap<u64, usize> = BTreeMap::new();
    let mut wait_of: BTreeMap<u64, usize> = BTreeMap::new();
    for (l, lane) in salients.iter().enumerate() {
        for (i, s) in lane.iter().enumerate() {
            if i + 1 < lane.len() {
                adj[node(l, i)].push(node(l, i + 1)); // program order
            }
            match s {
                Salient::Push { seq } => {
                    // A `SeqTag` is unique per (src,dst,data) and loop bodies
                    // inline once, so each seq has exactly one Push in the
                    // flattened stream. If a future lowering ever flattened a
                    // seq twice, last-write-wins here would SILENTLY drop the
                    // earlier edge from the HB graph — an under-approximation
                    // (false-accept). Fail loud in dev rather than miscompile.
                    debug_assert!(
                        !push_of.contains_key(seq),
                        "duplicate Push SeqTag {seq} in the flattened stream"
                    );
                    push_of.insert(*seq, node(l, i));
                }
                Salient::Wait { seq } => {
                    debug_assert!(
                        !wait_of.contains_key(seq),
                        "duplicate Wait SeqTag {seq} in the flattened stream"
                    );
                    wait_of.insert(*seq, node(l, i));
                }
                _ => {}
            }
        }
    }
    for (seq, p) in &push_of {
        if let Some(w) = wait_of.get(seq) {
            adj[*p].push(*w); // data edge: send happens-before receive
        }
    }

    // Reachability (small graphs; recompute per query rather than build the
    // full transitive closure).
    let reaches = |from: usize, to: usize| -> bool {
        if from == to {
            return true;
        }
        let mut seen = vec![false; total];
        let mut stack = vec![from];
        seen[from] = true;
        while let Some(n) = stack.pop() {
            for &m in &adj[n] {
                if m == to {
                    return true;
                }
                if !seen[m] {
                    seen[m] = true;
                    stack.push(m);
                }
            }
        }
        false
    };

    // Group the barriers by tag and the lanes that carry them.
    let mut tags: BTreeSet<u64> = BTreeSet::new();
    for lane in &salients {
        for s in lane {
            if let Salient::Sync { tag } = s {
                tags.insert(*tag);
            }
        }
    }

    let name_of =
        |w: WorkerId| names.worker.get(&w).cloned().unwrap_or_else(|| format!("{w:?}"));

    for tag in tags {
        // For each participating lane, the span [first, last] of this tag's
        // barrier instances, and its IO node indices.
        let mut first_sync: BTreeMap<usize, usize> = BTreeMap::new();
        let mut last_sync: BTreeMap<usize, usize> = BTreeMap::new();
        let mut io_idx: Vec<Vec<usize>> = vec![Vec::new(); lanes.len()];
        for (l, lane) in salients.iter().enumerate() {
            for (i, s) in lane.iter().enumerate() {
                match s {
                    Salient::Sync { tag: t } if *t == tag => {
                        first_sync.entry(l).or_insert(i);
                        last_sync.insert(l, i);
                    }
                    Salient::Io => io_idx[l].push(i),
                    _ => {}
                }
            }
        }
        // Participants are inferred from which lanes carry this tag's
        // `Salient::Sync` — equivalent to `Event::Sync.participants` because
        // `petri_to_events` emits one `Event::Sync` into EVERY declared
        // participant's stream (1:1), so lane-presence == the declared set.
        let participants: Vec<usize> = first_sync.keys().copied().collect();
        if participants.len() < 2 {
            continue; // a single-lane (vacuous) barrier orders nothing.
        }
        // Each `SyncTag` appears AT MOST ONCE per worker: a barrier is one
        // program point and `flatten_salients` inlines loop bodies once, so
        // an in-loop barrier is not duplicated. Hence first==last per lane and
        // "before the first instance" / "after the last instance" is simply
        // "before / after the barrier". If a future lowering ever emitted the
        // SAME tag twice in one worker, IO BETWEEN the instances would be
        // dropped from both sets — an UNDER-approximation (false-ACCEPT, the
        // unsafe direction). Assert the invariant so that regresses loudly
        // rather than silently miscompiling.
        debug_assert!(
            first_sync == last_sync,
            "control-sync guard assumes each SyncTag occurs once per worker \
             (loop bodies inline once); tag {tag} repeats in some worker, which \
             would make the IO-straddle test under-approximate (false-accept)"
        );
        // IO straddling the barrier, per lane: strictly before its (sole)
        // instance on `wi` / strictly after its (sole) instance on `wj`.
        for &wi in &participants {
            let fi = first_sync[&wi];
            let before: Vec<usize> = io_idx[wi].iter().copied().filter(|&i| i < fi).collect();
            if before.is_empty() {
                continue;
            }
            for &wj in &participants {
                if wi == wj {
                    continue;
                }
                let lj = last_sync[&wj];
                for &aj in io_idx[wj].iter().filter(|&&i| i > lj) {
                    for &bi in &before {
                        if !reaches(node(wi, bi), node(wj, aj)) {
                            return Err(EmitError::UnsupportedFeature(format!(
                                "embedded-pattern multi-MCU BIN: control-only barrier \
                                 (sync tag {tag}) orders an external IO side effect on \
                                 worker `{wi_n}` (before the barrier) before one on \
                                 worker `{wj_n}` (after the barrier), but NO data edge \
                                 (link_push/link_recv) carries that ordering. The \
                                 `MultiMcuShim` lowers `irq_barrier` to a NO-OP, which \
                                 would SILENTLY drop this ordering. A standalone control \
                                 barrier needs a real inter-MCU UART barrier protocol \
                                 (TASK-0049.05.01 / parent TASK-0049.05); the no-op is \
                                 value-correct only when every barrier ordering is \
                                 subsumed by a blocking data edge (as in 02-split-add).",
                                wi_n = name_of(lanes[wi]),
                                wj_n = name_of(lanes[wj]),
                            )));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// axiSram base address the input fixture is injected at (matches the
/// single-machine `tests/renode/embedded/run.resc` + the shim's
/// `NUC_INPUT_REGION`). Mapped in the platform, NOT in `memory.x`, so the
/// stack/.bss never collide with the injected bytes.
const AXISRAM_BASE: &str = "0x24000000";

/// Render the multi-machine Renode `.resc` that co-simulates one machine
/// per worker, wires them on the plan's `UARTHub`s, and start-gates the
/// boot so receivers enable RX before senders transmit (TASK-0049.05).
///
/// The caller (the `renode-multimcu` just recipe) injects, before
/// `include`-ing this script:
///   * `$<worker>Bin` = @<path to that worker's cross-compiled ELF>, one
///     per used worker (e.g. `$hostBin`, `$w0Bin`);
///   * the OUTPUT-CAPTURE file var(s) the saver USART1(s) write to
///     (TASK-0049.10.05): with EXACTLY ONE saver this is the single
///     `$uartFile` (byte-identical to pre-C2; the `renode-multimcu` recipe
///     injects/reads it for 02-split-add `host`); with ≥2 savers each saver
///     gets a DISTINCT `$<worker>Uart` (ex14 `$feUart`, `$rfUart`) — the
///     exact vars + their reference-layout concatenation order are
///     [`TransportPlan::output_captures`] (decl-order, slice D concatenates
///     in that order);
///   * `$input`        = @<path to the input.bin injected into loaders>.
pub(crate) fn render_multimachine_resc(plan: &TransportPlan) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let worker_names: Vec<&str> = plan.workers.values().map(|w| w.name.as_str()).collect();

    writeln!(s, ":name: nucleus-m11-multimcu").ok();
    writeln!(
        s,
        ":description: Co-simulate {} STM32H7 MCUs ({}) wired by UARTHub(s); \
         generated by the embedded-pattern backend (TASK-0049.05).",
        worker_names.len(),
        worker_names.join(", ")
    )
    .ok();
    s.push('\n');
    s.push_str(
        "# GENERATED multi-machine Renode script (embedded-pattern, M11 BIN).\n\
         # Do not edit; rerun `nucleus build --shim stm32h7` on a multi-worker\n\
         # schedule to regenerate. One machine per worker, wired by a UARTHub\n\
         # per CHANNEL (SeqTag) (Renode's UARTHub is a broadcast bus; a\n\
         # dedicated hub per channel keeps every transport stream on its own\n\
         # byte FIFO, so two same-direction channels between one worker pair\n\
         # never cross — TASK-0049.05.02).\n\
         #\n\
         # START-GATING (load-bearing, TASK-0049.01): Renode's\n\
         # UARTBase.WriteChar DROPS bytes that arrive before the receiver\n\
         # enables RX. Each firmware enables RX on all its link USARTs as its\n\
         # FIRST action; this script releases the machines RECEIVERS-FIRST so\n\
         # every receiver is RX-enabled before any peer transmits to it.\n\n",
    );

    // 1. One UARTHub per channel (seq).
    for h in &plan.hubs {
        writeln!(s, "emulation CreateUARTHub \"{}\"", h.name).ok();
    }
    s.push('\n');

    // 2. One machine per worker: platform + hub connects + capture/inject.
    for plan_w in plan.workers.values() {
        writeln!(s, "mach create \"{}\"", plan_w.name).ok();
        s.push_str("machine LoadPlatformDescription @platforms/cpus/stm32h743.repl\n");
        // Connect this worker's USART for each CHANNEL (seq) to that
        // channel's dedicated hub (TASK-0049.05.02). `seq_usart` is injective
        // so each connect targets a distinct USART + distinct hub.
        for (seq, usart) in &plan_w.seq_usart {
            if let Some(h) = plan.hubs.iter().find(|h| h.seq == *seq) {
                writeln!(s, "connector Connect {} {}", usart.renode_name, h.name).ok();
            }
        }
        // Effectful-output capture (TASK-0049.10.05). Driven off the plan's
        // `output_captures` (the SINGLE SOURCE OF TRUTH for the per-saver
        // file var + drained symbol + decl-order) so the `.resc`, the shim,
        // and slice D's concat order cannot drift. Single saver -> `$uartFile`
        // (recipe-compatible, byte-identical to pre-C2); multi-saver -> each
        // saver's DISTINCT `$<worker>Uart` so they no longer collide on one
        // shared file.
        for cap in plan.output_captures.iter().filter(|c| c.worker == plan_w.worker) {
            writeln!(
                s,
                "# This worker saves output `{:?}` -> capture its USART1 to the \
                 file backend (slice D concatenates in output_captures order).",
                cap.data,
            )
            .ok();
            writeln!(s, "usart1 CreateFileBackend ${} true", cap.file_var).ok();
        }
        if plan_w.loads_input {
            writeln!(
                s,
                "# This worker loads input -> inject the fixture into axiSram @ {AXISRAM_BASE}.",
            )
            .ok();
            writeln!(s, "sysbus LoadBinary $input {AXISRAM_BASE}").ok();
        }
        s.push('\n');
    }

    // 3. Fine global quantum so the machines interleave finely (mirrors the
    //    bundled scripts/multi-node/*.resc multi-MCU UART pattern).
    s.push_str("emulation SetGlobalQuantum \"0.00001\"\n\n");

    // 4. Load every ELF (caller injects $<worker>Bin per worker).
    for plan_w in plan.workers.values() {
        writeln!(s, "mach set \"{}\"", plan_w.name).ok();
        writeln!(s, "sysbus LoadELF ${}Bin", plan_w.name).ok();
    }
    s.push('\n');

    // 5. Start-gating: halt all, then release RECEIVERS-FIRST with a boot
    //    window between each so RX is enabled before the next sender starts.
    s.push_str(
        "# Hold every machine halted, then release in receivers-first boot\n\
         # order with a boot window between each (RX-enable before TX).\n",
    );
    for plan_w in plan.workers.values() {
        writeln!(s, "mach set \"{}\"", plan_w.name).ok();
        s.push_str("cpu IsHalted true\n");
    }
    s.push('\n');
    for (i, w) in plan.boot_order.iter().enumerate() {
        let name = &plan.workers[w].name;
        writeln!(s, "mach set \"{name}\"").ok();
        s.push_str("cpu IsHalted false\n");
        if i + 1 < plan.boot_order.len() {
            // Boot window: let this (more receiver-ish) machine enable RX
            // and settle before the next machine starts transmitting.
            s.push_str("emulation RunFor \"0.002\"\n");
        } else {
            // Last machine released: run for a FIXED emulated-seconds window.
            // 0.3 s is sufficient for the 02-split-add single-pass case (one
            // host->w0->host hop), NOT a proof of "to completion". A deeper
            // pipeline (e.g. ex14's 4-frame x 3-MCU schedule) needs a larger
            // or save-count-gated window; sizing the window to schedule depth
            // is tracked with the unframed-multiplex transport work
            // (TASK-0049.05.02). This window is bounded + determinate.
            s.push_str("emulation RunFor \"0.3\"\n");
        }
    }
    s.push_str("quit\n");
    s
}

/// Receivers-first boot order: workers that RECEIVE before they first SEND
/// boot earlier, so their RX is enabled before any peer transmits to them
/// (TASK-0049.01 start-gating, generalised from the 2-MCU smoke fixture to
/// a pipeline DAG). The key is `(waits_before_first_push DESC, worker_id
/// ASC)`: a pure sink (no Push) sorts first; an early sender (Push before
/// any Wait) sorts last. LIMIT: this is a HEURISTIC correct for pipeline
/// DAGs; a cyclic interconnect (mutual early sends) has no static
/// receivers-first order and needs a real RX-ready handshake — filed as a
/// follow-up. The generated `.resc`'s byte-exact diff is the empirical check.
fn compute_boot_order(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    used: &[WorkerId],
) -> Vec<WorkerId> {
    let mut ranked: Vec<(usize, WorkerId)> = used
        .iter()
        .map(|w| {
            let evs = per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]);
            (waits_before_first_push(evs), *w)
        })
        .collect();
    // DESC by waits-before-first-push, ASC by worker id (deterministic).
    ranked.sort_by(|x, y| y.0.cmp(&x.0).then(x.1 .0.cmp(&y.1 .0)));
    ranked.into_iter().map(|(_, w)| w).collect()
}

/// Count Wait events that occur before the worker's first Push (flattened,
/// recursing into loops in order). `usize::MAX` if the worker never Pushes
/// (a pure sink — boot it first of all).
fn waits_before_first_push(events: &[Event]) -> usize {
    let mut waits = 0usize;
    let mut saw_push = false;
    fn walk(events: &[Event], waits: &mut usize, saw_push: &mut bool) {
        for e in events {
            if *saw_push {
                return;
            }
            match e {
                Event::Push { .. } => {
                    *saw_push = true;
                    return;
                }
                Event::Wait { .. } => *waits += 1,
                Event::Loop { body, .. } => walk(body, waits, saw_push),
                _ => {}
            }
        }
    }
    walk(events, &mut waits, &mut saw_push);
    if saw_push {
        waits
    } else {
        usize::MAX
    }
}
