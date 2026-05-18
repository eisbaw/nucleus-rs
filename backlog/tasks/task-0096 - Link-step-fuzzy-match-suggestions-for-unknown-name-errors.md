---
id: TASK-0096
title: 'Link step: fuzzy-match suggestions for unknown name errors'
status: To Do
assignee: []
created_date: '2026-05-18 00:42'
labels:
  - compiler
  - link
  - diagnostics
  - M0-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
When the link step emits UnknownKernel / UnknownData / UnknownLoop / UnknownTransferData, it currently reports the offending name verbatim. For typo-class user mistakes, a 'did you mean X?' hint (Levenshtein distance against the algorithm's symbol table) would materially help. The link step has both symbol tables in hand. Acceptance: a single LinkError variant carries an Option<String> suggestion; the test for negative cases asserts the suggestion when one is computable.
<!-- SECTION:DESCRIPTION:END -->
