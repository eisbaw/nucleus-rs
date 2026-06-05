//! The control-only `Event::Sync` subsumption guard (TASK-0049.05.01) plus
//! the worker-name sanitiser it shares with the output-capture var naming
//! (TASK-0450 split).

use std::collections::{BTreeMap, BTreeSet};

use backend_common::EmitError;
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use super::scan::is_effectful_io;

/// Sanitise a worker name into a Renode/monitor variable token: keep ASCII
/// alphanumerics, map every other char to `_`. The `.resc` var is referenced
/// as `$<name>Uart`; worker names in this repo are already simple tokens
/// (`fe`, `rf`, `host`, `w0`) so this is a defensive normalisation, mirroring
/// how the recipe derives `$<worker>Bin` from worker directory names.
pub(super) fn sanitize_var(name: &str) -> String {
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
