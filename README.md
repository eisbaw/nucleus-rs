# Nucleus v2

A pre-compiler that takes **two** annotated source files in the Nuc
language — an *algorithm* (Rust kernels + dataflow + iteration) and a
*schedule* (workers + mapping + blocking + IO semantics) — and emits
split, statically-scheduled, parallel code for a range of backends
spanning commodity CPU, HPC clusters, and embedded targets.

## What this is / isn't

**Is**: a thesis-grade implementation of the algorithm/schedule split,
with a falsifiable cross-backend bit-identical differential test as the
correctness gate. The same algorithm runs under multiple radically
different decompositions (single-worker, batch-parallel, distributed)
and must produce byte-identical output across backends. The central
commitment is the **algorithm/schedule separation** — the algorithm
states *what* to compute; the schedule states *where, when, and how* to
compute it; the compiler proves they fit and emits the code.

**Isn't**: a production polyhedral compiler, an auto-tuner, or a
distributed training framework. Backward pass and collectives are
deliberately out of scope for v2.

## Pointers

- **[`nuc-nucleus/PRD.md`](nuc-nucleus/PRD.md)** — the specification.
  Start here. Everything else is implementation.
- **[`nuc-nucleus/examples/`](nuc-nucleus/examples/)** — 29 worked
  examples (fourteen driving examples per PRD §9 plus fifteen later
  extensions, 15–29) from element-wise add (one kernel, one for-loop)
  to CNN inference (multi-layer i32 deterministic), a multi-MCU
  hearing aid, a DMA-async + PIO-sync transport demo, a map-reduce
  dot product (inner product), a rank-1 outer product (the
  rank-expansion counterpart of a reduction), a distributed
  XOR-combine bin-parity (the non-sum accumulator-combine identity),
  a distributed MIN-combine bin-min (the non-zero-identity
  accumulator combine, init to `i32::MAX`), a distributed FLOAT
  MIN-combine bin-fmin (the f32 order-independent combine, init to
  `f32::INFINITY`), and a distributed reproducible FLOAT SUM bin-fsum
  (the opt-in `combine=fsum` fixed-order fold: bit-identical across
  backends for a given schedule, though not the naive IEEE sum — plain
  `combine=sum` on a float stays rejected as non-associative per PRD
  §10.1). Each example is `prog.algo.nuc` + one or more
  `schedules/*.sched.nuc` + a `kernels.rs` + an independent reference
  impl + an `input.bin` + an expected `reference.bin`.
  <!-- check-readme-counts: examples=29 (filesystem-truth gate; bump when adding/removing an examples/NN-* dir) -->
- **[`docs/`](docs/)** — grammar documents and the reference-impl
  policy.
- **[`nucleus/`](nucleus/)** — the Rust workspace: `nucleus-compiler/`
  (parser + IR + passes), `backends/` (pthreads-sync, mp-tcp-bufsync, …),
  `driver/` (the `nucleus` CLI), and `e2e/` (the differential matrix).
- **[`backlog/`](backlog/)** — task tracker (`backlog` CLI). Tasks
  carry plans, notes, and dependencies; decisions live under
  `backlog/decisions/`.

## Running

The repo provides a Nix dev shell + a `justfile`:

```
nix develop -c just build       # cargo build --workspace
nix develop -c just test        # cargo test --workspace
nix develop -c just e2e         # full cross-backend differential
nix develop -c just ci          # the gate CI runs
```

`just e2e` is the load-bearing test: it builds + runs every
example × schedule × backend cell and diffs the output against the
reference. Bit-identical or it fails.
