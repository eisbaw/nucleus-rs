---
id: TASK-0443
title: >-
  Observe ci.yml GH-action runtime glue on a REAL GitHub runner (throwaway
  private fork) — faithful walltime + install-nix/cache
status: To Do
assignee: []
created_date: '2026-06-04 09:24'
labels: []
dependencies:
  - TASK-0442
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Residual from TASK-0442 (promoted from a notes-only mention to a real node so it does not rot inside a Done task — cf. feedback-opacity-gate-rot).

TASK-0442 observed ci.yml nix-only: actionlint exit 0 + act -l + act --dryrun proved the YAML is valid and its job graph (triggers/matrix/if-guards) resolves as designed, and all four payload recipes exist + are locally green. What remains UNOBSERVED on any faithful runner: the GitHub-action runtime glue actually executing — actions/checkout@v4, cachix/install-nix-action@v27 installing Nix on a hosted runner, DeterminateSystems/magic-nix-cache-action@v8 against GitHub's cache service, actions/cache@v4 — AND the hosted-runner WALLTIME for the ~1-2GB renode/embedded Nix closure cold-start (the original cost concern). act-under-rootless-podman canNOT faithfully reproduce these (install-nix would fail for harness/privilege reasons unrelated to ci.yml).

GATING CONDITION (do NOT do speculatively): only worth doing IF a clickable green CI badge a reviewer can click becomes a requirement. For thesis-defence local-reproducibility, TASK-0442's nix-only evidence is sufficient and this stays parked.

Scope when picked up: push a throwaway private fork to GitHub, trigger one ci.yml run (workflow_dispatch), observe which jobs go green / which hit walltime (esp. renode-multimcu cold-start), and report. If a job fails or times out, capture the real error and file the fix. Note: git remote is currently EMPTY by design.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 only undertaken IF a green-CI-badge requirement materializes (else stays parked, documented)
- [ ] #2 one real ci.yml run observed on a GitHub runner via throwaway private fork; per-job green/fail + renode-multimcu cold-start walltime recorded
- [ ] #3 any real-runner failure (walltime, install-nix, cache) captured with the actual error + a fix task filed
- [ ] #4 throwaway fork removed afterward; no secrets/PII pushed (run secret-scan before push)
<!-- AC:END -->
