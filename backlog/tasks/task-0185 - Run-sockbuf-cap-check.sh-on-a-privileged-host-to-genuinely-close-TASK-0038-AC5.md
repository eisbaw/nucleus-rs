---
id: TASK-0185
title: >-
  Run sockbuf-cap-check.sh on a privileged host to genuinely close TASK-0038
  AC#5
status: To Do
assignee: []
created_date: '2026-05-19 04:12'
updated_date: '2026-05-19 04:13'
labels:
  - environment-blocked
  - mp-tcp
  - verification
dependencies:
  - TASK-0174
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Environment-blocked verification node (mirrors the TASK-0166 maintainer-action precedent). The mp-tcp-bufsync SO_*BUF fail-loud decision logic is proven deterministically by the pure check_effective_sock_buf unit tests (TASK-0174 cycle), and a genuine end-to-end netns harness (nuc-nucleus/sockbuf-cap-check.sh + just sockbuf-cap-check) is shipped and ready. It cannot run in the dev sandbox because net.core.wmem_max is init_user_ns-owned, not per-netns, so unshare -Urn + CAP_NET_ADMIN cannot lower it (the harness readback-detects this and honestly SKIPs). This task tracks running that ready harness on a host/CI where net.core.wmem_max is writable (host root, or a privileged container with --sysctl, or userns-sysctl-enabled CI), where it must observe a generated run.sh exit non-zero with the wire::apply_sock_buf clear error naming the OS cap, thereby genuinely closing TASK-0038 AC#5 and unblocking TASK-0038 -> Done -> TASK-0036 -> Done.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Run just sockbuf-cap-check on a host/CI where net.core.wmem_max is writable; it must NOT skip
- [ ] #2 A generated mp-tcp-bufsync run.sh exits non-zero with the wire::apply_sock_buf clear error naming net.core.wmem_max (the regression arm of the harness fires)
- [ ] #3 TASK-0038 AC#5 checked and TASK-0038 set Done; TASK-0174 ACs reconciled
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Created by phase3-ralph review gate (mped-architect Finding 5) as the standalone environment-blocked verification node for TASK-0038 AC#5 — mirrors the TASK-0166 maintainer-action precedent (env-blocked work must be a first-class addressable node, not prose buried in an In-Progress task). The harness and pure-logic proof already exist (TASK-0174); this task is purely the privileged-host execution. Depends on TASK-0174 (harness deliverable). Closing this checks TASK-0038 AC#5 -> TASK-0038 Done -> TASK-0036 Done.
<!-- SECTION:NOTES:END -->
