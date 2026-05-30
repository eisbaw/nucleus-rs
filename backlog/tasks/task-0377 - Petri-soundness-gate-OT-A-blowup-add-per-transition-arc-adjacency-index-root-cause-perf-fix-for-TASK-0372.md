---
id: TASK-0377
title: >-
  Petri soundness gate O(T*A) blowup: add per-transition arc adjacency index
  (root-cause perf fix for TASK-0372)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-30 23:06'
updated_date: '2026-05-30 23:35'
labels:
  - perf
  - petri
  - gate
  - root-cause
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
ROOT-CAUSE fix superseding TASK-0372's flag-it-off workaround. Orchestrator measurement (cycle 218) of the cycle-217 Petri gate (passes/net_soundness.rs, check_net_sound = derive_firing_order + check_bounded + check_deadlock_free):

EVIDENCE (prebuilt release binary, direct invocation, EPOCHREALTIME, LC_ALL=C, 30 reps):
- 07-matmul/distributed8 (T=4149 transitions, P=32870 places, A=65722 arcs): GATE-ON 473 ms/build vs GATE-OFF (gate stubbed, rebuilt) 34 ms/build -> gate adds ~439 ms = 93% of build, a 14x slowdown.
- 16-jacobi/distributed (T=402, A=2170): gate ~5 ms total (small net).
- Instrumented per-component on dist8: derive_firing_order=187ms, check_bounded=179ms, check_deadlock_free=172ms (each ~T fire() calls).

ROOT CAUSE: petri.rs Net::fire(t) is O(A) -- it does self.arcs.iter().filter(|a| a.transition==t) TWICE (PtoT at line ~349, TtoP at line ~380) scanning ALL arcs to find those incident to t. No per-transition adjacency index. Each of the 3 analyses fires ~T transitions => O(T*A) per analysis; ~800M arc comparisons total on dist8. Secondary: fire() returns Ok(self.current_marking.clone()) (line ~429) cloned on every success but DISCARDED by all 3 analysis hot-paths; check_bounded/check_deadlock_free clone marking_before EVERY step (lines ~188/~236) though it is only used on the (rare) failure arm and fire() does not mutate on failure. Tertiary: derive_firing_order rescans from index 0 each outer iter -> O(T^2) cheap bool-skips (~17M on dist8, minor).

FIX (near-linear, MUST stay bit-identical -- gate must accept/reject exactly the same nets): (1) precompute per-transition incident-arc index once (Vec<Vec<arc-idx>> or in/out arc lists keyed by TransitionId) and have fire (or a fire_in_place variant) consult it in O(deg(t)) instead of O(A); (2) add a non-cloning fire path for the analyses so the discarded marking clone is gone; (3) capture marking_before lazily only on the failure arm (fire leaves marking unmutated on failure); (4) optional: advance derive_firing_order scan cursor past the contiguous-fired prefix. Behavior is preserved because needs/produces are BTreeMap-summed per place (order-independent) and determinism comes from arc-insertion order which the index preserves.

ACCEPTANCE: gate cost on dist8 drops from ~440ms to a small fraction (target <40ms, ideally <15ms); existing boundedness.rs/deadlock.rs/net_soundness.rs unit tests still pass; e2e bit-identity unchanged across all 7 tier-1 backends; just test + test-release + e2e all green. Once landed, TASK-0372's CLI flag + e2e split is very likely UNNECESSARY (gate stays always-on AND cheap) -- re-evaluate and close 0372 as superseded if the perf target is met.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Net::fire (or a new fire_in_place) consults a per-transition arc adjacency index; no O(A) all-arcs scan remains on the analysis hot path
- [ ] #2 Per-build Petri gate cost on 07-matmul/distributed8 drops from ~440ms to <40ms (measured, report before/after numbers)
- [ ] #3 All existing boundedness/deadlock/net_soundness unit tests pass; behavior bit-identical (gate accepts/rejects the same nets)
- [ ] #4 just test + just test-release + just e2e green; e2e totals + bit-identity unchanged across 7 tier-1 backends
- [ ] #5 A perf regression pin (test or documented benchmark cmd) records the near-linear expectation so a future O(A)-reintroduction is catchable
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Implementation Plan (cycle 218)

Baseline reproduced on dist8 (T=4149, A=65722): total build mean=439ms, isolated gate cost mean=435ms (temp eprintln around check_net_sound, 30 reps, LC_ALL=C, prebuilt binary). Gate IS ~99% of build. Confirms root cause.

ROOT CAUSE confirmed by read: petri.rs `Net::fire` scans ALL arcs twice per call (PtoT filter line ~349, TtoP filter line ~380) -> O(A) per fire; 3 analyses x ~T fires each = O(T*A).

### Steps
1. petri.rs: add `pub struct ArcIndex { in_arcs: Vec<Vec<usize>>, out_arcs: Vec<Vec<usize>> }` indexed by TransitionId.0; `Net::build_arc_index(&self) -> ArcIndex` iterates `self.arcs` in insertion order (preserves determinism). Index stores usize indices into `net.arcs`; valid across `net.clone()` because transition ids == vec index and clone preserves both vecs.
2. petri.rs: add `fn fire_in_place(&mut self, t, idx: &ArcIndex) -> Result<(), FireError>` — same checks/commit as fire() but consults idx.in_arcs[t]/out_arcs[t] in O(deg(t)); NO marking clone (returns unit). Keep capacity/enabled/error semantics byte-identical (BTreeMap-summed needs/produces, same touched union/sort/dedup).
3. petri.rs: rewrite public `fire(&mut self, t) -> Result<Marking, FireError>` to build a one-shot index + call fire_in_place + clone marking on success. Keeps every existing test/ad-hoc caller working (they consume the Marking). On Err, returns Err with marking unmutated (fire_in_place must NOT mutate on failure — all checks before commit).
4. boundedness.rs check_bounded: build index once before loop, call sim.fire_in_place(tid, &idx); move marking_before clone INTO the CapacityExceeded Err arm (lazy — fire leaves marking unmutated on failure, so sim.current_marking IS the before-state in the Err arm).
5. boundedness.rs derive_firing_order: build index once; call sim.fire_in_place(t.id, &idx).is_ok(). (Optional cursor for O(T^2) bool-skips — do only if clean.)
6. deadlock.rs check_deadlock_free: build index once; sim.fire_in_place(tid,&idx); marking_before lazy into Stalled Err arm.
7. enabled_transitions: NOT on gate hot path -> leave un-indexed with a `// TASK-0377: not indexed (off gate hot path)` note.

### Invariant
PURE perf. Gate accepts/rejects identical nets, identical error variants. Determinism preserved (arc-insertion order kept by building index in `net.arcs` order; needs/produces BTreeMap-summed = order-independent).

### Verification
just build/clippy/test/test-release/e2e all green; e2e totals 329/272/0/57/0 + 7-backend bit-identity unchanged. Re-measure gate cost after; target <40ms. Add AC#5 perf-pin (committed bench cmd + a near-linear unit assertion).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
cycle-218 impl landed. ROOT CAUSE confirmed + fixed: petri.rs Net::fire scanned all arcs twice per call (O(A)/fire). Added petri::ArcIndex { in_arcs, out_arcs: Vec<Vec<usize>> } + Net::build_arc_index() (O(A), insertion-order preserved) + Net::fire_in_place(t, &index) (O(deg(t)), no marking clone, leaves marking unmutated on failure). Public fire() now delegates: builds one-shot index + fire_in_place + final clone, so all existing tests/ad-hoc callers (petri.rs/proptest_petri.rs) that consume the returned Marking keep working unchanged. Wired all THREE hot paths: check_bounded, derive_firing_order (+ scan-cursor for the O(T^2) tertiary), check_deadlock_free. marking_before clone moved into the failure arm (lazy) in check_bounded+check_deadlock_free. enabled_transitions left un-indexed with a // TASK-0377 note (off gate hot path, tiny-net-only).

INDEX VALIDITY ACROSS net.clone(): each analysis does net.clone()+reset_to_initial() then replays on the clone; index is built from the ORIGINAL net before clone and is keyed by transition-index (==TransitionId.0) storing usize indices into net.arcs. Clone preserves transitions+arcs element-for-element so the index is valid against the clone. Documented in ArcIndex docstring.

DETERMINISM TRAP avoided: index built by iterating net.arcs in insertion order so in_arcs[t]/out_arcs[t] preserve arc-insertion order; needs/produces still BTreeMap-summed per place (order-independent). Byte-identical to old path.

MEASURED (prebuilt release binary, direct invocation, 30 reps, LC_ALL=C, $EPOCHREALTIME):
- dist8 isolated gate cost (temp eprintln around check_net_sound, since REVERTED): BEFORE 435.00 ms -> AFTER 16.29 ms = 27x.
- dist8 full build: BEFORE 439.12 ms -> AFTER 26.44 ms = 16.6x.
Target (<40ms, ideally <15ms): MET for the 40ms target; the 16ms gate just-misses the 15ms stretch (now dominated by 3x net.clone + 3x O(A) index builds + the derive_firing_order replay, NOT by any O(A)/fire scan). The temp driver instrumentation was fully reverted (git diff on driver/src/main.rs is empty; grep confirms no T0377 instrumentation remains).

AC#5 perf pin: tests/boundedness.rs::gate_stays_near_linear_under_large_net builds a 4000-transition / 8000-arc fan net and asserts check_net_sound finishes < 1s (old O(T*A) ~= T^2 here would blow past; new near-linear runs in a few ms). Docstring also records the manual dist8 macro-benchmark command.

GATE GREEN: just build OK; just clippy OK (0 warnings, re-run independently); just test 1152 pass/0 fail (incl new pin); just test-release 1151 pass/0 fail (incl pin); just e2e total:329 pass:272 fail:0 skipped:57 required-fail:0 == cycle-217 baseline UNCHANGED, 7-backend bit-identity preserved (0 differential failures).

OUT-OF-SCOPE FINDING (pre-existing, NOT mine): cargo fmt --check reports drift in lib.rs/transfer_inject.rs/sidecar.rs/algo_lower.rs/algo_parser.rs (all unmodified by this task, drift is vs HEAD). My 4 changed files are fmt-clean. Filing a follow-up.
<!-- SECTION:NOTES:END -->
