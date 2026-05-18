---
id: TASK-0148
title: 'Petri DOT: visual token dots inside places for initial marking'
status: To Do
assignee: []
created_date: '2026-05-18 05:19'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0035 places carry the initial marking as a numeric label (`name\\n<initial>/<capacity>`). PRD §8.5 mentions "initial marking is rendered as dots inside places". Rendering N small black dots inside each place circle (N = initial marking) is more glanceable for human inspection — Graphviz can stack child nodes or use a record/HTML label. Defer until a user complains; the numeric label is unambiguous.
<!-- SECTION:DESCRIPTION:END -->
