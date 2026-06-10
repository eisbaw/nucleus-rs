---
id: TASK-0458
title: >-
  unroll=N: loud reject (accepted-but-unimplemented) until TASK-0293 lands a
  consumer
status: Done
assignee: []
created_date: '2026-06-09 22:00'
updated_date: '2026-06-10 09:59'
labels:
  - fail-fast
  - sched
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P1.3), independently verified: unroll=N is parsed (sched/parser.rs:585), divisibility-validated against block (sched/lower.rs:970-999), lowered to ResolvedLoopOption::Unroll (lower.rs:1055) — and consumed by NO pass (grep: only lower.rs touches the variant). A schedule author tuning unroll=8 silently gets nothing — a fail-fast violation and the exact silent-downgrade pattern the capability matrix exists to forbid elsewhere.

PRD 6.3.3 defers implementation to TASK-0293 (reopen on concrete LLVM-vs-DSL divergence evidence). This task is NOT blocked by TASK-0293 — it lands FIRST, making the surface honest until a consumer exists; TASK-0293 would then replace the reject with the real transform. Preferred: hard error naming the option as unimplemented and citing the deferral; alternative (record the decision here if taken): an unmissable warning. Keep the grammar production; thesis appendix B already states the option is inert — keep paper and compiler claims aligned whichever way this lands.

Note: an earlier filing of this task (TASK-0458) was archived because a wrong blocking dependency on TASK-0293 could not be removed via the CLI.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A schedule using unroll=N fails loudly (or the warn decision is recorded here) naming the unimplemented option and TASK-0293
- [x] #2 Negative test pins the diagnostic
- [x] #3 TASK-0293 cross-referenced both directions; thesis appendix B claim still accurate after the change
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Reject unroll=N at sched-lowering as accepted-but-unimplemented. (1) new typed variant SchedLowerErrorKind::UnrollUnimplemented{var} in sched/ir.rs + Display arm naming the option, flagging unimplemented, citing TASK-0293/PRD 6.3.3. (2) reject in sched/lower.rs lower_loop AFTER the existing block+unroll-divisibility check (so UnrollNotDivisibleByBlock still wins for non-divisible pairs; every other unroll=N falls through to loud reject). (3) classification-table row + Independent recovery class. (4) negative tests in tests/sched_unroll_unimplemented.rs (owned). HARD ERROR chosen, not warn (no-silent-downgrade philosophy, matches pipeline=1/block+pipeline precedent). No example schedule uses unroll= (verified grep nuc-nucleus/examples). Forward-note appended to TASK-0293.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation complete (2026-06-10). unroll=N now loud-rejected at sched-lowering as accepted-but-unimplemented. HARD ERROR chosen (not warn): matches the no-silent-downgrade philosophy and the sibling pipeline=1 (UnitPipelineOption) / block+pipeline (BlockPipelineConflict) precedents. Files: (1) nucleus/nucleus-compiler/src/sched/ir.rs — new typed variant SchedLowerErrorKind::UnrollUnimplemented{var} (~L649) + Display arm (~L793) naming the option, flagging "not yet implemented", citing TASK-0293/PRD 6.3.3. (2) nucleus/nucleus-compiler/src/sched/lower.rs — reject in lower_loop (~L1011) placed AFTER the UnrollNotDivisibleByBlock divisibility check so the more-specific non-divisible diagnostic still wins; remove-on-TASK-0293 comment in place; classification-table row UnrollUnimplemented=Independent (~L164). (3) nucleus/nucleus-compiler/tests/sched_unroll_unimplemented.rs — 4 negative tests: bare unroll rejected, diagnostic names option+cites TASK-0293, block-divisible unroll STILL rejected (no silent accept for the divisible case), block-nondivisible keeps UnrollNotDivisibleByBlock (ordering guard). VERIFICATION: cargo test -p nucleus-compiler --test sched_unroll_unimplemented => 4 passed/0 failed. cargo test -p nucleus-compiler sched => all green (note the name-filter "sched" does NOT match the negative_* test names, so run the binary explicitly with --test). clippy --lib --test sched_unroll_unimplemented --test sched_lower --test sched_parser -D warnings => clean. EXAMPLE CHECK: grep nuc-nucleus/examples for unroll= => zero schedule directives (only prose comments about Petri static unrolling in 05-stencil/21/29). Forward-note appended to TASK-0293.

AC#3 PARTIAL — file-ownership boundary. AC#3 requires the thesis appendix B claim to stay accurate. It is NOW INACCURATE: paper/appendices/B-grammar.tex:154-156 says unroll "is accepted by the parser but is currently inert --- no transform pass consumes it", but after this change unroll=N is loudly REJECTED, not inert. Fixing it requires editing paper/appendices/B-grammar.tex, which was OUTSIDE this wave-task file-ownership scope (ownership was sched/*.rs + tests/sched_unroll*). Did NOT touch it. Filed TASK-0463 to update the appendix B sentence to "rejected at sched-lowering as accepted-but-unimplemented, a loud error not a silent no-op" (which actually strengthens the no-silent-downgrade story the same paragraph makes two sentences later). Cross-reference back direction (TASK-0293 forward-note) is DONE. GOTCHA for orchestrator/wave-gate: a foreign uncommitted edit to nucleus/nucleus-compiler/tests/petri_to_events.rs (another agent removing a petri_to_events wrapper test) leaves an unused-import (NotifyMode) that fails crate-wide clippy -D warnings — NOT part of TASK-0458, NOT in my ownership, untouched. The full-crate clippy/test gate will stay RED until that other agent fixes their import; my lib + sched test targets are clean in isolation.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
unroll=N now loud-rejects at sched-lowering: typed SchedLowerErrorKind::UnrollUnimplemented{var} naming the option + citing TASK-0293/PRD 6.3.3, placed AFTER the divisibility check so UnrollNotDivisibleByBlock still wins for non-divisible pairs. Hard error per no-silent-downgrade. 4 negative tests; pre-existing positive reordering test swapped to partition=workers (orchestrator fold-in; pipeline would trip TASK-0215 block+pipeline conflict); masked unroll in duplicate-loop test also swapped (review P3). No example uses unroll= (grep). Both cross-references landed: TASK-0293 forward-note + thesis appendix B updated via TASK-0463 (commit 32c3109, PDF green). Landed c72209d + e907f63; architect GO; gate green.
<!-- SECTION:FINAL_SUMMARY:END -->
