---
id: TASK-0423
title: >-
  PRD §8.3: cross-worker Sync participant-set agreement (same SyncTag =>
  agreeing participant sets) is documented-but-NOT-checked
status: Done
assignee:
  - claude
created_date: '2026-06-02 02:27'
updated_date: '2026-06-02 18:34'
labels:
  - compiler
  - event-contract
  - prd-invariant-audit
  - cycle-241
dependencies:
  - TASK-0428
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) GAP-4, VERIFIED. event_validate.rs:48-54 openly states two Sync events sharing a SyncTag on different workers should have AGREEING participant sets, but the module does NOT check that agreement. With SyncTag now the cross-worker barrier join-key (TASK-0172), a backend lowering a partial/non-uniform barrier trusts agreement that nothing verifies. SCOPE: extend the cross-worker phase of validate_event_lists with a SyncTag->participant-set consistency check; new EventValidationError::SyncParticipantDisagreement { sync }; bite-test. INHERITS GAP-2 (TASK-0422): only meaningful once validate_event_lists is actually wired to a production caller. Low value (latent; no shipping schedule produces disagreeing sets today).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0428 (cycle-242): TASK-0428 (your transitive prerequisite via TASK-0422) is resolved — inv(2) Push/Wait pairing now holds corpus-wide (premise stale, not a real gap). Your dependency chain note still holds: this SyncTag participant-set agreement check is only meaningful once validate_event_lists has a production caller (TASK-0422), and TASK-0422 now has a concrete two-step path (confirm post-mediation inv(2) for mp-* backends, then wire the validator). No code change here; still latent (no shipping schedule produces disagreeing participant sets — TASK-0428 sweep would have surfaced an EmptySyncParticipants but not a disagreement, since that variant does not exist yet).

Implementation plan (cycle-245):
1. Add EventValidationError::SyncParticipantDisagreement { sync: SyncTag } (mirrors EmptySyncParticipants shape) + Display arm.
2. Collect BTreeMap<SyncTag, BTreeSet<BTreeSet<WorkerId>>> across all workers + Loop recursion: in the existing Event::Sync arm of walk_events, insert participants under sync. Thread &mut through check_per_worker/walk_events like pushes/waits.
3. Cross-worker phase of validate_event_lists ONLY: iterate map in BTreeMap (SyncTag) order, push one SyncParticipantDisagreement for every tag with >1 distinct participant set. Emitted after Push/Wait cross-worker errors. NOT added to validate_event_lists_strict_per_worker (cross-worker check) nor the petri_to_events debug_assert.
4. Module doc: move the gap from What-is-NOT-checked to invariant (6) under What-is-checked; fix stale "does NOT yet check" text.
5. Bite test in tests/event_validate.rs: two Sync same tag different participants => Err(SyncParticipantDisagreement); positive guard: same tag identical participants => no disagreement.
Acceptance: e2e MUST hold 385/328/0/57/0 (corpus proof no shipping schedule disagrees).

DONE cycle-245 (commit 81f85ab). Implementation as planned.

Gotchas / subtleties / decisions:
- Data structure: BTreeMap<SyncTag, BTreeSet<BTreeSet<WorkerId>>>. BTreeSet<WorkerId> is Ord, so BTreeSet-of-BTreeSet is a valid set-of-DISTINCT-sets; len()>1 == disagreement. Deterministic iteration (BTreeMap by SyncTag) gives stable emission order. Chosen over a "first set wins, compare rest" approach because set-of-distinct-sets is simpler and naturally dedups identical views from many workers.
- Cross-worker, therefore lives ONLY in validate_event_lists, NOT in validate_event_lists_strict_per_worker nor the petri_to_events:269 debug_assert (mirrors invariant (2) exactly). The strict subset still THREADS a throwaway sync_participants map through check_per_worker (single walker, double-duty) but never consumes it -- documented inline.
- Empty participant set is still recorded under its tag (so empty-vs-nonempty also surfaces as a disagreement); the pure-empty case is additionally caught by invariant (3) EmptySyncParticipants. No double-count problem since they are distinct variants.
- #[allow(clippy::too_many_arguments)] added to walk_events (now 7 params) -- the alternative (a context struct) was rejected as churn for a private 2-call-site helper.
- Bite test genuinely bites: neg_ map is otherwise clean (two valid Syncs, no Push/Wait), so removing the cross-worker closure makes expect_err fail. pos_ guard proves the check keys on DISTINCT sets not on seeing the tag twice.

Gate (actual numbers): e2e 385/328/0/57/0 (UNCHANGED -- corpus proof: the new HARD gate on every codegen build for all 7 backends rejected ZERO shipping schedules; no cell disagrees). dev tests 1262/0/3 (+2), release 1262... wait release 1260/0/3 (+2). clippy -D warnings clean (independently re-run). build + test + test-release + e2e all green.

Honest limits: the check is LATENT (no current schedule produces disagreeing sets) -- confirmed empirically by green e2e, NOT by exhaustive proof. It is a tripwire against future partial/non-uniform-barrier codegen that mis-computes a participant set on one worker. Defense-in-depth only: the existing corpus sweeps (tests/petri_to_events.rs::task0428_*, driver/tests/task0422_01_*) now also implicitly assert no disagreement since they call validate_event_lists; no new sweep added (e2e covers end-to-end). No new follow-up tasks needed -- no stub/shortcut.
<!-- SECTION:NOTES:END -->
