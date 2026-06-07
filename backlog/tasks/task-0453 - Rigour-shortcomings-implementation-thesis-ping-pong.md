---
id: TASK-0453
title: 'Rigour + shortcomings: implementation/thesis ping-pong'
status: To Do
assignee: []
created_date: '2026-06-06 22:51'
updated_date: '2026-06-07 05:44'
labels:
  - rigour
  - thesis
  - epic
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
EPIC. Drive more rigour by turning the thesis's honestly-documented shortcomings (paper/chapters/10-discussion.tex limitations + threats-to-validity, paper/chapters/11-future-work.tex, and defence-prep weaknesses W1-W5 in TASK-0452.08) into PLANNED, dependency-linked IMPLEMENT+THESIS-UPDATE task pairs, then ping-pong: implement a fix under full phase3 discipline, then revise the thesis section documenting that shortcoming to reflect the new capability HONESTLY (residual kept as documented limitation). FUNDAMENTAL trade-offs (expressiveness-vs-deterministic-firing etc.) stay honest limitations and are NOT planned away (see the Fundamental-limitations register child). Rule: strengthen ACTUAL rigour, never relabel; every claim codebase-verified; no regressions (just ci GREEN, e2e baseline held, thesis PDF green).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each ADDRESSABLE shortcoming has a dependency-linked IMPLEMENT->THESIS-UPDATE pair filed
- [ ] #2 FUNDAMENTAL limitations registered as honest-not-planned
- [ ] #3 Ping-pong cycles land both sides (code + thesis) per cycle with gates green
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-5 (P5 .05/.15) landed GO. STEP-0 finding: bounded static-firing-order-preserving data-dependent iteration was ALREADY shipped single-worker by the 0341.02.01.x epic (example 21-jacobi-converge); P5 was not unimplemented, only its multi-worker arm (= open S7 0341.02.01.08). Realizable rigour this cycle (invariant-safe, single-worker): new example 29-jacobi-cap-hit exercises the previously-untested cap-hit/did-NOT-converge WORST-CASE replay path end-to-end byte-identical (closes S5 P3-2 e2e gap), substantiating the thesis "worst-case replay bounded by the cap" claim. Thesis (.15 DONE): ch05/ch10/ch11/appendix reframed bounded iteration as REALIZED single-worker (not future); multi-worker + float-predicate + unbounded kept as honest residuals. e2e 490/427/0/63/0 -> 497/428/0/69/0; just ci GREEN; PDF 123pp; review GO x2 each side. .05 stays In Progress (AC#1 multi-worker = S7). Honest-stop on multi-worker per the deepest-invariant guardrail (feasible but a large deferred slice, NOT an invariant breach).

CYCLE-6 (P6 .06/.16): HONEST-STOP. A rigorous runtime perf/scaling study is not defensibly feasible here -- measured: corpus-scale runtime is overhead-dominated (matmul N=16=0.86ms, compute=us), a signal needs large N (naive N=512=103ms), but multi-worker/communicating schedules can NOT compile at large N (distributed matmul gate RSS ~25GB OOM at N=512; no gate-skip flag), and loopback!=cluster on a single hybrid-core box. The runtime study is COUPLED to the unsolved communicating-case gate residual. Per guardrail, shipped NO shaky benchmark; kept the thesis "runtime out of scope" stance and SHARPENED ch11 sec:fw-quant with this grounded structural reason (commit ee7872c). .06 Done-as-DEFERRED; .16 Done (sharpening not results). paper-correctness+paper-accuracy reviewed (accuracy caught + fixed an over-precision). PDF 124pp green; no code change so e2e 497/428/0/69/0 unaffected.
<!-- SECTION:NOTES:END -->
