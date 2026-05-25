---
id: TASK-0314
title: >-
  collect_pair_tiles: loosen signature from &Vec<Event> to AsRef<[Event]> for
  symmetry with collect_xfer_pairs(&[Event]) (TASK-0300 cycle-130 architect P2
  #3)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 07:56'
updated_date: '2026-05-25 08:12'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 131 review-hardening fold-back (commit e03a626 + this commit)

Mandatory parallel read-only review gate (qa-test-runner + mped-architect, in parallel):

### qa-test-runner: GO
- just build / clippy / test (859/0/3) / test-release (859/0/3 — identical to dev) / e2e (108/92/0/16/0 on BOTH consecutive runs, deterministic) / check-textual-replace-on-codegen / check-include-str-coverage — all 7 arms green.
- The 5 collect_pair_tiles tests all pass (4 from cycle 130 + vec_of_slices_input_compiles_and_collects new this cycle).
- ?Sized + AsRef widening landed with zero clippy warnings and zero rustc inference noise.

### mped-architect: GO (1 P2 + sundry P3 mention-only)

Folded in-thread on this same cycle:

- **P2-1** (FIXED): comment-doc-lie regression introduced by THIS cycle's widening. The cycle-130 docstring 'Determinism of first under hypothetical drift' paragraph asserted as a helper-contract property that callers pass per_worker.values() from a BTreeMap<WorkerId, Vec<Event>>. After cycle 131 widened the type contract, that BTreeMap-specific framing describes only one of multiple valid callers (the new vec_of_slices test deliberately uses a Vec<&[Event]> whose order is insertion-order, not WorkerId-ascending). Docstring restructured with a '# Contract' section (first-sighting under input iterator order; helper has no opinion about that order) separated from a '# Current-caller convention (informational, not part of the contract)' section (the 4 backends' BTreeMap::values() pattern). Defends against [[feedback-comment-doc-lie-recurring]] firing on a docstring that the cycle's own signature change made stale.

P3 findings (mention-only, no fold-back):
- P3-1: ?Sized bound load-bearing for the new test (T = [Event] needs it); architect verified correct.
- P3-2: test regression-sensitivity premise validated by architect (Vec<&[Event]> would fail to compile under hypothetical narrowing back to &Vec<Event>).
- P3-3: honest scope clean.
- P3-4: no cross-impact regression risk.

### Honest scope at AC level

All 4 ACs of TASK-0314 met by cycle 131:
- AC#1 (signature widened to AsRef<[Event]>): commit e03a626.
- AC#2 (4 backends still compile + e2e baseline preserved): commit e03a626 (per_worker.values() still satisfies the bound via Vec<Event>: AsRef<[Event]> std blanket).
- AC#3 (new test proves impedance removal): commit e03a626 (vec_of_slices_input_compiles_and_collects with Vec<&[Event]>::iter().copied()).
- AC#4 (cycle-130 4 tests unchanged): commit e03a626 (all 5 tests green, including the 4 cycle-130 ones).

### Forward-carry / lessons feed-forward

- Lesson for any future cycle that widens a generic signature on backend-common: the SAME cycle MUST audit the cycle-N docstring for narrative-tense / contract-vs-convention drift. The cycle-131 P2-1 demonstrates the pattern fires on docstrings the *current cycle's own* widening invalidates — not just legacy docs. Worth one re-read of every docstring on the changed symbol BEFORE committing.
- The architect P2-2 sibling-helper observation (only collect_pair_tiles is generic; collect_xfer_pairs / collect_worker_rendezvous / collect_barriers_by_tag / collect_pre_init_sets all take &[Event] directly) is the correct shape for those helpers — they already take the most permissive slice shape. The asymmetry is correct and DOES NOT need follow-up; if a future cycle hoists ANOTHER multi-worker fold helper (one that walks the per-worker outer iterator), it should match the cycle-131 AsRef<[Event]> generic shape.

## Cycle 131 status

In Progress → Done after the review-hardening fold-back lands on a green re-gate (108/92/0/16/0 preserved through the hardening commit).
<!-- SECTION:NOTES:END -->
