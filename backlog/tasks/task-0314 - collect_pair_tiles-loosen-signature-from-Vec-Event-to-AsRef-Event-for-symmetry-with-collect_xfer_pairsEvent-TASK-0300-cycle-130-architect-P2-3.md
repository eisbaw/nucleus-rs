---
id: TASK-0314
title: >-
  collect_pair_tiles: loosen signature from &Vec<Event> to AsRef<[Event]> for
  symmetry with collect_xfer_pairs(&[Event]) (TASK-0300 cycle-130 architect P2
  #3)
status: To Do
assignee: []
created_date: '2026-05-25 07:56'
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
