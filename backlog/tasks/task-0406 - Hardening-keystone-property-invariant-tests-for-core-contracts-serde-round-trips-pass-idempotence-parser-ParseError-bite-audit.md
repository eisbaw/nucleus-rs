---
id: TASK-0406
title: >-
  Hardening keystone: property/invariant tests for core contracts (serde
  round-trips, pass idempotence) + parser ParseError bite audit
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 06:44'
updated_date: '2026-06-01 07:35'
labels:
  - hardening
  - testing
  - property-tests
  - serde
  - parser
  - cycle-236-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-236 next-keystone after the prove-the-check-bites wave SATURATED (TASK-0400/0401/0402/0404: every genuinely-reachable typed error variant across passes + lowering + sched + link + emit now has a bite test or a documented unreachability pin). The next named hardening dimension (phase3 skill: fuzzing and property tests for core contracts) NOT yet systematically done:

1. SERDE ROUND-TRIPS: property/round-trip tests for the serialised contracts. Some exist (halo_widths_serde_roundtrip in sidecar_halo.rs; reuse sibling). AUDIT which serialisable contract types (NameSidecar, the Event contract incl Event::Sync SyncTag, ACFG sidecar, reuse_widths) have a round-trip pin and which do not; add round-trip + required-field-contract-version tests for the gaps. Memory: Event::Sync had a serde required-field contract-version caveat (project-event-sync-synctag).

2. PASS IDEMPOTENCE / DETERMINISM: where a pass is expected to be idempotent (running twice == once) or deterministic (same input -> byte-identical output), add a property pin. Codegen determinism is already e2e bit-identity-pinned; the COMPILER-PASS layer (transfer_inject, sync_inject, partition passes) determinism is less explicitly pinned. chumsky parser error-determinism was pinned by TASK-0399.

3. PARSER ParseError BITE AUDIT: TASK-0399 fuzz-tested parse_algo/parse_sched for panic-freedom + ParseErrors-non-empty + error-determinism, but did NOT confirm per-error-SHAPE bite coverage. Audit whether the distinct parse-error shapes the grammar can produce each have a representative negative test. NOTE the chumsky error type is a Simple-based single type, not a rich variant enum, so per-variant may be N/A; confirm and document.

METHOD (forward-carried, load-bearing): coverage/inventory audits silently UNDER-count -- see feedback-coverage-audit-undercount-recurring (cycle-236 fired twice: Explore missed inline cfg(test) mods; awk window truncated an enum). Re-derive every denominator from a STRUCTURAL boundary; grep BOTH tests/ dirs AND inline cfg(test) mods; a matching/round count is not corroboration; verify with an independent re-count.

LOWER leverage than the typed-error-enum wave that is now done; this is the next genuine hardening avenue, not loop-filler. Best STARTED IN A FRESH CONTEXT (the cycle-236 session did two full cycles + 4 subagent reviews; the serde-type inventory is a substantial fresh read).
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DELIVERED (commit 6062673): bite test `negative_unexpected_eof_kind_on_truncated_input` added to BOTH tests/algo_parser.rs and tests/sched_parser.rs, asserting an EOF-truncated construct classifies as ParseErrorKind::UnexpectedEof (algo: `const N : usize =`; sched: an opened `schedule { ` block). Mutation-proven in both (flip expected variant to Unexpected -> fails at algo_parser.rs:746 / sched_parser.rs:735). qa GO (build/clippy 0/0/test 1236 dev/1235 rel/e2e 385/328/0/57/0 x2 stable) + architect GO (empirically ran both inputs: exactly 1 error each, kind=UnexpectedEof, column at EOF; both ParseErrorKind variants now bitten in both suites = parser kind-coverage complete).

AUDIT RESULT (the 3-part contract-test dimension):
- PART 1 SERDE ROUND-TRIPS: SATURATED. Event 7/7 variants round-tripped incl nested Loop + serde-default missing-field path; capabilities.toml (the ONLY production serde boundary) parse + round-trip + 6 negatives; NameSidecar byte-identical round-trip + old-wire deserialize. Architect-confirmed no other production serialize path exists (Event/ACFG/sidecar serde is feature-gated, test-only) so further round-trip tests are low-value. One OPTIONAL low-value residual: Event::Loop with populated Some(block_tag)/Some(check_frame) serialize->deserialize not round-tripped (no production JSON path => doc-and-skip defensible).
- PART 2 DETERMINISM/IDEMPOTENCE: SATURATED. Determinism pinned on every production pass (block_transform, all 3 partition passes, reuse/halo inference, acfg_to_net via acfg_to_petri, check_net_sound, acfg_to_events, inject_check_frames, parser proptest x4); idempotence pinned where it is a real invariant (inject_syncs, host_mediation x2, host_data_relay, safe_push_reorder, transfer_inject). My "transfer_inject not idempotent" guess was wrong (it HAS idempotence pins) -- strengthens saturation.
- PART 3 PARSER ParseError: was the REAL gap. ParseErrorKind is a 2-variant enum (Unexpected, UnexpectedEof; error.rs:111); Unexpected bite-asserted 8x, UnexpectedEof had ZERO despite being production-reachable. NOW closed (both parsers).

CONCLUSION: the property/contract-test hardening dimension is EXHAUSTED. Combined with the prior cycles (TASK-0400/0401/0402/0404 typed-error-enum bite coverage SATURATED; doc-citation fence sub-wave saturated; parser fuzz TASK-0399), the test-coverage hardening wave is genuinely complete. Remaining hardening avenues are REVIEW-PASS type (dead-code/limitation audit; doc-invariant-assertion audit) -- filed TASK-0407 + TASK-0408 -- and the feature backlog is environment-blocked (MPI/Renode) or grammar-epic-deferred.

HONEST-FAILURE DISCLOSURE (architect adversarial-verify P3, corrected not hidden): my first-pass audit called the parser error "a single Simple-based type, per-variant N/A" -- FALSE. It is a 2-variant enum with one un-bitten arm. This was my THIRD coverage under-count this session (after the TASK-0402 Explore-missed-inline-tests and TASK-0404 awk-truncated-enum). ALL THREE caught by the read-only review gate -- the safety net working exactly as designed. The pattern (memory feedback-coverage-audit-undercount-recurring) held a fourth time: never assume a "single type / N/A" without re-deriving from the structural definition. The adversarial-verify framing (ask the reviewer to FALSIFY a saturation claim, not confirm it) is what surfaced the gap -- a saturation claim is the highest-risk overclaim and must be adversarially checked, not self-certified.
<!-- SECTION:FINAL_SUMMARY:END -->
