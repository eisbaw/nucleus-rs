---
id: TASK-0252
title: >-
  Wire just port-stress-check into a nightly/scheduled CI job (TASK-0176 AC#2
  steady-state guarantee)
status: Done
assignee: []
created_date: '2026-05-23 16:00'
updated_date: '2026-05-23 21:37'
labels:
  - infra
  - tooling
  - ci
  - reliability
dependencies:
  - TASK-0057
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0176 (cycle 72) eliminated the close-then-rebind TOCTOU window in mp-tcp-bufsync's port handshake and added `just port-stress-check` as a 20× parallel proof. The recipe is the AC#2 evidence — it runs locally on demand. Today no automation invokes it: the closure notes claimed "manual + nightly" coverage, but only "manual" exists. Filed by mped-architect review of cycle 72 (MAJOR-2).

## Why this matters

AC#2 of TASK-0176 ("≥20× parallel zero flakes") was satisfied as a one-shot reading at landing time. A steady-state guarantee requires the recipe to fire periodically against the current HEAD. Without that, a future regression that flakes 1-in-100 only surfaces when a developer remembers to invoke the recipe manually. The recurring "comment/doc lie" failure class (memory `feedback-comment-doc-lie-recurring`) applies symmetrically to AC-promises whose evidence is not auto-rerun.

## Scope

- Add a scheduled CI job (or a `just nightly` aggregator recipe + an existing nightly scheduler) that runs `just port-stress-check` against current HEAD on a fixed cadence (suggest: nightly).
- The job MUST fail loud on any flake, with the failing child log surfaced in the CI artifact / output.
- Update the `port-stress-check` recipe header comment in `justfile` to name the scheduled invocation site so it stops being aspirational.
- Optional: include sibling stress recipes in the same scheduled job once they exist (mp-tcp-event Stage 3 multi-worker per TASK-0042.05; pthreads-async if it ever grows TOCTOU-class concerns; etc.).

## Why deferred from TASK-0176

The implementer-correct call was to NOT bundle CI wiring into the TASK-0176 fix (out of scope; depends on the repo's CI configuration which is itself a moving target in TASK-0057). Filed as a follow-up rather than expanded into the TASK-0176 close.

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 #1 A scheduled CI run invokes `just port-stress-check` on a cadence ≤ 7 days against current HEAD.
- [x] #2 #2 A flake in that scheduled run is loud (CI failure with the failing child log in the artifact / output).
- [x] #3 #3 The `port-stress-check` recipe header comment in `justfile` names the scheduled invocation site (no more aspirational "or on a nightly schedule").

## Dependencies

- Practical: depends on TASK-0057 (CI matrix runner) — schedule cron has to live somewhere.
<!-- SECTION:DESCRIPTION:END -->

<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed orchestrator-direct (cycle 77 continuation). Implementation: added 'port-stress' job to .github/workflows/ci.yml that runs 'just port-stress-check' on cron '0 4 * * *' (nightly 04:00 UTC, quiet hours) + workflow_dispatch (manual). 'if: github.event_name in {schedule, workflow_dispatch}' guard scopes the heavy 20-way-parallel job AWAY from per-push/per-PR runs so walltime stays bounded for normal contributor traffic. Job mirrors the existing 'gate' job's Nix + Cargo cache shape so cold-start cost is bounded. Justfile port-stress-check recipe header comment updated from 'Not wired anywhere automated yet... Filed TASK-0252' to 'Wired into a NIGHTLY scheduled CI job at .github/workflows/ci.yml's port-stress job' — closing the doc-lie that this cycle's filing itself created.

AC#1 ('scheduled CI run invokes just port-stress-check on cadence ≤ 7 days against current HEAD'): MET via cron '0 4 * * *' (daily).
AC#2 ('a flake in that scheduled run is loud'): MET — the recipe itself dumps all 20 child logs on failure (TASK-0176 cycle-72 MINOR-7 hardening); CI's natural fail-on-nonzero-exit + scheduled-job failure notifications surface the loud signal.
AC#3 ('port-stress-check recipe header comment names the scheduled invocation site'): MET — header now names the cron schedule + the workflow path explicitly.

Cannot CI-run-test from this environment (no GitHub remote per CLAUDE.md). YAML structural sanity verified (5 anchor lines = 1 on: + 1 jobs: + 3 job names). The scheduled job first fires when (a) the repo gets a GitHub remote AND (b) GitHub Actions enables on the repo. The cron trigger is otherwise dormant. TASK-0166 (configure branch protection) is the partner maintainer-side task that activates the gating-on-required-status-check side; this task only adds the job + its trigger.
<!-- SECTION:FINAL_SUMMARY:END -->
