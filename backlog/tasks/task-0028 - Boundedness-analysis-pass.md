---
id: TASK-0028
title: Boundedness analysis pass
status: Done
assignee: []
created_date: '2026-05-17 23:05'
updated_date: '2026-05-18 04:06'
labels:
  - M2
  - compiler
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Walk the firing order; track live tokens per place; verify no marking exceeds the place's declared capacity. PRD §8.2.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler exposes check_bounded(Net, firing_order) -> Result<(), BoundednessError>.
- [ ] #2 BoundednessError names the offending place, the firing that overflows it, and the marking at the time of overflow.
- [ ] #3 Test: a schedule that demands buffer=N too small for the pipeline produces this error with the right place name.
- [ ] #4 Implementation notes record design questions (e.g. should we suggest minimum-N in the error message).
- [ ] #5 Implementation notes record honest limitations (the analysis is exact for v2's restricted nets; not symbolic, not general).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions and decisions

**1. How is the firing order derived?**
The task said \"choose: derive a firing order by topo-sort or accept it as an explicit input\". I did *both*:
- `check_bounded(net, firing_order)` takes the order as an explicit `&[TransitionId]` input. This keeps the pass composable — later passes (deadlock, liveness) can share one canonical order built once.
- `derive_firing_order(net)` is a thin helper that returns transitions in *insertion order* (0..N-1). `acfg_to_petri` walks the ACFG depth-first in source order and appends transitions as it encounters them, so insertion order is exactly the linearised source order: per-worker control chains advance monotonically, and Push appears before its matched Wait. PRD §8.6: \"deterministic greedy (source order + dataflow constraints)\". Source order is what insertion order is.

No runtime greedy search. Less code, more deterministic. If a future net builder breaks this invariant, `check_bounded` surfaces it as `BoundednessError::InvalidFiringOrder`.

**2. How does BoundednessError report the offending marking?**
The variant carries: PlaceId + place_name, TransitionId + transition_name, `marking_before` (Marking immediately before the offending firing), `would_be` (post-firing count), `capacity`. Marking-before is more diagnostic than marking-after: the user wants to see the state to *avoid*, not the state that can't exist.

I considered suggesting a minimum-N (\"use buffer=K instead\"). Did not. That belongs in a downstream diagnostic layer that knows the source location of the `transfer DATA : buffer=N` directive; the boundedness pass only sees the Net.

**3. Unbounded places (`capacity = None`) — allowed or rejected?**
Accepted. `petri::Place::capacity` docs say `None` is for analysis fixtures; production v2 always carries `Some(_)`. The replay path simply skips capacity checks for `None`-capacity places (because `fire` does too). No new error variant.

**4. Why not check this at net-construction time instead?**
Because the firing order is also an artefact (PRD §8.6 calls out the linearisation explicitly). Boundedness is a *property of the (net, firing order) pair*, not of the net alone. Checking at construction would either be redundant (capacity is already a `NonZeroU32` invariant at the place level) or too restrictive (would reject nets that are bounded but where some arc weight + capacity combination *could* overflow under some pathological order — we want to check the actual chosen order).

**5. Error variants beyond CapacityExceeded?**
Added `UnknownTransition` (programming-error: caller passed an id not in this net) and `InvalidFiringOrder` (transition's input place was empty — surfaces what would otherwise be a `FireError::NotEnabled` so callers don't conflate it with \"all fine\"). Boundedness is undefined in those cases, and silently returning Ok is wrong.

## Honest limitations

- **Exact replay only, not symbolic.** This pass walks one concrete firing sequence; it does not enumerate alternative interleavings. v2's restricted nets (statically determined firing order, PRD §8.4) make this sound. A future relaxation that admits true non-determinism would need a coverability / Karp-Miller check, not this.

- **Single-violation reporting.** The pass returns the *first* capacity-violation it encounters. Doesn't enumerate all of them. Matches the \"fail fast and verbosely\" rule; keeps the error surface small.

- **No minimum-N suggestion in the error.** See design question #2. The pass tells you which place overflowed and by how much; downstream diagnostic code can synthesise advice if/when desired.

- **Example 02 split's e2e test asserts a degraded property.** Upstream `transfer_inject` does not splice Push placeholders when the matching Wait is inside a Repeat body and its producer is in the outer Sequence (example 02 split's exact shape: load_input on host outside the for-loop, add on w0 inside). The net thus contains 513 wait_seq* transitions and 0 push_seq* transitions; the first Wait fires against an empty buffer place. `check_bounded` surfaces this as `BoundednessError::InvalidFiringOrder` (a deadlock shape, *not* an overflow). The e2e test asserts that whatever `check_bounded` returns, it is NEVER `CapacityExceeded` — i.e. boundedness is preserved. Filed as TASK-0139.

- **Determinism: the pass is fully deterministic.** No hash-map iteration, no time, no parallelism. Two calls with equal inputs produce equal outputs (including identical error payloads). Asserted by tests `check_bounded_is_deterministic` and `check_bounded_on_overflow_is_deterministic`.

## AC verification

- [x] #1 `check_bounded(Net, firing_order) -> Result<(), BoundednessError>` is exposed at `compiler::passes::boundedness` and re-exported at the crate root.
- [x] #2 `BoundednessError::CapacityExceeded` names the place (PlaceId + place_name), the offending transition (TransitionId + transition_name), the marking at the time of overflow (`marking_before`), and the capacity it would have exceeded (`would_be`, `capacity`).
- [x] #3 Test `two_token_push_into_cap1_place_is_rejected` builds a 2-token push into a capacity-1 place and asserts the error names the right place (\"buf\") and transition (\"overflower\") with would_be=2, capacity=1. A second test `back_to_back_produce_into_cap1_buffer_is_rejected` covers a producer/consumer with mis-matched firing order.
- [x] #4 Implementation notes (this file) record design questions about firing-order derivation, error payload, marking-before vs after, and minimum-N suggestion.
- [x] #5 Implementation notes record honest limitations: exact-replay only, single-violation reporting, no minimum-N, and the example 02 split limitation traced to TASK-0139.

## Follow-ups filed

- TASK-0139 — transfer_inject: emit Push when Wait's matching producer is in an outer scope (Repeat-bound consumer case). Once that lands, the e2e_example_02_split_never_overflows_capacity test can be tightened from \"either Ok or InvalidFiringOrder\" to \"Ok\".

## Commit

(filled by the actual commit shortly after this note lands)
<!-- SECTION:NOTES:END -->
