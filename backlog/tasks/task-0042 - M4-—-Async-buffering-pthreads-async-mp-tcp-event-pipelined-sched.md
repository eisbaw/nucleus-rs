---
id: TASK-0042
title: 'M4 — Async + buffering (pthreads-async, mp-tcp-event, pipelined sched)'
status: In Progress
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-17 23:08'
updated_date: '2026-05-23 23:52'
labels:
  - M4
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-1 milestone: add async + buffered backends and the pipelined schedule pattern. Examples 9 and 11 land. PRD §11. This task is a placeholder; refine into sub-tasks before starting M4.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 pthreads-async backend lands (std::thread + condvar + ring buffer).
- [x] #2 mp-tcp-event backend lands (mio for epoll-based readiness).
- [ ] #3 pipelined.sched.nuc pattern works for examples 9 and 11.
- [x] #4 buffer=N is validated against Petri net place capacity end-to-end.
- [ ] #5 Test: M4 differential matrix is green.
- [x] #6 Implementation notes record design questions discovered during async-codegen work.
- [x] #7 Implementation notes record honest limitations (e.g. mio's polling overhead; whether to also offer tokio variant).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR M4-CAPSTONE ASSESSMENT (phase3-ralph cycle 79b, 2026-05-24; supersedes the To Do framing).

All M4 implementation work the task names is SUBSTANTIVELY ACHIEVED. AC status:

- AC#1 ✓ MET. pthreads-async backend lands (TASK-0042.01 cycle 17 single-worker; TASK-0228 Wave B-2 cycle 26 multi-worker with file-scope Ring<T> + per-pair Arc<Ring<T>> + per-worker thread::spawn). Bit-identical against reference.bin on 13/pipeline_parallel (sha d893337208d7b469...) — the M4 HEADLINE target satisfying async + buffer=3 + notify=event.

- AC#2 ✓ MET. mp-tcp-event backend lands. Stages 1+2 cycle 41 (single-worker delegation); Stage 3 cycle 79 + review-hardening cycle 79b (multi-worker mio reactor + per-(seq, peer) outbound queue + per-seq inbound queue + typed Chan<T>; rendezvous-file handshake via TASK-0176 pattern; mio = 0.8 as the one allowed crate per PRD §12). Three [[required]] cells bit-identical: 02-split/split, 11/pipelined (sha f2c2069c...), 13/batch_parallel (sha d8933372...).

- AC#3 PARTIAL. 11/pipelined.sched.nuc works on pthreads-async + mp-tcp-event (bit-identical sha f2c2069c... — independent cross-backend confirmation on the same oracle). 09/pipelined needs worker-to-worker mesh (TASK-0175 closed-deferred-until-TASK-0117 driver); current ContractGap fail-loud — NOT silent miscompile. Same residue applies to mp-tcp-bufsync 09/pipelined.

- AC#4 ✓ MET. buffer=N is validated end-to-end: NameSidecar.transfer_buffer_for_seq (TASK-0233) carries policy.buffer through to backend codegen; Petri net boundedness pass (TASK-0213) enforces place capacity; pthreads-async emits Ring<T> capacity-N; mp-tcp-event emits BoundedFrameRing capacity-N. Verified bit-identical on 13/pipeline_parallel × pthreads-async (buffer=3) and 11/pipelined × {pthreads-async, mp-tcp-event} (buffer=2).

- AC#5 SUBSTANTIVELY MET. M4 differential matrix: e2e 88/73/0/15/0 required-fail. The 15 SKIPs are: 8 capability mismatches (TASK-0210 schedules whose surface is satisfied by only one tier-1 column — these are STRUCTURALLY correct skips, not gaps), 4 mp-tcp-event w↔w (TASK-0175 / TASK-0117 — same blocker mp-tcp-bufsync has), 1 mp-tcp-event 05/distributed (TASK-0181 closed but capability mismatch still gates), 2 other distributed cells. No required-fail. Cross-backend non-flaky (xbackend-check-negative bites on 16-cell mp-tcp corruption; determinism-check-negative bites on 73-cell perturbation).

- AC#6 ✓ MET. Implementation notes scattered across TASK-0042.01 + TASK-0042.02 + TASK-0042.05 + TASK-0228 final summaries record: Ring<T> vs Slot<T> decision; mp-tcp-event Chan<T> vs ring naming; opportunistic drain_writes after enqueue (provably non-lossy via queue-is-source-of-truth); Rc<RefCell<TcpStream>> for CTRL shimming barrier sites; demux-by-seq vs demux-by-peer (relies on globally-unique SeqTag per TASK-0233); SeqTag for stable barrier identity (TASK-0172).

- AC#7 ✓ MET. Honest limitations recorded:
  - mio polling adds ~µs reactor-trip overhead per Push/Wait above raw TCP send/recv (lib.rs preamble); mp-tcp-bufsync's blocking-sync has lower latency on contended cases but cannot satisfy async/buffered.
  - tokio variant deliberately rejected — PRD §12 'one well-known crate' allowance is mio, not tokio (single source of multiplexer overhead).
  - 30s deadlock watchdog now NUC_REACTOR_DEADLOCK_TIMEOUT_S-configurable (cycle 79b F2 fix); kernels that legitimately exceed 30s without intermediate Push trip the watchdog by default — operator extends env knob.
  - w↔w mesh (TASK-0175) and distributed-placement-on-async (TASK-0117) remain ContractGap-deferred, fail-loud.
  - mp-tcp-event runtime_src.rs host-compile coverage landed cycle 79b (include_str! files MUST also be #[cfg(test)] mod X; — generalised as [[feedback-include-str-compile-coverage]] memory).

Status decision: TASK-0042 stays In Progress on AC#3 / AC#5's worker-to-worker residue (TASK-0175 / TASK-0117). Same closure-deferred pattern as M3 capstone TASK-0041 (which stays In Progress on the live-CI-runner residue). Closing now would AC-game the capstone — leaving In Progress with this precise note is the discipline.

Substantive conclusion: the user-goal part 'implement milestone 4 (async + buffering)' is SUBSTANTIATED — three of the four async/buffered acceptance cells (11 + 13/batch_parallel + 13/pipeline_parallel) are bit-identical on multi-tier-1-column cross-backend differential. The only residue is environmental + topology-blocked, not capability or correctness.
<!-- SECTION:NOTES:END -->
