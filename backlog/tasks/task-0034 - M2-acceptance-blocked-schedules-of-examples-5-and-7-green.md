---
id: TASK-0034
title: 'M2 acceptance: blocked schedules of examples 5 and 7 green'
status: Done
assignee: []
created_date: '2026-05-17 23:06'
updated_date: '2026-05-22 13:32'
labels:
  - M2
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Milestone gate. Examples 5 and 7 under both naive and blocked schedules, on pthreads-sync, all bit-identical. Determinism CI green.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 'just e2e --milestone M2' exits 0.
- [x] #2 Matrix is examples {1,2,3,5,7} × schedules {naive, blocked-where-applicable} × backends {pthreads-sync}. All cells green.
- [x] #3 --emit-pn produces a DOT file for at least one example that renders meaningfully.
- [x] #4 Boundedness and deadlock checks pass for all M2 examples.
- [x] #5 Implementation notes record any features that almost made M2 but were cut (e.g. reuse, double-buffering, pipeline).
- [x] #6 Implementation notes record honest limitations (still one backend; cross-backend differential test arrives at M3).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 47 (2026-05-22) — closed. M2 milestone acceptance is comprehensively GREEN; this is a tracker closure of a long-since-substantively-satisfied gate.

AC#1: 'just e2e-milestone M2' → total: 8 pass: 8 fail: 0 skipped: 0, exit 0. Verified live cycle 47.

AC#2: The M2 milestone band per nuc-nucleus/e2e-matrix.toml lines 39-42 covers pthreads-sync blocked cells + remaining naive cells for examples 5 + 7. The just-e2e-milestone M2 run exercises 8 cells: 01/02/03 naive (M1 inherited cumulatively), 05/naive + 05/blocked, 07/naive + 07/blocked, plus 02/split. All 8 PASS bit-identical to reference.

AC#3: --emit-pn produces a meaningful DOT file. Verified cycle 47 on 05-stencil/blocked × pthreads-sync: 805 lines of Petri net DOT (places + transitions + arcs) with the documented header naming the algo + sched + backend triple.

AC#4: Boundedness + deadlock checks pass for all M2 examples. Structurally satisfied by 'just test' running the compiler test suite continuously across all 21+ cycles in 2026-05-22, including the acfg_to_petri::e2e_example_* tests (cycle 40 / TASK-0218 explicitly exercised this with the path-1-only post-elision check).

AC#5 (features cut from M2): the M2 spec was 'static scheduling + Petri net + blocked.sched.nuc for examples 5/7 + determinism in CI'. ALL of those landed:
- static scheduling: TASK-0020/0021/0022 (M1, inherited).
- Petri net: build_acfg + acfg_to_petri + boundedness + deadlock detection (M2 work).
- blocked.sched.nuc for examples 5, 7: shipped + passing as required cells.
- Determinism in CI: 'just determinism-check' + 'just determinism-check-negative' (TASK-0145/0157/0188) both green; the negative-falsifier 'just determinism-check-negative' is required for the just ci recipe.
- Cut: nothing notable. Pipelining (pipeline=D) + reuse + multi-buffering were always M4 work and arrived there per plan (cycles 26-41).

AC#6 (honest limitations recorded):
- 'still one backend' for M2 — cycle 47: NO LONGER TRUE. M3 brought mp-tcp-bufsync (TASK-0036); M4 brought pthreads-async + mp-tcp-event partial (cycles 16-41). The matrix is now 4 backends × ~8 examples × ~2-4 schedules = 88 cells.
- 'cross-backend differential test arrives at M3' — confirmed: M3 / TASK-0041 / TASK-0178 wired the cross-backend bit-identical differential, currently bites against xbackend negative falsifier (CORRUPTED_DETECTED=1 verified cycle 47).
- M2 was a one-backend gate; the cross-backend invariants that came with M3 + M4 cycles 26-41 have made M2's single-backend assumptions ARCHITECTURALLY OBSOLETE in the friendly sense (the M2 cells stay green, but the milestone's 'this is the limit of what we can prove' framing is superseded by the 4-backend matrix).

This closes the M2 milestone acceptance gate. The relevant work has been in for cycles; this tracker closure cleans up the stale 'In Progress' parent.
<!-- SECTION:FINAL_SUMMARY:END -->
