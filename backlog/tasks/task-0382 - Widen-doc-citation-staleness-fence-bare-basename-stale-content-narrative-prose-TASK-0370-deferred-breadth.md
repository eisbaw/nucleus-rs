---
id: TASK-0382
title: >-
  Widen doc-citation-staleness fence: bare-basename + stale-content +
  narrative-prose (TASK-0370 deferred breadth)
status: To Do
assignee: []
created_date: '2026-05-31 02:20'
updated_date: '2026-05-31 03:12'
labels:
  - tooling
  - ci
  - doc-lie
  - robustness
  - cycle-220-followup
dependencies:
  - TASK-0370
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-220 TASK-0370 delivered check-doc-citation-staleness covering FULLY-QUALIFIED nucleus/*.rs:N citations (file-exists + line-in-range) over source/docs/READMEs/PRD/nuc-nucleus, excluding backlog/tasks. Three lie-shape/target classes were DEFERRED because they resist zero-FP mechanization in one cycle (empirically measured in TASK-0370):

(i) BARE-BASENAME citations (lib.rs:N, multi_worker.rs:N) — the BULK of source citations. Ambiguous resolution root (12+ lib.rs files) and cross-crate prose references (e.g. check_frame.rs "pre-extraction pthreads-sync at lib.rs:991") MISATTRIBUTE under a naive crate-relative resolver. Needs a crate-scoped, prose-aware resolver (only validate when the basename is unique within the citing crate AND the surrounding prose does not name another crate). Genuine same-crate stale citation example currently in-tree but uncaught: mp-tcp-event/tests/multi_worker_emit.rs:646 cites multi_worker.rs:854 (file now 296 LoC).

(ii) STALE-CONTENT detection — line still exists but the code at it moved (e.g. docs cited pthreads-sync/src/lib.rs:694..758 for single-worker check-emit; line 694 now holds render_reuse_marker_comment). Line-count check cannot see this; needs a content fingerprint / symbol-anchor convention.

(iii) PRESENT-TENSE NARRATIVE PROSE scanning of *.md and *.sched.nuc headers — the existing check-narrative-doc-lie pattern set FP-floods here (171 legitimate hits on backlog/tasks alone). Needs either a high-discipline curated target (not general prose) or a fundamentally different objective shape.

Also consider: should backlog/tasks citations be validated at all? Currently excluded as immutable filing-time-historical provenance (CLAUDE.md forbids hand-editing task md). A "historical-citation must carry a cycle/filing stamp" convention could make them auditable without rewriting history — design question, not obviously worth it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Bare-basename citations validated zero-FP via a crate-scoped prose-aware resolver (or a documented decision that this stays out of scope)
- [ ] #2 Stale-content (line-exists-but-code-moved) detection OR a project convention (symbol-anchor mandate) that makes it moot
- [ ] #3 Decision recorded on present-tense narrative-prose scanning of md/.sched.nuc (curated target or explicit out-of-scope)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-220 TASK-0383 split produced a fresh in-tree instance of the BARE-BASENAME-LOCATION lie this task's AC#1 targets, and it was caught ONLY by manual silent-sibling grep — the check-doc-citation-staleness fence is structurally blind to it. Concrete: splitting the BIN tests out of embedded-pattern/src/tests.rs into tests/bin_shape.rs left a comment in embedded-pattern/src/lib.rs reading 'pinned by bin_rejects_multi_worker_* in tests.rs' — now a LIE (the test lives in tests/bin_shape.rs). It carried no `:N` line number (just the bare basename `tests.rs` as a location claim in prose), so the fully-qualified fence never saw it. This is a NEW lie-shape variant for AC#1's scope: not 'bare basename + line number' but 'bare basename as a prose location claim with NO line number'. A crate-scoped resolver keyed on `<basename>.rs` tokens in `//` comments (validating that a named test/symbol still resides in the cited file) would have caught it. Generalises: ANY split-and-cite cycle (the TASK-0340 epic + this sibling) is a high-yield source of exactly this defect — worth a dedicated grep arm even before the full prose-aware resolver lands.
<!-- SECTION:NOTES:END -->
