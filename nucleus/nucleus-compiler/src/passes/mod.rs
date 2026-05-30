//! Compiler passes that consume and produce an [`crate::acfg::ACFG`].
//!
//! Each submodule is one self-contained transformation. The pipeline
//! order is fixed by the PRD §5 diagram:
//!
//! 1. [`sync_inject`] — TASK-0017. Insert barrier syncs where
//!    control-flow joins require it.
//! 2. [`transfer_inject`] — TASK-0018. Insert matched Push/Wait
//!    placeholders for every dataflow edge that crosses workers.
//! 3. (future) Petri-net construction — later milestones.
//!
//! Passes are pure functions `ACFG -> ACFG` (or `(&LinkedIR, ACFG) ->
//! ACFG` where the link-pass output is also needed; see
//! [`transfer_inject::inject_transfers`]) so they compose with
//! function-composition rules and tests can pipe them in any order.

pub mod acfg_to_petri;
pub mod block_transform;
pub mod boundedness;
// TASK-0261 cycle 82 (prerequisite): shared affine-stride helpers
// (`affine_decompose`, `eval_const_int`, `expr_mentions`) used by both
// `halo_inference` (TASK-0260 Stage 1) and `reuse_inference` (TASK-0261
// Stage 1). Lifted from `halo_inference` per cycle-81 review forward-
// carry; `pub(crate)` so out-of-crate callers cannot bypass the
// pass-level validation that wraps them.
pub mod common;
pub mod deadlock;
// TASK-0368: combined Petri-net soundness gate. Bundles `boundedness`
// + `deadlock` into one `check_net_sound` entry point that the driver
// runs on EVERY build (a failure is a compile error). Wires PRD §8's
// "analyses fall out as standard properties; failures are compile
// errors" into the shipping path, not just the test suite. Exact-
// replay over one deterministic firing order — sound for v2's
// statically-ordered restricted nets, NOT a general reachability
// engine. See module doc.
pub mod net_soundness;
// TASK-0260 Stage 1: halo region inference from kernel access patterns.
// Runs AFTER `apply_partition_blocks2d` (driver pass order), AFTER
// `build_acfg` (needs name_iter_vars / name_kernels). Pure +
// observationally-inert in Stage 1: writes `ACFG::halo_widths` for
// Stage 2 (transfer_inject extension, TASK-0263) to consume.
pub mod halo_inference;
// TASK-0329 cycle 160: backend-local host-mediation injection. Invoked
// by the driver for `mp-tcp-bufsync` / `mp-tcp-event` (whose
// one-CTRL-stream-per-(host,worker) star topology cannot lower a
// host-excluding barrier without a w↔w mesh). Adds host as a
// mediating participant in every Sync excluding it; pthreads-sync /
// pthreads-async skip this pass (their shared-memory barrier
// primitives handle host-excluding barriers natively). See module
// docs for the CTRL-arm-vs-DATA-arm cycle-148/149 split rationale.
pub mod host_mediation_inject;
// TASK-0329.01.02 cycle 163: backend-local host-mediated data-relay
// injection (slice 2 of the TASK-0329.01 keystone). Invoked by the
// driver for `mp-tcp-event` only (mp-tcp-bufsync gated capability-side
// per AC#5 bufsync audit). For every Push/Wait pair whose endpoints
// are BOTH non-host, replaces the pair with four sibling Xfers routing
// the transfer through host. The new host endpoints project naturally
// onto host's per-worker event list including INSIDE Repeat bodies —
// satisfies F1/F5 forward-carried lessons from slice 1 cycle 162a.
// See module docs for the B1-vs-B2 decision rationale + AC mapping.
pub mod host_data_relay_inject;
// TASK-0261 Stage 1: reuse loop-option inference (PRD §6.3.3 + §13).
// For each `for V : reuse;` loop, walk the body's kernel-arg
// `IrExpr::DataRef` indices, classify their affine `(coeff, offset)` in
// V, and persist a per-(IterVar, DataId, axis) delay-line slot into
// `ACFG::reuse_widths`. Pure + observationally-inert in Stage 1: the
// backend walker / codegen consumer is Stage 2 (TASK-0265).
pub mod reuse_inference;
// TASK-0052.02: real-time `check loop V : latency_max=T` projection.
// Runs AFTER `petri_to_events`, BEFORE backend codegen — see module
// docstring for the dependency rationale.
pub mod inject_check_frames;
pub mod partition_blocks2d;
pub mod partition_rows;
pub mod partition_workers;
pub mod petri_to_events;
// TASK-0329.01.01: safe push-before-wait reordering at the event-list
// layer. Hoists hoistable worker-to-worker Pushes above preceding w2w
// Waits within a top-level boundary to break the wait-before-push
// deadlock cycle on mp-tcp-event's synchronous host-relay. Backend-
// local: driver applies for `mp-tcp-event` only (slice 1 of the
// TASK-0329.01 keystone). See module docs for the hoistable predicate
// (P1.1 distinguishes w2w-Wait vs host->worker-Wait).
pub mod safe_push_reorder;
pub mod sync_inject;
pub mod transfer_inject;
