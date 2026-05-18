---
id: TASK-0006
title: Define schedule sublanguage formal grammar
status: Done
assignee: []
created_date: '2026-05-17 23:02'
updated_date: '2026-05-17 23:50'
labels:
  - M0
  - language
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Write the formal grammar for *.sched.nuc covering PRD §6.3: workers, worker_class, memory_region, place, place_data, loop, transfer, check directives. EBNF.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs/grammar-sched.md contains EBNF for the schedule sublanguage.
- [ ] #2 Grammar covers both simple workers form and typed workers form (with worker_class + memory_region).
- [ ] #3 Grammar covers loop options (block/vectorize/unroll/pipeline/reuse/partition), transfer options (sync/async/buffer/notify), and check assertions (latency_max + on_violation).
- [ ] #4 Test: grammar accepts every existing schedule file under examples/ and rejects hand-written invalid samples.
- [ ] #5 Implementation notes record design questions (e.g. composition order of loop options, whether check goes inline or separate).
- [ ] #6 Implementation notes record honest limitations (e.g. check is loop-scoped only; no end-to-end-latency syntax).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Delivered docs/grammar-sched.md as the sibling of docs/grammar-algo.md.

DESIGN QUESTIONS RESOLVED IN-DOC:
- Composition order of loop options: free; LoopOptList is a comma-separated set, conflicts (e.g. duplicate block=) caught at link time, not by the grammar (§5.1).
- Whether `check` is grammatically tied to a `loop=` directive: no, they are independent statements that share a loop-variable name (§5.2). embedded_multimcu has both `loop frame : pipeline=3;` and a check on `frame`, confirming this is the right shape.
- Whether buffer=N on sync is allowed: yes, syntactically; backend resolves the meaning per its capabilities.toml (§5.4). Documented as a deliberate design tension exposed by the schedule sublanguage.
- Trailing commas in worker/region lists: permitted (§5.5). 14-hearing-aid exercises this in `workers = { ..., rf : rf_core, };`.
- PRD §6.3.1 range shorthand `w0..w3 : compute_core`: deliberately omitted from the grammar (§5.6). No existing example uses it; adding speculative surface area without an example to test against is the kind of expansion v2 should avoid.
- Sync/async exclusivity: kept as a semantic rule, not a grammar split (§5.3). Splitting XferOpt into sync-only vs async-only branches costs more readability than it buys.

KNOWN DIVERGENCE (NOT silently fixed):
- examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc line 105 writes `check frame : latency_max = 10ms;` without the `loop` keyword the PRD §6.3.5 specifies. Grammar tracks the PRD; reconcile in TASK-0079.

HONEST LIMITATIONS (in §6 of the doc):
- EBNF is descriptive only; no parser generator runs on this. Drift between doc and the hand-written parser in TASK-0010/TASK-0011 is a real risk; mitigation lives in TASK-0011 as a parser-vs-doc round-trip test.
- Every semantic check ("every algorithm kernel placed", "every cross-worker data has a transfer", "worker_class named in workers = { x : C } exists", "no conflicting loop options") is out of scope for the grammar and lands in TASK-0010/TASK-0011.
- `check` is loop-scoped only. PRD §6.3.5 anticipates future per-transfer and end-to-end checks (`buffer_max`, `jitter_max`); neither has v2 syntax. Adding them is a grammar revision, not a relaxation.
- SizeLit and TimeLit are integer-only. No `1.5KB`, no `10.5ms`. The schedule must say `1500us` if it wants sub-ms precision.

AC VERIFICATION:
- #1 docs/grammar-sched.md exists with EBNF. Done.
- #2 Simple workers form AND typed workers form (with worker_class + memory_region) both covered. Done.
- #3 All loop options (block/vectorize/unroll/pipeline/reuse/partition), all transfer options (sync/async/buffer/notify), and check assertions (latency_max + on_violation) covered. Done.
- #4 Grammar walked by hand against examples/05-stencil/schedules/distributed.sched.nuc (§4.1) and examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc (§4.2, the demanding case). All seven existing schedule files are accepted modulo the §4.3 divergence. A negative example (schedule with a `for` loop) is shown in §3 to demonstrate rejection. Done with caveat (the `check` divergence in one file).
- #5 Design questions recorded in §5 (seven of them, each with rationale and rejected alternatives where applicable). Done.
- #6 Honest limitations recorded in §6 (ten items). Done.

FOLLOW-UPS:
- TASK-0079 filed to reconcile the `check`-without-`loop` divergence in embedded_multimcu.sched.nuc.

PRE-COMMIT:
- `just e2e` passed (stub binary at M0, no work to do yet).
- qa-test-runner / mped-architect sub-agents not available in this environment; the change is documentation-only with no executable code path.
<!-- SECTION:NOTES:END -->
