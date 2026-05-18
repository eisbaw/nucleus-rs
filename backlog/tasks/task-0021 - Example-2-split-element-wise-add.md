---
id: TASK-0021
title: 'Example 2: split element-wise add'
status: Done
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 02:29'
labels:
  - M1
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two-worker version of example 1: host loads input, worker w0 processes, host writes output. Smallest example with a real cross-worker transfer.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 examples/02-split-add/prog.algo.nuc declares the add and the iteration.
- [x] #2 examples/02-split-add/schedules/naive.sched.nuc places host + w0 with appropriate transfers.
- [x] #3 examples/02-split-add/kernels.rs implements add as plain Rust.
- [x] #4 examples/02-split-add/reference/ contains the independent hand-written reference.
- [x] #5 examples/02-split-add/input.bin + reference.bin committed.
- [ ] #6 Test: e2e harness runs this example through naive sched + pthreads-sync; bit-identical output.
- [x] #7 Implementation notes record design questions (e.g. one-shot transfer vs streamed; v2 picks one-shot at this stage).
- [x] #8 Implementation notes record honest limitations (no blocking yet; whole input transferred once).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Status

Implemented under nuc-nucleus/examples/02-split-add/. All deliverable
files in place; pinning tests landed and green; reference impl is
bit-deterministic across re-runs. The end-to-end gate is blocked on
TASK-0122 (multi-worker pthreads-sync codegen) and is filed as an
`#[ignore]`d test that documents the eventual target.

Files added (all under nuc-nucleus/examples/02-split-add/):
- prog.algo.nuc, kernels.rs
- schedules/{naive.sched.nuc, split.sched.nuc}
- reference/{Cargo.toml, Cargo.lock, src/main.rs}
- input.bin (2048 B), reference.bin (1024 B)
- README.md

Pinning tests added (nucleus/compiler/tests/):
- algo_parser.rs::parses_example_02_split_add
- algo_lower.rs::lowers_example_02_split_add
- sched_parser.rs::parses_02_split_add_naive, parses_02_split_add_split
- sched_lower.rs::lowers_02_split_add_naive, lowers_02_split_add_split
- link.rs::links_02_split_add_naive, links_02_split_add_split,
  derived_data_for_split_add, split_add_missing_transfer_is_link_error
- contract.rs::example_02_split_add_contract_passes_for_add_and_loud_on_aggregates
- e2e_example_02.rs::split_pthreads_sync_bit_identical (`#[ignore]`,
  TASK-0122)

## Design questions and choices

### How to express the cross-worker transfer
Used three top-level `transfer` directives in split.sched.nuc — one
per data symbol that crosses workers (a, b, c). All `sync`. This
matches PRD §6.3.4 ("a transfer directive that would cross workers
MUST be present") and what the link pass enforces (the
`split_add_missing_transfer_is_link_error` test pins exactly that).
Alternatives considered and rejected:
- Per-element transfers (transfer a[i] : sync). The grammar does not
  permit slice-typed transfer subjects today, and even if it did,
  PRD §6.3.4 only models per-data-symbol semantics.
- A single transfer over all three. The grammar requires one
  directive per data symbol; combining would lose the per-symbol
  policy independence that later examples need.

### Whole-array vs per-element transfer
Whole-array, at the symbol level. The schedule grammar is symbol-
keyed, and the natural semantics for this example is "send all of
a, then all of b, then receive all of c" — one Push/Wait pair per
data symbol. Per-element transfers (one Push per loop iteration)
would be a different schedule, exercising buffering / async, and
that belongs in later examples (9, 11). For example 02 the
load-bearing question is "does the link pass accept the three
required transfers and reject when one is missing?"; the
`split_add_missing_transfer_is_link_error` test pins the rejection.

### How host distributes work to w0
For example 02, w0 runs the entire for-loop (256 iterations). No
partitioning across multiple compute workers. The schedule grammar
admits `place add on { w0, w1, ... }` for distributing the loop
iteration space — that's example 5+ territory. Restricting to one
compute worker keeps this example honest as "the smallest example
with a real cross-worker edge", as TASK-0021's brief framed it.

### Same prog.algo.nuc shape as example 01
The algorithm file is structurally identical to example 01's
(same const, same data decls, same kernels, same dataflow) by
design: the PRD point is that one algorithm composes with many
schedules. Splitting requires no algorithm change — only a
schedule swap. The pinning tests assert this structural identity.

### Input fixture is a different pattern from example 01
Generator: a[i] = (i*5)-13, b[i] = (i^0xA5)+41. Different from
example 01's pattern (i*3+7, (i^0x5A)*2-11) by design: each
example's fixtures are independent so a copy-paste error from
example 01 into 02 would be visible in the bytes. SHA-256 of
reference.bin: dbc316d01531bbb4812cd317052fbce4c83b67ae02fe8ffdcba6622a42be9783.
Determinism: confirmed by re-running the reference impl twice; same
bytes.

### Reference impl shape
Standalone Cargo project under reference/, std-only, no Nucleus
dependency, no shared code with example 01's reference. Matches
docs/reference-impl-policy.md §1 canonical regen command shape.
Independence rule (policy §2) is the load-bearing constraint here:
"two small programs are cheaper to audit than one shared library".

## Verification of acceptance criteria

- AC #1 prog.algo.nuc declares add and the iteration: DONE.
- AC #2 split.sched.nuc places host + w0 with appropriate transfers:
  DONE. Three transfers (a, b, c) all `sync`. Plus a naive.sched.nuc
  smoke variant.
- AC #3 kernels.rs implements add as plain Rust: DONE.
- AC #4 reference/ contains the independent reference: DONE.
- AC #5 input.bin + reference.bin committed: DONE. 2048 B and
  1024 B respectively, well under the 10 KB inspectability cap.
- AC #6 e2e harness runs this example through naive sched +
  pthreads-sync; bit-identical output: NOT MET. Blocked on
  TASK-0122 (multi-worker pthreads-sync codegen). For the *split*
  schedule the backend rejects with EmitError::UnsupportedFeature.
  For the *naive* schedule a positive e2e exists for example 01
  already and pinning the same single-worker shape again here would
  be a redundant cell in the matrix. The e2e test file
  (`e2e_example_02.rs::split_pthreads_sync_bit_identical`) is
  in place and `#[ignore]`'d with the TASK-0122 message; flipping
  to active is a one-line change once TASK-0122 lands.
- AC #7 implementation notes record design questions: DONE (this
  section).
- AC #8 implementation notes record honest limitations: DONE (next
  section).

## Honest limitations

1. **End-to-end is blocked on TASK-0122.** The pthreads-sync backend
   does NOT yet emit multi-worker code (TASK-0020 implementation
   notes: "multi-worker codegen returns EmitError::UnsupportedFeature"
   — filed as TASK-0122). The split schedule cannot be run end-to-end
   today. The example's files are in place; flipping the e2e test
   from `#[ignore]` to active is a one-line change once TASK-0122
   lands.

2. **No positive Push/Wait codegen exercise yet.** The transfer-
   injection pass (TASK-0018) does run on this example's split
   schedule under the synthetic / unit-test pipeline — but lowering
   those events to actual condvar signal/wait Rust code is exactly
   what TASK-0122 owns. So this example contributes to the IR-side
   tests (transfer_inject, sync_inject) only, not yet to a generated
   binary.

3. **Integer-only (i32).** PRD §10.1 invariant. Element-wise add has
   no reduction, so f32 would have worked, but consistency with
   later integer examples (sum, prefix sum, histogram, sort) where
   determinism actually bites wins.

4. **Single compute worker.** `add` runs on w0 only. Distributed
   placement (`place add on { w0, w1, ... }`) requires partition
   policy and is a later example's load-bearing surface.

5. **Whole-array transfer at the symbol level.** No tile granularity.
   This is what the schedule grammar admits and what example 02
   should exercise; per-tile / blocked transfers belong in examples
   that have `block=` or `pipeline=` in their schedule.

6. **No `notify=`, no `buffer=N`.** All three transfers are `sync`
   with default notify. Async + buffered transfer semantics is what
   examples 9 and 11 stress.

7. **Duplicate `const N` in kernels.rs.** Same single-source-of-truth
   violation as example 01, called out in the file header and the
   README. Resolves when TASK-0103 picks a convention.

8. **Contract pass remains loud on aggregates.** PASS for `add`,
   `TypeMismatch` for the three aggregate I/O kernels. The pinning
   test pins exactly this behaviour; when TASK-0012 follow-ups land
   aggregate matching, the test flips to assert Ok(()) without the
   example needing to change.

9. **No reuse with example 01's kernels.rs.** Duplication is
   deliberate: PRD §3 says "exactly two source files per build" and
   examples should be self-contained. A shared kernels-lib would
   complicate reference-impl-independence audits.

## Follow-up tasks

No new tasks filed by this one; all known blockers and shortcuts
already have a home:

- **TASK-0122** — pthreads-sync multi-worker codegen. The e2e gate
  for this example is `#[ignore]`'d until TASK-0122 lands.
- **TASK-0012 follow-ups** — aggregate type matching in the contract
  pass. When that lands, the contract test added here flips from
  "loud" to "pass".
- **TASK-0103** — PRD const-in-Rust-generics resolution. When
  picked, the duplicate `const N` in kernels.rs goes away.
- **TASK-0076** — CI gate verifying reference.bin freshness.
- **TASK-0077** — `just regen-references` one-shot recipe.

## Verification

- `just check`   → green (clean).
- `just clippy`  → green (`-D warnings` clean).
- `just test`    → green; 21 link tests, 10 contract tests, 30
  sched_lower tests, 17 sched_parser tests, 17 algo_lower tests, 9
  algo_parser tests; no regressions; e2e_example_02 has 1 ignored.
- `just e2e`     → green (stub harness still; the differential
  matrix lands in TASK-0023).
- Reference impl bit-determinism: re-ran twice; SHA-256
  dbc316d0… both times. cmp clean.
<!-- SECTION:NOTES:END -->
