---
id: TASK-0409
title: >-
  Hardening: complete comment-doc-lie sweep of the 5 unswept large passes (+
  grep test-body/match-arm comments, not only ///)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 08:57'
updated_date: '2026-06-01 13:46'
labels:
  - hardening
  - doc-lie
  - review-pass
  - cycle-237-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0408 (cycle-237) was a bounded 10-claim spot-check; ~66 grep hits across the high-traffic passes remain unverified. UNSWEPT modules: halo_inference.rs (132KB, largest), reuse_inference.rs (65KB), host_data_relay_inject.rs (54KB), acfg_to_petri.rs/petri_to_events.rs bodies, event_plan/plan.rs claims beyond the grep listing. METHOD REFINEMENT (load-bearing, from TASK-0408 cb5fc51 fold-back): the comment-doc-lie sweep MUST grep test-body comments AND match-arm comments, NOT only /// docstrings -- the architect found a 4th eval_const-conflation sibling hiding in a #[test] body comment (common.rs:650) that the /// -only grep could not see. Extend the keyword recipe to // comments too. HONEST EXPECTED YIELD: LOW -- TASK-0408 found 9/10 claims TRUE and characterised the swept modules as in genuinely good shape (defect density is narrative-WHY accuracy, lower than expected; staleness fences saturated). File-and-defer so the avenue is durably recorded, not lost; pick up in a fresh context when higher-leverage work is exhausted. Sibling of TASK-0407 (dead-code audit) -- both are the remaining review-pass endgame dimensions.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## forward-carried from TASK-0407 (cycle-236 dead-code/limitation review-pass)

Two items bearing on the continued comment-doc-lie sweep:

1. CONFIRMED doc-lie found + fixed in commit e38267f: backend-common/src/lib.rs:75-78 convenience-root-re-export comment claimed backends reach "the most-frequently used surface" via the crate root. Reality (grepped all consumer crates): of 35 root re-exports only 4 are consumed via the crate ROOT (EmitError, elect_host_from_name_workers, elect_host_from_worker_names, render_fire_args_nostd); the other 31 are reached via the SUBMODULE path. The comment asserted the opposite usage pattern. This is the SAME class as the TASK-0408 eval_const-attribution lie: a comment describing a usage/causation FACT that is empirically false. LESSON for 0409: comments asserting "X is the common/frequent/typical path" are CLAIMS -- grep the actual call sites and count before trusting; usage-frequency claims are as falsifiable as causation claims.

2. Gotcha that affects any allow(dead_code)/grep-based doc sweep: all 8 #[allow(dead_code)] in backends/mpi-blocking/src/multi_worker.rs (lines 351-381) and backends/mpi-nonblocking/src/multi_worker.rs (lines 433-494) live INSIDE emitted string-literal preludes (let prelude = "\ ... "; out.push_str(prelude)), as do every backend KERNELS_MOD_ATTR const. A docstring/comment audit must not treat the // comments inside those string literals as compiler-level docs -- they are emitted-code comments. Confirm by locating the enclosing string-literal boundary before classifying.

Forward-carried from TASK-0410/0411 (cycle-237): two concrete lessons for the comment/doc sweep. (1) DISTINGUISH intra-doc-link [`X`] from plain markdown code-span `X` — only the bracketed form is resolved by rustdoc and can break the cargo doc gate; a stale code-span mention of a removed symbol is harmless prose but a stale [`crate::path::Removed`] is a HARD doc-link break the just-ci gate does NOT catch (it builds no docs). When a sweep finds a comment naming a symbol, classify which form it is. (2) cargo doc --workspace --no-deps stable metric: sum the per-crate generated-N-warning summary lines (currently 10 = embedded-pattern 2 + mpi-blocking 2 + mpi-nonblocking 4 + nucleus-compiler 2); raw grep -ciE warning|error gives 16 because of 4 summary lines + 2 source-context lines that contain the matched word — use the generated-N sum, not the raw grep, as the before/after invariant.

## Implementation Plan + progress (cycle-239 implementer)

Bounded spot-check of the 3 LARGEST unswept passes. Verified 12 load-bearing X-because-Y / enforced-invariant claims via real code traces:

halo_inference.rs (PRIORITIZED):
- :790-791 NOT-because/but-because (scatter h[i] affine self-read stays FATAL) = TRUE; pinned by task0384_bin_partitioned_scatter_rmw_stays_fatal.
- :187-201 example-16 advisory/fatal (B-prime uses ivs_in_index not scope) = TRUE; error_is_fatal_under_partition precise-variant arm + bprime_* tests.
- :194-198 coefficient-must-be-+1 (iv*2, -iv reject as StridedAccessNotSupported) = TRUE (coeff != 1 arm line 1411).
- :215-220 same-kernel-twice union via max = TRUE (per_iv.entry.or_insert(0) + if width > entry, line 1445-1448).
- :147/:1428 UnknownLoopVar fail-closed (typed err not panic) = TRUE.
- :1824-1853 test-body sentinel cross-file claims (task0299_06/task0303_07 assert==0; task0303_05 assert==1 strict-positive) = ALL TRUE, names resolve in tests/sidecar_halo.rs.

reuse_inference.rs:
- :211-214 ReuseSlot.length always >1 (degenerate length-1 dropped) = TRUE (finalise_accum if length<=1 continue, line 745); backed by degenerate_only_bare_iv_records_no_entry test.
- :620-623 classify_index per-axis offset SET, non-iv skip, multi-iv->MultipleIterVarsInIndex = TRUE.
- :135 axis-in-inner-key rationale = TRUE (accum keyed (DataId,axis)).

host_data_relay_inject.rs:
- :97-104 seq allocator sufficient-because (max_existing_seq + monotonic) = TRUE.
- :155-170/:290-319 in-Repeat scoping (top-level left alone) regressed-because empirical claim = TRUE (rewrite_at root inside_repeat=false; flips true only at Repeat body line 400).
- :198 driver does NOT apply pass on bufsync = TRUE (driver/src/main.rs:531 gates to mp-tcp-event||mp-uds-event).

DEFECT FOUND (1, doc-only): stale line citation acfg_to_petri.rs:486-497 for buffer_place_for (fn actually at 470; pipeline_depth_for_seq seeding at 488-499). Appeared TWICE (silent-sibling): docstring :117 + inline comment :324. Fixed BOTH to robust mechanism reference (names the field-access, no brittle line range). Plain code-spans, no [doc-link] change.

Method: grep covered BOTH /// docstrings AND plain // test-body/match-arm comments per cycle-237 lesson. No behavioral defect found behind any doc-lie. No assertion/test added (the one invariant checked, length>1, is already test-backed). HONEST LIMIT: did NOT reach acfg_to_petri.rs / petri_to_events.rs bodies (the 2 lower-priority passes) nor the ~50 remaining unverified grep hits in the 3 priority files.

## CYCLE-239 CLOSE (commit 21e5a94)

GATE (re-run this cycle, not copied): build clean; clippy clean (forced fresh -p nucleus-compiler --all-targets -D warnings, no doc_lazy_continuation); just test 1237 (dev); just test-release 1236; just e2e 385/328/0/57/0. All == prior baseline (doc-only change, expected no movement).

VERIFIED 12 claims (all TRUE except 1 stale line-citation; full list in earlier note). The single defect: cycle-163b cross-ref to acfg_to_petri::buffer_place_for cited line range :486-497 but fn is at :470 and the pipeline_depth_for_seq field-access at :488-499. Appeared TWICE (silent sibling: module docstring + Phase-3 inline comment) -- fixed BOTH. Replaced brittle line range with rot-proof mechanism reference naming the actual access self.acfg.pipeline_depth_for_seq.get(&x.seq). Adversarial precision pass caught my own first draft dropping the self. receiver -- corrected before commit.

GOTCHAS for next subagent continuing this sweep:
1. LINE CITATIONS ARE THE DOMINANT DOC-LIE CLASS in these passes. The X-because-Y narrative content was 100% accurate across all 12 spot-checks; the ONLY lie was a hardcoded file:line citation that rotted. When you continue, grep for the pattern <file>.rs:<digits> across the remaining passes -- that is where the yield is, NOT the prose claims. Fix by naming the symbol + mechanism, never re-stamp a fresh line number (it just re-rots).
2. The 3 priority passes are in genuinely good narrative shape (matches TASK-0408 LOW-yield prediction). Halo/reuse/host_data_relay X-because-Y claims are heavily cross-tested.
3. test-body sentinel comments (e.g. halo_inference.rs:1824 cross-referencing task0299_06/task0303_07/task0303_05 in tests/sidecar_halo.rs) DO make verifiable cross-file claims -- I verified all three names resolve with the asserted ==0/==1 forms. Clean.

HONEST LIMITS (NOT reached):
- acfg_to_petri.rs (555 LoC) + petri_to_events.rs (445 LoC) bodies -- the 2 lower-priority passes named in the title. NOT swept this cycle (budget). Their // and /// comments are unverified.
- ~50 remaining keyword-grep hits in the 3 priority files beyond the 12 spot-checked. Avenue stays OPEN; this was a bounded honest spot-check, not exhaustion.
- event_plan/plan.rs claims (mentioned in task desc) NOT reached.

Status decision: the 3-large-file bounded spot-check IS complete (fixed the lies found, gate green, would stand behind every correction). The 2 lower-priority passes + remaining hits are explicitly deferred -- recommend keeping TASK-0409 OPEN (or filing a thin follow-up) rather than Done, since the title says 5 passes and only 3 were reached.

ORCHESTRATOR REVIEW GATE (cycle-240): architect read-only GO on 21e5a94 (orchestrator self-ran gate: build clean, clippy -D warnings exit 0 forced-fresh no doc_lazy_continuation, nucleus-compiler lib 178 passed, e2e codegen-invariant for comment-only). Architect INDEPENDENTLY spot-checked 3 of the implementer 11-TRUE verdicts (halo_inference scatter-rejection-not-banding @790, advisory/fatal-keys-off-ivs_in_index @187, host_data_relay in-Repeat inside_repeat-flag @155) -- ALL SOUND and test-pinned, not accidentally-correct. Citation fix confirmed accurate (buffer_place_for at acfg_to_petri.rs:470 reads pipeline_depth_for_seq.get at 494-497). FOLD-BACK (commit fcf4f35): architect found a left-behind silent-sibling line-citation; orchestrator recursive-grep sweep (architect grep was non-recursive passes/*.rs, MISSED the transfer_inject/ split subdir) found TWO remaining brittle file.rs:NNN cites and converted both to rot-proof symbol refs: (1) host_data_relay_inject.rs:27 accurate-but-brittle; (2) transfer_inject/elision.rs:101 was a LIVE STALE doc-lie (cited algo/ir.rs:256-260 for DoubleAssignment but that is the collect_dataref_names docstring; DoubleAssignment is at ir.rs:328/405 + lower.rs:1038). Recursive grep now confirms ZERO file.rs:NNN line cites remain in passes/ (all split subdirs). KEY FINDING: narrative X-because-Y claims are 100pct accurate across this + TASK-0408 (12/12 + prior); the ONLY doc-lie yield in the large passes is line-citation rot, now CLOSED for passes/. RE-SCOPE (per architect): line-citation residue CLOSED; remaining TASK-0409 scope = acfg_to_petri.rs + petri_to_events.rs body prose (~1000 LoC, 2 unreached passes) + event_plan/plan.rs -- LOW yield (prose corroborated 100pct accurate). Staying In Progress for that thin low-yield tail; a fresh session can finish or deem exhausted.

CYCLE-241 CLOSE (commit c458213, fresh session). Finished the thin low-yield tail the cycle-240 architect re-scope left open: swept the 2 unreached passes (acfg_to_petri.rs, petri_to_events.rs) + backend-common event_plan/plan.rs. Method per cycle-237 lesson: grepped /// docstrings AND // test-body/match-arm comments AND file.rs:NNN line-cites (ZERO remain anywhere). FOUND + FIXED 2 internal-contradiction doc-lies, both in acfg_to_petri.rs module docs: (1) partition_workers/rows/blocks2d called downstream while the next clause says acfg_to_petri consumes their already-replicated ACFG — they run UPSTREAM (driver main.rs 304/317/329 before acfg_to_net 585/625); reworded to upstream + they run BEFORE this pass. (2) buffer==0 rejection attributed to the expect-guard; the real loud guard is assert!(cap_u64 GT 0) in buffer_place_for (the NonZeroU32 expect can never fire on a 0); reworded to name both guards correctly. petri_to_events.rs + event_plan/plan.rs verified CLEAN: every X-because-Y claim traced TRUE; plan.rs CHECK-ORDER note (A->B->C->host-barrier->D) matches code exactly; PRD section 8.1/8.2/8.3/8.4/8.6 refs resolve; cross-file test host_excluding_barrier_is_typed_contract_gap resolves; push_wait_pair_covers / splice_pushes_for_waits / validate_event_lists_strict_per_worker all exist. GATE (orchestrator-re-run): build+clippy clean (forced-fresh, no doc_lazy_continuation), test+test-release green, e2e 385/328/0/57/0 baseline preserved (doc-only, e2e-inert). architect read-only GO: both fixes independently traced TRUE, no new lie introduced, no clippy/doc-link hazard. SCOPE NOW COMPLETE: all 5 large passes (halo/reuse/host_data_relay cycle-239 + acfg_to_petri/petri_to_events now) + event_plan swept. DURABLE FINDING across TASK-0408+0409: narrative X-because-Y prose in the passes is ~100pct accurate; the entire doc-lie yield was (a) stale line-citations (closed cycle-239/240) and (b) 2 internal-contradiction directional/wrong-guard slips — cross-checking a docstring that explains the same property twice is the highest-yield tell.
<!-- SECTION:NOTES:END -->
