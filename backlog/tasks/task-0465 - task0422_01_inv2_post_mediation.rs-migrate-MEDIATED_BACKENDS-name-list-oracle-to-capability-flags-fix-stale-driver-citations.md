---
id: TASK-0465
title: >-
  task0422_01_inv2_post_mediation.rs: migrate MEDIATED_BACKENDS name-list oracle
  to capability flags + fix stale driver citations
status: Done
assignee: []
created_date: '2026-06-10 10:44'
updated_date: '2026-06-10 13:08'
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
- [x] #1 MEDIATED_BACKENDS const replaced by capability-flag-derived set; a new backend flipping star_topology_host_mediation=true is automatically covered by the sweep (prove with a temp-toml test or equivalent)
- [x] #2 Stale present-tense name-list/driver-line comments in the file corrected to the capability-flag reality (symbols, not line numbers)
- [x] #3 Sweep still green; non-vacuity floor preserved
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle work (agent): migrated MEDIATED_BACKENDS const -> capability-flag-derived set in driver/tests/task0422_01_inv2_post_mediation.rs. New machinery: ALL_BACKENDS scan-universe list + caps_path() + mediated_from_caps(pure filter on star_topology_host_mediation, carries host_data_relay) + mediated_backends() (loads each backends/*/capabilities.toml via production load_capabilities, fail-loud on missing/garbled + name-field pin). Sweep + both probes share the single mediated_from_caps path (de-dup). AC#1: new test derived_set_auto_covers_a_new_mediated_backend proves auto-coverage two ways: (1) temp capabilities.toml under CARGO_TARGET_TMPDIR through production load_capabilities, (2) pure mediated_from_caps over synthetic in-memory Capabilities (fictional star backend included, non-mediated peer excluded). AC#2: stale present-tense src/main.rs line citations (:484-493/:531/:537-546/~464-553/:513-520) replaced with SYMBOL refs (apply_host_mediation_inject / apply_host_data_relay_inject / caps.star_topology_host_mediation / caps.host_data_relay) in elect_host, apply_mediation_only, apply_data_relay, header, pipelined-docstring. AC#3: non-vacuity floor preserved + added !mediated.is_empty() floor; ok>=55*mediated.len() now derived. Independence reasoning recorded in test header (Why reading the same toml is still a sound oracle: flag-value oracle is task0455_09; this test asserts validation RAN+held per mediated backend, catches mediation-pass regression). VERIFIED: cargo test -p nucleus --test task0422_01_inv2_post_mediation dev 4/4 + release 4/4; rustfmt clean; clippy clean on driver crate (when backend-common buildable).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
MEDIATED_BACKENDS const retired: the sweep derives its mediated set from backends/*/capabilities.toml via the PRODUCTION load_capabilities, fail-loud on load errors; auto-coverage proven by a temp-toml probe asserting the fictional backend lands exactly in the derived set with its relay bit (pid-unique temp file per scratch convention, fold-in 9f3434b). All stale src/main.rs line citations replaced with symbols. Independence reasoning in the test header (flag VALUES delegated to the task0455_09 frozen pin + e2e differential). Review P2.4 closed in the same fold-in: ALL_BACKENDS <-> directory set-equality pins added to BOTH oracle files, so a new backend dir fails loud. Tests 4+pin/0 dev AND release. Landed b7ef0df; architect GO.
<!-- SECTION:FINAL_SUMMARY:END -->
