# Getting started: your first Nucleus program

This tutorial walks you through writing a Nucleus program **from
scratch**, compiling it for two different backends, and confirming that
both produce byte-identical output. It is the smallest end-to-end loop
that exercises the central idea of the project — the **algorithm /
schedule split** — without copying one of the worked examples.

The complete tutorial sources live in
[`docs/tutorial/`](tutorial/) (deliberately *outside*
`nuc-nucleus/examples/`, so the e2e differential matrix does not
enumerate them). The whole thing is also wired into a `just` recipe so
it cannot silently rot:

```
nix develop --command just tutorial
```

That recipe builds the program on both backends, runs both, and fails
loudly if either stops compiling or the two outputs diverge.

---

## 0. Prerequisites

Everything runs inside the Nix dev shell. From the repo root:

```
nix develop
```

All `cargo` / `just` commands below assume you are inside that shell
(the toolchain pin in `flake.nix` is the single source of truth for the
Rust version). The compiler driver is the `nucleus` binary in the
`nucleus/` workspace.

---

## 1. The idea: algorithm vs schedule

A Nucleus program is **two** files:

- An **algorithm** (`*.algo.nuc`) — *what* to compute. It declares data
  arrays, kernels (real Rust functions), and a dataflow over them. It
  names no worker, no transfer, and no backend.
- A **schedule** (`*.sched.nuc`) — *where, when, and how*. It assigns
  each kernel to a worker, declares any loop transforms, and declares a
  `transfer` for every data symbol that crosses a worker boundary.

The compiler proves the two fit together and emits parallel code for a
chosen backend. The same algorithm can run under wildly different
schedules — single worker, split across processes, distributed — and
every backend must produce **byte-identical** output. That byte-identity
is the project's correctness gate.

The kernel bodies themselves are ordinary Rust in an adjacent
`kernels.rs`; Nucleus never rewrites them.

---

## 2. The algorithm: scale-and-bias

We compute an integer "scale-and-bias" (a small saxpy):

```
c[i] = K * a[i] + b[i]      for i in 0 .. N,   K = 3
```

[`docs/tutorial/prog.algo.nuc`](tutorial/prog.algo.nuc):

```nuc
const N : usize = 64;

data a : i32[N];
data b : i32[N];
data c : i32[N];

kernel scale_bias : (i32, i32) -> i32 pure;

kernel load_a : ()       -> i32[N] effectful;
kernel load_b : ()       -> i32[N] effectful;
kernel save_c : (i32[N]) -> ()     effectful;

a <-- load_a();
b <-- load_b();

for i : 0 .. N {
    c[i] <-- scale_bias(a[i], b[i]);
}

save_c(c);
```

Notes that matter:

- **`pure` vs `effectful`.** `scale_bias` is `pure`: the compiler may
  reorder, deduplicate, or eliminate it. The I/O kernels are
  `effectful`: their ordering is preserved and they are never
  duplicated.
- **Integer arithmetic.** `i32` is bit-deterministic by language
  definition — exactly what the cross-backend bit-identical differential
  needs. The scale factor `K` lives in the kernel body, not the algo,
  because it is a property of the arithmetic, not the dataflow.
- **Single assignment.** The `for` loop writes each `c[i]` exactly once.

The kernel bodies are in [`docs/tutorial/kernels.rs`](tutorial/kernels.rs);
`scale_bias` uses wrapping arithmetic (`K.wrapping_mul(a).wrapping_add(b)`)
so overflow is deterministic. The I/O kernels read/write little-endian
`i32` words and take their paths from `NUC_INPUT_PATH` /
`NUC_OUTPUT_PATH`.

---

## 3. Schedule #1: naive (single worker)

[`docs/tutorial/schedules/naive.sched.nuc`](tutorial/schedules/naive.sched.nuc)
puts everything on one worker (`host`). No data crosses a worker
boundary, so no `transfer` directives are needed:

```nuc
schedule for "../prog.algo.nuc" {
    workers = { host };

    place load_a     on host;
    place load_b     on host;
    place save_c     on host;
    place scale_bias on host;
}
```

Build it for the shared-memory backend `pthreads-sync` (a single
binary):

```
cd nucleus
cargo run --bin nucleus -- build \
    --algo      ../docs/tutorial/prog.algo.nuc \
    --sched     ../docs/tutorial/schedules/naive.sched.nuc \
    --kernels   ../docs/tutorial/kernels.rs \
    --backend   pthreads-sync \
    --out       /tmp/tut-naive
```

This emits a self-contained Cargo project under `/tmp/tut-naive`. Build
and run it (the generated binary is `nuc-generated`):

```
cd /tmp/tut-naive && cargo build --release
NUC_INPUT_PATH=$PWD/../../docs/tutorial/input.bin \
NUC_OUTPUT_PATH=$PWD/output.bin \
    ./target/release/nuc-generated
```

`output.bin` now holds N little-endian `i32` words: `3*a[i] + b[i]`.

---

## 4. Schedule #2: split (two workers, one cross-worker edge)

[`docs/tutorial/schedules/split.sched.nuc`](tutorial/schedules/split.sched.nuc)
keeps the *same algorithm* but splits it across two workers: `host` does
I/O, `w0` does the compute. Now three data symbols cross the boundary,
so each needs a `transfer`:

```nuc
schedule for "../prog.algo.nuc" {
    workers = { host, w0 };

    place load_a on host;
    place load_b on host;
    place save_c on host;
    place scale_bias on w0;

    transfer a : sync;   // host -> w0
    transfer b : sync;   // host -> w0
    transfer c : sync;   // w0 -> host
}
```

Omitting a required `transfer` is a **hard error**, not a silent
default. Build it for the multi-process backend `mp-tcp-bufsync` (two OS
processes over TCP loopback):

```
cd nucleus
cargo run --bin nucleus -- build \
    --algo      ../docs/tutorial/prog.algo.nuc \
    --sched     ../docs/tutorial/schedules/split.sched.nuc \
    --kernels   ../docs/tutorial/kernels.rs \
    --backend   mp-tcp-bufsync \
    --out       /tmp/tut-split
```

A multi-process backend emits a `run.sh` launcher instead of a single
binary:

```
cd /tmp/tut-split && cargo build --release
bash run.sh ../../docs/tutorial/input.bin /tmp/tut-split/output.bin
```

---

## 5. The payoff: byte-identity

The two outputs were produced by radically different machinery — one
binary sharing memory, two processes talking over TCP — yet they must be
the same bytes:

```
cmp /tmp/tut-naive/output.bin /tmp/tut-split/output.bin && echo "identical"
```

That is the whole point. If they ever differ, a backend has a bug; the
differential test is what catches it.

The `just tutorial` recipe automates all of the above (build both, run
both, `cmp` the outputs) and is the rot-proofing for this document: if
the tutorial program stops compiling or the backends diverge, the recipe
exits non-zero.

---

## 6. Where to go next

- **Worked examples:** [`nuc-nucleus/examples/`](../nuc-nucleus/examples/)
  — 29 examples from element-wise add to CNN inference and a multi-MCU
  hearing aid, each with a `README.md`.
- **The grammar (the language contract):**
  [`docs/grammar-algo.md`](grammar-algo.md) and
  [`docs/grammar-sched.md`](grammar-sched.md).
- **What is stable vs may change:** [`docs/stability.md`](stability.md).
- **CLI reference (flags, exit codes, `--emit-pn`):**
  [`docs/cli-reference.md`](cli-reference.md).
- **Diagnostics:** [`docs/diagnostics-audit.md`](diagnostics-audit.md)
  audits every user-facing error surface.
- **The specification:** [`nuc-nucleus/PRD.md`](../nuc-nucleus/PRD.md).
