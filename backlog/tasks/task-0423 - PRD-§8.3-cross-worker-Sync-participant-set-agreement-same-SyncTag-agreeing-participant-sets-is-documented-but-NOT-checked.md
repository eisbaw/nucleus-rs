---
id: TASK-0423
title: >-
  PRD §8.3: cross-worker Sync participant-set agreement (same SyncTag =>
  agreeing participant sets) is documented-but-NOT-checked
status: To Do
assignee: []
created_date: '2026-06-02 02:27'
updated_date: '2026-06-02 16:06'
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
<!-- SECTION:NOTES:END -->
