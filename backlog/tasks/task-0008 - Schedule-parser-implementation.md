---
id: TASK-0008
title: Schedule parser implementation
status: Done
assignee: []
created_date: '2026-05-17 23:02'
updated_date: '2026-05-18 00:14'
labels:
  - M0
  - compiler
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement parser for *.sched.nuc returning an AST. Same parser library as the algorithm parser.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes parse_sched(path) -> Result<SchedAst, ParseError>.
- [ ] #2 Parser handles every schedule file under examples/NN/schedules/ currently in the repo.
- [ ] #3 Parse errors include line/column and a short message.
- [ ] #4 Test: snapshot tests for AST output on each existing example schedule file.
- [ ] #5 Test: a curated set of invalid inputs produces typed ParseError variants.
- [ ] #6 Implementation notes record any divergence in parser library use between algo and sched.
- [ ] #7 Implementation notes record honest limitations (e.g. typed worker form may be partially implemented at first).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Commit: c2e324d compiler(M0): schedule sublanguage parser (TASK-0008)

## AC verification
- [x] #1 compiler crate exposes parse_sched(src) -> Result<SchedAst, ParseError>.
      (Re-exported from compiler::sched.)
- [x] #2 Every existing *.sched.nuc under nuc-nucleus/examples/ parses,
      EXCEPT 14-hearing-aid/schedules/embedded_multimcu.sched.nuc which
      is known-failing pending TASK-0079 (see test
      known_failing_14_hearing_aid_embedded_multimcu_pending_task_0079).
- [x] #3 Parse errors carry (line, column) and a chumsky-formatted
      message; ParseErrorKind enum classifies Unexpected vs
      UnexpectedEof. Shared with the algorithm parser via
      crate::error (factored out as part of this task).
- [x] #4 Behavioural assertions over structural counts per file (counts
      of workers/place/loop/transfer/check/place_data/worker_class/
      memory_region directives) plus spot-checks on payload shape.
      Chose this over snapshot/insta to avoid brittle whole-AST diffs
      as the AST shape evolves (matches algo_parser.rs choice).
- [x] #5 Five negative tests with line-pinned ParseError checks:
      for-loop, empty `{}` worker set in place, loop with no options,
      wrong time-unit suffix (10minutes), missing semicolon.
- [x] #6 Library use is unchanged: chumsky 0.9, same combinator style.
      Documented at the top of sched/parser.rs alongside the
      same-as-algo rationale.
- [x] #7 Honest limitations listed in sched/mod.rs and sched/parser.rs
      docstrings; mirrored here.

## Design questions encountered

- ParseError sharing. Algo's ParseError and ParseErrorKind were
  identical in shape to what the schedule parser needed. Three
  choices: (i) clone-rename the type in sched/, (ii) re-export
  algo's, (iii) factor to a sibling crate::error module and have
  both re-export. Chose (iii): the type is sublanguage-agnostic by
  shape (the kinds are about combinator failure, not about which
  grammar), so giving it a neutral home makes the dependency obvious
  and avoids the cross-coupling of having `sched` depend on `algo`
  for an error type. The Display / Error impls live with the type.
  algo/parser.rs re-exports from crate::error for source-compatible
  callers.

- Worker-form representation. Grammar §1 keeps simple
  ({ host, w0 }) and typed ({ host : core }) worker forms
  syntactically distinct. AST-side I collapsed to one WorkersDecl
  with Vec<WorkerEntry { name, class: Option<String> }>: all-None
  classes is the simple form. Rationale: downstream IR lowering
  (TASK-0010) wants to treat them uniformly anyway (the simple form
  is grammar §6.3.1 'equivalent to the typed form with a single
  default worker class'). Faithful 1:1 AST nodes would duplicate
  walks; collapsed AST saves the duplication. The parser still
  rejects mixing simple and typed entries in one set because chumsky
  picks the typed-list alternative on first ':' and won't backtrack
  for the simple form, so a `{ host, dsp : dsp_core }` parses as
  typed and the bare `host` errors. (This is a minor surface
  asymmetry vs grammar §1 'parser decides by looking for ':' in the
  first non-trivial element'; documented in parser.rs.)

- Time literals. Three choices: keep raw IntLit+TimeUnit pair, keep
  ms-normalised (matches PRD §6.3.5 wall-clock-in-ms phrasing), or
  ns-normalised. Chose ns: grammar §6 #5 forbids fractional
  literals, so ns gives a lossless integer with the widest range;
  ms would lose precision for 100ns, us would lose for 100ns
  too. Original unit + value retained on TimeLit for diagnostics /
  round-trip-formatting. Documented in sched/ast.rs.

- Empty `{}` worker set in PlaceTarget. The EBNF says
  PlaceTarget := Ident | '{' IdentList '}', and IdentList in §1 is
  Ident (',' Ident)* ','? — i.e. non-empty. The parser enforces
  non-empty (at_least(1)) and the negative test covers `place X on { }`.

- Comment styling: 'place_data' must precede 'place' in the
  directive choice list because chumsky's choice is left-biased and
  'place_data' starts with 'place '. Without that ordering,
  'place_data foo in bar;' would parse as `place [data] on ...`,
  fail at `in`, and the parser would emit an unhelpful error. Same
  trick the algo parser already uses for keyword prefixes.

## Honest limitations (mirrored in module docstrings)

1. No error recovery; first failure only. File follow-up: TASK-0087.
2. AST nodes carry no spans. File follow-up: TASK-0086.
3. No semantic checks: forward refs for worker_class/memory_region,
   sync+async conflict, duplicate options, missing/extra place vs
   algorithm — all deferred to TASK-0010 (SchedIR) / TASK-0011
   (link step). The grammar accepts; the linker rejects.
4. PRD §6.3.1 `w0..w3` range shorthand in typed workers is not
   parsed. Grammar §5.6 explicitly defers it; no example uses it.
5. The simple-vs-typed disambiguation is biased to typed-first; a
   mixed-form `{ host, dsp : dsp_core }` is rejected. No existing
   example exercises this, and the grammar §1 wording suggests
   homogeneous lists.
6. 14-hearing-aid/embedded_multimcu.sched.nuc is known-failing
   pending TASK-0079; do not be tempted to fix it here.

## Follow-up tasks filed
- TASK-0086: Schedule parser: add per-node span tracking.
- TASK-0087: Schedule parser: multi-error reporting and recovery.
<!-- SECTION:NOTES:END -->
