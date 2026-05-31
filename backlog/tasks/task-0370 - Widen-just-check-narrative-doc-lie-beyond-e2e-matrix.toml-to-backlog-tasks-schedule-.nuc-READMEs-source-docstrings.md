---
id: TASK-0370
title: >-
  Widen just check-narrative-doc-lie beyond e2e-matrix.toml to backlog/tasks,
  schedule .nuc, READMEs, source docstrings
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-30 11:08'
updated_date: '2026-05-31 02:12'
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
- [ ] #3 Wired into just ci so a future doc-lie in the covered locations fails the gate
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
<!-- SECTION:NOTES:END -->
