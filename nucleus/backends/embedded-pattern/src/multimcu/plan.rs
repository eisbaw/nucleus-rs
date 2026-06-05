//! The whole-schedule multi-MCU transport-plan data types and the
//! `TransportPlan::build` constructor (TASK-0049.05 / TASK-0450 split).

use std::collections::{BTreeMap, BTreeSet};

use backend_common::EmitError;
use nucleus_compiler::event::{DataId, Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use super::boot_order::compute_boot_order;
use super::input_offsets::{compute_input_offsets, compute_output_capture};
use super::scan::{
    collect_saved_symbols, collect_seq_endpoints, collect_seqs, has_effectful_load, SeqEndpoints,
};

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
