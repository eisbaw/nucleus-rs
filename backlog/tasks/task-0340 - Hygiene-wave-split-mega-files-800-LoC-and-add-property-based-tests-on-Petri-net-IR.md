---
id: TASK-0340
title: >-
  Hygiene wave: split mega-files (>800 LoC) and add property-based tests on
  Petri-net IR
status: To Do
assignee: []
created_date: '2026-05-26 09:46'
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
