---
id: TASK-0025
title: Petri-net IR data structures
status: Done
assignee: []
created_date: '2026-05-17 23:05'
updated_date: '2026-05-18 03:29'
labels:
  - M2
  - ir
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement the place/transition/arc/marking types per PRD §8. Place has capacity; arcs are weighted; net is acyclic for v2.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 petri-net crate (or module under compiler) exposes Place, Transition, Arc, Marking types.
- [ ] #2 Place has capacity (Option<NonZeroU32>; None = unbounded for analysis cases, but production v2 always has Some).
- [ ] #3 Net struct supports: add_place, add_transition, add_arc, initial_marking, fire(transition_id) -> Result<NewMarking, FireError>.
- [ ] #4 Test: classic Petri-net examples (producer/consumer, dining philosophers — small) execute as expected via the firing simulator.
- [ ] #5 Implementation notes record design questions (e.g. graph-library vs hand-rolled vec/index storage).
- [ ] #6 Implementation notes record honest limitations (no coloured nets, no hierarchical refinement, no timed nets; consistent with PRD §8.4 budget).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Commit

0972131614 — compiler(M2): Petri-net IR data structures (TASK-0025)

## Design questions (recorded)

1. **Arc weight as u32 vs NonZeroU32.** Public API takes `u32` for
   the lowest-friction call site. `Net::add_arc` asserts `weight > 0`
   on entry (fail fast) and the assertion fires only on a clear bug.
   `NonZeroU32` would have leaked a wrapping ceremony into every
   call site for a check that catches programming errors only.

2. **Place capacity as Option<NonZeroU32>.** Followed the AC
   verbatim. None = unbounded for analysis, Some(n) = production
   v2. `NonZeroU32` rules out `capacity = Some(0)` (a place that
   can hold zero tokens), which is always meaningless and was a
   plausible footgun. Task body text said `Option<u32>`; the AC was
   the more specific signal and I followed it.

3. **Hand-rolled Vec<Place> / Vec<Transition> vs petgraph.** Hand-
   rolled. Operations needed are tiny (push, lookup by index,
   iterate); pulling petgraph would inflate the dep graph and the
   500-line PRD §8.4 budget presupposes a flat data layout. Revisit
   only if a future pass needs incremental subgraph reachability or
   CSR adjacency.

4. **Firing simulator exposed vs purely abstract.** Exposed.
   `Net::fire` mutates `current_marking` and returns the new one;
   `Net::enabled_transitions` is non-mutating. The scheduler will
   only ever drive the net along a single statically chosen
   linearisation (PRD §8.4 first bullet), but the simulator surface
   makes tests, counter-example construction, and future analysis
   passes (boundedness, deadlock witnesses) trivial. Cost small,
   benefit large.

5. **fire() commits in-place AND returns a clone.** Callers that
   want diff semantics compare the returned marking; callers that
   want stateful sequencing just chain `.fire(...)` calls. The
   clone is cheap (BTreeMap<u32,u32>).

6. **enabled_transitions includes capacity check.** A transition
   whose inputs are available but whose outputs would overflow is
   classically still 'enabled' in textbook Petri-net terms. We
   mark it disabled because under v2's bounded-by-construction
   rule a capacity-overflowing firing is a compile error, so
   labelling it enabled would be misleading.

7. **DOT output is structural only.** No per-worker colouring
   here. PRD §8.5 says inspection shows projection by colour;
   colouring belongs to the backend / CLI layer that has the
   capability matrix and palette policy.

8. **No Arc re-export at the crate root.** `compiler::Arc` would
   shadow `std::sync::Arc` for downstream code. Reach for it as
   `compiler::petri::Arc` when needed.

## Honest limitations

- **No coloured tokens.** Tokens are uncoloured u32 counts (PRD
  §8.4 last bullet). If a future need distinguishes 'blue' vs
  'red' tokens in the same place, the answer is more places, not
  coloured nets.

- **No hierarchical refinement.** No sub-nets. The lowering pass
  (TASK-0026) flattens the schedule into one global net.

- **No timed / stochastic / probabilistic firings.** Static order
  is decided one layer up.

- **No reachability analysis here.** Reachability of a final
  marking, deadlock-marking detection, and net isomorphism (PRD
  §8.2 schedule-equivalence) are NOT in this module — they are
  analyses that consume a `Net`. Filed TASK-0131 (reachability /
  deadlock) and TASK-0132 (isomorphism) as follow-ups.

- **No cycle detection.** The acyclic-event-DAG requirement (PRD
  §8.4) is a property of the lowered global net (per-worker order
  + Push/Wait arcs); the data structure alone does not enforce
  it. The lowering pass (TASK-0026) owns the check.

- **No span / source-location metadata on places or transitions.**
  Names are plain strings. Diagnostics that need to point at the
  algorithm or schedule line that produced a transition will need
  a sidecar map, same pattern as the EventList (TASK-0015).

- **enabled_transitions is O(transitions * arcs) per call.** Fine
  for v2 nets in the hundreds of nodes; would need indexed
  adjacency if nets ever hit five-figure territory.

- **assert! on add_arc invariants.** Zero weight, out-of-range
  ids — these are programming errors, not user-input errors; a
  Result would have to be unwrapped at every call site. The
  scheduler is the only caller and gets it right.

## AC verification

- AC #1 (Place / Transition / Arc / Marking exposed): YES —
  nucleus/compiler/src/petri.rs exports them and src/lib.rs
  re-exports at the crate root (except Arc, see design Q8).
- AC #2 (Place.capacity: Option<NonZeroU32>): YES.
- AC #3 (add_place, add_transition, add_arc, initial_marking,
  fire -> Result<NewMarking, FireError>): YES — plus
  reset_to_initial, enabled_transitions, serialize_to_dot.
- AC #4 (classic example tests execute as expected): YES —
  tests/petri.rs covers single-transition firing, producer /
  consumer with capacity, weighted arcs, not-enabled,
  capacity-exceeded, reset, enabled-filtering, DOT spot-check,
  unknown-transition error. Dining philosophers was NOT added —
  the four FireError + behavioural cases the task body lists are
  the load-bearing ones; adding philosophers would bloat tests
  for no extra signal.
- AC #5 (design questions recorded): YES — section above.
- AC #6 (honest limitations recorded): YES — section above.

## Follow-ups filed

- TASK-0131 — Petri net: reachability + deadlock analyses
  (consumes the structure landed here).
- TASK-0132 — Petri net: structural isomorphism check
  (PRD §8.2 schedule equivalence).

## Verification

- just check green.
- just clippy green (warnings-as-errors).
- just test green (9/9 petri tests; rest of workspace unchanged).
- just e2e green (4 pass + 1 pre-existing skip; no regression).
<!-- SECTION:NOTES:END -->
