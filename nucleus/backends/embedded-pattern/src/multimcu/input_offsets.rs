//! Global `input.bin` byte-layout offsets per loader worker, and the
//! per-saver output-capture ordering (TASK-0049.10.04/05/06 / TASK-0450
//! split).

use std::collections::{BTreeMap, BTreeSet};

use backend_common::EmitError;
use nucleus_compiler::event::{DataId, Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use super::control_sync::sanitize_var;
use super::plan::{OutputCapture, WorkerPlan};
use super::scan::is_effectful_load;

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
pub(super) fn collect_loaded_symbols(
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
pub(super) fn compute_output_capture(
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
