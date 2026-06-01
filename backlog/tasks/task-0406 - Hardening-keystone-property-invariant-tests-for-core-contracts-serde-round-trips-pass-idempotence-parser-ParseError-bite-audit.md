---
id: TASK-0406
title: >-
  Hardening keystone: property/invariant tests for core contracts (serde
  round-trips, pass idempotence) + parser ParseError bite audit
status: To Do
assignee:
  - '@mark'
created_date: '2026-06-01 06:44'
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
