---
id: TASK-0334
title: >-
  TASK-0331 sibling sweep extension: empirical verification of the 4 cells
  cycle-150 forward-linked to TASK-0329 without runtime verification
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 21:57'
updated_date: '2026-05-25 22:10'
labels:
  - e2e-matrix
  - documentation
  - opacity-rot
  - forward-carried-from-TASK-0331
  - empirical-verification
dependencies:
  - TASK-0331
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0331 cycle 150 audited e2e-matrix.toml's TASK-0175 citations. Five in-scope citations were classified. **ONLY ONE** (05/distributed-2d × mp-tcp-event) was empirically promoted to [[required]] and run; that promotion surfaced TASK-0332 as a new architectural limitation.

The other FOUR cells were forward-linked from TASK-0175 to TASK-0329 in prose without runtime verification:

1. 03-reduction/distributed × mp-tcp-bufsync (lines ~398-440)
2. 13-cnn-inference/batch_parallel × mp-tcp-bufsync (lines ~471-479)
3. 03-reduction/distributed × mp-tcp-event (line ~989)
4. 13-cnn-inference/pipeline_parallel × mp-tcp-event (lines ~1126-1146)

Cycle-150's own closure called this out explicitly as a forward-carried lesson: 'NEVER write the prose without first inspecting the emitted code' (feedback-orchestrator-narrative-also-wrong, third firing).

## Cycle-156c (orchestrator) empirical finding

Codegen for 03-reduction/distributed × mp-tcp-bufsync **succeeds** (no ContractGap fires; barriers already include host). Running the built binary triggers a runtime seq-tag mismatch:

```
wire: seq tag mismatch: receiver expected 4, wire delivered 8 — Push/Wait pairing diverged
```

Inspecting the emitted code:
- HOST waits per-consumer: seq=4,5,6,7 (one per worker for half1 path), then seq=8,9,10,11 (same data again for half2 path).
- W0 sends partials TWICE back-to-back: seq=8 first, then seq=4 — wrong order against host's FIFO wait sequence.

So the **actual blocker is a transfer_inject defect** (duplicate Push emission for multi-use shared data + producer-side ordering inversion against consumer Wait sequence), not the host-excluding-barrier rejection TASK-0329 names.

## Acceptance criteria

### AC#1: empirically verify each of the 4 cells

For each cell: regenerate the project, attempt to build + run, classify the actual failure (codegen reject / build error / runtime).

### AC#2: file precise defect tasks where the actual blocker differs from TASK-0329

Each cell whose actual blocker is NOT a host-excluding barrier gets a new tracker task with the empirically observed mechanism.

### AC#3: update e2e-matrix.toml skip reasons to cite the empirically-verified blocker

Replace the TASK-0329 forward-link with the actual defect task in the skip reason text. Preserve the architectural lineage prose where useful, but lead with the empirical mechanism.

## Honest scope

- LOW priority. Doc/audit-hardening; no code emit changes; no e2e baseline shift.
- Follows the cycle-150 forward-carried hygiene rule directly.
- Bounded by 4 cells; each takes ~5 minutes of codegen+run inspection.

## Cross-reference

- TASK-0331 (cycle 150) — the audit task whose closure flagged this gap as a forward-carried lesson but did not extend the empirical-verification step to the 4 unverified cells.
- TASK-0329 — the destination citation cycle-150 wrote into the 4 skip reasons; this task empirically verifies whether that citation is correct.
- feedback-orchestrator-narrative-also-wrong (memory) — third-firing prevention.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-157 implementation plan (orchestrator-direct)

This is a doc/audit cycle — no compiler edits. The codegen for each cell either succeeds or fails fast; the audit captures which.

### AC#1 — empirical verification (4 cells)

For each cell run `nucleus build --backend <B> --algo ... --sched ... --kernels ... --out /tmp/task0334-<n>`. If codegen succeeds: build the project, copy input.bin, run `bash run.sh`. Record the actual failure mechanism.

Already done in pre-cycle exploration:
- 03-reduction/distributed × mp-tcp-bufsync — codegen OK; runtime seq-tag mismatch (4 vs 8). Producer worker emits two Push of partials back-to-back with mismatched seq order vs host's per-consumer Wait sequence.

Remaining:
- 13-cnn-inference/batch_parallel × mp-tcp-bufsync
- 03-reduction/distributed × mp-tcp-event
- 13-cnn-inference/pipeline_parallel × mp-tcp-event

### AC#2 — file precise defect tasks

For 03-reduction/distributed × {bufsync, mp-tcp-event}: shape is host-as-multi-consumer of partials. File a single task TASK-0335 (or similar) covering both backends with the same root cause (transfer_inject emits per-consume Push/Wait without dedupe of same-data multi-use on the producer side).

For 13-cnn-inference cells: classify per cell after run.

### AC#3 — update e2e-matrix.toml skip reasons

Replace TASK-0329 forward-link with the empirically-verified defect task (TASK-0335 or similar). Preserve architectural-lineage prose only where the host-mediated-star observation is still architecturally relevant (it isn't, for the verified-different-blocker case).

## Cycle 157 implementation outcome (orchestrator-direct)

### AC#1 — empirical verification of all 4 cells

| Cell | Codegen | Build | Run | Output | Actual blocker |
|---|---|---|---|---|---|
| 03-reduction/distributed × mp-tcp-bufsync | OK | OK | seq-tag mismatch panic | n/a | **TASK-0335** (transfer_inject duplicate Push + ordering inversion) |
| 13-cnn-inference/batch_parallel × mp-tcp-bufsync | OK | OK | OK | **bit-identical x3** | NONE — wrongly skipped, **PROMOTED to [[required]]** |
| 03-reduction/distributed × mp-tcp-event | OK | OK | OK | **bit-identical x3** | NONE for correctness (defect masked by per-(DataId,SeqTag) channel demux), **PROMOTED to [[required]]** with masking disclosure |
| 13-cnn-inference/pipeline_parallel × mp-tcp-event | ContractGap | n/a | n/a | n/a | **TASK-0329** (empirically verified — barriers {w1,w2,w3} genuinely exclude host) |

Score: 3 of 4 cycle-150 forward-links were WRONG (orchestrator-narrative misattribution); 1 of 4 was correct.

### AC#2 — defect tasks filed

- **TASK-0335** (Medium, M6): transfer_inject duplicate-Push for multi-consume shared data. Two compounding sub-defects: per-host-consume-site Push emission (semantically redundant) + producer-side ordering inversion (4-then-8 vs FIFO 8-then-4). bufsync fails LOUD at runtime; mp-tcp-event masks via per-channel demux.

### AC#3 — e2e-matrix.toml skip-reason update

Four sites updated:
- 03-reduction/distributed × mp-tcp-bufsync: cite TASK-0335 (empirically verified)
- 13-cnn-inference/batch_parallel × mp-tcp-bufsync: promoted [[skip]] → [[required]]
- 03-reduction/distributed × mp-tcp-event: promoted [[skip]] → [[required]] with mp-tcp-event-masking disclosure
- 13-cnn-inference/pipeline_parallel × mp-tcp-event: TASK-0329 confirmed empirically (verbatim ContractGap pinned in prose)

Plus narrative banner at line ~580 (03-reduction/distributed × pthreads-async commentary) updated: three-way differential now covers {pthreads-sync, pthreads-async, mp-tcp-event}; bufsync remains skipped on TASK-0335.

### Verification gate (cycle 157)

- e2e baseline: 112/96/0/16/0 → **112/98/0/14/0** (+2 promotions, no regressions). Verified non-flake × 2 (4-way differential on both promoted cells, ~600ms-4s per cell).
- just test: 80 suites passed, 0 failed.
- just test-release: 80 suites passed, 0 failed.
- just clippy: zero warnings.

### Honesty notes

- The cycle-150 closure ITSELF flagged the orchestrator-narrative-also-wrong pattern as a 3rd-firing forward-carried lesson. The exact same pattern fired THREE MORE TIMES in the same task closure's prose (wrong TASK-0329 forward-link for cells 1, 2, 3 of 4). This audit is the 4th-firing empirical verification step the cycle-150 lesson explicitly prescribed. Memory note feedback-orchestrator-narrative-also-wrong incremented to 4th firing.
- mp-tcp-event's promotion (03/distributed) is correctness-sound but DOES rest on a wire-shape-masking property. If TASK-0335 lands the Push dedupe, the cell will remain bit-identical AND use half the bandwidth — strictly improvement. Disclosed in skip prose.
- The empirical verification cost was ~15 minutes total (codegen + build + run × 4 cells). The cumulative cost of 4 wrong narrative attributions over cycles 149/150 was substantially higher.

### Forward-carried lessons

- For audit-class tasks that touch skip-reason prose, the AC must explicitly require empirical verification (codegen + run + cmp against reference.bin) for each cell whose attribution is being changed. cycle-150's AC#2 only required this for ONE candidate; the other three got prose-only updates.
- The 'skip reason' format should distinguish 'narratively-attributed' from 'empirically-verified' blockers. A simple convention: prose that opens with 'EMPIRICALLY VERIFIED cycle N (TASK-XXXX):' is verified; absence of that prefix means narrative-only.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Empirical verification cycle. 3 of 4 cycle-150 forward-links to TASK-0329 were wrong (orchestrator-narrative-fourth-firing); 1 was correct. Two cells promoted [[skip]] → [[required]] (13-cnn/batch_parallel × mp-tcp-bufsync; 03/distributed × mp-tcp-event with masking disclosure). One new defect task filed (TASK-0335, transfer_inject duplicate-Push). One skip reason confirmed (13-cnn/pipeline_parallel × mp-tcp-event genuinely hits TASK-0329 barrier rejection). E2E baseline: 112/96/0/16/0 → 112/98/0/14/0 (non-flake × 2).
<!-- SECTION:FINAL_SUMMARY:END -->
