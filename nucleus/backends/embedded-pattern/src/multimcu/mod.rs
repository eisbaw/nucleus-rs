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
//!   * a BOOT ORDER computed by SIMULATING the staged machine release
//!     (a small deterministic fixpoint — see
//!     [`boot_order::compute_boot_order`]) so the
//!     receiver-RX-enabled-before-sender-TX start-gating discipline
//!     (TASK-0049.01: Renode's `UARTBase.WriteChar` DROPS bytes that
//!     arrive before the receiver enables RX) holds BY CONSTRUCTION — a
//!     worker's first `link_push` to a peer P never precedes P's release.
//!     This handles the CYCLIC ex14 interconnect (dsp↔fe, dsp↔rf), which
//!     has no static "receivers-first" sort order but does have a valid
//!     deterministic release order (dsp, rf, fe) the fixpoint finds
//!     (TASK-0049.05.03). A genuine mutual-eager-send cycle has no sound
//!     static order and falls back to a deterministic best-effort + the
//!     byte-exact Renode gate (a robust RX-ready handshake is the deferred
//!     TASK-0049.05.02 item-1).
//!
//! The `SeqTag`/peer come straight from the events: `Event::Push` carries
//! its `dst` worker, `Event::Wait` its `src` worker — so the peer of a
//! channel is read directly, no cross-referencing.
//!
//! ## Module layout (TASK-0450 split of the former single 1478-LoC file)
//!
//! This module was split along the docstring seams into cohesive
//! submodules; the external surface (`crate::multimcu::X`) is preserved
//! verbatim by the re-exports below:
//!
//!   * [`plan`] — the transport-plan data types + `TransportPlan::build`.
//!   * [`scan`] — event-tree scan helpers (channels, endpoints, IO/load
//!     classification, saved symbols).
//!   * [`input_offsets`] — global `input.bin` byte-layout + output-capture
//!     ordering.
//!   * [`control_sync`] — the control-only `Event::Sync` subsumption guard.
//!   * [`resc`] — the multi-machine `.resc` renderer.
//!   * [`boot_order`] — the staged-release boot-order fixpoint.

mod boot_order;
mod control_sync;
mod input_offsets;
mod plan;
mod resc;
mod scan;

// External surface — every `crate::multimcu::X` path the consumers
// (`lib.rs`, `skeleton::multimcu`, `tests/*`) reference, preserved
// byte-for-byte via these re-exports (TASK-0450 AC#2). `compute_boot_order`
// is NOT re-exported: it was a private `fn` in the pre-split file (only
// `TransportPlan::build` calls it, via the `super::boot_order` path), so it
// was never part of the `crate::multimcu::` surface.
pub(crate) use self::control_sync::verify_control_sync_subsumed;
// `compute_input_offsets` is referenced only from `tests/input_offsets.rs`
// (the non-test lib reaches it via `super::input_offsets` inside
// `TransportPlan::build`), so this re-export is unused in a non-test build —
// it preserves the `crate::multimcu::compute_input_offsets` test path the
// pre-split `pub(crate) fn` definition exposed.
#[allow(unused_imports)]
pub(crate) use self::input_offsets::compute_input_offsets;
pub(crate) use self::plan::{TransportPlan, UsartSlot, WorkerPlan};
pub(crate) use self::resc::render_multimachine_resc;
