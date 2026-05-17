# Example 13 — Small CNN Inference

Layer-wise dataflow demonstrating the algorithm/schedule split across
three radically different decomposition patterns. One algorithm
(`prog.algo.nuc`), three schedules, same output everywhere.

## What this stresses

| Axis | What                                                         |
| ---- | ------------------------------------------------------------ |
| Algorithmic | Multi-stage layer dataflow. Per-sample loop nest.     |
| Scheduling  | Three orthogonal decompositions of the same algorithm: serial, batch-parallel, pipeline-parallel. |
| Backends    | All three schedules must work on every tier-1 backend, on MPI for tier 2, and in Renode on tier 3. |

This is the load-bearing demonstration of the v2 pitch: same algorithm,
different schedules, same correct output, across radically different
transports.

## Required schedules

All three are required for any backend to claim conformance on this
example.

- `naive.sched.nuc` — one worker, sequential batch. Smoke test.
- `batch_parallel.sched.nuc` — four compute workers; partition the
  batch loop.
- `pipeline_parallel.sched.nuc` — three compute workers; one layer
  per worker, three samples in flight.

## Why no training

Training requires backward-pass gradient synchronisation. The cheap
way is AllReduce; v2 emits only point-to-point in M7. Until collective
recognition lands (post-M8), training won't fit the model. Inference
is sufficient to demonstrate the schedule story, and is what most
deployed ML cares about anyway.

## Reference

`reference/` (TODO) contains a hand-written single-threaded Rust
implementation of the same forward pass. CI compares all backends
against its `output.bin` for a fixed `input.bin` and fixed weights.

Weights are deterministic — either baked into the kernel functions as
`const` arrays, or loaded from a committed binary file. Determinism is
non-negotiable for the differential test (see §10.1 of the PRD).

## What this example does *not* stress

- Halo regions (handled by example 5).
- Reuse on inner loops (example 5).
- Reduction patterns requiring collectives (example 3, partially).
- Wavefront / triangular dependencies (example 10).

If you find yourself wanting any of those, use the appropriate example
to test that axis — don't bloat this one.
