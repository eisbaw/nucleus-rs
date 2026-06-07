---
id: TASK-0453
title: 'Rigour + shortcomings: implementation/thesis ping-pong'
status: To Do
assignee: []
created_date: '2026-06-06 22:51'
updated_date: '2026-06-07 09:50'
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

CYCLE-10 (P10 .10/.20) landed GO — THE LAST RANKED SHORTCOMING. STEP-0 finding: a genuinely-feasible minimal second-family slice DID exist (unlike P6/P9 honest-stops): nRF52840 (Cortex-M4F) shares the thumbv7em target already in .#embedded, Renode bundles nrf52840.repl + a working UARTE EasyDMA model, de-risk smoke proved byte-capture headless. Implemented `--shim nrf52840` single-worker BIN: SHARED NucleusShim trait + generic run<S> + kernel extraction; only concrete shim/memory.x/--nmagic differ. Byte-exact ex1/5/9 in Renode (just renode-embedded-nrf-all). Gate GREEN, e2e 497-428-0-69-0 unchanged (e2e-inert). Thesis ch07/07b/09/10/11: one->two families, residual kept (sprawl in-principle unbounded; cheap only because shared target+bundled model; nRF multi-MCU + timing/diagnostic-path-on-2nd-family = future work TASK-0453.10.01). qa+architect+paper-accuracy+paper-correctness all GO/folded.

EPIC STATUS: all 10 addressable shortcomings (P1..P10) now processed across cycles 1-10. IMPLEMENTED real new rigour: P1 (generative diff-fuzz harness), P2 (transfer-honesty thesis; wire-level precise = open TASK-0453.22), P3 (reproducible fsum reduction), P4 (symbolic looped-net gate), P5 (cap-hit worst-case e2e, single-worker), P8 (mechanical reference-independence CI fence), P9 (tier-2 MPI matrix 3->8 blocking cells), P10 (second MCU family nRF52840 byte-exact). HONEST-STOP (no fabrication): P6 (runtime perf study — coupled to unsolved communicating-gate residual + loopback!=cluster), P7 (precise gather/scatter — whole-array IS the trivial sound envelope, tightening is the open problem). Every thesis side revised to match verified code, residuals kept honest. DEFERRED/OPEN: S7 multi-worker break (TASK-0341.02.01.08), TASK-0453.22 (wire-level precise transfer all 7 backends), TASK-0453.10.01 (nRF check-loop runtime), TASK-0453.21 (fundamental-limitations register — honest-not-planned), TASK-0454 (if any). e2e tier-1 baseline 497/428/0/69/0; MPI tier 12/12; embedded TWO families byte-exact.
<!-- SECTION:NOTES:END -->
