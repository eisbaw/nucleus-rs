---
id: TASK-0083
title: 'Algorithm parser: hint message for schedule directives in algorithm files'
status: To Do
assignee: []
created_date: '2026-05-18 00:03'
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
