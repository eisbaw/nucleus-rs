---
id: TASK-0124
title: 'pthreads-sync: emit per-worker EventList instead of walking AlgoIR'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 02:13'
updated_date: '2026-05-19 00:23'
labels:
  - M2
  - backend
dependencies:
  - TASK-0170
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0020 codegen walks AlgoIR statements directly because the ACFG strips index expressions and the per-worker EventList (Fire/Alloc/Push/Wait/Sync/Free) is not yet produced (waits on TASK-0027). Once TASK-0027 lands, the backend should consume per-worker EventLists rather than the AlgoIR. This unifies tier-1 backends around the EventList contract (PRD §7.4 / §8.3) and eliminates the LinkedIR dependency from the emit signature.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 emit() signature changes to (per_worker_event_lists: BTreeMap<WorkerId, Vec<Event>>, kernels_rs_path, out_dir, sidecar_name_map).
- [x] #2 Codegen no longer references AlgoIR/LinkedIR; only Event-typed input.
- [x] #3 All existing tier-1 backends agree on this contract before M3 lands.
- [x] #4 Depends on TASK-0027.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Build fresh master oracle: run full gate on clean HEAD, snapshot every generated main.rs/Cargo.toml/run.sh from nucleus/target/e2e-determinism into /tmp/oracle_0124.
2. AC#1: change pthreads-sync emit() to (per_worker: BTreeMap<WorkerId,Vec<Event>>, name tables, NameSidecar, kernels_rs_path, out_dir). Driver builds acfg_to_events(&acfg)+build_sidecar(&linked,&acfg)? and passes.
3. AC#2 single-worker: rewrite render_main_rs as an Event walk (Loop/Fire/Sync) using FireBinding+name tables+sidecar (reconstruct calls per eventlist_alone_reconstructs_stencil_kernel_call; loop bound source-form via sidecar.loop_bounds+consts; pre-init from sidecar.data_types over indexed Fire outputs). NO AlgoIR/LinkedIR in codegen path.
4. multi_worker.rs: assess whether EventList Push/Wait now suffices to drop LinkedIR dep for byte-identical ex02. If yes, switch; if not, HONEST PARTIAL with filed blocker + precise note.
5. Reconcile review finding (7): point sufficiency tests at REAL backend render fns.
6. Verify byte-identical vs oracle for 01/02/03/05/07 (incl blocked). Full gate green before every commit. Preserve NUC_NONDET_TEST nonce.
7. Honest final report + forward-carry to TASK-0036.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
STALE "BLOCKED" note (value-binding gap) is now SUPERSEDED: TASK-0150 (index exprs through ACFG) and TASK-0156 (Event::Fire carries FireBinding) ARE landed on master. The VALUE gap that note described is closed — proven by eventlist_alone_reconstructs_stencil_kernel_call (rebuilds the exact blur3 call from the EventList ALONE). However, full-pipeline investigation (this session) found a SECOND, distinct blocker the original note did not identify: LOOP STRUCTURE + TYPES/CONSTS, not values.

INVESTIGATION (this session, against master, gates verified green 7/0/3 before/after — NO code changed):

Built master generated trees for 01/02/03/05/07 into /tmp/nuc_master_NN as the byte-identical oracle. Master pthreads-sync emits ROLLED loops with symbolic bounds:
- ex01: `for i in (0_i64)..(256_i64)`
- ex05: `for y in (1_i64)..((16_i64 - 1_i64))` (note: H-1 kept UNEVALUATED — lives only in AlgoIR loop bounds; ACFG Repeat.range is already 1..15)
- ex03/07: nested rolled `for w/i`, `for i/j/k`
- ex02 (multi-worker): rolled loops on BOTH host & w0 + Slot<Vec<i32>> typing + barrier-in-loop; multi_worker.rs DELIBERATELY ignores ACFG Xfer nodes and synthesises from LinkedIR data_producers/consumers (documented cross-scope Push/Wait imbalance in petri_to_events.rs).

acfg_to_events UNROLLS every ACFGNode::Repeat into N flat identical Fire copies (test repeat_unrolls_in_event_list; petri_to_events.rs + acfg_to_petri module docs state this M2 trade explicitly). Therefore an EventList-only walk emits 256 / 196 / etc UNROLLED kernel statements, NOT the rolled for-loop -> NOT byte-identical to master, AND emit.rs::main_rs_calls_every_kernel asserts the rolled `256_i64` bound (would fail).

Additionally the per-worker EventList + the proposed NameSidecar carry NO data ResolvedType (needed for `vec![0; 256]` pre-init sizing, `Vec<i32>` slot type, scalar-arg casts) and NO const values / unevaluated const bound exprs. DataSlice = DataId + index IrExprs only.

CONCLUSION (honest, per CLAUDE.md no-workaround / no-fake-success): TASK-0124 AC#1+AC#2 as specified (drop acfg/linked from emit(); codegen consumes ONLY Event + sidecar) is NOT achievable byte-identically under the current Event contract. This is the loop-structure/type analogue of the value-binding gap TASK-0156 fixed. Re-rolling unrolled Fires heuristically, or keeping an AlgoIR walk behind a new signature, would be a workaround / fake AC#2 — explicitly refused.

Deliberately did NOT introduce a NameSidecar + dual signature while still walking AlgoIR: that satisfies NEITHER AC#1 (acfg/linked not actually removed) NOR AC#2 (still AlgoIR), churns the API, and risks the byte-identical invariant for zero real AC progress. Honest partial > fake complete.

Filed precise blockers: TASK-0159 (Event contract must carry loop-nest structure / stop blanket unroll) and TASK-0160 (NameSidecar must carry per-DataId ResolvedType + const values). TASK-0124 needs BOTH landed first, THEN the emit() switch is mechanical (the value half is already proven by TASK-0156). Added deps task-0159, task-0160. No code committed (none written); backlog-only changes.

Forward-carried from TASK-0142: the reason this migration is non-trivial — render_main_rs currently emits loops by walking LinkedIR::algo source IrStmt directly (lib.rs ~25-33,78-81), NOT the ACFG/Event stream. acfg_to_petri/petri_to_events unroll Repeat by range length, so the Event stream has no rolled-loop / iter-var / symbolic-bound info (depends on TASK-0159/0160). When EventList-only codegen lands, block=N tiling (TASK-0142) will need to become visible in emitted code: today 05-stencil/blocked passes only because single-worker codegen ignores the tiled ACFG and the result is schedule-independent. A correct EventList path must emit the (tile-loop, intra-tile-loop) nest INCLUDING the trailing partial tile (static Sequence[full-nest, partial-tile] shape produced by block_transform) — verify the per-worker EventList projection preserves that structure rather than re-flattening it.

Forward-carried from TASK-0159 (commit ee309ff): the per-worker EventList is now STRUCTURE-PRESERVING for loops. A loop projects to Event::Loop { iter_var: IterVar, range: Range<i64>, body: Vec<Event> } (mirrors ACFGNode::Repeat), NOT N flat unrolled Fires. To emit a rolled `for` you walk Event::Loop and emit `for <iter_var> in range.start..range.end { <body> }` — Fires/Push/Wait/Sync inside the loop are NESTED in body, recurse into them. A nested loop is a nested Event::Loop.

TRAILING PARTIAL TILE (block= / tiling): a non-divisible block decomposes to TWO SIBLING Event::Loops with DIFFERENT ranges in the same worker list (full-tile loop + shorter trailing-partial-tile loop), NOT one parameterised loop. Emit each sibling loop verbatim in order; do not try to merge them.

EDGE CASES: (a) a worker that does nothing inside a loop gets NO Loop at all (not an empty-bodied one) — so absence of a Loop = that worker is idle in that scope. (b) A degenerate/empty range (e.g. 5..5) is still emitted as an Event::Loop with that empty range — emitting `for v in 5..5 {}` (zero iterations) is correct and faithful.

LIMITATION you will hit for full AC#2 byte-identical: range is a CONCRETE Range<i64> (e.g. 1..15), the symbolic bound (H-1) is already folded by build_acfg and does NOT reach the EventList. Rendering `(16_i64 - 1_i64)` verbatim is blocked on TASK-0160; with TASK-0159 alone you can only render the concrete `1_i64..15_i64`. TASK-0124 full AC#2 needs BOTH TASK-0159 (loop structure, done) AND TASK-0160 (symbolic bound + ResolvedType/const sidecar).

Forward-carried from TASK-0160 (commit 4a79d6e): the TYPE/CONST/SYMBOLIC-BOUND half is now landed. Both TASK-0124 AC#2 prerequisites exist: TASK-0159 (Event::Loop loop structure) + TASK-0160 (NameSidecar).

EXACT SIDECAR SHAPE TASK-0124 CONSUMES:
compiler::sidecar::NameSidecar { data_types: BTreeMap<DataId,ResolvedType>, consts: BTreeMap<String,ConstValue{ty:ScalarType,value:i64}>, loop_bounds: BTreeMap<IterVar,LoopBound{lo:IrExpr,hi:IrExpr}> }. Build it once after build_acfg via compiler::build_sidecar(&linked, &acfg) (it needs LinkedIR for the UNEVALUATED for-bounds + consts that build_acfg folds away). Keyed by the SAME DataId/IterVar the EventList carries (acfg.name_data / acfg.name_iter_vars) — direct join, no name round-trip.

HOW THE BACKEND GETS EACH PIECE FROM EventList+NameSidecar (no AlgoIR):
- vec! pre-init length: sidecar.alloc_len(did) = product(data_types[did].dims) (scalar dims==[] -> 1). Element/zero literal + slot type: data_types[did].scalar (ScalarType) -> same match arms as pthreads-sync rust_scalar_type/rust_scalar_zero (i32 -> Vec<i32>, Arc<Slot<Vec<i32>>>, "0"). did from Event::Alloc.data / DataSlice.data.
- scalar-arg casts: same ScalarType from data_types (or the kernel param type — currently from AlgoIR; if a backend needs per-param scalar types beyond data, that is a SEPARATE small gap, see follow-up TASK-0161 below; for the e2e set 01/02/03/05/07 the casts in question are iter-var i64 -> usize index casts which do not need the sidecar).
- rolled loop: walk Event::Loop { iter_var, range, body }. For the SOURCE-form bound do NOT use range (folded, e.g. 1..15) — look up sidecar.loop_bounds[iter_var] and render lo/hi with sidecar.consts: Ident resolved via consts -> `{value}_i64`, BinOp -> `({l} {op} {r})`, IntLit -> `{v}_i64`. That reproduces pthreads-sync render_const_expr EXACTLY: 05-stencil emits `for y in (1_i64)..((16_i64 - 1_i64))`. If a loop var has NO loop_bounds entry it is a block_transform-synthesised tile loop (no source form) -> use the concrete Event::Loop.range (correct: a synthesised tile loop has no source bound). Proven by tests/petri_to_events.rs::sidecar_renders_stencil_symbolic_loop_bound_in_source_form + sidecar_alone_sizes_preinit_and_types_slots_for_all_e2e_examples.

WHAT REMAINS FOR TASK-0124 (the actual switch — NOT done here):
1. Change pthreads-sync emit() signature to (per_worker_event_lists: BTreeMap<WorkerId,Vec<Event>>, name tables, NameSidecar, kernels_rs_path, out_dir) per AC#1.
2. Replace render_main_rs AlgoIR walk with an EventList walk: recurse Event::Loop (emit `for {var} in ({lo})..({hi}) {{`), Event::Fire (reconstruct call from FireBinding + name tables, TASK-0156 pattern eventlist_alone_reconstructs_stencil_kernel_call), Event::Alloc/Push/Wait/Sync/Free. Pre-init: walk Event::Alloc (or the indexed-LHS set) -> vec! from sidecar.
3. multi_worker.rs: same sidecar for Slot<Vec<i32>> typing; note multi_worker DELIBERATELY ignores ACFG Xfer and synthesises Push/Wait from LinkedIR data_producers/consumers (documented cross-scope imbalance) — TASK-0124 must decide whether the EventList Push/Wait pairing (post TASK-0136/0139 finaliser) now suffices to drop that LinkedIR dependency too.
4. Verify byte-identical: the emit.rs unit test asserting the rolled `256_i64`/`(16_i64 - 1_i64)` bound must still pass; e2e 01/02/03/05/07 byte-identical; trailing-partial-tile (block=) = two sibling Event::Loops with DIFFERENT ranges (forward-carried from TASK-0142/0159) must emit BOTH verbatim.
5. Map check: a worker idle in a loop scope gets NO Event::Loop (absence = idle); a degenerate range still emits a Loop with empty range.

TASK-0124 AC#2 is now achievable byte-identically (no workaround / no AlgoIR smuggle). The value half (FireBinding) was TASK-0156; loop structure TASK-0159; types/consts/symbolic-bound TASK-0160. The switch is mechanical from here.

CORRECTION to prior forward-carry note: the per-kernel-param-type follow-up was filed as TASK-0169 (not the placeholder "TASK-0161" mentioned inline above). TASK-0169 extends NameSidecar with per-KernelId param/return ResolvedType for render_call_arg scalar-arg casts. For e2e 01/02/03/05/07 the only casts are iter-var i64->usize INDEX casts (not kernel-param), so TASK-0124 may be byte-identical for those 5 WITHOUT TASK-0169 — but verify during TASK-0124 whether any trips render_call_arg param_ty before finalising dependency ordering. Added dep task-0124->task-0169.

ORCHESTRATOR-FOLDED review findings from the TASK-0160 gate (act on these when implementing the backend switch): (1) The "01/02/03/05/07 byte-identical WITHOUT TASK-0169" claim is sound reasoning but STATICALLY UNPROVEN — render_call_arg applies a param_ty cast only for scalar-arith args (IntLit|Ident|Neg|BinOp) into a scalar param; DataRef element reads never consult param_ty. Verify per-kernel for the 5 examples whether any trips that path; if any does, TASK-0169 is a hard prerequisite, else it can follow. Do NOT treat byte-identical-without-0169 as established. (2) LANDMINE: build_sidecar HARD-PANICS if two same-named loops have different bounds (shared IterVar cannot represent both). No current example hits it but it is a compiler-runtime panic; add an explicit guard/AC before the EventList codegen path goes live (also relevant to TASK-0167). (3) DRIFT RISK: TASK-0160 sufficiency test re-implements render_const_expr/rust_scalar_* (they need RenderCtx); when emit() becomes EventList-based, switch the backend to consume the sidecar via the REAL render functions so the byte-match cannot silently drift. (4) Sidecar has no production consumer until you land — it is a proven honest-placeholder; closing TASK-0124 is what makes it live.

[forward-carried from TASK-0169 — RESOLVES the open ordering question the architect flagged]

TASK-0169 landed (commit e929d63): NameSidecar now carries kernel_sigs: BTreeMap<KernelId, KernelSig{params:Vec<ResolvedType>, ret:Option<ResolvedType>}>, keyed by the same KernelId Event::Fire carries. With it the (EventList + name tables + NameSidecar) contract is FULLY AlgoIR-free for pthreads-sync codegen — the last known gap (render_call_arg's ctx.algo.kernels read) is closed.

RESOLVED FINDING: NONE of the e2e set 01/02/03/05/07 trips render_call_arg's param_ty scalar-cast path. Every kernel call argument in those 5 is an ArgBinding::Data element/whole-array read (add(a[i],b[i]); accumulate(partials[w],a[w][i]); blur3(img_in[y-1][x],...); madd(c[i][j],a[i][k],b[k][j]); combine(partials[0],partials[1])). render_call_arg's `param_ty.is_scalar()` cast branch is reachable ONLY from the IrExpr::IntLit|Ident|Neg|BinOp arm (ArgBinding::Scalar), never the DataRef arm — so for the 5 e2e examples no kernel-param scalar cast is emitted. The cast actually emitted in the e2e set is the iter-var i64->usize INDEX cast (render_flat_index), which is unrelated to kernel_sigs.

CONSEQUENCE for TASK-0124 ordering: the AlgoIR->EventList switch is byte-identical for 01/02/03/05/07 WITHOUT TASK-0124 needing to consult kernel_sigs at runtime — but the contract is only fully AlgoIR-free WITH TASK-0169 present. TASK-0124 is therefore a clean MECHANICAL switch (no behaviour-bearing reconstruction needed for the 5; just route render_call_arg's param_ty via NameSidecar.kernel_sig(Event::Fire.kernel).params instead of ctx.algo.kernels[callee].params for the general case so the path is AlgoIR-free for programs that DO hit it).

LIMITATION (honest): the kernel-param scalar-cast path is proven AlgoIR-free only by a SYNTHETIC test (sidecar_alone_reconstructs_scalar_arg_cast_no_algoir_walk: dilate:(i32[256],usize)->i32 + Scalar arg `i+1` -> `((i + 1)) as usize` from kernel_sigs alone), NOT by an e2e integration run, because no e2e example exercises it. If TASK-0124 adds/uses an example with a Scalar-arg-to-scalar-param call, it must add an e2e cell — and note the double-paren faithful output `((expr)) as T` (render_int_expr parenthesises a BinOp, the cast wraps again).

STILL-OPEN LANDMINES folded earlier (carry into TASK-0124 implementation):
- same-name-loop panic: collect_loop_bounds in sidecar.rs PANICS if two loops share a var name with DIFFERENT bounds (a shared IterVar cannot represent both). No e2e example hits it; TASK-0124 must not introduce one without addressing this.
- sufficiency-test render-fn drift: the test helpers (render_int_expr_mirror, render_bound_from_sidecar, elem_type_from_sidecar, zero_lit_from_sidecar) HAND-MIRROR pthreads-sync render_int_expr/render_const_expr/rust_scalar_type/rust_scalar_zero. They will silently drift if the backend's spelling changes. When TASK-0124 switches the backend to the sidecar, REPLACE these mirrors with the real backend functions (or assert against real codegen output) to remove the drift risk.

ORCHESTRATOR-FOLDED from TASK-0169 review gate (both GO): (5) RESOLVED the open ordering question — NONE of 01/02/03/05/07 trips render_call_arg param_ty scalar-cast path (DataRef args never consult param_ty; only Scalar args into scalar params do; code-provable at lib.rs:619-646 + machine-asserted by sidecar_kernel_sigs_match_algoir_for_all_e2e_examples). So TASK-0124 is byte-identical for the 5 WITHOUT runtime-needing kernel_sigs, AND the contract is now FULLY AlgoIR-free (last gap closed by TASK-0169). (6) The same-name-loop collect_loop_bounds panic is now a first-class task TASK-0170 (dep edge added: task-0124 -> task-0170) — TASK-0124 must not let the EventList path reach that bare panic. (7) Finding #1 (replace hand-mirrored render_int_expr/elem_type/zero_lit/render_bound helpers in tests with the REAL backend fns when emit() is EventList-based) stands — the mirror-drift goes live the moment TASK-0124 consumes the sidecar.

[forward-carried from TASK-0170]
- The build_sidecar same-name-loop bound-conflict PANIC is now a TYPED error. The EventList-only path you build here is panic-SAFE: build_sidecar can only return a clean SidecarError, never abort.
- build_sidecar SIGNATURE CHANGED: pub fn build_sidecar(linked: &LinkedIR, acfg: &compiler::acfg::ACFG) -> Result<NameSidecar, compiler::sidecar::SidecarError>. When you wire build_sidecar into the driver/backend, propagate the Err via the existing String-error channel exactly like apply_block_transforms: `.map_err(|e| format!("sidecar error: {e}"))?` (driver prints `nucleus: error: ...`). SidecarError impls Display + std::error::Error; re-exported as compiler::SidecarError / compiler::sidecar::SidecarError.
- Reachability finding: a same-name-different-bounds loop pair (e.g. `for i:0..N {..} for i:0..M {..}`, distinct data so single-assignment holds) is a VALID program that reaches this error. ACFG::name_iter_vars assigns one IterVar per NAME so both loops collapse onto one Event::Loop.iter_var / one loop_bounds entry. Until TASK-0171 lands, such programs are a typed compile error (cannot be codegenned) — do NOT attempt to special-case it in the backend; surface the error.
- Option-c deep fix (distinct IterVar identity so these COMPILE) = TASK-0171 (depends on TASK-0170).

IMPLEMENTED (the actual switch). emit() signature is now (per_worker: &BTreeMap<WorkerId,Vec<Event>>, names: &NameTables, sidecar: &NameSidecar, kernels_rs_path, out_dir) — &acfg/&linked DROPPED (AC#1). Driver builds acfg_to_events(&acfg)+build_sidecar(&linked,&acfg).map_err(|e|format!("sidecar error: {e}"))? + reverse NameTables (incl inner_block_iter_vars). Codegen path imports ONLY compiler::event + compiler::sidecar + the INERT IrExpr/IrBinOp/ResolvedType/ScalarType grammar the EventList itself carries — NO AlgoIR/LinkedIR/ACFG/algo.kernels/data_producers, single-worker AND multi_worker (AC#2 honest, grep-verified).

Single-worker: render_main_rs walks Event::Loop/Fire/Sync. Pre-init from FireBinding indexed outputs sized via sidecar.data_types (sorted by name). Calls reconstructed from FireBinding+name tables (eventlist_alone_reconstructs_stencil_kernel_call pattern). Loop bound: source-form via sidecar.loop_bounds+consts; synthesised tile loop (no loop_bounds entry) -> concrete Event::Loop.range. render_call_arg param cast via sidecar.kernel_sig (TASK-0169), not algo.kernels.

multi_worker: rewired to EventList. Slots from Push/Wait DataIds (sorted=old xfers order), type from sidecar.data_type. Barrier id = per-worker PRE-ORDER Sync index; VALIDATES uniform participants and fails loud (ContractGap) on partial-barrier (Event::Sync has no stable cross-worker id — filed TASK-0172). NUC_NONDET_TEST nonce preserved verbatim. Shared renderers via pub(crate) shims (one impl, no single/multi drift).

KEY GOTCHA / latent bug TASK-0124 SURFACED: old AlgoIR backend emitted UNTILED code for 05/07 blocked (walked source IrStmt; block_transform only rewrites ACFG). The EventList faithfully carries the tiled nest, so the EventList backend MUST do the absolute-index rebinding block_transform DEFERS to codegen (its module doc line ~83: codegen computes LO+tile*N+inner). Without it 07-matmul madd (accumulator) double-counted -> e2e FAIL/diff. Implemented rebinding for the EVENLY-DIVISIBLE single-nest case (07 block=8 N=16): inner loop emitted over concrete 0..N, body var substituted (0_i64 + (tile*N_i64) + inner) via abs_subst threaded in RenderCtx (empty for every non-blocked program => byte-identical there). NON-DIVISIBLE/trailing-partial (05 block=4) NOT rebound (full vs partial nest need different bases LO+tile*N vs LO+num_full*N; EventList lacks num_full) -> filed TASK-0173; 05-blocked stays runtime-correct only because blur3 is idempotent (honest).

Finding (7) reconciled: added pthreads-sync/tests/emit.rs::golden_real_codegen_strings_pin_sidecar_consumption pinning exact REAL emit() strings (05 symbolic bound, 07 rebinding); petri_to_events.rs mirror doc now points at it (compiler cannot dep on backend, so a backend golden test is the anti-drift anchor).

GATES (nix develop): just test workspace 0 failed (incl emit.rs 256_i64 assertion green); just e2e total 10 pass 8 fail 0 skipped 2 required-fail 0; just determinism-check 8/0 byte-identical; just determinism-check-negative correctly bites; cargo clippy --workspace -- -D warnings (the required gate / just clippy) CLEAN. Per-example byte/runtime: 01 naive, 02 naive(single), 02 split(multi), 03 naive, 05 naive, 05 blocked, 07 naive, 07 blocked ALL e2e PASS + determinism byte-identical. (Pre-existing clippy --tests len-zero in untouched acfg_to_petri.rs is NOT mine — stash-verified; required gate excludes --tests.)

OBSOLETE TEST removed: emit.rs::distributed_placement_is_rejected — it asserted a backend rejection that TASK-0124 deliberately moves out of the backend (no LinkedIR in emit() => AC#2); coverage now upstream/capability + e2e SKIP (TASK-0117). Documented in-file.

ORCHESTRATOR REVIEW GATE (phase3-ralph, the keystone): qa-test-runner GO + mped-architect GO, both read-only, scrutinised the central honesty question hard. Numbers RE-RUN by reviewers: just test 437 passed/0 failed/1 ignored (emit.rs rolled-bound 256_i64 + (16_i64-1_i64) assertions pass; new golden test passes); just e2e 10/8/0/2/required-fail 0 x3 identical (output.bin==reference.bin = real runtime correctness incl 05/07-blocked vs hand-written Nucleus-independent oracle); just determinism-check 8/0 byte-identical x2; determinism-check-negative bites x2 (NUC_NONDET_TEST nonce survived multi_worker rewrite); clippy clean. AC#2 AlgoIR-free VERIFIED by grep both lib.rs+multi_worker.rs (only inert IrExpr/IrBinOp/ResolvedType/ScalarType + Event/sidecar; no AlgoIR/LinkedIR/ACFG/data_producers/statement-walk — LinkedIR dep genuinely moved to the driver = correct MPED composition). HONESTY QUESTION RESOLVED HONEST: architect verified via git that old render_main_rs(&linked.algo) walked AlgoIR source and block_transform only rewrites the ACFG -> old backend provably emitted UNTILED code for blocked -> literal source-identity impossible by construction; runtime-correct-vs-independent-reference + determinism is the only sound signal (NOT a goalpost move). ORCHESTRATOR DIRECTLY OBSERVED the one un-observed claim: built 01-elementwise-add + 03-reduction with the OLD backend (worktree @ 54b872b^) vs NEW (HEAD) and diff -rq = BYTE-IDENTICAL for both non-blocked cells (source-identity confirmed where the old backend WAS a valid oracle). removed test distributed_placement_is_rejected legitimately obsolete (AC#2 removes LinkedIR from emit() so backend cannot validate placement; keeping it would test a responsibility the layer no longer has). TASK-0172 (Sync barrier identity by pre-order index; uniform->byte-identical, non-uniform->ContractGap fail-loud) + TASK-0173 (non-divisible blocked accumulator would be wrong; 05-blocked safe via blur3 idempotence, traced/verified; no required cell hits it; precise filed blocker) = honest fail-loud partials, dependents of 0124, not unmet ACs. ORCHESTRATOR HARDENING: 3 stale comments fixed in-thread (e2e-matrix.toml 05-blocked "single-worker emits from source so byte-identical" was correctness-MISLEADING post-0124 -> rewritten to the tiled+blur3-idempotent+TASK-0173 truth; 07-matmul/blocked.sched.nuc "still ignored" -> ACTIVE/required; pthreads-sync Cargo.toml "Read LinkedIR/ACFG types" -> AlgoIR-free contract); verified inert (e2e 8/0/2, determinism 8/0 byte-identical preserved). TASK-0124 Done is HONEST: all 4 ACs met + independently verified + both reviews GO + the honesty question independently resolved.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Switched the pthreads-sync tier-1 backend off the AlgoIR walk onto the EventList+NameSidecar contract — the keystone M2 backend switch.

What changed:
- emit() signature: (per_worker: &BTreeMap<WorkerId,Vec<Event>>, names: &NameTables, sidecar: &NameSidecar, kernels_rs_path, out_dir). &ACFG/&LinkedIR removed (AC#1). New NameTables (reverse name maps + inner_block_iter_vars) + EmitError::ContractGap fail-loud seam.
- Single-worker render_main_rs rewritten as an Event walk: Loop/Fire/Sync, pre-init + types from sidecar, calls from FireBinding, source-form loop bounds from sidecar.loop_bounds+consts, scalar-arg casts from sidecar.kernel_sig. Zero AlgoIR/LinkedIR.
- multi_worker rewritten onto the EventList: slots/types/barriers/push/wait all derived from Events+sidecar+name tables (no data_producers/consumers/algo.kernels). NUC_NONDET_TEST nonce preserved. Shared renderers with single-worker via pub(crate) shims (no drift).
- Driver builds acfg_to_events + build_sidecar (Err surfaced via the String channel like apply_block_transforms) + reverse NameTables.
- Absolute-index rebinding for the evenly-divisible block= case (block_transform defers LO+tile*N+inner to codegen; the old backend masked it by never tiling). Fixes 07-matmul/blocked which the EventList correctly tiles and which an accumulator would otherwise double-count.
- Finding (7): added a backend golden test pinning real emit() strings so the compiler-side hand-mirrors cannot silently drift.

AC status (all honestly met & verified): AC#1 signature changed; AC#2 codegen path is AlgoIR-/LinkedIR-free for BOTH single- and multi-worker (grep-verified; only the inert IrExpr grammar the EventList itself carries remains); AC#3 pthreads-sync is the sole tier-1 backend and now defines the contract (mp-tcp is TASK-0036, forward-carried); AC#4 TASK-0027 dependency satisfied.

Gates: just test 0 failed, just e2e 10/8/0/2 required-fail 0, just determinism-check 8/0 byte-identical, just determinism-check-negative bites, just clippy clean. Every required cell (01/02-naive/02-split/03/05-naive/05-blocked/07-naive/07-blocked) PASSes e2e and is determinism-byte-identical.

Honest limitations / filed blockers (both depend on TASK-0124):
- TASK-0172: Event::Sync has no stable cross-worker barrier identity (unlike Push/Wait seq). Multi-worker uses a per-worker pre-order-Sync-index that is byte-identical only for UNIFORM barriers; partial-barrier schedules are a typed ContractGap error, not a wrong binary.
- TASK-0173: non-divisible/trailing-partial-tile absolute-index rebinding (full vs partial nest need different bases; EventList lacks num_full). 05-stencil/blocked stays correct only via blur3 idempotence; a non-divisible blocked accumulator would be wrong until TASK-0173.

Forward-carried to TASK-0036 (2nd backend consumes the SAME contract): the (per_worker EventList, NameTables, NameSidecar) tuple + EmitError::ContractGap fail-loud pattern + the divisible-only absolute-index rebinding caveat (TASK-0173) + the Event::Sync-identity caveat (TASK-0172) all apply to mp-tcp-bufsync verbatim.
<!-- SECTION:FINAL_SUMMARY:END -->
