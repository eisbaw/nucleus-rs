---
id: TASK-0431
title: >-
  TASK-0430 follow-up: pure index-kernel scalar arg gets no sig-driven cast (i64
  vs i32 latent E0308)
status: Done
assignee:
  - '@me'
created_date: '2026-06-02 23:43'
updated_date: '2026-06-03 03:33'
labels:
  - compiler
  - scatter
  - grammar-extension-epic
  - broaden
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
render_int_expr IrExpr::Call arm (TASK-0430, render/expr.rs) emits a pure index-kernel call kernels::callee(args) WITHOUT a per-param as-type cast, unlike render_fire_arg which casts iter-var scalars to the kernel sig param type. For the landed 08-histogram/textbook example the only arg is an i32 gather load (input[i]), which typechecks. But a pure index-kernel whose param is i32 and whose arg is a bare iter var (rendered i64 in generated code) would hit E0308 at build of the generated crate. Not a silent miscompile (rustc catches it loudly), but a usability gap. Fix needs plumbing the callee KernelId + sidecar kernel_sig param types into render_int_expr (today it takes only an IrExpr). Scope: thread an optional sig lookup or a richer ctx into render_int_expr Call arm; add an example/test exercising an iter-var scalar arg to an index kernel. LOW priority - no current example needs it; the bounded/textbook scatters use data-ref args only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 render_int_expr emits a per-param as-type cast for scalar args of a pure index-position kernel call, matching render_fire_arg
- [x] #2 example/test exercises an iter-var (i64) scalar arg to an i32-param index kernel and builds clean across tier-1 backends
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation Plan (cycle):

AC#1 (shared cast fix) FIRST, fully gated + committed:
- render/expr.rs IrExpr::Call arm: resolve callee KernelId by inverting names.kernel by name (same pattern as render_gather_index_load on names.data), read ctx.sidecar.kernel_sig, cast each scalar arg (arg) as <ty> via rust_scalar_type — mirror render_fire_arg scalar path EXACTLY (always cast when param is Some+scalar; bare otherwise). Degrade (no panic) on missing name/sig/oob-index/non-scalar param.
- Rewrite the now-stale comment above the arm (was: emits WITHOUT cast, latent E0308 filed TASK-0431) to describe the new cast + degradation behavior. comment/doc-lie discipline.
- Add render-layer unit tests in tests/render_guard_siblings.rs: (a) bare iter-var i to i32-param shift => kernels::shift((i) as i32); (b) i32 gather arg => (input[..]) as i32 inert no-op (matches always-cast rule); (c) non-scalar param => bare (no cast); (d) no-sig => bare (degradation). Update the 2 existing positive tests (empty fixtures => bare) comments to note the no-sig degradation.

AC#2 (demonstration that BUILDS CLEAN across tier-1): the existing 19/08 index-kernel arg is input[i] (i32 gather), NOT a bare iter var, so it does NOT exercise the i64->i32 path. Need a bare-iter-var arg. e2e harness = one reference.bin per dir, so a variant in an existing dir must match that dir oracle (impossible for a different algo). => NEW minimal example dir 20-permute-identity: out[idx(i)] <-- copy(in[i]); kernel idx:(i32)->i32 pure with idx(i)=i (identity) so idx(i) renders kernels::idx((i) as i32) — the exact bare-iter-var-i64->i32-param shape. Output = copy of input => simple permissive oracle. Standalone reference/ (std-only, rem-free copy), own input.bin, reference.bin, README, schedule, runnable_examples + [[required]] cells for the backends that pass.

If AC#2 ripples beyond this cycle: land AC#1 + render-test fully gated, leave In Progress with honest notes.

e2e baseline to verify (not trust): 413/356/0/57/0.

CYCLE OUTCOME (both ACs landed, gated green, value-correct).

AC#1 (commit c355059): render_int_expr IrExpr::Call arm now applies a sidecar-driven per-param scalar cast (arg) as <ty>, mirroring render_fire_arg EXACTLY. Resolves callee KernelId by inverting ctx.names.kernel by name (same pattern render_gather_index_load uses on names.data), reads ctx.sidecar.kernel_sig, casts each scalar arg via the new cast_index_arg helper. Degrades to bare arg (NO panic) on missing name / missing-or-None sig / out-of-range param index / non-scalar param — identical to render_fire_arg fallback (panic-not-diagnostic rule). Rewrote the now-stale comment above the arm (was: args WITHOUT cast, latent E0308, filed TASK-0431) to describe the new behavior; grep-swept the tree for stale TASK-0431/latent-E0308/no-per-param-cast prose — only my own new accurate refs remain. +4 render-layer unit tests in backend-common/tests/render_guard_siblings.rs (16/16 pass dev AND release): bare iter-var i->i32 param => kernels::shift((i) as i32); i32 gather arg => (input[..]) as i32 inert no-op (always-cast rule); non-scalar param => bare; no-sig => bare (degradation). Relabelled the 2 pre-existing positive Call-arm pins to note the no-sig degradation.

AC#2 (commit 8378f72): NEW example dir 20-index-cast-permute. out[idx(i)] <-- pass(src[i]) with idx(i)=i, pass(x)=x (identity). The index emits out[(kernels::idx((i) as i32)) as usize] = kernels::pass(src[(i) as usize]) — the (i) as i32 is the AC#1 cast, VERIFIED by direct nucleus build inspection AND value-correct run vs reference.bin. Standalone std-only reference (direct copy, no idx/pass call), deterministic Knuth-hash input.bin (128/256 negatives), single-worker naive schedule. Registered in runnable_examples + 7 [[required]] M6 cells; all 7 tier-1 backends PASS bit-identical vs reference.bin.

WHY a NEW dir (not a variant in 19/08): the e2e harness uses ONE reference.bin per example dir, and every shipped X1 cell calls the index kernel with a GATHER LOAD arg (bucket(input[i]), already i32) — NONE exercises a bare iter-var i64->i32 shape. A variant in an existing dir would have to match that dir oracle (impossible for a different algo).

ORACLE-STRENGTH LIMIT (disclosed honestly in README + matrix comment): idx/pass are identity => reference.bin == input.bin (a copy). A backend that merely copied input->output without evaluating idx/pass would also byte-match. So the LOAD-BEARING AC#2 assertion is NOT the (trivial) arithmetic but BUILDS-CLEAN-across-backends: pre-TASK-0431 (no cast) the generated crate fails E0308, which the harness (compiles each emitted crate) turns into a cell FAIL. Render-string also unit-pinned directly (AC#1). The two together are the complete proof.

clippy re unnecessary_cast: workspace just clippy (-D warnings) CLEAN. The redundant (i32-arg) as i32 emission lives only in GENERATED source compiled by rustc in the e2e harness — NOT clippy-gated (verified: e2e compiles emitted crates with cargo build, not clippy). Matched render_fire_arg always-cast rule; did NOT over-engineer an already-i32 skip.

GOTCHA (forward-carry — general, not TASK-0431-specific): a data symbol named in compiles to let mut in = ... in the generated host source and fails (in is a reserved Rust keyword). Renamed the input array in->src. Caught empirically (rustc, fail-fast). No general keyword-collision guard exists in the front-end today — a candidate hardening follow-up if more such collisions surface (NOT filed; single instance, low value).

e2e: 413/356/0/57/0 (baseline, reproduced) -> 420/363/0/57/0 (+7 pass, the 7 new cells). Full just ci EXIT 0 (check/clippy/test 1276 dev/test-release/structural fences textual-replace+include-str+doc-citation+mega-file + 4 negative/determinism arms ALL OK; xbackend-corruption arm correctly bit 49 applied/15 detected; required-coverage typo detected). The 5 existing index-kernel cell families (08 textbook/scatter + distributed, 19) STILL PASS unchanged (cast is inert no-op on their i32 gather args).

NOTE: orchestrator runs an independent parallel review gate after return; this self-cert is not authoritative.

REVIEW GATE (cycle 248, orchestrator-independent parallel read-only): qa-test-runner GO + mped-architect GO.

qa NUMBERS (re-run, not transcribed): build OK; clippy clean -D warnings incl a FORCED rebuild of backend-common/render/expr.rs (no clippy::unnecessary_cast on the new (i32-arg) as i32 path — generated crates are rustc-compiled by the harness, not clippy-gated; workspace itself clean); just test 1276/0/3 dev; just test-release 1274/0/3 (dev->release delta 2 = pre-existing TASK-0291 debug_assert should_panic); just e2e 420/363/0/57/0 with 0 fail / 0 required-fail; full just ci EXIT 0 (all structural fences OK incl check-doc-cell-path-staleness resolving the new 20-index-cast-permute/naive citations + check-mega-files; all 4 negative/determinism arms bit correctly). New 20-index-cast-permute cells 7/7 PASS individually; existing 08-histogram/textbook 7/7 + 19-histogram-unconstrained 14/14 still PASS unchanged (cast is inert no-op on their i32 gather args). Stable across 4 e2e runs.

architect COMPLETENESS: PASS. AC#1 cast_index_arg (expr.rs:135) mirrors render_fire_arg scalar arm (fire.rs:365-375) EXACTLY; KernelId inversion matches render_gather_index_load pattern; PANIC-SAFE at every edge (kernel_sig->Option, params.get->Option, is_scalar total, rust_scalar_type total match, name find->Option; all 4 degradation paths fall through to bare arg via _ => arm; no unwrap/panic/[] on new path). +4 unit tests genuinely exercise i32-cast / inert-gather / non-scalar-degrade / no-sig-degrade. Comment-lie sweep CLEAN: stale Call-arm comment fully rewritten, git-grep across examples/source/matrix found no surviving no-cast/filed-TASK-0431 lie (the new example prose is correct COUNTERFACTUAL framing). AC#2 build-clean argument STATICALLY VALID (idx param genuinely i32, iter var i genuinely i64 -> pre-fix E0308 -> cell FAIL -> a PASS witnesses the cast). Oracle independent per policy. No silent backend drop (7/7 required). No AC-gaming.

P1/P2: none. P3 dispositions: (P3-1) AC#2 value-oracle weak (identity idx/pass -> reference.bin == input.bin; a copy-only backend would also match). AC genuinely met (build-clean + render-string pins carry the proof) so NOT a re-open; filed TASK-0431.01 to strengthen to a non-identity reversal for a discriminating value-oracle. (P3-2) Rust-keyword / DSL-identifier collision (data `in` -> `let mut in` rustc failure; worked around in->src) is a real latent defect CLASS across every identifier x every backend, NOT a one-off; filed TASK-0433 (fail-loud front-end keyword check or r#-escaping codegen) — overriding the implementer choice not to file. (qa P3 pre-existing intra-doc-link warnings, not introduced here, ci does not gate on them.)
<!-- SECTION:NOTES:END -->
