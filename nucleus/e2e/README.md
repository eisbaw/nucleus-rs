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

## Generative differential arm (`diff-fuzz`)

The curated matrix above is complemented by a generative,
property-based cross-backend differential fuzzer
(`src/bin/diff_fuzz/`, the `diff-fuzz` binary). From a seed it
SYNTHESISES structured single-assignment integer programs and
subjects each to the same test the curated rig applies: compile
across the program's tier-1 backend set, run against generated
input, and require mutual byte-identity — additionally cross-checked
against an in-process Rust reference computed directly from the
synthesised program.

It relies on NO hand-written example. Generation is deterministic in
the seed, so any divergence reproduces exactly (`--prog-seed`
regenerates a single failing program). It is run on demand, not on
every build, because each synthesised program costs several
independent backend builds.

```bash
# Default: 8 programs, seed 1.
just diff-fuzz

# Larger seeded sweep:
just diff-fuzz 12345 120          # seed=12345, k=120

# Reproduce one failing program from a failure report:
nix develop --command cargo run --release --bin diff-fuzz -- --prog-seed <N>

# Per-command timeout knob (a hang -> reported FAIL, never a stall):
DIFF_FUZZ_TIMEOUT_SECS=120 just diff-fuzz 1 8
```

### Families (the synthesised subclass)

Five structured families, each modelled on a proven curated example:

- **pipeline1d** — 1-D elementwise integer pipeline, host+w0 split
  (`02-split-add`). 7-backend.
- **stencil2d** — 2-D vertical 3-point stencil whose `y±1` reads are
  in the partition axis, forcing halo inference; `partition=rows`
  with plain `sync` transfers (`05-stencil`, sync variant).
  7-backend.
- **reduction** — partitioned binned reduction over ALL SIX combine
  operators (sum / or / xor / and / min / max) with identity-element
  edge cases — empty bins, and the non-zero identities min=`i32::MAX`
  / max=`i32::MIN` / and=all-ones (`26-bin-min`). 7-backend.
- **partition_workers** — multi-COMPUTE-worker `partition=workers`
  elementwise map (`03-reduction` distributed shape). 7-backend.
- **for_until** — bounded `for..until` single-worker convergence
  shape, cap + exact integer halt predicate (`21-jacobi-converge`).
  **pthreads-sync ONLY** — the curated matrix itself skips
  `21-jacobi-converge` on the other six backends (the break emit is
  single-worker pthreads-sync today; the cross-backend break
  differential is epic S7). For this family the harness checks
  self-consistency + reference agreement on that single backend.

### Honest scope of the in-process reference

The reference guards against COMPILER common-mode (all backends
mistranslating the SAME kernel identically). It does NOT guard
against SPECIFICATION common-mode: each operator's reference (`apply`)
and emitted kernel body are two transcriptions of the SAME operator
definition, so a conceptual error in an op's definition would appear
identically in both and escape — the same author-intent common-mode
bound the thesis already states for the hand-written corpus oracles.

## Tests

`cargo test -p e2e` covers the harness internals (arg parser, JSON
emitter + parser, perf-regression comparator, threshold gating,
manifest schema, cell planning, etc.) AND the `diff-fuzz` internals
(per-family generators, reference oracles, the seeded RNG, and the
per-command timeout / process-group kill). The matrix itself is run
via `just e2e`; the generative arm via `just diff-fuzz`.
