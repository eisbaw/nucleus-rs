---
id: TASK-0305
title: >-
  Decide: tighten halo_widths narrative-pin tests with key-exists assertion
  (would narrow halo_inference contract)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 03:09'
updated_date: '2026-05-25 05:53'
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

Cycle-120 architect review-gate flagged a project-wide soundness gap shared by all three current halo_widths narrative-pin tests (task0299_06_*, task0303_05_*, task0303_07_*): all three use .unwrap_or(0) on the contract degree of freedom (halo_inference's contract at the "TASK-0305 cycle-122 project decision (Option B)" paragraph in halo_inference.rs (search for "absent ≡ explicit-0") permits either explicit-0 entry OR omission). A regression that silently produced NO entries for the pinned kernels would pass all three tests vacuously — they cannot distinguish 'inspected, halo 0' from 'not inspected at all'.

## The trade-off (decision required)

**Option A — TIGHTEN the tests** (add a key-exists assertion):
- Each test additionally asserts that halo_widths[kid] entry IS PRESENT (independent of value).
- Cost: NARROWS the halo_inference contract degree of freedom from 'explicit-0 OR omission' to 'explicit-0 ONLY' for the pinned kernels.
- Today's implementation chooses explicit-0 per the in-module test `fn no_halo_bare_iv` in halo_inference.rs, so the tightening is observationally inert today.
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

1. Project decision: A, B, or C. Record as a decision note in CLAUDE.md or as a code comment at the "TASK-0305 cycle-122 project decision (Option B)" paragraph in halo_inference.rs (search for "absent ≡ explicit-0") (or PRD §X).
2. If A: add the key-exists assertion to task0299_06_*, task0303_05_*, task0303_07_* AND update the docstrings to disclose the narrowing. ~3 lines per test.
3. If B: add a one-line note to each test docstring explicitly acknowledging the soundness floor (vacuous-pass on silent-skip is acceptable per contract).
4. If C: this becomes a multi-task arc — halo_inference contract change + sidecar contract change + new contract-pin tests + (A) hardening as the final cycle.

## Honest scope

LOW priority. The risk this defends against (halo_inference's walker silently losing entries) is not a known failure mode today. The decision is more about long-term test-suite philosophy than immediate correctness.

## Cross-references

- TASK-0299 (cycle 119, Done) — first narrative-pin precedent.
- TASK-0303 (cycle 120, Done) — sibling-sweep that exposed the project-wide pattern.
- the "TASK-0305 cycle-122 project decision (Option B)" paragraph in halo_inference.rs (search for "absent ≡ explicit-0") — the contract degree of freedom in question.
- cycle-120 architect review-gate Recommendation #2.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION cycle 122.

DECISION: Option B (record contract degree of freedom as design choice). Rationale:
- The existing tests remain robust to contract-permitted halo_inference representation toggles (an explicit-0 entry OR omission).
- The vacuous-pass risk on a silent-skip regression is judged unlikely (halo_inference's walker pattern always emits explicit-0 per the `fn no_halo_bare_iv` in-module test in halo_inference.rs; the walker doesn't drop entries in practice).
- The contract IS the protection against test coupling — narrowing the tests (Option A) would couple them to a single representation choice, breaking on a contract-permitted refactor.
- Option C (strengthen contract + new contract-pin test + then Option A) is a multi-task arc not justified by current evidence.

STEPS:
1. Add an explicit soundness-floor acknowledgement to task0303_05_stencil_distributed_2d_halo_widths_pinned_to_one (matching the existing wording on task0299 and task0303_07).
2. Strengthen task0303_07's existing degree-of-freedom paragraph to also EXPLICITLY name the vacuous-pass arm.
3. Add the project decision as a one-line note at the "TASK-0305 cycle-122 project decision (Option B)" paragraph in halo_inference.rs (search for "absent ≡ explicit-0") (the canonical contract doc).
4. Verify just test passes (no behaviour change).

GATE: nix develop --command bash -c 'just clippy && just test'
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION cycle 122 (2026-05-25). DOC-ONLY.

DECISION RECORDED: Option B (preserve halo_inference's absent ≡ explicit-0 contract degree of freedom).

SHIPPED:
- nucleus/nucleus-compiler/src/passes/halo_inference.rs (Option B contract paragraph; search for "absent ≡ explicit-0") — added explicit project-decision marker paragraph naming Option B, citing the production emit site by symbolic anchor (`per_iv.entry(iv).or_insert(0)` — durable across line moves) and explaining the trade-off (vacuous-pass arm accepted vs contract robustness preserved).
- nucleus/nucleus-compiler/tests/sidecar_halo.rs task0303_05 — added soundness-floor acknowledgement: the >0 pin is contract-form-independent BY CONSTRUCTION; no vacuous-pass arm here (unlike the == 0 pins in task0299 / task0303_07).
- nucleus/nucleus-compiler/tests/sidecar_halo.rs task0303_07 — strengthened the existing degree-of-freedom paragraph to EXPLICITLY name the vacuous-pass arm and cite the cycle-122 decision lineage.

GATE: just clippy (-D warnings) clean; sidecar_halo 12/12 pass. No e2e change required (doc-only).

REVIEW GATE (cycle 122 parallel read-only):
- qa-test-runner: GO (with line-number nit on the doc citation — applied).
- mped-architect: initially NO-GO. Found P1a (wrong line number 1184 → should be 848, the production emit site — cited in 2 places), P1b ("robust to neither form" wording inverted at sidecar_halo.rs:697-699 — should be "robust UNDER EITHER form"), P2 (real coverage gap — no test currently asserts Some(0) for bare-iv emit, all use unwrap_or vacuous-tolerant patterns). All three folded back in-thread:
  * P1a: replaced line citation with symbolic search hint (`search for per_iv.entry(iv).or_insert(0)` — durable across line moves) in both halo_inference.rs:67 and sidecar_halo.rs:783.
  * P1b: wording rewritten to "robust UNDER EITHER contract form".
  * P2: filed as TASK-0307 (structural Some(0) key-exists pin at the in-module no_halo_bare_iv test boundary) — closes the vacuous-pass arm without coupling downstream tests; compatible with Option B.

GOTCHAS + FORWARD-CARRY:
- The architect's catch on P1a is the precise feedback-comment-doc-lie-recurring pattern firing INSIDE a commit whose explicit purpose is doc-lie defence. Forward-carry to TASK-0307 + future doc-citation work: prefer SYMBOLIC search hints (`grep for X`) over absolute line numbers, which rot with edits.
- P1b is the precise feedback-comment-doc-lie pattern at sentence granularity (an inverted negative); the fix was a one-word rewrite ("neither" → "UNDER EITHER"). Two-claim docstrings need each clause verified.
- The Option B decision is sound today because `per_iv.entry(iv).or_insert(0)` inside `classify_index` (halo_inference.rs) is the only emit path — every inspected (kernel, iv) gets explicit-0. If a future refactor moves to true conditional emission (e.g. skipping no-halo entries to compress the sidecar), the cycle-122 narrative pins (task0299, task0303_07) WOULD silently become vacuous; TASK-0307's sentinel is the structural defence.

FILES SHIPPED:
- nucleus/nucleus-compiler/src/passes/halo_inference.rs (+13 lines of contract doc + symbolic search hint)
- nucleus/nucleus-compiler/tests/sidecar_halo.rs (+21 lines of soundness-floor disclosure across two test docstrings)
- backlog/tasks/task-0307 - ... (new, P2 follow-up)
<!-- SECTION:NOTES:END -->
