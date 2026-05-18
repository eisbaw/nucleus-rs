---
id: TASK-0010
title: Define and lower schedule to SchedIR
status: Done
assignee: []
created_date: '2026-05-17 23:03'
updated_date: '2026-05-18 00:34'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define the schedule IR data types and the AST → SchedIR lowering pass. PRD §6.3.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes SchedIR types: WorkerSet, WorkerClass, MemoryRegion, Placement, PlaceData, LoopDirective, TransferDirective, CheckDirective.
- [ ] #2 Lowering supports both simple worker form and typed worker form (the latter desugars into the former with one default class).
- [ ] #3 Schedule completeness checks land here: every kernel must have a place; every cross-worker data symbol must have a transfer.
- [ ] #4 Test: lowering snapshot tests on each example schedule.
- [ ] #5 Test: incomplete schedules (missing place, missing cross-worker transfer) produce typed errors with the offending symbol name.
- [ ] #6 Implementation notes record design questions (e.g. how check directives compose; whether buffer=N on async is implied).
- [ ] #7 Implementation notes record honest limitations (e.g. check assertion machinery may be stubbed at M0 and finished at the latency milestone).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Commit: 2ae9071 compiler(M0): SchedIR types and AST -> IR lowering pass (TASK-0010)

## Design questions / choices made

- Default-class representation. The simple worker form
  (`workers = { host, w0 }`) is bound to a synthetic worker class
  named `__default` (DEFAULT_WORKER_CLASS constant). Three options
  were considered:
    (i) keep an `Option<class>` on ResolvedWorker (matches AST),
    (ii) synthesise the default class as a sibling ResolvedWorkerClass
        entry, with workers naming it,
    (iii) split into two IR variants (SimpleWorker / TypedWorker).
  Chose (ii). Rationale: downstream code wants one shape for
  "look up this worker's capabilities" — branching on the surface
  form duplicates lookup logic for no semantic gain. PRD sec 6.3.1
  literally says simple form is "equivalent to the typed form with a
  single default worker class", so the IR collapses to the typed form
  with a synthetic class. The `is_default` flag on
  ResolvedWorkerClass lets a backend know it has free rein over the
  default's capabilities. User-written class named `__default`
  collides loudly via DuplicateWorkerClass — the loud failure mode
  is preferred over silent picking up of the user's class.

- Synthesis only on demand. The default class is inserted only when
  at least one simple-form entry exists. If every entry is typed,
  the synthetic class doesn't appear in worker_classes. Keeps the
  IR's worker_classes faithful to "what the user wrote, plus what
  was implicit". Trade-off: backends must accept the absence of
  __default when all workers are typed.

- Bucketed source order on directive maps. places / place_data /
  loops / transfers / checks are BTreeMap-by-key (kernel / data /
  var). Source order on these is discarded. Rationale: per grammar
  sec.2 note 2 the directives are declarative and order within a
  kind is informational; semantics depend only on the (key, payload)
  set. The map also gives O(1) lookup, which the next pass
  (TASK-0011 link) will want. Per-directive option lists keep their
  order (Vec) because the duplicate / mutually-exclusive detection
  follow-up (TASK-0093) will want to report which option was first.

- Two-pass lowering. Pass 1 collects declarations (worker_class,
  memory_region, workers); pass 2 lowers reference-bearing
  directives (place, place_data, loop, transfer, check) against
  the symbol tables. Single-pass tempted but breaks the
  grammar-promised order-independence (note 2): `place k on w` can
  appear before `workers = { w }`. The clean fix is the two-pass
  pattern.

- Numeric-option zero-check. block/vectorize/unroll/pipeline/buffer
  reject N=0 with typed errors. The task spec called out block,
  vectorize, pipeline, buffer explicitly; I added unroll for
  consistency since it shares the same semantic ("how many" must
  be positive). All five live behind the same `positive` closure
  in lower_loop_option / lower_transfer_option.

- Time-literal normalisation: inherited from AST. The parser already
  normalises TimeLit to u64 nanoseconds; lowering passes the value
  through unchanged. No additional normalisation at the IR layer.

- Did NOT enforce: kernel-name resolution, data-symbol resolution,
  loop-variable resolution, completeness (every kernel has a place,
  every cross-worker data has a transfer). All four need the
  algorithm IR. Per the task instruction, these are TASK-0011 work.

- Did NOT enforce: capability-matrix validation (e.g. async on a
  sync-only backend). Per task: belongs in M1 (TASK-0019 owns
  capabilities.toml; the capability-vs-schedule cross-check is a
  later pass). Filed nothing new for it — TASK-0019 already exists.

## Honest limitations

1. Single-error reporting. Bails on the first violation. Same as
   the algorithm side; the parser's limitation propagates. Filing
   a follow-up for multi-error reporting in lowering is overkill
   while the parser is single-error too; revisit when both layers
   move together.

2. No spans. AST nodes carry no positions (TASK-0086 follow-up
   for the parser side). SchedLowerError variants carry identifying
   names but no line/column. When spans land, variants gain
   position fields without surface break.

3. `accessible_by` is NOT validated. The lowering pass passes the
   AST's list through to ResolvedMemoryRegion.accessible_by
   verbatim. Per grammar sec.2 note 4 "resolution is the linker's
   job" — but in fact `accessible_by` names refer to declared
   worker_class names (or worker names), which are entirely
   schedule-internal. Could be validated here. Filed as TASK-0095.

4. Duplicate / mutually-exclusive options on one directive are not
   detected. `block=64, block=128` on one loop, `sync, async` on
   one transfer — accepted (the Vec preserves the source order).
   Grammar sec.2 notes 5, 7 call these linker concerns; filed as
   TASK-0093.

5. `place k on { w0, w0 }` (duplicate worker in a Many target) is
   accepted. Filed as TASK-0094.

6. The synthetic-default-class collision rule: a user-written
   `worker_class __default { ... }` is rejected, BUT only when a
   simple-form worker is also present (because that's when the
   synthesis runs). A user could declare `worker_class __default`
   in a schedule with only typed workers and the IR would happily
   index it — and a future simple-form addition would then collide
   suddenly. Not filing a follow-up: the documented convention is
   "don't name your class `__default`", and the loud-failure path
   is preserved.

7. Multiple `workers = { ... }` directives are rejected
   (DuplicateWorkersDecl). The grammar's SchedItem rule technically
   permits repetition; the IR rejects because merge semantics
   (override? concatenate?) are not specified in the PRD. If a real
   example wants two decls, the rule can be relaxed with a clear
   merge policy.

## AC verification

- AC #1 (compiler crate exposes SchedIR types). MET. sched::ir
  exports ResolvedWorkerClass, ResolvedMemoryRegion, ResolvedWorker,
  ResolvedPlacement, ResolvedPlaceData, ResolvedLoopDirective,
  ResolvedTransferDirective, ResolvedCheckDirective, plus all option
  enums and SchedIR / SchedLowerError. Re-exported via sched::mod.

- AC #2 (both simple and typed worker forms lower to one shape).
  MET. ResolvedWorker { name, class } — simple-form workers get
  class = DEFAULT_WORKER_CLASS; typed-form workers keep their
  written class. The synthetic class is inserted into
  worker_classes only when at least one simple-form entry exists.
  Two tests cover both forms (lowers_05_stencil_naive,
  lowers_typed_workers_and_memory_regions).

- AC #3 (completeness checks: every kernel has place; every cross-
  worker data has transfer). PARTIAL / DEFERRED. SchedIR does not
  see the algorithm. The task instruction explicitly defers these
  to TASK-0011 (link step). What this task DOES enforce is the
  schedule-internal uniqueness side (at most one place per kernel,
  at most one transfer per data); the existence side belongs to
  TASK-0011. Stated honestly in module docstring and notes.

- AC #4 (lowering snapshot tests on each example schedule). MET via
  structural assertions (counts + spot-checks), not full-IR
  snapshots — same choice as algo_lower.rs to avoid brittleness as
  the IR shape evolves. Five files exercised; the sixth
  (14-hearing-aid/embedded_multimcu) is parse-failing per
  TASK-0079 so cannot be lowered.

- AC #5 (incomplete schedules produce typed errors with offending
  name). MET for the SCHEDULE-INTERNAL side (negative tests
  for every defended variant). The algorithm-cross-check side
  (TASK-0011) will produce the kernel/data name in its errors.

- AC #6 (notes record design questions). MET (above).

- AC #7 (notes record honest limitations). MET (above).

## Follow-up tasks filed

- TASK-0093: detect duplicate / mutually-exclusive options on a
  single directive.
- TASK-0094: detect place_set with duplicate worker names.
- TASK-0095: validate `accessible_by` references against declared
  worker_class / worker names.

## Verification

- just check  -> pass
- just clippy -> pass (-D warnings)
- just test   -> pass (27 sched_lower + 14 sched_parser
                       + 13 algo_lower + 10 algo_parser
                       + 0 in remaining crates)
- just e2e    -> pass (stub binary)
<!-- SECTION:NOTES:END -->
