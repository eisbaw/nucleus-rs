//! Double-buffered (depth-2) DMA descriptor-ring lowering for
//! `mode=dma` cross-worker transfers carrying `buffer>=2`
//! (TASK-0455.02).
//!
//! ## Why this module exists
//!
//! The default `mode=dma` Push/Wait lowering (in [`crate::render`]) is an
//! **arm + immediate completion-spin** — a depth-1, structurally
//! SYNCHRONOUS shape (the producer blocks on completion at the arm site).
//! That is correct for a `buffer=1` DMA edge but cannot express the
//! headline tier-3 contribution: a DOUBLE-BUFFERED DMA pipeline where the
//! sender arms transfer `i+1` while transfer `i` is still in flight — the
//! 2013 ISP-firmware shape that motivated the whole "IO semantics as
//! swappable schedule directives" thesis (PRD §7.3
//! `embedded-cortexm-dma-irq`). On bare metal there is no OS to absorb the
//! latency, so the double buffer IS the latency-hiding mechanism.
//!
//! When the schedule declares `transfer D : async, buffer=2, notify=event,
//! mode=dma`, this module emits a **depth-2 descriptor ring** instead.
//! NOTE (wave-6 review P3.6): the dispatch keys SOLELY on `mode=dma` +
//! `buffer>=2` — loop position is NOT consulted. The canonical (and only
//! shipped) shape is the loop-nested producer/consumer of example 09; a
//! TOP-LEVEL buffered DMA edge would also take the ring, with its lone
//! descriptor never drained in-loop (byte-safe only under the modelled
//! synchronous engine — same caveat-1 territory as the lazy final drain):
//!
//!   * a module-static OCCUPANCY counter + HIGH-WATER mark
//!     (`NUC_DMA_RING_OCC_<seq>` / `NUC_DMA_RING_HWM_<seq>`), so the ring
//!     reaching depth 2 is an OBSERVABLE state property (AC#4), not merely
//!     a capability the gate accepted;
//!   * `notify`-selected completion: `notify=event` waits via
//!     `shim.dma_link_irq_wait` (the IRQ-completion hook — a `wfi`-style
//!     wait on the modelled DMA-complete event), any other mode keeps the
//!     existing `while !dma_link_poll {}` busy-spin. This is the FIRST
//!     consumer of the per-seq `XferFacts::notify` fact (TASK-0455.08).
//!
//! ## The DRAIN-WHEN-FULL double-buffer construction
//!
//! Each loop iteration emits (for a PUSH):
//!   1. arm the DMA descriptor (from the data local `name`);
//!   2. `OCC += 1`, record `HWM = max(HWM, OCC)`;
//!   3. **only when the ring is FULL** (`OCC == RING_DEPTH`) drain the
//!      OLDEST descriptor (notify-selected wait, `OCC -= 1`).
//!
//! So iteration 0 arms (`OCC = 1`, not full — no drain); iteration 1 arms
//! (`OCC = 2`, **HWM reaches 2** — both descriptors in flight — then drains
//! the oldest back to `OCC = 1`); steady state oscillates 1↔2. That is the
//! genuine depth-2 ring: at the top of every iteration `i >= 1` two
//! descriptors are simultaneously enqueued. The HWM static is the AC#4
//! witness that the ring genuinely reached depth 2.
//!
//! ## Honest model caveats
//!
//! 1. **Synchronous modelled DMA — no per-slot staging.** Under Renode
//!    `dma_link_arm` delegates to `link_push` and the bytes move at arm
//!    time; `dma_link_poll` / `dma_link_irq_wait` return promptly (no real
//!    DMA-complete IRQ). Because the transfer completes synchronously
//!    inside the arm, the data local `name` is fully consumed before the
//!    next iteration overwrites it — so NO second physical staging buffer
//!    is needed for soundness on the modelled path, and the ring arms
//!    directly from `name`. The two in-flight slots therefore do not
//!    produce genuine TEMPORAL overlap on the non-cycle-accurate emulator —
//!    what is sound and observable is the ring OCCUPANCY reaching 2 (the
//!    high-water static) plus byte-exact output. A REAL silicon engine
//!    (where `dma_link_arm` returns immediately and the stream reads `name`
//!    asynchronously) WOULD need a per-slot staging copy so iteration `i+1`
//!    does not clobber `name` while `i`'s stream is mid-flight; that
//!    per-slot staging + real DMA-complete IRQ is the forward follow-up
//!    (TASK-0048.12 / TASK-0438.03), the same boundary the single-buffer
//!    `mode=dma` arm already carries.
//! 2. **Lazy final drain.** The last-armed descriptor of a loop is NOT
//!    drained inside the loop body (the drain only fires when the ring goes
//!    FULL, and the final arm may leave `OCC = 1`). Because the modelled
//!    DMA already transmitted the bytes at arm time, the value output is
//!    UNAFFECTED (byte-exact holds); only the cosmetic `OCC` residue of 1
//!    survives `run`. On a real async engine a loop epilogue drain would be
//!    needed; the per-event renderer does not inject loop epilogues, and
//!    the per-iteration `irq_barrier` the (frozen) walker emits already
//!    serialises completion, so this is an honest documented limit, not a
//!    miscompile (the byte-exact Renode gate would fail LOUD on any
//!    dropped byte).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use backend_common::EmitError;
use nucleus_compiler::acfg::NotifyMode;
use nucleus_compiler::event::{Event, SeqTag};
use nucleus_compiler::sched::TransportMode;
use nucleus_compiler::sidecar::NameSidecar;

/// The depth-2 ring is the only buffered depth the embedded backend
/// declares (`max_buffer = 2` in `capabilities.toml`). A `buffer` fact
/// above this is rejected at the capability gate before codegen, so this
/// is the one slot count the ring emit ever uses.
pub(crate) const RING_DEPTH: u64 = 2;

/// Sanitise a `SeqTag` into the suffix used in the per-seq ring static
/// names (`NUC_DMA_RING_OCC_<suffix>`). A `SeqTag` is a `u64`, so the
/// decimal spelling is already a valid identifier tail — but we route it
/// through one helper so the static-name spelling has a single source of
/// truth shared by the static-decl emit and the per-event references (no
/// textual-replace drift; feedback-textual-replace-codegen-unsafe).
pub(crate) fn ring_suffix(seq: SeqTag) -> String {
    seq.0.to_string()
}

/// `true` iff a `mode=dma` transfer on `seq` carries `buffer >= 2` — the
/// signal to emit the depth-2 ring instead of the single-buffer arm+spin.
/// A `buffer` fact of `None` (no `xfer_facts` entry) or `1` keeps the
/// existing single-buffer path (byte-identical to pre-TASK-0455.02 for
/// every `buffer=1` / no-`buffer=` DMA edge — load-bearing for the
/// 22-dma-pio-demo gate, whose three edges are all single-buffered).
pub(crate) fn is_double_buffered(buffer: Option<u64>) -> bool {
    buffer.is_some_and(|n| n >= RING_DEPTH)
}

/// Walk the worker's event tree (recursing into loop bodies) and collect
/// the static-name suffix of every DISTINCT seq carrying a `mode=dma` +
/// `buffer>=2` Push or Wait — the seqs whose lowering emits the depth-2
/// ring. Deterministic (`BTreeSet` keyed by `SeqTag` -> ascending suffix
/// order), so the emitted static block is stable across builds.
pub(crate) fn collect_ring_suffixes(events: &[Event], sidecar: &NameSidecar) -> Vec<String> {
    let mut seqs: BTreeSet<SeqTag> = BTreeSet::new();
    collect_into(events, sidecar, &mut seqs);
    seqs.into_iter().map(ring_suffix).collect()
}

fn collect_into(events: &[Event], sidecar: &NameSidecar, out: &mut BTreeSet<SeqTag>) {
    for ev in events {
        match ev {
            Event::Push { seq, .. } | Event::Wait { seq, .. } => {
                let is_dma = matches!(sidecar.xfer_transport(*seq), TransportMode::Dma);
                if is_dma && is_double_buffered(sidecar.xfer_buffer(*seq)) {
                    out.insert(*seq);
                }
            }
            Event::Loop { body, .. } => collect_into(body, sidecar, out),
            _ => {}
        }
    }
}

/// Emit the module-scope ring statics for every double-buffered seq: an
/// OCCUPANCY counter (live in-flight descriptor count) + its HIGH-WATER
/// mark. Shared by the lib and bin skeletons (like `render_count_statics`)
/// so both emit the IDENTICAL statics the `run` body references.
///
/// Plain `static mut` (NOT `AtomicU32`, unlike the check-loop counters):
/// the ring statics are touched ONLY by the single firmware thread inside
/// `run` (a Cortex-M firmware is single-core and the ring is local to one
/// worker's event stream), so a `static mut` updated under `unsafe` is the
/// no_std-honest, alloc-free shape. `non_upper_case_globals` /
/// `static_mut_refs` are allowed at the crate header.
pub(crate) fn render_ring_statics(s: &mut String, suffixes: &[String]) {
    if suffixes.is_empty() {
        return;
    }
    s.push_str(
        "// TASK-0455.02: per-`mode=dma`+`buffer=2` descriptor-ring occupancy\n\
         // trackers. OCC is the live in-flight descriptor count; HWM is the\n\
         // high-water mark (reaches RING_DEPTH=2 for a genuinely double-\n\
         // buffered edge — the AC#4 observability witness). Single-core\n\
         // firmware, touched only inside `run`, so plain `static mut` suffices\n\
         // (no AtomicU32; cf. the check-loop counters). No per-slot staging\n\
         // buffer: the modelled DMA completes synchronously inside the arm, so\n\
         // `name` is consumed before the next iteration overwrites it (a real\n\
         // async engine would stage per-slot — module docs / TASK-0048.12).\n",
    );
    for suf in suffixes {
        let _ = writeln!(
            s,
            "static mut NUC_DMA_RING_OCC_{suf}: u32 = 0;\n\
             static mut NUC_DMA_RING_HWM_{suf}: u32 = 0;"
        );
    }
    s.push('\n');
}

/// Emit the depth-2 ring PUSH for a `mode=dma` + `buffer>=2` seq
/// (drain-when-full double buffer; see module docs).
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_ring_push(
    out: &mut String,
    pad: &str,
    name: &str,
    byte_len: &str,
    chan: u64,
    suf: &str,
    notify: NotifyMode,
) -> Result<(), EmitError> {
    writeln!(
        out,
        "{pad}// DMA double-buffered push of `{name}` (seq {chan}, buffer=2): arm the\n\
         {pad}// descriptor, track occupancy (high-water = 2 is the AC#4 double-buffering\n\
         {pad}// witness), drain the oldest only when the ring is FULL."
    )
    .ok();
    // Arm the descriptor directly from the data local (the modelled DMA
    // consumes `name` synchronously inside the arm — module docs).
    writeln!(
        out,
        "{pad}shim.dma_link_arm({chan}, {name}.as_ptr() as *const u8, {byte_len});"
    )
    .ok();
    render_arm_bump_and_drain(out, pad, chan, suf, notify);
    Ok(())
}

/// Emit the depth-2 ring WAIT (receive) for a `mode=dma` + `buffer>=2`
/// seq. Symmetric to [`render_ring_push`] in the static + notify surface,
/// but the receive side completes the descriptor IMMEDIATELY (NOT
/// drain-when-full): the data local `name` IS the receive destination and
/// is consumed by the VERY NEXT compute statement, so deferring the
/// completion would let that compute read a not-yet-received buffer (a real
/// data-dependency hazard, not just a model nicety). Double-buffering is a
/// PRODUCER-side latency-hiding optimisation (the producer runs ahead of an
/// in-flight transfer); the consumer must always have its current frame
/// before computing. So the recv arm + completes per iteration (occupancy
/// 0->1->0); it bumps its OWN binary's occupancy/high-water statics (push
/// and wait of a cross-worker seq live in different firmware binaries, so
/// the consumer-side HWM only ever reaches 1; the depth-2 witness is the
/// PRODUCER-side HWM — wave-6 review P3.5 corrected this claim) so the
/// ring's depth-2 witness (driven by the producer side) is observable on
/// the same `seq`, and it still honours `notify=event` via the IRQ-wait.
pub(crate) fn render_ring_recv(
    out: &mut String,
    pad: &str,
    name: &str,
    byte_len: &str,
    chan: u64,
    suf: &str,
    notify: NotifyMode,
) -> Result<(), EmitError> {
    writeln!(
        out,
        "{pad}// DMA double-buffered receive of `{name}` (seq {chan}, buffer=2): arm the\n\
         {pad}// receive descriptor, then complete IMMEDIATELY (the consumer needs the\n\
         {pad}// frame before computing — double-buffering is producer-side; see docs)."
    )
    .ok();
    writeln!(
        out,
        "{pad}shim.dma_link_recv_arm({chan}, {name}.as_mut_ptr() as *mut u8, {byte_len});"
    )
    .ok();
    // Bump occupancy + record high-water (shared with the producer side's
    // ring on this seq), then complete immediately and drop occupancy.
    writeln!(
        out,
        "{pad}unsafe {{\n\
         {pad}    NUC_DMA_RING_OCC_{suf} += 1;\n\
         {pad}    if NUC_DMA_RING_OCC_{suf} > NUC_DMA_RING_HWM_{suf} {{ NUC_DMA_RING_HWM_{suf} = NUC_DMA_RING_OCC_{suf}; }}\n\
         {pad}}}"
    )
    .ok();
    render_completion_wait(out, pad, chan, notify);
    writeln!(out, "{pad}unsafe {{ NUC_DMA_RING_OCC_{suf} -= 1; }}").ok();
    Ok(())
}

/// Emit the shared post-arm sequence: bump occupancy + record high-water,
/// then drain the oldest descriptor ONLY when the ring is full
/// (`OCC == RING_DEPTH`). This is what makes the occupancy genuinely reach
/// 2 (the arm bumps to 2 before the conditional drain pulls it back to 1).
fn render_arm_bump_and_drain(out: &mut String, pad: &str, chan: u64, suf: &str, notify: NotifyMode) {
    writeln!(
        out,
        "{pad}unsafe {{\n\
         {pad}    NUC_DMA_RING_OCC_{suf} += 1;\n\
         {pad}    if NUC_DMA_RING_OCC_{suf} > NUC_DMA_RING_HWM_{suf} {{ NUC_DMA_RING_HWM_{suf} = NUC_DMA_RING_OCC_{suf}; }}\n\
         {pad}}}"
    )
    .ok();
    // Drain the oldest ONLY when the ring is full — this is the deferred
    // completion that lets two descriptors coexist (OCC reaches RING_DEPTH).
    writeln!(
        out,
        "{pad}if unsafe {{ NUC_DMA_RING_OCC_{suf} }} == {depth} {{",
        depth = RING_DEPTH
    )
    .ok();
    let inner = format!("{pad}    ");
    render_completion_wait(out, &inner, chan, notify);
    writeln!(out, "{inner}unsafe {{ NUC_DMA_RING_OCC_{suf} -= 1; }}").ok();
    writeln!(out, "{pad}}}").ok();
}

/// Emit the `notify`-selected completion wait for `chan` (no occupancy
/// bookkeeping — the caller does that).
///
/// - `notify == Event` -> `shim.dma_link_irq_wait(chan)`: the IRQ-driven
///   completion (the FIRST honouring consumer of `XferFacts::notify`).
/// - any other notify -> the `while !dma_link_poll {}` busy-spin.
fn render_completion_wait(out: &mut String, pad: &str, chan: u64, notify: NotifyMode) {
    match notify {
        NotifyMode::Event => {
            writeln!(
                out,
                "{pad}// notify=event: IRQ-driven completion (XferFacts::notify, TASK-0455.02).\n\
                 {pad}shim.dma_link_irq_wait({chan});"
            )
            .ok();
        }
        _ => {
            writeln!(
                out,
                "{pad}// notify default/poll: busy-spin completion (same as single-buffer DMA).\n\
                 {pad}while !shim.dma_link_poll({chan}) {{ core::hint::spin_loop(); }}"
            )
            .ok();
        }
    }
}
