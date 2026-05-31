---
id: TASK-0390
title: >-
  Bite-test for the LHS-index banding arm of
  scatter_target_replicates_whole_array (TASK-0384 review P3)
status: To Do
assignee: []
created_date: '2026-05-31 16:41'
updated_date: '2026-05-31 17:58'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0389: the worker per-channel Wait order is now SORTED to the host per-channel Push (producer-statement) order in build_waits_for_op (transfer_inject.rs), so a distributed scatter cell whose data declaration order differs from data_in traversal order is now FIFO-correct on bufsync/poll WITHOUT relying on traversal-order coincidence. If TASK-0390 adds a scatter fixture whose LHS-index array (input) is declared after another host->worker array, the FIFO ordering is already handled by the TASK-0389 sort — but scatter currently has only ONE host->worker input per channel (histogram is a private worker partial), so the multi-data-per-channel mismatch shape does not arise for the current scatter program. The TASK-0389 producer-rank key has a known residual for loop-output Push hoist on a shared channel (TASK-0389.01).
<!-- SECTION:NOTES:END -->
