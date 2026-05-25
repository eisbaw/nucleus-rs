---
id: TASK-0319
title: >-
  Future-audit discipline: every 'every X' doc claim must carry a grep-witness
  anchor (TASK-0318 cycle-137 architect P1 + P3-3 forward-carry)
status: Done
assignee:
  - orchestrator-self
created_date: '2026-05-25 11:11'
updated_date: '2026-05-25 15:34'
labels:
  - compiler
  - doc-lie
  - silent-sibling
  - audit-discipline
  - forward-carried-from-TASK-0318
dependencies:
  - TASK-0318
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0318 cycle 137 was a doc-lie audit on `nucleus/nucleus-compiler/src/passes/transfer_inject.rs` that fixed a silent-sibling defect in the `rewrite_partition_tiles_inner` audit listing. The first draft of the fix RE-INSTANTIATED the same defect class — it claimed to enumerate 'every other site that mutates x.tile or w.tile' but missed a third sibling (`build_waits_for_op` line 2483). The architect's read-only review caught the omission via grep; the cycle-137 fold-back rewrote the listing to:
1. Name all three sites by name + line.
2. INCLUDE the exact grep witness in the comment (`IterTile::new(enclosing_tile.to_vec())` returns exactly these three sites).
3. Cite the cross-checked grep scopes for structural-safety.

## Why this is a future-audit pattern, not a code task

The cycle-137 lesson generalises: any audit comment that claims 'every X is covered by Y' is a future-silent-sibling-defect candidate UNLESS the comment carries (a) the grep pattern used to enumerate X, and (b) the resulting match list / count. The pattern then becomes machine-checkable by any later reader.

## Acceptance criteria

1. When a future audit cycle authors a 'every X' enumeration in a code comment, include the exact grep pattern + expected match count (or named list of matches) in the comment text.
2. Apply this discipline retroactively to other existing silent-sibling audits in transfer_inject.rs and backend-common (look for comment fragments like 'every other site', 'every callsite', 'no other place', 'all of X'). For each found, audit whether the claim is anchored to a verifiable grep; if not, add one or file a follow-up.
3. Document the pattern in MEMORY.md under feedback-silent-sibling-defect as a meta-strengthening: audit-fixes are sibling-defect candidates themselves; the mitigation is grep-witness anchoring.

## Honest scope

- LOW priority. Discipline reminder, not a defect. Trigger: next cycle that touches a silent-sibling audit listing OR the next time feedback-silent-sibling-defect fires.
- The architect's TASK-0318 P3-3 also recommended that future audits with checkable closure criteria stage through To-Do → In-Progress → Done with explicit grep-checkable AC (rather than created-Done with prose-only narrative). This task is itself an example of that discipline.

## Cross-reference

- TASK-0318 final notes (`backlog task view TASK-0318 --plain`).
- transfer_inject.rs:1696-1746 (the cycle-137 rewritten listing, now with grep witness).
- MEMORY.md `feedback-silent-sibling-defect`.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 138 forward-carry: a new dimension on grep-witness discipline

TASK-0320 cycle 138 demonstrated a new failure mode this discipline must defend against:

A **verification stamp** (e.g. a comment that names absolute line numbers as a stamp-of-correctness "as of cycle X, the witness yields N matches at lines L1 / L2 / ...") is ITSELF a doc-lie candidate if the edit that AUTHORS the stamp shifts the lines it cites. Cycle 138's first fold-back wrote line stamps that were already-stale at write-time because the bullet itself grew by ~22 lines.

**Mitigation discovered in cycle 138**:
1. Write the bullet with the stamp.
2. Save the file (do not yet commit).
3. Re-run the grep against the now-saved file.
4. Apply a DIGIT-ONLY edit to update the stamp to post-edit line numbers (no line-count change → no further drift).
5. Re-verify with grep one more time.

**Stronger structural mitigation**: make function/symbol names the PRIMARY anchor in the witness ("the `XferRole::Wait` match-arm in `inject_in_sequence`"); treat absolute line numbers as ADVISORY-ONLY. The function/symbol name is stable under arbitrary later edits; the line stamp is a convenience for the reader to find the citation quickly but should not be the load-bearing index.

**Suggested fold-into-AC**: TASK-0319 AC#1 should be amended to add: "If absolute line numbers are included as a verification stamp, they MUST be the post-write values, not pre-write. The verification protocol is grep-after-save → digit-only edit to fix → re-grep."

(This forward-carry was added by cycle-138 orchestrator, not author of original TASK-0319.)

## Cycle 146 — orchestrator self-implemented audit sweep

### What landed (AC#1 + AC#2)

1. **transfer_inject.rs silent-sibling audit listing** (lines ~1903-1965): migrated 9 stale absolute-line citations (per-function entry lines + emit-pair sites + cross-checked grep-scope range citations) to function-name anchors as PRIMARY indices + grep-witness anchors as machine-checkable verification. Cycle-137 grep-witness for site-1 (`IterTile::new(enclosing_tile.to_vec())`) re-verified — 3 production sites unchanged. New grep witness added for site-3 (4 cardinal emit_pair sites inside inject_halo_strip_xfers).

2. **pthreads-sync/tests/reuse_marker.rs**: replaced 4 stale absolute-line citations (`multi_worker_walker.rs:404`, `:478`, `pthreads-sync/src/lib.rs:653`, `:675`) with function-name anchors (`render_worker_events_inner` and `render_event`) and a grep witness.

3. **backend-common/tests/multi_worker_reuse_marker.rs**: replaced 2 stale line citations (`multi_worker_walker.rs:478`, `:404`) with function-name anchors + grep witness mentioning the two production sites inside `render_worker_events_inner`.

4. **backend-common/tests/wait_assign_slice.rs**: replaced 2 stale line citations (`multi_worker_walker.rs:809` Wait, `:789` Push) with function-name anchors + a precise grep-witness pattern (`{rendezvous_prefix}_{rid}.(push|wait)`) that returns EXACTLY two production matches (excludes docstring examples by using the literal `{rid}` placeholder, not `{id}`).

5. **MEMORY.md cross-reference**: AC#3 is met by existing memory entries — `feedback-silent-sibling-defect` cycles 137-138 + 145 already document the audit-fix-is-sibling-defect-candidate pattern and the grep-witness mitigation.

### Verification gate (cycle-146 self-run)

- `just check`: clean.
- `just clippy --all-targets -D warnings`: clean.
- `just test` (dev): all pass, no regression.
- `just e2e`: 112/92/0/20/0 — IDENTICAL to pre-cycle-146 baseline.

### Gotchas + forward-carries

1. **The cycle-137 audit listing itself stamp-drifted by ~700 lines** in the 8 cycles since it was last refreshed. Cycle 146's mitigation is structural: function-name anchors are the load-bearing index; line numbers (where retained) are advisory-only. New grep witnesses are MACHINE-CHECKABLE (any reader can re-run the grep, verify the match count, and follow the enumeration).

2. **Grep-witness pattern hygiene**: when authoring a grep witness, prefer match-strings that EXCLUDE non-production hits (docstrings, comments, test source). The cycle-146 wait_assign_slice.rs grep uses the literal `{rid}` placeholder to exclude the docstring examples that use `{id}` — "exactly N production sites" claims must match the GREP COUNT, not require the reader to mentally filter.

3. **What this cycle does NOT cover**: there are ~50-100 other absolute file:line citations across nucleus/ that are NOT silent-sibling audits (e.g. external pointers like "see lib.rs:551"). Migrating those is beyond TASK-0319's scope — they're documentation-helper anchors, not load-bearing universal-quantifier claims. Filed observation, not a follow-up task.

## Cycle 146 — final state after review fold-back

### Review gate (parallel read-only)

- **qa-test-runner**: GO. 4/4 grep-witness verifications PASS. Full ci run truncated by agent; orchestrator verified e2e 112/92/0/20/0 pre-commit + post-commit.
- **mped-architect**: GO with P2.1 + P2.2 + P3.1 follow-ups (architect caught header/body asymmetry — 9th firing of the silent-sibling pattern, in a NEW shape).

### Architect findings folded back in-thread (commit ab4ba6b)

- **P2.1 (header/body asymmetry within scope-file)**: 5 stale in-body citations in 3 in-scope files migrated to function-name anchors: wait_assign_slice.rs 604/692/781, multi_worker_reuse_marker.rs 283/380, reuse_marker.rs 70-71/160.
- **P2.2 (out-of-scope sibling)**: partition_blocks2d.rs:40 migrated; grep-witness disambiguated 5-vs-1 (one production site at `render_worker_events_inner` Event::Loop arm, four read-only `collect_*` walker siblings).
- **P3.1 (memory cycle-146 closure note)**: feedback-silent-sibling-defect.md updated with 9th-firing entry promoting 'header/body asymmetry within a single file' as a new shape.

### Self-discovered fold-back during cycle (commit 17f480c)

- emit_pair grep-witness tightened from `emit_pair(neighbour` (5 hits including audit-listing self-reference) to `emit_pair\(neighbour,` (4 production sites — trailing comma excludes self-ref).
- IterTile witness re-stated: 5 total hits (3 production + 1 module-doc + 1 audit-listing self-ref) with `grep -vE ':\s*//'` filter trick for production-only count.

### TASK-0319 status

**Done.** AC#1 + AC#2 + AC#3 all landed. The future-audit discipline is now anchored in 4 silent-sibling audit listings + 1 out-of-scope sibling + 1 memory entry. The architect's 'header/body asymmetry' finding is the 9th firing of the silent-sibling pattern — captured as a new structural shape.

### Final verification gate

- `just check`: clean.
- `just clippy --all-targets -D warnings`: clean.
- `just test` (dev): all pass.
- `just e2e`: 112/92/0/20/0 (pre-cycle baseline preserved).

### Forward-carries

- **TASK-0319 charter is now closed**, but the *discipline* it codified continues — every future cycle that authors an 'every X' / 'exactly N sites' claim MUST carry a grep-witness anchor; the witness MUST disambiguate self-references and comment matches; the load-bearing index is the function/symbol name, not the line number.
- **Memory entry** feedback-silent-sibling-defect.md now carries 9 firings — the architect's read-only review is the safety net that keeps catching the pattern across new shapes.

### Lessons forward-carried into memory

- 9th firing observation: "header/body asymmetry within a single file" is a new shape. Orchestrator hygiene: when retroactively migrating audit listings, scope is the WHOLE FILE — grep `:[0-9]+` matches across the touched file (including assertion-message strings, per-test docstrings, inline comments) and migrate each match.
- The architect's heuristic 'did the orchestrator's execution actually match the AC wording?' caught this cycle's scope-cut vs scope-execution mismatch (sibling of cycle-129 wording-execution mismatch from feedback-orchestrator-narrative-also-wrong).
<!-- SECTION:NOTES:END -->
