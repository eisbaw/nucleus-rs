---
id: TASK-0465
title: >-
  task0422_01_inv2_post_mediation.rs: migrate MEDIATED_BACKENDS name-list oracle
  to capability flags + fix stale driver citations
status: To Do
assignee: []
created_date: '2026-06-10 10:44'
labels:
  - driver
  - test
  - silent-sibling
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from TASK-0455.09 (capability-driven mediation, commit c2e245a) + wave-2 architect review P3.4. The driver test task0422_01_inv2_post_mediation.rs keeps the LAST surviving copy of the backend-name lists: a MEDIATED_BACKENDS const (~line 72) used as the test oracle for which backends get post-mediation EventList validation. Migrate it to read the same capability flags production now uses (load_capabilities + star_topology_host_mediation etc.), de-duplicating the oracle — otherwise a new mediated backend updates capabilities.toml but silently leaves this sweep un-covering it (silent-sibling class).

Also in the same file (review P3.4): comments at ~lines 70-71, 81, 94 still describe the DELETED driver name-list gate in present tense with hard line numbers (src/main.rs:531, :484-493, :537-546 — all gone/shifted by c2e245a). Fix to describe the capability-flag gate, using symbol references not line numbers.

Note: keep the test ORACLE independent enough to still catch a production regression — reading the same toml as production is acceptable because the test asserts the VALIDATION ran per mediated backend, not the flag values themselves; record the reasoning in the test header.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 MEDIATED_BACKENDS const replaced by capability-flag-derived set; a new backend flipping star_topology_host_mediation=true is automatically covered by the sweep (prove with a temp-toml test or equivalent)
- [ ] #2 Stale present-tense name-list/driver-line comments in the file corrected to the capability-flag reality (symbols, not line numbers)
- [ ] #3 Sweep still green; non-vacuity floor preserved
<!-- AC:END -->
