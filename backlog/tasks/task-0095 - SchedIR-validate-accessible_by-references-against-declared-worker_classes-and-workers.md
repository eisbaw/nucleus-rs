---
id: TASK-0095
title: >-
  SchedIR: validate accessible_by references against declared worker_classes and
  workers
status: To Do
assignee: []
created_date: '2026-05-18 00:33'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
memory_region R { accessible_by = { name1, name2 } } currently passes through to the IR without checking whether name1/name2 are declared worker_class or worker names. Grammar sched.md sec.2 note 4 says "resolution is the linker's job" — but for accessible_by the resolution is purely schedule-internal and can be done in SchedIR lowering. Add validation.
<!-- SECTION:DESCRIPTION:END -->
