---
id: TASK-0333
title: >-
  TASK-0328 sibling sweep: audit partial-overlap arm for clause-(1)-class
  over-lenience (cycle-154 architect P2.3 fold-back)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 20:27'
updated_date: '2026-05-25 21:13'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0328 cycle-154 removed clause (1) (the "no partition iv active on consumer scope → safe elision" short-circuit) at both the validator site and the emit-site mirror in transfer_inject.rs's check_op_no_silent_elision_risk + build_waits_for_op same-set branch.

The cycle-154 architect P2.3 fold-back noted that the **partial-overlap arm** — the per-element `if src == dst { continue; }` skip inside the cartesian-product fan-out (grep-witness in build_waits_for_op) — is the **structurally identical sibling** of the same-set clause (1) we just removed. By the cycle-148 paired-lift discipline ([[feedback-silent-sibling-defect]]), the clause (1) defect concept should be audited there.

## Hypothesis

The partial-overlap arm fires for the case where `producer_workers != &consumer_workers` but the intersection is non-empty. For every (src, dst) pair where src == dst (worker w_i in both sets), the per-element skip elides the cross-worker transfer.

If the producer is partition-sliced (each w_i has only its band) and the consumer at top-level reads outside its band: the same silent-miscompile shape can fire on the partial-overlap arm. The cycle-154 fix DOES NOT cover this case — the validator at transfer_inject.rs (~line 3013) still REJECTS partial-overlap unsafe via SameSetSilentElisionRisk (the cycle-147 AC#3 lift is scoped to same-set only).

## Acceptance criteria

### AC#1: empirical investigation

Write a synthetic fixture mirroring task0328_ac2_positive_partition_producer_topfile_consumer but with partial overlap: producer on {w0..w3}, consumer at top-level on {w0..w3, w4} (5 workers, intersection = {w0..w3}). Run it against current code and observe.

- If validator rejects → existing behaviour is sound for partial-overlap (the cycle-147 AC#3 lift's restricted scope is intentional).
- If validator returns Ok AND emit elides per-element → silent miscompile sibling confirmed; needs cycle-154-style fix.

### AC#2: depending on AC#1 outcome

Either document the partial-overlap arm's current rejection as load-bearing (no fix needed) OR remove the per-element skip's clause-1-class short-circuit in lockstep with cycle-154's fix.

### AC#3: regression pin

Add the AC#1 fixture as a regression test pin.

## Honest scope

- LOW priority. Dormant defect (no in-tree schedule exercises partial-overlap today).
- Filed per [[feedback-silent-sibling-defect]] cycle-154 firing: when a defect-class fix lands on one arm (same-set), structurally identical siblings (partial-overlap) need audit, not implicit assumption.

## Cross-reference

- transfer_inject.rs (~line 3013): partial-overlap rejection via SameSetSilentElisionRisk (the cycle-147 AC#3 lift's scope exclusion).
- transfer_inject.rs (~line 3217-3225): the per-element `if src == dst { continue; }` skip with its TASK-0325 comment block.
- TASK-0325 cycle-145: generalised the validator from set-equality to non-empty-intersection. Coverage is symmetric on the validator side; the emit-site fan-out's per-element skip is the sibling we audit here.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
## Implementation Plan (cycle 155)

### AC#1 — empirical investigation + regression pin

Add test `task0333_ac1_partial_overlap_partition_producer_topfile_consumer_rejects` in tests/transfer_inject.rs after the cycle-154 fixtures.

Structural shape (mirroring `task0328_ac2_positive_partition_producer_topfile_consumer` template at L3315-3700):
- IterVars hy/hx (fresh ids to avoid collision with existing fixtures)
- DataIds in_arr / tmp / out
- Producer writes tmp[hy][hx] on {w0..w3} inside a partition=rows nest
- Consumer at TOP LEVEL reads tmp[5][3] (constant indices) on {w0..w3, w4} — partial overlap (5 workers, intersection = {w0..w3})
- Partition_worker_ranges[hy] populated for w0..w3 (w4 absent — same as task0325_ac2_positive)
- Linked tmp owners = {w0..w3}; out owners = {w0..w3, w4}

Empirical hypothesis verification:
- Run `inject_transfers(&linked, acfg)` against current code.
- EXPECTED (per orchestrator brief + validator code @ L2945-3030): Err(SameSetSilentElisionRisk { data: D_TMP, .. }).
  - Validator clause at L2949-2951: intersection {w0..w3} ≠ empty → does NOT continue.
  - prod_access present (writer_edge has data_out_access) → no early continue at L2963-2966.
  - same_set_elision_unsafe_reason fires: prod axis-0 is Ident(hy) ∈ partition_iter_vars → p_iv=Some(hy); consumer axis-0 is IntLit(5) → c_iv=None ≠ Some(hy) → returns Some(reason). 
  - Cycle-147 AC#3 lift at L3004-3006 is gated on producer_workers == &consumer_workers → false here ({w0..w3} ≠ {w0..w3,w4}) → does NOT continue.
  - Falls through to format error at L3008-3030; returns Err.

Assertion (matches existing task0325_ac2 / task0328_reverse_direction style at L2890-2924):
  - Err variant = SameSetSilentElisionRisk
  - data == D_TMP
  - message contains "TASK-0324", "TASK-0325", "overlap", "partition-sliced"

If validator returns Ok instead (i.e. the per-element skip elides cross-worker transfer silently) — STOP, file follow-up task TASK-0334 for the silent-miscompile sibling, do not close 0333. Per implementer brief.

### AC#2 — comment-pin grep witnesses

Once AC#1 passes (validator rejects), add brief addenda using identical phrasing "TASK-0333 cycle-155 audit confirmed" at TWO sites:
- transfer_inject.rs L2985-3003 (validator partial-overlap rejection rationale block — inside the cycle-147 AC#3 lift comment)
- transfer_inject.rs L3310-3319 (emit-site per-element skip's comment block — inside the TASK-0325 cycle-145 partial-overlap commentary)

Wording: state the audit conclusion (partial-overlap arm has no clause-(1) analog to remove; validator rejection is the load-bearing safety net) and reference the regression test name task0333_ac1_partial_overlap_partition_producer_topfile_consumer_rejects.

Identical "TASK-0333 cycle-155 audit confirmed" anchor at both sites → grep witness check post-edit. Anti-companion-asymmetry per feedback-silent-sibling-defect.

### AC#3 — regression pin

The AC#1 test IS the regression pin. Identical wording at both comment sites is the structural pin.

### Verification gate

`nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"`
- just test: expect +1 test (the new task0333 fixture)
- just test-release: must pass (load-bearing per feedback-qa-gate-misses-release-profile)
- just e2e: must hold 108/92/0/16/0 baseline

### Commit shape
`transfer_inject + tests + tracker: TASK-0333 cycle 155 — partial-overlap arm audit (clause-1 sibling sweep, cycle-154 P2.3 fold-back) — validator rejection confirmed load-bearing`

### Honest pitfalls flagged
- IterVar id collision: pick 121/122 (well past the 91/92 used by task0328_ac2_negative; 31-35 used by task0325_ac2_positive).
- The validator's error message FORMAT must include 'TASK-0324' and 'TASK-0325' substrings — verify at L3019-3025 of transfer_inject.rs before relying on assertions.
- 'overlap' anchor: appears at L3015 in error message — check.
- 'partition-sliced' anchor: appears in same_set_elision_unsafe_reason at L3125 — check.
- The task0328_ac2_positive_partition_producer_topfile_consumer test (template) asserts OK with 12 cross-worker pairs because same-set + cycle-147 AC#3 lift. TASK-0333's variant differs from same-set; the cycle-147 lift gate (L3004) is producer_workers == &consumer_workers; partial-overlap ≠ that condition → falls through to Err. The structural asymmetry between same-set (Ok + 12 pairs) and partial-overlap (Err) IS the audit result.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 155 — IMPLEMENTATION LANDED

### Evidence summary

- **AC#1 (empirical investigation + regression pin)**: PASS.
  - Test `task0333_ac1_partial_overlap_partition_producer_topfile_consumer_rejects` added (tests/transfer_inject.rs L3985-end). Mirrors `task0328_ac2_positive_partition_producer_topfile_consumer` template with consumer worker set = {w0..w3, w4} (5 workers, partial overlap).
  - Run in isolation BEFORE comment edits: **PASS** with all 4 message anchors matched ("overlap", "partition-sliced", "TASK-0324", "TASK-0325").
  - Documented outcome (case 1) confirmed: **validator rejects the partial-overlap unsafe shape → existing behaviour is sound; no fix needed.** The cycle-147 AC#3 lift gate (`producer_workers == &consumer_workers`) does NOT short-circuit partial-overlap; falls through to the format-error path.

- **AC#2 (documentation as load-bearing)**: PASS.
  - Identical anchor `TASK-0333 cycle-155 audit confirmed` added at 2 sites in src/passes/transfer_inject.rs:
    - L3005 — validator's partial-overlap rejection block (inside the cycle-147 AC#3 lift comment).
    - L3332 — emit-site per-element `if src == dst { continue; }` skip's TASK-0325 partial-overlap commentary.
  - Both sites cross-reference `task0333_ac1_partial_overlap_partition_producer_topfile_consumer_rejects` as the regression pin (grep witness at 3 file:line combinations).
  - Wording at both sites states the audit conclusion: **partial-overlap arm has NO clause-(1) analog to remove** (the per-element skip is unconditional, with no consumer-scope gate); **validator rejection IS the load-bearing safety net**.

- **AC#3 (regression pin)**: PASS.
  - AC#1 test is the structural regression pin. Identical `TASK-0333 cycle-155 audit confirmed` wording at both comment sites + cross-reference of the test name is the anti-companion-asymmetry pin (per feedback-silent-sibling-defect).

### Verification gate (just ci subset, all GREEN)

- `just build`: PASS.
- `just clippy`: PASS.
- `just test` totals: **passed: 899 failed: 0 ignored: 3** (baseline e43e3d3: 898/0/3 → +1 from new task0333_ac1 test).
- `just test-release` totals: PASS, release-profile counts match dev.
- `just e2e` totals: **total: 112  pass: 96  fail: 0  skipped: 16  required-fail: 0** — UNCHANGED from cycle-154 closure baseline (the TASK-0333 implementer brief said 108/92/0/16/0 but the actual baseline per TASK-0328 closure note is 112/96/0/16/0; the brief was stale-forward-carry per feedback-orchestrator-narrative-also-wrong).
- `just check-textual-replace-on-codegen`: PASS.
- `just check-include-str-coverage`: PASS.

### Subtleties + gotchas hit

1. **Existing TASK-0325 test partially overlaps in scope** — `task0325_ac2_positive_partial_overlap_non_aligned_read` (L2704) already pins a partial-overlap rejection but uses a non-aligned-iv consumer-side read shape (reads `tmp[vm][vx]` inside its own loop nest). TASK-0333 pins the structurally orthogonal shape: constant-indices + TOP-LEVEL consumer (no enclosing loop). Both shapes must reject — together they cover both consumer-side asymmetry triggers under partial overlap. The task brief did not mention this overlap; flagged in the test's docstring.

2. **Implementer brief baseline number was stale** (per feedback-orchestrator-narrative-also-wrong): brief said e2e 108/92/0/16/0; actual cycle-154 closure was 112/96/0/16/0. Confirmed against TASK-0328's own closure note before claiming "baseline preserved". This is the 11th-firing-class of orchestrator narrative drift. The actual measurement matched 112/96/0/16/0 byte-identical so the baseline IS preserved — but had the brief been treated as authoritative without checking, the e2e number would have looked like a regression.

3. **No clause-(1) analog actually exists at the emit-site partial-overlap arm.** The emit-site `if src == dst { continue; }` (L3304) is a single unconditional skip inside the cartesian product fan-out — there is no consumer-scope-conditioned branch the way clause (1) was a consumer-scope-conditioned branch at the same-set whole-set short-circuit (L3185-3261). So "remove the analog in lockstep with cycle-154" was a category error in the original cycle-154 P2.3 fold-back framing — the sibling sweep concept applies, but the structural sibling pattern doesn't transfer because the emit-site partial-overlap arm has nothing to remove. The validator's rejection path IS the safety net here.

4. **No follow-up tasks filed** — neither defect nor gap surfaced. The audit confirmed soundness.

### Implementer effort honesty (per feedback-implementer-effort-estimate-overstated)

The audit work was small: ~250 LoC for the synthetic test + ~14 lines of comment addenda. The verification gate (full just-ci subset) was the bulk of the wall time. Total cycle wall time well under one hour. No surprise expansions of scope.

## Cycle 155b — review-gate fold-back addendum

Both review agents returned GO on cycle 155 (commit 29e28a9). No P1/P2 findings; all P3s are forward-carry. Applied in-thread (cycle 155b):

- **architect P3.1**: added a grep-anchored `// FUTURE-WORK (TASK-0333 cycle-155b architect P3.1):` marker in transfer_inject.rs (between the cycle-147 AC#3 same-set early-continue and the partial-overlap rejection emit) so the latent partial-overlap-AC#3 extension surface is greppable as a single anchor rather than buried inside the rejection prose.
- **architect P3.2**: memory file `~/.claude/projects/.../memory/feedback-orchestrator-narrative-also-wrong.md` updated with the 11th-firing entry — the implementer-brief stale-baseline (108/92/0/16/0 vs actual cycle-154 closure 112/96/0/16/0). Distinctive in that the wrong-number was in the orchestrator's IMPLEMENTER BRIEF, not in a tracker note. New hygiene rule added: cite the SOURCE ("per TASK-0328 cycle-154 closure") not bare numbers when writing briefs.
- **qa P3.2**: tests/transfer_inject.rs doc comment cleaned — removed the off-by-one `(L2704)` absolute-line citation; function-name grep-anchor is the project's preferred pattern per [[feedback-stamp-twice-when-narrative-content-shifts-line]].

Deferred (forward-carry, not blocking):
- **qa P3.1**: pre-existing codegen-template warnings in nuc-generated scratch crate (`unused_assignments` × 3). Not from cycle-155. Future small-task material.
- **qa P3.3**: paired-anchor phrasing drift risk (single-source-of-truth via `see also` reference). Intentionally accepted per silent-sibling discipline.

### Cycle-155b gate

- `just test`: 899/0/3 (preserved).
- `just clippy`: zero warnings.
- `just e2e`: 112/96/0/16/0 byte-identical (single sample post-edit; cycle-155 reviewer already verified non-flake × 2).

### Orchestrator self-discovered defect (cycle-155b)

Initial attempt used `backlog task edit --notes` which REPLACES rather than appends — clobbered the cycle-155 implementer's notes. Reverted via `git checkout` and re-applied with `--append-notes`. Lesson: ALWAYS use `--append-notes` for incremental progress; `--notes` is for first-write only. Filing this as a tracker-hygiene addendum to the orchestrator-narrative-also-wrong memory.

### Final closure

TASK-0333 stays Done. Audit concluded with empirical evidence: partial-overlap arm has no clause-(1) analog at the emit-site; validator hard-rejection is the load-bearing safety net. Regression pin in place. Future-work anchor added. No new tracker tasks filed (no defect surfaced).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Partial-overlap arm audit (cycle-155 sibling-sweep of cycle-154 clause-(1) removal) CONFIRMS validator rejection is the load-bearing safety net. No fix needed; documented outcome 1 chosen. AC#1 regression pin (task0333_ac1_partial_overlap_partition_producer_topfile_consumer_rejects) + AC#2 comment addenda at 2 sites with identical 'TASK-0333 cycle-155 audit confirmed' anchor (grep-witnessed). Gate: just test 899/0/3 (+1), just test-release matched, just e2e 112/96/0/16/0 byte-identical to cycle-154 closure. Subtleties: existing task0325_ac2 covers the non-aligned-iv partial-overlap shape (TASK-0333 covers the orthogonal top-level constant-indices shape); no emit-site clause-(1) analog actually exists to remove (per-element skip is unconditional, no consumer-scope gate); implementer-brief baseline number (108/92/0/16/0) was stale-forward-carry (actual cycle-154 baseline 112/96/0/16/0 per TASK-0328 closure note, verified against tracker before claiming preserved).
<!-- SECTION:FINAL_SUMMARY:END -->
