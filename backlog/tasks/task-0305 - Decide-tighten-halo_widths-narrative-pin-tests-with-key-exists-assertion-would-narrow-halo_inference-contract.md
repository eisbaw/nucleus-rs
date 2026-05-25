---
id: TASK-0305
title: >-
  Decide: tighten halo_widths narrative-pin tests with key-exists assertion
  (would narrow halo_inference contract)
status: To Do
assignee: []
created_date: '2026-05-25 03:09'
labels:
  - M5
  - compiler
  - test-coverage
  - halo_inference
  - contract-decision
  - forward-carried-from-TASK-0303
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Cycle-120 architect review-gate flagged a project-wide soundness gap shared by all three current halo_widths narrative-pin tests (task0299_06_*, task0303_05_*, task0303_07_*): all three use .unwrap_or(0) on the contract degree of freedom (halo_inference's contract at halo_inference.rs:53-57 permits either explicit-0 entry OR omission). A regression that silently produced NO entries for the pinned kernels would pass all three tests vacuously — they cannot distinguish 'inspected, halo 0' from 'not inspected at all'.

## The trade-off (decision required)

**Option A — TIGHTEN the tests** (add a key-exists assertion):
- Each test additionally asserts that halo_widths[kid] entry IS PRESENT (independent of value).
- Cost: NARROWS the halo_inference contract degree of freedom from 'explicit-0 OR omission' to 'explicit-0 ONLY' for the pinned kernels.
- Today's implementation chooses explicit-0 per no_halo_bare_iv (halo_inference.rs:1184), so the tightening is observationally inert today.
- Risk: a future refactor that legitimately toggles halo_inference's representation toward omission (contract-permitted) would break these tests.

**Option B — RECORD the contract degree of freedom as the design choice** (defer Option A):
- The existing tests remain robust to contract-permitted representation toggles.
- The vacuous-pass risk on a silent-skip regression remains, but is judged unlikely (halo_inference's walker pattern doesn't lose entries in practice).
- The contract is the protection against test coupling.

**Option C — STRENGTHEN the contract** (separate, larger change):
- Promote the contract from 'absence OR explicit-0 are equivalent' to 'every inspected (kernel, iv) MUST have an explicit entry'.
- Pin the new contract with a separate test (assert that for every kernel mentioned in the algorithm under a for-loop, halo_widths has a record).
- THEN Option A becomes consistent with the contract.

## Acceptance criteria

1. Project decision: A, B, or C. Record as a decision note in CLAUDE.md or as a code comment at halo_inference.rs:53-57 (or PRD §X).
2. If A: add the key-exists assertion to task0299_06_*, task0303_05_*, task0303_07_* AND update the docstrings to disclose the narrowing. ~3 lines per test.
3. If B: add a one-line note to each test docstring explicitly acknowledging the soundness floor (vacuous-pass on silent-skip is acceptable per contract).
4. If C: this becomes a multi-task arc — halo_inference contract change + sidecar contract change + new contract-pin tests + (A) hardening as the final cycle.

## Honest scope

LOW priority. The risk this defends against (halo_inference's walker silently losing entries) is not a known failure mode today. The decision is more about long-term test-suite philosophy than immediate correctness.

## Cross-references

- TASK-0299 (cycle 119, Done) — first narrative-pin precedent.
- TASK-0303 (cycle 120, Done) — sibling-sweep that exposed the project-wide pattern.
- halo_inference.rs:53-57 — the contract degree of freedom in question.
- cycle-120 architect review-gate Recommendation #2.
<!-- SECTION:DESCRIPTION:END -->
