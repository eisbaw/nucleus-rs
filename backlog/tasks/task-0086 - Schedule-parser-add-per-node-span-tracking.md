---
id: TASK-0086
title: 'Schedule parser: add per-node span tracking'
status: To Do
assignee: []
created_date: '2026-05-18 00:13'
labels:
  - M0
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0008 self-report follow-up. The schedule parser (nucleus/compiler/src/sched/parser.rs) currently tracks source position only on ParseError, not on AST nodes. Downstream semantic passes (TASK-0010 SchedIR lowering, TASK-0011 link step) will want spans on every directive/option for good diagnostics ('place undefined kernel X at line 42'). Add a Span = (usize, usize) byte-range field to each AST node and have the parser thread chumsky's span info through. Same follow-up exists for the algorithm parser (TASK-0007); coordinate so both parsers expose Span the same way.
<!-- SECTION:DESCRIPTION:END -->
