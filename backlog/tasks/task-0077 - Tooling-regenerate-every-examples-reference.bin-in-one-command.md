---
id: TASK-0077
title: 'Tooling: regenerate every example''s reference.bin in one command'
status: Done
assignee: []
created_date: '2026-05-17 23:40'
updated_date: '2026-05-23 20:54'
labels:
  - tooling
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add a single entry point that re-runs each examples/NN-name/reference/ regeneration command in sequence and writes the resulting reference.bin files in place. Two shapes considered: (a) a 'just regen-references' recipe that shells out per example; (b) a '--regen-refs' flag on the nucleus-e2e binary that walks examples/ and dispatches. Prefer (b) once the e2e harness is non-trivial; (a) is acceptable as a stub at M0–M1. Used by maintainers when a kernel body changes and references must move in lockstep (docs/reference-impl-policy.md §3, §4). Must fail loudly if any reference command exits non-zero, and must not commit results — that is a human review step.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Recipe glob-discovers reference impls (no per-example bloat — PRD §12.3)
- [ ] #2 Fails LOUD on first non-zero exit (set -e)
- [ ] #3 Does NOT commit results (TASK-0077: human review step)
- [ ] #4 Verified end-to-end: runs all 10 examples (01-13 minus 08/10/12/14); zero reference.bin byte diffs on idempotent run
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed orchestrator-direct (cycle 77 continuation). Implemented task option (a): added 'just regen-references' recipe to justfile that glob-discovers nuc-nucleus/examples/*/reference/Cargo.toml + runs each --in/--out regen in sequence. 14-hearing-aid (M11; TASK-0054) skipped naturally by glob (no reference impl yet). Examples 04/06 also support --gen-input but the recipe deliberately scopes to reference.bin regen using the COMMITTED input.bin — input.bin regen is per-example manual flow (changing fixture shape is a separate decision). Set -e + per-step echo for loud-fail + traceability. Anti-bloat rule (PRD §12.3) respected: ONE recipe, not one-per-example, with the per-example data driven by filesystem discovery. End-to-end tested: ran the recipe over all 10 existing examples (01, 02, 03, 04, 05, 06, 07, 09, 11, 13), exit 0, zero reference.bin byte diffs (idempotent — committed bins match what the reference impls produce today). Task option (b) -- the --regen-refs flag on nucleus-e2e -- was the deferred alternative; recipe stub is sufficient for current maintenance needs and lower-risk (no nucleus-e2e API change required).
<!-- SECTION:FINAL_SUMMARY:END -->
