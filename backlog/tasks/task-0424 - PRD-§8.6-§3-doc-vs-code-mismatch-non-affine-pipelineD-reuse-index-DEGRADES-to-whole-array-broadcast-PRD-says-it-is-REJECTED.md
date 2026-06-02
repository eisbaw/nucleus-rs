---
id: TASK-0424
title: >-
  PRD §8.6/§3 doc-vs-code mismatch: non-affine pipeline=D/reuse index DEGRADES
  to whole-array broadcast, PRD says it is REJECTED
status: Done
assignee:
  - '@me'
created_date: '2026-06-02 02:27'
updated_date: '2026-06-02 08:37'
labels:
  - compiler
  - docs
  - prd-invariant-audit
  - cycle-241
  - doc-code-mismatch
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) GAP-5, VERIFIED. PRD §8.6 says affine indices only, the Petri-net IR does not relax this (implying REJECTION of non-affine). The code does NOT reject: affine_decompose returning None silently falls back to whole-array broadcast (transfer_inject/partition.rs:257,558,574). This is VALUE-CORRECT (whole-array is the safe superset) so NOT a soundness bug, but it is a documentation/enforcement mismatch (PRD claims rejection; code does graceful degradation). RESOLUTION OPTIONS: (a, cheapest+honest) reconcile PRD wording to non-affine indices conservatively degrade to whole-array broadcast; OR (b) if the project wants fail-loud discipline here (cf. TASK-0366 CumulativeWholeArrayFallback which WAS made fail-loud), add an ADVISORY diagnostic (not hard error, since correctness holds) when a pipeline=D/reuse-tagged loop hits the non-affine fallback. Low value for correctness; flagged as the documented-X-code-does-Y class. Pointer: src/passes/transfer_inject/partition.rs (affine_decompose None branch); PRD §8.6 line ~941-943.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation plan (cycle-242):
(a) PRD §8.6 reuse/affine bullet (PRD.md ~1261-1263): reconcile wording — non-affine/data-dependent reuse/pipeline=D indices are NOT rejected; they conservatively DEGRADE to a whole-array broadcast (value-correct safe superset). Precise per-worker tiled slices only when the index is affine AND forms a contiguous partition-covered dim prefix. Forward-pointer to TASK-0424 + compute_partition_bounds_with_dim_prefix.
(b) Advisory NUC_TRACE in compute_partition_bounds_with_dim_prefix (partition.rs ~526-580). It returns EMPTY bounds (=> whole-array) on THREE distinct paths: ambiguous-multi-iv early return (~559), sparse-after-hole early return (~575), terminal Some(bounds) with bounds.is_empty() (no partition-covered dim / opaque non-affine). Add a nuc_trace! naming data+worker+reason on all three. Stays ADVISORY — function still returns Some(...) unchanged; no hard error / debug_assert. The None paths (532/534) are NOT touched (they signal caller fall-back; caller already traces at ~284).
Gate: build+clippy+test+test-release+e2e (baseline 385/328/0/57/0 must hold; trace is env-gated so e2e bit-identity preserved).

DONE (cycle-242). Commit 2df2ff7.

(a) PRD §8.6 "Where does reuse get its information?" bullet reconciled. BEFORE: "Restrict reuse to affine-indexed loops only and reject the rest." AFTER: affine index forming a contiguous partition-covered dim prefix => precise per-worker tiled slice; non-affine/data-dependent index is NOT rejected => conservatively DEGRADES to whole-array broadcast (value-correct safe superset). Forward pointer to TASK-0424 + compute_partition_bounds_with_dim_prefix.

(b) Advisory crate::nuc_trace! added at ALL THREE empty-bounds (whole-array) return paths of compute_partition_bounds_with_dim_prefix (partition.rs): ambiguous-multi-iv early return (~586), sparse-non-prefix early return (~605), terminal Some(bounds) with bounds.is_empty() (~616). Each names data+worker+reason. Stays ADVISORY: function returns Some(...) unchanged on every path; byte-silent with NUC_TRACE unset (e2e bit-identity held). NOT a hard error/debug_assert (panic-on-valid-input avoided). The None paths (532/534) are caller fall-back, already traced by TASK-0317 at the call site (~284) — intentionally NOT touched.

GATE (all green): build OK; clippy clean (re-ran, no doc_lazy_continuation); test 1251 dev / 1249 release (+1 each from new unit test); e2e 385/328/0/57/0 (baseline held exactly).

NEW TEST: task0424_ambiguous_multi_iv_drops_to_whole_array in tests_tiles.rs pins the previously-untested ambiguous-multi-iv RETURN VALUE. The other two empty-bounds paths were already pinned (task0317_sparse_coverage_drops_to_whole_array; task0373_gather_outer_array_dim_is_opaque_not_iv_attributed exercises the terminal opaque path). No stderr-trace-text assertion: TASK-0285 removed the in-source trace capture sink, so trace TEXT is only verifiable by scraping stderr — out of scope; we pin return-value invariance instead.

GOTCHAS / FEED-FORWARD:
1. HOW non-affine reaches empty bounds: record_access_per_dim (TASK-0373) makes a data-dependent/gather index dim OPAQUE (empty iv set, sticky). An opaque dim has NO partitioned iv => per_dim_cover slot None => (if at dim 0) empty prefix => terminal bounds.is_empty() => "no partition-covered dim" trace. So the non-affine case lands on the TERMINAL path, not an early return.
2. The trace fires on LEGITIMATE value-correct sparse layouts too, not only genuine non-affine. EMPIRICALLY VERIFIED with NUC_TRACE=1: 17-spmv gather (x, data-dependent index) => "no partition-covered dim" reason; 07-matmul b[k][j] x blocks2d => "sparse-non-prefix coverage" reason. The docstring/trace are honest that this is ANY whole-array degradation, not exclusively non-affine. A future reader must NOT read the trace as "non-affine detected".
3. No double-fire: the two early returns (return Some(Vec::new())) bypass the terminal bounds.is_empty() check, so each degradation emits exactly one trace line. Verified.

REVIEW GATE CAVEAT (honest): qa-test-runner + mped-architect subagent spawn was NOT available as a callable tool this session. Per the feedback-api-overload-during-review-gate inline-fallback pattern, the orchestrator performed the review inline (full build/clippy/test/test-release/e2e + critical diff read: confirmed no doc-lie, no double-trace, behaviour unchanged, e2e bit-identity). Gate CONTENT preserved; independence-of-result lost. Low-risk change (docs + env-gated advisory trace, zero codegen-path effect).

Cycle-242 orchestrator review gate (independent, read-only) — restores the independence the implementer lost when it self-reviewed inline:
- qa-test-runner: GO. build OK; clippy clean (forced fresh re-check, no doc_lazy_continuation); test 1251 dev / 1249 release (0 failed, 3 ignored); e2e 385/328/0/57/0 x2, no flake.
- mped-architect: GO with P2 (silent-sibling) + two P3 nits, all folded back in commit 3565602:
  * P2: sibling order_halo_strip_bounds_by_data_dim degraded to whole-array at its ambiguity (od==id) and non-prefix (_-arm) branches WITHOUT the advisory trace its sibling just got — fixed: parallel NUC_TRACE added at both (repo recurring defect class #2, silent-sibling).
  * P3: terminal trace in compute_partition_bounds_with_dim_prefix listed 2 sub-causes; the 1=>arm also reaches it when a partitioned iv lacks a per-worker range (3rd sub-cause) — message + comment now enumerate all three.
  * P3 (process): review-independence restored by this gate; implementer self-review was the documented api-overload fallback but is NOT the loop norm.
Post-fold-back gate re-run by orchestrator: build OK, clippy clean, test 1251 dev / 1249 release, e2e 385/328/0/57/0 x2. TASK-0424 stays DONE — both deliverables complete + hardened.
<!-- SECTION:NOTES:END -->
