---
id: TASK-0467
title: >-
  Thesis-sync: for..until is now 7-backend single-worker (was 'one backend
  only') — distributed still blocked
status: Done
assignee: []
created_date: '2026-06-10 22:43'
updated_date: '2026-06-11 04:30'
labels:
  - thesis-sync
  - documentation
  - for-until
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0341.02.01.08 (epic S7, cycle-S7). That task flipped the 12 sibling-backend [[skip]] cells on 21-jacobi-converge/naive + 29-jacobi-cap-hit/naive to [[required]]: the for..until break + convergence reduction + runtime final-read + cap-hit observability now run BYTE-IDENTICAL on ALL 7 tier-1 backends (the single-worker naive schedule delegates to the shared render_single_worker_main on every backend; the multi-worker fail-loud guard is never reached for a host-only schedule). e2e moved 504/431/0/73/0 -> 504/443/0/61/0.

The paper still says the for..until family is checked on ONE backend only. These claims are now STALE (paper/** is OUT OF SCOPE for the implementer; fix here):

1. paper/chapters/08-validation.tex:126 — "the for..until loop is checked on the single pthreads-sync backend only, mirroring the curated matrix own multi-worker skip for that construct, not seven-way." NOW: it IS seven-way on the single-worker path; the curated matrix has NO for..until skips left (12 [[skip]] -> [[required]]). The remaining honesty is the DISTRIBUTED (multi-worker partition) path, not the backend count.

2. paper/chapters/10-discussion.tex:62 — "the two convergence examples off their single-worker path recorded as skips" — there are NO converge skips now; reword to the distributed-path caveat.

3. paper/chapters/10-discussion.tex:326 — "the for..until family is checked on one backend only, mirroring the curated multi-worker skip rather than the full seven-way matrix" — NOW seven-way single-worker; distinguish single-worker (done, 7-way) from distributed multi-worker (still blocked).

4. paper/chapters/10-discussion.tex:339-341 — "lifting the for..until family from its single backend to the full multi-worker matrix once that construct cross-backend path lands" — the SINGLE-BACKEND->seven-backend lift has landed (single-worker); reword to: the remaining lift is single-worker -> DISTRIBUTED multi-worker (collective all-reduce+broadcast), which is blocked on 16-jacobi/distributed (honest-BLOCKED on all 7: 5 wait_slice + 2 TASK-0330) PLUS the new collective-break machinery.

5. paper/chapters/11-future-work.tex sec:fw-affine (around line 130) — the collective all-reduce-and-broadcast description is STILL ACCURATE as future work for the DISTRIBUTED path (do NOT remove); but it can now note the single-worker 7-backend differential already landed, so the remaining gap is purely the partitioned (multi-worker) collective break.

6. paper/chapters/05-*.tex sec:lang-limits — if it says the for..until is single-backend, update to single-WORKER (7-backend) with distributed as the open item.

KEY DISTINCTION the rewordings must preserve: SINGLE-WORKER for..until = now 7-backend byte-identical (landed). DISTRIBUTED (partition=workers) for..until with a COLLECTIVE break = still honest-BLOCKED (inherits 16-jacobi/distributed blockage + needs the collective all-reduce+broadcast). The paper currently conflates "single backend" with "single-worker"; the change separates them.

docs/** may carry the same conflation — grep docs/ for "for..until" + "single" and "pthreads-sync only".
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
STAGE 1 (recon): grep paper/ for the 6 stale sites listed in TASK-0467 + the old e2e triple 504/431/73; verify the generative for_until scope against nucleus/e2e/src/bin/diff_fuzz/family/until.rs (curated matrix vs generative harness differ). STAGE 2 (surgical edits, paper/** only): ch05 sec:lang-limits single-backend->single-WORKER 7-backend, distributed open; ch08:126 for..until checked 7-way single-worker (curated matrix 12 skip->required), generative family note stays single-backend per until.rs; ch10:62/326/339 reword converge skips->distributed-path caveat + single-worker(7-way done) vs distributed(blocked); ch11 sec:fw-affine keep design para, update status sentence (single-worker landed, distributed gap remains). Update baseline numbers 504/431/73->504/443/61 if threaded via preamble macros. STAGE 3 (verify): nix develop --command just build FROM paper/; latexmk exit 0; zero undefined refs; grep changed claims for cross-section consistency; record page delta; every number traced to matrix/tracker.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DONE (paper/** only; no git ops). All six listed stale sites + four additional cross-section-consistent sites updated. EVERY number traced to nuc-nucleus/e2e-matrix.toml + TASK-0341.02.01.08 (504/443/0/61/0).

SITES EDITED:
1. paper/preamble.tex:101-103 — \nmatrixpass 431->443, \nmatrixskip 73->61 (\nmatrixtotal 504 unchanged); added a dated provenance comment block (+12 pass / -12 skip, the 12 flipped sibling cells).
2. paper/chapters/05-nuc-language.tex sec:lang-limits — fixed garbled heading ("multi-worker only on the single-worker path"->"realized only on the single-worker path"); added single-worker for..until now byte-identical across all 7 tier-1 backends ("seven-backend, not single-backend"); multi-worker break = fail-loud-rejected (NOT "recorded as a skip" — no such cell exists).
3. paper/chapters/08-validation.tex:126 — the GENERATIVE diff-fuzz family note STAYS single-backend (pthreads-sync only), VERIFIED against the dispatch path nucleus/e2e/src/bin/diff_fuzz/program.rs:139-141 Family::ForUntil1d(_)=>&UNTIL_BACKENDS (len 1) + the test assertion at :194-195 — NOT the narrative. Dropped the now-false justification "mirroring the curated matrix multi-worker skip"; stated both scopes precisely (generator single-backend = harness limit; construct itself now 7-backend via the curated matrix).
4. paper/chapters/09-results.tex skip breakdown :40-54 — 38->26 remaining-after-embedded; language-shape 14->2; removed the "twelve from two for..until ... skipped on the six non-pthreads-sync" stale clause; added explicit "twelve formerly-skipped sibling cells now pass". 35 embedded + 24 capability + 2 w2w = 61. Also :202-212 sec:res-limits reworded (loop-break no longer a skip on single-worker; only distributed remains).
5. paper/chapters/10-discussion.tex — :62 (converge "off single-worker path recorded as skips" -> distributed collective break is future work, NOT a skip), :127-135 ("recorded as skips on multi-worker AND CROSS-BACKEND paths" was doubly stale: it IS cross-backend now), :331 + :346 (generator one-backend = generator-side limit, distributed lift = backend-capability gap; distinguished single-worker-landed from distributed-blocked).
6. paper/chapters/11-future-work.tex sec:fw-affine — KEPT the collective all-reduce-and-broadcast DESIGN paragraph (still accurate future work); updated the STATUS sentence (single-worker now 7-backend; distributed named future work, fail-loud-rejected, NOT "recorded as skips").
7. paper/appendices/A-examples.tex:185-208 — both Jacobi rows "a skip elsewhere" -> 7-backend single-worker, distributed future work.
8. paper/figures/coverage-heatmap.tex:27 — example-21 row {s,P,s,s,s,s,s}->{P,P,P,P,P,P,P} (the for..until row was the ONLY all-but-one-skipped row; now all 7 pass).

KEY DISTINCTION PRESERVED: single-WORKER for..until = now 7-backend byte-identical (curated matrix, landed); GENERATOR for_until family = still single-backend pthreads-sync (dispatch-path-verified); DISTRIBUTED (partition=workers, collective break) = honest-BLOCKED future work (TASK-0341.02.01.08.01), NOT a recorded skip (no such cell exists in e2e-matrix.toml).

VERIFICATION: latexmk exit 0 (nix develop --command just build FROM paper/); zero undefined refs/cites (the lone TU/lmr/bx/sc font-shape warning is cosmetic + pre-existing); zero LaTeX errors; no rerun-needed. Page delta 129->129 (zero; surgical prose). Referenced labels sec:res-quant / sec:fw-affine / sec:res-limits each resolve to exactly one definition in main.aux.

GOTCHA (out of my ownership, filed as observation): nucleus/e2e/src/bin/diff_fuzz/program.rs:58-60 UNTIL_BACKENDS docstring still says "the other six are [[skip]]\x27d in e2e-matrix.toml" — STALE since TASK-0341.02.01.08 flipped them to [[required]]. The dispatch (len-1 backend set) is still correct; only the comment lies. Sibling code-comment-doc-lie for the e2e-agent to fix. Also: coverage-heatmap.tex omits examples 28 (bin-fsum) + 29 (cap-hit) entirely (pre-existing, predates this task) and the s (all-skipped) legend key is now orphaned (no row uses s after the 21-row flip).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Thesis synced to the S7 ground truth: for..until stated as seven-backend single-worker (curated matrix, required cells) with the distributed collective break kept as named future work (design paragraph preserved); the generative-family narrower scope stated as a distinct fact; baseline macros 443/61; all six listed sites + four additional stale siblings the greps found (ch09 skip-breakdown, sec:res-limits, appendix A, coverage-heatmap row). PDF 129pp, zero undefined refs, zero page delta. Follow-ups: TASK-0468 (heatmap missing rows), revision triggers on TASK-0341.02.01.08.01. Landed 0c0e808; architect GO (all four spot-checks verified against matrix/code).
<!-- SECTION:FINAL_SUMMARY:END -->
