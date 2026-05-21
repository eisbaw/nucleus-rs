---
id: TASK-0223
title: 'AC#3 scope-split for TASK-0064: example .resc script + UART capture harness'
status: To Do
assignee: []
created_date: '2026-05-21 17:07'
labels:
  - M10
  - infra
  - tooling
  - embedded
  - renode
  - scope-split
dependencies:
  - TASK-0064
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Split out from TASK-0064 (which delivered AC#1+AC#2: renode in the .#renode dev shell, --version verified at Renode v1.16.1.0).

AC#3 deferred because it requires a sizeable design pass that does NOT fit the same 30-min cycle:

1. MCU target choice. PRD §10.3 implies STM32H7 (matches TASK-0048 M10 shim). Need to pick the exact Renode-shipped .repl platform file (e.g. platforms/cpus/stm32h753.repl or stm32f4-discovery.repl as a smaller stepping stone) and document why.

2. Firmware artefact. A .resc needs a binary to load. Cross-compiled Rust firmware depends on TASK-0062 (embedded Rust target). Stepping-stone: use a tiny C/asm hex from the Renode samples, OR a precompiled .elf checked in under tests/fixtures/. Decide before writing the .resc.

3. UART capture mechanics. Renode supports 'sysbus.usart1 CreateFileBackend @uart.txt true' for deterministic file capture. Need to confirm which UART instance the chosen .repl exposes and whether buffering/flush on shutdown is reliable.

4. Batch-mode harness. 'renode --disable-xwt --console <script.resc>' is the headless entry point. Need a deterministic quit (e.g. 'startQuit' / 'q' after 'sysbus.uart WaitForLine ...' or a timed quit).

5. Reference output + diffing. Capture must be byte-stable across reruns (no timestamps in UART). Decide between exact-match and a regex/line-prefix oracle.

6. Just recipe + CI hook. Must NOT run in default 'just ci' (default shell has no renode). Likely a 'just e2e-tier3' / 'just renode-smoke' recipe invoked only under '.#renode' shell or a dedicated CI job.

Dependencies:
  - depends on TASK-0064 (this task, prereq DONE)
  - related to TASK-0062 (cross-compile Rust embedded target) — if firmware is Rust, this becomes a hard dep
  - feeds TASK-0048 (M10 STM32H7 Renode shim) which is the real consumer

Acceptance:
  - [ ] #1 Repo contains a .resc script under tests/renode/ (or similar) that loads a firmware artefact, runs to completion, and quits cleanly
  - [ ] #2 UART output is captured to a file deterministically across reruns
  - [ ] #3 A just recipe runs the .resc headlessly under nix develop .#renode and diffs against a checked-in reference
  - [ ] #4 The recipe fails LOUD on UART mismatch (no silent skip)
  - [ ] #5 README / TASK-0048 notes updated to point at the recipe
<!-- SECTION:DESCRIPTION:END -->
