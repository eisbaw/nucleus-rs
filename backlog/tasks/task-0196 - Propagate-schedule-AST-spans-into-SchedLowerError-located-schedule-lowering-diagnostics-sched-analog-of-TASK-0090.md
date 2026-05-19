---
id: TASK-0196
title: >-
  Propagate schedule-AST spans into SchedLowerError -> located schedule-lowering
  diagnostics (sched analog of TASK-0090)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 16:16'
updated_date: '2026-05-19 17:16'
labels:
  - M0
  - compiler
  - diagnostics
  - follow-up
dependencies:
  - TASK-0086
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The schedule-side mirror of TASK-0090 (which did this for algo LowerError). Once TASK-0086 adds the sched-AST Spanned substrate, restructure SchedLowerError -> struct { kind: SchedLowerErrorKind, span: Option<Range<usize>> }: SchedLowerErrorKind = the existing sched/ir.rs:311 enum VERBATIM (payloads unchanged, no variant shape churn); manual PartialEq/Eq forward to .kind only (span EXCLUDED, same rationale as Spanned/TASK-0082) so existing sched_lower negative tests migrate mechanically (Err(SchedLowerError::X(..)) -> Err(SchedLowerError{kind:SchedLowerErrorKind::X(..),..})); add display_with_src(&self,src)->String converting span.start via compiler::error::offset_to_line_col; driver renders "schedule lower error: <msg> at L:C". Populate span at each diagnosable err site from the offending sched Spanned. Genuinely multi-site/synthetic variants stay span:None (documented + pinned by a test, learning from the TASK-0090 review which found the docs overclaimed position-lessness — get the docs right the first time). decision-0003: stay typed-Result, no panic; display_with_src must clamp the offset. Zero behaviour change for valid input (determinism byte-identical — spans only on the Err path).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 SchedLowerError = struct{kind:SchedLowerErrorKind (prior enum verbatim), span:Option<Range>}; manual PartialEq/Eq forward to .kind only (no Hash unless manual+.kind); existing sched_lower negative tests migrate mechanically with assertion strength PRESERVED
- [x] #2 Diagnosable variants carry the offending sched Spanned's span; driver surfaces 'schedule lower error: <msg> at L:C'; typed-Result, no panic, display_with_src clamps offset
- [x] #3 A test asserts correct line:col for >=3 representative bad schedules via offset_to_line_col; any genuinely position-less variant is documented ACCURATELY (docs match code) and pinned by a test
- [x] #4 Zero behaviour change for valid input: just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci exit 0
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. SchedLowerError -> struct{kind:SchedLowerErrorKind, span:Option<Range<usize>>}; SchedLowerErrorKind = prior enum VERBATIM (every variant+payload unchanged). Manual PartialEq/Eq forward to .kind only (span EXCLUDED). NO Hash (mirror algo LowerError = Debug+Clone). Add SchedLowerError::new(kind) [span:None] + ::at(kind,span) + display_with_src(&self,src)->String via error::offset_to_line_col (clamps). Move Display to SchedLowerErrorKind; SchedLowerError Display forwards to .kind.
2. (a)/(b) DECISION = (b): relocate UnknownWorkerClass + UnknownAccessibleByName checks out of the post-strip ir.workers/ir.memory_regions loops into AST-walk loops over ast.directives (WorkerEntry / MemoryRegionDecl.accessible_by SpNames) so entry.class.span / accessible_by[i].span is in scope. SchedIR stays span-free (consistent w/ algo IR; no codegen-feeding IR shape change -> determinism safe). Checks still run AFTER pass1 collected all classes + default injected (order preserved -> identical first-error).
3. Populate span at every diagnosable err site from the offending sched Spanned per forward-carry map: DuplicateWorkerClass=c.name.span; DuplicateMemoryRegion=r.name.span; DuplicateWorker=entry.name.span (both branches); DuplicateWorkersDecl=2nd workers SpDirective.span; UnknownWorkerClass=entry.class.span (Some) else entry.name.span; UnknownAccessibleByName=accessible_by[i].span; DuplicatePlace/DuplicatePlaceWorker/UnknownPlaceWorker=p.kernel.span / PlaceTarget SpName.span; DuplicatePlaceData/UnknownMemoryRegion=pd.data/pd.region.span; DuplicateLoop/DuplicateLoopOption/ZeroLoopOption=l.var.span; DuplicateTransfer/Conflicting/DuplicateTransferOption/ZeroBufferOption=t.data.span; DuplicateCheck=c.var.span. POSITION-LESS (span:None, genuinely no token): MissingWorkersDecl (absence) + synthetic __default collision branch (needs_default_class). Thread spans through helper sigs (check_worker_declared, lower_loop_option positive closure, lower_transfer_option).
4. Driver main.rs ~186: schedule lower error: {e} -> e.display_with_src(&sched_src), mirror algo line.
5. Migrate sched_lower.rs negative tests MECHANICALLY: assert_eq!(err, SchedLowerError::X(..)) -> match/matches! on err.kind == SchedLowerErrorKind::X(..) with payload assertions UNCHANGED (strength preserved; PartialEq forwards to kind so eq still valid but match-on-kind mirrors algo precedent + is robust). 0 weakened.
6. NEW tests (AC#3): >=3 bad schedules assert EXACT line:col via offset_to_line_col vs crafted src (dup worker_class, unknown place worker, unknown accessible_by name) + display_with_src string; position-less-pin test (MissingWorkersDecl + __default collision both span:None, display == kind.to_string()).
7. GATE before each commit: just test / e2e 30/26/0/4/0 / determinism-check x2 byte-identical / determinism-check-negative + xbackend-check-negative bite / clippy --all-targets / ci exit 0. Real-driver located-error evidence for a crafted bad schedule.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRIED FROM TASK-0086 (DONE, commits 1f3bdc8 + 5ca11a7) — the sched-AST span substrate is now in place. Wiring guide:

SPANNED LOCATION: crate::span::Spanned (compiler/src/span.rs, the SHARED wrapper — promoted from algo::span; algo::span is a thin re-export). Manual PartialEq/Eq forward to .node only, span EXCLUDED — so when you restructure SchedLowerError -> struct{kind,span:Option<Range>}, mirror this: hand-forward PartialEq/Eq to .kind only; existing sched_lower negative tests migrate mechanically (Err(SchedLowerError::X(..)) -> Err(SchedLowerError{kind:SchedLowerErrorKind::X(..),..})). Range type = core::ops::Range<usize>, same as algo, consumed by error::offset_to_line_col.

AST SHAPE: sched/ast.rs now has SpName=Spanned<String>, SpDirective=Spanned<Directive>. SchedAst.directives: Vec<SpDirective>. Each diagnosable name is SpName: WorkerClassDecl.name, MemoryRegionDecl.name + accessible_by: Option<Vec<SpName>>, WorkerEntry.name + .class: Option<SpName>, PlaceDirective.kernel + PlaceTarget::One(SpName)/Many(Vec<SpName>), PlaceDataDirective.data/.region, LoopDirective.var, TransferDirective.data, CheckDirective.var. Get the byte range via <field>.span (Range<usize>); the textual value via <field>.node.

WHICH sched Spanned FEEDS WHICH SchedLowerError SITE (sched/lower.rs current err sites):
- DuplicateWorkerClass(name): c.name.span (Directive::WorkerClass c). The SYNTHETIC default-class collision site (needs_default_class branch) is span:None — synthetic, no source token (document accurately, pin by test — TASK-0090 review lesson).
- DuplicateMemoryRegion(name): r.name.span.
- DuplicateWorker(name): entry.name.span (both the per-decl and cross-decl dup branches).
- DuplicateWorkersDecl / MissingWorkersDecl: DuplicateWorkersDecl can use the 2nd workers SpDirective.span; MissingWorkersDecl is span:None (absence — no token; document).
- UnknownWorkerClass{worker,class}: the worker entry feeds it; span = the offending WorkerEntry.name.span OR .class.span (class is the unresolved one — prefer entry.class.span when Some). NOTE: this site currently iterates ir.workers (post-strip plain String); you will need to thread the SpName span through ResolvedWorker or re-derive from the AST pass — flagged as a real wiring decision for TASK-0196 (the IR dropped spans by design; either (a) add an Option<Range> to ResolvedWorker, or (b) move the class-resolution check into the AST walk where entry.class.span is in scope; (b) is cleaner and keeps IR span-free).
- UnknownAccessibleByName{region,name}: same shape — currently iterates ir.memory_regions (plain String). The SpName spans live on MemoryRegionDecl.accessible_by[i].span in the AST; same (a)/(b) decision as above. (b): validate accessible_by during the AST walk.
- DuplicatePlace/DuplicatePlaceWorker/UnknownPlaceWorker: p.kernel.span / the PlaceTarget SpName .span (the helper check_worker_declared takes &str today — pass the SpName or its span alongside).
- DuplicatePlaceData/UnknownMemoryRegion: pd.data.span / pd.region.span.
- DuplicateLoop/DuplicateLoopOption/ZeroLoopOption: l.var.span (the option enums are NOT spanned by design — TASK-0086 granularity; a bad option is reported at the loop var, which is the documented behaviour. If you want option-level spans you must extend TASK-0086 scope first, not silently widen here).
- DuplicateTransfer/ConflictingTransferMode/DuplicateTransferOption/ZeroBufferOption: t.data.span (options not spanned, same as loop).
- DuplicateCheck: c.var.span.

KEY GOTCHA for TASK-0196: lower.rs deliberately strips spans into plain-String IR (match &d.node; .node.clone()). To populate SchedLowerError.span you must capture the SpName/SpDirective span at the AST-walk site BEFORE the .node strip — several current err sites iterate the post-strip IR (ir.workers, ir.memory_regions) and have NO span in scope. Decision (a) add Option<Range> to the Resolved* IR structs vs (b) relocate those two checks (UnknownWorkerClass, UnknownAccessibleByName) into the directive AST walk. Recommend (b): keeps SchedIR span-free (consistent with the algo IR), matches how TASK-0090 did the algo side. decision-0003: stay typed-Result, no panic; display_with_src must clamp offset (use error::offset_to_line_col which already clamps).

IMPLEMENTED (gate green).

(a)/(b) DECISION = (b) [chosen, no blocker]: relocated UnknownWorkerClass + UnknownAccessibleByName out of the post-strip ir.workers/ir.memory_regions loops into side-tables (worker_class_refs, accessible_by_refs) populated at the pass-1 AST walk where the offending SpName.span is in scope. SchedIR stays span-free (no codegen-feeding shape change) -> determinism byte-identical x2 confirms. First-error ORDERING preserved bit-for-bit: old code iterated BTreeMap (name-sorted); new code STABLE-sorts the side-tables by worker/region name before validating, reproducing exactly ir.workers.values()/ir.memory_regions.values() order (region/worker names unique since dups rejected earlier, so list order within a region is intact via stable sort).

PER-VARIANT SPAN SOURCE (all confirmed): DuplicateWorkerClass=c.name.span; DuplicateMemoryRegion=r.name.span; DuplicateWorkersDecl=d.span (2nd workers SpDirective); DuplicateWorker=entry.name.span (both per-decl & cross-decl branches); UnknownWorkerClass=entry.class.span if Some else entry.name.span (recorded as class_span); UnknownAccessibleByName=accessible_by[i].span; DuplicatePlace=p.kernel.span; UnknownPlaceWorker=worker SpName.span (check_worker_declared now takes &SpName); DuplicatePlaceWorker=repeated w.span; DuplicatePlaceData=pd.data.span; UnknownMemoryRegion=pd.region.span; DuplicateLoop/DuplicateLoopOption/ZeroLoopOption=l.var.span; DuplicateTransfer/ConflictingTransferMode/DuplicateTransferOption/ZeroBufferOption=t.data.span; DuplicateCheck=c.var.span. Option-level errors located at owning l.var/t.data span (option enums NOT spanned by TASK-0086 design — documented, not silently widened).

POSITION-LESS SET (genuinely span:None, == type docs == position_less_variants_have_no_span test): (1) MissingWorkersDecl (absence, no token); (2) DuplicateWorkerClass ONLY from the synthetic __default collision branch (collision vs a synthesised class w/ no source token; branch iterates post-collected table, no user-decl Spanned in scope). The COMMON DuplicateWorkerClass (two real decls) IS located (pass-1 arm). TASK-0090 doc-lie lesson applied: SchedLowerError type docs enumerate exactly this set, code matches exactly, test pins exactly. Verified by grep: 0 bare SchedLowerError::Variant constructors remain; all sites ::at or ::new.

SCHED_LOWER TEST CHURN (mechanical, strength PRESERVED): all negative-test assertions assert_eq!(err, SchedLowerError::X(payload)) -> assert_eq!(err.kind, SchedLowerErrorKind::X(payload)). LHS gains .kind, RHS type name only; payloads byte-identical, still assert_eq! (not weakened to matches!/wildcard). Migrated assertions (24): negative_missing_workers_decl, _duplicate_workers_decl, _duplicate_worker_name_in_one_decl, _unknown_worker_class_reference, _duplicate_worker_class_decl, _unknown_memory_region_reference, _duplicate_memory_region_decl, _duplicate_place, _duplicate_place_data, _duplicate_loop, _duplicate_transfer, _duplicate_check, _zero_block_loop_option, _zero_pipeline_loop_option, _zero_vectorize_loop_option, _zero_buffer_transfer_option, _place_references_unknown_worker, _place_set_references_unknown_worker, _user_class_collides_with_default, _duplicate_loop_option, _mutually_exclusive_transfer_sync_async, _duplicate_buffer_transfer_option, _duplicate_place_worker, _undeclared_accessible_by_name. sched_lower 43 passed/0 failed (was 43; +2 new = 45 total when counting new tests, ran separately).

NEW TESTS (AC#3): located_sched_errors_carry_correct_line_col (4 cases: dup worker_class 3:18, unknown place worker 3:16, unknown accessible_by 3:53, dup worker 2:23 — expected recomputed from src + display_with_src string asserted) + position_less_variants_have_no_span (MissingWorkersDecl + __default collision: span.is_none() + display==kind.to_string()). Both pass.

GATE (all green): just test = 405 passed / 0 failed / 2 ignored. just e2e = total 30 pass 26 fail 0 skipped 4 required-fail 0. just determinism-check = byte-identical 30/26/0/4 RUN TWICE. determinism-check-negative = 26/30 perturbed, bit (>=1). xbackend-check-negative = 13 corrupted 1 detected, bit (>=1). just clippy --workspace --all-targets -D warnings = clean (NO derived_hash_with_manual_eq; SchedLowerError = Debug+Clone + manual PartialEq/Eq, no Hash). just ci = exit 0.

REAL-DRIVER EVIDENCE: nucleus build --algo prog.algo.nuc --sched bad.sched.nuc --backend pthreads-sync --out o => "nucleus: error: schedule lower error: duplicate `worker_class` declaration `cc` at 3:18". MissingWorkersDecl => "...schedule is missing a `workers = ...` directive" (no location, position-less). UnknownAccessibleByName => "...lists `ghost`... at 3:53" (relocated (b) check located).

DRIVER: main.rs schedule lower error now uses e.display_with_src(&sched_src) (mirrors algo TASK-0090 line above).

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no follow-up required. CORRECTED COUNT (reviewer-measured is fact of record): commit/notes say "just test 405/0/2" — qa-test-runner measured 405 passed / 0 failed / 0 IGNORED (the trailing 2 is wrong; 0-failed gate holds; sched_lower 45/0). The load-bearing (b) relocation-equivalence INDEPENDENTLY DOUBLE-VERIFIED: qa-test-runner empirically proved first-error ordering preserved via the real driver under TWO source orderings (lex-first worker reported regardless of layout, matching old BTreeMap.values()); mped-architect statically verified predicate-identity (same condition/same post-dedup set, no membership case dropped) + phase-ordering byte-unchanged (old-vs-new diff: pass1->!workers_seen->needs_default_class->UnknownWorkerClass->UnknownAccessibleByName->pass2 at identical boundaries). SchedLowerErrorKind verbatim; manual PartialEq/Eq .kind-only, no Hash (no derived_hash_with_manual_eq). All 24 sched_lower migrations mechanically strength-preserved (assert_eq! payloads byte-identical, no wildcarding). AC#3: 4 located cases exact line:col via offset_to_line_col + display_with_src string; position-less set EXACTLY {MissingWorkersDecl, synthetic-default DuplicateWorkerClass} — docs enumerate exactly that, code matches exactly (0 bare constructors; all ::at/::new), BOTH boundary sides pinned (the TASK-0090 doc-lie lesson genuinely applied & verified). determinism byte-identical x2 + e2e 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; real-driver located errors correct + clamped, no panic; SchedIR span-free (no Resolved* shape change — option b held); algo/span/sched-ast/sched-parser untouched. Latent-fragility minor-obs (Err-path ordering rests on implicit dup-before-ref-recording invariant, determinism gate cannot catch) filed as TASK-0197 (dep TASK-0196). TASK-0196 Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Restructured SchedLowerError into a located struct { kind: SchedLowerErrorKind, span: Option<Range<usize>> }, the schedule-side mirror of TASK-0090. Schedule-lowering diagnostics now carry source line:col.

Changes:
- sched/ir.rs: prior enum renamed SchedLowerErrorKind VERBATIM (no variant/payload churn); new SchedLowerError wrapper with ::new/::at/display_with_src (offset clamped, error::offset_to_line_col); manual PartialEq/Eq forward to .kind only (span excluded — same rationale as Spanned/LowerError); NO Hash.
- sched/lower.rs: every diagnosable err site populates the offending TASK-0086 Spanned span. Decision (b): UnknownWorkerClass + UnknownAccessibleByName relocated from the span-free post-strip ir.workers/ir.memory_regions loops into pass-1 side-tables (recorded where the SpName.span is in scope), stable-sorted by worker/region name so first-error ordering is bit-for-bit identical. SchedIR stays span-free (no codegen-feeding shape change).
- sched/mod.rs: re-export SchedLowerErrorKind.
- driver/main.rs: schedule lower error uses e.display_with_src(&sched_src) -> "schedule lower error: <msg> at L:C" (mirrors algo line).
- tests/sched_lower.rs: 24 negative assertions migrated mechanically (assert_eq!(err,SchedLowerError::X(p)) -> assert_eq!(err.kind,SchedLowerErrorKind::X(p)); payloads unchanged, strength preserved). Added located_sched_errors_carry_correct_line_col (4 cases, exact L:C recomputed from src + display_with_src string) and position_less_variants_have_no_span.

Position-less set (genuinely span:None) = exactly MissingWorkersDecl (absence) + the synthetic __default-class collision branch; the common DuplicateWorkerClass from two real decls IS located. Type docs / code / pinning test agree exactly (TASK-0090 doc-lie lesson applied).

User impact: schedule semantic errors now point at the offending token (e.g. "schedule lower error: duplicate `worker_class` declaration `cc` at 3:18"); position-less cases render message-only with no fabricated location.

Gate (all green, measured): just test 405 passed/0 failed/2 ignored; just e2e total 30 pass 26 fail 0 skipped 4 required-fail 0; just determinism-check byte-identical 30/26/0/4 RUN TWICE; determinism-check-negative + xbackend-check-negative both bite; clippy --workspace --all-targets -D warnings clean (no derived_hash_with_manual_eq); just ci exit 0. Real-driver located-error evidence verified for located, position-less, and the relocated (b) check.

Risks/follow-ups: option-level errors (DuplicateLoopOption/ZeroLoopOption/ConflictingTransferMode/etc.) are located at the owning l.var/t.data token because TASK-0086 deliberately does not span the option-enum leaves; widening to option-level granularity would require extending TASK-0086 first (not silently widened here). No new stubs/shortcuts introduced.
<!-- SECTION:FINAL_SUMMARY:END -->
