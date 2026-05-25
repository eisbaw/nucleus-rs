---
id: TASK-0318
title: >-
  Cycle 137 doc-lie audit on transfer_inject.rs — fix silent-sibling listing at
  rewrite_partition_tiles_inner
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 10:53'
updated_date: '2026-05-25 11:11'
labels:
  - compiler
  - transfer_inject
  - hardening
  - doc-lie
  - comment-doc-lie-recurring
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Cycle 137 audit summary (orchestrator + read-only verification)

Per `feedback-comment-doc-lie-recurring` (MEMORY.md): audited 5 multi-claim docstrings in nucleus/nucleus-compiler/src/passes/transfer_inject.rs. Findings:

### Audited (clean)
- `rewrite_partition_tiles_inner` outer docstring (lines 1656-1666): all 3 claims verify against code.
- `rewrite_partition_tiles` inline DFS-pre-order doc (lines 1601-1618): all 6 claims verify (pre-order in collect_partitioned_iter_var_nest_order, contains_key filter, dedup, etc.).
- `compute_partition_bounds_with_dim_prefix` docstring (lines 1921-1964): all 4 enumerated cases match the actual control flow (None on 2 arms; Some(empty) on ambiguity + sparse-coverage + all-holes; per_dim_cover construction). Minor imprecision: doc enumerates 'ambiguity' and 'sparse coverage' as the two Some(empty) paths but the all-holes case (data accessed but no dim partition-covered) also returns Some(empty) — covered by 'do not form a contiguous prefix' framing.
- `order_halo_strip_bounds_by_data_dim` docstring (lines 2021-2061): substantially correct. Loose-but-accurate fixture-name reference 'positive_3x3 / positive_2x2 / determinism / placement' resolves to the 4 fixture-families (positive_3x3_*, positive_2x2_*, halo_strip_synthesis_is_deterministic_across_runs, positive_placement_after_producing_op).

### Finding (P2 — fixed in-cycle)
- Silent-sibling audit listing at `rewrite_partition_tiles_inner` lines 1698-1723 (pre-cycle-137 form) named `inject_in_node_with_tile` as a tile-mutating site. Verification: that function does NOT mutate `x.tile` (it dispatches; the actual mutation is in its callee `inject_in_sequence` at line 858). The listing also omitted `hoist_invariant_waits` at line 1171, which has the structurally identical `w.tile = IterTile::new(enclosing_tile.to_vec())` pattern as a separate post-pass NOT in the inject_in_node_with_tile family.
- Both unlisted/mis-named sites are STRUCTURALLY SAFE (build from enclosing_tile, never consult partition_ranges or data_dim_iv_map) — the audit's substantive conclusion ('no other site re-imports the axis-mapping assumption') is sound. The fix is doc-only: cycle 137 rewrites the listing to enumerate by mutation-pattern category (build-from-enclosing-tile / extend-already-filtered-bounds / hand-craft-from-sidecar) with both inject_in_sequence + hoist_invariant_waits explicitly named under the first category.

### Cycle-137 lesson (feed-forward)
Per `feedback-silent-sibling-defect`: when authoring a silent-sibling audit listing as a future-proofing comment, name FUNCTIONS not FAMILIES (or be explicit that the name is family-scoped). The cycle-118 + cycle-133 author intended 'inject_in_node_with_tile' as the family name covering its recursion into inject_in_sequence; a reader doing a grep-based cross-check on the exact function name would find no x.tile mutation and conclude the listing was wrong. `hoist_invariant_waits` is a separate pass (not in any walk family) so it must be listed by name.

## Outcome
- Doc-only edit: in-place rewrite of the silent-sibling audit listing in transfer_inject.rs (no code change; no test change).
- Gate: build + clippy + test (dev + release) + e2e unchanged from cycle-136 baseline (872/0/3 tests, 108/92/0/16/0 e2e).

## Forward-carry
- No new code follow-ups filed: both newly-listed sites were already safe. The cycle is purely about closing the doc-lie defect class.
- If a future N-dim partition pass adds a 4th tile-mutation pattern (e.g. consults partition_ranges + data_dim_iv_map at `hoist_invariant_waits`), the cycle-137 listing must be extended; the listing is now categorised by mutation-pattern category so the extension point is obvious.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 137 architect P1 fold-back (NO-GO → GO after re-edit)

The first version of the cycle-137 fix omitted a THIRD sibling site: `build_waits_for_op` (line 2483, same `IterTile::new(enclosing_tile.to_vec())` pattern as the two listed sites). Architect read-only review (read the entire file with grep witness) flagged this as P1 — the cycle-137 fix re-instantiated the very silent-sibling defect class it was meant to close.

The architect also flagged two P2/P3 citation imprecisions in the cycle-137 first draft:
- P2: `(e.g. line ~2465)` for `inject_halo_strip_xfers` actually points at `build_waits_for_op`. Real fresh-tile construction sites in `inject_halo_strip_xfers` are lines 2894, 2915, 2936, 2957 (four cardinal-direction emit_pair calls after `order_halo_strip_bounds_by_data_dim` returns).
- P3-1: `extend_xfer_tiles_for_halo` line cite `~2356` is the source-range clamp comment; actual mutation is at line 2374, function entry at line 2234, worker `extend_xfer_tiles_inner` at line 2317.

All three folded back in-thread. The updated listing now:
1. Names all THREE `IterTile::new(enclosing_tile.to_vec())` sites: inject_in_sequence (858), hoist_invariant_waits (1171), build_waits_for_op (2483).
2. Includes a grep-witness pointer ("`IterTile::new(enclosing_tile.to_vec())` returns exactly these three sites") so the universal-quantifier claim is machine-checkable.
3. Cites the cross-checked grep scopes for structural-safety (zero partition_ranges / data_dim_iv_map references in scopes 724-959 + 1062-1212 + 2411-2493).
4. Corrected line cites for extend_xfer_tiles_inner (2374) and inject_halo_strip_xfers (2894/2915/2936/2957) with the function-entry lines (2234 / 2679) for context.

## Cycle-137 meta-lesson (feed-forward to memory)

A doc-audit cycle that fixes a silent-sibling defect can ITSELF re-instantiate the same defect if its 'every X' enumeration is not anchored to a grep witness. Discipline for next time: when authoring a 'every site that …' claim in a comment, INCLUDE the exact grep pattern + match count in the comment so any reader (including future-self) can re-verify by running the grep and comparing the count. The cycle-137 first draft authored a 'every other site that mutates x.tile or w.tile' claim without anchoring it to a grep — the architect's grep witness exposed the omission immediately.

This pattern strengthens `feedback-silent-sibling-defect` (in MEMORY.md): audit-fixes ARE sibling-defect candidates themselves.

## Cycle-137 P3 feed-forwards (NOT folded back this cycle, filed for future audit cycles)

- Architect P3-2: module-doc idempotence claim at transfer_inject.rs:161-166 says 'sibling Xfer nodes carrying the same (src, dst, data, tile)' as the dedup key. The two actual dedup sites use DIFFERENT keys: inject_in_sequence (line 887-897) keys on (src, dst, data, tile); hoist_invariant_waits (line 1172-1178) keys on (src, dst, data) ONLY (no tile). NOT introduced by cycle 137; predates this cycle. → file as TASK-0320 in a future audit cycle.
- Architect P3-3: TASK-0318 was created Done with no AC list. Pattern is fine for a pure-prose audit narrative, but for future audits with checkable closure criteria, prefer To-Do → In-Progress → Done with an explicit grep-checkable AC. → file as TASK-0319 forward-carrying the discipline.
<!-- SECTION:NOTES:END -->
