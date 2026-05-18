---
id: TASK-0091
title: Relax declarations-before-use in AlgoIR lowering
status: To Do
assignee: []
created_date: '2026-05-18 00:25'
labels:
  - M0
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0009 enforces declarations-before-use by lowering items in source order. If a real example needs forward references between consts/data/kernels, switch to a two-pass lowering: collect declarations first, then evaluate. Out of scope until a driving example needs it.
<!-- SECTION:DESCRIPTION:END -->
