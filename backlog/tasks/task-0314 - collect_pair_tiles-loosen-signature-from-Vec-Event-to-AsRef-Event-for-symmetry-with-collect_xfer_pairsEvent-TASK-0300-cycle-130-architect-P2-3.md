---
id: TASK-0314
title: >-
  collect_pair_tiles: loosen signature from &Vec<Event> to AsRef<[Event]> for
  symmetry with collect_xfer_pairs(&[Event]) (TASK-0300 cycle-130 architect P2
  #3)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 07:56'
updated_date: '2026-05-25 08:00'
labels:
  - backend-common
  - refactor
  - hardening
  - forward-carried-from-TASK-0300
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0300 cycle 130 added `pub fn collect_pair_tiles<'a, I: IntoIterator<Item = &'a Vec<Event>>>(events_per_worker: I) -> BTreeMap<(DataId, SeqTag), IterTile>` in nucleus/backend-common/src/multi_worker_walker.rs (search for 'collect_pair_tiles' anchor; the helper sits adjacent to collect_xfer_pairs in the collect_* cluster).

The cycle-130 mped-architect review (P2 #3) flagged that the helper's `IntoIterator<Item = &'a Vec<Event>>` constraint requires a `&Vec<Event>`, not a `&[Event]`, while the underlying primitive `collect_xfer_pairs(events: &[Event], out: ...)` is the more permissive slice shape. A future caller folding a flat `Vec<Event>` (single concatenated stream) or a `Vec<Vec<Event>>` (test fixture or non-BTreeMap source) must own its Vecs to satisfy the current signature.

## What this task does

Loosen the helper signature to be symmetric with the underlying primitive:

```rust
pub fn collect_pair_tiles<'a, I, T>(events_per_worker: I) -> BTreeMap<(DataId, SeqTag), IterTile>
where
    I: IntoIterator<Item = &'a T>,
    T: AsRef<[Event]> + 'a + ?Sized,
{
    let mut out: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    for evs in events_per_worker {
        collect_xfer_pairs(evs.as_ref(), &mut out);
    }
    out
}
```

Existing callers (`per_worker.values()` where per_worker is `BTreeMap<WorkerId, Vec<Event>>`) yield `&Vec<Event>`; `Vec<T>: AsRef<[T]>` is standard, so existing call sites remain unchanged.

## Acceptance criteria

1. Helper signature widened as above.
2. All 4 backend call sites still compile + e2e baseline 108/92/0/16/0 preserved.
3. Add one new test to `nucleus/backend-common/tests/collect_pair_tiles.rs` that exercises the looser signature with a `Vec<&[Event]>` input (proves the impedance-removal is real).
4. The cycle-130 4 existing tests still pass unchanged.

## Honest scope

- LOW priority. Forward-looking. No current caller demands the looser signature.
- 0.5 cycle when picked up.
- Reason this was not folded into the TASK-0300 cycle-130 hardening commit: the cycle-130 fold-back rule in the orchestrator skill is for 'small, precise findings (missing assertion, doc overclaim, silent fallback)' — a signature change for hypothetical future callers crosses honest-scope into a follow-up.

## Cross-references

- nucleus/backend-common/src/multi_worker_walker.rs — `collect_pair_tiles` definition and adjacent `collect_xfer_pairs` primitive.
- nucleus/backend-common/tests/collect_pair_tiles.rs — 4 existing tests (cycle 130).
- TASK-0300 cycle 130 architect P2 #3.
- Memory: [[backend-common-crate-is-shared-codegen-home]] — backend-common is shared substrate across 4 tier-1 backends.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 131 plan:

1. Widen collect_pair_tiles signature in nucleus/backend-common/src/multi_worker_walker.rs from `IntoIterator<Item = &'a Vec<Event>>` to `IntoIterator<Item = &'a T>` where `T: AsRef<[Event]> + 'a + ?Sized`. Body becomes `collect_xfer_pairs(evs.as_ref(), &mut out)`. Existing per_worker.values() callers unchanged (Vec<Event>: AsRef<[Event]> is std).

2. Add one new test `vec_of_slices_input_compiles_and_collects` to nucleus/backend-common/tests/collect_pair_tiles.rs that constructs a `Vec<&[Event]>` and asserts collect_pair_tiles folds it correctly — proves the looser signature actually accepts a non-Vec source (not just compiles against the old call pattern).

3. Cheap gate: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e'. Baseline 108/92/0/16/0 MUST hold; the 4 cycle-130 tests must still pass.

4. Commit: 'backend-common + test: TASK-0314 cycle 131 — loosen collect_pair_tiles signature to AsRef<[Event]>'.

5. Parallel read-only review gate (qa-test-runner + mped-architect).

AC mapping:
- AC#1 (helper signature widened): step 1.
- AC#2 (e2e baseline preserved + 4 backends still compile): step 3.
- AC#3 (new test proves impedance removal): step 2.
- AC#4 (cycle-130 tests still pass unchanged): step 3 (cycle-130 tests use per_worker.values() → unchanged).
<!-- SECTION:PLAN:END -->
