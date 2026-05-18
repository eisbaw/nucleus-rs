---
id: TASK-0080
title: 'Algorithm parser: multi-error reporting'
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
Surface more than one parse error in a single pass. chumsky's Simple<char> already collects multiple alternatives at the same position; what we need is to recover from one syntactic failure and continue parsing the rest of the input, then bundle the collected errors into a Vec<ParseError> on return. Touches src/algo/parser.rs (recovery combinators) and the ParseError signature (probably becomes Vec<ParseError> or a struct that owns Vec).
<!-- SECTION:DESCRIPTION:END -->
