---
id: TASK-0144
title: 'block=N: reject bad combinations with other loop options'
status: To Do
assignee: []
created_date: '2026-05-18 04:25'
labels:
  - M3
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0030 AC #5: 'block= is applied left-to-right with other loop options; some combinations may not yet be supported and should be rejected'.

The current pass handles block= in isolation. Bad combinations to reject (PRD §6.3.3 says 'bad combinations are rejected at compile time, not at runtime'):
- block=N with unroll=M where M does not divide N
- vectorize=M with block=N where vectorise width and tile size disagree
- partition=blocks2d on a non-2D loop nest
- pipeline=D with block=N where D >= num_tiles

Each combination needs an entry in the schedule-validate pass (a sibling of block_transform that runs before any transform) returning a clear error.

Also: detect 'block=64, block=128' on the same loop (currently last-wins; should be DuplicateLoopOption).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 schedule-validate pass rejects each named bad combination with a clear error
- [ ] #2 unit tests cover each rejection path
- [ ] #3 valid combinations (e.g. block=N alone, or block=N + reuse) continue to work
<!-- AC:END -->
