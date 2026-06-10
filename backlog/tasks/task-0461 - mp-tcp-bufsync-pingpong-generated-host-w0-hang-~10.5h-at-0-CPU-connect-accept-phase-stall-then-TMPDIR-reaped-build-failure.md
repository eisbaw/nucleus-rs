---
id: TASK-0461
title: >-
  mp-tcp-bufsync pingpong: generated host/w0 hang ~10.5h at 0% CPU
  (connect/accept-phase stall), then TMPDIR-reaped build failure
status: To Do
assignee: []
created_date: '2026-06-10 08:37'
labels:
  - test-flake
  - mp-tcp-bufsync
  - gate
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Observed 2026-06-10 during a baseline gate run (just test inside nix develop, background, started 00:06).

EVIDENCE (captured live at ~10:34 before self-resolution):
- pingpong_matches_pthreads_sync_bit_for_bit ran 37713.05s total. Generated processes host (pid 62639), w0 (62640), run.sh (62632), and the test binary (62533) sat at 0.0%% CPU from ~00:07 to 10:35 (ps etime ~10:28:xx for all).
- ss -tnp at 10:34 showed NO established TCP sockets for host or w0 — consistent with a connect/accept-phase stall (a LISTEN socket would need ss -l to show; w0 sleeping in a connect-retry backoff would also show nothing).
- At 10:35 the pair unwedged (cause unknown; coincident with a read-only ss/proc probe), wrote the 4-byte output.bin, and the test CONTINUED — then FAILED at pingpong.rs:237 building the pthreads comparison project: rustc could not create a temp dir under /tmp/nix-shell.FWTu6g (os error 2) — the 10-hour-old nix shell TMPDIR had been reaped (likely tmpfiles aging) during the hang. just test exited 101; test-release and e2e never ran.
- Shared scratch parent nucleus/target/mp-tcp-bufsync-pingpong-scratch/ has accumulated ~812 subdirectories (link count 814) — per-run dirs are never reclaimed; separate disk-rot concern, same family as the e2e scratch-lifecycle memory.

THREE sub-problems, fix the first as the core:
(1) HANG: generated mp-tcp host/w0 can wedge indefinitely in the connect/accept phase. Likely family: connect-vs-listen startup race / port collision under parallel cargo test (port-stress thread TASK-0176/TASK-0252). Root-cause and fix in the generated run/connect path (bounded connect retry with deadline + loud failure), NOT by test-side masking.
(2) NO TIMEOUT: cargo test has no per-test timeout, so one wedged generated pair stalls the whole gate for a night. Add a harness-level watchdog to generated-program integration tests (kill + fail with ps/socket capture). Sibling of the diff-fuzz per-command timeout already filed as TASK-0453.01.01.
(3) SCRATCH ROT: reclaim per-run pingpong scratch dirs (~812 accumulated).

Related: TASK-0426 (same backend, different mode: scratch-dir NotFound race; its AC#2 10x-green validation should be re-run after THIS fix lands). Orchestrator note: the gate invocation that hid the failure used "... | tail -25" — pipeline exit code masking; never pipe the gate through tail (the renode memory documents the same footgun).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Root cause of the connect/accept-phase hang identified with a witness (not hypothesis) and fixed in the generated connect/listen path — bounded retry with deadline, loud failure
- [ ] #2 Generated-program integration tests carry a watchdog: a wedged pair fails the test within minutes with diagnostic capture, never stalls the gate for hours
- [ ] #3 Pingpong scratch dirs reclaimed on success; accumulated ~812 stale dirs cleaned
- [ ] #4 Full just test green 10 consecutive runs after the fix (subsumes TASK-0426 AC#2 evidence)
<!-- AC:END -->
