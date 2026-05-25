---
id: TASK-0309
title: >-
  sidecar_halo.rs::lower() uses STRICT-A halo_inference but driver uses
  partition-aware-B — add lower_partition_aware() to mirror driver pipeline
  (TASK-0304 cycle-124 architect P2.1)
status: Done
assignee:
  - mark
created_date: '2026-05-25 05:03'
updated_date: '2026-05-25 06:50'
labels:
  - M5
  - test-coverage
  - halo_inference
  - driver-divergence
  - forward-carried-from-TASK-0304
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0304 cycle-124 architect review-gate (qa-test-runner + mped-architect) flagged a pre-existing divergence inherited by the new tests:

- The test helper at `nucleus/nucleus-compiler/tests/sidecar_halo.rs:46-68 lower()` calls `apply_halo_inference` (strict-A variant: fail-fast on any non-affine / strided / data-dependent index).
- The driver at `nucleus/driver/src/main.rs:396` calls `apply_halo_inference_partition_aware` (variant B: fatal only when the offending iv carries a Partition directive; otherwise recorded as advisory and lowering proceeds).

For shipped distributed schedules in M5 (05-stencil + 06-separable-filter + 07-matmul) the two variants AGREE on the halo_widths map because the inputs are fully affine; so the existing `task0299_*` / `task0303_*` / new `task0304_*` tests pass under both. But the tests do NOT exercise the SAME pipeline the driver does — a future regression that manifests only under the partition-aware-B path would slip through.

## Acceptance criteria

1. Either:
   - Migrate `sidecar_halo.rs::lower()` to call `apply_halo_inference_partition_aware` (potentially breaking the existing TASK-0275 strict-failure tests at lines 359-589; those need a separate idiom).
   - Or add a sibling `lower_partition_aware()` helper that mirrors the driver pipeline and migrate `task0299_*` / `task0303_*` / `task0304_*` to use it.
2. The TASK-0275 in-module tests for strict-A error behaviour (`task0275_partition_aware_rejects_*` + `task0275_partition_aware_accepts_*`) MUST continue to use the strict-A helper. Hint: a `lower()` + `lower_partition_aware()` split keeps each test idiom unambiguous.
3. e2e baseline 108/92/0/16/0 preserved (no production-code change; this is test-pipeline alignment).

## Honest scope

LOW priority. The divergence is currently observationally inert (the two variants agree on every shipped fixture). Filing because the cycle-119 memory `feedback-implementer-disclosure-mechanism-wrong` warned about exactly this kind of pipeline-divergence between test and driver, and a future M6+ schedule may exercise the partition-aware-B path differently.

## Cross-references

- TASK-0304 cycle 124 architect review-gate P2.1.
- Memory: `feedback-implementer-disclosure-mechanism-wrong` (cycle 119 — orchestrator note claimed driver uses strict-A; the actual code at driver/main.rs:396 uses partition-aware-B; the lesson includes the test-vs-driver divergence vector).
- `nucleus/nucleus-compiler/src/passes/halo_inference.rs` (module-doc section "## Strict vs advisory vs partition-policy-aware entry points"; search for that heading) — the contract paragraph documenting the 3 entry points (strict-A, advisory, partition-aware-B).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle-128 implementation plan (orchestrator-led, 2026-05-25)

Orchestrator-led after an Agent 529 Overload prevented spawning the mped-architect implementer; the work is mechanical (add a sibling helper + swap call sites in 7 test functions) and the brief was fully prepared. Mandatory parallel review gate still runs.

### Migration target call sites (cycle-128 grep)

Confirmed via `grep -n 'lower(' sidecar_halo.rs`:

**KEEP using `lower()` (strict-A pipeline; pinning halo_widths under fail-fast semantics) — 4 sites:**
- line 76: stencil_3x3_produces_halo_one_on_both_axes
- line 136: naive_example_01_produces_empty_halo_widths
- line 161: serde_roundtrip_preserves_halo_widths
- line 189: serde_legacy_payload_default_halo_widths_empty

**MIGRATE to `lower_partition_aware()` (mirror driver pipeline) — 7 sites:**
- line 609: task0299_06_separable_filter_distributed_halo_widths_pinned_to_zero
- line 721: task0303_05_stencil_distributed_2d_halo_widths_pinned_to_one
- line 809: task0303_07_matmul_distributed_halo_widths_pinned_to_zero
- line 891: task0304_06_separable_filter_distributed_transfer_inject_no_halo_extension_on_in_arr_hy
- line 992: task0304_05_stencil_distributed_transfer_inject_halo_one_extension_on_img_in_y
- line 1137: task0310_05_stencil_distributed_2d_transfer_inject_halo_one_extension_on_img_in_y_and_x
- line 1304: task0310_07_matmul_distributed_transfer_inject_no_halo_extension_on_a_i

**Do NOT migrate**: the 5 task0275_partition_aware_* tests at lines 360-589 — already bypass `lower()` (construct synthetic IR via build_linked_for_partition_test and call apply_halo_inference_partition_aware directly). Unaffected by this change.

### lower_partition_aware signature

Mirrors `lower()` `(LinkedIR, ACFG)` return shape. Difference: call `apply_halo_inference_partition_aware(&linked, acfg)` which returns `Result<(ACFG, Vec<HaloInferenceError>), HaloInferenceError>` (per the contract paragraph at halo_inference.rs module-doc section "## Strict vs advisory vs partition-policy-aware entry points"). Pattern: `let (acfg, _advisory_errors) = apply_halo_inference_partition_aware(...).expect("halo_inference_partition_aware")`. The advisory vector is discarded with `_` because the helper's callers (task0299/0303/0304/0310) pin halo_widths VALUES on real distributed schedules; an unexpected error on a previously-passing fixture would surface elsewhere (e2e bit-identical).

### Docstring discipline (cycle-127 lessons)

The new docstring on `lower_partition_aware` uses cycle-122 symbolic anchors:
- `fn apply_halo_inference_partition_aware` (halo_inference.rs:471 verified greppable)
- `fn apply_halo_inference` (halo_inference.rs:411 verified greppable)
- `## Strict vs advisory vs partition-policy-aware entry points` (halo_inference.rs:100 verified single-hit greppable)

No absolute-line citations in new code.

### Verification gate

`nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"`. Required: 854/0/3 dev, 854/0/3 release, 108/92/0/16/0 e2e. The migration is observationally inert (both variants agree on every shipped fixture today per TASK-0309 description) so all 7 migrated tests should still pass; gate numbers should be IDENTICAL.

### Honest scope

The migration is observationally inert today. It defends against a future M6+ schedule that would silently regress under the partition-aware-B path while passing strict-A. Test count: unchanged (no new tests, no removed tests).

## Cycle-128 review-hardening addendum (architect P2 #1 + qa P2-1 closure, 2026-05-25)

The cycle-128 architect review-gate caught a meta-recurrence: this very task's Implementation Plan (the block above this addendum) re-introduced 3 fresh `halo_inference.rs:NNN` absolute-line citations (anchor-table entries citing :411, :471, :100) WITHIN THE SAME SESSION cycle 127 (TASK-0311) closed the class-wide sweep of EXACTLY THAT DEFECT PATTERN. The list of "Migration target call sites" above (line 76, line 136, ... line 1304) is also pre-shift and stale — the cycle-128 helper-add + docstring expansion shifted ~50 lines, then the multi-line reformat of line 668 shifted another line.

### Corrigendum (the cycle-127 symbolic-anchor discipline, retroactively applied)

The anchor table in the plan block above should have read (and is hereby superseded by):

- `fn apply_halo_inference_partition_aware` — greppable in `nucleus/nucleus-compiler/src/passes/halo_inference.rs`; cycle-128 verified single-definition + multiple call-site hits across nucleus/.
- `fn apply_halo_inference` — greppable in `nucleus/nucleus-compiler/src/passes/halo_inference.rs`; cycle-128 verified single-definition + multiple call-site hits.
- `## Strict vs advisory vs partition-policy-aware entry points` — greppable in the module-doc section of `nucleus/nucleus-compiler/src/passes/halo_inference.rs`; cycle-128 verified single hit (unique heading).

The "Migration target call sites" lists (lines 61-73 of this notes block) cite pre-shift line numbers. The function-name suffix of each list entry is the durable anchor; the `line NNN:` prefix is stale by ~50 lines on each entry. Reading them as "function `<name>`" is correct; reading the line numbers is wrong.

### Cycle-128 in-file comment-doc-lie hardening (architect P2 #2 + qa P2-1)

Both reviewers flagged additional comment-doc-lies introduced by the helper migration:

- `nucleus/nucleus-compiler/tests/sidecar_halo.rs` `fn lower` original docstring: the legacy clause "as the driver does it" became false post-migration (the driver now uses `apply_halo_inference_partition_aware`, not `apply_halo_inference`). FIXED: docstring rewritten to lead with "strict-A" framing + explicit note that the driver uses partition-aware-B, pointing to `lower_partition_aware`.
- `nucleus/nucleus-compiler/tests/sidecar_halo.rs` lines 734 + 925: comment blocks inside migrated test bodies still said "via the `lower()` helper" / "same `lower()` helper" — false post-migration. FIXED: both updated to cite `lower_partition_aware()` with TASK-0309 cycle 128 marker.
- `nucleus/nucleus-compiler/tests/sidecar_partition_blocks2d.rs` line 55: docstring "Mirrors `sidecar_halo.rs::lower` exactly" — false (it actually mirrors `lower_partition_aware`, which only existed after cycle 128). FIXED: docstring updated to cite the correct sibling.
- `lower_partition_aware` docstring P3-1 precision: "the offending iv carries a Partition directive" tightened to "ANY iv in the enclosing-loop scope of the typed error carries a Partition directive, otherwise advisory" — matches the production contract paragraph precisely.

### The durable lesson (folded into `feedback-comment-doc-lie-recurring` cycle-128 update)

Closing a defect-class sweep in cycle N does NOT immunise cycle N+1's own work from the same defect class. The next cycle's tracker-plan-writing is at elevated risk precisely because the lessons aren't yet automatic. A pre-commit `grep` of the freshly-written notes against the just-closed defect-class pattern is the cheap mitigation; the architect review-gate is the safety net.

## Final summary (cycle 128, 2026-05-25)

TASK-0309 LANDED via option (b) helper-split. `lower_partition_aware()` added as a sibling of `lower()` in `nucleus/nucleus-compiler/tests/sidecar_halo.rs`; 7 call sites migrated from `lower(` to `lower_partition_aware(` covering task0299_06 / task0303_05_distributed-2d / task0303_07 / task0304_06 / task0304_05 / task0310_05_distributed-2d / task0310_07. The 4 strict-A pinning sites (stencil_3x3_produces_halo_one_on_both_axes, naive_example_01_produces_empty_halo_widths, and two serde-roundtrip tests) continue using `lower()`. The 5 task0275_partition_aware_* tests bypass both helpers via `build_linked_for_partition_test` and are unaffected.

### Per-AC verdict

- **AC#1 — helper split (option b)**: GREEN. `lower_partition_aware()` added at top of file next to `lower()`; signature matches `(LinkedIR, ACFG)`; calls `apply_halo_inference_partition_aware` with `(acfg, _advisory_errors)` tuple destructure mirroring the driver's pattern at driver/main.rs.
- **AC#2 — TASK-0275 strict-A tests unchanged**: GREEN. The 5 in-module strict-A tests at the top of the file (after the cycle-128 helper-add shift) continue to construct synthetic IR via build_linked_for_partition_test and call apply_halo_inference_partition_aware directly; not touched by the diff.
- **AC#3 — e2e 108/92/0/16/0 preserved**: GREEN. qa-test-runner's own measurement: `total: 108 pass: 92 fail: 0 skipped: 16 required-fail: 0`. Both strict-A and partition-aware-B agree on the halo_widths map for every shipped distributed fixture (all are fully affine), so the migration is observationally inert today — exactly as predicted at filing.

### Review gate

Parallel read-only review (qa-test-runner + mped-architect) returned GO on both:
- qa-test-runner: gate GREEN at all expected baselines (854/0/3 dev + release; 108/92/0/16/0 e2e). 1 P2 + 3 P3 findings, all comment-doc-lie class.
- mped-architect: GO with 2 P2 + 3 P3 findings, all comment-doc-lie class.

### Review-driven hardening (in-thread, before commit)

The two reviewers' overlapping P2 findings revealed a meta-recurrence of the cycle-127 (TASK-0311) defect class: the cycle-128 work itself re-introduced fresh instances of the exact defect class cycle 127 just closed:

1. **Tracker plan re-introduced `halo_inference.rs:411/471/100` absolute-line citations** (architect P2 #1). Closed by the addendum block above this final-summary.
2. **`fn lower` legacy docstring "as the driver does it" clause** (qa P2-1). After cycle-128, the driver uses partition-aware-B, not strict-A — the appended paragraph correctly clarified but the legacy adjacent clause now contradicted. Closed: docstring rewritten to lead with "strict-A pipeline" + explicit note that the driver uses partition-aware-B.
3. **Two in-file comment blocks at lines 734 + 925** said "via the `lower()` helper" inside migrated test bodies (architect P2 #2 / qa P3-2). Closed: both updated to cite `lower_partition_aware()` with the TASK-0309 cycle-128 marker.
4. **Cross-file ricochet at sidecar_partition_blocks2d.rs:55** claimed "Mirrors `sidecar_halo.rs::lower` exactly" — true at filing, false after cycle 128 (it actually mirrors `lower_partition_aware` now). Closed: docstring updated to cite the correct sibling.
5. **`lower_partition_aware` docstring precision** (qa P3-1): "the offending iv" tightened to match the production contract's "ANY iv in the enclosing-loop scope of the typed error carries a Partition directive, otherwise advisory" wording.

### Gotchas + lessons forward-carried

1. **The next cycle following a defect-class sweep is the highest-risk cycle for that exact defect class.** Cycle 127 (TASK-0311) closed the class-wide halo_inference.rs absolute-line citation sweep; cycle 128's tracker-plan-writing immediately re-introduced 3 fresh instances. The architect review-gate caught it cleanly, but the cheaper place is at the implementer's self-audit step. Pre-commit `grep -rn '<just-swept-pattern>'` on freshly-written diff before sending to the gate. **Folded into `feedback-comment-doc-lie-recurring` cycle-128 entry.**

2. **Helper-split introduces cross-file doc-lie ricochet.** Introducing a new sibling helper (e.g., `lower_partition_aware`) invalidates third-party docs that reference the unsuffixed singular name. The sibling `sidecar_partition_blocks2d.rs::lower` docstring claim "Mirrors `sidecar_halo.rs::lower` exactly" became false at cycle-128 boundary because the sibling NOW mirrors `lower_partition_aware`. Rule: when introducing a new sibling helper / variant / entry point, run `grep -rn 'helper_name' nucleus/` before commit and audit every hit for now-stale "exactly mirrors" / "as X does it" clauses.

3. **Appended-paragraph correctness does NOT immunise the legacy adjacent paragraph.** Cycle 128's docstring expansion on `fn lower` correctly stated "strict-A pipeline" in the appended block, but the legacy first paragraph still said "as the driver does it" — and the driver no longer does it that way. Multi-paragraph docstrings are independent rhetorical units; an appended correction does NOT rewrite the preceding adjacent claim.

4. **`replace_all` is justified when the substitution substring is structurally unique within a single file.** The cycle-126 sed-batch lesson cautions about substitutions across MANY FILES with SEMANTICALLY UNRELATED contexts. Cycle 128's `let (_linked, acfg) = lower(` → `let (_linked, acfg) = lower_partition_aware(` substitution was within ONE file with a unique disambiguator (`_linked` vs `linked` — the underscore prefix on the unused variable distinguishes the 7 migration targets from the 4 strict-A-kept sites that consume `linked`). The architect review-gate confirmed: cycle-126 lesson preserved, not violated.

5. **Forward-carried to future M6+ schedule-divergence work**: the new `lower_partition_aware` is observationally inert today; both variants agree on every shipped fixture (all are affine). If a future M6+ schedule lands with non-affine reads or strided kernel-arg indexes under a partitioned iv, the cycle-128 split lets the test pin halo_widths-from-partition-aware-B WITHOUT requiring the strict-A pipeline to accept the same fixture (which it can't). The helper-split is the precondition for that future divergence work.

6. **Forward-carried to TASK-0312 (broader corpus sweep)**: the same `feedback-comment-doc-lie-recurring` cycle-128 lesson applies. When TASK-0312 runs the broader stale-citation sweep, the cycle's own tracker plan is the highest-risk place for re-introducing the very citations being swept. Pre-commit grep on the freshly-written notes is the discipline.

### Files changed

- `nucleus/nucleus-compiler/tests/sidecar_halo.rs` — `lower_partition_aware()` added; 7 call sites migrated; legacy `fn lower` docstring fixed; 2 in-file comment-doc-lies fixed; docstring precision tightened.
- `nucleus/nucleus-compiler/tests/sidecar_partition_blocks2d.rs` — cross-file ricochet docstring fixed.
- `backlog/tasks/task-0309 - ... .md` — status Done + plan + cycle-128 review-hardening addendum + this final summary.
- `~/.claude/projects/-home-mpedersen-topics-mark-thesis/memory/feedback-comment-doc-lie-recurring.md` — cycle-128 lesson appended (defect-class sweep does NOT immunise its own next cycle; cross-file doc-lie ricochet on sibling-helper introduction).
<!-- SECTION:NOTES:END -->
