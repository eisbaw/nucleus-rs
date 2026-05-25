---
id: TASK-0320
title: >-
  Module-doc/code drift: transfer_inject.rs:161-166 idempotence claim says
  key=(src,dst,data,tile); hoist_invariant_waits dedup key omits tile (architect
  P3-2 forward-carry from TASK-0318 cycle 137)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 11:11'
updated_date: '2026-05-25 11:42'
labels:
  - compiler
  - transfer_inject
  - doc-lie
  - comment-doc-lie-recurring
  - forward-carried-from-TASK-0318
dependencies:
  - TASK-0318
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0318 cycle 137 architect P3-2: the module-level docstring at `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:161-166` claims idempotence is by dedup on `(src, dst, data, tile)`. The two actual dedup sites have DIFFERENT keys:

- `inject_in_sequence` (line 887-897): `(src, dst, data, tile)` ✓ matches the doc.
- `hoist_invariant_waits` (line 1172-1178): `(src, dst, data)` only — `tile` is NOT in the match arm.

The module doc overclaims uniformity that the two sites do not have. NOT introduced by cycle 137; predates this cycle (architect spotted during the cycle-137 audit).

## Acceptance criteria

1. Determine whether the divergence is intentional (e.g. `hoist_invariant_waits` runs at a point where two Waits with the same (src, dst, data) but different tiles MUST be merged or are guaranteed identical) or a real defect.
2. If intentional: fix the module-doc to reflect the per-site policy. E.g. 'inject_in_sequence dedups on full (src, dst, data, tile); hoist_invariant_waits dedups on (src, dst, data) because the tile is rebuilt from enclosing_tile.to_vec() the moment before the dedup check, so any two Waits surviving to this point have identical tiles by construction.'
3. If a real defect: file the concrete shape (e.g. 'two cross-worker Waits on the same data with different tile granularities would be silently merged') as a separate code task and pin the divergence as a regression test in transfer_inject_hoist.rs or hoist_invariant_waits's test fixture.

## Honest scope

- LOW priority. Pre-existing drift; cycle-137 surfaced it but did not introduce it. No e2e regression observed under shipped schedules (every shipped Wait pair has a unique (src, dst, data) per scope under the M5 partition shapes).
- Trigger: next M6+ schedule that produces two cross-worker Waits on the same data with different tile granularities, OR a quality-coverage cycle that audits the module doc against the code.

## Cross-reference

- transfer_inject.rs:161-166 (module doc, the claim).
- transfer_inject.rs:887-897 (inject_in_sequence dedup, includes tile).
- transfer_inject.rs:1172-1178 (hoist_invariant_waits dedup, omits tile).
- TASK-0318 cycle 137 architect P3-2 (the surfacing review).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 138 implementation plan (orchestrator-direct, no implementer subagent)

Per `feedback-spawned-agents-refuse-code-edits`: implement directly in-thread.

### Verification (done before edit)
- `inject_in_sequence` (line 887-897): dedup key `(src, dst, data, tile)` — uses full 4-tuple, confirmed by grep of the `matches!` arm.
- `hoist_invariant_waits` (line 1172-1178): dedup key `(src, dst, data)` ONLY — `tile` is NOT in the match arm.
- Line 1171 (one line before the dedup check) executes `w.tile = IterTile::new(enclosing_tile.to_vec())` — the tile is freshly rewritten before the dedup, so any two surviving Waits in the same scope are GUARANTEED to have identical `tile` by construction. Adding `tile` to the dedup key would be redundant but functionally equivalent.

### Conclusion (AC#1)
Divergence is INTENTIONAL, not a defect. The per-site policy is:
- `inject_in_sequence` dedups on full (src, dst, data, tile) — runs at a layer where placeholders carry arbitrary tile granularities, so the tile is part of the identity.
- `hoist_invariant_waits` dedups on (src, dst, data) — the tile is normalised to enclosing_tile.to_vec() on the line above, so the (tile) dimension of the key is constant within a scope.

No code change, no new test required. The fix is doc-only at transfer_inject.rs:161-166.

### Per AC#2 (intentional → fix module-doc)
Rewrite the module-doc bullet to explain the per-site policy:
- inject_in_sequence: full 4-tuple including tile.
- hoist_invariant_waits: (src, dst, data) only — tile is rewritten to enclosing_tile.to_vec() the moment before the dedup, so the tile component of the key is constant by construction.
- Cite both sites by line range (point of dedup, not the surrounding helper).

### Gate
`nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"` — expected unchanged from cycle-137 baseline (872/0/3 tests, 108/92/0/16/0 e2e), since this is doc-only.

## Cycle 138 final notes — fold-back chain

### Edit chain (orchestrator-direct)

- **Edit 1**: rewrote idempotence bullet from "two dedup sites, same key" to "two sites, different keys (intentional)" — flagged TASK-0320 AC#1 + AC#2.
- **Architect P1 NO-GO**: missed THIRD dedup site (`splice_pushes_for_waits` Push dedup at line 1019), stale line cites (`~887/~1171/~1172` vs actual 906/1019/1193), and a locally-right-but-globally-misleading reasoning argument.
- **Edit 2 (fold-back)**: rewrote bullet to enumerate three sites (Wait at `inject_in_sequence`, Push at `splice_pushes_for_waits`, Wait at `hoist_invariant_waits::place_or_bubble`), switched from line numbers to function-name anchors, added grep-witness anchor (per TASK-0319 discipline), reframed the divergence as "dedup-set composition" not "arbitrary tile granularities".
- **Architect P1 NO-GO (round 2)**: line-number stamp in the grep witness drifted self-consistent — the doc edit grew the file by ~22 lines, so the lines I cited (910/1019/1193 → 933/1042/1216) were already stale at the moment of writing.
- **Edit 3 (fold-back-2)**: bumped line stamps to post-edit values 943/1052/1226 + 985/1011/1179/1323/1413/1434.
- **Clippy `doc_lazy_continuation` failure**: my fold-back-2 dropped a blank `//!` separator before the witness paragraph, triggering clippy on 5 lines. Recovered by inserting the blank `//!`.
- **Edit 4 (clippy fix + digit re-stamp)**: added blank `//!` separator at line 193, +1 net line shift, updated stamps to 944/1053/1227 + 996/1022/1190/1334/1424/1445.
- **Edit 5 (off-by-one fix)**: my predicted +2 shift for edit 4 was actually +1 — corrected the line stamps digit-only (no further line shift).

### Final state

`grep -nE 'existing\.role == XferRole::|x\.role == XferRole::' transfer_inject.rs` returns 9 matches, exactly matching the doc's stamp:
- Dedup checks (4-tuple Wait, 4-tuple Push, 3-tuple Wait): **944 / 1053 / 1227**.
- Role-scans (non-dedup): **996 / 1022 / 1190 / 1334 / 1424 / 1445**.

### Acceptance criteria status

- **AC#1**: Divergence is INTENTIONAL. Reasoning: `hoist_invariant_waits::place_or_bubble` rewrites tile to `enclosing_tile.to_vec()` on the line immediately before its dedup check, and the dedup-set is LOCAL to that scope, so every dedup-set member carries the same tile by construction. `inject_in_sequence`'s `out: Vec<ACFGNode>` dedup-set may contain pre-existing Waits NOT rewritten by the current sequence pass, so `tile` is identity-bearing. ✓ verified.
- **AC#2**: Module-doc fix applied at transfer_inject.rs:161-220. Per-site policy documented. Grep witness with function-name anchors + line-number verification stamp. ✓ verified by architect read-only review (GO).
- **AC#3**: Not triggered — divergence was intentional, not a defect.

### Gates

- `just build && just clippy` ✓ (zero warnings, `-D warnings`)
- `just test` ✓ 872 passed / 0 failed / 3 ignored (cycle-137 baseline preserved)
- `just test-release` ✓ 872/0/3 (matches dev, no debug-assert divergence)
- `just e2e` ✓ 108/92/0/16/0 (cycle-137 baseline preserved exactly)
- Read-only review gate (qa-test-runner + mped-architect parallel, architect re-invoked after each of two fold-backs): GO

### Forward-carried lessons (TO BE filed if not auto-folded)

- **NEW pattern**: a verification stamp (e.g. "as of cycle X, line numbers are N/M/...") is itself a doc-lie candidate IF the edit that authors it shifts the lines it cites. Mitigation: write the stamp, re-grep, then update with a digit-only edit. Make function/symbol names the primary anchor and treat line numbers as advisory-only.
- This is a NEW dimension on `feedback-comment-doc-lie-recurring` and an instance of TASK-0319's discipline (grep-witness anchoring) — to be folded into MEMORY.md.

### Cycle conclusion

Doc-only edit, baseline preserved across all gates. The doc-lie defect of TASK-0318 P3-2 is closed; no code change required.
<!-- SECTION:NOTES:END -->
