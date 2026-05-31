---
id: TASK-0370
title: >-
  Widen just check-narrative-doc-lie beyond e2e-matrix.toml to backlog/tasks,
  schedule .nuc, READMEs, source docstrings
status: Done
assignee:
  - '@mped'
created_date: '2026-05-30 11:08'
updated_date: '2026-05-31 02:39'
labels:
  - tooling
  - ci
  - doc-lie
  - robustness
  - cycle-213-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (R5, robustness). VERIFIED: the check-narrative-doc-lie recipe in the justfile targets only nuc-nucleus/e2e-matrix.toml, but the comment/doc-lie class is the projects #1 recurring defect (12+ firings) and fires across backlog/tasks/*.md, schedule .nuc headers, README files, and source docstrings — currently caught only by repeated MANUAL citation sweeps (open: TASK-0308/0311/0312/0313 and the cycle-213 P2 fix). Extend the recipes pattern set + file targets so the structural check covers those locations, converting recurring manual sweeps into a gate-time catch. Must stay zero-false-positive on the current tree (same bar as the other check-* recipes).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 check-narrative-doc-lie scans backlog/tasks/*.md, nuc-nucleus/examples/*/schedules/*.sched.nuc headers, README.md files, and crate source docstrings (or a justified subset) in addition to e2e-matrix.toml
- [ ] #2 The widened patterns capture at least the historically-recurring lie shapes (stale absolute-line citations, phantom function names, "every X" claims without a grep-witness, "only N backends remain" staleness) and run clean (exit 0, zero false positives) on the current tree
- [x] #3 Wired into just ci so a future doc-lie in the covered locations fails the gate
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTATION PLAN (cycle-220, implementer) — chosen subset + WHY.

EMPIRICAL SCOPING (reproduced orchestrator findings + extended):
- Naive widening of the existing PRESENT-TENSE pattern set to new file globs FLOODS false positives: 171 hits on backlog/tasks, ~19 on READMEs, etc., nearly all legitimate domain language (e.g. ignore="...not yet implemented", true backlog statuses). Pattern-class widening is the trap. REJECTED.
- The recurring lie SHAPES that ARE objective are line/file citations (cycle-138 stale-line, cycle-181b split-file deixis). I measured them.
- BARE-BASENAME citations (lib.rs:N, multi_worker.rs:N) are INTRACTABLE for zero-FP: ambiguous resolution root (12 lib.rs files), and cross-crate prose references make even same-crate resolution MISATTRIBUTE. Proof: a crate-relative resolver flags check_frame.rs lib.rs:991-997 as "stale in backend-common/src/lib.rs (92 lines)" but the prose actually means pthreads-sync/lib.rs — wrong file, wrong verdict. REJECTED bare-basename resolution.
- FULLY-QUALIFIED citations (nucleus/<crate>/src/<path>.rs:N or nuc-nucleus/...) have EXACTLY ONE resolution = unambiguous = zero "which file" guessing. THIS is the zero-FP-safe subset.
- backlog/tasks/*.md MUST be EXCLUDED: (a) CLAUDE.md forbids hand-editing task md; (b) task descriptions legitimately cite FILING-TIME line numbers that are now stale-by-design (e.g. task-0340.01 title encodes "lib.rs-1997-LoC"; file now 329 LoC) — immutable historical provenance, not lies. Including them would force FP-flood or history-rewrite. 13 stale-line + ~29 nofile fully-qualified citations live there, all historical.

CHOSEN SUBSET: a NEW objective recipe check-doc-citation-staleness that scans NON-backlog targets (nucleus/**/src + tests **/*.rs, docs/, README*.md, PRD.md, nuc-nucleus/) for FULLY-QUALIFIED .rs:N / .rs:N-M / .rs:N..M citations and asserts file-exists AND max-cited-line <= wc -l. Objective => NO escape-hatch needed (addresses the markdown # / .rs // escape-hatch-syntax gap). Catches stale-line + split-file(NOFILE) citations in the editable surface; guards future fully-qualified citations gate-time.

LIMITATIONS (deferred, follow-ups to file): bare-basename citations (the bulk of source citations); stale-CONTENT where the line still exists but the code moved (line-count check cannot see this); present-tense narrative prose in md/nuc (FP-floods). The existing check-narrative-doc-lie TOML check is unchanged.

ORCHESTRATOR REVIEW GATE (2026-05-31, independent, read-only): GO x2 with in-thread hardening. qa-test-runner: new recipe exits 0 on current tree (zero-FP), BITE-tested (fails on past-EOF + split-into-directory citations, recovers to 0), sibling check-* recipes pass, build/clippy clean, test 1165/1164/3, e2e 329/272/0/57/0. mped-architect: doc re-anchors independently VERIFIED accurate (pthreads-sync lib.rs Real-time check loop comment + emit_log_branch/emit_count_branch at event_walker.rs all exist) — no new doc-lie introduced at the doc-fix peak-risk site. IN-THREAD FIXES applied by orchestrator post-review: (P2.1) re-anchored the one disclosed live stale citation mp-tcp-event/tests/multi_worker_emit.rs:646 (cited multi_worker.rs:854, symbol build_fails_on_missing_sidecar_buffer_entry actually at pthreads-async/src/multi_worker.rs:905) to a symbol-only anchor, dropping the rotted line number per cycle-138 — do not ship a doc-lie fence while leaving a known live doc-lie; (P3.1) hardened maxl extraction sed|awk -> grep -oE [0-9]+$ + numeric guard so the range forms .rs:N-M / .rs:N..M the regex admits cannot silently false-negative; (P3.2) wc -l -> awk END{print NR} so a no-trailing-newline file (docs/README in scope, not rustfmt-newline-enforced) cannot false-positive. Re-verified: hardened recipe still exits 0 AND still bites (range form lib.rs:50..99999 now correctly parsed to STALE).

P2.2 HONEST FRAMING (architect): the fence value is PROSPECTIVE, not retroactive. Today it scans ~1 distinct live fully-qualified citation (nucleus/driver/src/main.rs:399, in-range); the bulk of real citations are bare-basename (deferred TASK-0382) and the one fixable live FQ stale cite was hand-fixed this cycle. So the gate currently defends FUTURE fully-qualified citations, not meaningful current debt. The Final Summary should be read with that scope. Separately, the cycle-220 review gate surfaced a PRE-EXISTING just-ci RED (check-mega-files fails on sidecar.rs 1164 + embedded-pattern/tests.rs 1030, neither caused by this cycle nor in scope) — filed as TASK-0383.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DELIVERED (cycle-220, commit f4edda4): new objective recipe check-doc-citation-staleness, wired into just ci after check-narrative-doc-lie. Verifies every FULLY-QUALIFIED nucleus/...rs:N (and .rs:N-M / .rs:N..M) citation resolves to an existing file with the cited line in range. Catches cycle-138 stale-line-past-EOF + cycle-181b split-file deixis. No escape-hatch needed (objective).

PROOF zero-FP: nix develop --command bash -c "just check-doc-citation-staleness" exits 0 on the current tree (output: "OK: every fully-qualified nucleus/*.rs:N citation resolves to an in-range line."). BITE-tested: injecting a fake :99999 line citation + a citation to the split-away multi_worker_walker.rs both FAIL the recipe; a valid :1 citation passes. NOT defanged.

FIXED 1 in-tree finding (the only one outside backlog/tasks): docs/check-loop-latency-max.md cited multi_worker_walker.rs:300..355 (file is now the directory multi_worker_walker/ -> NOFILE) and pthreads-sync/src/lib.rs:694..758 (stale-CONTENT: line 694 now holds render_reuse_marker_comment, not the single-worker check emit). Re-anchored BOTH to stable symbol/comment names (no line numbers) per the cycle-138 prefer-symbol-anchor rule -> immune to the very staleness this fence guards.

AC STATUS (honest):
- AC#3 (wired into just ci): FULLY MET.
- AC#1 (scans the new targets, "or a justified subset"): PARTIALLY MET via a justified subset — fully-qualified citations across source .rs + docs/ + README*.md + PRD.md + nuc-nucleus/; backlog/tasks DELIBERATELY excluded (immutable historical provenance, CLAUDE.md forbids hand-editing). NOT ticked because it is a subset, not the full target list.
- AC#2 (capture the 4 named lie shapes, zero-FP): PARTIALLY MET — captures stale absolute-line citations (1 of the 4 named shapes) + split-file deixis, zero-FP PROVEN. Does NOT capture phantom function names, "every X" claims, or "N backends" prose (those FP-flood or need code cross-reference; see limitations). NOT ticked.

DEFERRED BREADTH -> TASK-0382 (dep TASK-0370), referenced by name in the recipe comment: (i) bare-basename citations (the BULK of source citations — ambiguous resolution + cross-crate-prose misattribution; a genuine uncaught stale one exists: mp-tcp-event/tests/multi_worker_emit.rs:646 cites multi_worker.rs:854 @296 LoC); (ii) stale-CONTENT (line exists, code moved); (iii) present-tense narrative-prose scanning of md/.sched.nuc (171 legitimate FP on backlog/tasks with the existing pattern set).

GOTCHAS for the next person: bare-basename resolution is a FP TRAP — a crate-relative resolver flags check_frame.rs cites of "pthreads-sync at lib.rs:991" as stale against backend-common/src/lib.rs (wrong file). Fully-qualified-only is the only zero-FP class. Line-count-only checks miss stale-content. The present-tense pattern set CANNOT be pointed at md prose without flooding.

GATE: build clean, clippy clean, test 1165/0, test-release 1164/0 (1-test delta = expected should_panic/debug_assert divergence), e2e 329/272/0/57/0 (baseline UNCHANGED). Marking Done with explicit subset scope; AC#1/#2 left UNTICKED (honest partial) with the breadth tracked in TASK-0382.
<!-- SECTION:FINAL_SUMMARY:END -->
