---
id: TASK-0013
title: 'Example 1: element-wise add — algorithm + naive schedule + reference'
status: Done
assignee: []
created_date: '2026-05-17 23:03'
updated_date: '2026-05-18 01:07'
labels:
  - M0
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Smallest possible end-to-end example. Algorithm: c[i] <-- add(a[i], b[i]) over a 1D iteration. Naive schedule places everything on host. Reference impl is a hand-written Rust function.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 examples/01-elementwise-add/prog.algo.nuc declares input arrays a, b, output c, kernel add, dataflow.
- [x] #2 examples/01-elementwise-add/schedules/naive.sched.nuc places everything on host.
- [x] #3 examples/01-elementwise-add/kernels.rs implements add as a plain Rust function.
- [x] #4 examples/01-elementwise-add/reference/ contains an independent hand-written Rust implementation.
- [x] #5 examples/01-elementwise-add/input.bin and reference.bin committed; small enough (<10KB) to be inspected.
- [x] #6 examples/01-elementwise-add/README.md describes what the example stresses.
- [x] #7 Test: reference impl run on input.bin produces reference.bin (CI check).
- [x] #8 Implementation notes record any decisions about input format and size.
- [x] #9 Implementation notes record honest limitations (e.g. integer-only at this point).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Status

Implemented at commit 056a1c2. All ACs ticked (see below).

## Design questions and choices

### Integer vs float

Chose **i32**. PRD §10.1 leans integer for bit-identical tier-1 testing.
Even though element-wise add has no reduction (so f32 would have worked
without controversy), staying integer-typed:

1. matches the convention later examples (sum, prefix-sum, histogram,
   sort) actually need;
2. makes overflow semantics explicit via `i32::wrapping_add` — no
   fast-math, no FMA, no platform-dependent rounding;
3. produces fixtures that read cleanly in `hexdump -C`.

### Vec<i32> vs &[i32] vs [i32; N]

Chose **Vec<i32>** for I/O kernel Rust signatures. `[i32; N]` would
trip TASK-0103 (PRD const-in-generics bug). `&[i32]` would force the
caller (= a hypothetical future runtime) to manage the storage; for a
fully owned, transferable buffer Vec is the cleaner shape. Documented
in kernels.rs header and README.

Cost: `const N: usize = 256;` is duplicated inside kernels.rs alongside
the Nuc-side `const N`. Deliberate single-source-of-truth violation,
called out in code comments; resolves when TASK-0103 picks a
convention.

### Reference impl shape: single .rs or Cargo project

Chose **Cargo project** under `reference/`. Reasons:

- Matches docs/reference-impl-policy.md §1 canonical regen command
  (`cargo run --manifest-path .../reference/Cargo.toml`).
- Gives an explicit `[workspace]` empty table so the crate stays
  outside the nucleus workspace by intent, not by accident.
- `cargo run --release` is the obvious incantation users will reach
  for; needing a separate `rustc` line would be friction.

Cost: a sibling `Cargo.lock` is tracked. Acceptable — locks tier-1
reference impls to a reproducible dep graph (currently zero deps, but
defensive against future additions).

### input.bin format

Two i32 LE arrays concatenated: `a[0..N]` then `b[0..N]`. N=256 so
each array is 1024 bytes, total 2048 bytes. reference.bin is 1024
bytes. Both well under the 10 KB AC cap and hex-dumpable.

Pattern (in README and reproducible in 4 Python lines):
- `a[i] = i * 3 + 7`
- `b[i] = (i ^ 0x5A) * 2 - 11`

Non-trivial (varied across i, not symmetric) so a bug that drops the
b argument or swaps indices is observable in c.

## Verification of acceptance criteria

- AC #1: prog.algo.nuc has `data a, b, c : i32[N]`, kernel `add`,
  load/save kernels, dataflow with for-loop. **Done.**
- AC #2: schedules/naive.sched.nuc places everything on host. **Done.**
- AC #3: kernels.rs implements `add` as plain Rust. **Done.**
- AC #4: reference/ is a standalone Cargo crate, std-only. **Done.**
- AC #5: input.bin (2048 B) and reference.bin (1024 B) committed,
  <10 KB cap respected. **Done.**
- AC #6: README.md describes stresses, format, regen. **Done.**
- AC #7: reference impl on input.bin produces reference.bin
  bit-identically; verified via cmp + sha256sum. CI hook to gate this
  is TASK-0076 (already filed). **Done modulo CI hook.**
- AC #8: implementation notes record format/size decisions. **Done
  (this note).**
- AC #9: implementation notes record limitations. **Done (next
  section).**

Additional verification beyond AC list (per task instructions):
- algo parser test: `parses_example_01_elementwise_add` — counts and
  purities asserted.
- algo lower test: `lowers_example_01_elementwise_add` — N resolved
  to 256, ResolvedType for `a`, statement-shape sequence.
- sched parser test: `parses_01_elementwise_add_naive`.
- sched lower test: `lowers_01_elementwise_add_naive`.
- link test: `links_01_elementwise_add_naive`.
- contract test:
  `example_01_elementwise_add_contract_passes_for_add_and_loud_on_aggregates`
  — pins the current scalar-only behaviour.
- `cargo clippy --workspace -- -D warnings` clean.
- `cargo clippy` clean on the reference crate.
- `cargo fmt --all -- --check` clean.

## Honest limitations

1. **Contract pass is scalar-only at present.** The contract pass
   produces `TypeMismatch` with an "aggregate type matching is not yet
   implemented" message for `load_input`, `load_input_b`, and
   `save_output`. This is intended (loud failure rather than silent
   acceptance) at TASK-0012's scope. The contract test pins exactly
   this behaviour: pass on `add`, loud on the three aggregate I/O
   kernels. When aggregate matching lands, the test should flip to
   asserting `Ok(())` without the example file needing to change.

2. **Duplicate `const N` in kernels.rs.** Single-source-of-truth
   violation, documented in the file header and README. Resolves when
   TASK-0103 picks a convention for how Nuc consts flow into kernels.rs
   (substitution / duplication / dynamic shape).

3. **Only naive schedule.** Example 2 picks up distributed
   decomposition; this one is the smoke test. No `block`, no
   `vectorize`, no `transfer` — those exercise different surfaces
   appropriate to later examples.

4. **No actual runtime yet.** kernels.rs is a contract artefact at M0
   — there is no Nucleus-emitted host program to call into it. The
   reference impl is what produces reference.bin today. End-to-end
   "Nucleus-compiled binary diffs against reference.bin" lands at M1
   when the pthreads-sync backend exists. This example's job at M0
   is to be the input the M1 backend will consume.

5. **No CI gate on reference.bin freshness.** docs/reference-impl-policy.md
   §6 calls this out: until M2, drift is a reviewer-checklist concern,
   not a CI check. TASK-0076 owns that hook.

6. **Small input.** N=256 fits comfortably in 10 KB; doesn't exercise
   any size-related corner. Acceptable for a smoke test; larger
   examples will stress IO buffering later.

## Follow-up tasks

No new tasks filed by this one; all known shortcuts already have a
home:

- TASK-0012 follow-ups: aggregate type matching in the contract pass.
  When that lands, the contract test added here flips from "loud" to
  "pass".
- TASK-0103: PRD const-in-Rust-generics resolution. When picked, the
  duplicate `const N` in kernels.rs goes away.
- TASK-0076: CI gate verifying `reference.bin` is fresh.
- TASK-0077: `just regen-references` one-shot recipe.
<!-- SECTION:NOTES:END -->
