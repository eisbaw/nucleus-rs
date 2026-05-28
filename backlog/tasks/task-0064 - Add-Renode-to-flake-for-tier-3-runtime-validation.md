---
id: TASK-0064
title: Add Renode to flake for tier-3 runtime validation
status: Done
assignee:
  - '@mped'
created_date: '2026-05-17 23:24'
updated_date: '2026-05-28 21:04'
labels:
  - M10
  - infra
  - tooling
  - embedded
  - renode
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-3 (M10+) per PRD §10.3 uses Renode as the default runtime validation harness. nixpkgs has a 'renode' package (Mono-based). Add it under a separate devShell or behind a feature flag so the heavy Mono runtime is not pulled in for tier-1 development. CI job to spin up .resc scripts and diff UART output against reference.bin is a separate task.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 renode binary available in a dev shell
- [x] #2 renode --version works
- [x] #3 An example .resc script in the repo loads, runs to completion, and the harness can capture UART output
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1) Refactor flake.nix to share a basePackages list (DRY single source of truth for the toolchain).
2) Add a second devShell named 'renode' = basePackages ++ [ pkgs.renode ].
3) Keep shellHook='' on both shells (PRD §12.1, user CLAUDE.md).
4) Verify default shell unchanged via 'just ci'.
5) Verify AC#1 'which renode' returns /nix/store path.
6) Verify AC#2 'renode --version' returns version string with exit 0.
7) AC#3 (.resc smoke + UART capture) — likely scope-split: needs MCU target choice, batch-mode CLI investigation, reference output. Will scope-split as TASK-0064.01 with precise gaps documented.
8) Run 'nix flake check' and 'nix flake show' for hygiene.
9) Commit scoped, conventional, no AI credit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 2026-05-21 implementer notes
================================

DELIVERED (AC#1, AC#2):
- flake.nix: refactored to a DRY basePackages list and added an opt-in 'renode' devShell.
- nix develop .#renode -c which renode    -> /nix/store/jsm1rq8d5mjw80m8v5h0wa12r1vgyddz-renode-1.16.1/bin/renode
- nix develop .#renode -c renode --version -> 'Renode v1.16.1.0 (.NET 9.0.15)', exit 0. Headless, no GUI needed.

DEFERRED (AC#3): filed as TASK-0223. See that ticket for the precise design issues (MCU target, firmware artefact, UART capture, batch-mode harness, golden output, just recipe).

VERIFICATION (default shell unregressed):
- just test  -> 539 passed / 0 failed / 2 ignored
- just e2e   -> 36 / pass 29 / fail 0 / skipped 7 / required-fail 0
- just ci    -> exit 0
- nix flake check -> both shells evaluate clean
- nix flake show  -> devShells.{default,renode} visible across all four supported systems

DESIGN NOTES:
- DRY: basePackages = [ rustToolchain, rust-analyzer, just ] is the single source of truth. The renode shell appends pkgs.renode and inherits the MSRV pin verbatim.
- shellHook='' on both shells (PRD §12.1, user CLAUDE.md, no verbose echos).
- nixpkgs renode pin: 1.16.1 (Mono/.NET 9.0.15 closure). meta.version reported '1.16.0' but the realised store path is renode-1.16.1; this is a nixpkgs metadata cosmetic inconsistency, not a functional issue.
- Closure cost honest accounting: the .#renode shell first-fetch pulled in mono-6.14.1, dotnet-runtime-9.0.15 + sdk + wrapped variants, libgdiplus, tk, perl strip-nondeterminism, glibc-locales, etc. — exactly the bloat the opt-in shape was designed to avoid pulling into the default shell.

HONEST LIMITATIONS:
- Default-shell unit test count was 539/0/2 (matches onboarding). e2e was 36/29/0/7 (matches onboarding). The 'just ci' run also exercises the negative-inversion gate which prints '36 pass:0 fail:29' as the INTENDED corruption-sanity scan; final 'just ci' exit code is 0, no regression.
- Renode was tested ONLY at --version and 'which'. We did NOT run a real .resc nor verify Mono can JIT under this sandbox. If TASK-0223 hits Mono sandboxing issues that's the next ticket's problem.
- aarch64-linux / *-darwin renode shells were NOT realised, only evaluated. nixpkgs claims renode is available on those systems but actual JIT on Apple Silicon may differ. Out of scope for tier-3 on x86_64-linux CI.

FORWARD-CARRY:
- TASK-0048 (M10 STM32H7 Renode shim) prereq 'is Renode in the flake' is now satisfied.
- TASK-0068 (plan tiered devShells before M7) partially advanced — the basePackages DRY shape is the template for future tier-2/tier-3 shells.
- TASK-0223 (this cycle's scope-split) is the precise follow-up for AC#3.

COMMIT: 632d98c

PARTIAL completion 2026-05-21
================================

Status: stays In Progress (NOT Done) because AC#3 is deferred to TASK-0223.

AC status:
  [x] #1 renode binary in dev shell  -> DONE (commit 632d98c)
  [x] #2 renode --version works      -> DONE (Renode v1.16.1.0, exit 0)
  [ ] #3 .resc + UART capture        -> DEFERRED -> TASK-0223

Gate at hand-off:
  just test 539/0/2; just e2e 36/29/0/7 required-fail:0; just ci exit 0; nix flake check clean.

Next-cycle plan: pick up TASK-0223 OR allow TASK-0048 (M10 STM32H7 shim) to consume just AC#1+AC#2 as the unblock signal it needed.

AC#3 reconciled to MET (2026-05-28). Verified empirically this session: `nix develop --command just renode-uart-smoke` cross-compiled the thumbv7em-none-eabihf no_std firmware (.#embedded), booted it headless in Renode on the bundled stm32h743 (.#renode), and the harness captured USART1 -> 'NUCLEUS-M10-OK' sentinel, exit 0. The AC#3 design pass was split to TASK-0223 (Done) and consumed by the M10 arc (TASK-0048.01/02/03); the `just renode-uart-smoke` (hand-written template) and `just renode-embedded EX` (generated-project -> reference.bin diff for ex1/5/9) recipes are the landed realisation. .resc loads firmware, runs to completion, quits cleanly, UART captured to file deterministically. TASK-0064 moved Done; all 3 ACs met.
<!-- SECTION:NOTES:END -->
