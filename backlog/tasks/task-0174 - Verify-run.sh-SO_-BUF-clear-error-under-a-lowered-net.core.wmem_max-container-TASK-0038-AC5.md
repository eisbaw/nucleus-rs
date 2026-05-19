---
id: TASK-0174
title: >-
  Verify run.sh SO_*BUF clear-error under a lowered net.core.wmem_max container
  (TASK-0038 AC#5)
status: To Do
assignee: []
created_date: '2026-05-19 00:52'
labels: []
dependencies:
  - TASK-0036
  - TASK-0038
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0038 AC#5 requires: an OS that caps SO_SNDBUF below the schedule requirement (forced by lowering net.core.wmem_max in a container) produces a CLEAR error. The fail-loud read-back-and-panic path is implemented in mp-tcp-common wire::apply_sock_buf (Linux doubles SO_*BUF internally; we require effective got/2 >= requested else panic naming the cap). It was NOT executed inside an actual container with a lowered sysctl during TASK-0036/0038, so the end-to-end clear-error behaviour is unverified. This task: run a generated mp-tcp-bufsync project inside a container (or netns) with net.core.wmem_max/rmem_max lowered below a transfer's payload size and assert run.sh fails with the cited clear error message. Depends on TASK-0036/0038.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A reproducible harness (container or net namespace) lowers net.core.wmem_max below a known transfer size
- [ ] #2 A generated mp-tcp-bufsync run.sh fails with the wire::apply_sock_buf clear error naming the OS cap
- [ ] #3 TASK-0038 AC#5 is then checked and TASK-0038 moved to Done
<!-- AC:END -->
