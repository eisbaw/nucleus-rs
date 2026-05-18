---
id: TASK-0081
title: 'Algorithm parser: error recovery (skip-to-next-stmt)'
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
Add recovery so the parser can resume at the next plausible statement / item boundary after a syntactic failure, instead of bailing on the first error. Pairs with TASK-0080 (multi-error reporting). Use chumsky's recovery combinators (nested_delimiters / skip_then_retry_until).
<!-- SECTION:DESCRIPTION:END -->
