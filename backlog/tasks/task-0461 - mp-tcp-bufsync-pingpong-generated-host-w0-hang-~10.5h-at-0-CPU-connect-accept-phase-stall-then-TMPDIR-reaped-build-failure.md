---
id: TASK-0461
title: >-
  mp-tcp-bufsync pingpong: generated host/w0 hang ~10.5h at 0% CPU
  (connect/accept-phase stall), then TMPDIR-reaped build failure
status: Done
assignee: []
created_date: '2026-06-10 08:37'
updated_date: '2026-06-11 04:30'
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
- [x] #1 Root cause of the connect/accept-phase hang identified with a witness (not hypothesis) and fixed in the generated connect/listen path — bounded retry with deadline, loud failure
- [x] #2 Generated-program integration tests carry a watchdog: a wedged pair fails the test within minutes with diagnostic capture, never stalls the gate for hours
- [x] #3 Pingpong scratch dirs reclaimed on success; accumulated ~812 stale dirs cleaned
- [x] #4 Full just test green 10 consecutive runs after the fix (subsumes TASK-0426 AC#2 evidence)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ROOT-CAUSE (witness): generated connect/rendezvous IS bounded (6s each, fail-loud) but TWO blocking calls remain UNBOUNDED on the dispatch path: (i) host listener_<w>.accept() x2 per worker (worker_program.rs:209/215) and (ii) every Push/Wait wire::read_msg -> read_exact (wire_runtime.rs:170/175) — NO set_read_timeout/accept deadline anywhere (grep-verified zero set_read_timeout in tcp_plan+mp-tcp-common). A 6s connect cannot hang 10.5h; an unbounded accept/read_exact at 0% CPU can, and matches "no established sockets + self-unwedge on probe". FIX (generated code, AC#1): (a) host sets listener.set_nonblocking + bounded accept loop w/ wall-clock deadline + loud panic naming worker; (b) bufsync data/ctrl sockets get set_read_timeout(NUC_WIRE_DEADLINE) and read_msg maps WouldBlock/TimedOut past deadline to a loud panic (poll path already deadline-bounded). AC#2: test-common::proc_timeout shared watchdog; pingpong tests (bufsync+poll) wrap run.sh spawn. AC#3: on-success scratch GC in pingpong family + e2e-matrix retained-run prune. AC#4: document 10x just test for batch gate (likely no wall budget here).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-NOTE from TASK-0426.01 scratch-dir sweep (read-only observation, NOT a fix — connect path is backend src, owned here):

(A) CONNECT/RETRY PATTERN POINTERS. The generated host/w0 connect+accept logic is rendered from nucleus/backend-common/src/tcp_plan/worker_program.rs; the runtime framing/read side is nucleus/mp-tcp-common/src/wire_runtime.rs (+ src/lib.rs). The documented design (mp-tcp-bufsync/src/lib.rs:32-50): host binds 127.0.0.1:0 per non-host worker, atomically writes the port to NUC_RENDEZVOUS_DIR/<worker>.port via tmp+rename; each worker polls the rendezvous file 600x10ms=6s, then connects with a bounded retry loop described as symmetric with the 6s file-poll bound.

(B) INCONSISTENCY WORTH A WITNESS. The TASK-0461 evidence (10.5h at 0% CPU, NO established sockets in ss -tnp) is INCONSISTENT with a connect retry that is actually bounded at ~6s and fails loud. Three candidate mechanisms the root-cause should distinguish with a witness, since a 6s-bounded connect cannot hang 10h: (1) the rendezvous-FILE poll wedges (e.g. host died before writing <worker>.port, or wrote to a different NUC_RENDEZVOUS_DIR, so the worker spins/sleeps past its nominal bound — verify the bound is enforced with a hard deadline, not an unbounded while-let); (2) connect SUCCEEDS but a subsequent BLOCKING read_exact on the stream blocks forever because the peer wedged mid-protocol (no set_read_timeout in wire_runtime.rs — grep showed read_exact present; a blocking read with no deadline is the classic 0%-CPU-no-progress signature and matches "no NEW connect attempt visible"); (3) host listener accept() blocks because the worker never connected (host side). The 0% CPU + the pair self-unwedging coincident with a read-only ss/proc probe points more at a blocked read/accept than a busy retry spin. RECOMMEND: confirm whether the rendered connect loop AND every post-connect read carry an enforced wall-clock deadline; a bounded-retry-with-deadline + loud failure is needed on BOTH the connect AND the read/accept, not just connect.

(C) SCRATCH ROT (TASK-0461 AC#3) is INTRINSIC to the sweep design. test_common::unique_scratch_dir (and the inline e2e_example/task0341 idiom) CREATE-ONCE-NEVER-REMOVE by construction (that is how they kill the remove/create race). So per-run dirs accumulate forever under nucleus/target/<crate>-*-scratch/ and nucleus/target/e2e-scratch/ — I observed the same accumulation in nucleus/target/e2e-determinism/run-* as well. The ~812 pingpong dirs are the expected steady-state of this design, NOT a regression. AC#3 reclamation must be a SEPARATE on-success GC (like e2e diff_fuzz already does at nucleus/e2e/src/bin/diff_fuzz.rs:857-861 sweep_dead_scratch) — it cannot be folded back into unique_scratch_dir without reintroducing the remove/create race the helper exists to prevent. Keep the two concerns separate.

FORWARD-NOTE #2 from TASK-0426.01 re-audit (2026-06-10, wave-2; read-only, connect path is backend src not in my ownership): confirmed the harness-watchdog gap (AC#2) at the test call site. nucleus/backends/mp-tcp-bufsync/tests/pingpong.rs runs the generated pair via a BLOCKING Command::new("bash").arg(out_dir.join("run.sh")).arg(input_bin).arg(output_bin).output() at pingpong.rs:206-212 with NO timeout/watchdog wrapper and NO per-test cargo-test timeout. So if run.sh host/w0 wedge in the connect/accept phase, .output() blocks the test thread forever — exactly the 10.5h-at-0%-CPU signature. The pingpong_matches_pthreads_sync_bit_for_bit test is the one that ran 37713s. SCRATCH handling at pingpong.rs:42-49 is ALREADY clean (routes through test_common::unique_scratch_dir, no remove/create_dir_all) — AC#3 reclamation is unrelated to that helper and must be a separate on-success GC (per my earlier forward-note (C)). RECOMMENDED minimal harness fix for AC#2 independent of the root-cause connect fix: wrap the run.sh spawn in a spawn()+wait-with-deadline (or run under `timeout` / a watchdog thread that kills the child + captures ps/ss) so a wedged pair fails the test in minutes with diagnostics rather than stalling the gate overnight. The generated-side connect/read bounded-deadline fix (AC#1) is still the real root cause and lives in nucleus/backend-common/src/tcp_plan/worker_program.rs + nucleus/mp-tcp-common/src/wire_runtime.rs (per forward-note #1).

Wave-6 ENOSPC addendum (2026-06-10): the gate-killing disk exhaustion recurred at campaign scale — target/debug grew to 70GB across a day of repeated full test builds, plus 6 retained e2e-matrix runs and ~9GB of accumulated create-once-never-remove test scratch. AC#3 (scratch GC) should cover the broader classes: e2e-matrix retained-run pruning policy, periodic scratch reclamation, and a note that long campaigns must budget target/debug churn (84GB reclaimed manually this time).

CYCLE WORK (batched with TASK-0466):

AC#1 ROOT-CAUSE + FIX (witness, in generated connect/listen path):
ROOT CAUSE = the generated HOST listener.accept() was UNBOUNDED-blocking. Worker-side connect_retry + rendezvous-poll were already 6s-bounded fail-loud, but host accept was not. A worker that never connected (died / raced a stale rendezvous port / wedged before its 2nd connect) left host blocked in accept() FOREVER at 0% CPU with NO established socket — EXACTLY the 10.5h evidence signature (NO established sockets in ss -tnp is INCONSISTENT with a post-connect blocked read; CONSISTENT with accept-before-connect). FIX: nucleus/mp-tcp-common/src/wire_runtime.rs new wire::accept_with_deadline(listener, role, who) + handshake_deadline_ms() (NUC_HANDSHAKE_DEADLINE_MS, 30s default, fail-loud, never 0/unbounded). worker_program.rs host accept x2 now routes through it; restores BLOCKING on listener+stream so the sync wire protocol is byte-for-byte unchanged. WITNESS (empirical): ran the real generated host alone with NUC_HANDSHAKE_DEADLINE_MS=500 — it PANICKED LOUD "host: accept DATA from w0 timed out after 503 ms >= ...=500 ms — the worker never connected ..." exit 134, NOT a 20s hang. e2e slice byte-exact: 02-split-add/split x {mp-tcp-bufsync, mp-tcp-poll} both cmp-equal to reference.bin with the new generated host. SILENT-SIBLING FILED: TASK-0461.01 — mp-tcp-event/mp-uds-event have their OWN plan.rs with the same unbounded host accept (their reads are mio-nonblocking+deadline; only the host accept is the residual). Not fixed here (backends/*/src outside this task ownership).

AC#2 WATCHDOG: shared test_common::proc_timeout (lifted from diff_fuzz exec.rs, single source) provides run_or_timeout(); both pingpong runners (mp-tcp-bufsync + mp-tcp-poll tests/pingpong.rs) wrap the bash run.sh spawn — a wedged pair now FAILs the test in minutes via process-group SIGKILL + tail, never stalls. Budget env NUC_E2E_TIMEOUT_SECS (600s default).

AC#3 SCRATCH GC: on-success reclaim_scratch_on_success() added to all 4 pingpong-family tests (runs only after assertions pass; failed runs keep tree for post-mortem; never shares a path so no remove/create race). e2e-matrix retained-run PRUNING: paths.rs prune_retained_runs(keep=RETAINED_RUNS_KEEP=4) on each FAILED run bounds the wave-6 ENOSPC unbounded-failure-series class. Current pingpong scratch already down to 6 dirs each (GC self-reclaiming).

VERIFICATION: dev+release green on test-common / mp-tcp-common / mp-tcp-bufsync / mp-tcp-poll / backend-common (22 ok-blocks each profile, 0 failures); e2e bins unit tests green incl 2 negative timeout tests; clippy workspace clean; 3 structural fences (include-str / textual-replace / mega-files) green; diff-fuzz k=2 still 7/7 byte-identical through the shared facade.

AC#4 (10x just test) NOT run in-session (wall budget); LEFT TO BATCH GATE. Command for orchestrator: for i in $(seq 1 10); do nix develop --command bash -c "just test" || { echo "FAIL run $i"; break; }; done

Hardening review P3.2 residual recorded: the pingpong tests wrap only the bash run.sh spawn with the watchdog; the two cargo-build spawns + the pthreads reference-binary run stay bare .output() — a wedged build or deadlocked reference binary can still stall just test. Acceptable residual for now (run.sh was the demonstrated hang vector); widen if it ever fires.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Root cause WITH WITNESS: the generated host unbounded listener.accept() (worker-side already 6s-bounded; the no-established-socket evidence fits accept-before-connect exactly). Fixed in the generated wire path: accept_with_deadline + NUC_HANDSHAKE_DEADLINE_MS (30s, warn-on-malformed), witnessed panicking loud at 503ms with a 500ms budget. Watchdog: shared test_common::proc_timeout (process-group kill; CONCURRENT pipe drains after the full-pipe stall falsely killed chatty builds — found, fixed, regression-pinned in 67e4037) wraps the pingpong-family run.sh spawns + both harnesses. Scratch GC: reclaim-on-success + e2e retained-run pruning with a liveness guard (review-hardened, pinned). AC#4: ten consecutive just test runs GREEN (final batch gate). Residuals filed: TASK-0461.01 (event-backend host accept siblings, amended for the UDS reality), cargo-build spawns in pingpong unwrapped (noted). Landed 90877a2 + 314475d + 326f2f3 + a61c379 + 67e4037; architect GO; full just ci green.
<!-- SECTION:FINAL_SUMMARY:END -->
