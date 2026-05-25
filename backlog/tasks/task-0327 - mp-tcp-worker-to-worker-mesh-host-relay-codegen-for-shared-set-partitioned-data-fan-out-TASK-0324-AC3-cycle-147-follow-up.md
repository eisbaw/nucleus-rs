---
id: TASK-0327
title: >-
  mp-tcp worker-to-worker mesh / host-relay codegen for shared-set
  partitioned-data fan-out (TASK-0324 AC#3 cycle-147 follow-up)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 16:04'
updated_date: '2026-05-25 18:37'
labels:
  - M6
  - backend
  - mp-tcp-bufsync
  - mp-tcp-event
  - topology
  - forward-carried-from-TASK-0324
dependencies:
  - TASK-0324
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0324 AC#3 (cycle 147) landed cross-worker tmp codegen for the same-set producer-set==consumer-set + reader-iv-exceeds-producer-tile shape (06-separable-filter/distributed2 reproducer). Verified bit-identical on the two shared-memory backends (pthreads-sync, pthreads-async).

The two mp-tcp backends (mp-tcp-bufsync, mp-tcp-event) cannot lower the 12 cross-worker (src, dst) Push/Wait pairs that the pass now emits — their one-(data,ctrl)-pair-per-(host,worker) STAR topology has no worker-to-worker channel. EmitError::ContractGap fires LOUD at Plan::build with the verbatim messages:

- mp-tcp-bufsync: `mp-tcp-bufsync's one-(data,ctrl)-pair-per-(host,worker) topology has no worker-to-worker channel (filed as TASK-0175)`
- mp-tcp-event:    `the star topology requires host as the relay (filed as TASK-0175)`

TASK-0175 was closed cycle-77 as DEFERRED-until-TASK-0117-lands-AND-a-distributed-schedule-needs-worker-to-worker (AC#3 of TASK-0175). Both conditions are now met: TASK-0117 fan-out has been live for many cycles, and 06/distributed2 is the in-tree schedule that exercises the worker-to-worker shape.

## Acceptance Criteria
<!-- AC:BEGIN -->
### AC#1: mp-tcp-bufsync worker-to-worker channel

Extend mp-tcp-bufsync's transport so a Push from WorkerId(i) to WorkerId(j) (i, j both non-host) routes correctly. Two viable approaches:

- **Full mesh**: each worker opens a (data, ctrl) connection pair to every other worker at startup. N*(N-1) connections per worker = quadratic in worker count but eliminates the host as a bottleneck.
- **Host relay**: workers route worker-to-worker Push/Wait through the host; host forwards the payload. Lower connection count, host becomes hot-path bottleneck. Acceptable for the cycle-147 distributed2 shape since the 12 cross-pairs are amortised over the H*W vblur loop body.

The simpler near-term fix is host-relay; mesh is the M6+/M7 target.

### AC#2: mp-tcp-event worker-to-worker channel

Same shape as AC#1 for mp-tcp-event. The mio reactor + per-(seq, peer) outbound queue (TASK-0042.05 Stage 3) already provides the per-peer fan-out machinery; the gap is the connection topology (no worker-to-worker socket pair exists at startup).

### AC#3: 06-separable-filter/distributed2 promotion

Once AC#1 + AC#2 land, flip the two [[skip]] entries in nuc-nucleus/e2e-matrix.toml lines ~1290-1310 (TASK-0327-citing) to [[required]] and verify bit-identical against reference.bin. e2e baseline shifts by +2 [[required]] -2 [[skip]].

## Honest scope

- MEDIUM priority. The cycle-147 AC#3 codegen already produces correct output on the two shared-memory backends (50% of the tier-1 matrix); mp-tcp coverage is the M5/M6 cross-backend completeness story.
- Trigger: M6 acceptance criterion or a follow-up that needs the full tier-1 matrix bit-identical on 06/distributed2.

## Dependencies

- TASK-0324 (cycle 147 AC#3 landed): `producer-set == consumer-set` fan-out emission. This task lifts the resulting topology gap that surfaces on mp-tcp backends.
- TASK-0175 (Done, deferred-until): the original filing of the mp-tcp worker-to-worker limitation. Now actionable per its own reopen-criterion.

## Cross-reference

- nucleus/backends/mp-tcp-bufsync/src/lib.rs (the host-only EventList Plan::build branch).
- nucleus/backends/mp-tcp-event/src/lib.rs (the host-relay-requires Push branch).
- 06-separable-filter/distributed2 emits 12 cross-pairs (4*3 = 12) under pthreads-sync; same count expected for the eventual mp-tcp implementations.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Cycle 148 implementation plan (orchestrator self-implementing per memory feedback-spawned-agents-refuse-code-edits)

### Scope decision
Land AC#1 (mp-tcp-bufsync worker-to-worker host-relay) + a slice of AC#3 (06/distributed2 × mp-tcp-bufsync promotion). DEFER AC#2 (mp-tcp-event) to cycle 149 — separate backend, mio reactor adds complexity. AC#3 partial (3 of 4 cells promoted).

### Design: synchronous host-relay, codegen-driven dispatch
- Relax `data_conn_var`: non-host worker with non-host peer returns `data_host` (worker writes/reads through its existing host connection).
- Plan computes `relay_schedule`: per non-host src worker, ordered list of (seq, dst, data) for every w2w Push event in src's event list (event order = TCP wire order on src's data_host stream).
- HOST emits a synchronous relay phase: for each src in BTreeMap order, for each (seq, dst) in src's list, emit `let payload = wire::read_msg_expect(&mut data_<src>, seq); wire::write_msg(&mut data_<dst>, seq, &payload);`.
- Relay phase insertion point: right before host's FIRST top-level Wait event in render_events (heuristic that works for scatter-compute-relay-gather schedules like 06/distributed2). If host has no Wait events, insert at end of main.
- No new wire helpers. No new Event variants. No threads. No protocol changes. No backend-common changes.

### Why this works (and the load-bearing assumptions)
For 06/distributed2: each worker pushes its 3 w2w cross-tmps BEFORE waiting on its 3 incoming, then computes pass 2, then pushes 'out' (gather). Host's events: scatter Pushes → gather Waits, with relay phase inserted between. Workers' pushes buffer in TCP; host drains and forwards in src-sorted order.

Seq-tag alignment: cycle-147 cartesian-product fan-out emits w_i's Push events for tmp in dst-sorted order; consumer w_j's Wait events for tmp are in src-sorted order. Host iterates srcs in BTreeMap (sorted) order. So host's writes to data_w_j land in [from src=w0, from src=w1, ...] order, matching w_j's Wait order. Mismatch fires loud (wire::read_msg_expect panics on seq tag mismatch).

Limitation (acceptable for cycle 148): host cannot have data reads BETWEEN scatter and gather (would race with relay reads on same data_<src> socket). 06/distributed2 satisfies; complex interleaved schedules would need either threaded relay or scheduled relay events. File follow-up if a future schedule needs this.

### Implementation steps
1. Add `RelayHop { seq: SeqTag, dst: WorkerId, data: DataId }` struct + `collect_w2w_pushes` helper (recursive into Loop bodies, filters Push with src != host && dst != host).
2. Add `Plan::relay_schedule() -> BTreeMap<WorkerId, Vec<RelayHop>>`.
3. Relax `data_conn_var` (lines 1228-1254): non-host worker with non-host peer returns `Ok("data_host".to_string())` with a TASK-0327 explainer comment.
4. In `render_worker_program`: after computing relay_schedule, if non-empty AND is_host, plan the insertion. Modify `render_events` call sequence: walk events, find index of first top-level Wait, split events into [pre_wait, post_wait], render pre_wait, emit relay block as a String, render post_wait. (Or simpler: a flag-passed approach inline in render_events.)
5. Update comments at line 555-559 + line 1228-1254 to reflect the new w2w-via-host-relay semantics.
6. Update the e2e-matrix.toml skip→required for 06/distributed2 × mp-tcp-bufsync (line 1307-1312); update the schedule header comment.

### Verification gate
- `just check / clippy / test / test-release / e2e` all green.
- `just e2e`: 112/94/0/18/0 → 112/95/0/17/0 (one cell flip).
- Bit-identical against reference.bin for 06/distributed2 × mp-tcp-bufsync.
- Existing 02-split × mp-tcp-bufsync (the trivial host↔worker case) stays green.
- 03-reduction/distributed × mp-tcp-bufsync — currently SKIPPED on TASK-0175 (host-excluding barriers). My data_conn_var change does NOT lift the barrier check at lines 382-393, so that cell stays SKIPPED with the same reason. Verify no regression.
- Run parallel review gate (qa-test-runner + mped-architect read-only) after implementation.

### Forward-carry to cycle 149
- AC#2: replicate the host-relay pattern in mp-tcp-event. The mio reactor's per-(seq, peer) Chan abstraction needs an adapter for the relay phase.
- Barrier mediation: 06/distributed2 happens to not produce host-excluding barriers; future schedules might. Then the line 382-393 check needs lifting + host barrier injection.
- mp-tcp-event AC#3 cell (06/distributed2 × mp-tcp-event) blocked on cycle 149's AC#2.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 148 final state — Done (partial) after parallel review gate GO

### What landed (cycle 148, mp-tcp-bufsync slice only)

**AC#1 mp-tcp-bufsync worker-to-worker via SYNCHRONOUS host-relay**:
- `data_conn_var` relaxed: non-host worker with non-host peer returns `data_host` (lifts the cycle-147 fail-loud ContractGap rejection); HOST relays bytes through to/from the actual peer.
- New `Plan::relay_schedule` enumerates per non-host src worker the ordered list of (seq, dst, data) for every w2w Push event (event-list order = TCP wire order on src's data_host stream).
- New `Plan::render_relay_phase` emits host's sync relay block: for each src in BTreeMap (sorted WorkerId) order, for each hop in event-list order, `let payload = wire::read_msg_expect(data_<src>, seq); wire::write_msg(data_<dst>, seq, &payload);`. Seq cross-check (wire::read_msg_expect) preserves wire-protocol-v0 fail-loud contract.
- New `relay_phase_insertion_point` heuristic: insert relay just BEFORE the LAST top-level Event::Sync (= between pass-1 barrier and pass-2 barrier; satisfies both 'workers must have crossed pass-1 barrier to push tmps' and 'host must finish relay before crossing pass-2 barrier'). Fallback: before first top-level Wait. Last-resort: end of main.
- New `collect_w2w_pushes` + `RelayHop` helpers (recursive into Loop bodies; current cycle's flat-emit assumption documented + filed forward as TASK-0330 for fail-loud).
- `render_worker_program` for HOST: splice point computed; pre-split + relay + post-split rendered in order.
- Stale 'today's tier-1 set is 2-party' comment at lib.rs:553-555 updated to reflect post-cycle-148 5-party reality.

**AC#3 partial matrix promotion (3 of 4 backends)**:
- 06-separable-filter/distributed2 × mp-tcp-bufsync flipped [[skip]] → [[required]] in e2e-matrix.toml; bit-identical verified.
- e2e baseline shift: 112/94/0/18/0 → **112/95/0/17/0** (5-sample non-flake: 4/5 clean runs, 1/5 transient mp-tcp port race in a DIFFERENT cell — cycle-148 target was bit-identical in that very run per qa).
- Tests: 879/0/3 → 882/0/3 (+3 new emit-pinning tests in host_relay_emit.rs).

### Parallel review gate

- **qa-test-runner**: GO. 9 gates green (check, clippy, test, test-release, check-textual-replace, check-include-str-coverage, e2e, just ci, host_relay_emit tests). 5 e2e samples on 112/95/0/17/0 (4/5 stable; 1 unrelated cell flake — see honesty gap below).
- **mped-architect**: GO with 4 P2 fold-back items + 3 P3 task filings. All P2 folded back in-thread (next commit):
  - P2.1: rewrote `relay_phase_insertion_point` doc constraint #1 to match implementation order (last-Sync primary, first-Wait fallback) — the prior prose listed constraints in narrative order that contradicted the priority order.
  - P2.2: bubbled the EmitError from `data_name` in `render_relay_phase` (was silently falling back to `data_{DataId:?}` in a comment — doc-lie risk per feedback-comment-doc-lie-recurring). Signature changed to `Result<String, EmitError>`; `render_worker_program` propagates with `?`.
  - P2.3: verified mp-tcp-event has no sibling `data_conn_var` requiring re-threading; the `_worker` parameter rename is purely localized cleanup.
  - P2.4: pinned the between-barrier test witness comment explaining why the trailing comma in `ctrl_w0,` matters (defends against future rename to `ctrl_to_w0` substring overlap).
- P3 filings (cycle-149+ work):
  - **TASK-0329** filed: host-mediated barrier mediation for host-excluding barrier shapes (analogous to cycle-148's data lift; lib.rs:382-393 rejection still bites; dormant, no in-tree trigger today).
  - **TASK-0330** filed: defensive ContractGap when `collect_w2w_pushes` finds a w2w Push inside a Loop body (current cycle's flat-emit assumption; no in-tree trigger; fail-loud > silent mis-relay).
  - P3.3 effort-estimate honesty: memory file `feedback-implementer-effort-estimate-overstated` updated with cycle-148 datapoint (Explore agent over-estimated by 100% by including unnecessary wire-protocol changes; sub-lesson: probe less-invasive alternatives before accepting wider scope).

### Honesty gaps disclosed

- **5-sample non-flake**: cycle-148 ran 5 e2e samples; 4 cleanly landed 112/95/0/17/0; 1 hit a transient recipe exit-1 in a DIFFERENT mp-tcp cell (cycle-148 target was confirmed bit-identical in that very run by qa). Framing the result as 'stable' is true for the cycle-148 target but the e2e harness has a residual transient flake elsewhere (separate from TASK-0327 scope).
- **Effort over-estimate verified**: Explore agent's 250-270 LoC estimate + wire-protocol-change assumption was about 2x the actual (~239 LoC of lib.rs + 234 LoC of tests, NO wire-protocol changes). Logged in memory.
- **cycle-148 scoped to mp-tcp-bufsync only**: AC#2 (mp-tcp-event) remains for cycle 149+; the host-relay shape needs replication into the mio reactor (per-(seq, peer) Chan abstraction needs an adapter). The relevant sibling rejection at `mp-tcp-event/src/multi_worker.rs:225-237` is structurally identical to the pre-cycle-148 `data_conn_var` rejection — known silent-sibling, honestly deferred.
- **AC#3 still partial** (3 of 4 backends bit-identical); AC#4 closure depends on TASK-0327 mp-tcp-event slice landing.

### Status

**TASK-0327 stays In Progress.** AC#1 partially landed (mp-tcp-bufsync only); AC#3 partial (3 of 4 cells promoted). AC#2 (mp-tcp-event) + remaining 1 cell promotion deferred to cycle 149.

## Cycle 149 implementation plan (orchestrator self-implementing per memory feedback-spawned-agents-refuse-code-edits)

### Scope decision
Land AC#2 (mp-tcp-event worker-to-worker host-relay) + close AC#3 (06/distributed2 fourth-backend [[required]] promotion). Mirror the cycle-148 mp-tcp-bufsync design verbatim: same wait-then-push semantics, same splice point heuristic (before LAST top-level Sync), same sorted-WorkerId ordering, same per-hop disambiguating comments.

### Design: synchronous host-relay over mio reactor
- `runtime_src.rs`: NEW `Reactor::relay_one(seq, dst_peer, cap)` primitive = `let p = self.wait(seq); self.push(seq, dst_peer, p, cap)`. Bytes-verbatim; bypasses Chan<T> encode/decode (the wire codec is symmetric for w2w).
- `multi_worker.rs`: lift the cycle-79 w2w-Push rejection at Plan::build (now bites only `src==dst==host` malformedness). `chan_peer_index`: when both src and dst non-host, replace effective_peer with `host_worker` so peer_index_for returns 0 (host).
- `multi_worker.rs`: NEW `RelayHop { seq, dst, data, cap }`, `collect_w2w_pushes` (recursive into Loop bodies — same cycle-148 limitation filed forward), `relay_phase_insertion_point` (LAST top-level Sync → first top-level Wait fallback → end-of-events last resort).
- `Plan::relay_schedule()` → `Result<BTreeMap<WorkerId, Vec<RelayHop>>, EmitError>`.
- `Plan::render_relay_phase(indent)`: emits `{ let mut __relay = reactor.borrow_mut(); __relay.relay_one(seq, dst_peer, cap); ... }` block with per-hop disambiguating comments.
- `render_worker_program` for HOST with non-empty relay schedule: split events at insertion point, call walker::render_worker_events twice (pre + post) with relay block between (mirror of cycle-148 bufsync line 762-784 splice).

### Test surface
- `tests/multi_worker_emit.rs`: REPLACED negative `worker_to_worker_push_is_typed_contract_gap` (cycle 79) with positive `worker_to_worker_push_emits_host_relay` (3-worker synthetic fixture). Asserts host main contains the banner + `__relay.relay_one(0u64, 1usize, 1usize)` for the w1→w2 hop + splice between bar_0 and bar_1; src/dst workers route via peer_idx=0.
- `tests/host_relay_emit.rs`: NEW file mirroring `mp-tcp-bufsync/tests/host_relay_emit.rs` cycle-148 structure. 3 tests on the in-tree 06/distributed2 fixture:
  - `host_emit_includes_task_0327_relay_phase_with_12_hops`: banner + 12 hops + 12 relay_one calls.
  - `host_relay_phase_sits_between_the_two_barrier_rounds`: bar_0.wait < relay marker < bar_1.wait.
  - `non_host_worker_routes_w2w_through_host_peer_only`: w0.rs has 8 Chan::new instances, all with peer_idx=0.
- e2e-matrix.toml: flipped [[skip]] → [[required]] for 06-separable-filter/distributed2 × mp-tcp-event.

### Verification gate (cycle-149 self-run, pre-parallel-review)
- `just check`: clean.
- `just clippy`: clean (-D warnings).
- `just test`: all tests pass (dev profile, includes the 3 new host_relay_emit tests + the rewritten multi_worker_emit test).
- `just test-release`: green after 1 transient flake on `e2e_example_07/naive_pthreads_sync_bit_identical` (re-ran clean — pthreads-sync scratch-dir collision, unrelated to TASK-0327 which touches mp-tcp-event only).
- `just check-textual-replace-on-codegen`: OK.
- `just check-include-str-coverage`: OK.
- `just e2e`: 112/96/0/16/0 (was 112/95/0/17/0; +1 pass -1 skip on the cycle-149 cell). 2 back-to-back samples: bit-identical PASS for 06/distributed2 × mp-tcp-event in both.

### Forward-carry to cycle 150+
- AC#2 fully landed; **AC#3 fully landed**: all 4 tier-1 backends now bit-identical on 06/distributed2 (pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event). TASK-0324 AC#4 also closes here as a consequence — see TASK-0324's cycle-149 final-state addendum.
- Honest sibling carry: the host-mediated barrier mediation gap (TASK-0329 cycle 148, host-excluding `bar_<bid>.wait()` for CTRL barriers excluding host) is still gated. Cycle 149 lifted the DATA arm only — the CTRL barrier-excluding-host arm requires a separate barrier-relay (`bar` shim mediates through host) and is filed forward.
- collect_w2w_pushes recursion into Loop bodies (TASK-0330's mp-tcp-bufsync sibling) — for mp-tcp-event the same flat-emit assumption applies. No in-tree trigger; defensive ContractGap is a future cycle.

## Cycle 149 final state — Done after parallel review gate GO

### What landed (cycle 149, mp-tcp-event slice; closes AC#1+AC#2+AC#3)

**AC#2 mp-tcp-event worker-to-worker via SYNCHRONOUS host-relay** (mirror of cycle-148's mp-tcp-bufsync slice; same heuristic, same splice point, same ordering):

- `Reactor::relay_one(seq, dst_peer, cap)` added to runtime_src.rs: `let p = self.wait(seq); self.push(seq, dst_peer, p, cap);` — bytes-verbatim, bypasses Chan<T> encode/decode.
- `chan_peer_index` lifted at multi_worker.rs: w2w pairs (both src and dst non-host) now resolve `effective_peer = host_worker`, routing through peer_idx=0.
- Plan::build's old w2w-Push rejection (cycle 79, multi_worker.rs:225-237) lifted; the loop now bites only the genuinely-malformed `src == dst == host` projection (the defensive check stayed).
- New RelayHop struct + `collect_w2w_pushes` helper + `relay_phase_insertion_point` heuristic (LAST top-level Sync → first top-level Wait fallback → end-of-events) + Plan::relay_schedule + Plan::render_relay_phase.
- render_worker_program for HOST with non-empty relay: split events at insertion point, walker::render_worker_events twice (pre + post) with relay block between.
- Module-doc updates: lib.rs added point 5 "Host-relay for worker-to-worker Push/Wait"; multi_worker.rs CHECK-ORDER NOTE updated; "Honest limitations" worker-to-worker bullet split DATA (lifted) vs CTRL (still gated on TASK-0329).

**AC#3 06-separable-filter/distributed2 promotion (fourth-backend slice)**:

- e2e-matrix.toml: flipped [[skip]] → [[required]] for distributed2 × mp-tcp-event. AC#3 is now FULLY closed — all 4 tier-1 backends bit-identical.
- e2e baseline shift: 112/95/0/17/0 → **112/96/0/16/0**.
- Both qa-test-runner samples PASS (3.24s and 3.26s); no flake observed on target cell.
- Tests: 882/0/3 (post-cycle-148) → **885/0/3** (+3 host_relay_emit.rs cycle-149 tests).

### Parallel review gate

- **qa-test-runner**: GO. All gates green. `just ci` exit 0 including the 4 negative/determinism arms (cross-backend differential 21/5, required-coverage 1, etc.). 2 e2e samples line-by-line identical pass/skip composition. Target cell PASS in both; no regressions in any other cell; previously-skipped cells stay correctly skipped.
- **mped-architect**: GO with 4 P2 fold-back items + 2 P3 task filings. All P2 folded back in-thread (this commit):
  - **P2.1**: TASK-0330's defensive ContractGap scope amended to cover BOTH mp-tcp-bufsync and mp-tcp-event collect_w2w_pushes Loop-body siblings (the cycle-149 slice inherited the same flat-emit limitation).
  - **P2.2**: e2e-matrix.toml opacity-rot fold-back: 786-791 (05-stencil/distributed-2d × mp-tcp-event) now names TASK-0294 as the remaining blocker (host 2D slice-paste); 1082 (09/pipelined × mp-tcp-event) now names TASK-0329 as the remaining CTRL-arm blocker. Both cite cycle-149 as the DATA-arm lift. Lines 985 and 1143 are accurate-but-narratively-loose (host-excluding barrier only) — deferred to TASK-0331's full audit.
  - **P2.3**: `relay_phase_insertion_point` doc updated to mark constraint #3 as INERT on mp-tcp-event (per-seq demux at the reactor's inbound map removes the bufsync constraint by construction). The fallback is kept for narrative symmetry only.
  - **P2.4**: `Reactor::relay_one` docstring extended with explicit safety invariant: unlike bufsync's `wire::read_msg_expect` seq cross-check, mp-tcp-event's per-seq demux relies on the wire-protocol-v0 globally-unique-seq invariant. A future weakening would surface as a silent wrong-payload race; documented for the defensive check site at Plan::build if needed.
- P3 filings:
  - **TASK-0330** (P2.1+P3.1 amendment): scope extended to both backends; same defensive ContractGap pattern needed in both.
  - **TASK-0331** filed: e2e-matrix.toml skip-reason audit post-cycle-149 (opacity-rot of TASK-0175 citations after DATA-arm lift). Lines 985 + 1143 + any other unaudited TASK-0175-citing entry. Empirical-verification discipline per cycle-119 precedent.

### Honesty gaps disclosed

- **`just test-release` flake on first sample**: `e2e_example_07/naive_pthreads_sync_bit_identical` failed transiently (scratch-dir collision in pthreads-sync e2e test). Re-ran clean on subsequent samples (3 total). UNRELATED to cycle 149 (which touches mp-tcp-event only); pthreads-sync code unchanged this cycle. NOT a regression of this cycle.
- **2-sample non-flake for the 06/distributed2 × mp-tcp-event cell** (qa-test-runner) — meets the cycle-119 milestone-close empirical-verification precedent but does not extend to the rest of the e2e matrix (the broader composition was 2-sample-identical, also non-flake at the macro level).
- **Effort-estimate honesty**: cycle-149's implementation was ~280 LoC pass-side + ~28 LoC runtime-side + ~270 LoC tests = ~580 LoC total in mp-tcp-event. Cycle-148 self-disclosed ~239 LoC pass + ~234 LoC tests for the bufsync slice. Roughly parity. The architect's read-only review caught 4 P2 items in ~250s — comparable to cycle 148.
- **Architect P3.2 NOT empirically validated this cycle**: the 786-791 (05/distributed-2d × mp-tcp-event) prose-update is unverified; TASK-0294 may not be the SOLE remaining blocker. Cycle-149 chose not to empirically promote this cell (would need TASK-0294 to land first). The skip reason update is a best-effort narrative correction.

## TASK-0327 closure

**TASK-0327 status: DONE.**

All 3 ACs landed:
- AC#1 (mp-tcp-bufsync w2w): cycle 148.
- AC#2 (mp-tcp-event w2w): cycle 149.
- AC#3 (06/distributed2 four-backend promotion): cycle 148 (3 of 4) + cycle 149 (4 of 4).

Closes TASK-0324 as a transitive dependency (TASK-0324 AC#4 needed AC#3 = full closure). M5 cross-backend-completeness milestone for the silent-miscompile reproducer now achieved.

### Forward-carried lessons (cycle 149 → future cycles)

1. **Paired-lift discipline (architect P2.1)**: when a defect class spans multiple backends, fixing one + filing a sibling task for the other is the cycle-148→149 precedent. The sibling task should be filed at the END of the first cycle, not waited for the architect to catch. Memory feedback-silent-sibling-defect.md should be updated with this paired-lift hygiene as the 10th firing example.

2. **Opacity-gate-rot fires across narrative surfaces (P2.2 + P3.2)**: when a precise sub-task is split out from a coarse parent (TASK-0175 → TASK-0327 + TASK-0329 in cycles 148-149), every prose surface citing the parent task is potentially rotted. The full grep audit should happen in the SAME cycle that splits the task, not deferred. Memory feedback-opacity-gate-rot.md should be updated.

3. **Per-seq demux is a different safety profile than per-socket FIFO (P2.4)**: mp-tcp-event's reactor naturally isolates seqs into independent queues; bufsync's wire-protocol relies on FIFO ordering. The cycle-149 mp-tcp-event slice inherits a DIFFERENT defensive surface than cycle-148 bufsync's slice. Documenting this is essential — the failure mode under contract violation is also different (silent wrong-payload vs fail-loud seq mismatch). Memory note candidate.
<!-- SECTION:NOTES:END -->

- [ ] #1 AC#1: mp-tcp-bufsync worker-to-worker channel
- [ ] #2 AC#2: mp-tcp-event worker-to-worker channel
- [ ] #3 AC#3: 06-separable-filter/distributed2 promotion
<!-- AC:END -->
