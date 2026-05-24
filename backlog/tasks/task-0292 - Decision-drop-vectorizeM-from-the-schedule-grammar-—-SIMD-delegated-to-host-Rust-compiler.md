---
id: TASK-0292
title: >-
  Decision: drop vectorize=M from the schedule grammar — SIMD delegated to host
  Rust compiler
status: Done
assignee: []
created_date: '2026-05-24 22:14'
updated_date: '2026-05-24 22:35'
labels:
  - language
  - decision
  - compiler
  - grammar
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Decision

The schedule directive `vectorize=M` is **dropped from the Nucleus grammar**. The framing it carried — "unroll inner body M-way; expects SIMD-friendly ops" — is deliberately delegated to the host Rust compiler + LLVM auto-vectorisation.

## Rationale

PRD §6.2.2's kernel-body opacity rule is load-bearing: "Nucleus generates wrapper code that calls the kernel; it does **not** substitute text into kernel bodies." That rule is what makes Nucleus a *scheduler*, not a Rust frontend.

A real `vectorize=M` consumer would require either:
1. A language extension: kernels declare scalar + lane-M variants; vectorize=M selects between them. Real work; the gain over LLVM auto-vectorisation is unclear for the shipped i32-arithmetic stencils.
2. SIMD code generation at the call site — impossible without reading the kernel body (opacity rule).
3. Delegating to LLVM — what already happens by default.

Nucleus's centre of gravity is partitioning + reuse + pipelining + halo synthesis. SIMD is the host compiler's job; Nucleus's bit-identical-across-backends guarantee is preserved by NOT prescribing a SIMD width that backends would have to honour.

## Acceptance criteria

1. `vectorize=M` removed from the schedule parser (`sched/parser.rs`).
2. `LoopOption::Vectorize` removed from `sched/ast.rs`.
3. `ResolvedLoopOption::Vectorize` removed from `sched/ir.rs`.
4. `SchedLowerErrorKind::VectorizeNotDivisibleByBlock` (TASK-0144.01) removed — dead with no producer.
5. Lowering arm removed from `sched/lower.rs`.
6. Tests pinning the parsing / lowering / validation of vectorize removed (sched_parser.rs, sched_lower.rs, transfer_inject_hoist.rs, block_transform.rs as needed).
7. Shipped schedules using `vectorize=8` updated to drop the directive (05-stencil/distributed.sched.nuc + 05-stencil/reuse.sched.nuc).
8. PRD §6.3.3 table updated (vectorize row dropped); §6.3.3 bad-combinations sentence updated; the historical 2013-thesis-paragraph reference preserved (history).
9. docs/grammar-sched.md updated.
10. `just ci` green; e2e baseline unchanged.

## Honest scope

This is bookkeeping; no behavioural change. The shipped 05-stencil/distributed + 05-stencil/reuse cells PASS today with `vectorize=8` precisely because the directive is inert. Dropping it is a no-op on behaviour and a clarifying step on the language surface.

## Cross-references

- PRD §6.2.2 (kernel-body opacity rule — load-bearing for this decision).
- PRD §6.3.3 (loop transformations table — the surface being trimmed).
- TASK-0144.01 (the validation rule being removed as dead).
- TASK-0050 + TASK-0051 (M9 worker_class.simd / place_data — different surface, not affected).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Landed orchestrator-direct (cycle 2026-05-25). Grammar/AST/IR/lower/tests/shipped-schedules/PRD/docs all updated. just ci green (92/79/0/13/0 e2e baseline unchanged; all 4 negative arms bite).
<!-- SECTION:NOTES:END -->
