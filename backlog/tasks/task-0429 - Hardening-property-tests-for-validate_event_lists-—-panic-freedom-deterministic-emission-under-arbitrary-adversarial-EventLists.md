---
id: TASK-0429
title: >-
  Hardening: property tests for validate_event_lists — panic-freedom +
  deterministic emission under arbitrary/adversarial EventLists
status: Done
assignee:
  - implementer
created_date: '2026-06-02 20:17'
updated_date: '2026-06-02 20:53'
labels:
  - compiler
  - event-contract
  - hardening
  - property-test
  - cycle-246
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Backlog-maturity hardening wave (cycle-246). The full validate_event_lists (event_validate.rs, the production gate wired at driver/src/gate.rs via TASK-0422/0423) has only 10 hand-built #[test] bite cases (tests/event_validate.rs) and NO property test. Its two LOAD-BEARING contract claims are unproven under fuzzing: (1) the docstring 'Pure function; never panics on user-reachable input' — the gate at gate.rs relies on this (a panic there crashes the compiler on the very input it should DIAGNOSE, the exact panic-not-diagnostic class this project rejects); (2) deterministic, sorted error emission (the module doc at ~278-283 + the gate error message depend on it; cited repeatedly across cycles 242-245 but only positively tested). proptest is already a dev-dep and proptest_serde.rs:417-479 ALREADY has event_leaf()/event_strategy() generating arbitrary Event values (incl. nested Loop bodies) — reuse or model on it. SCOPE: add property tests (new tests/proptest_event_validate.rs, or a proptest! block in tests/event_validate.rs) generating arbitrary BTreeMap<WorkerId, Vec<Event>> (including malformed: self-push, unmatched pairs, empty/disagreeing Syncs, deeply-nested Loops) and assert: (P1 panic-freedom) validate_event_lists returns Ok/Err without panicking for ANY generated input; (P2 determinism) validating the same input twice yields an identical error Vec, AND validating a WORKER-ORDER-permuted input yields the identical error SET (BTreeMap iteration makes worker order irrelevant — pins cross-worker determinism); (P3 reorder-invariance, optional) permuting events WITHIN a worker does not change the set of UnmatchedPush/Wait or SyncParticipantDisagreement errors (set-keyed by (src,dst,data,tile,seq)/SyncTag — pins the safe_push_reorder soundness argument from TASK-0422.02 as a property). Keep it honest: if a property reveals a real non-determinism or panic, that is a REAL finding to root-cause, NOT a test to weaken. Pointers: nucleus-compiler/src/event_validate.rs (validate_event_lists:293, the emission-order doc, EventValidationError); nucleus-compiler/tests/proptest_serde.rs:417-479 (event strategy); nucleus-compiler/tests/proptest_petri.rs (proptest! house style + shrink config).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation Plan (implementer, cycle-246)

TEST-ONLY hardening. New file `nucleus-compiler/tests/proptest_event_validate.rs`. No production code change.

Strategy: copy/adapt the Event generators from proptest_serde.rs (proptest strategies are test-binary-local house style; not shared). Build a `BTreeMap<WorkerId, Vec<Event>>` strategy: 1..=4 workers (small WorkerId domain so matched Push/Wait + same-SyncTag collisions actually occur), each 0..=4 arbitrary Events incl recursive Loop bodies, Alloc/Free, empty/disagreeing Syncs, self-pushes.

Properties:
- P1 panic-freedom (PRIMARY): for any generated map, `validate_event_lists` returns Ok/Err without panic (harness catches panics). Same for `validate_event_lists_strict_per_worker` (debug_assert path). ProptestConfig 1024 cases.
- P2 determinism: validate twice == identical Vec (same errors, same order). Worker-order NOT permuted (BTreeMap canonicalizes; trivially invariant — per task note).
- P3 within-worker reorder-invariance: rotate events within each worker Vec; the SET of order-INDEPENDENT errors (UnmatchedPush/UnmatchedWait/SyncParticipantDisagreement/PushToSelf/EmptySyncParticipants) is unchanged. OverlappingAlloc/FreeWithoutAlloc are ORDER-SENSITIVE — EXCLUDED from the compared set (documented scoping).

Honest-failure: a panic / non-determinism / reorder-set-change is a REAL finding — root-cause, do not weaken.

Domain-narrowing rationale: WorkerId/DataId/SeqTag/SyncTag drawn from small domains (0..=3) so inv(2)/inv(6) matched arms are actually hit; assess coverage in final notes.

## Implementation complete (implementer, cycle-246) — all properties PASS

NEW FILE: nucleus-compiler/tests/proptest_event_validate.rs (test-only; ZERO production code change).

4 property-test fns, ProptestConfig::with_cases(1024) (validator is cheap; generous count for matched-arm coverage). Proptest default-seeded -> deterministic; ran >=2x stable.

- p1_validate_never_panics (P1 PRIMARY): validate_event_lists never panics on any generated BTreeMap<WorkerId,Vec<Event>>; assert Err is non-empty.
- p1_validate_strict_never_panics (P1): same for validate_event_lists_strict_per_worker (the acfg_to_events debug-assert path).
- p2_validate_is_deterministic (P2): two calls => IDENTICAL Vec (prop_assert_eq). Worker order NOT permuted (BTreeMap canonicalizes — trivially invariant, per task note).
- p3_within_worker_reorder_preserves_order_independent_errors (P3): rotate_left(1) within each worker Vec; the SET of ORDER-INDEPENDENT errors is unchanged.

### P3 variant-scoping (the real semantic distinction)
ORDER-INDEPENDENT (compared): UnmatchedPush, UnmatchedWait (cross-worker BTreeMap-indexed key), SyncParticipantDisagreement (SyncTag set-of-sets), EmptySyncParticipants (per-event), PushToSelf (per-event).
ORDER-SENSITIVE (EXCLUDED): OverlappingAlloc, FreeWithoutAlloc — depend on within-worker order (Free-before-Alloc vs after); may legitimately differ under reorder. Set keyed by Display string (variants are Eq but not Ord/Hash; Display prints every field => total deterministic textual identity).

### FINDINGS: NO property failed. No panic, no non-determinism, no reorder-set change surfaced. The validator docstring claim "never panics on user-reachable input" + deterministic emission both HOLD under 1024-case fuzzing (and a 4096-case throwaway coverage probe). No bug found; no follow-up task filed.

### Generator coverage (empirical, throwaway 4096-case probe, then removed)
ok=709 any_err=3387; per-arm hit counts: unmatched_push=4709 unmatched_wait=4705 disagree(inv6)=722 self_push=1159 empty_sync=939 overlap_alloc=281 free_no_alloc=4499. CONFIRMS the small-id-domain (0..=3) decision works: inv(2) matched+unmatched arms and inv(6) same-SyncTag disagreement are all exercised heavily — with u64-wide ids these collisions would be ~never hit. 709 Ok cases exercise the matched (non-error) Push/Wait closure.

### Gotchas / decisions
- COPIED (not shared) the Event strategy from proptest_serde.rs: proptest strategies are test-binary-local house style here; narrowed id domains for collision coverage. Loop arm kept (depth<=2) to fuzz the validators recursion; block_tag/check_frame=None (validator-irrelevant). Fire bindings=Default (validator ignores Fire).
- Display-string set key for P3: chosen because EventValidationError is Eq but neither Ord nor Hash; Display renders all fields.
- proptest "FileFailurePersistence ... failed to find lib.rs" stderr line appears under --nocapture for ALL proptest test binaries in this repo (petri=14, serde=2) — pre-existing house behaviour, harmless (only affects regression-seed persistence on failure), not introduced here.

### Gate (green)
just build OK; just clippy clean (-D warnings, re-run independently); dev test 1268/0/3 (was 1264, +4); release test 1266/0/3 (was 1262, +4); just e2e 385/328/0/57/0 EXACT (no regression); check-mega-files OK; check-include-str-coverage OK.

Cycle-246 review gate (parallel read-only): qa-test-runner GO + mped-architect GO. qa: e2e 385/328/0/57/0, dev 1268/0/3, release 1266/0/3, clippy clean, fences green, 4 property tests stable x3 runs (proptest seeded, no flake). architect ran an INDEPENDENT 4096-case probe corroborating non-vacuity: all 7 error arms fire (SyncParticipantDisagreement/inv6 667x, UnmatchedPush 4616, UnmatchedWait 4687, EmptySyncParticipants 982, PushToSelf 1175, OverlappingAlloc 288, FreeWithoutAlloc 4522, Ok 711) — small-id-domain (0..=3) claim VERIFIED true in committed strategy; P3 variant-scoping verified correct against event_validate.rs:513-594 (Alloc/Free order-sensitive correctly EXCLUDED; Unmatched*/SyncParticipantDisagreement/EmptySyncParticipants/PushToSelf order-independent correctly INCLUDED); no prop_assume!/no #[ignore]/no production code touched. P1: none. P2: none. P3-1 (Display-set-key comment overclaimed "every field printed" — renders tile.rank() not tile contents) + P3-2 (copied Event strategy lacked the completeness guard proptest_serde.rs carries — silent-sibling trap) BOTH FIXED IN-THREAD commit 70f5e9b (gate re-run green: e2e 385/328/0/57/0, dev 1268/release 1266, clippy clean). P3-3 (matched-pair/Ok-closure arm thinly exercised ~1% of cases — acceptable for a panic/determinism/reorder suite; flagged so a future strengthen-matched-pair-coverage idea is not lost, NO task filed). Status stays Done.
<!-- SECTION:NOTES:END -->
