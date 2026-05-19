---
id: TASK-0036
title: 'mp-tcp-bufsync backend crate (second backend, tier 1)'
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 01:02'
labels:
  - M3
  - backend
dependencies:
  - TASK-0038
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Multi-process over TCP loopback, sync blocking, buffered. PRD §7.1. Workers are OS processes; transport is std::net::TcpStream; sync = blocking recv. Forces capability matrix to be real.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 backends/mp-tcp-bufsync/ is a crate with capabilities.toml.
- [x] #2 Emit produces N Rust binaries (one per worker) plus a run.sh that launches them and wires them up over loopback.
- [x] #3 Workers connect via a deterministic handshake; ports either auto-allocated or passed via env.
- [x] #4 Each Push lowers to a length-prefixed write on the appropriate socket; each Wait to a blocking read.
- [x] #5 Test: synthetic two-worker pingpong matches pthreads-sync output bit-for-bit.
- [x] #6 Implementation notes record design questions (e.g. handshake protocol; whether to use SO_REUSEADDR; how to handle Bind errors).
- [x] #7 Implementation notes record honest limitations (no buffer-pool reuse; one allocation per transfer at M3; perf is not a goal).
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Create mp-tcp-common crate: wire-protocol-v0 framing (8B LE len + 8B LE SeqTag + payload), encode/decode for scalar+Vec types, round-trip unit test (TASK-0037 AC#4).
2. docs/wire-protocol-v0.md (TASK-0037 AC#1/#2/#3).
3. Create backends/mp-tcp-bufsync crate: capabilities.toml (tier1, transport tcp-loopback, sync, buffered), emit() with SAME (per_worker,names,sidecar,kernels,out)->EmitResult signature as pthreads-sync. Reuse pthreads-sync pure render helpers via a shared codegen module (avoid drift) — extract shared single-worker/expr/index renderers into a pub crate surface consumed by both, OR depend on pthreads-sync pub(crate) shims made pub.
4. Single-worker path: emit one binary == straight-line (reuse pthreads-sync single-worker emitter output byte-for-byte where possible; it produces nuc-generated). Multi-worker path: emit one src/bin/<worker>.rs per used worker + run.sh; Push=length-prefixed TCP write, Wait=blocking read; deterministic handshake: host listens on TCP port from env NUC_PORT_<pair> (run.sh allocates), workers connect; barrier-over-TCP via same pre-order Sync index identity + uniform-barrier ContractGap (inherit TASK-0172); divisible block rebinding inherited (TASK-0173).
5. run.sh (TASK-0038): launch one process per WorkerId, set SO_*BUF env from Petri-net per-channel buffer reqs, wait, non-zero on any worker failure naming the worker.
6. Driver: register mp-tcp-bufsync backend dispatch (mirror pthreads-sync block, multi-binary EmitResult).
7. e2e harness: detect multi-process backend (run.sh present / capabilities transport) and run run.sh instead of single binary; matrix: add mp-tcp-bufsync + cells.
8. AC#5: synthetic two-worker pingpong test bit-for-bit vs pthreads-sync.
9. Full gate: test/e2e/determinism/clippy inside nix develop, TCP tests >=3x for flakiness.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
[forward-carried from TASK-0124] The (per_worker EventList, NameTables, NameSidecar) contract is now PROVEN sufficient for a tier-1 backend: pthreads-sync consumes ONLY compiler::event + compiler::sidecar + the inert IrExpr grammar the EventList carries (NO AlgoIR/LinkedIR/ACFG), single- AND multi-worker, byte-identical e2e+determinism. mp-tcp-bufsync should consume the SAME tuple. Reuse the pattern: driver builds acfg_to_events(&acfg)+build_sidecar(&linked,&acfg).map_err(...)? + reverse NameTables; backend emit() takes (&per_worker,&names,&sidecar,kernels,out). Use an EmitError::ContractGap fail-loud variant for any missing contract fact (never default). CAVEATS that apply identically to the 2nd backend: (1) Event::Sync carries NO stable cross-worker barrier identity — TASK-0172; multi-worker barrier-id recovery is a per-worker pre-order-index heuristic valid only for UNIFORM barriers (fail loud otherwise). (2) block_transform DEFERS absolute-index rebinding (LO+tile*N+inner) to codegen; the EventList faithfully carries the tiled nest so the backend MUST rebind or an accumulator double-counts. TASK-0124 rebinds only the evenly-divisible case; non-divisible/partial-tile is TASK-0173. Any EventList-consuming backend inherits both.

IMPLEMENTED. Second tier-1 backend done: backends/mp-tcp-bufsync (crate + capabilities.toml, AC#1), workers are OS processes over TCP loopback, sync = blocking recv, buffered.

Architecture: same emit() signature + EventList contract as pthreads-sync (AlgoIR/LinkedIR/ACFG-free; consumes only compiler::event + compiler::sidecar + the inert IrExpr grammar). NO renderer drift (TASK-0124 flagged risk): expression/index/kernel-call/loop-bound/single-worker rendering is the SINGLE pthreads-sync implementation, exposed pub and called via shims (render_fire_args_pub/render_flat_index_pub/render_const_expr_pub/render_single_worker_main/rust_type_of/render_array_init_for). Verified: single-process mp-tcp main body is BYTE-IDENTICAL to pthreads-sync main.rs. Only the multi-PROCESS transport is mp-tcp-specific.

AC#2: emit produces N Rust binaries (src/bin/<worker>.rs, one per used WorkerId) + src/wire.rs + src/kernels.rs + Cargo.toml ([[bin]] per worker) + run.sh that launches+wires them (TASK-0038). 0/1 used workers => single binary (shared single-worker renderer).
AC#3: deterministic handshake — host is the server (binds one listener per non-host worker, accept()s twice: 1st=DATA 2nd=CTRL, role by accept order, no handshake bytes); worker connects twice (DATA then CTRL) with a bounded connect-retry (liveness wait, not data sync; no sleeps-as-sync). Ports auto-allocated by a std-only Rust __nuc_pick_port binary (NO python3 — reproducible under nix develop) and passed via env NUC_TCP_PORT_<worker>.
AC#4: each Push lowers to wire::write_msg (length-prefixed write on the data socket); each Wait to wire::read_msg_expect (blocking read + seq-tag fail-loud cross-check + decode).
AC#5: synthetic two-worker pingpong test (tests/pingpong.rs) runs BOTH backends on the identical pipeline and asserts mp-tcp-bufsync output == pthreads-sync output BIT-FOR-BIT (and == the 1720 oracle). Passes, non-flaky across repeated runs.
AC#6 design questions recorded: handshake protocol (two conns per pair, role by accept order); SO_REUSEADDR deliberately NOT set (stale-port clash must fail loud, not silently rebind); Bind errors => host panics naming the port (fail-loud).
AC#7 honest limitations recorded: no buffer-pool reuse; one Vec<u8> allocation per transfer at M3; perf is NOT a goal.

KEY TCP-SPECIFIC GOTCHA (rejected approach): a FIRST design put data + barrier on ONE stream keyed by a shared seq space. It DEADLOCKED/mis-framed: the relative order of a barrier vs a data transfer differs between producer and consumer (host emits a,b then a barrier; worker reaches the barrier first), so the worker would read the `a` data frame as the barrier token. pthreads-sync only escapes this because its Slot and Barrier are SEPARATE objects (no shared FIFO). FIX (root cause, not workaround): TWO connections per (host,worker) pair — DATA (Push/Wait) and CTRL (barriers). Within each channel order is independently consistent (projection guarantees Push order==Wait order; uniform-barrier pre-order index aligns); cross-channel order never matters. This is why two connections, not one.

Inherited caveats (fail-loud, identical to pthreads-sync, NOT fixed here): (1) Event::Sync has no stable cross-worker id — per-worker pre-order Sync index, uniform-barriers-only, EmitError::ContractGap otherwise (TASK-0172). Confirmed working: 03-reduction/distributed correctly fails LOUD with the non-uniform-barrier ContractGap rather than emit a wrong binary. (2) block_transform absolute-index rebinding handled by the shared single-worker renderer (divisible case); non-divisible is TASK-0173. No required mp-tcp cell hits a blocked MULTI-worker schedule.

Follow-ups filed: TASK-0174 (verify AC#5-style SO_*BUF clear-error in a lowered-wmem_max container — deferred from TASK-0038), TASK-0175 (worker-to-worker channel / host-excluding barrier mesh — current STAR topology fails loud on peer!=host, sufficient for tier-1 scope). Code comments reference these IDs.

GATE (inside nix develop): just test = 0 failed (incl. mp-tcp-common 7, pingpong 1, pthreads-sync multi_worker unchanged). just e2e = total 20 / pass 16 / fail 0 / skipped 4 / required-fail 0 (pthreads-sync 8 example-schedule pairs UNCHANGED; mp-tcp-bufsync 8 new differentially-green). just determinism-check = 0 fail (mp-tcp byte-identical across two builds too). just determinism-check-negative = still bites. cargo clippy --workspace -D warnings = clean. Generated code warning-clean (role-specific imports). 02-split/split run 5/5 byte-identical, pingpong 3/3 stable — no TCP flakiness observed.

Differentially green under BOTH pthreads-sync AND mp-tcp-bufsync (same reference.bin oracle): 01-elementwise-add/naive, 02-split-add/naive, 02-split-add/SPLIT (the load-bearing host+w0 two-OS-process differential), 03-reduction/naive, 05-stencil/naive, 05-stencil/blocked, 07-matmul/naive, 07-matmul/blocked.

[forward-carried from TASK-0036] For TASK-0041 (M3 cross-backend acceptance gate, orchestrator-driven): the e2e harness is now transport-aware (CapabilitiesSniff.transport: shared-memory => single binary, else => run.sh). The differential is REAL and pinned in e2e-matrix.toml as required cells under BOTH backends. The 4 distributed cells are [[skip]] for BOTH backends (TASK-0117/0172, not transport-specific). For further mp-tcp-* backends: reuse mp-tcp-common (wire v0) and the pthreads-sync pub shared renderers; the STAR topology + two-channel (data/ctrl) split is the proven pattern; non-uniform-barrier and worker-to-worker are the known fail-loud edges (TASK-0172/0175).

ORCHESTRATOR HONESTY CORRECTION (phase3-ralph review gate, mped-architect 🔴 MUST-FIX): TASK-0036 was set Done by the implementer, but its AC#2 ("plus a run.sh") and AC#5 (pingpong drives run.sh end-to-end) are exercised by run.sh whose deliverable is TASK-0038, which is honestly In Progress (its AC#5 — SO_*BUF clear-error in a lowered-wmem_max container — deferred to TASK-0174). A Done task transitively resting on an In-Progress deliverable violates the project DoD. Per phase3-ralph honest-failure discipline this is corrected, NOT re-gamed: TASK-0036 -> In Progress with an explicit Dependencies: TASK-0038 edge. All of TASK-0036s OWN ACs are functionally MET and independently verified by the review gate (run.sh works; the differential is real); only the transitive TASK-0038 container-socket-buffer edge remains. TASK-0036 flips to Done when TASK-0038 closes (TASK-0174). ORCHESTRATOR-VERIFIED gate numbers (qa-test-runner re-ran, not transcribed): just e2e 5x VERBATIM IDENTICAL total 20/pass 16/fail 0/skip 4/required-fail 0 (02-split-add/split under mp-tcp-bufsync PASS all 5, zero flakiness); pingpong 3/3; wire 7/7; pthreads-sync 8 pairs UNCHANGED; determinism both backends byte-identical; negative bites x2; clippy clean; AlgoIR-free verified by grep; differential REAL — 02-split/split mp-tcp output.bin SHA256 == hand-written reference.bin (independent third oracle), harness genuinely runs run.sh launching N OS processes. mped-architect: differential genuine (not circular) — transport+projection independently re-derived in mp-tcp Plan; shared renderer means shared-ARITHMETIC bugs caught only by reference.bin oracle (honestly disclosed in the e2e-matrix comment, not overclaimed); single-stream-deadlock root-cause + two-channel(DATA/CTRL) fix sound; TASK-0172/0175 real typed-ContractGap fail-loud (verified in code); port-picker TOCTOU bounded + fail-loud (not silent-wrong).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added mp-tcp-bufsync — the SECOND tier-1 backend — making the project core thesis falsifiable: the same (algorithm, schedule) now produces a BIT-IDENTICAL output.bin under two independent backends.

What changed:
- New crate nucleus/backends/mp-tcp-bufsync (+ capabilities.toml, tier 1, transport=tcp, sync, buffered). Workers are OS PROCESSES; transport std::net::TcpStream over loopback; sync = blocking recv.
- New crate nucleus/mp-tcp-common: TCP wire protocol v0 (TASK-0037), single-source-of-truth runtime copied verbatim into generated projects.
- docs/wire-protocol-v0.md.
- pthreads-sync: pure renderers exposed pub (single shared codegen implementation; mp-tcp-bufsync calls them — no drift, the TASK-0124-flagged risk is structurally eliminated). No behaviour change to pthreads-sync output (byte-identical, determinism gate unchanged).
- Driver: NameTables built once, both backends dispatched; mp-tcp-bufsync registered.
- e2e harness: transport-aware run phase (shared-memory => single binary; tcp => run.sh). Determinism check is backend-agnostic and works unchanged.
- e2e-matrix.toml: mp-tcp-bufsync added; required cells under BOTH backends; 4 distributed cells [[skip]] for both (TASK-0117/0172, not transport-specific).
- run.sh generation (TASK-0038): per-worker process launch, env-sized SO_*BUF, non-zero naming the failed worker, reproducible std-only port picker (no python3).

Why two TCP connections per (host,worker) pair: a one-stream design dead-locked because barrier vs data ordering differs between producer and consumer; splitting DATA and CTRL channels restores the per-channel order consistency pthreads-sync gets from separate Slot/Barrier objects. Root-cause fix, not a workaround.

User impact: cross-backend differential is now real and CI-pinned. 8 example/schedule pairs are byte-identical under BOTH pthreads-sync AND mp-tcp-bufsync against the hand-written reference oracle — including the load-bearing 02-split-add/split (host + w0 as two OS processes over loopback TCP).

Tests: just test 0 failed (mp-tcp-common 7, pingpong differential 1, pthreads-sync multi_worker unchanged); just e2e 20/16-pass/0-fail/4-skip/0-required-fail; determinism-check 0 fail; determinism-check-negative still bites; clippy --workspace -D warnings clean. 02-split/split 5/5 + pingpong 3/3 — no TCP flakiness.

Risks / follow-ups: TASK-0174 (verify AC#5-style SO_*BUF clear-error in a lowered-wmem_max container — fail-loud code present but container scenario unverified; TASK-0038 stays In Progress until then). TASK-0175 (worker-to-worker / host-excluding-barrier mesh — current STAR topology fails LOUD on peer!=host, sufficient for tier-1 scope). Inherited fail-loud caveats: TASK-0172 (non-uniform barrier — confirmed correctly rejected, not mis-emitted), TASK-0173 (non-divisible blocked accumulator — no required mp-tcp cell hits it). Perf is explicitly not a goal at M3.
<!-- SECTION:FINAL_SUMMARY:END -->
