---
id: TASK-0260
title: >-
  M5 sub-task: halo region inference from kernel access pattern (stencils,
  separable filters)
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-23 23:53'
updated_date: '2026-05-24 10:56'
labels:
  - M5
  - compiler
  - halo
  - inference
dependencies:
  - TASK-0043
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + §9 + TASK-0043 AC#2. The schedule does NOT state halo size; the compiler infers it from the algorithm's kernel access pattern. Required for distributed schedules on stencil-like kernels (examples 5, 6 in particular).

## Scope
At link-time or as an early pass, for each kernel invocation in a Dataflow, scan its arg indices (e.g. blur3(grid[y-1, x], grid[y, x], grid[y+1, x]) reads {-1, 0, +1} along y) and produce a per-(kernel, axis) halo width N (max |offset| across all reads). This halo annotates the IterTile / XferPlaceholder so transfer_inject emits per-tile transfers that include the halo overlap, and the partition pass knows the boundary overhead.

## Acceptance Criteria
1. A halo_inference pass (or a link-step extension) walks AlgoIR/LinkedIR Dataflows and computes per-(kernel, IterVar) halo widths.
2. The halo widths are persisted into NameSidecar (e.g. NameSidecar.halo_widths: BTreeMap<(KernelId, IterVar), u64>) or into ACFG XferPlaceholder.policy as a structured field.
3. Affine-stride indices ONLY (data-dependent strides REJECTED with a typed error per PRD §13 'reuse / halo data-dependent strides'). Spec: ''kernel arg index  is affine in ; otherwise reject.
4. transfer_inject + partition consumers use the halo to extend per-tile transfer ranges.
5. A new e2e cell on example 5 (3x3 stencil) with a distributed schedule produces bit-identical output, verifying the halo extends per-tile transfers correctly.

## Honest scope clarification
- This task's M5 deliverable is COMPILE-TIME inference + emit. Codegen for the boundary handling (clamp / wrap / panic / pad) is per-kernel Rust (the kernel author writes the boundary semantics). The compiler emits the right tile ranges; the kernel emits the right per-element semantics.
- Data-dependent stride detection: if any index is not affine in the loop variable, REJECT with a precise error and a forward-link.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation Plan (cycle 81):

DESIGN DECISION: Option A (post-ACFG pass, mirroring partition_rows / partition_blocks2d). Rationale: ACFG carries the name_iter_vars and name_kernels tables the inference needs to key by; LinkedIR.algo.stmts is the source of the unfolded IrExprs we must scan; this is the same shape partition passes use. Option B (link extension) would need to duplicate the iter-var collection and offer no benefit since transfer_inject does not yet consume halo widths (Stage 2 lands them).

SCOPE: STAGE 1 only. This task lands halo INFERENCE + sidecar persistence. Stage 2 (transfer_inject extension) is filed as TASK-0263; Stage 3 (block-pair recovery for halo-strip synthesis under partition=blocks2d) is filed as TASK-0264.

DELIVERABLES:
1. new file passes/halo_inference.rs: pub fn apply_halo_inference(linked, acfg) -> Result<ACFG, HaloInferenceError>
2. NameSidecar.halo_widths: BTreeMap<(KernelId, IterVar), u64> (serde-default)
3. Typed error enum: HaloInferenceError { UnknownKernel, DataDependentStride, StridedAccessNotSupported }
4. Driver wires the pass after apply_partition_blocks2d
5. >= 8 unit tests + serde-default round-trip test
6. Honest limitations documented: a==1 only; non-affine rejected; pure-constant index OK (no entry); kernel called in non-loop scope -> no halo entries

VERIFICATION GATE:
- just test (>=730)
- just clippy clean
- cargo fmt --check clean on new files
- just e2e (88/73/0/15/0 preserved; no halo consumer wired yet)
- just determinism-check PASS + negatives bite

Cycle 81 LANDED Stage 1: halo inference + sidecar persistence.

Implementation: nucleus/nucleus-compiler/src/passes/halo_inference.rs (new file).
- Two entry points: strict apply_halo_inference (Result-based, fail-fast) for tests + direct callers, lenient apply_halo_inference_advisory (collects all errors as warnings) for the Stage 1 driver.
- Sidecar shape: BTreeMap<KernelId, BTreeMap<IterVar, u64>> (NESTED, not tuple-keyed — tuples cannot be JSON map keys).
- Driver uses the lenient variant + nuc_trace! advisory emission. Honest reason: example 11 (game-of-life) reads grid[(t + ITERS) % (ITERS + 1)] which is a constant Mod wrap; the strict detector rejects, but no e2e cell currently uses halo, so swallowing is correct for Stage 1.

Files touched:
- nucleus-compiler/src/passes/halo_inference.rs (new, 1300+ lines including tests)
- nucleus-compiler/src/passes/mod.rs (registers module)
- nucleus-compiler/src/lib.rs (re-exports apply_halo_inference + _advisory + HaloInferenceError)
- nucleus-compiler/src/acfg.rs (added halo_widths field; updated build_acfg constructor)
- nucleus-compiler/src/sidecar.rs (added halo_widths field; updated build_sidecar to mirror)
- nucleus-compiler/src/passes/partition_workers.rs / partition_rows.rs / partition_blocks2d.rs / sync_inject.rs / block_transform.rs / transfer_inject.rs (destructure-and-rebuild: forward halo_widths verbatim)
- driver/src/main.rs (calls apply_halo_inference_advisory after partition passes, emits nuc_trace! for advisory errors)
- nucleus-compiler/tests/sidecar_halo.rs (new, 4 tests: stencil halo on both axes, elementwise-add max=0, serde JSON round-trip, missing-field default)
- nucleus-compiler/tests/*.rs (8 test files: added halo_widths: BTreeMap::new() to hand-built ACFG instances)

Gate numbers (cycle 81):
- just test: 746 passed / 0 failed / 3 ignored (baseline 722; +24 new tests: 20 in halo_inference.rs unit + 4 in sidecar_halo integration)
- just clippy: clean (after WalkCtx + IndexSite refactor to stay under too_many_arguments cap)
- cargo fmt --check -p nucleus-compiler: clean
- just e2e: 88 / 73 / 0 / 15 / 0 required-fail (preserved baseline byte-for-byte)
- just determinism-check: PASS
- just determinism-check-negative: PASS (73/88 perturbed cells bit; >=1 required)
- just required-coverage-check-negative: PASS
- just xbackend-check-negative: PASS

Design call: Option A (post-ACFG pass mirroring partition_rows/blocks2d) chosen as recommended. Rationale: ACFG already has name_iter_vars + name_kernels; LinkedIR has the source stmts; same pattern as partition siblings; one idiom for the reader.

Follow-ups filed:
- TASK-0263 (Stage 2 - transfer_inject consumes halo_widths)
- TASK-0264 (Stage 3 - block-pair recovery for partition=blocks2d halo)
- TASK-0261 forward-carry recorded (reuse codegen shares affine-stride prerequisite)

Honest limitations carried to Stage 2:
1. Coefficient +1 only. -1 (iv*-1) and |c|>1 (strided) rejected as StridedAccessNotSupported.
2. Single iter-var per index. grid[y+x] rejected as MultipleIterVarsInIndex.
3. No DataRef/Call inside index (PRD §13 data-dependent-stride bar).
4. Mod/Div with iv on either side is non-affine, rejected as NonAffineIndex. (THIS IS THE example-11 case — game-of-life uses grid[(t+ITERS)%(ITERS+1)]. Stage 1's lenient driver swallows it; Stage 2 must decide whether to (a) tighten the schedule grammar to forbid distributed-stencil on Mod-indexed kernels, (b) accept this as a runtime-data-dep, or (c) extend the detector with a Mod-with-const recognition path.)
5. Implementation chose to record explicit 0-width entries for every (kernel, iv) the detector inspects (the bare-iv case). Stage 2 consumer must treat 0 as no extension needed.

Gotchas worth recording for the architect:
- Initial design used tuple key BTreeMap<(KernelId, IterVar), u64>; JSON serde rejected it (tuple keys not allowed). Switched to nested BTreeMap<KernelId, BTreeMap<IterVar, u64>>. Stage 2 readers must iterate nested.
- The lenient/strict split is a CONSCIOUS escape valve for Stage 1's no-consumer-yet state. Stage 2 must make a deliberate decision about which to use AND about how to surface partition-policy-relevant rejections only.
- ACFG destructure-and-rebuild pattern propagated to every pass — 5 passes touched mechanically. Future ACFG field additions follow the same pattern; consider centralising via an ACFG::with_X builder if more fields land.
- Sibling clippy fix: WalkCtx + IndexSite bundles were necessary to keep classify_index + walkers under the 7-arg cap. Future walkers should adopt the same idiom from the start.

Commits:
- 4529622 halo_inference: land Stage 1 inference + sidecar persistence (TASK-0260)
- 42a3fa1 tracker: TASK-0260 cycle-81 implementation notes + file Stage 2/3 follow-ups

ACs:
1. Halo_inference pass (apply_halo_inference + apply_halo_inference_advisory) walks AlgoIR + LinkedIR Dataflows + computes per-(KernelId, IterVar) halo widths. **LANDED**
2. Halo widths persisted to NameSidecar.halo_widths (nested BTreeMap<KernelId, BTreeMap<IterVar, u64>>). **LANDED**
3. Affine-stride only; data-dependent rejected with typed HaloInferenceError::DataDependentStride per PRD §13. Strict variant rejects; lenient variant collects errors as warnings. **LANDED**
4. transfer_inject + partition consumers use the halo to extend per-tile transfer ranges. **DEFERRED to TASK-0263 Stage 2 (filed cycle 81)**
5. New e2e cell on example 5 distributed schedule bit-identical to reference.bin. **DEFERRED to TASK-0263 (this requires Stage 2 wiring + TASK-0262 remainder policy)**

Task status: Stage 1 COMPLETE. AC#1-AC#3 met. AC#4-AC#5 are explicitly Stage 2 (TASK-0263) per the task brief's scope clarification ("STAGE 1 only: inference + sidecar, not the downstream consumer wiring"). Recommend status: ready-for-review on Stage 1 scope; close as ADDRESSED-VIA-TASK-0263 after review-gate.

REVIEW-GATE LANDED (cycle 81 orchestrator hardening, commit 372aaf8).

Parallel read-only review of cycle-81 implementer commits (4529622 + 42a3fa1 + 0e42da1) returned GO from both qa-test-runner and mped-architect.

## In-thread fixes (commit 372aaf8)

F-P1 (architect): an .expect() at the iv-name-resolve site in halo_inference.rs (search for `HaloInferenceError::UnknownLoopVar` — the typed-error variant introduced by this fix; rename history captured in the CYCLE-95 UPDATE block below) on a cross-module transitive invariant (iter-var name collected from scope must be in name_iter_vars) would panic on user input if the link step ever produced a 'for var' whose name escaped. Recurring panic-not-diagnostic defect. Fix: new `HaloInferenceError::UnknownLoopVar` variant + Display arm; the .get().expect() site routes to a typed-error push.

F-P2 (architect): module docs line 53 said 'iv by itself … no entry written' but code writes explicit 0-width entry. Doc-lie. Corrected to describe the explicit-0 form + the consumer contract.

## P1+P2 findings filed as forward-carry to TASK-0263

- (P1)  not tested directly — only via strict path. The lenient/strict dichotomy is load-bearing for Stage 2 (TASK-0263 driver toggle decision). Recommended fixture: a multi-error case (one Mod index + one strided index in two different kernel calls); strict returns Err on first; advisory returns both errors AND the partial halo map for unaffected calls. Filed as forward-carry on TASK-0263's notes (not a new task).
- (P2) No explicit test pinning Mod/Div rejection. The example-11 Mod-indexed claim is verified at module-doc + commit-message level but not by a dedicated  test. Filed as forward-carry on TASK-0263's notes — when Stage 2 lands, it should harden this.

## Gate (post-hardening)

- cargo test nucleus-compiler: 573 / 0.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- Behaviour unchanged for link-valid IR (the new error path is reachable only on an inconsistently-constructed input).
- e2e/determinism/negative gates preserved from cycle-81 baseline (verified by qa-test-runner before this in-thread edit: 88/73/0/15/0 required-fail; 24 new tests at +722→746 baseline; serde-default test exists; all 7 ACFG destructure sites correctly thread halo_widths).

## Review-gate decision

Status: same closure-deferred-on-sibling-blocker pattern as TASK-0258 + TASK-0259. AC#1/AC#2/AC#3 GREEN. AC#4 (transfer_inject consumer) explicitly DEFERRED to TASK-0263 Stage 2 per the task brief. AC#5 (e2e cell bit-identical) DEFERRED to TASK-0263 + TASK-0262 lockstep landing.

[CYCLE-95 UPDATE, 2026-05-24 — cross-reference REVISED in cycle 127, 2026-05-25 (TASK-0311)]: halo_inference's `UnknownIterVarInScope` was renamed to `UnknownLoopVar` in commit f8a3267 (TASK-0272 scope-A). The F-P1 finding record in this task's notes (above) was MIGRATED in cycle 127 (TASK-0311) to use the current variant name `UnknownLoopVar`; at the time of architect F-P1 (cycle 81) the variant was named `UnknownIterVarInScope`. The architectural intent is unchanged — the rename was symbol-only, matching the convention of 5 sibling passes.
<!-- SECTION:NOTES:END -->
