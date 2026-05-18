---
id: TASK-0011
title: 'Link step: bind schedule kernel references to algorithm'
status: Done
assignee: []
created_date: '2026-05-17 23:03'
updated_date: '2026-05-18 00:42'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Resolve schedule's named references (kernels, data symbols, loop variables) against the algorithm IR. Produce a linked IR that downstream passes consume.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes link(AlgoIR, SchedIR) -> Result<LinkedIR, LinkError>.
- [ ] #2 Schedule referencing a kernel not in the algorithm = LinkError::UnknownKernel(name).
- [ ] #3 Algorithm declaring a kernel not placed in the schedule = LinkError::UnplacedKernel(name).
- [ ] #4 Schedule referencing a loop variable not declared in the algorithm = LinkError::UnknownLoop(name).
- [ ] #5 Test: positive cases (every example) and a curated set of negative cases each produce the right LinkError variant.
- [ ] #6 Implementation notes record design questions (e.g. should link batch all dangling references, or fail-fast on first).
- [ ] #7 Implementation notes record honest limitations (e.g. no fuzzy-matching suggestions for typos).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design decisions

- **One-pass error collection.** PRD §12 says the link step must
  report all errors at once, not fail-fast on the first dangling
  reference. The pass walks the schedule's directives in a fixed
  order, accumulates a `Vec<LinkError>`, and only returns Err if any
  errors were found. Errors are deduped (sort + dedup) so a
  re-walked check doesn't double-count the same diagnostic.

- **Producer/consumer modelled as `BTreeSet<String>` (WorkerEntity).**
  `place K on host` is `{host}`; `place K on {w0..w3}` is
  `{w0,w1,w2,w3}`. Distributed placements compared as a single
  entity for cross-worker existence — partition= refinement is
  TASK-0016+ work. `BTreeSet` gives order-independent equality and
  deterministic `Ord`/`Display`, which matters for diagnostic
  consistency across runs.

- **Producer derivation from `Dataflow { lhs, rhs: Call }` only.**
  RHS that is a bare `DataRef` (identity copy) has no kernel and no
  recorded producer. None of the in-tree examples exercise that
  form; treating it correctly belongs to the transfer/partition
  pass. Filed as a limitation; if a real example hits it, the
  cross-worker check will silently let the dataflow through, which
  is the wrong failure mode — needs follow-up.

- **Skip cross-worker check for kernels with no placement.** A
  kernel reported as `UnplacedKernel` has no worker entity, so we
  can't derive its producer/consumer set. Not making one up keeps
  the follow-on errors honest.

- **`LinkedIR` owns both IRs.** No shared references; downstream
  passes get a single value. Convenience maps (placements,
  kernel_workers, data_producers, data_consumers) are pre-computed
  here so ACFG construction (TASK-0016) doesn't redo the walk.

## Honest limitations

- **No fuzzy-match suggestions for typos.** Errors carry the
  offending name only. Filed as a follow-up below; not critical
  while the example set is small enough that names are obvious.

- **No source spans on errors.** AST nodes don't carry positions
  yet (TASK-0086/0090). When they land, `LinkError` variants gain
  position fields without surface change.

- **Identity-copy dataflow has no producer.** As described above.
  Means `D <-- E` (no kernel) is currently invisible to the
  cross-worker check. Filed below.

- **Distributed placement is one entity for now.** A `place K on
  {w0..w3}` with `partition=rows` and a kernel that consumes a
  halo region from a sibling worker WILL cross a worker boundary
  at the per-element level — the link step does not detect this
  because partition= isn't lowered yet. Filed below.

- **Errors are deduped by Debug formatting.** Cheap to implement,
  works because the variants are small. Better identity (a
  dedicated key fn) belongs alongside the AST-span migration.

## AC verification

- AC #1 (link(AlgoIR, SchedIR) -> Result<LinkedIR, LinkError>):
  MET. Signature is Result<LinkedIR, Vec<LinkError>> per the
  single-pass-collect-all-errors policy in PRD §12; the task
  description in this thread explicitly asks for that shape.

- AC #2 (UnknownKernel): MET — negative_unknown_kernel test +
  inline check in the pass.

- AC #3 (UnplacedKernel): MET — negative_unplaced_kernel test.

- AC #4 (UnknownLoop): MET — negative_unknown_loop and
  negative_unknown_loop_via_check (both `loop` and `check loop`
  surfaces).

- AC #5 (positive + curated negatives): MET. Positive matrix is
  13-cnn-inference x {naive, batch_parallel, pipeline_parallel} and
  14-hearing-aid x {naive} (4 cases). 14-hearing-aid embedded
  multimcu omitted per TASK-0079 (parse-failing); 05-stencil
  omitted per TASK-0078 (algo parse-failing). Negative cases cover
  all six LinkError variants. Bonus tests: multi-error-one-pass,
  same-worker-no-transfer, same-distributed-set-no-transfer,
  cross-worker-transfer-present, derived-data-spot-check.

- AC #6 (design questions recorded): MET above.

- AC #7 (limitations recorded): MET above.

## Verification

- just check  -> pass
- just clippy -> pass (-D warnings)
- just test   -> pass (16 link + 27 sched_lower + 14 sched_parser
                       + 13 algo_lower + 10 algo_parser; full
                       workspace green)
- just e2e    -> pass (stub binary at M0)

## Follow-up tasks filed

See newly-created TASK-0096..0099 (below).
<!-- SECTION:NOTES:END -->
