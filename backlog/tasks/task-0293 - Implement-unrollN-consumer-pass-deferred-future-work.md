---
id: TASK-0293
title: Implement unroll=N consumer pass (deferred / future work)
status: To Do
assignee: []
created_date: '2026-05-24 22:15'
labels:
  - language
  - compiler
  - future-work
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

`unroll=N` is a schedule directive accepted by the Nucleus grammar (`loop V : unroll=N`) that today is parsed, lowered to `ResolvedLoopOption::Unroll(N)`, validated against block=N (TASK-0144 Stage 2: `block=N + unroll=M-not-divisible` is rejected), and then NEVER CONSUMED. No pass reads it; no codegen emits N body-copies.

PRD §6.3.3 currently describes it as "Plain unrolling, no vector grouping". The implementation has been deferred — the project's centre of gravity is partitioning + reuse + pipelining + halo synthesis. LLVM unrolls aggressively in the host build; the DSL-side value of a *deterministic* unroll factor is real (control over the generated source for pedagogy + reproducibility) but not load-bearing for the thesis story today.

## Acceptance criteria (when reopened)

1. New pass `passes/unroll_transform.rs`, sibling of `block_transform.rs`. Reads `ResolvedLoopOption::Unroll(N)` on a `Repeat` node; rewrites the Repeat body into a `Sequence` of N copies with the iv substituted by `iv + 0`, `iv + 1`, ..., `iv + (N-1)`. Outer loop step becomes `N`. Remainder iterations handled (option: scalar tail, or restrict to ranges with `len % N == 0`).
2. Pass ordering in the driver: after `block_transform`, before `partition_*` (or equivalent — the right slot is the one that lets the unrolled body's iv-substitution still feed downstream partitioning).
3. Tests: positive (unrolled body has exact N copies; iv substitution correct), negative (range not divisible — either remainder tail OR explicit reject), determinism (two runs identical).
4. e2e cell that exercises the unroll on a real example (likely 01-elementwise-add/unrolled or similar — bit-identical to a hand-written reference).
5. Schedule grammar already accepts `unroll=N`; no grammar change. Lowering already preserves the variant; no lower change.

## Honest scope

- 1-2 cycles when picked up. The block_transform pass is the closest template; the unroll transform is a structural simplification of it (no tile loop, just N body copies in sequence).
- Don't pick this up unless there is concrete evidence the LLVM-unroll-vs-DSL-prescribed-unroll difference matters for a shipped fixture. Possible triggers: pedagogical examples in the thesis where the generated source must show explicit unrolling; a backend (tier-3 embedded) where LLVM unroll is configured conservatively.

## Why this is filed rather than dropped

User's explicit answer to the orchestrator's drop-vs-defer question (session 2026-05-25): "Keep as future work". `vectorize=M` is being dropped entirely (separate task); `unroll=N` is deferred so the grammar surface stays open for the future implementation.

## Cross-references

- PRD §6.3.3 (loop transformations table — mark `unroll=N` as deferred to this task).
- TASK-0030 (block_transform pass — implementation template).
- TASK-0133 (Petri-net iteration encoding optimisation — different concern, closed as DEFERRED; this task is about the SOURCE-level unroll consumer, not the Petri-net encoding).
- TASK-0144 Stage 2 (block + unroll-not-divisible validation — already in place, will be exercised once a consumer lands).
<!-- SECTION:DESCRIPTION:END -->
