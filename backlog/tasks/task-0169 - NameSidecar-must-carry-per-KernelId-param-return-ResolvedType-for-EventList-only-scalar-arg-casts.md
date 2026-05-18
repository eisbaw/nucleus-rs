---
id: TASK-0169
title: >-
  NameSidecar must carry per-KernelId param/return ResolvedType for
  EventList-only scalar-arg casts
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 23:05'
updated_date: '2026-05-18 23:26'
labels:
  - M2
  - compiler
  - backend
dependencies:
  - TASK-0160
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocks TASK-0124 full byte-identical. TASK-0160 landed NameSidecar with data_types (per DataId ResolvedType), consts, and loop_bounds — sufficient for vec! pre-init sizing, slot typing, and source-form loop bounds. But pthreads-sync render_call_args (backends/pthreads-sync/src/lib.rs ~600-640) still reads ctx.algo.kernels.get(callee).params (Vec<ResolvedType>) to decide scalar-arg casts (e.g. an iter-var-derived i64 fed to a usize-typed kernel param needs a cast; whole-array vs scalar param dispatch in render_call_arg). The EventList FireBinding (TASK-0156) carries the argument VALUES but not the kernel's declared param/return ResolvedTypes, and NameSidecar (TASK-0160) carries only DATA types, not KERNEL signature types. An EventList-only backend (TASK-0124) therefore cannot reproduce the exact cast/dispatch without AlgoIR kernels. Extend NameSidecar with kernel_sigs: BTreeMap<KernelId,{params:Vec<ResolvedType>, ret:Option<ResolvedType>}> (keyed by the same KernelId Event::Fire carries; built from linked.algo.kernels via acfg.name_kernels, exactly like data_types is built from algo.data). Additive; same no-fold/no-Net-touch discipline as TASK-0160. Prove sufficiency by reconstructing a scalar-cast call arg for the e2e set from sidecar+EventList alone (TASK-0156 style). NOTE: for the current e2e set 01/02/03/05/07 the only casts are iter-var i64->usize INDEX casts (not kernel-param), so TASK-0124 may be byte-identical WITHOUT this for those 5 — but the contract is not fully AlgoIR-free until kernel sigs are in the sidecar; verify during TASK-0124 whether any of 01/02/03/05/07 actually trips render_call_arg's param_ty path before deciding TASK-0124 ordering.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 NameSidecar carries per-KernelId param + return ResolvedType, keyed by the KernelId Event::Fire carries, deterministically (BTreeMap)
- [x] #2 build_sidecar populates it from linked.algo.kernels via acfg.name_kernels (same pattern as data_types); eval_const/Net/Repeat.range untouched
- [x] #3 Sufficiency test (TASK-0156 style): a scalar-arg cast call is reconstructed from NameSidecar+EventList alone, no AlgoIR kernels walk
- [x] #4 Determinism + bit-identical e2e 01/02/03/05/07 preserved (no sidecar codegen consumer yet)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add KernelSig {params: Vec<ResolvedType>, ret: Option<ResolvedType>} + kernel_sigs: BTreeMap<KernelId, KernelSig> to NameSidecar (sidecar.rs). Replace KNOWN GAP comment block with real field + docs; update module/struct docs so the "fully AlgoIR-free" claim is accurate.
2. ir.rs: add feature-gated serde derive to ResolvedKernel (mirror TASK-0160 ScalarType/ResolvedType/ResolvedConst treatment) so the sidecar is serialisable.
3. build_sidecar: populate kernel_sigs by inverting acfg.name_kernels -> KernelId, looking up linked.algo.kernels[name] (same pattern + same fail-loud-on-desync as data_types).
4. Sufficiency test in tests/petri_to_events.rs (TASK-0156/0160 style): FINDING - none of 01/02/03/05/07 feeds an iter-var scalar expr to a scalar param (all call args are DataRef element reads), so render_call_arg param_ty scalar-cast path is NOT tripped by the e2e set. Test uses a SYNTHETIC ResolvedKernel with a scalar usize param + a constructed Event::Fire-style scalar ArgBinding to reproduce render_call_arg cast `(expr) as usize` from kernel_sigs ALONE (no ctx.algo.kernels). Also assert kernel_sigs == algo.kernels params/ret for all 5 + determinism + serde roundtrip.
5. Gate (nix develop): just test / e2e / determinism-check / determinism-check-negative / clippy -D warnings before each commit. e2e+determinism MUST stay byte-identical (no codegen consumer).
6. Forward-carry the resolved finding to TASK-0124 notes.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented kernel_sigs: BTreeMap<KernelId, KernelSig{params:Vec<ResolvedType>, ret:Option<ResolvedType>}> on NameSidecar.

DESIGN: dedicated KernelSig struct (NOT embedding ResolvedKernel) — mirrors the ConstValue/ResolvedConst precedent. ResolvedKernel has a `purity: Purity` field irrelevant to codegen; embedding it would force a serde derive on Purity. KernelSig reuses ResolvedType's existing feature-gated serde (TASK-0160) so ZERO new derives on AlgoIR types — strictly additive, minimal serde surface.

build_sidecar section (d): inverts acfg.name_kernels -> KernelId, copies linked.algo.kernels[name].{params,ret}; same fail-loud-on-desync panic as data_types section (a).

RESOLVED FINDING (the open TASK-0124 ordering question): NONE of e2e 01/02/03/05/07 trips render_call_arg's param_ty scalar-cast path. Every kernel call arg in those 5 (add(a[i],b[i]); accumulate(partials[w],a[w][i]); blur3(img_in[y-1][x],...); madd(c[i][j],a[i][k],b[k][j]); combine(partials[0],partials[1])) is an ArgBinding::Data element/whole-array read. render_call_arg's `param_ty.is_scalar()` cast branch is reachable ONLY from the IrExpr::IntLit|Ident|Neg|BinOp arm (ArgBinding::Scalar), never the DataRef arm. The test sidecar_kernel_sigs_match_algoir_for_all_e2e_examples ASSERTS this across all 5 (walks every Event::Fire, fails loudly if any Scalar arg lands on a scalar param). Confirmed passing.

CONSEQUENCE: TASK-0124 is byte-identical for those 5 WITHOUT runtime-depending on kernel_sigs — but the contract is only FULLY AlgoIR-free WITH it. So TASK-0124 is a clean mechanical switch; ordering unblocked.

AC#3 sufficiency proof uses a SYNTHETIC kernel (dilate:(i32[256],usize)->i32) + synthetic Scalar arg `i+1`, reproducing render_call_arg's exact output `((i + 1)) as usize` (double parens: render_int_expr parenthesises the BinOp, the cast wraps again — faithful to the real backend, asserted byte-for-byte) from kernel_sigs ALONE, no AlgoIR constructed at all.

GATE (nix develop): just test all groups 0 failed; just e2e 10/8/0/2 byte-identical UNCHANGED; determinism-check 8/0 byte-identical; determinism-check-negative correctly bit; clippy --workspace -D warnings clean.

ORCHESTRATOR REVIEW GATE (phase3-ralph): qa-test-runner GO + mped-architect GO ("Done correctly scoped"), both read-only. Numbers RE-RUN by reviewers: just test all 0 failed (petri_to_events 21/0; both new sidecar tests pass); just e2e UNCHANGED 10/8/0/2/required-fail 0; determinism-check 8/0 byte-identical; determinism-check-negative bites 2/2 non-flaky; clippy clean; de-risk invariant PROVEN (git diff over acfg/passes/backends/algo EMPTY — additive only). Architect: KernelSig dedicated-struct is the right MPED trade (avoids Purity/serde drag; derived-on-demand from AlgoIR each build, no runtime divergence), the resolved finding is CODE-PROVABLE (render_call_arg cast branch unreachable from DataRef arm; lib.rs:619-646) so TASK-0124 byte-identical-for-5 is not on sand, AC#3 non-circular & byte-faithful today, honesty strong, no AC-gaming. ORCHESTRATOR HARDENING: finding#3 fixed in-thread (KernelSig divergence-hazard doc; re-verified petri_to_events 21/0 + clippy clean); finding#2 ENCODED as TASK-0170 (collect_loop_bounds panic guard; dep edges task-0170->task-0160, task-0124->task-0170); finding#1 already forward-carried to TASK-0124. TASK-0169 Done is honest: all 4 ACs met + independently verified + both reviews GO.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added per-KernelId kernel signatures to the codegen-contract NameSidecar, closing the last AlgoIR read in pthreads-sync codegen.

What changed:
- sidecar.rs: NameSidecar.kernel_sigs: BTreeMap<KernelId, KernelSig{params: Vec<ResolvedType>, ret: Option<ResolvedType>}>, replacing the KNOWN GAP (TASK-0169) comment block. build_sidecar section (d) inverts acfg.name_kernels and copies linked.algo.kernels[name].{params,ret} — same pattern + fail-loud-on-desync as data_types (a). KernelSig is a dedicated struct (ConstValue/ResolvedConst precedent), reusing ResolvedType's TASK-0160 feature-gated serde, adding ZERO new AlgoIR derives. Module/struct docs updated so the fully-AlgoIR-free claim is accurate; kernel_sig() accessor + KernelSig exported.
- lib.rs: export KernelSig.
- tests/petri_to_events.rs: sidecar_kernel_sigs_match_algoir_for_all_e2e_examples (params/ret == AlgoIR for all 5 + determinism + the resolved-finding assertion) and sidecar_alone_reconstructs_scalar_arg_cast_no_algoir_walk (synthetic scalar-param kernel proves `((i + 1)) as usize` from kernel_sigs ALONE, no AlgoIR).

Why: an EventList-only backend (TASK-0124) needs the callee param types to reproduce render_call_arg's scalar-cast/dispatch decision; Event::Fire carried values (TASK-0156) but not signatures, and NameSidecar (TASK-0160) carried only DATA types.

User impact: none yet (no codegen consumer); strictly additive metadata, e2e/determinism byte-identical by construction.

Resolved finding: NO e2e example (01/02/03/05/07) trips render_call_arg's param_ty scalar-cast path — all call args are ArgBinding::Data reads. TASK-0124 is byte-identical for those 5 without runtime-depending on kernel_sigs; the contract is fully AlgoIR-free WITH it. TASK-0124 is a clean mechanical switch.

Tests: just test all groups 0 failed; just e2e 10/8/0/2 byte-identical UNCHANGED; determinism-check 8/0; determinism-check-negative bit; clippy -D warnings clean. Commit e929d63.

Limitations/risks: no e2e example exercises the kernel-param scalar-cast at runtime, so that path of TASK-0124 is proven only by a synthetic test, not an integration run. Recorded as a forward-carried note on TASK-0124.
<!-- SECTION:FINAL_SUMMARY:END -->
