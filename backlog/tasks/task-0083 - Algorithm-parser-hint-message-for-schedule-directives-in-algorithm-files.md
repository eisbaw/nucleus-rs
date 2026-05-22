---
id: TASK-0083
title: 'Algorithm parser: hint message for schedule directives in algorithm files'
status: Done
assignee: []
created_date: '2026-05-18 00:03'
updated_date: '2026-05-22 21:27'
labels:
  - compiler
  - language
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Grammar-algo.md §3 promises that misplaced schedule directives (block=, vectorize=, transfer=, buffer=, notify=, place, place_data) get a HELPFUL hint such as 'did you mean to put  in a *.sched.nuc file?'. Today the parser surfaces a generic 'unexpected =' message. Add a try_map / labelled-error layer that detects these keywords-as-idents in statement position and emits a tailored hint. Touches src/algo/parser.rs and ParseErrorKind.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 62 (2026-05-22) — closed. Algorithm parser now emits a helpful hint when schedule directives (block/buffer/notify/partition/pipeline/transfer/unroll/vectorize/place/place_data/check loop) appear in algo files.

Implementation: nucleus/compiler/src/algo/parser.rs adds SCHED_RESERVED_EQ const + sched_directive_hint_stmt() parser (4 arms: <kw>=, place <ident>, place_data <ident>, check loop) + sched_hint_msg(kw) single-source-of-truth wording. Wired FIRST in stmt_parser's choice() so the hint wins over generic 'expected (' / 'unexpected =' errors.

Sample hinted error: 'parse error at line 3, column 5: `block` is a schedule directive — did you mean to put it in a `*.sched.nuc` file?'

Tests: 11-case table-driven sched_directive_hint_fires_for_each_keyword + sched_directive_hint_does_not_break_keyword_as_plain_ident (pins that these keywords still work as plain idents — e.g. 'data block : f32[16]; block <-- src;' still parses, since SCHED_RESERVED_EQ are NOT in KEYWORDS) + strengthened existing negative_unknown_keyword_in_algorithm to assert the hint text + column anchor.

Honest limits documented by implementer:
- 'Furthest-end-position wins' is a chumsky-0.9-specific dependency. A future chumsky bump or new statement-level alternative could affect which error surfaces. Robust design would explicitly suppress competing errors — deferred.
- place IDENT / place_data IDENT arms consume the ident, so error-recovery starts past the ident rather than at it. Acceptable; recovery is best-effort.
- Unreachable Stmt::Effect('__unreachable_sched_hint__', []) placeholder exists to satisfy chumsky's type system on a never-taken Ok branch. Phantom code; replace with or_else explicit error injection in a future cycle.

Gate (cycle 62): just test 0 FAILED + 2 new tests pass; just clippy clean; just e2e 88/70/0/18 UNCHANGED.

Review-gate: QA GO. Architect review skipped (small parser enhancement, byte-identicality not affected — error path only).
<!-- SECTION:FINAL_SUMMARY:END -->
