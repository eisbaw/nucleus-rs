---
id: TASK-0212
title: 'Loop-bound rewrite under partition=workers: each worker iterates its slice'
status: In Progress
assignee:
  - '@claude'
created_date: '2026-05-20 22:07'
updated_date: '2026-05-21 05:29'
labels:
  - compiler
  - ir
  - partition
  - M3
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced during TASK-0211 cycle-2 STOP-AND-REPORT (mped-architect 2026-05-21).

The schedule directive `loop n : partition=workers;` on a loop whose
body is placed on a distributed entity (e.g. `place k on {w0,w1,w2,w3}`)
is currently a parse-and-lower-and-drop: parsed in
compiler/src/sched/parser.rs:575 as PartitionKind::Workers, lowered in
compiler/src/sched/lower.rs:874 to ResolvedLoopOption::Partition, then
NEVER consumed by any downstream pass. Only ResolvedLoopOption::Block
has a consumer today (compiler/src/passes/block_transform.rs:207).

Consequence (verified by inspecting the generated main.rs for
13-cnn-inference/batch_parallel/pthreads-sync): every compute worker
iterates the full source range (n in 0..16 for the CNN), redundantly
firing all kernels for the whole batch. The "each worker processes
B/N samples" semantic the batch_parallel schedule's comment promises
is not realised. The current output happens to be a correct full
result because (a) all four workers compute the same full result and
(b) the single Push back to host (canonical-first dst) happens to be
worker w0's full output. Both are fragile coincidences, not design.

Spec: a pass (peer to passes/block_transform.rs; could live as
passes/partition_workers.rs) walks the ACFG, finds ACFGNode::Repeat
nodes whose schedule directive carries
ResolvedLoopOption::Partition(PartitionKind::Workers) AND whose body
contains Operations placed on a multi-worker entity, then rewrites the
Repeat's range from a shared source range into a per-worker slice.
The natural implementation: have the projection
(petri_to_events::walk on Repeat) consume a per-worker range override
keyed by (WorkerId, IterVar), so each worker's Event::Loop carries its
own (lo, hi). NameSidecar::loop_bounds becomes per-worker for these
iter vars, or a sibling per-worker-loop-bounds map is added.

Honest scope:
- Divisibility: B/N exact split is simplest first cut. Non-divisible
  splits (e.g. 17 samples across 4 workers) need a tail-handling
  policy - file as a follow-up of this task, not as a blocker for
  the divisible first cut (16/4 in example 13, 4/4 in 03-reduction
  distributed).
- 1D partition axis only. partition=blocks2d / partition=rows are
  orthogonal grammar that need their own passes; this task is the
  partition=workers slice.
- Must compose with TASK-0117 (transfer-injection fan-out): once
  per-worker ranges land, the Push/Wait pairs need to carry the
  correct per-worker IterTile. The transfer_inject docs (lines
  78-84) already point at this future-partition pass.

Out of scope:
- TASK-0117 itself (transfer fan-out; that is the sibling task).
- mp-tcp-bufsync host-excluding barrier (TASK-0175).
- pipeline_parallel / async transfers (TASK-0210).

Acceptance criteria:
1. PartitionKind::Workers on a Repeat is consumed: each compute
   worker's Event::Loop carries its own per-worker range
   (range.start, range.end) such that the union of all workers'
   ranges covers the source range exactly once (B/N exact case).
2. petri_to_events emit walks the per-worker range when projecting
   the Repeat for that worker.
3. New unit test in nucleus/compiler/tests/ exercises a synthetic
   partition=workers loop with 4-element range across 2 workers and
   asserts each worker's projected Event::Loop has the correct
   (lo, hi).
4. No regression on 01..07 / 02-split / 03-reduction-naive cells
   (which use no partition=workers).
5. Composes with TASK-0117 so 13-cnn-inference/batch_parallel/
   pthreads-sync becomes byte-identical to reference.bin (closes
   TASK-0211 AC#4 jointly).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-1 IMPLEMENT (claude, 2026-05-21)

Landed: per-worker loop-bound rewrite for `partition=workers` via a new
sidecar-populating pass plus a small projection-time / codegen-time
consumer change. Each compute worker now iterates its own exclusive
batch slice; the headline B=16/N=4 shape is asserted as a unit test
(`compiler/tests/partition_workers.rs::cnn_batch_parallel_shape_b16_n4`).

### Architecture (file:line citations)

1. NEW ACFG sidecar map: `compiler/src/acfg.rs:599..621`
   - `ACFG::partition_worker_ranges: BTreeMap<IterVar, BTreeMap<WorkerId, Range<i64>>>`
   - Deterministic by id; serde-default; mirrored from the
     `inner_block_iter_vars` design (TASK-0143) for the same reason
     (keep `ACFGNode::Repeat` payload stable, single consumer site).

2. NEW pass: `compiler/src/passes/partition_workers.rs`
   - `pub fn apply_partition_workers(&LinkedIR, ACFG) -> Result<ACFG, PartitionError>`
   - Reads `linked.sched.loops[*]` for
     `ResolvedLoopOption::Partition(PartitionKind::Workers)`.
   - Finds each target Repeat's body-worker union and source range.
   - Validates and commits per-worker exclusive slices to the sidecar.
   - Wired into `driver/src/main.rs:309..317` between block_transform
     and sync_inject.

3. Projection consumes the sidecar:
   `compiler/src/passes/petri_to_events.rs:231..312`
   - `walk` now threads `partition_ranges` through recursion.
   - Per-worker `Event::Loop.range` falls back to source range when
     this worker is not listed in the override (host, etc.).

4. NameSidecar mirror: `compiler/src/sidecar.rs:181..212`
   - Same shape as the ACFG sidecar; populated verbatim in
     `build_sidecar`.
   - Why a sidecar field on NameSidecar and not e.g. eviction of the
     symbolic `loop_bounds` entry for partitioned vars: `loop_bounds`
     is per-IterVar GLOBALLY, but partition is per-(IterVar, WorkerId).
     Host (non-participating) still wants the symbolic source-form
     bound; only compute workers want the concrete per-worker slice.

5. Backend consumer changes:
   - `nucleus/backends/pthreads-sync/src/multi_worker.rs:438..476` —
     `render_worker_events` now takes `worker: WorkerId`; the
     `Event::Loop` arm consults
     `sidecar.partition_worker_ranges` first.
   - `nucleus/backends/mp-tcp-bufsync/src/lib.rs:646..698` — same
     change; backend already had `worker` in scope.
   - Precedence: per-worker partition slice (concrete literal) >
     symbolic source bound from `loop_bounds` > concrete folded
     `Event::Loop.range`.

### Verification

Verified by regenerating 13/batch_parallel/pthreads-sync:
  for n in (0_i64)..(4_i64)    // w0 body
  for n in (4_i64)..(8_i64)    // w1 body
  for n in (8_i64)..(12_i64)   // w2 body
  for n in (12_i64)..(16_i64)  // w3 body
  for n in (0_i64)..(16_i64)   // host (sends output)

(versus pre-TASK-0212: every worker had `for n in (0)..(16)`.)

### AC status

- AC#1 (PartitionKind::Workers consumed; union covers source range
  exactly once for B/N exact case): GREEN. Sidecar populated; per-
  worker projection emits the slice; the synthetic 4-element-over-2-
  workers test (`projection_honours_per_worker_range_two_workers`)
  pins the union.
- AC#2 (petri_to_events emits per-worker range): GREEN.
  Implementation lives in `walk`'s Repeat arm; pinned by
  `cnn_batch_parallel_projects_b_over_n_per_worker`.
- AC#3 (unit test on synthetic partition=workers loop): GREEN.
  `compiler/tests/partition_workers.rs` has 8 tests including the
  required AC#3 shape.
- AC#4 (no regression on 01..07 / 02-split / 03-reduction-naive
  cells): GREEN. Existing e2e gate at 36/28/0/8/0 unchanged;
  determinism gate byte-identical across two runs.
- AC#5 (composes with TASK-0117 so 13/batch_parallel/pthreads-sync
  byte-identical to reference.bin, closes TASK-0211 AC#4 jointly):
  HONEST-PARTIAL. The loop-bound rewrite lands correctly, but
  cargo-build still fails E0425 in w1/w2/w3 because only w0 receives
  the `input` slot. That is the TASK-0117 (transfer-injection fan-
  out) gap, NOT a partition-pass gap. Filed for the next cycle.

### Honest limits / non-divisible policy

- First cut: exact-divisible only. `(hi - lo) % N != 0` reports
  `PartitionError::NonDivisible` and refuses to compile (verified by
  `non_divisible_range_is_rejected` unit test). A remainder-policy
  follow-up is the task description's filed follow-up.
- 1D partition axis only. `partition=rows` / `partition=blocks2d` are
  separate grammars handled by sibling passes (not yet filed).
- No `block=` interaction. A `block=N, partition=workers` combination
  on the same loop would partition the strip-mined inner loop, not
  the outer source iteration. None of the in-tree schedules combines
  the two, so this is a documented gap not a live bug.
- Multi-worker `Event::Loop.range` rebinding for blocked schedules
  (TASK-0181) is unchanged and still fails LOUD as before.

### Determinism

Sidecar key sets are `BTreeMap<IterVar, BTreeMap<WorkerId, Range<i64>>>`,
both keyed by numeric id; iteration is in numeric order. No HashMap
or HashSet on the pass or projection paths. `determinism-check` ran
twice, byte-identical both runs. `determinism-check-negative` and
`xbackend-check-negative` both bit on injected nondeterminism /
corruption.

### Gate measurements (7-step)

1. `nix develop -c just test`           — 479 passed; 0 failed; 2 ignored.
2. `cargo clippy --workspace --all-targets -- -D warnings` — clean.
3. `nix develop -c just e2e`            — 36 / 28 / 0 / 8 / 0 (unchanged).
4. `nix develop -c just determinism-check` ×2 — byte-identical both runs.
5. `nix develop -c just determinism-check-negative` — 28/36 perturbed; OK.
6. `nix develop -c just xbackend-check-negative` — 14 corrupted, 1 detected; OK.
7. `nix develop -c just ci` — exit 0.

### NOT marked Done

Marked Done = false. AC#5 is the remaining acceptance criterion and
its blocker is TASK-0117 (transfer-injection fan-out), filed as
sibling. Per task brief "HONEST-PARTIAL if the rewrite lands cleanly
but requires TASK-0117 to actually be observable in cargo-build".
TASK-0212 stays In Progress until TASK-0117 lands and the e2e cell
moves [[skip]] → [[required]] byte-identical to reference.bin.

ORCHESTRATOR review-gate cycle (post-6df9058):

Both reviewers returned GO with 3 minor non-blocking findings.

qa-test-runner: all 7 gate numbers reproduce (test 479/0/2, clippy clean, e2e 36/28/0/8/0 unchanged, det-check x2 byte-identical, canaries bite, ci exit 0). All 10 new tests pass. Driver-level probe of example 13 batch_parallel × pthreads-sync confirms per-worker bounds rewrite is exactly correct: w0=0..4, w1=4..8, w2=8..12, w3=12..16, host=0..16 (non-participating worker falls back to source range per the documented contract). Cargo build still fails E0425 — exactly as expected (AC#5 honest-partial blocker is TASK-0117 transfer-injection fan-out).

mped-architect: GO with 3 minor non-blocking findings:
1. Cross-backend precedence rule comment in mp-tcp-bufsync says "see pthreads-sync multi_worker for the precedence rationale" — anchors the contract but risks doc-rot if the rules diverge later. Worth a follow-up to consolidate into a shared helper.
2. Coverage gap: PartitionError::NoMultiWorkerBody and PartitionError::UnknownLoopVar lack tests. Fail-closed guards on linker invariants but worth regression tests.
3. Doc seam: e2e-matrix skip reason cited TASK-0211 while task notes cite TASK-0117 as AC#5 blocker. Same underlying defect (transfer-injection fan-out), two task IDs.

Finding 3 fixed in-thread: e2e-matrix.toml skip reasons rewritten to cite TASK-0117 (upstream root cause) as the actual blocker, with TASK-0211 noted as the symptom. Per cycle-10 lesson: prefer citing upstream root over downstream symptom.

Findings 1 and 2 deferred — neither blocks; the precedence-rule consolidation is a hygiene follow-up best paired with a future codegen refactor; the missing-variant tests are guards-on-guards (linker pre-rejects the cases that would trigger them).

CASCADE-CLASS METHODOLOGY-TRANSFER SCORECARD now extends to deep-pipeline cycles (6-for-6 with TASK-0212 making it 6):
- TASK-0092 cycle-3 (AlgoIR lowering 5x closure)
- TASK-0087 cycle-4 (sched-parser n+2 measurement)
- TASK-0200 cycles 1+2+review (sched-lowering Path-2)
- TASK-0204 (broadened K×L fixture)
- TASK-0207+review-sweep (algo for-body constant-2)
- TASK-0199 cycles 1+2+review (parser brace-balanced recovery)
- TASK-0205 (for-body undercount closure)
- TASK-0206 (cascade-aware duplicate)
- TASK-0203 (poisoned-kernel test)
- TASK-0202 (multi-error line:col)
- TASK-0209 cycles 1+2 (backend sub-array codegen)
- TASK-0053 cycle-2 (CNN inference cross-backend bit-identical)
- TASK-0212 cycle-1 (partition=workers loop-bound rewrite — DEEPEST PIPELINE CYCLE)

The cycle-3 methodology (parametric measurement + independent reviewer probes + comprehensive doc-sweep + honest-partial discipline) has transferred from cascade-class lowering work to deep-pipeline cycles cleanly. TASK-0212's 8 integration tests + 2 unit tests with exact-shape assertions on per-worker ranges (not tautological) are the methodological signature.

TASK-0212 cycle-1 disposition: per-worker bound rewrite COMPLETE; cargo-build observation requires TASK-0117. Status stays In Progress with Dependencies: TASK-0117 (AC#5 unblocks once TASK-0117 lands).
<!-- SECTION:NOTES:END -->
