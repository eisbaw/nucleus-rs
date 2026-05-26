---
id: TASK-0336
title: Lift driver host-election rule to shared helper (cycle-163b architect P2.5)
status: To Do
assignee: []
created_date: '2026-05-26 04:56'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

cycle-163b architect P2.5 fold-back finding: driver `nucleus/driver/src/main.rs` independently mirrors `Plan::build`'s host-election rule in THREE conditional wirings:

- cycle-160 `apply_host_mediation_inject` (CTRL-arm host mediation).
- cycle-162 `apply_safe_push_reorder` (slice 1 / Option D event-list-layer reorder).
- cycle-163 `apply_host_data_relay_inject` (slice 2 / Option B2 ACFG-layer routing).

Each site independently picks: 'worker literally named "host" filtered by used_workers, else used_workers.iter().next()'. The rule is currently respected at all 3 sites (cycle-163 QA verification GREEN), but the mirroring surface is exactly the `feedback-driver-must-mirror-backend-election-exactly` recurrence — adding a 4th wiring (e.g., a slice-3 threaded relay) or refactoring `Plan::build`'s rule risks drift.

## Acceptance criteria

### AC#1: shared helper

Lift the host-election rule to a single helper, e.g. `pub fn elect_host(used: &BTreeSet<WorkerId>, name_workers: &BTreeMap<String, WorkerId>) -> WorkerId` in `backend-common` (or wherever `Plan::build` itself can consume it). All 3 driver wirings + `Plan::build` call the helper instead of inlining the rule.

### AC#2: regression pin

Negative test: if the helper is removed and the rule re-inlined in 2 of the 4 sites with a divergence, the test catches the drift. Likely: a parameterised test exercising 'named host', 'first-used fallback', 'tied-name resolution' across both sites.

### AC#3: no behavioral change

`just e2e` baseline preserved (no cell regresses or promotes); host-election outcome for every existing cell is byte-identical pre/post-refactor.

## Cross-reference

- TASK-0329.01.02 cycle 163b architect P2.5 finding (parent fold-back commit).
- Memory `feedback-driver-must-mirror-backend-election-exactly` — load-bearing for this task's existence.
- `nucleus/driver/src/main.rs` — 3 wiring sites (search for `apply_host_mediation_inject`, `apply_safe_push_reorder`, `apply_host_data_relay_inject`).
- `nucleus/backends/mp-tcp-event/src/multi_worker.rs` `Plan::build` (lines around 153-160) — the source-of-truth rule that driver mirrors.

## Honest scope

LOW priority — currently zero defects (3-of-3 sites correct as of cycle 163b). This is hardening / fragility-reduction. Promote priority on first drift instance.
<!-- SECTION:DESCRIPTION:END -->
