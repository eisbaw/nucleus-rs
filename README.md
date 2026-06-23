# Nucleus

A pre-compiler for portable parallel software. You write **two** files —
an *algorithm* (what to compute: Rust kernels, dataflow, iteration) and
a *schedule* (how to lay it out: workers, mapping, blocking, and IO
semantics) — and Nucleus emits split, statically-scheduled, parallel
Rust for a range of backends spanning commodity CPU, HPC clusters, and
embedded microcontrollers. The algorithm never mentions a worker, a
buffer, a transport, or a barrier; change the schedule (or the backend)
and the algorithm stays untouched.

## Lineage

Nucleus is a **clean-room reimplementation** of the author's MSc (Hons)
thesis work, originally carried out at Intel, on compiling a single
annotated source for **multi-ASIP VLIW** targets — arrays of
application-specific VLIW processors. That original system proved the
core idea but was deliberately narrow: a single proprietary target, one
backend, and a handful of demonstrations, with a genericity claim that
could not be *falsified* because there was nothing to vary it against.

This v2 is a from-scratch, clean-room port — it shares the *ideas*, not
the code — rebuilt in **Rust** both as the language the compiler is
written in and as the language it emits for every target. The scope and
the rigour are both substantially higher: ten backends across three
tiers (commodity CPU, MPI clusters, and bare-metal embedded), 29 worked
examples, a Petri-net soundness gate that rejects unbounded or
deadlocking schedules at compile time, and — the load-bearing addition —
a **cross-backend bit-identical differential test** that compiles the
*same* algorithm under every schedule and backend and requires
byte-for-byte identical output. That iso-output demonstration across
radically different runtimes is what turns the portability claim from an
assertion into something a single differing byte can refute.

## What this is / isn't

**Is**: an implementation of the algorithm/schedule split, with a
falsifiable cross-backend bit-identical differential test as the
correctness gate. The same algorithm runs under multiple radically
different decompositions (single-worker, distributed, pipelined) and
must produce byte-identical output across every backend. The central
commitment is the **algorithm/schedule separation** — the algorithm
states *what* to compute; the schedule states *where, when, and how*;
the compiler synthesises the transfers and synchronisation, proves the
result is bounded and deadlock-free, and emits the code.

**Isn't**: a production polyhedral compiler, an auto-tuner, or a
distributed training framework. The supported algorithm class is affine,
static, single-assignment, and integer-centric (deterministic
floating-point only where reductions do not reorder). Data-dependent
control flow, recursion, dynamic scheduling, and a GPU backend are
deliberately out of scope; `check` assertions are *checked*, not
optimised against (there is no cost model).

## One algorithm, many schedules

The algorithm declares shaped data, kernel contracts (real Rust
functions, bodies in a sibling `kernels.rs`), and a single-assignment
dataflow body. Here is an element-wise add:

```
const N : usize = 256;

data a : i32[N];
data b : i32[N];
data c : i32[N];

kernel add          : (i32, i32) -> i32 pure;
kernel load_input   : ()         -> i32[N] effectful;
kernel load_input_b : ()         -> i32[N] effectful;
kernel save_output  : (i32[N])   -> ()     effectful;

a <-- load_input();
b <-- load_input_b();
for i : 0 .. N {
    c[i] <-- add(a[i], b[i]);
}
save_output(c);
```

The same algorithm runs under a single-worker schedule:

```
schedule for "../prog.algo.nuc" {
    workers = { host };
    place load_input   on host;
    place load_input_b on host;
    place save_output  on host;
    place add          on host;
}
```

…and under a two-worker schedule, where the compiler infers and
schedules the cross-worker transfers (each crossing data symbol carries
an explicit `transfer` directive — omitting one is a compile error, not
a silent default):

```
schedule for "../prog.algo.nuc" {
    workers = { host, w0 };
    place load_input   on host;
    place load_input_b on host;
    place save_output  on host;
    place add          on w0;
    transfer a : sync;
    transfer b : sync;
    transfer c : sync;
}
```

Both schedules drive the *same* algorithm file and, on the same input,
must produce the *same* bytes — on every capable backend. That
requirement is the whole test.

## Read more

- **[`paper/main.pdf`](paper/main.pdf)** — the full write-up: the
  design, the two sublanguages, the Petri-net IR and its compile-time
  soundness gate, the ten-backend target ladder, the validation
  methodology, and the results. The authoritative reference; start here
  for the *why*.
- **[`nuc-nucleus/examples/`](nuc-nucleus/examples/)** — 29 worked
  examples, from element-wise add to a multi-layer CNN inference, a
  multi-microcontroller hearing-aid pipeline, a DMA-async + PIO-sync
  transport demo, and a family of binned-combine reductions. Each is
  `prog.algo.nuc` + one or more `schedules/*.sched.nuc` + a `kernels.rs`
  + an independent reference implementation + an `input.bin` + an
  expected `reference.bin`.
  <!-- check-readme-counts: examples=29 (filesystem-truth gate; bump when adding/removing an examples/NN-* dir) -->
- **[`nucleus/`](nucleus/)** — the Rust workspace: `nucleus-compiler/`
  (parser, IR, and passes), `backends/` (the ten backends across three
  tiers), `driver/` (the `nucleus` CLI), and `e2e/` (the differential
  matrix harness).
- **[`docs/`](docs/)** — [`tutorial.md`](docs/tutorial.md) (write and
  run a program from scratch), [`cli-reference.md`](docs/cli-reference.md)
  (build flags, exit codes, `--emit-pn`), the grammar references
  ([algorithm](docs/grammar-algo.md), [schedule](docs/grammar-sched.md)),
  [`numeric-determinism.md`](docs/numeric-determinism.md), and
  [`stability.md`](docs/stability.md).

## Build and run

The repo provides a Nix dev shell and a `justfile`:

```
nix develop -c just build       # cargo build --workspace
nix develop -c just test        # cargo test --workspace
nix develop -c just e2e         # full cross-backend differential matrix
nix develop -c just ci          # the full gate
```

`just e2e` is the load-bearing test: it builds and runs every
(example × schedule × backend) cell and diffs the output against the
committed reference. Bit-identical, or it fails.

The paper is built from its own pinned environment:

```
cd paper && nix develop -c just build   # lualatex + biber -> paper/main.pdf
```
