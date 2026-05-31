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
//!     worker (the shim's `link_push`/`link_recv` seq->USART table);
//!   * one `UARTHub` per worker-PAIR that shares any transport edge
//!     (Renode's UARTHub is a BROADCAST bus — a dedicated hub per pair
//!     keeps point-to-point traffic collision-free, and the 9 USARTs the
//!     stm32h743 platform models give ample fan-out);
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
use nucleus_compiler::event::{Event, FireBinding, WorkerId};
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
/// than eight DISTINCT peers fails loud (a real STM32H743 has exactly
/// these UARTs).
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
    /// `link_push(seq)` and `link_recv(seq)` consult it. (All channels to
    /// the same peer share that peer's USART; opposite directions on one
    /// hub never temporally overlap in the pipeline schedules.)
    pub seq_usart: BTreeMap<u64, UsartSlot>,
    /// Distinct peers -> the USART this worker uses to talk to that peer.
    /// Drives the worker's `connector Connect` lines in the `.resc`.
    pub peer_usart: BTreeMap<WorkerId, UsartSlot>,
    /// This worker performs an effectful load (`a <-- load_input()`) — its
    /// machine gets `$input` injected into axiSram.
    pub loads_input: bool,
    /// This worker performs an effectful save (`save_output(c)`) — its
    /// USART1 is captured to `$uartFile`.
    pub saves_output: bool,
}

/// One inter-MCU `UARTHub`, wiring exactly two workers. The USART each
/// endpoint Connects with is the worker's own `peer_usart[other]` (the
/// single source of truth), so the hub only needs the worker-pair + name.
#[derive(Debug, Clone)]
pub(crate) struct Hub {
    pub name: String,
    pub a: WorkerId,
    pub b: WorkerId,
}

/// The whole-schedule transport plan, shared by the shim codegen and the
/// `.resc` generator so the two CANNOT drift.
#[derive(Debug, Clone)]
pub(crate) struct TransportPlan {
    /// Per used worker, in `WorkerId` order.
    pub workers: BTreeMap<WorkerId, WorkerPlan>,
    /// One hub per worker-pair sharing any edge, in deterministic order.
    pub hubs: Vec<Hub>,
    /// Receivers-first release order for the `.resc` start-gating.
    pub boot_order: Vec<WorkerId>,
}

impl TransportPlan {
    /// Build the plan from the per-worker event lists. Fails loud
    /// (`EmitError`) on a worker with more distinct peers than the USART
    /// pool can serve, or a worker missing from `NameTables` — never
    /// silently truncates.
    pub(crate) fn build(
        per_worker: &BTreeMap<WorkerId, Vec<Event>>,
        names: &NameTables,
    ) -> Result<TransportPlan, EmitError> {
        let used: Vec<WorkerId> = per_worker
            .iter()
            .filter(|(_, evs)| !evs.is_empty())
            .map(|(w, _)| *w)
            .collect();

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

            // Distinct peers, in WorkerId order, get USART slots in order.
            let mut peers: BTreeSet<WorkerId> = BTreeSet::new();
            collect_peers(evs, &mut peers);
            if peers.len() > TRANSPORT_USARTS.len() {
                return Err(EmitError::UnsupportedFeature(format!(
                    "worker `{name}` has {} distinct cross-MCU peers but the \
                     STM32H743 platform models only {} transport USARTs \
                     (usart1 is reserved for the output-capture stream). A \
                     denser interconnect needs a different transport topology \
                     (TASK-0049.05).",
                    peers.len(),
                    TRANSPORT_USARTS.len()
                )));
            }
            let mut peer_usart: BTreeMap<WorkerId, UsartSlot> = BTreeMap::new();
            for (i, p) in peers.iter().enumerate() {
                peer_usart.insert(*p, TRANSPORT_USARTS[i]);
            }

            // Map each channel (seq) to its peer's USART.
            let mut seq_usart: BTreeMap<u64, UsartSlot> = BTreeMap::new();
            map_seqs(evs, &peer_usart, &mut seq_usart);

            workers.insert(
                *w,
                WorkerPlan {
                    worker: *w,
                    name,
                    seq_usart,
                    peer_usart,
                    loads_input: has_effectful_load(evs),
                    saves_output: has_effectful_save(evs),
                },
            );
        }

        // One hub per unordered worker-pair that shares an edge. Each
        // endpoint uses its own assigned USART for the peer (they need not
        // be the same index — `connector Connect` joins both to the hub).
        let mut hubs: Vec<Hub> = Vec::new();
        let mut seen_pairs: BTreeSet<(WorkerId, WorkerId)> = BTreeSet::new();
        for (w, plan) in &workers {
            for peer in plan.peer_usart.keys() {
                let (a, b) = if w < peer { (*w, *peer) } else { (*peer, *w) };
                if !seen_pairs.insert((a, b)) {
                    continue;
                }
                // Fail loud on a one-sided edge: the peer must carry a
                // matching Push/Wait (so it has a USART assigned back to us).
                for (x, y) in [(a, b), (b, a)] {
                    if workers.get(&x).and_then(|p| p.peer_usart.get(&y)).is_none() {
                        return Err(EmitError::ContractGap(format!(
                            "transport edge {a:?}<->{b:?} is one-sided: worker \
                             {x:?} has no matching Push/Wait with {y:?} \
                             (TASK-0049.05)"
                        )));
                    }
                }
                let na = &workers[&a].name;
                let nb = &workers[&b].name;
                hubs.push(Hub {
                    name: format!("link_{na}_{nb}"),
                    a,
                    b,
                });
            }
        }

        let boot_order = compute_boot_order(per_worker, &used);

        Ok(TransportPlan {
            workers,
            hubs,
            boot_order,
        })
    }
}

/// Collect the distinct peer workers a worker talks to (Push `dst` / Wait
/// `src`), recursing into loop bodies.
fn collect_peers(events: &[Event], out: &mut BTreeSet<WorkerId>) {
    for e in events {
        match e {
            Event::Push { dst, .. } => {
                out.insert(*dst);
            }
            Event::Wait { src, .. } => {
                out.insert(*src);
            }
            Event::Loop { body, .. } => collect_peers(body, out),
            _ => {}
        }
    }
}

/// Map every channel `seq` this worker uses to its peer's assigned USART.
fn map_seqs(
    events: &[Event],
    peer_usart: &BTreeMap<WorkerId, UsartSlot>,
    out: &mut BTreeMap<u64, UsartSlot>,
) {
    for e in events {
        match e {
            Event::Push { dst, seq, .. } => {
                if let Some(u) = peer_usart.get(dst) {
                    out.insert(seq.0, *u);
                }
            }
            Event::Wait { src, seq, .. } => {
                if let Some(u) = peer_usart.get(src) {
                    out.insert(seq.0, *u);
                }
            }
            Event::Loop { body, .. } => map_seqs(body, peer_usart, out),
            _ => {}
        }
    }
}

/// A worker has an effectful LOAD iff it fires a whole-array output with no
/// inputs (`a <-- load_input()`) — mirrors `render_fire`'s classification.
fn has_effectful_load(events: &[Event]) -> bool {
    events.iter().any(|e| match e {
        Event::Fire { bindings, .. } => is_effectful_load(bindings),
        Event::Loop { body, .. } => has_effectful_load(body),
        _ => false,
    })
}

/// A worker has an effectful SAVE iff it fires an output-less kernel
/// (`save_output(c)`) — mirrors `render_fire`'s classification.
fn has_effectful_save(events: &[Event]) -> bool {
    events.iter().any(|e| match e {
        Event::Fire { bindings, .. } => bindings.output.is_none(),
        Event::Loop { body, .. } => has_effectful_save(body),
        _ => false,
    })
}

fn is_effectful_load(bindings: &FireBinding) -> bool {
    matches!(&bindings.output, Some(o) if o.indices.is_empty()) && bindings.inputs.is_empty()
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
///   * `$uartFile`     = @<path the output-capture worker's USART1 writes>;
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
         # per worker-pair (Renode's UARTHub is a broadcast bus; a dedicated\n\
         # hub per pair keeps point-to-point transport collision-free).\n\
         #\n\
         # START-GATING (load-bearing, TASK-0049.01): Renode's\n\
         # UARTBase.WriteChar DROPS bytes that arrive before the receiver\n\
         # enables RX. Each firmware enables RX on all its link USARTs as its\n\
         # FIRST action; this script releases the machines RECEIVERS-FIRST so\n\
         # every receiver is RX-enabled before any peer transmits to it.\n\n",
    );

    // 1. One UARTHub per worker-pair.
    for h in &plan.hubs {
        writeln!(s, "emulation CreateUARTHub \"{}\"", h.name).ok();
    }
    s.push('\n');

    // 2. One machine per worker: platform + hub connects + capture/inject.
    for plan_w in plan.workers.values() {
        writeln!(s, "mach create \"{}\"", plan_w.name).ok();
        s.push_str("machine LoadPlatformDescription @platforms/cpus/stm32h743.repl\n");
        // Connect this worker's USART for each peer to that pair's hub.
        for (peer, usart) in &plan_w.peer_usart {
            if let Some(h) = plan
                .hubs
                .iter()
                .find(|h| (h.a == plan_w.worker && h.b == *peer) || (h.b == plan_w.worker && h.a == *peer))
            {
                writeln!(s, "connector Connect {} {}", usart.renode_name, h.name).ok();
            }
        }
        if plan_w.saves_output {
            s.push_str(
                "# This worker saves output -> capture its USART1 to the file backend.\n",
            );
            s.push_str("usart1 CreateFileBackend $uartFile true\n");
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
            // Last machine released: run the whole co-simulation to
            // completion (bounded, fully determinate).
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
