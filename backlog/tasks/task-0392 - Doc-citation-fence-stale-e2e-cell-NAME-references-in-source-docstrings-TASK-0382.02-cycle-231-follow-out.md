---
id: TASK-0392
title: >-
  Doc-citation fence: stale e2e-cell-NAME references in source docstrings
  (TASK-0382.02 cycle-231 follow-out)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-01 00:38'
updated_date: '2026-06-01 01:49'
labels:
  - tooling
  - ci
  - doc-lie
  - cycle-221-followup
dependencies:
  - TASK-0382.02
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-231 follow-out of TASK-0382.02. The check-doc-test-name-staleness fence (cycle-231) validates back-ticked task<NNNN> UNIT-TEST name citations against defined fns. A SEPARATE high-confidence shape remains unvalidated: back-ticked e2e-CELL-NAME citations in source docstrings (e.g. the ec50108 lie 'gather_2out_loop' renamed to 18-multigather/distributed). These must validate against the cell universe in nuc-nucleus/e2e-matrix.toml, NOT against fn defs. HARD part / zero-FP: e2e cell identifiers have two shapes -- the NN-name/variant example path AND bare snake_case aliases (gather_2out_loop) -- and the latter is hard to disambiguate from an ordinary symbol mention. Design: restrict to back-ticked tokens that ALSO appear (or used to) as cell keys/paths in e2e-matrix.toml, SKIP on ambiguity. LOW; purely additive; only build if the alias-vs-symbol ambiguity can be made zero-FP, else keep deferred.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle-233 implementation plan (orchestrator in-thread per feedback-spawned-agents-refuse-code-edits; cycle-231 precedent) ===

SCOPE DELIVERED THIS CYCLE: the FEASIBLE zero-FP subset of TASK-0392 — back-ticked e2e example-PATH cites of shape `NN-name/variant` (slash-separated, digit-led). The bare snake_case ALIAS shape (`gather_2out_loop`) stays DEFERRED (genuinely ambiguous vs ordinary symbols; high FP).

EMPIRICAL GROUNDING (orchestrator, gitignore-respected rg): exactly 7 unique back-ticked `NN-name/variant` refs in *.rs today (01-elementwise-add/naive, 03-reduction/distributed, 05-stencil/{distributed,distributed-2d,reuse}, 06-separable-filter/distributed, 18-multigather/distributed). ALL 7 resolve to a real nuc-nucleus/examples/NN-name/schedules/variant.sched.nuc. No live lie — purely additive future-proofing against the ec50108 cell-rename class.

DESIGN (mirror check-doc-test-name-staleness, justfile:778):
- new recipe check-doc-cell-path-staleness; wire into just ci after check-doc-test-name-staleness.
- token shape regex requires a LETTER right after BOTH the leading NN- and the / (example dirs are always NN-word, variants always letter-led) so date-like `06-01/2026` cannot false-match.
- rule: for back-ticked `NN-name/variant`, FAIL unless nuc-nucleus/examples/NN-name/schedules/variant.sched.nuc exists. Catches BOTH example-dir rename and variant/cell rename. examples/ is the documented source-of-truth (e2e-matrix.toml header).
- POSIX-portable (mktemp+trap, no <()), gitignore-respecting rg, -g !target/**.
- bite-proof RUN1 (current tree OK) / RUN2 (inject `18-multigather/bogus` -> FAIL with loc) / RUN3 (revert OK).

GATE: nix develop -c just build clippy test test-release e2e + the new fence; then parallel read-only qa-test-runner + mped-architect.

=== Cycle-233 DELIVERY (orchestrator in-thread; commit f9344ba) ===

DELIVERED (feasible zero-FP SUBSET, NOT the full task): new justfile recipe check-doc-cell-path-staleness + ci wiring (after check-doc-test-name-staleness). Validates back-ticked e2e cell-PATH cites of shape `NN-name/variant` in .rs against nuc-nucleus/examples/NN-name/schedules/variant.sched.nuc. Catches BOTH example-dir rename and schedule/variant rename. Mirrors the cycle-231 sibling fence structurally (schedule-file resolution target instead of fn defs).

BITE-PROVEN: RUN1 current tree OK (7 unique cell-paths / 12 citation sites, all resolve); RUN2 injected `18-multigather/bogus-variant` -> FAIL with precise loc + correct "no schedule file" branch; RUN3 revert OK.

GATE (qa-test-runner re-ran, NOT implementer-claimed): build clean; clippy 0/0 (doc_lazy_continuation did NOT fire); test 1206/0/3 dev; test-release 1205/0/3 (1-test delta = the debug_assert-gated #[should_panic] compiled out, expected); e2e 385/328/0/57/0 x3 identical (non-flake). qa GO + architect GO.

REVIEW FOLD-BACK (both findings fixed IN-THREAD before commit, in the same recipe comment -- soft-claim shapes this very fence polices):
- P2 (qa+architect): "exactly 7 such refs" was ambiguous -> "7 unique cell-paths (12 citation sites)".
- P1 (architect): zero-FP rationale omitted the LOAD-BEARING trailing back-tick anchor (what actually excludes suffixed `05-stencil/distributed.sched.nuc` [a `.`] and deeper `14-hearing-aid/schedules/...` [2nd `/`] in-tree siblings) -- added an explicit invariant note so a future maintainer relaxing the regex is warned. Verified both sibling shapes exist in-tree and are correctly NON-matched.

STAYS IN PROGRESS (honest scope, architect-confirmed): the task also wants the BARE snake_case cell-ALIAS shape (`gather_2out_loop`, the literal ec50108 token). That stays DEFERRED -- a bare alias is not disambiguable from an ordinary symbol mention without a curated alias map (high FP). REMAINING DELIVERABLE on this task = the bare-alias arm (only build with a curated alias map or keep deferred).

GOTCHA for the bare-alias arm (forward to whoever picks it up): the `NN-name/variant` arm is zero-FP ONLY because the slash+leading-digits+closing-backtick shape is self-disambiguating. A bare alias has none of that -- do NOT attempt a loose snake_case matcher; it will FP on ordinary symbols. The viable path is a curated alias map (cell-alias -> current cell-path) checked into e2e-matrix.toml or a sidecar, then validate cites against it.
<!-- SECTION:NOTES:END -->
