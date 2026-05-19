---
id: TASK-0196
title: >-
  Propagate schedule-AST spans into SchedLowerError -> located schedule-lowering
  diagnostics (sched analog of TASK-0090)
status: To Do
assignee: []
created_date: '2026-05-19 16:16'
updated_date: '2026-05-19 16:39'
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
- [ ] #1 SchedLowerError = struct{kind:SchedLowerErrorKind (prior enum verbatim), span:Option<Range>}; manual PartialEq/Eq forward to .kind only (no Hash unless manual+.kind); existing sched_lower negative tests migrate mechanically with assertion strength PRESERVED
- [ ] #2 Diagnosable variants carry the offending sched Spanned's span; driver surfaces 'schedule lower error: <msg> at L:C'; typed-Result, no panic, display_with_src clamps offset
- [ ] #3 A test asserts correct line:col for >=3 representative bad schedules via offset_to_line_col; any genuinely position-less variant is documented ACCURATELY (docs match code) and pinned by a test
- [ ] #4 Zero behaviour change for valid input: just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci exit 0
<!-- AC:END -->

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
<!-- SECTION:NOTES:END -->
