---
id: TASK-0174
title: >-
  Verify run.sh SO_*BUF clear-error under a lowered net.core.wmem_max container
  (TASK-0038 AC#5)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-19 00:52'
updated_date: '2026-05-19 04:03'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. PROBE unshare -Urn + in-netns sysctl net.core.wmem_max (DONE: userns+netns works, but wmem_max write = EPERM; net.core.wmem_max is init_user_ns-owned, NOT per-netns namespaced; needs real root / privileged container).
2. (A) Factor decision logic out of wire_runtime.rs apply_sock_buf into a pure side-effect-free fn check_effective_sock_buf(want, effective_got) -> Result<(), String> returning the EXACT clear-error string naming net.core.wmem_max/rmem_max. Refactor apply_sock_buf to call it (behaviour byte-identical: still panics on Err same message). Add unit tests: enough=>Ok; clamped=>Err naming the OS cap; boundary got/2 doubling.
3. (B) Add justfile recipe sockbuf-cap-check + #[ignore] integration test: attempt unshare -Urn + in-netns sysctl; if it works run a real mp-tcp-bufsync run.sh with NUC_SO_BUF>cap and assert the clear error; if blocked (this sandbox) SKIP with precise reason (init_user_ns-owned wmem_max; needs host/CI with privileged sysctl). Honest-skip, not faked.
4. Gate: just test, just e2e UNCHANGED (30/26/0/4/0), determinism + negatives bite, clippy clean, just ci.
5. (B) environment-blocked => do NOT check TASK-0038 AC#5 nor set 0038 Done; forward-carry recipe-ready + env needed. TASK-0174: (A) done; (B) honest-skip.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
PROBE RESULT (verbatim): \`unshare -Urn\` WORKS (uid maps to root-in-userns, fresh netns, \`ip link show lo\` succeeds). BUT lowering net.core.wmem_max is BLOCKED: \`sysctl -w net.core.wmem_max=4096\` prints "Operation not permitted" yet EXITS 0 (sysctl masks the EPERM in its exit code — a real gotcha that initially produced a false-positive in the harness probe). Readback proof: /proc/sys/net/core/wmem_max stays 4194304 (the default) after the in-netns write attempt. ROOT CAUSE: net.core.wmem_max is owned by the initial netns user namespace (init_user_ns), NOT per-netns namespaced; the kernel checks the capability against init_user_ns for this global core sysctl, so CAP_NET_ADMIN over a fresh netns is insufficient. Needs real host root / privileged container (docker run --sysctl net.core.wmem_max=4096) / userns-sysctl-enabled CI. NOT a seccomp block.

(A) DELIVERED: factored the fail-loud DECISION out of wire_runtime.rs apply_sock_buf into pure fn check_effective_sock_buf(want, effective_got, opt) -> Result<(), String> returning the EXACT clear-error string naming net.core.wmem_max/rmem_max. apply_sock_buf now calls it (behaviour byte-identical: still panics with the same message). 3 deterministic unit tests added (ok-when-enough; fails-loud-naming-OS-cap-when-clamped; got/2-doubling boundary exact). mp-tcp-common 7->10 tests, all pass.

(B) DELIVERED but ENVIRONMENT-SKIPPED: nuc-nucleus/sockbuf-cap-check.sh + \`just sockbuf-cap-check\` recipe. Probes via WRITE+READBACK (not sysctl exit code), stages input.bin, runs a real 02-split-add/split mp-tcp-bufsync run.sh in the lowered-cap netns and asserts the clear error. In THIS sandbox it honestly SKIPs (exit 0, informational) with the precise init_user_ns reason. Ready to RUN the genuine reproduction on any host where the sysctl write is permitted.

GATE (nix develop): just test 0 failed (mp-tcp-common 10/10 incl. 3 new); just e2e UNCHANGED total 30/pass 26/fail 0/skipped 4/required-fail 0 (all mp-tcp multi-process cells incl 02-split/split PASS — refactor behaviour-identical); just determinism-check 30/26/0/4 byte-identical; determinism-check-negative + xbackend-check-negative still bite; clippy --workspace -D warnings clean; just ci exit 0.

GOTCHA for future: the determinism FAIL line seen inside \`just ci\` is the EXPECTED determinism-check-negative arm (NUC_NONDET_TEST=1 injects a pid/nanos nonce into pthreads-sync src/main.rs ON PURPOSE); the recipe then correctly reports OK. Not a regression.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0174 honest status: (A) pure-logic fail-loud proof DELIVERED + committed; (B) end-to-end netns reproduction harness DELIVERED and READY but ENVIRONMENT-SKIPPED in this sandbox (cannot lower net.core.wmem_max — init_user_ns-owned, not per-netns).

WHAT CHANGED:
- nucleus/mp-tcp-common/src/wire_runtime.rs: factored the SO_*BUF fail-loud DECISION out of apply_sock_buf into a pure side-effect-free fn check_effective_sock_buf(want, effective_got, opt) -> Result<(), String> returning the EXACT clear-error string naming net.core.wmem_max/rmem_max. apply_sock_buf now calls it; behaviour byte-identical (still panics with the same message; single source of truth for the message text — net improvement).
- nucleus/mp-tcp-common/src/lib.rs: 3 deterministic unit tests pinning the clear-error behaviour incl. the Linux got/2-doubling boundary subtlety.
- nuc-nucleus/sockbuf-cap-check.sh + justfile `sockbuf-cap-check` recipe: the genuine end-to-end netns reproduction (unshare -Urn + in-netns sysctl + real run.sh under the lowered cap), with a readback-verified honest-skip (NOT sysctl-exit-code, which masks EPERM).

PER-AC (TASK-0174):
- AC#1 (reproducible harness lowering net.core.wmem_max): harness + recipe written and reproducible; CANNOT lower the cap in this sandbox (init_user_ns ownership). PARTIAL — ready for a privileged host/CI.
- AC#2 (a generated mp-tcp-bufsync run.sh fails with the wire::apply_sock_buf clear error): the DECISION is proven deterministically by the pure-logic unit tests; the literal end-to-end run.sh-fails-under-real-cap is environment-blocked. PARTIAL.
- AC#3 (TASK-0038 AC#5 checked + TASK-0038 Done): NOT done — would require a genuine (B) run; not faked.

NONE of AC#1/#2/#3 checked: the genuine in-container reproduction did not execute. (A) is a strong durable portable guard for AC#5 intent but does not literally satisfy AC#2/#3. TASK-0174 stays In Progress; the netns recipe is ready and only needs a host where net.core.wmem_max is writable (host root / privileged container --sysctl / userns-sysctl-enabled CI).

FORWARD-CARRY: TASK-0038 AC#5 stays HONESTLY OPEN (environment-blocked, reason recorded — like the TASK-0166 no-runner standing limitation); do NOT check it or set TASK-0038 Done. TASK-0036 (Dependencies: TASK-0038) consequently stays In Progress. Orchestrator decision, not self-checked.

GATE (nix develop): just test 0 failed (mp-tcp-common 10/10); just e2e UNCHANGED 30/26/0/4/required-fail 0; determinism byte-identical; both negatives bite; clippy clean; just ci exit 0.
<!-- SECTION:FINAL_SUMMARY:END -->
