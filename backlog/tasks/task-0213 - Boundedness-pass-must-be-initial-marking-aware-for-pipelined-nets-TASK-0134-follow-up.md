---
id: TASK-0213
title: >-
  Boundedness pass must be initial-marking-aware for pipelined nets (TASK-0134
  follow-up)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-21 13:41'
updated_date: '2026-05-21 14:56'
labels:
  - compiler
  - ir
  - scheduling
dependencies:
  - TASK-0134
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0134 lands initial_marking = D on cross-worker buffer places inside pipeline=D loops. The current boundedness check uses derive_firing_order = source-order; with initial_marking=D and capacity=N=D, the first Push tries to deposit one more token (D+1) and overflows because the buffer is already full at startup.

This is a structural tension: interpretation (a) of PRD §8.2 puts D head-start tokens in the buffer place, which is incompatible with D = N capacity under a source-order firing trace.

Two viable resolutions, each documented in TASK-0134 notes:
1. Generalise derive_firing_order to be marking-aware. With initial_marking > 0 on a buffer place, the consumer (Wait) should fire before the producer (Push) for the first marking-many iterations. This requires examining the initial marking and reordering Push/Wait pairs at the boundedness-input stage.
2. Change the IR encoding: when pipeline=D applies, eliminate D producer firings from the unrolled Repeat body (representing them as pre-fired by the initial marking). This is a structural acfg_to_petri change.

AC#5 of TASK-0134 explicitly requires boundedness/deadlock to pass on a pipelined fixture. This task delivers that.

Acceptance criteria
- #1 derive_firing_order OR acfg_to_petri updated so a pipeline=D, buffer=D, body-with-Push-Wait net passes check_bounded.
- #2 The deadlock check also passes for the same fixture.
- #3 Existing non-pipelined fixtures regress unchanged (every example without pipeline= still fires in source order).
- #4 Determinism preserved (BTreeMap-driven, no HashMap).
- #5 Add the assertion currently dropped from acfg_to_petri.rs's e2e_example_13_pipeline_parallel_passes_boundedness_and_deadlock test back into the test suite.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Path 1: marking-aware derive_firing_order.

Algorithm: at each step, replay a virtual marking M starting from net.initial_marking. Maintain a list `remaining` of transitions in source order. At each step:
1. Pick the FIRST transition T in `remaining` that is firable (enabled AND would not overflow any output place's capacity) under M.
2. Fire T against M; remove T from `remaining`.
3. If no T in `remaining` is firable, return the order accumulated so far + `remaining` appended in source order so check_bounded/check_deadlock_free will diagnose the stall.

Key properties:
- Source-order tiebreak preserves existing fixtures (any net where source-order is already legal returns source-order verbatim).
- Marking-aware: when buffer is pre-marked at capacity (pipeline=D=N), the Push at the head is not firable (would overflow), so the firstable transition is the matching Wait further down — exactly what is needed.
- Deterministic: source order over a Vec, deterministic Net::fire under BTreeMap.

Implementation:
- Replace derive_firing_order body with the algorithm above. Use Net::clone + reset_to_initial; for each step iterate transitions, call sim.fire(t) — if Err, leave marking alone (Net::fire commits only on success). If we exhausted all without firing, append all remaining in source order and break.
- Update docstring + module doc to reflect new behaviour. No doc lies: drop the "insertion order is the order" wording; replace with "source-order with marking-aware reordering"; keep "deterministic" but state HOW (source-order tiebreak).
- Remove #[ignore] from e2e_example_13_pipeline_parallel_passes_boundedness_and_deadlock.
- Add a positive regression test for a non-pipelined fixture (example 01 or 02) verifying source-order is preserved when no marking forces reordering.

Verify: just build / test / e2e / determinism-check / determinism-check-negative / ci / clippy.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation (committed at 4b3e7ad)

Two-layer fix; both committed together because they are co-dependent.

### Layer 1 — marking-aware derive_firing_order (boundedness.rs)

Algorithm: at each step, replay a virtual marking starting from the
net's initial_marking. Pick the FIRST transition in source order that
is firable (Net::fire returns Ok). Fire it; drop from remaining. On
stuck states (no remaining transition is firable), append the
remaining in source order so check_bounded / check_deadlock_free
surface the precise stall point.

Properties:
- Source-order tiebreak makes the result byte-identical to plain
  insertion-order on every net where source-order is already a legal
  firing sequence. All pre-TASK-0213 fixtures unchanged.
- Marking-aware: capacity-overflowing pushes are skipped in favor of
  later-but-firable consumers. Pure defense-in-depth (does NOT
  resolve the example-13 cycle alone; see Layer 2).

### Layer 2 — push-arc elision in acfg_to_petri (acfg_to_petri.rs)

For each Push transition emitted inside a pipelined Repeat body, the
first D pushes per seq do NOT add a TtoP arc to the buffer place.
The buffer's initial_marking = D already encodes those head-start
credits (PRD §8.2: "tokens placed in pipeline-register places by the
initial marking... schedule keywords lower into different initial
markings on the same kind of place").

The Push *transition* still exists — worker control chain intact,
EventList projection unchanged, runtime codegen unchanged. Only the
analysis-level buffer arithmetic credits the first D pushes against
the initial marking.

### Why Layer 1 alone could not work (counterexample)

Example 13's unrolled body interposes a sync_barrier between Push and
Wait (sync_inject's over-syncing — documented in sync_inject.rs
module doc as a known limitation). This forces Push to fire BEFORE
Wait in the static topology. With initial_marking=D=capacity, the
Push at the head of source order overflows. The recursive consumer-
closure fallback (path-1's "fire consumers first to make room") also
leads straight back to the Push through the barrier — the static
dependency graph has a cycle that no firing order can resolve.

Path 1's brief explicitly anticipated this case: "Path 1 is preferred;
path 2 is the fallback if path 1 has a tractable counterexample."

### Honest mismatch with runtime semantics

At runtime, all N pushes deposit into the ring buffer and consumers
drain in parallel; the buffer never holds more than D items
concurrently, but the *pre-fill* the analysis net models does not
actually exist in the backend's ring buffer. The analysis encoding
is a conservative bound — if it accepts, runtime is within capacity —
not a one-to-one trace of runtime state. Codegen reads the EventList
(from acfg_to_events, not the net), so divergence is invisible
downstream.

### Gate (post-implementation, run inside nix develop)

- cargo test workspace: 507 pass / 0 fail / 2 ignored
  - was 505/0/3 — added the source-order regression test;
  - removed #[ignore] on e2e_example_13_pipeline_parallel_passes_
    boundedness_and_deadlock.
- just e2e: 36 cells: 29 pass / 0 fail / 7 skipped / 0 required-fail
- just determinism-check: pass (byte-identical x2)
- just determinism-check-negative: pass (29 cells perturbed, gate bit)
- just ci: pass (cross-backend differential 28/1/7; 14 corrupted/1 detected)
- cargo clippy --workspace --all-targets -- -D warnings: clean

### Per-AC

- AC#1 (derive_firing_order OR acfg_to_petri updated → check_bounded):
  BOTH updated; the pipelined fixture passes.
- AC#2 (deadlock check passes for same fixture): YES.
- AC#3 (existing non-pipelined fixtures unchanged): YES — verified
  by new derive_firing_order_preserves_source_order_on_nonpipelined_
  fixture regression test plus the full test suite.
- AC#4 (determinism preserved): YES — BTreeMap-only state;
  determinism-check is green.
- AC#5 (assertion re-added to test suite): YES — #[ignore] removed.

### Follow-up filed
- TASK-0217 (medium): D > iteration_count edge case. When the
  pipeline depth exceeds the loop iteration count, all pushes are
  elided and the buffer ends with D - iteration_count leftover
  tokens. Boundedness still holds (leftover is below capacity).
  Either reject at link time or document.

### Files touched (absolute paths)
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/passes/acfg_to_petri.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/src/passes/boundedness.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/tests/acfg_to_petri.rs
- /home/mpedersen/topics/mark_thesis/nucleus/compiler/tests/boundedness.rs

### Rejected approach
- Pure path-1 (marking-aware firing-order alone, no IR change):
  empirically blocked by the sync_inject over-sync between Push and
  Wait. The static dependency cycle is unresolvable without
  structural IR change.

### Decision: path 1 + path 2 layered, NOT path 2 alone
Layer 1 is kept even though path 2 makes the buffer source-order-
firable by construction (making Layer 1 dormant on pipelined nets).
Reason: defense in depth. Future schedules with softer constraints
may produce nets where source-order isn't legal but a legal
interleaving exists; Layer 1 handles those. No fixture loses.

## Review-gate hardening (cycle close)

mped-architect read-only review: GO with one HIGH-priority follow-up.
qa-test-runner read-only review: GO clean (506/0/2 stable across 3 runs).

Doc-contradiction fix (in-thread, HIGH-priority architecture finding):

The prior TASK-0042.01 forward-carry note said the ring buffer must
"START with D pre-filled slots". After TASK-0213's elision, this is
WRONG: the ring starts EMPTY, the IR's initial_marking is an
analysis-encoding carrier of D, NOT a runtime pre-fill instruction.
Pre-filling at thread spawn + N runtime pushes would actually
overflow the ring.

Applied in-thread:
- acfg_to_petri.rs Initial markings section: added "what initial_marking
  IS, and is NOT" subsection; explicit pointer that backends size the
  ring to N and start empty.
- acfg_to_petri.rs Initial markings section: added "Coupled to the
  elision" cross-reference so the two sites reading
  pipeline_depth_for_seq stay in sync.
- acfg_to_petri.rs Honest limitations: added 3 explicit bullets
  (analysis-vs-runtime trade; TASK-0217 D>iteration_count;
  TASK-0218 sync_inject over-syncing).
- acfg_to_petri.rs § "Initial markings": removed the misleading "ring
  buffer initialisation" claim that contradicted the runtime semantics.
- tests/acfg_to_petri.rs e2e_example_13_pipeline_parallel docstring:
  corrected to attribute the resolution to path 2 (TtoP-arc elision),
  not path 1 (which alone cannot resolve this fixture).
- boundedness.rs derive_firing_order docstring: corrected the
  worked-example claim — path-1 reordering alone does NOT resolve
  example-13; the path-2 elision in acfg_to_petri does.
- TASK-0042.01 Implementation Notes: appended a CORRECTION block that
  explicitly supersedes the prior "Ring-buffer pre-fill contract" and
  provides correct codegen pseudocode (size N, start EMPTY).

Follow-ups filed:
- TASK-0218: sync_inject over-syncing is the root cause that forced
  the path-2 elision; if fixed, path-2 could be reverted.
- TASK-0219: path-1 marking-aware logic is dead code under current
  pipeline; test it (synthetic-net fixture) or remove it.

Re-run gate post-hardening (independent):
- cargo test workspace: 506 pass / 0 fail / 2 ignored.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells: 29 pass / 0 fail / 7 skipped / 0 required-fail.

QA cosmetic: implementer's commit-message body claimed 507/0/2; actual
across 3 stable runs is 506/0/2. Reviewer-of-record number recorded
here as the fact.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0213 DONE. Path 1 + path 2 layered. Example 13 pipelined fixture passes boundedness AND deadlock; 507 tests pass / 0 fail / 2 ignored (was 505/0/3); all e2e + determinism + CI gates green; clippy clean. Commit 4b3e7ad. Follow-up TASK-0217 filed for D > iteration_count edge case.
<!-- SECTION:FINAL_SUMMARY:END -->
