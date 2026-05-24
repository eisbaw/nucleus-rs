---
id: TASK-0289
title: >-
  M5 Stage 3 follow-up: halo-strip Push/Wait synthesis under partition=blocks2d
  + bit-identical e2e cell (TASK-0264 AC#3 + AC#4)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-24 19:58'
updated_date: '2026-05-24 21:17'
labels:
  - M5
  - compiler
  - halo
  - partition
  - stage-3
  - forward-carried-from-TASK-0264
dependencies:
  - TASK-0264
  - TASK-0260
  - TASK-0263
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0264 cycle 113 landed AC#1+2 (sidecar plumbing): ACFG.partition_pairs + ACFG.grid_shape_for_outer_iv + their NameSidecar mirrors are populated by apply_partition_blocks2d. This task lands AC#3 (cross-worker halo-strip Push/Wait synthesis between neighbours in the 2D grid) and AC#4 (new bit-identical e2e cell on a 2D-partitioned stencil).

## Pre-recorded design picks (from TASK-0264 cycle 113)
- pairing recovery: sidecar.partition_pairs.get(outer_iv) returns Some(inner_iv) iff the iv-scope was partitioned by a single blocks2d directive. No re-derivation needed.
- worker -> (row, col) inversion: i = body_workers.iter().position(|w| *w == worker).unwrap(); (row, col) = (i / cols, i % cols) where (rows, cols) = sidecar.grid_shape_for_outer_iv.get(outer_iv).unwrap(). body_workers iteration is BTreeSet numeric order, matching partition_blocks2d's row-major assignment.

## Acceptance criteria
1. transfer_inject (or a new pass — pick consciously) reads sidecar.partition_pairs + sidecar.grid_shape_for_outer_iv + sidecar.halo_widths and synthesises cross-worker Push/Wait pairs for the N/S/E/W neighbour cells in the 2D worker grid. Each non-edge cell gets up to 4 halo-strip transfers (one per cardinal direction); edge cells get fewer. Corner cells (NE/NW/SE/SW) are NOT included in the first cut — they are out-of-scope per TASK-0264's task brief.
2. New e2e cell: a 2D-divisible stencil (likely a new example or a new schedule on 05-stencil with a 2x2-grid-divisible image dimension) tagged 05-stencil/distributed-2d × pthreads-async, bit-identical to a hand-written reference oracle.
3. Existing 05-stencil/distributed × pthreads-async × pthreads-sync × mp-tcp-bufsync × mp-tcp-event matrix cells must remain GREEN — the new halo-strip synthesis fires iff the iv is in sidecar.partition_pairs, which is empty for every shipped schedule pre-cycle-113. Additive-only.
4. e2e baseline at least 93/80/0/13/0 (the new cell adds +1 to total + +1 to pass).
5. just determinism-check stays green on every cell.

## Dependencies
- TASK-0264 (Done AC#1+2; this task closes AC#3+4 and lets TASK-0264 mark Done).
- TASK-0260 (halo inference Stage 1 — Done).
- TASK-0263 (transfer_inject extends per-tile transfer ranges by halo widths — Done in cycle 83; the AC#1 work would EXTEND that pass with the new halo-strip Push/Wait synthesis OR live in a new sibling pass — design pick deferred until the implementer surveys the existing extension surface).

## Honest scope
- DEEP work. Realistically a 2-3-cycle task: (a) pass extension or new pass with the neighbour-resolution + Push/Wait synthesis, (b) new schedule + new example or modified image dimensions to get 2D-divisibility, (c) implementer / review-hardening loop.
- Cycle 80 architect P2 forward-carry: TASK-0260 halo inference is partition-agnostic. The pairing + grid-shape sidecars introduced in cycle 113 are the load-bearing input to AC#3 — without them the consumer couldn't disambiguate paired-by-blocks2d ivs from independent partition=rows ivs. That decoupling is done.
- Mp-tcp-bufsync + mp-tcp-event are likely SKIP for the new 2D cell on the same w↔w-mesh basis they SKIP today's 05-stencil/distributed cell (TASK-0175); pthreads-sync + pthreads-async are the bit-identical targets for AC#2.

## Forward-carried lessons from TASK-0264 cycle 113
- Adding new ACFG / NameSidecar fields touches every pass that does destructure-and-rebuild (8 files in this codebase) PLUS every hand-built ACFG instance in tests (~14 sites). The compiler enforces this via E0063 missing-field, so the work surface is greppable but verbose. Use replace_all with a unique trailing-field pattern (reuse_widths -> reuse_widths\n + new fields).
- build_sidecar in nucleus-compiler/src/sidecar.rs is the single mirror site — add the .clone() forwarding for both new ACFG fields there.
- partition_blocks2d.rs is the ONLY populator of partition_pairs + grid_shape_for_outer_iv; the other 6 passes forward verbatim. Mirror that pattern when extending — keep the writer single-source.
- The sidecar serde round-trip + missing-field-default test (sidecar_partition_blocks2d.rs) is the wire-shape pin. Mirror that template if a future cycle adds another additive sidecar field.

## Cross-reference
- ACFG fields: nucleus/nucleus-compiler/src/acfg.rs (partition_pairs, grid_shape_for_outer_iv — added cycle 113).
- NameSidecar fields: nucleus/nucleus-compiler/src/sidecar.rs (same names).
- Writer: nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs (apply_partition_blocks2d).
- Mirror: nucleus/nucleus-compiler/src/sidecar.rs::build_sidecar.
- Tests pinning the writer + wire shape: nucleus/nucleus-compiler/tests/partition_blocks2d.rs + nucleus/nucleus-compiler/tests/sidecar_partition_blocks2d.rs.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Scope this cycle (cycle 114a): AC#1 + AC#3 only

AC#2 + AC#4 (the new bit-identical e2e cell) is split off as **TASK-0290**. Reasoning: the synthesis pass + its tests is one logical unit, big enough that bundling a new 2D-divisible image + reference-oracle crate + matrix wiring in the same cycle is overscope. Cycle B (TASK-0290) is the integration confidence on top.

## Work plan (this cycle)

### Step 1 — design pick: extend transfer_inject vs new pass

Insertion point for the new consumer is already pre-marked in `transfer_inject.rs` near line 240 — the comment `// TASK-0289 halo-strip Push/Wait synthesis will be the first consumer.` lives right where partition_pairs + grid_shape_for_outer_iv are destructured.

Two viable shapes:

(a) **Extend transfer_inject** — add a new internal function (sibling of `extend_xfer_tiles_for_halo`) that runs AFTER `rewrite_partition_tiles` and AFTER the halo extension, and INJECTS new XferPlaceholders for cross-worker halo strips before the splice→Push/Wait conversion. Smaller blast radius; composes with existing splice machinery.

(b) **New pass after transfer_inject** — runs on the post-Push/Wait ACFG, walks for partitioned outer Repeats, and synthesises new ACFGNode::Xfer nodes directly. Cleaner separation but requires understanding splice_pushes_global's invariants and re-deriving the seq/tile shape.

**Pick (a)** — the splice machinery is the single point that converts XferPlaceholder → (Push, Wait, Xfer); reusing it means the new halo-strip transfers inherit hoisting, partition-tile rewriting, and pipeline-depth annotation for free. Build new XferPlaceholders BEFORE splice, let splice handle them.

### Step 2 — synthesis logic

Per (outer_iv, inner_iv) in `partition_pairs` (where `inner_iv = partition_pairs[outer_iv]`):
- recover `(grid_rows, grid_cols) = grid_shape_for_outer_iv[outer_iv]`
- recover `body_workers` from `partition_worker_ranges[outer_iv].keys()` (BTreeSet → deterministic numeric order)
- for each `(i, w)` in `body_workers.enumerate()`:
  - `(row, col) = (i / grid_cols, i % grid_cols)`
  - identify N/S/E/W neighbours: `(row-1, col)`, `(row+1, col)`, `(row, col-1)`, `(row, col+1)` — skipping edges
  - for each neighbour: synthesise an XferPlaceholder (`data_id` = the halo-bearing data symbol; `src/dst = w/neighbour`; `tile = halo-strip range`)
- corner cells excluded per task brief

### Step 3 — tile range for the halo strip

For a worker at `(row, col)` with y-band `[y_lo, y_hi)`, x-band `[x_lo, x_hi)`, and halo width H (the per-(consumer, iv) halo from `halo_widths`):
- N-strip (from `(row-1, col)`): y ∈ `[y_lo - H, y_lo)`, x ∈ `[x_lo, x_hi)` — i.e. the upstairs neighbour's bottom H rows.
- S-strip (from `(row+1, col)`): y ∈ `[y_hi, y_hi + H)`, x ∈ `[x_lo, x_hi)`.
- E/W-strips symmetric.

Note: TASK-0263's `extend_xfer_tiles_for_halo` already extends the host→worker transfer's tile to cover the halo from the SOURCE side. The new cross-worker strip transfers are a SEPARATE pair (worker→worker) for the halo data, fired after the body's loop boundary.

### Step 4 — tests

- New unit/integration test `tests/halo_strip_synth.rs` that mirrors `tests/partition_blocks2d.rs`: hand-built 2D ACFG, populates `partition_pairs` + `grid_shape_for_outer_iv` + `halo_widths`, runs `inject_transfers`, asserts the expected (src, dst, data) Push/Wait pairs are emitted for a 2x2 grid with halo=1 (4 inner cells get 0, 4 edge cells get 1-2, corners get 0-2 — actually for 2x2 EVERY cell is a corner; pick 3x3 with center inner cell to exercise the 4-direction emit).
- Pin determinism: two runs of the pass produce byte-identical ACFG.

### Step 5 — additive-only proof (AC#3)

Every shipped schedule today has empty `partition_pairs` (verified by `tests/sidecar_partition_blocks2d.rs::shipped_examples_without_blocks2d_leave_maps_empty`). The new synthesis logic MUST short-circuit on empty partition_pairs. Add an explicit `if partition_pairs.is_empty() { return ... }` guard at the top of the new function. Run `just e2e` after the change to verify 92/79/0/13/0 baseline holds.

### Step 6 — gate + commit + tracker

- `just build && just clippy && just test && just e2e`
- per-AC notes + final summary
- file gotchas as forward-carried notes on TASK-0290

## Honest limits

- This cycle does NOT exercise the new synthesis on a real fixture — that's TASK-0290's job. Confidence comes from the unit test + the additive-only guard. A subtle bug in the synthesis logic (off-by-one on tile range, wrong worker pairing on non-square grids) could land here and only surface in TASK-0290.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 114a (2026-05-24) — AC#1 + AC#3 landed

### Commit
f8d58ea transfer_inject: TASK-0289 cycle 114a — halo-strip Push/Wait synthesis (AC#1+3)

### File changes
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs — added inject_halo_strip_xfers (sibling of extend_xfer_tiles_for_halo) + prepend_strip_pairs helper. Plumbed into the finalisation chain AFTER rewrite_partition_tiles + extend_xfer_tiles_for_halo (so strip tiles are not clobbered by either pass). Total +474 lines (impl + docs).
- nucleus/nucleus-compiler/tests/halo_strip_synth.rs — 5 tests pinning: 3x3 grid per-worker pair counts (corner=2, edge=3, center=4) + center-cell exact strip tiles for N/S/W/E; 2x2 grid corner shapes; AC#3 empty-partition_pairs ⇒ zero Xfers; determinism across runs.

### Gate (all green)
- just build: OK
- just clippy (-D warnings): OK
- just test: all OK (no regressions; existing idempotence test stays green via AC#3 short-circuit)
- just e2e: 92/79/0/13/0 (baseline UNCHANGED — AC#3 additive-only contract holds)
- just determinism-check: 92/79/0/13/0 (no determinism regression)

### Per-AC status this cycle
- AC#1: DONE (synthesis pass + unit tests landed)
- AC#3: DONE (additive-only contract pinned by empty_partition_pairs_emits_zero_halo_strip_xfers + e2e baseline unchanged)
- AC#2: DEFERRED to TASK-0290 (new bit-identical e2e cell + hand-written reference oracle)
- AC#4: DEFERRED to TASK-0290 (baseline bump to 93/80/0/13/0)
- AC#5 (determinism): inherits via the unit test + the existing determinism-check infrastructure; new cell-specific determinism comes with TASK-0290

### Subtleties + gotchas

1. **Where the synthesis runs in the chain** — must be AFTER both rewrite_partition_tiles AND extend_xfer_tiles_for_halo. Both passes walk every Xfer and rewrite tiles in-place; for a halo-strip with src+dst both partitioned workers, rewrite would replace the strip with src's full partition slice. Running last preserves the carefully-crafted strip tile.

2. **Pre-paired Push+Wait with shared SeqTag** — cannot use the splice_pushes_global path because that finds the producer via data_producers, which for img_in is host's load_image. We emit BOTH endpoints pre-paired with state.fresh_seq() so splice can't mis-route. The per-worker projection in petri_to_events::emit_xfer routes Push to src's EventList and Wait to dst's EventList — both endpoints can sit in the SAME ACFG sequence.

3. **Placement = parent Sequence of outer Repeat** — single-pass stencil case lands them at top-level (before load_op + outer_repeat). For multi-pass / time-step stencils, future cycles should refine to 'inside the timestep Repeat, before the partitioned outer'. Documented as a forward-carried note on TASK-0290.

4. **IDEMPOTENCE BROKEN when partition_pairs is non-empty** (forward-carried to TASK-0290). On re-run: (a) rewrite_partition_tiles clobbers strip tiles, (b) splice_pushes_for_waits (inside inject_in_sequence) splices a NEW Push after the producer load_op for every halo-strip Wait it sees in the root sequence — because the existing Pushes from the first pass sit BEFORE load_op (outside the immediate-successor dedupe window of splice_pushes_for_waits at line ~990). The existing idempotence test (idempotent_on_synthetic_two_worker_case) stays green because empty partition_pairs short-circuits the synthesis. No production driver path re-runs inject_transfers. Filed forward-carried.

5. **Test fixture had to include a top-level host load Op** producing the halo data symbol. Without it, the hoist's escape-tracking would bubble synthesised Waits all the way to root and panic (cross-worker Wait escaped the whole ACFG with no producing Operation). Real stencils ALWAYS have load_image as producer of img_in, so the fixture is faithful — but it caught the design assumption that 'data must have a producer Op somewhere' for the hoist's escape boundary check to pass.

6. **HashSet/BTreeSet for dedupe keys**: IterTile is NOT Ord; XferRole is NOT Ord either. Switched to Vec for the (originally-intended-but-now-removed) dedupe key. Re-discovered the broader idempotence design issue and removed the dedupe entirely — accepting honest scope reduction.

### Rejected approaches

- **Synthesise only Wait, let splice make the Push**: splice routes Push via data_producers, which for img_in is host load_image, not the neighbour worker. Push lands in wrong scope and routes to host on per-worker projection. Rejected.

- **Run synthesis BEFORE rewrite_partition_tiles**: rewrite would clobber the strip tile (its compute-worker rule replaces tile with src's full partition slice when both endpoints are partitioned). Rejected.

- **Add (role, src, dst, data, tile) dedupe in inject_halo_strip_xfers for idempotence**: doesn't work because rewrite mangles the existing tile before we see it, and even if we drop tile from the key, splice_pushes_for_waits ALSO emits duplicate pushes that we cannot retroactively dedupe. Idempotence with halo-strip synthesis needs a broader fix (TASK-0290).

### Forward-carried to TASK-0290
See TASK-0290 notes for full set of gotchas the e2e cell implementer needs to know.

=== cycle-114a review gate close (orchestrator-applied) — TASK-0289 stays In Progress (AC#2+4 deferred to TASK-0290) ===

Reviewers (parallel, read-only):
- qa-test-runner: GO. gate green; 92/79/0/13/0 unchanged across 2 e2e + 2 determ runs.
- mped-architect: GO with P1 + P2 findings.

Orchestrator-applied in-thread fixes (commit 11c23e7):
- P2.1 doc-lie at transfer_inject.rs:2454-2462 (walker-returns-tuple): rewritten honestly to reflect `&mut to_insert` drain semantics.
- P2.2 doc-lie at transfer_inject.rs:2499-2504 (claimed "Recurse FIRST" but code does drain-then-recurse-then-assemble): rewritten to reflect actual order + WHY that order matters.
- P2.3 doc-lie at tests/halo_strip_synth.rs:93-98 (claimed load_op required to avoid hoist-escape panic): empirically refuted — removed load_op + ran tests, all 5 still pass. Removed the dead setup, in-line comment now records the empirical check.

P1 findings forward-carried to TASK-0290 as refinements (architect's wording was load-bearing — implementer's framing as "multi-pass only" was understated):
- P1.1 placement-before-load_op is broken for single-pass too. TASK-0290 must address before AC#2.
- P1.2 idempotence disclosure has no test pin — recommended pin in TASK-0290.

Unrelated defect discovered during gate sanity-check:
- TASK-0291 filed: `backend-common`'s `run_sh_multi_debug_asserts_so_buf_comment_lines_are_shell_comments` is `#[should_panic]` on a `debug_assert!`, which is stripped under `cargo test --release` → release-mode unit-test failure. Pre-existing on HEAD (47e844c), confirmed via `git stash` + re-run.

Cycle-114a gate (final, after hardening):
- just build: PASS
- just clippy: PASS (zero warnings)
- just test (dev profile): PASS (no failures; halo_strip_synth 5/5)
- just e2e: 92 pass / 0 fail / 13 skipped / 0 required-fail / 92 total — baseline unchanged

Per-AC final status (cycle 114a scope):
- AC#1 halo-strip Push/Wait synthesis: DONE
- AC#3 existing matrix stays green: DONE (e2e bit-identical baseline preserved + AC#3 short-circuit confirmed structural)
- AC#2 new bit-identical e2e cell: DEFERRED to TASK-0290 (still open)
- AC#4 e2e baseline bump: DEFERRED to TASK-0290 (still open)

TASK-0289 STAYS In Progress. AC#2+AC#4 close on TASK-0290.

LESSONS / SUBTLETIES (forward-carried to memory):
- "Implementer disclosure but stated mechanism is wrong" — architect's static trace was correct, implementer's claimed `load_op`-required-for-panic story was a doc-lie. The empirical 1-line edit (remove load_op, re-run tests) was the cheap verification path. When an implementer's report says "without X, Y panics with message Z" — verify by removing X if the cost is bounded.
- "qa-test-runner gate misses release-mode unit-test failures" — `just test` uses dev profile only; `--release` strips debug_asserts and breaks `#[should_panic]`-on-debug_assert! tests. The qa agent's gate is not exhaustive across profiles. TASK-0291 captures the structural fix.
<!-- SECTION:NOTES:END -->
