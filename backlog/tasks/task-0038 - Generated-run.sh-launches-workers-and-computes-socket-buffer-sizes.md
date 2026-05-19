---
id: TASK-0038
title: Generated run.sh launches workers and computes socket buffer sizes
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 00:53'
labels:
  - M3
  - backend
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
For multi-process backends, each Nucleus build emits a run.sh that: launches one worker process per WorkerId, sets SO_SNDBUF/SO_RCVBUF via env or sysctl, waits for completion, returns non-zero on any worker failure. PRD §8.6, §12 risks.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 run.sh emitted alongside the per-worker binaries when backend is mp-tcp-*.
- [x] #2 run.sh sets SO_SNDBUF / SO_RCVBUF (via env passed to each binary, which calls setsockopt) sized from the Petri net's per-channel buffer requirements.
- [x] #3 run.sh exits non-zero if any worker fails or times out; reports which worker failed.
- [x] #4 Test: run.sh launches a multi-worker example and reports correct exit status.
- [ ] #5 Test: an OS that caps SO_SNDBUF below required (forced via lowering net.core.wmem_max in a container) produces a clear error.
- [x] #6 Implementation notes record design questions (e.g. should buffer sizing happen in run.sh or be baked into binaries; v2 picks via env).
- [x] #7 Implementation notes record honest limitations (no per-channel granularity if OS-level cap binds; v2 uses the single highest requirement).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as part of TASK-0036. The mp-tcp-bufsync backend emits run.sh alongside the per-worker binaries (AC#1): launches one OS process per WorkerId (host first, then each non-host worker), waits on every PID, exits non-zero NAMING the failed worker if any worker fails or exits non-zero (AC#3, verified: e2e runs run.sh and the harness checks exit status; 02-split/split passes 5/5).

AC#2: run.sh computes NUC_SO_BUF from the schedule per-channel buffer requirement (largest single cross-worker payload in bytes, sized from the NameSidecar ResolvedType; sync=1 message; 64KiB floor) and exports it; each worker binary calls wire::apply_sock_buf() which setsockopt(SO_SNDBUF/SO_RCVBUF) — i.e. buffer sizing is baked into binaries that read the env (AC#6 design question: env-passed-to-binary chosen over sysctl-in-run.sh, so an unprivileged run still works).

AC#3 port allocation: a std-only Rust helper binary __nuc_pick_port (emitted in src/bin/, built by the SAME cargo build) binds 127.0.0.1:0 and prints the kernel-assigned ephemeral port; run.sh exports NUC_TCP_PORT_<worker>. Deterministic, no fixed ports, no clashes across concurrent matrix cells. IMPORTANT: replaced an initial python3-based picker because python3 is NOT in the nix dev shell (only on the host system) — that would have been non-reproducible (fundamentals/reproducibility violation). The Rust picker works under `nix develop` with nothing extra on PATH.

AC#4: the e2e harness (transport-aware) runs run.sh for tcp backends and verifies exit status + diffs output.bin; the AC#5 pingpong test also drives run.sh end-to-end. Both pass.

AC#6 design questions recorded: (a) buffer sizing baked into binaries via env vs run.sh sysctl — chose env-passed-to-binary (no privilege needed, deterministic); (b) SO_*BUF set via a dependency-free extern "C" setsockopt (libc always linked) rather than pulling socket2/libc crates (reproducibility/minimal-deps); (c) handshake = two connections per (host,worker) pair (data + ctrl), role assigned by host accept() order, no handshake bytes; worker connects with bounded retry (liveness wait, not data sync). SO_REUSEADDR deliberately NOT set (a stale-port bind clash should fail LOUD, not silently rebind a wrong socket). Bind errors: host panics naming the port (fail-loud).

AC#7 honest limitations recorded: no per-channel buffer granularity — v2 uses the single highest requirement across all channels; if an OS-level cap (net.core.wmem_max/rmem_max) binds below the requirement, wire::apply_sock_buf reads the value back and FAILS LOUD with the exact numbers (does not proceed under-sized).

AC#5 (forced-low net.core.wmem_max in a container produces a clear error): the fail-loud read-back-and-panic code path IS implemented (wire::apply_sock_buf: Linux internally doubles SO_*BUF; we require effective got/2 >= requested else panic naming the cap). BUT I did not execute it inside an actual container with a lowered sysctl, so I am NOT checking AC#5 honestly — the code is present and unit-reasoned but the container scenario is unverified. Filed as a follow-up.

AC status: #1,#2,#3,#4,#6,#7 met and verified (run.sh emitted + launches per-worker processes + SO_*BUF env-sized + non-zero-naming-failed-worker + e2e/pingpong drive it; design Qs + limitations recorded above). AC#5 NOT checked — the lowered-net.core.wmem_max container scenario is implemented (fail-loud read-back) but unverified end-to-end; deferred to TASK-0174 (dependency edge on TASK-0036/0038). Task stays In Progress until TASK-0174 verifies AC#5.
<!-- SECTION:NOTES:END -->
