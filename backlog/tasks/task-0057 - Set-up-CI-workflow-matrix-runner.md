---
id: TASK-0057
title: Set up CI workflow (matrix runner)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-17 23:10'
updated_date: '2026-05-18 22:16'
labels:
  - infra
  - tooling
  - M0
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Set up CI (likely GitHub Actions or self-hosted GitLab CI) that runs 'just check', 'just clippy', 'just test', and 'just e2e' inside nix develop. Matrix runs per milestone.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 .github/workflows/ (or .gitlab-ci.yml) runs jobs: check, clippy, test, e2e — all inside 'nix develop' or a Nix-shell wrapper.
- [ ] #2 CI exits non-zero on any failure; merges blocked on green.
- [ ] #3 Matrix runner is parameterised by milestone label; PRs to milestone branches run the relevant tier.
- [ ] #4 Test: a deliberate clippy warning fails CI.
- [ ] #5 Test: an e2e cell failure shows up clearly in the workflow output.
- [ ] #6 Implementation notes record design questions (e.g. self-hosted runner vs GitHub-hosted; cost; cache strategy for Nix and Cargo).
- [ ] #7 Implementation notes record honest limitations (e.g. no tier-3 Renode CI until M10; tier-2 MPI CI lands at M7).
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add `just ci` aggregate recipe: runs check, clippy, test, e2e, determinism-check, determinism-check-negative; exits non-zero on first failure. Single source of truth for CI==local. Comment clippy step referencing TASK-0162.
2. Create .github/workflows/ci.yml: Nix-install action (cachix/install-nix-action), milestone matrix (M0-M3 active tier-1 gate today; M7 MPI / M10 Renode commented placeholder rows allowed-to-skip), each job runs `nix develop -c just ci` (or split per-recipe). Workflow non-zero on any failure.
3. Demonstrate AC#4 locally: introduce deliberate clippy warning, run `nix develop -c just ci`, observe non-zero, revert.
4. Demonstrate AC#5 locally: force an e2e cell failure, run gate, observe clear non-zero readable output, revert.
5. Verify full gate green: nix develop -c just ci.
6. Commit (no push). Record AC#6 design Qs + AC#7 honest limits + AC#4/#5 real-CI-pending honesty in notes. Check only honestly-met ACs.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## CI implementation (TASK-0057)

Added `just ci` aggregate recipe (justfile) + `.github/workflows/ci.yml`. CI delegates the entire gate to `nix develop -c just ci` so "what CI does" == "what runs locally" — one source of truth, cannot drift.

Gate steps (all inside nix develop): check, clippy, test, e2e, determinism-check, determinism-check-negative. just is fail-fast (aborts on first non-zero line), so any failure -> non-zero -> failed job -> failed workflow (AC#2).

### Local verification actually run (not asserted)
- `nix develop -c just ci` clean: EXIT=0; test 0 failed; e2e total 10 pass 8 fail 0 skipped 2 required-fail 0; determinism 8/0 byte-identical; determinism-check-negative correctly bites.
- AC#4 demo: appended a deliberate `clippy::needless_return` fn to driver/src/main.rs; `just ci` exited 101 at the clippy step ("error: unneeded return statement", "recipe clippy failed", "recipe ci failed"); reverted exactly (empty git diff); clippy green again.
- AC#5 demo: corrupted 4 bytes of 01-elementwise-add/reference.bin; `just e2e` exited 1, surfaced "01-elementwise-add naive pthreads-sync FAIL/diff first byte differs at offset 0", summary "fail: 1 ... required-fail: 1", "error: recipe e2e failed"; reverted via git checkout; final `just ci` green again.

### AC#4/#5 HONESTY (real-CI verification PENDING)
This repo has NO git remote and no runner; the workflow cannot be triggered or observed on a real GitHub Actions runner from here. AC#4 and AC#5 are verified at the LEVEL OF GATE LOGIC locally (above): the exact command CI runs (`nix develop -c just ci` / the e2e step) demonstrably exits non-zero with clear output on a deliberate clippy warning and on an e2e cell failure. Real-runner confirmation of the YAML wiring (Nix install action, caches, matrix) is UNVERIFIED and pending a remote/runner. Recorded honestly; AC#4/#5 left checked only for the locally-verified logic with this explicit caveat.

### AC#6 — design questions / decisions
- Runner: GitHub-hosted ubuntu-latest chosen for zero-ops start. Nix toolchain build is the cost driver; mitigated by DeterminateSystems/magic-nix-cache-action + actions/cache for ~/.cargo + nucleus/target keyed on Cargo.lock. Self-hosted runner becomes attractive at M7 (MPI: needs OpenMPI/slurm-localhost) and M10 (Renode: heavy emulator, license/IO) where a persistent Nix store + warm cargo target dir cut minutes/run and avoid per-run egress. Trade-off: self-hosted = maintenance + security surface (untrusted PR code on own hardware) vs GitHub-hosted = clean isolation + minutes cost. Decision deferred to when tier-2/3 land.
- Cache strategy: Nix store cached via magic-nix-cache (GHA cache backend); Cargo cached separately keyed on nucleus/Cargo.lock hash so a dep bump invalidates cleanly. Risk: GHA cache 10GB/repo eviction can cause cold rebuilds; acceptable at M0.
- Matrix (AC#3): keyed by `milestone` (M0..M6 = active tier-1, identical `just ci`); milestone-specific required set is enforced INSIDE the e2e harness via e2e-matrix.toml, not duplicated in YAML (single source of truth). Tier-2 (M7 MPI) and tier-3 (M10 Renode) rows are written as commented `include:` placeholders, deliberately DISABLED (backends/runtimes do not exist; enabling now would fake success — forbidden). fail-fast:false so all milestone cells report.
- clippy step delegates to the `clippy` recipe (workspace, not --all-targets) as defined TODAY; TASK-0162 will tighten that recipe and `just ci` inherits it for free (code comment in justfile references TASK-0162).

### AC#7 — honest limitations
- No git remote / no runner: workflow YAML wiring unverified on a real runner (see AC#4/#5 honesty above).
- Branch protection / "merges blocked on green" (AC#2 second half) CANNOT be configured from the repo — it is a GitHub repo-settings action a maintainer must take (mark the `gate` jobs as required status checks). Workflow exits non-zero on failure (verified locally); the merge-block is org/settings, not code. Filed as the maintainer-action portion of AC#2; recorded as a limitation, not silently claimed.
- No tier-2 MPI CI until M7; no tier-3 Renode CI until M10 (PRD §11). Placeholders disabled, follow-ups TASK-0164 (M7 MPI CI) / TASK-0165 (M10 Renode CI) referenced in ci.yml comments — note: those task ids are placeholders cited in-comment; create when M7/M10 are scheduled.
- Found + filed TASK-0163: e2e harness silently ignores an unknown schedule in a [[required]] entry instead of FAILing — a real CI blind spot (a stale/typo required cell vanishes). Forward-carried.
<!-- SECTION:NOTES:END -->
