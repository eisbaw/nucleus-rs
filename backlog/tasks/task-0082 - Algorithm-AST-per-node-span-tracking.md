---
id: TASK-0082
title: 'Algorithm AST: per-node span tracking'
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
AST nodes (Item, Stmt, Expr, ...) currently carry no source-span info. TASK-0009 (AlgoIR lowering) and TASK-0011 (link step) want spans to point at user source in diagnostics. Either (a) wrap each node in a Spanned<T> { node: T, span: Range<usize> }, or (b) attach a parallel side-table keyed by node id. Bias: (a), small wrapper, derive PartialEq via the inner node to keep tests cheap. Update parse_algo accordingly.
<!-- SECTION:DESCRIPTION:END -->
