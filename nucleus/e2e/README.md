# nucleus/e2e

The Nucleus v2 end-to-end differential matrix runner (`nucleus-e2e`).

## What it does

Walks the cell matrix declared in `nuc-nucleus/e2e-matrix.toml`:
each `(example, schedule, backend)` triple is one CELL. The harness:

1. Lowers the example via the compiler driver to per-cell scratch.
2. Builds the emitted Rust project (`cargo build --release`).
3. Runs the binary against the example's `input.bin`.
4. Diffs the resulting `output.bin` against the committed
   `reference.bin` (the std-only third-witness oracle).

The cell is PASS iff the bytewise diff is empty. The cross-backend
bit-identical differential is the headline thesis check.

## Usage

```bash
# Default (sequential, text summary):
just e2e

# Parallel — N worker threads, ~2-3x speedup at N=4:
nucleus-e2e --jobs 4

# Filter:
nucleus-e2e --example 13-cnn-inference --schedule pipeline_parallel

# Milestone-cumulative gating (M1 ⊆ M2 ⊆ M3 ⊆ M4):
just e2e-milestone M3

# CI integrations:
nucleus-e2e --format=junit            # JUnit XML on stdout
nucleus-e2e --emit-timings out.json   # Per-cell timings JSON
nucleus-e2e --baseline base.json      # Diff vs a baseline (delta table on stderr)
```

## Falsifier seams

The bit-identical thesis is only meaningful if the differential
actually BITES. Two negative gates are wired:

- `just determinism-check-negative` — perturbs one of the two
  determinism-mode trees post-emit; harness must report ≥1
  perturbed cell.
- `just xbackend-check-negative` — corrupts mp-tcp-bufsync's wire
  codec; harness must report ≥1 cross-backend diff detected.

Both are required by `just ci`.

## Tests

`cargo test -p e2e` covers the harness internals (arg parser, JSON
emitter + parser, perf-regression comparator, threshold gating,
manifest schema, cell planning, etc.). The matrix itself is run
via `just e2e`.
