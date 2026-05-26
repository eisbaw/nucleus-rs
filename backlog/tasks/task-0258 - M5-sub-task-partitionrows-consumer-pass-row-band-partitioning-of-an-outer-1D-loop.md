---
id: TASK-0258
title: >-
  M5 sub-task: partition=rows consumer pass (row-band partitioning of an outer
  1D loop)
status: Done
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-23 23:53'
updated_date: '2026-05-26 08:04'
labels:
  - M5
  - compiler
  - partition
dependencies:
  - TASK-0043
  - TASK-0249
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + TASK-0043 AC#1. partition=rows is currently REJECTED at sched-lower as UnsupportedPartitionKind (TASK-0249 cycle 70). M5 needs a real consumer.

## Scope
Add nucleus/nucleus-compiler/src/passes/partition_rows.rs as a sibling to passes/partition_workers.rs. Walks the ACFG, finds Repeat nodes with ResolvedLoopOption::Partition(PartitionKind::Rows), and partitions the OUTER iteration range across the placement's worker set (round-robin band assignment by default).

## Acceptance Criteria
1. partition_rows pass exists; called from passes/mod.rs in the canonical pass order.
2. A 1D outer Repeat with partition=rows on a place-set of N workers gets per-worker row-band ranges in NameSidecar.partition_worker_ranges (same shape partition=workers uses today).
3. partition=rows on a NON-1D loop is rejected at sched-lower as a typed error (UnsupportedPartitionKind or a new variant — matches PRD §6.3.3 'bad combinations rejected at compile time').
4. UnsupportedPartitionKind for Rows is REMOVED from sched-lower (TASK-0249 reject becomes accept-and-route-to-consumer).
5. A new e2e cell exercises partition=rows on examples 5 or 6; bit-identical vs reference.bin on at least one tier-1 backend.

## Open questions
- Round-robin row-band vs strict equal-band assignment for non-divisible row counts. Default: same trailing-partial discipline that block_transform.rs uses (TASK-0218 / TASK-0181).
- Halo inference for stencil examples (5, 6) is TASK-0043 AC#2 — sibling task, not this one.

## Forward-carry from TASK-0249
The reject site at sched/lower.rs::lower_loop_option (the PartitionKind::Rows arm of UnsupportedPartitionKind) must be REMOVED when this consumer lands; otherwise the schedule never reaches the partition_rows pass. Same surgical edit pattern partition_workers used when it landed.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DESCRIPTION CORRECTION + clarification (orchestrator cycle 79b, pre-implementer): the original description said 'row-band partitioning of an outer 1D loop'. PRD §6.3.3 line 519 is explicit: 'partition=rows on a 1D iteration' is a BAD COMBINATION rejected at compile time. partition=rows is specifically for the OUTER of a 2D nest — it row-bands the outer (y) loop, leaving the inner (x) loop intact per worker. This is the original 05-stencil/distributed use case TASK-0249 surfaced (the inert  directive on a 2D y/x nest).

Refined scope for the implementer:
1. partition_rows pass applies ONLY when the partition=rows directive is on the OUTER loop of a 2D nest (Repeat-of-Repeat in the ACFG, on the same worker entity). Reject otherwise at sched-lower OR at the pass entry, with a typed UnsupportedPartitionKindFor1DLoop variant (NOT the existing UnsupportedPartitionKind blanket reject — that becomes too coarse).
2. Semantics: row-band the outer-loop range across the placement workers (same algorithm partition_workers uses for 1D, but applied to the outer of the 2D); inner loop body executes unchanged per worker.
3. Output: NameSidecar.partition_worker_ranges[outer_iv][worker_id] = row_band_range, exactly as partition_workers populates today. No NEW sidecar field needed — transfer_inject + the backend walker already consume partition_worker_ranges and apply per-worker slice handling (host-side gather via render_wait_assign).
4. The reject site at sched/lower.rs::lower_loop_option's PartitionKind::Rows arm: REMOVE the UnsupportedPartitionKind reject for Rows (keep for Blocks2d until TASK-0259 lands). Replace with an accept-and-route-to-consumer arm.
5. The NEW reject site (typed UnsupportedPartitionKindFor1DLoop or similar) fires when partition=rows is applied to a non-outer-of-2D context. Test both negative paths.

This is mostly a 'wire partition=rows through the existing partition_workers infra' task; the heavy lifting (per-worker range -> sidecar -> emit) already exists. Estimated scope: ~150-250 LoC including tests, mostly mechanical.

Halo inference (TASK-0260) is a SIBLING task — partition_rows alone does NOT solve the stencil halo problem; without halo widths, a row-band-partitioned stencil produces wrong output at the band boundaries. Plan ahead: when this task lands and an e2e cell is added, ensure either (a) the cell's algorithm has no halo (the cell verifies partition=rows mechanism only), or (b) the cell SKIPS until TASK-0260 lands. Pure partition=rows without halo will produce incorrect output on stencils — do NOT mark the cell [[required]] until halo inference is also wired.

Implementation Plan (cycle 79c — implementer):

1. NEW pass file: nucleus/nucleus-compiler/src/passes/partition_rows.rs
   - Mirrors partition_workers.rs shape: `apply_partition_rows(&LinkedIR, ACFG) -> Result<ACFG, PartitionRowsError>`.
   - Errors: PartitionRowsError with 4 variants:
       UnknownLoopVar  (linker-invariant)
       NotOuterOf2DNest  (PRD §6.3.3 'partition=rows on 1D iteration': category error)
       NoMultiWorkerBody (mirrors PartitionError)
       NonDivisible      (mirrors PartitionError)
   - Algorithm: walk ACFG, find Repeat with ResolvedLoopOption::Partition(PartitionKind::Rows). Verify body contains an inner Repeat structurally (Repeat-of-Repeat via find_outer_with_inner_repeat helper, peeking through Sequence). Verify that inner-Repeat body's worker union >= 2. Apply row-band slicing (same divisible/round-robin algorithm partition_workers uses on the outer iter_var). Write into ACFG.partition_worker_ranges[outer_iv][worker_id].

2. Wire into nucleus/driver/src/main.rs: import `apply_partition_rows`; call IMMEDIATELY after apply_partition_workers (line 332). Both consume + return ACFG; pure sequential composition.

3. nucleus/nucleus-compiler/src/passes/mod.rs: add `pub mod partition_rows;` next to `partition_workers`.

4. nucleus/nucleus-compiler/src/lib.rs: add `pub use passes::partition_rows::{apply_partition_rows, PartitionRowsError};` next to the partition_workers export.

5. sched-lower change in nucleus/nucleus-compiler/src/sched/lower.rs:1120-1133:
   - Remove the `PartitionKind::Rows` arm from the alternation; only Blocks2d rejects now.
   - Update doc-comment to reflect that Rows now lowers and routes to the partition_rows consumer.
   - Display message in src/sched/ir.rs:816..835 updated: when kind == Rows, this is unreachable (won't fire) but encoded for exhaustiveness — keep the message accurate by keeping the keyword mapping; remove 'rows' from the actionable suggestion (replace with 'omit the directive').

6. Test files:
   (a) nucleus/nucleus-compiler/tests/partition_rows.rs — new file. Mirror partition_workers.rs tests shape:
       - positive: synthetic 2D Repeat-of-Repeat, partition=rows on outer over 4-worker body. Per-worker ranges 0..4, 4..8, 8..12, 12..16 for source range 0..16.
       - negative_1d_iter_rejected: synthetic 1D Repeat (no inner Repeat in body) → NotOuterOf2DNest.
       - negative_single_worker_body: synthetic 2D Repeat-of-Repeat with single-worker body → NoMultiWorkerBody.
       - negative_non_divisible: synthetic 2D Repeat-of-Repeat, range 0..17 across 4 workers → NonDivisible.
       - positive_deterministic_two_runs: byte-identical between two runs (BTreeMap discipline).
   (b) nucleus/nucleus-compiler/tests/sched_lower.rs: 
       - rename existing `negative_partition_rows_is_rejected` → `positive_partition_rows_now_lowers` and flip the assertion: lowers ok, includes ResolvedLoopOption::Partition(PartitionKind::Rows).
       - keep `negative_partition_blocks2d_is_rejected` unchanged (Blocks2d still rejects).
       - keep `positive_partition_workers_still_lowers` unchanged.

7. nucleus/nucleus-compiler/tests/sched_parser.rs + tests/sched_lower.rs: `parses_05_stencil_distributed` and `lowers_05_stencil_distributed` expect count_loops()/loops.len() == 1 today. After restoring the y-directive: count is 2 and the y-loop options include ResolvedLoopOption::Partition(PartitionKind::Rows). Update comment to cite TASK-0258 (consumer landed) instead of TASK-0249 (silent-drop closed).

8. nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc:
   - Re-introduce `loop y : partition=rows;` after the algo's outer y loop.
   - Rewrite header NOTE block: TASK-0249 removed the directive because no consumer existed; TASK-0258 restored it now that partition_rows lands. Cell remains [[skip]] (TASK-0117 / TASK-0042.05 / halo are sibling gates). 
   - Footer note: halo inference (TASK-0260) is the remaining barrier to a bit-identical stencil cell.

9. Update the partition_workers.rs head-comment caveat (the "## Honest limitations" / "**1D partition axis only.**" bullet): 'partition=rows now consumed by passes/partition_rows.rs (TASK-0258)'. Keep `Blocks2d rejects at sched-lower as UnsupportedPartitionKind`.

10. Verification gate (run via nix develop -c just <recipe>):
    a. just test
    b. just clippy (cargo clippy --workspace --all-targets -- -D warnings)
    c. cd nucleus && cargo fmt --check -p nucleus-compiler
    d. just e2e (88/?/0/? — must preserve 0 required-fail and 0 failures)
    e. just determinism-check
    f. just determinism-check-negative
    g. just xbackend-check-negative

11. Commits in 2-3 logical units:
    a. passes/partition_rows.rs + mod.rs + lib.rs export + driver wire-up
    b. sched/lower.rs + sched/ir.rs accept-Rows update + tests update
    c. 05-stencil/distributed.sched.nuc restoration + parser/lower tests update

12. Out of scope (will file follow-ups on commit):
    - Halo inference (TASK-0260 already filed)
    - Stencil e2e cell exercising partition=rows + halo bit-identical to reference.bin (blocked-on TASK-0260)

Cycle 79c implementation complete (commits ef85b99, 5e4acc9).

## Landed

- nucleus-compiler/src/passes/partition_rows.rs (NEW, ~330 LoC incl. tests): the consumer pass. Walks ACFG for Repeat nodes with ResolvedLoopOption::Partition(Rows), verifies outer-of-2D structural pre-condition (Repeat-of-Repeat via find_outer_of_2d + contains_repeat helpers, both 100% covered by 4 #[cfg(test)] unit tests), validates multi-worker body + divisibility, applies the same row-band slicing algorithm partition_workers uses, writes per-(IterVar, WorkerId) ranges into the SHARED ACFG::partition_worker_ranges sidecar (downstream consumers don't distinguish which directive produced the override).
- nucleus-compiler/src/passes/mod.rs: pub mod partition_rows;
- nucleus-compiler/src/lib.rs: pub use passes::partition_rows::{apply_partition_rows, PartitionRowsError};
- nucleus-compiler/src/passes/partition_workers.rs head-comment caveat (the "## Honest limitations" / "**1D partition axis only.**" bullet) updated: 'partition=rows now consumed by partition_rows (TASK-0258)'; Blocks2d remains rejected at sched-lower (TASK-0259).
- nucleus-compiler/src/sched/lower.rs:1109..1133: PartitionKind::Rows arm added to the LoopOption::Partition match (alongside Workers). PartitionKind::Blocks2d remains the only kind rejected. Comment block updated to document the cycle-79c state with full citations.
- nucleus-compiler/src/sched/ir.rs:643..671 / 816..843: UnsupportedPartitionKind docstring + Display message updated — only Blocks2d reaches this variant from the live lower call site; Workers + Rows arms remain in the match for exhaustiveness so any future PartitionKind addition fails to compile.
- nucleus/driver/src/main.rs: import apply_partition_rows + call site IMMEDIATELY after apply_partition_workers (sequential composition).
- nucleus-compiler/tests/partition_rows.rs (NEW, 6 integration tests): outer_of_2d_records_per_worker_row_bands (positive), negative_partition_rows_on_1d_iter_is_rejected, negative_single_worker_body_is_rejected, negative_non_divisible_range_is_rejected, partition_rows_is_deterministic_across_runs, no_directive_is_identity.
- nucleus-compiler/tests/sched_lower.rs: negative_partition_rows_is_rejected → positive_partition_rows_now_lowers (flipped). negative_partition_blocks2d_is_rejected updated to assert the new Display includes 'partition=rows' as a TASK-0258 sibling suggestion. positive_partition_workers_still_lowers unchanged regression guard.
- nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc: header NOTE block rewritten to document TASK-0258 + the divisibility caveat. Directive HELD in commented form (see below).
- backlog/tasks/task-0262 - TASK-0258 follow-up: remainder policy filed (status To Do).

## Gates (run via nix develop -c just <recipe>)

- just test: 700 passed / 0 failed / 3 ignored. Was 690 baseline, +10 new (6 integration + 4 unit). VERIFIED.
- just clippy: clean (-D warnings, --all-targets). VERIFIED.
- just e2e: 88 pass=73 fail=0 skipped=15 required-fail=0. UNCHANGED baseline. VERIFIED.
- just determinism-check: byte-identical across both runs (88 cells). VERIFIED.
- just determinism-check-negative: 73/88 perturbed, correctly bit. OK. VERIFIED.
- just xbackend-check-negative: 16 applied, 1 detected, correctly bit. OK. VERIFIED.

## AC status (per task brief)

- AC#1 (partition_rows pass exists; called from passes/mod.rs in canonical pass order): GREEN.
- AC#2 (synthetic 2D Repeat-of-Repeat with partition=rows on 4-worker place set → per-worker row-band ranges): GREEN. Pinned by outer_of_2d_records_per_worker_row_bands (positive — w0:0..4, w1:4..8, w2:8..12, w3:12..16). Inner iter_var is NOT partitioned (intact per worker) — also pinned in the same test.
- AC#3 (partition=rows on a 1D loop is rejected at sched-lower as a typed error): RESOLVED-DIFFERENTLY, see honest limit below. The check moved from sched-lower to the partition_rows PASS entry (NotOuterOf2DNest). The AST shape needed to verify the outer-of-2D pre-condition is only available after build_acfg, not at sched-lower. Pinned by negative_partition_rows_on_1d_iter_is_rejected.
- AC#4 (UnsupportedPartitionKind for Rows is REMOVED from sched-lower): GREEN. Only Blocks2d remains rejected.
- AC#5 (new e2e cell exercises partition=rows + bit-identical reference.bin on ≥1 tier-1 backend): BLOCKED-ON-TASK-0260 (halo inference). Pure row-band partitioning of a stencil produces wrong output at row-band boundaries — no e2e cell can be bit-identical without halo synthesis. The 05-stencil/distributed schedule directive itself is additionally held back by the 14-not-divisible-by-4 issue (filed as TASK-0262). Honest non-claim per task brief.

## Honest gotchas / surprises

1. **05-stencil divisibility blocker**: the algo y-loop = 1..15 = length 14. 4 workers want length % 4 == 0. partition_rows's first-cut policy refuses to compile. Restoring the directive AS-IS today would make every nucleus build of 05-stencil/distributed fail at compile time. Resolution: hold the directive in commented form with the full citation block + file TASK-0262 (shared remainder policy with partition_workers). Cell remains [[skip]] across all 4 tier-1 backends, so no e2e behaviour change.

2. **AC#3 reject site is the PASS, not sched-lower**: the brief said 'partition=rows on a non-outer-of-2D context: reject at compile time'. At sched-lower the AST has not yet been built into the ACFG, so the 'outer-of-2D' structural check cannot run there — sched-lower only sees the loop directives, not the algo's nest shape. The check naturally lives in partition_rows.rs as PartitionRowsError::NotOuterOf2DNest. This is the spec-correct location; the brief's framing was slightly imprecise. Documented in the pass docstring + the sched/lower.rs comment.

3. **Shared sidecar field**: partition_rows writes into the SAME ACFG::partition_worker_ranges field as partition_workers. Intentional — downstream consumers (sync_inject, petri_to_events, the backend walkers) don't distinguish which directive produced the per-worker range. The 'extra validation' partition=rows adds is captured by the pass entry; downstream IR shape is identical. Disjoint IterVar keys by grammar construction (at most one partition= per loop), so the two passes' order is observationally irrelevant.

4. **No shared helper extraction**: tempting to share the divisible/round-robin slicing math between partition_rows and partition_workers (it's ~6 lines). Deliberately NOT done in this cycle — the two passes diverge in their structural pre-condition and error types; a refactor would touch partition_workers.rs's 9 TASK-0212-pinned tests and make any regression bisect to the consolidation commit rather than to a real bug. A backend-common-style consolidation belongs to a future cleanup task (see memory: project-backend-common-crate).

5. **Repo-wide cargo fmt --check drift**: baseline 146 pre-existing fmt diffs across files I touched. My new partition_rows.rs (source + test) is clean per rustfmt. The driver/src/main.rs change went through rustfmt. The pre-existing drift in lib.rs / sched/lower.rs / sched/ir.rs / sched_lower.rs / partition_workers.rs is unchanged by this commit — those are existing issues to be addressed by a fmt-sweep task, not this one.

## Forward-carried lessons (appended to siblings)

- TASK-0259 (partition_blocks2d): the partition_rows pattern (typed errors + structural pre-condition check at pass entry, NOT sched-lower; reuse ACFG::partition_worker_ranges; PartitionRowsError-style error variants) is the template. Blocks2d's structural pre-condition is 'outer pair of a 2D nest' (likely Repeat-of-Repeat with both iter vars partitioned). UnsupportedPartitionKind currently only fires for Blocks2d — when this task lands, remove that variant entirely or document it as exhaustiveness-only.
- TASK-0260 (halo inference): the partition_rows pass writes into the shared partition_worker_ranges sidecar. Halo inference needs to consult that sidecar to know which workers OWN which row-bands and which neighbours need halo transfer synthesis. Coordinate via partition_worker_ranges; the sidecar key set (BTreeMap<IterVar, BTreeMap<WorkerId, Range<i64>>>) is the natural interface.
- TASK-0261 (reuse): orthogonal to this task; the schedule grammar's 'reuse' directive needs its own consumer. Not blocked on TASK-0258 / TASK-0262.
- TASK-0262 (remainder policy): both partition_rows and partition_workers must adopt the same policy. Pick the policy in coordination with block_transform's trailing-partial discipline (TASK-0142 / TASK-0218); same sidecar field consumer-side, same harm-class first-cut limit.

## Disposition

Status remains In Progress. AC#5 (new e2e cell exercising partition=rows + bit-identical reference.bin on ≥1 tier-1 backend) is BLOCKED-ON-TASK-0260 (halo inference) AND BLOCKED-ON-TASK-0262 (remainder policy for 05-stencil/distributed). The other 4 ACs are GREEN:

- AC#1 GREEN (pass exists, wired in passes/mod.rs canonical order)
- AC#2 GREEN (synthetic 2D Repeat-of-Repeat produces per-worker row-bands)
- AC#3 GREEN-WITH-CORRECTION (partition=rows on 1D is rejected, but at the partition_rows pass entry — NotOuterOf2DNest — not at sched-lower; sched-lower has no AST shape information to do the check there. Documented in code + commit message + this notes block.)
- AC#4 GREEN (UnsupportedPartitionKind for Rows removed from sched-lower; only Blocks2d still rejects.)
- AC#5 BLOCKED (joint deliverable with TASK-0260; documented above)

The mechanism (partition=rows lowers + the pass writes correct per-worker row-bands + the sidecar surface integrates with all downstream consumers) is COMPLETE and pinned by tests. The 'bit-identical e2e cell' that would close AC#5 cannot land until halo inference (TASK-0260) is wired AND the remainder policy (TASK-0262) handles the 05-stencil row-count.

Per implementer-contract rule 5 ('mark Done only if every AC is genuinely met'), this task stays In Progress until TASK-0260 + TASK-0262 land and the joint e2e cell becomes bit-identical. When that happens, this task can be closed with a final-summary referencing those commits.

REVIEW-GATE LANDED (cycle 79c orchestrator hardening, commit 042565f).

Parallel read-only review of cycle-79c implementer commits (ef85b99 + 5e4acc9 + 3b0310e) returned GO from both qa-test-runner and mped-architect. Two P2 doc-honesty findings applied in-thread; the codegen/pass work is unchanged.

## In-thread fixes (commit 042565f)

F1 (architect): docstring line 43 claimed '~6 lines of arithmetic' shared with partition_workers. Actual byte-identical duplication is ~24 LoC (6-line row-band math + 18-line collect_op_workers helper). Corrected; named the 3-way-warning for TASK-0259 + TASK-0244 follow-up consolidation.

F2 (architect): docstring line 23 said 'Repeat-of-Repeat on the same worker entity', but code (collect_op_workers + inner-body union) does NOT enforce same-worker. Corrected: row-band is sliced across the inner body's worker union; same-worker is typical-not-pinned.

## Gate (post-hardening, this cycle)

- cargo test nucleus-compiler: 527 / 0.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 88 / 73 / 0 / 15 / 0 required-fail (preserved; behaviour-unchanged hardening).
- Workspace test count post-cycle: 700 / 0 / 3 (claimed + measured by qa-test-runner; +10 vs baseline 690 — 4 unit + 6 integration tests pinning positive + 4 negative arms).
- just determinism-check + determinism-check-negative + xbackend-check-negative: all PASS / bite correctly. 2x non-flaky on e2e + determinism.

## P2 forward-carry findings (NOT closed in this cycle)

- Test gap: sibling Repeat in the same enclosing Sequence is unpinned (find_outer_of_2d would correctly route, but no test). 3D Repeat-of-Repeat-of-Repeat behaviour is implicit 'first match wins' but unpinned. Mixed worker entities between outer and inner is currently accepted via the inner-body worker-union route. These are forward-carry to TASK-0259's structural-check tests — if Blocks2d needs to differentiate any of these, file explicit pinning tests in BOTH passes.
- ~24 LoC partition_workers/partition_rows duplication: explicitly DEFERRED to the TASK-0244 backend-common-style consolidation, with the 3-way warning embedded in TASK-0259 forward-carry.
- collect_op_workers helper is currently byte-identical across passes/partition_workers.rs and passes/partition_rows.rs. When TASK-0259 lands and the duplication becomes 3-way, the lift to a shared passes/partition_common.rs becomes warranted.

## Review-gate decision

Status stays In Progress. The codegen/pass WORK is COMPLETE and review-GO. Of the 5 ACs:
- AC#1 (pass exists, in canonical pass order): GREEN.
- AC#2 (synthetic 2D Repeat-of-Repeat with partition=rows → per-worker row-band ranges): GREEN.
- AC#3 (partition=rows on non-outer-of-2D → typed error): GREEN-WITH-CORRECTION (located at pass entry, not sched-lower — sched-lower has no algo-nest visibility per lower_sched signature; correction documented).
- AC#4 (UnsupportedPartitionKind for Rows REMOVED from sched-lower): GREEN.
- AC#5 (new e2e cell exercising partition=rows + bit-identical reference.bin): BLOCKED-NOT-FAILED on TASK-0260 (halo inference: stencil cells produce wrong output at row-band boundaries without halo synthesis) AND TASK-0262 (remainder policy: the existing 05-stencil/distributed 14-row range is non-divisible by 4 workers).

Honest reading: AC#5 cannot close until BOTH TASK-0260 (halo) AND TASK-0262 (remainder policy) land. Same closure-deferred-on-sibling-blocker pattern as TASK-0042.05's AC#2/AC#4 (blocked on TASK-0175). When those land in lockstep, the 05-stencil/distributed [[skip]] cell promotes to [[required]] with bit-identical reference.bin, closing AC#5 + TASK-0258 simultaneously.

## Cycle 168 closure audit — all 5 ACs MET (orchestrator-direct, tracker-only)

Sibling-blockers TASK-0260 (halo inference, closes cycle 168) and TASK-0262 (remainder policy, Done) both lift. Re-verified each AC:

### AC#1 (partition_rows pass exists; called from passes/mod.rs) ✓ MET cycle 79c
### AC#2 (2D outer Repeat with partition=rows → per-worker row-bands) ✓ MET cycle 79c (positive_outer_of_2d_records_per_worker_row_bands)
### AC#3 (partition=rows on non-outer-of-2D context REJECTED with typed error) ✓ MET cycle 79c via PartitionRowsError::NotOuterOf2DNest at pass entry (correction from "at sched-lower" — the AST shape needed is only available post-build_acfg; documented in pass docstring + sched/lower.rs comment)
### AC#4 (UnsupportedPartitionKind for Rows REMOVED from sched-lower) ✓ MET cycle 79c
### AC#5 (new e2e cell exercises partition=rows + bit-identical on ≥1 tier-1 backend) ✓ MET via 06-separable-filter/distributed (uses partition=rows on outer hy axis per cycle-116 / TASK-0296). All 4 tier-1 backends PASS bit-identical against 06-separable-filter/reference.bin in cycle-168 gate (112/102/0/10/0).

### Closing this task
The partition=rows consumer pass landed cycle 79c with full positive + negative test coverage. The Stage-1 brief framed AC#5 as blocked-on-TASK-0260; with halo inference + transfer_inject consumer landed (TASK-0260 + TASK-0263, both closing cycle 168), the 06-separable-filter/distributed e2e cell exercises partition=rows AND the halo machinery AND bit-identical to reference. Closing per honest-failure discipline applied positively.

Gate at closure: e2e 112/102/0/10/0. No source change this cycle (tracker-only).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 168 closure (orchestrator-direct, tracker-only). All 5 ACs MET. AC#1-AC#4 cycle 79c (partition_rows pass + pass-entry reject + sched-lower lift). AC#5 via 06-separable-filter/distributed (partition=rows on hy, cycle 116/TASK-0296) — all 4 tier-1 backends PASS bit-identical against reference.bin. Cycle-168 gate: e2e 112/102/0/10/0.
<!-- SECTION:FINAL_SUMMARY:END -->
