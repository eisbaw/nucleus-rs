---
id: TASK-0340
title: >-
  Hygiene wave: split mega-files (>800 LoC) and add property-based tests on
  Petri-net IR
status: To Do
assignee: []
created_date: '2026-05-26 09:46'
updated_date: '2026-05-26 10:49'
labels:
  - tech-debt
  - hygiene
  - testing
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Hygiene wave flagged by 2026-05-25 audit (post cycle-171 M6-decomposition planning). Two deferred-not-cancelled debts that should land before M6 codegen amplifies them:

(A) Mega-files. Six src files breach the 800-LoC smell threshold; three breach 1500. Top offenders (LoC / comment ratio):
- nucleus/backend-common/src/render.rs               1687 / 0.46
- nucleus/backends/mp-tcp-bufsync/src/lib.rs          1515 / 0.34
- nucleus/nucleus-compiler/src/acfg.rs                1440 / 0.57
- nucleus/nucleus-compiler/src/link.rs                1290 / 0.47
- nucleus/backend-common/src/multi_worker_walker.rs   1169 / 0.48
- nucleus/backends/mp-tcp-event/src/multi_worker.rs   1140 / 0.21

acfg.rs at 57 percent comments is a comment-doc-lie magnet (per feedback-comment-doc-lie-recurring). render.rs + multi_worker_walker.rs are the shared spine of all 4 backends; a bug in either touches every tier-1 cell. M6 will add 3 backend crates + 3 examples on top of this substrate; splitting first prevents the smell from propagating.

(B) Zero property tests / zero fuzz across the entire workspace. The Petri-net IR (PRD section 8) whose soundness is the central thesis claim is tested by 49 hand-curated cases across acfg_to_petri / petri_to_events / boundedness / deadlock. A 50-line proptest on bounded-ACFG generators is the highest expected-ROI gap in the test suite.

Sub-concern: nucleus/e2e/src/main.rs is 7316 LoC with 76 internal tests covering the JSON/JUnit report formatter, not compiler correctness. Visually inseparable from compiler tests today.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Audit produces the canonical list of src .rs files greater than 800 LoC under nucleus/{backend-common,nucleus-compiler,backends}/src/; the six current offenders above are explicitly covered, plus any new addition
- [ ] #2 Each listed file split into cohesive sub-modules along seams already named by its module-level docstring (no behaviour change). Per-file split is one atomic commit; final commit asserts e2e baseline preserved bit-identical (currently 98 required + 10 skip in e2e-matrix.toml; just e2e totals line preserved)
- [ ] #3 proptest dep added to nucleus-compiler dev-dependencies; at least 3 properties per pass for passes/boundedness.rs, passes/deadlock.rs, passes/petri_to_events.rs. Generators emit small bounded ACFGs; properties assert (i) boundedness pass agrees with bounded-reachability up to N steps, (ii) deadlock pass agrees with explicit enumeration on the same generated nets, (iii) petri_to_events output is acyclic per worker
- [ ] #4 Report-formatter tests in nucleus/e2e/src/main.rs (currently 76 internal #[test]) carved out into a sub-module file (e2e/src/report/tests.rs) or sub-crate (e2e_report). Compiler-correctness tests remain in main.rs; formatter tests are visually separated
- [ ] #5 New just recipe check-mega-files added to ci: asserts no nucleus/**/src/*.rs file exceeds 1000 LoC. Recipe is wired into just ci as a regression-fence. Initial pass exempts any file the split intentionally leaves above 1000 LoC via an explicit allow-list (with rationale)
- [ ] #6 No new TASK-NNNN or cycle-NNN citations introduced in the refactored files (closes the comment-process-noise concentration smell: acfg.rs 74 mentions, mp-tcp-bufsync/lib.rs 68, sidecar.rs 57 at audit time)
- [ ] #7 Final cycle commit notes per-file LoC before/after and per-pass proptest count delta (no separate summary md file per CLAUDE.md cruft policy)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 176 — slice 1 landed (AC#5 of TASK-0340)

`just check-mega-files` regression-fence recipe + ci wiring landed (commit pending).

DELIVERED:
- AC#5 (recipe + ci wiring + initial allow-list with rationale): DONE.

OPEN (subsequent slices):
- AC#1 (canonical audit list): DONE incidentally via the allow-list (14 files >1000 LoC documented).
- AC#2 (per-file split into sub-modules): NOT-YET — substantive, slice-2+ work.
- AC#3 (proptest dep + properties for boundedness / deadlock / petri_to_events): NOT-YET — slice-3 work.
- AC#4 (e2e/main.rs report-formatter carve-out): NOT-YET — slice-4 work; recipe scope explicitly excludes nucleus/e2e/src per architect cycle-176 P2.3.
- AC#6 (no new TASK/cycle citations in refactored files): NOT-YET — lands with AC#2 splits.
- AC#7 (final commit notes per-file LoC before/after + proptest count delta): NOT-YET — final slice when AC#2 + AC#3 close.

ARCHITECT-DEFERRED FOLD-BACKS (forward-carried to slice-2):
- P2.1: STALENESS direction not enforced — a future split could leave a stale allow-list entry for a file no longer >1000. Architect empirically verified: replaced pthreads-async/multi_worker.rs (allow-listed, 1048 LoC) with 500-LoC stub, recipe PASSED. Slice-2 should add a sibling assertion that every allow-list pattern matches a still-oversized file. Concretely: refactor the recipe to enumerate allow-list paths positively (rather than as grep -v negative filters) so the "this allow-list entry is stale" direction also fails loudly.

## Cycle 177 — slice 2 staleness-check refactor landed

Architect cycle-176 P2.1 fold-back complete: check-mega-files now enumerates the allow-list POSITIVELY (printf-fed bash array via temp files + `comm -23`). Both directions FAIL LOUD:
- (A) new mega-file >1000 LoC outside allow-list.
- (B) allow-list entry whose file is NO LONGER >1000 LoC (split landed, file deleted, file shrank).

The cycle-176 architect-reproduced silent-pass case (replace pthreads-async/multi_worker.rs allow-listed 1048 LoC → 500-LoC stub) now FAILS LOUD with the precise direction-B message.

Cycle-177 architect (read-only) GO with two P1 fold-backs applied this cycle:
- P1.1 POSIX-shell portability — `comm -23 <(echo ...)` used bash process substitution; just defaults to `/bin/sh` which on dash/ash/busybox would syntax-error before either direction runs (silent-absence rather than silent-pass). Rewrote to temp-file form via `mktemp` + `trap EXIT`.
- P1.2 memory-citation correction — initial draft cited `feedback-silent-sibling-defect` but the actual class is `feedback-opacity-gate-rot` (each per-file filter is a deferral gate that rots silently when surrounding state shifts). Swapped citation.
- P2.1 (folded inline) — added `set -eu` + `set -o pipefail` so find-pipeline-internal errors propagate; dropped `2>/dev/null` so scope-vanish failures surface.

Deferred (P2.2 cosmetic — direction-A LoC count in failure message; P2.3 informational — printf-form pin comment; P3.2 cosmetic — one memory per line). Acknowledged P3.1 — AC#5 of TASK-0340 reads "asserts no file exceeds 1000 LoC + initial pass exempts via allow-list with rationale"; the staleness direction is a strict SUPERSET of the AC text. Recording the cycle-177 implementation as "AC#5 implemented with both directions; staleness direction exceeds AC text scope" rather than rewriting the AC (per memory `feedback-ac-rewrite-on-done-task`).

BITE-verified both directions on the POSIX-rewrite form:
- 1006-LoC stub in backend-common/src/ → FAIL (direction A).
- pthreads-async/multi_worker.rs truncated to 500 LoC → FAIL (direction B).

TASK-0340 AC status post-cycle-177:
- AC#5: DONE (both directions now), strict superset of original AC text.
- AC#1: DONE incidentally (cycle 176).
- AC#2-#4, #6-#7: PENDING (subsequent slices).
<!-- SECTION:NOTES:END -->
