---
id: TASK-0390
title: >-
  Bite-test for the LHS-index banding arm of
  scatter_target_replicates_whole_array (TASK-0384 review P3)
status: Done
assignee:
  - '@me'
created_date: '2026-05-31 16:41'
updated_date: '2026-05-31 18:54'
labels:
  - backend
  - scatter
  - test
  - rigour
  - halo_inference
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0384 review P3b. The scatter soundness discriminator algo_target_has_affine_partitioned_index (halo_inference.rs) was made symmetric: the Dataflow LHS path now descends lhs.indices via expr_bands_target (mirroring the RHS DataRef arm), so a buried banding access like foo[ histogram[j] ] <-- ... (j partitioned, histogram = the scatter target appearing affinely inside another arrays LHS index) keeps the scatter FATAL. This arm is ADDITIVE-CONSERVATIVE (can only keep MORE scatters FATAL, never admit a banding one) and is almost certainly UNREACHABLE in todays grammar, so it currently has NO unit test exercising it. Add a negative test mirroring task0384_bin_partitioned_scatter_rmw_stays_fatal: build a 2-statement fixture (canonical histogram[input[i]] scatter + a second stmt foo[histogram[j]] <-- g(...) with j partitioned) and assert scatter_target_replicates_whole_array(linked, histogram) == false BECAUSE of the LHS-index path (verify it bites by confirming the test fails if the lhs.indices recursion is removed). Recurring class: prove-the-guard-bites (feedback / TASK-0374/0379/0381). LOW.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add a bite-test for the LHS-index banding arm of scatter_target_replicates_whole_array (halo_inference.rs:867-873). Fixture: 2 statements — (A) canonical scatter `for i { histogram[input[i]] <-- inc(histogram[input[i]]) }`; (B) `for j { foo[histogram[j]] <-- inc(foo[j]) }` with j partition=workers. histogram appears affinely (j) inside foos LHS index → only the lhs.indices arm at 867-873 returns true. Assert !scatter_target_replicates_whole_array(linked,"histogram"). Prove it BITES by removing 867-873 and confirming the test flips/fails (manual experiment, documented). Mirror task0384_bin_partitioned_scatter_rmw_stays_fatal infra (build_linked_gather_arity, single inc kernel reused for both stmts). Gate: just test + bite-experiment + cheap subset + e2e (no codegen change → e2e additive-neutral, 371 baseline holds).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0389: the worker per-channel Wait order is now SORTED to the host per-channel Push (producer-statement) order in build_waits_for_op (transfer_inject.rs), so a distributed scatter cell whose data declaration order differs from data_in traversal order is now FIFO-correct on bufsync/poll WITHOUT relying on traversal-order coincidence. If TASK-0390 adds a scatter fixture whose LHS-index array (input) is declared after another host->worker array, the FIFO ordering is already handled by the TASK-0389 sort — but scatter currently has only ONE host->worker input per channel (histogram is a private worker partial), so the multi-data-per-channel mismatch shape does not arise for the current scatter program. The TASK-0389 producer-rank key has a known residual for loop-output Push hoist on a shared channel (TASK-0389.01).

Implemented in-thread (trivial test-only change, no production code). Added task0390_lhs_index_banding_keeps_scatter_fatal to halo_inference.rs tests + inventory-comment bullet. Fixture: 2-stmt, stmt A canonical scatter histogram[input[i]] (i partitioned), stmt B foo[histogram[j]] <-- inc(foo[j]) (j partitioned). Asserts scatter_target_replicates_whole_array(linked,"histogram")==true for A-alone (baseline replicate) and ==false for A+B (LHS-index arm bites). BITE PROVEN two ways: (1) self-contained differential A-vs-A+B where stmt B LHS-ref foo!=target and RHS inc(foo[j]) are both non-banding so the flip is attributable solely to lhs.indices arm; (2) manual remove-arm experiment — disabling the lhs.indices clause (false &&) made the A+B assert FAIL while the A baseline still passed (then restored). Gate: nucleus-compiler lib 170->171 passed; just build+clippy+test+test-release exit 0; just e2e 371/314/0/57/0 unchanged (test-only, e2e-inert). GOTCHA hit + fixed: first docstring draft tripped the recurring clippy doc_lazy_continuation trap (markdown - sub-list with un-indented continuation paragraph) — rewrote as prose Case-A/Case-A+B with a blank-line-separated Empirically-confirmed paragraph. Holding Done until independent architect review GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE. Added task0390_lhs_index_banding_keeps_scatter_fatal pinning the LHS-index banding arm of algo_target_has_affine_partitioned_index (halo_inference.rs:867-873). Bite PROVEN two independent ways: (1) self-contained differential — A-alone (histogram[input[i]], i partitioned) replicates whole-array (==true), A+B (adds foo[histogram[j]], j partitioned) does NOT (==false); architect traced that B LHS-ref foo!=target (no trip at :853) and RHS inc(foo[j]) (no trip at :874) are both non-banding, so the flip is attributable SOLELY to the :867-873 lhs.indices clause; (2) remove-arm experiment (false && short-circuit) flips A+B to fail while A baseline holds — re-run independently by the architect. Gate: nucleus-compiler lib 170->171; build+clippy+test+test-release exit 0; e2e 371/314/0/57/0 unchanged (test-only). Independent architect review GO (no P1/P2; folded 2 P3 doc-word nits in c27b8be: commenting-out->disabling/false-&&, Case1/2->CaseA/A+B). Gotcha: first docstring draft tripped clippy doc_lazy_continuation (recurring trap) — rewrote markdown sub-list to prose. Arm remains unreachable in todays grammar (additive-conservative); test guards against a future regression of the lhs.indices descent.
<!-- SECTION:FINAL_SUMMARY:END -->
