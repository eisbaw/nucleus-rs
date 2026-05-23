---
id: TASK-0147
title: Golden DOT snapshot tests for --emit-pn and ACFG->Petri
status: Done
assignee: []
created_date: '2026-05-18 05:19'
updated_date: '2026-05-23 20:49'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0035 AC #4 asked for golden DOT files per example × required schedule with CI diff. TASK-0026 AC #4 asks for the same for the ACFG -> Petri pass output. TASK-0135 already filed the ACFG -> Petri half. This task scopes the unified snapshot infrastructure: where the golden files live (likely `nuc-nucleus/examples/<NN>/schedules/<S>.expected.dot`), how they are regenerated (a `just update-snapshots` recipe), and what the CI failure message looks like. Defer until at least one DOT-emitting pass has had a label refactor and we know which parts of the output are 'load-bearing vs label cosmetics'.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77). Description explicitly says 'Defer until at least one DOT-emitting pass has had a label refactor and we know which parts of the output are load-bearing vs label cosmetics'. Trigger condition unchanged — no label refactor has happened yet. Reopen when one lands and we have the load-bearing-vs-cosmetic distinction.
<!-- SECTION:FINAL_SUMMARY:END -->
