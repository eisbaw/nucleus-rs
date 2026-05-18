---
id: TASK-0089
title: Enforce kernel-purity vs statement-form
status: To Do
assignee: []
created_date: '2026-05-18 00:24'
labels:
  - M0
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After AlgoIR lowering, validate that dataflow-stmt RHS calls reference pure kernels and that effect-stmt callees reference effectful kernels. The IR preserves purity on ResolvedKernel; the check is a small pass over IrStmt. Filed as a follow-up from TASK-0009.
<!-- SECTION:DESCRIPTION:END -->
