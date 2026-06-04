---
id: TASK-0442
title: Observe ci.yml on a real runner once (act locally or throwaway private fork)
status: Done
assignee:
  - '@me'
created_date: '2026-06-04 08:16'
updated_date: '2026-06-04 09:25'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per WIP review (2026-06-04). git remote -v is EMPTY. .github/workflows/ci.yml lines ~33-46 honestly disclose CI has never run on a real runner. Three jobs (gate / milestone[M1,M2,M3] / port-stress / renode-multimcu) are DECLARED but unobserved. The renode-multimcu job alone needs ~1-2GB Nix closure cold-start which may exceed hosted-runner walltime.

Scope: Either (a) use 'act' locally to run ci.yml's gate job, OR (b) push a temporary fork to a throwaway private GitHub repo, observe one CI run, and report findings. If walltime fails or YAML is syntactically broken, capture the actual error and file fix tasks. ~1 cycle.

Why: For thesis defence local-reproducibility is enough; but if 'production' = green CI badge a reviewer can click, you have ZERO evidence the YAML is even syntactically valid. This is a low-cost truthfulness check.

Estimated effort: LOW priority, single cycle. May surface YAML defects requiring follow-up.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 actionlint static check run (nix, no docker): exit code recorded; this is the PRIMARY evidence the YAML is valid + semantically sound (expressions/matrix/if-guards/action-refs/shellcheck-on-run-blocks). Any finding -> fix task.
- [x] #2 act -l (nix run): YAML parses; all 4 jobs enumerated (gate/milestone/port-stress/renode-multimcu) with correct trigger events; recorded.
- [x] #3 act --dryrun push (nix run, -P to skip prompt, NO container executed): full job graph resolves — milestone matrix EXPANDS to M1/M2/M3, port-stress correctly EXCLUDED on push (if-guard), gate+renode-multimcu INCLUDED, action refs resolve, dryrun exit 0; recorded.
- [x] #4 each job run: payload recipe (ci, e2e-milestone, port-stress-check, renode-multimcu-gate) confirmed to EXIST in justfile with matching arg-shape (e2e-milestone M:); each is the SAME nix-local command already run green this project (cross-ref baselines e2e 427/364/0/63/0 + renode byte-exact).
- [x] #5 written observation report in TASK-0442 tracker notes+final-summary (NOT a committed .md); honest attribution; explicitly records that a docker/act REAL job-run was deliberately NOT pursued — unfaithful (act/podman fails install-nix for harness reasons, telling us nothing about ci.yml) AND unnecessary (nix+actionlint+dryrun is the right tool). Per user steer 'why docker? nix is surely enough'.
- [x] #6 residual gap documented honestly: the GH-action runtime glue (checkout/install-nix-action/magic-nix-cache/actions-cache) + hosted-runner walltime on the ~1-2GB renode closure remain UNOBSERVED; only a real GitHub runner (throwaway fork) faithfully observes them — act-under-podman cannot. No genuine ci.yml defect found (stated with evidence), OR fix task filed.
- [x] #7 the 3 untracked task drafts (task-0440/0441/0442) committed to the tracker in this pass.
- [x] #8 repo state honest: no source change; working tree clean after the cycle; only intended commits.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
PRE-BRIEF (orchestrator 2026-06-04): environment + ground truth established.
- git remote EMPTY (confirms task premise: ci.yml has zero observed runs).
- act NOT on PATH but available: `nix run nixpkgs#act` = 0.2.88. docker CLI present but act is driving the PODMAN socket (unix:///run/podman/podman.sock) here — rootless podman; privilege limits may worsen the Nix-in-container boundary.
- actionlint available: `nix run nixpkgs#actionlint` = 1.7.12.
GROUND TRUTH (orchestrator already ran):
- `act -l` -> parses OK, lists 4 jobs: gate (tier-1 gate), milestone (matrix M1/M2/M3, name shows literal ${{ matrix.milestone }}), port-stress, renode-multimcu. All show events push,pull_request,workflow_dispatch,schedule.
- `actionlint -color` -> EXIT 0, zero findings (strong: expressions/matrix/if-guards/action-refs/shellcheck all clean).
- `act push --dryrun` -> prompts interactively for default image (Large/Medium/Micro) then dies EOF in non-interactive shell. FIX: pass `-P ubuntu-latest=catthehacker/ubuntu:act-latest` to skip the prompt.
KEY INTERPRETIVE FRAME for the report: ci.yml jobs all do install-nix-action@v27 + magic-nix-cache@v8 + `nix develop -c just <step>`. Under act/podman a real job run will MOST LIKELY fail at 'Install Nix' (Nix install needs privileges act's container lacks) or at magic-nix-cache (needs the GitHub Actions cache service act does not provide). That failure is an ACT-HARNESS LIMITATION, not a ci.yml defect — must be stated as such. The DELIVERABLE is the honest report, NOT green CI. actionlint exit 0 is the real positive evidence the YAML is valid.
Report goes in TRACKER notes/final-summary, NOT a committed .md (repo policy: no summary files).

OBSERVATION REPORT (orchestrator-run nix-only, 2026-06-04). Approach revised per user steer 'why docker? nix is surely enough' — docker/act REAL job-run deliberately NOT pursued (see AC#5 rationale). All tooling via `nix run nixpkgs#<tool>`; no docker image pulled or container executed.

INVOCATIONS + RESULTS:
1. `nix run nixpkgs#actionlint` -> EXIT 0, zero findings. PRIMARY EVIDENCE the workflow is valid+sound: actionlint statically validates ${{ }} expressions, the matrix block, every job-level if: guard, action @ref versions, and runs shellcheck on every run: block. Clean.
2. `nix run nixpkgs#act -- -l` -> parses OK; enumerates all 4 jobs: gate (tier-1 gate), milestone (e2e tier ${{ matrix.milestone }}), port-stress, renode-multimcu; each Events=push,pull_request,workflow_dispatch,schedule. (act driving rootless podman socket here.)
3. `nix run nixpkgs#act -- push --dryrun -P ubuntu-latest=catthehacker/ubuntu:act-latest` -> EXIT 0, NO container executed (plan only). Job graph resolves correctly AND exercises RUNTIME behavior static lint cannot: milestone matrix EXPANDS to 3 jobs (e2e tier M1-1 / M2-2 / M3-3); port-stress correctly ABSENT from the push plan (its if: scopes to schedule||workflow_dispatch); gate + renode-multimcu PRESENT on push (renode if: includes push); action clones resolve (install-nix-action@v27, magic-nix-cache@v8); 'Set up job' + 'Pre Install Nix' succeed in-plan.
4. Job payload recipes all EXIST in justfile (so no run: step can fail with 'no recipe'): ci, e2e-milestone (signature `e2e-milestone M:` matches matrix arg), port-stress-check, renode-multimcu-gate. Each is the SAME nix-local command already verified green this project: just-ci components + e2e 427/364/0/63/0 (TASK-0439 qa re-run) + renode-multimcu-gate byte-exact 02-split-add 1024B + 14-hearing-aid 512B.

VERDICT: NO ci.yml defect found. Evidence = actionlint exit 0 + act -l clean parse + act --dryrun exit 0 with correct matrix-expansion & if-guard filtering + all payload recipes present & locally green. The YAML is syntactically valid, semantically sound, and its job graph (triggers/matrix/conditionals) resolves as designed.

RESIDUAL GAP (honest, unchanged from ci.yml's own lines 42-44 disclosure): the GH-ACTION RUNTIME GLUE — actions/checkout@v4, cachix/install-nix-action@v27 actually installing Nix on a hosted runner, DeterminateSystems/magic-nix-cache-action@v8 against GitHub's cache service, actions/cache@v4 — and the hosted-runner WALLTIME for the ~1-2GB renode/embedded Nix closure cold-start remain UNOBSERVED. Neither nix-local nor act-under-rootless-podman faithfully reproduces these (act would fail install-nix for privilege/harness reasons that say nothing about ci.yml). The ONLY faithful observation is one real GitHub run (throwaway private fork) — left as the documented honest residual, optionally TASK-able if a clickable green badge becomes a requirement. NOT filed as a defect because it is an observation-coverage gap, not a fault.

REVIEW GATE (orchestrator-run, parallel read-only). qa-test-runner: GO — independently reproduced EVERY empirical claim: actionlint 'Found total 0 errors' exit 0; act -l lists the 4 jobs; act --dryrun push exit 0, *DRYRUN* lines only (no real container pull/run), port-stress 0 mentions on push, milestone expands to M1/M2/M3; recipes at justfile:183(ci)/69(e2e-milestone M:)/231(port-stress-check)/1718(renode-multimcu-gate); commit b63c686 touches only backlog/ md. mped-architect: central verdict defensible + honestly scoped, approach-correction technically sound (NOT a dodge), AC-revision = honest re-scoping NOT gaming. Findings FOLDED:
- P2a (wording retraction): the invocation-3 phrase “'Set up job' + 'Pre Install Nix' succeed in-plan” OVERCLAIMS — act --dryrun ENUMERATES those step lines while resolving the plan; it does NOT execute them, nothing 'succeeded'. CORRECTED reading: 'Set up job + Pre Install Nix are enumerated in the resolved plan, NOT executed'. This is exactly the install-nix glue that the RESIDUAL GAP paragraph (correctly) lists as UNOBSERVED.
- P3 (soften): 'exercises RUNTIME behavior static lint cannot' overstates dryrun — precise claim is 'resolves matrix-expansion + if-guard filtering at PLAN time (no step executed)'. The defect-absence verdict rests on actionlint exit 0 + plan-resolution + recipe existence, NOT on any executed step.
- P2b: residual real-runner observation promoted from notes-only to a real node TASK-0443 (gated on 'green-badge requirement'), so it cannot rot in a Done task (feedback-opacity-gate-rot).
- P1 (AC#8 precision): AC#8 'working tree clean after the cycle' is attested with this PRECISE meaning: no source/workflow change this cycle + only intended commits + the SOLE untracked paths are the pre-existing, unrelated cruft/ spillover (s3_input.bin / s3_reference.bin / spike_s3_ref.py, dated 3-jun, NOT introduced by this cycle). After committing this addendum the working tree carries no uncommitted edits; only that pre-existing cruft remains untracked (the standing repo baseline every cycle). Under that reading AC#8 holds; the bare word 'clean' was imprecise, now pinned.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Observed ci.yml statically + structurally via nix-only tooling (no docker). actionlint 1.7.12 -> exit 0 (zero findings): the workflow's expressions, matrix, if-guards, action @refs, and run-block shell are all valid/sound. act 0.2.88 -l parses + lists all 4 jobs; act --dryrun push (no container executed) resolves the job graph correctly, expanding the milestone matrix to M1/M2/M3 and applying if-guards as designed (port-stress excluded on push; gate+renode-multimcu included). All four job payload recipes exist in justfile and are the same nix-local commands already verified green (e2e 427/364/0/63/0, renode byte-exact). VERDICT: no ci.yml defect; YAML is valid and its graph resolves as designed. Approach corrected mid-cycle per user steer ('why docker? nix is surely enough') — a docker/act real job-run was deliberately not pursued: act-under-rootless-podman would fail install-nix for harness reasons that say nothing about ci.yml, so it is both unfaithful and unnecessary. RESIDUAL (honest, matching ci.yml's own lines 42-44): the GH-action runtime glue (checkout/install-nix-action/magic-nix-cache/actions-cache) and hosted-runner walltime on the ~1-2GB renode closure remain UNOBSERVED; only a real GitHub run (throwaway fork) faithfully covers them — an observation-coverage gap, not a fault. The deliverable is this report, not a green badge.
<!-- SECTION:FINAL_SUMMARY:END -->
