# Nucleus v2 — Product Requirements Document

Status: draft.
Author: MPED.

## 1. Goal

Build a working pre-compiler that takes **two** annotated source files in
the Nuc language — an *algorithm* file (Rust kernels, dataflow,
iteration) and a *schedule* file (workers, mapping, blocking, IO
semantics) — and emits split, statically-scheduled, parallel code for a
range of backends spanning commodity CPU, HPC clusters, and embedded
targets.

The intended target ladder, from cheap to expensive (§7):

- **Tier 1 — CPU-simulatable.** OpenMP, pthreads, multi-process over
  TCP/UDS, varying IO modes. Used as the test harness that proves the
  model.
- **Tier 2 — HPC clusters.** MPI with blocking and non-blocking
  primitives.
- **Tier 3 — Embedded.** no_std Rust over custom DMA + interrupt
  controllers, per-MCU shims.

CPU-tier is what we build first because it is where the differential
test is cheap. MPI and embedded are what makes the project *worth*
building — the algorithm/schedule split delivers most value when the
hardware churns under you and the algorithm doesn't.

### Motivation

Three claims, in decreasing order of how much load they carry:

1. **IO has been a second-class citizen.** Compute scheduling has
   first-class language support (Halide, TVM, Tiramisu, Exo). IO
   semantics — sync vs async, buffered vs unbuffered, event vs poll,
   shared-memory vs DMA vs MPI — are written by hand, per platform,
   per program. Nucleus makes IO semantics a first-class schedule
   directive, swappable per-target without touching the algorithm.
2. **Hardware changes faster than algorithms do.** An algorithm
   written for x86 + threads should not have to be rewritten for an
   MPI cluster, an ARM SoC, or a custom NPU. Today it usually is.
3. **Decomposition is the bottleneck.** Algorithm code is comparatively
   cheap to write. Deciding how to decompose it across heterogeneous
   workers with explicit IO is the expensive, error-prone part. Make
   *that* the cheap thing to iterate on. Make it cheap to port to
   new wildly different platforms.

The algorithm/schedule split is the central design commitment: one
algorithm composes with many schedules, each schedule composes with
many backends. Changing the schedule must not require touching the
algorithm; changing the backend must not require touching the schedule.

### Success criterion

For tier 1 (CPU): every driving example in §9 compiles under every
listed schedule and every backend, runs on a developer laptop, and
produces bit-identical output across the full (schedule × backend)
matrix for the same input.

For tier 2/3: every driving example compiles for the target. Runtime
validation is best-effort (via simulator, localhost MPI, or
hardware-in-the-loop where available). The CPU tier remains the
falsification rig for the model itself; tier 2/3 backends are validated
by build and by running on the target where feasible.

## 2. Why v2 (delta vs the 2013 thesis)

| 2013 v1 old                                   | v2 new                                                  |
| --------------------------------------------- | ------------------------------------------------------- |
| Hive ISP firmware, single dead target         | Target ladder: CPU (tier 1), MPI (tier 2), embedded (tier 3). |
| One backend (`libcamhal_rhb`)                 | Many backends. Genericity must be falsifiable.          |
| C++11 with hand-rolled visitor framework      | Rust. Sum types, pattern matching, no Cheshire-cat.     |
| Kernels as text fragments via `${1}` substitution | Kernels as real Rust functions with shape-typed signatures. |
| Driving example didn't run end-to-end         | Test suite is the spec. No example = no feature.        |
| Aliasing punted (split-binding unsound)       | Single-assignment arrays. No aliasing by construction.  |
| Where-clauses could side-effect silently      | `where pure` mandatory; `where !effectful` opt-in.      |
| Data transfers inferred but not scheduled     | Transfers scheduled at compile time. Deadlock-free.     |
| IO as API-level afterthought                  | IO semantics first-class in schedule (sync/async/buf/notify). |
| Decomposition tangled into the source         | Algorithm and schedule are separate files.              |
| Ad-hoc deadlock / buffer-sufficiency analyses | One Petri-net IR; analyses fall out as standard properties. |

The point of v2 is **not** to redo the thesis. It is to build the thing the
thesis only sketched, on hardware everyone has, with the genericity claim
testable mechanically.

## 3. Non-goals

Listed explicitly so they stop creeping in.

- **No GPU / NPU / FPGA backend in v2 proper.** Tier 3 (embedded) is
  in scope; general-purpose GPU/NPU programming is not. The model
  doesn't preclude them — a future tier-4 could exist — but v2 ships
  without.
- **No automatic parallelisation.** The programmer annotates
  decomposition. Compiler does mapping and scheduling, not discovery.
  Note: the compiler does *infer transfers and halo regions* from
  declared access patterns. That's automatic transfer synthesis, not
  automatic parallelism extraction. Stated explicitly because the line
  is thin.
- **No polyhedral analysis.** Single-assignment + explicit views is
  enough for the planned examples. If a real case needs isl, that case
  is out of scope.
- **No dynamic scheduling, work stealing, or load balancing.** Static
  scheduling only. If a problem needs dynamic, it doesn't go in Nuc
  v2. (Static is also the only thing that makes sense for embedded
  real-time targets, so this restriction is paying double rent.)
- **No IDE, no debugger, no profiler.** A working `nucleus` CLI is
  enough.
- **No source-level language compatibility with 2013 Nuc.** Clean break.
- **No package manager, no general module system.** Exactly two source
  files per build: one algorithm, one schedule. No imports, no
  includes. Kernel function bodies live in adjacent Rust source files,
  built by the host toolchain (see §6.2.2).
- **No default schedule.** A program without an explicit schedule does
  not compile. Implicit defaults hide too much.
- **No promise of "platform-agnostic" for algorithms that don't
  decompose statically.** Data-dependent indexing, sparse access,
  recursive structure: out. The portability claim is "portable for
  algorithms whose decomposition fits the model," not "portable for
  all algorithms."

## 4. Users

Three user archetypes, all real:

1. **The HPC engineer** with an algorithm that needs to scale from a
   laptop (threads) to a cluster (MPI) without being rewritten.
   Schedule swap from `pthreads-async` to `mpi-nonblocking` is the
   workflow.
2. **The embedded firmware engineer** porting an algorithm between
   different MCU families (different DMA controllers, different
   memory layouts, possibly different cores). Algorithm stays put;
   schedule + target shim changes.
3. **The systems researcher** asking "is my algorithm bound by sync,
   by serialisation, or by transport?" and answering it by swapping
   schedule files across a range of CPU backends. This is the cheap
   tier-1 use case, and it's also how we validate the model for the
   other two.

All three share the same demand: **algorithm stays put; the schedule
and backend are the knobs**. The compiler does the rewriting.

## 5. Architecture

```
  prog.algo.nuc          prog.sched.nuc
       |                      |
       v                      v
  [ parse algo ]          [ parse sched ]
       |                      |
       v                      v
  [ desugar / type check ]    |
       |                      |
       v                      |
  [ algorithm IR ]            |
   (kernels + dataflow,       |
    no decomposition)         |
       |                      |
       +------ link ----------+      <-- bind kernels to workers,
       |                                 attach loop transforms,
       v                                 attach IO semantics
  [ build ACFG ]      -- application control-flow graph
       |                (nodes: op, repeat, sync, xfer)
       v
  [ transforms ]      -- vectorise, block, pipeline, reuse
       |
       v
  [ infer transfers ] -- producer/consumer edges across workers
       |
       v
  [ build global Petri net ]   -- transitions: firings/xfers/syncs
       |                          places:      data slots/channels/barriers
       v
  [ analyse net ]              -- boundedness, deadlock, liveness;
       |                          failures here are compile errors
       v
  [ project per worker ]       -- worker_id -> ordered EventList
       |
       v
  [ presentation layer ]       -- pick one of §7; consumes EventList
       |
       v
  out/{worker0.rs, worker1.rs, Cargo.toml, run.sh}
```

Two boundaries matter, both must hold for v2 to be honest:

1. **Algorithm / schedule boundary.** The algorithm file must compile to
   an IR that contains no worker bindings, no blocking, no buffer sizes,
   no IO semantics. A schedule must compile *against* an algorithm IR —
   if you can rewrite either file independently and the other still
   works, the boundary is real.
2. **Middle-end / presentation-layer boundary.** Same as v1: anything
   backend-specific above this line is a bug.

Anything that leaks across either boundary is a design defect, not a
nuisance.

## 6. Nuc language v2

Nuc v2 is two small sublanguages with one shared name table.

- **Algorithm sublanguage** (`*.algo.nuc`) — *what* to compute.
- **Schedule sublanguage** (`*.sched.nuc`) — *how* to lay it out in space
  and time, and what IO semantics to use. When and where.

A schedule references kernels and loops in the algorithm by name. A
schedule without an algorithm is meaningless; an algorithm without a
schedule does not compile.

### 6.1 Why two sublanguages

The 2013 thesis tangled `isp0::` annotations, `vectorize=N`,
`block_integral=N`, and where-clauses into the algorithm itself.
Trying a different decomposition meant editing the same file the
algorithm lived in — and exploration is precisely what the thesis listed
as a motivation in §4.1.2. v2 fixes this by stealing the
algorithm/schedule split from Halide and extending it to multi-process
and IO-semantic concerns.

Concretely:

- One algorithm composes with many schedules.
- Many schedules compose with many backends.
- Differential testing now spans (algorithm × schedule × backend), not
  just backend.

### 6.2 Algorithm sublanguage

Goal: just enough surface to express dataflow and iteration. No
decomposition. No mapping. No transfer semantics.

#### 6.2.1 Storage and data

- Arrays with explicit shape and element type. No pointers.
- Single-assignment within a scope. Mutation only via a fresh binding.
- Scalars are degenerate arrays.
- Views (`a[lo..hi]`) are read-only slices, not aliases for writing.

#### 6.2.2 Kernels

A kernel is a **real Rust function** with a shape-typed signature
declared in `.algo.nuc` and a body in an adjacent Rust file. Nucleus
generates wrapper code that calls the kernel; it does **not**
substitute text into kernel bodies.

In `prog.algo.nuc`:

```
kernel blur3 : (f32, f32, f32, f32, f32, f32, f32, f32, f32) -> f32  pure;
kernel load_image : () -> f32[H][W]  effectful;
kernel save_image : (f32[H][W]) -> ()  effectful;
```

In `kernels.rs` (sibling file, hand-written):

```rust
pub fn blur3(
    a: f32, b: f32, c: f32,
    d: f32, e: f32, f: f32,
    g: f32, h: f32, i: f32,
) -> f32 {
    (a + b + c + d + e + f + g + h + i) * (1.0 / 9.0)
}

pub fn load_image() -> Box<[[f32; W]; H]> { /* ... */ }
pub fn save_image(img: &[[f32; W]; H]) { /* ... */ }
```

- `pure` — function has no side effects; reorderable, deduplicable,
  eliminable.
- `effectful` — function has side effects; ordering preserved within
  its basic block, never duplicated.
- No third option. The 2013 semantic crack is closed.

What this buys us, in order of how load-bearing each is:

1. **Rust's type checker validates kernel bodies.** Zero typechecker
   code in Nucleus.
2. **Borrow checker applies.** Free safety guarantee on internals.
3. **No interpolation hygiene problems.** No `${}`, no name capture,
   no scoping pitfalls.
4. **Kernels are unit-testable as plain Rust** without Nucleus
   running. `cargo test` works.
5. **Rust-analyzer / IDE tooling works on kernels.**
6. **Errors point at the user's Rust source**, not at generated code.
7. **`#[inline]`, `#[cold]`, SIMD intrinsics, `unsafe`, `no_std`** —
   all already supported by Rust; no Nuc syntax needed.

The kernel declaration in `.algo.nuc` is a *contract* the Rust
function must satisfy: signature shape, purity. A `cargo build` step
in Nucleus's pipeline verifies the contract before scheduling.

Kernels declare *what*. They do **not** declare *where they run* —
that's the schedule's job.

#### 6.2.3 Dataflow and iteration

```
img_in <-- load_image();

for y : 1 .. H-1 {
for x : 1 .. W-1 {
    img_out[y][x] <-- blur3(
        img_in[y-1][x-1], img_in[y-1][x], img_in[y-1][x+1],
        img_in[y  ][x-1], img_in[y  ][x], img_in[y  ][x+1],
        img_in[y+1][x-1], img_in[y+1][x], img_in[y+1][x+1]
    );
}}

save(img_out);
```

Loops have plain bounds and an iteration variable. Loop variables are
the names the schedule will reference.

**Name resolution.** Iteration variables and data variables share one
namespace. Iteration variables shadow at their loop and go out of
scope at the loop's end. A name `y` inside a `for y : ...` body always
refers to the iteration variable; outside, it refers to whatever
`data y : ...` declared (or is undefined). No `@`-style prefix; the
compiler disambiguates by scope.

#### 6.2.4 What is intentionally not in the algorithm

- No worker names. No `w0::`, no `host::`.
- No `block=`, `vectorize=`, `unroll=`, `pipeline=`, `reuse`.
- No `transfer=`, `buffer=`, `notify=`.
- No conditionals across worker boundaries.
- No recursion, no closures, no higher-order ops, no generics.
- No exceptions in Nuc surface; errors at codegen time only.

If you find yourself wanting any of the above in the algorithm, that
desire belongs in the schedule.

#### 6.2.5 Recorded decision: in-array prefix scan is a kernel-level idiom for v2

*Status: accepted (v2 / M3). Origin: TASK-0039 finding, TASK-0179.*

A textbook in-array carried prefix scan
(`out[i] <-- scan_add(out[i-1], in[i])`) is **not directly
expressible** in the v2 algorithm sublanguage, and it deliberately
stays that way for v2. Three sublanguage properties combine to make
the carried form inexpressible (probed end-to-end in TASK-0039, not
assumed):

1. The shifted carry `out[i-1]` underflows `usize` at `i = 0`, and
   there is **no conditional** to guard the boundary (§6.2.4).
2. Single-assignment is keyed by data **symbol name**, so a
   base-case + loop split (`out[0] <-- ...; for i : 1 .. N { ... }`
   on the same `out`) is a double-assignment.
3. Loop bounds must fold to a compile-time `i64` constant, so a
   triangular / iter-var-dependent reformulation (`for j : 0 .. i`)
   is rejected — now as a clean `BuildAcfgError::NonConstLoopBound`
   diagnostic rather than a panic (TASK-0179).

**Decision.** In-array prefix scan remains a **kernel-level idiom**
for v2. The boundary/carry logic lives in the hand-written Rust
kernel, over the rectangular reduction-accumulator pattern, exactly
as the algorithm-vs-kernel split in §6.2.2 intends (kernels are
arbitrary Rust; the algorithm sublanguage stays minimal and
statically analyzable). `examples/04-prefix-sum` is the canonical
realisation: a 3-pass rectangular reduction-accumulator (block
totals → exclusive block offsets → within-block scan + offset) with
the masking/boundary predicate in the Rust kernels. It is
differentially green (byte-identical to an independent std-only
reference oracle) on both `pthreads-sync` and `mp-tcp-bufsync`.

**Rationale.** The kernel boundary is the *designed* escape hatch
(§6.2.2): pushing data-dependent boundary logic there keeps the
algorithm sublanguage free of conditionals (§6.2.4), single-valued
per symbol (§6.2.3), and with const-only loop extents — the
properties the Petri-net IR and the cross-backend differential rely
on. Adding a scan/segmented-scan builtin, a clamp/saturating-index
intrinsic, or a guarded-first-iteration form would each enlarge the
sublanguage and its analysis surface for one access pattern that the
kernel split already handles cleanly.

**Explicitly future language work (not v2 / not M3).** A
first-class boundary-free scan — a `scan`/`segmented-scan` builtin,
a clamp/saturating-index intrinsic, a guarded-first-iteration form,
or iter-var-dependent (triangular) loop bounds — is deferred. It is
not required to prove the model (the tier-1 differential matrix is
green without it) and is tracked as a language-evolution item, not a
v2 deliverable.

**Consequence for TASK-0179 AC#3** ("if supported, an example
expresses prefix scan WITHOUT pushing the boundary into a kernel"):
**not applicable under this decision.** v2 does *not* support a
boundary-free form, so no such example is a v2 deliverable, and
fabricating one would misrepresent the language. The canonical
accepted-limitation pattern is `examples/04-prefix-sum` (kernel-level
idiom). AC#3 becomes part of the deferred future-language-work item
above, not a v2 obligation.

### 6.3 Schedule sublanguage

A schedule binds an algorithm's kernels and loops to a worker topology
and chooses IO semantics. Schedules are short — a typical schedule for a
non-trivial example fits on a screen.

```
schedule for "stencil.algo.nuc" {
    workers = { host, w0, w1, w2, w3 };

    place load_image  on host;
    place save        on host;
    place blur3       on { w0, w1, w2, w3 };   // distributed

    loop y : block=64;
    loop x : vectorize=8, reuse;

    transfer img_in   : async, buffer=2, notify=event;
    transfer img_out  : sync;
}
```

#### 6.3.1 Workers (space)

Two forms. Use the simple form for homogeneous worker pools (typical
on commodity CPU and HPC clusters). Use the typed form for
heterogeneous targets (typical on embedded SoCs and accelerator
hybrids).

**Simple form:**

```
workers = { host, w0, w1, w2, w3 };
```

Names are user-chosen. Cardinality fixed at compile time. Dynamic
topology is out of scope (§3). Equivalent to the typed form with a
single default worker class.

**Typed form** (for tier 3 / heterogeneous targets):

```
worker_class control_core {
    simd     = none;
    memory   = shared;
};

worker_class compute_core {
    simd     = neon128;
    memory   = tightly_coupled[64KB] + shared;
};

memory_region shared_sram   { size=512KB; accessible_by={control_core, compute_core}; };
memory_region tcm_per_core  { size=64KB;  accessible_by={compute_core}; per_worker=true; };

workers = {
    host       : control_core,
    w0..w3     : compute_core,
};
```

Worker classes declare SIMD width, available memories, and (eventually)
other capabilities that a backend cares about. Memory regions declare
named address spaces, their sizes, who can access them, and whether
each worker gets a private instance.

`place_data D in MEMORY_REGION` directives bind algorithm data symbols
to memory regions:

```
place_data img_in  in shared_sram;
place_data tile    in tcm_per_core;     // one private copy per compute_core worker
```

For tier-1 CPU backends the typed form collapses to the simple form
(one class, one memory region). It's there so the same Nuc surface
extends to tier 3 without a second sublanguage.

#### 6.3.2 Placement

```
place KERNEL on WORKER;
place KERNEL on { W1, W2, ... };   // distributed; compiler partitions iter space
```

Every kernel referenced in the algorithm must have exactly one `place`.
An unplaced kernel is a compile error.

For distributed placement, the compiler partitions the loop iteration
space across the named workers. Partitioning policy is a schedule
option (next section).

#### 6.3.3 Loop transformations (time)

```
loop VAR : option [ , option ]* ;
```

| Option          | Meaning                                                    |
| --------------- | ---------------------------------------------------------- |
| `block=N`       | Tile iteration into chunks of N; transfers happen per tile |
| `vectorize=M`   | Unroll inner body M-way; expects SIMD-friendly ops         |
| `reuse`         | Identify and reuse loop-carried slices (the 2013 gap)      |
| `pipeline=D`    | Software-pipeline with depth D (replaces double-buffering) |
| `unroll=N`      | Plain unrolling, no vector grouping                        |
| `partition=...` | Policy for distributed placement (e.g. `rows`, `blocks2d`) |

Loop options are orthogonal where possible. Bad combinations
(`vectorize` on a divergent loop, `partition=rows` on a 1D iteration,
etc.) are rejected at compile time, not at runtime.

#### 6.3.4 Transfer / IO semantics

```
transfer DATA : option [ , option ]* ;
```

| Annotation       | Effect on inferred transfers                         |
| ---------------- | ---------------------------------------------------- |
| `sync`           | Producer blocks until consumer has received          |
| `async`          | Producer returns immediately; consumer waits at use  |
| `buffer=N`       | Permit N in-flight transfers (>1 enables pipelining) |
| `notify=event`   | Backend uses event/condvar/epoll notification        |
| `notify=poll`    | Backend uses busy/yield polling                      |

The compiler resolves these against the chosen backend's capability
matrix. Asking for `async` on a backend that doesn't support it is a
hard error, not a silent fallback.

A `transfer` directive that would cross workers (producer and
consumer on different workers) **must** be present. Omitting it is a
compile error citing the offending data symbol. This avoids the
silent-default trap where one schedule means different things on
different backends. Transfers entirely intra-worker need no directive
and emit no event.

#### 6.3.5 Runtime assertions

The schedule can attach **checkable** properties — observations the
generated code measures and verifies at runtime. These are *not*
constraints the compiler optimises against (v2 has no cost model);
they are *assertions* whose violation tells the user the schedule
doesn't meet the requirement and must be revised.

```
check loop VAR : assertion [ , assertion ]* ;
```

Available assertions:

| Assertion         | What is measured                                         |
| ----------------- | -------------------------------------------------------- |
| `latency_max = T` | Wall-clock duration of one iteration of the loop ≤ T.    |
| `on_violation = panic | log | count` | Action when an assertion fails. Default: `panic`. |

Future variants (`jitter_max`, `throughput_min`, per-transfer
`buffer_max`) are not in v2 but slot into the same `check` keyword
without grammar changes.

Time units are wall-clock seconds with the suffix `ns`/`us`/`ms`/`s`.
The implementation uses `std::time::Instant` on tier 1, `MPI_Wtime` on
tier 2, and a backend-specified monotonic clock on tier 3.

Example:

```
loop  frame : pipeline=3;
check loop frame : latency_max = 10ms;
```

The compiler injects measurement code at the loop iteration boundary
and emits a comparison against the threshold. On `panic` (the
default), violation aborts the program — appropriate for tier-1
testing where loud failure is wanted. On `log`, the violation is
reported to the backend's logging mechanism and execution continues —
appropriate for tier-3 production where panicking on an MCU bricks
the device. On `count`, violations are tallied and reported at
program exit — appropriate for batch validation runs.

**This is checkable, not prescriptive.** v2 does not adjust the
schedule to *meet* `latency_max`. It only checks whether the schedule
the user wrote *does* meet it. If the assertion fires, the user
edits the schedule. A future v3 with a cost model could add
`solve_latency_max` as a prescriptive sibling — same syntax shape,
different compiler behaviour. v2 ships the observation; the optimiser
comes later or not at all.

#### 6.3.6 What is intentionally not in the schedule

- No kernel bodies. No data declarations. No dataflow edges.
- No control flow.
- If a change to the schedule requires editing the algorithm, the split
  is broken and the compiler is wrong.

## 7. Presentation layers (target ladder)

Each backend is one Rust crate. All produce a Cargo workspace plus a
build/run harness appropriate to the tier. The capability matrix for
each backend is a sibling `capabilities.toml` — committable text data,
not Rust code — so it can be reviewed and diffed independently.

### 7.1 Tier 1 — CPU-simulatable (M1–M6)

The cheap test harness. Output is hosted Rust, links against std and
at most one well-known crate.

| Backend id      | Workers map to | Transport       | Notify           | Buffered  | Sync/Async |
| --------------- | -------------- | --------------- | ---------------- | --------- | ---------- |
| `openmp-rs`     | rayon threads  | shared memory   | barrier          | n/a       | sync       |
| `pthreads-sync` | std threads    | shared memory   | condvar          | n/a       | sync       |
| `pthreads-async`| std threads    | shared memory   | condvar          | yes (ring)| async      |
| `mp-tcp-poll`   | OS processes   | TCP loopback    | nonblocking poll | no        | sync       |
| `mp-tcp-event`  | OS processes   | TCP loopback    | mio/epoll        | yes       | async      |
| `mp-tcp-bufsync`| OS processes   | TCP loopback    | blocking         | yes       | sync       |
| `mp-uds-event`  | OS processes   | Unix domain     | mio/epoll        | yes       | async      |

### 7.2 Tier 2 — HPC cluster (M7+)

The same algorithm, scaled. Output is hosted Rust + an MPI binding
(e.g. `rsmpi`). Generates SPMD-style binaries: one Rust executable that
dispatches on `MPI_Comm_rank`.

| Backend id        | Workers map to | Transport | Notify          | Buffered    | Sync/Async |
| ----------------- | -------------- | --------- | --------------- | ----------- | ---------- |
| `mpi-blocking`    | MPI ranks      | MPI       | implicit (recv) | by MPI impl | sync       |
| `mpi-nonblocking` | MPI ranks      | MPI       | `MPI_Wait`      | yes (req)   | async      |

Future work, deliberately deferred: collective recognition (emit
`MPI_Allreduce`/`MPI_Scatter` when the scheduler recognises the
pattern). v2 emits point-to-point only; that's correct, just suboptimal.

### 7.3 Tier 3 — Embedded (M8+)

`no_std` Rust over per-MCU DMA + IRQ shims. Each MCU family is a
**separate shim crate** providing a trait implementation for DMA
descriptors, IRQ vector binding, and memory region addresses. The
generic `embedded-pattern` backend emits target-agnostic event-list
lowering; the shim provides target-specific hardware abstraction.

| Backend id                  | Shim provides                  | Notify          | Buffered      | Sync/Async |
| --------------------------- | ------------------------------ | --------------- | ------------- | ---------- |
| `embedded-cortexm-dma-irq`  | STM32H7, NXP RT11xx, ...       | IRQ + completion| yes (descriptor ring) | async |
| `embedded-riscv-dma-irq`    | Espressif, SiFive, ...         | IRQ + completion| yes           | async      |

Tier 3 requires the *typed* worker form (§6.3.1). Per-MCU shims are
shipped as separate crates; the v2 deliverable includes one reference
shim (likely Cortex-M7 / STM32H7) as proof-of-concept. Other shims are
out-of-tree contributions.

### 7.4 Capability matrix and backend orthogonality

Each backend declares a capability matrix as a sibling text file:

```toml
# capabilities.toml for mp-tcp-event
transport       = "tcp"
notify          = ["event"]
supports_async  = true
supports_buffer = true
max_buffer      = 1024
worker_classes  = ["default"]
memory_regions  = ["heap"]
```

Backend choice is **orthogonal to schedule**: a schedule must compile
against any backend whose capability matrix is a superset of the
schedule's demands. If a schedule asks for `transfer=async,
notify=event` and the backend is `pthreads-sync`, that pair is
rejected at compile time, not papered over.

Backends across tiers share the same EventList contract (§8.3). What
differs is what code each backend emits for the same events — a
`Push` is a memcpy under `openmp-rs`, a `socket.write` under
`mp-tcp-event`, an `MPI_Isend` under `mpi-nonblocking`, and a DMA
descriptor enqueue under `embedded-cortexm-dma-irq`.

Minimum bar to ship a backend: it compiles every (§9 example × every
required schedule from the example's README). Tier 1 backends must
also produce reference-matching output. Tier 2/3 backends must compile
and, where simulators or hardware-in-the-loop exist, produce
reference-matching output there.

## 8. Static scheduling: the Petri-net IR

This is the central technical contribution of v2.

### 8.1 What the scheduler produces

The scheduler is a pure function:

```
schedule : (AlgoIR, SchedIR) -> ( GlobalNet, { WorkerId -> EventList } )
```

`GlobalNet` is a deterministic, bounded, place/transition Petri net that
captures the entire program's firing pattern. The per-worker
`EventList`s are projections of `GlobalNet` — the totally-ordered
actions each worker performs. The presentation layer (§7) consumes
only the `EventList`s; the `GlobalNet` exists for analysis and
inspection.

### 8.2 Mapping

| Petri-net concept             | Nucleus meaning                                |
| ----------------------------- | ---------------------------------------------- |
| Transition                    | Kernel firing, transfer, or sync               |
| Place                         | Data slot, channel, or sync barrier            |
| Token                         | Data presence / control credit                 |
| Place capacity                | `buffer=N` from the schedule                   |
| Initial marking on a place    | Pipeline depth / latency-hiding head-start     |
| Reachability of final marking | "This schedule terminates from initial state"  |
| Boundedness                   | "Every place stays within its declared capacity" |
| Liveness                      | "No transition is forever-disabled"            |
| Deadlock                      | A reachable marking where no transition fires  |

What this gets us, mechanically, that hand-rolled analyses didn't:

- **Deadlock check** = reachability of a deadlocked marking. Decidable
  for v2's restricted nets (acyclic firing order, bounded places).
- **Buffer-sufficiency check** = boundedness check against declared
  `buffer=N`. Reject at compile time with a message naming the
  offending place and the marking that overflows it.
- **Schedule equivalence** = net isomorphism. Two schedules are "the
  same" iff their nets are isomorphic up to worker renaming. Useful
  for caching, regression testing, and reasoning about refactors.
- **Reuse, double-buffering, software-pipelining** all express the same
  way: tokens placed in pipeline-register places by the initial
  marking. No special case per transform — the schedule keywords
  lower into different initial markings on the same kind of place.

### 8.3 Event types (presentation-layer contract)

The per-worker `EventList` uses six event types. Each event is the
projection of a transition firing onto the worker that owns it:

```rust
enum Event {
    // TODO: We must elaborate these more
    Fire   { kernel: KernelId, tile: IterTile },
    Alloc  { data: DataId, tile: IterTile, region: Region },
    Push   { dst: WorkerId, data: DataId, tile: IterTile, seq: SeqTag },
    Wait   { src: WorkerId, data: DataId, tile: IterTile, seq: SeqTag },
    Sync   { participants: Set<WorkerId>, kind: SyncKind, sync: SyncTag },
    Free   { data: DataId, tile: IterTile },
}
```

`seq` is a compile-time sequence number on each matched `Push`/`Wait`
pair — the receive side knows which send goes with which receive
without runtime matching machinery. `sync` is the analogous stable
identity on `Sync`: every participant of one barrier carries the same
`SyncTag`, so disjoint per-worker `EventList`s agree on barrier
identity without a global walk — this is what lets a backend lower a
partial / non-uniform barrier (participant sets that differ between
barriers) correctly rather than recovering identity from a per-worker
pre-order index that only coincides for uniform barriers.

#### IterTile — iteration-space bounds of a datum

`IterTile` is a rectangular slice in iteration space. For a 2D loop
nest with iteration variables `(y, x)`, an `IterTile` is one half-open
interval per variable, in iteration-nest order:

```rust
struct IterTile {
    bounds: Vec<(IterVar, Range)>,   // e.g. [(y, 32..64), (x, 0..256)]
}
```

For non-iterated firings (top-level dataflow lines like
`img_in <-- load_image()`) the tile is empty. For a `Fire`, the tile
names the iteration coordinates this firing covers. For
`Alloc`/`Push`/`Wait`/`Free`, the tile names the slice of `data`
involved — derived from the kernel's declared access pattern projected
onto the firing's tile.

Same type used everywhere; backends interpret it as a
multidimensional extent on the appropriate array.

#### Region — opaque memory region handle

`Region` is a backend-interpreted handle naming where local backing
storage lives. Nucleus does not know its representation; the backend
decides:

- `pthreads-sync`: a heap-allocated `Box<T>`.
- `mp-tcp-event`: a slab from a pre-allocated ring buffer.
- `mpi-nonblocking`: a registered MPI buffer for in-place receive.
- `embedded-cortexm-dma-irq`: an address inside TCM, shared SRAM, or
  external SDRAM, plus a DMA descriptor slot.

Schedule directives (`place_data D in MEMORY_REGION` from §6.3.1)
choose which `Region` an `Alloc` resolves to. The compiler treats
`Region` as an opaque tag; the backend's `capabilities.toml` declares
which regions it supports and how they map to physical memory.

`Push`/`Wait`/`Free` don't carry `Region` directly — the source and
destination regions were fixed by the preceding `Alloc` events on the
respective workers; the backend looks them up.

#### SyncKind — control-only synchronisation

`Push`/`Wait` already carry data coherency. `Sync` exists only for
control-flow joins where no data crosses but progress must wait for
all participants:

```rust
enum SyncKind {
    Barrier,    // all listed participants arrive; then all proceed
}
```

v2 ships exactly one variant. Most cross-worker synchronisation rides
on data transfers, which `Push`/`Wait` handle. The only remaining
need is "all workers complete phase A before any starts phase B" —
a barrier. Tier-1 backends lower this to `std::sync::Barrier` or a
rayon scope; tier-2 to `MPI_Barrier`; tier-3 to a coordinated IRQ
rendezvous.

Other variants (rendezvous, quorum) are not added unless a driving
example needs them. If one does, that's the evidence the variant
earns its slot.

### 8.4 Restrictions that keep the net tractable

v2 nets are deliberately a small subclass of general Petri nets:

- **Statically determined firing order.** Order is decided at compile
  time, not by token availability at run time. No free-choice, no
  confusion, no conflicts.
- **Bounded by construction.** Every place has a stated capacity. A
  reachable marking that exceeds capacity is a compile error.
- **Acyclic global event DAG.** Per-worker order plus `Push`→`Wait`
  arcs form a DAG. Cycle = deadlock = compile error pointing at the
  cycle.
- **No coloured, stochastic, probabilistic, or hierarchical extensions.**
  Plain place/transition nets with capacities and initial markings.
  The Petri-net library v2 needs is small (~500 lines), not an academic
  tool.

Petri nets allow for dynamic behavior but we will stay static for now, but retain Petri nets for that future extension.

### 8.5 Inspection

The CLI exposes the net to the user:

```
nucleus build --algo prog.algo.nuc \
              --sched schedules/distributed.sched.nuc \
              --backend mp-tcp-event \
              --emit-pn out/schedule.dot
```

Produces a Graphviz file rendering the global net: places, transitions,
arcs, initial marking, place capacities, and the per-worker projection
shown by colour. Same model as the internal IR — no separate
visualisation codepath. This is the answer to "why is my schedule
deadlocking?" or "what does `pipeline=2` actually do here?"

### 8.6 What's hard, what could fail

- **Firing-order linearisation is NP-hard in general.** v2 picks a
  deterministic greedy order (source order + dataflow constraints) and
  validates that order against the net properties above. Not optimal.
  Reproducible. Inspectable.
- **TCP kernel-level backpressure.** Application-level place capacity
  must be ≤ kernel socket buffer / typical message size. v2 computes
  the required socket buffer size from the net and sets it via
  `SO_SNDBUF`/`SO_RCVBUF` in the generated `run.sh`. No application-
  level credit protocol.
- **Initial-marking generation for `pipeline=D` and `reuse`** requires
  static stride analysis. Affine indices only (already excluded in §3
  for the general case; the Petri-net IR does not relax this).

## 9. Driving examples

The test suite is the spec. Every example ships **one** algorithm and
**several** schedules. Every (algorithm, schedule, backend) cell of the
matrix must compile and produce the reference output.

| # | Algorithm                | Stresses                                  |
| - | ------------------------ | ----------------------------------------- |
| 1 | Element-wise add         | Smoke test. One worker, no transfers.     |
| 2 | Element-wise add (split) | Trivial space decomposition, one xfer.    |
| 3 | Reduction (sum/min/max)  | Tree reduction, sync barrier semantics.   |
| 4 | Prefix sum (scan)        | Two-pass dependency, ordering.            |
| 5 | 3x3 stencil (blur)       | Reuse, halo regions, blocking.            |
| 6 | 5x5 separable filter     | Two-pass stencil, intermediate buffer.    |
| 7 | Matrix multiply (blocked)| 2D blocking, all-to-all communication.    |
| 8 | Histogram                | Reduction with shared output array.       |
| 9 | Producer/consumer pipe   | Pipelining, buffer depth, async transfer. |
| 10| Wavefront (diagonal LU)  | Diagonal dependency, ordering, no SIMD.   |
| 11| Game of Life (multi-iter)| Multi-iteration stencil, double buffer.   |
| 12| Bitonic sort             | Static communication pattern, fits SDF.   |
| 13| Small CNN inference      | Layer-wise dataflow + batch parallelism + pipeline parallelism. Hits all three tiers. |
| 14| Hearing-aid pipeline     | Heterogeneous workers (RF/DSP/FE), bidirectional flow, multi-MCU embedded showcase. |

Example 13 is the load-bearing demonstration that the algorithm/schedule
split delivers across tiers. One algorithm (small CNN, forward pass
only — no training, no data-dependent branching), three schedules
(naive, batch-parallel, pipeline-parallel), validated on every tier-1
backend, on MPI for tier 2, and in Renode on tier 3. Training is
deliberately excluded — backward pass would need collective semantics
not in v2.

Example 14 stresses the parts no earlier example reaches: three
heterogeneous worker classes (analog front-end, DSP core, RF
controller), a fork-and-merge dataflow (mic and Bluetooth converge in
DSP, then split to speaker and outbound Bluetooth), and peripheral IO
wrapped in effectful kernels. It is the load-bearing demonstration of
the typed worker form (§6.3.1) and the multi-MCU Renode story
(M11). What it does *not* exercise: real-time deadlines and
continuous unbounded operation. Both are post-v2 concerns; the example
uses a finite frame loop and canned inputs to keep the differential
test honest.

Each example lives at:

```
examples/NN-name/
  prog.algo.nuc
  schedules/
    naive.sched.nuc           # single worker, no decomposition
    blocked.sched.nuc         # blocking only
    distributed.sched.nuc     # multi-worker, partitioned
    pipelined.sched.nuc       # async + buffered (where it applies)
    ...
  input.bin
  reference.bin
  README.md                    # what algorithm stresses, which schedules
                               # are required vs optional, and why
```

Not every schedule applies to every algorithm (a pipelined schedule on a
single-kernel reduction is nonsense). The README declares which
schedules are required. The full test matrix is
(required schedule, supporting backend) for each algorithm.

Examples added later must justify which orthogonal axis they cover —
either a new algorithmic dependency pattern *or* a new scheduling
challenge. No "kitchen sink" examples.

## 10. Validation

Cross-(schedule × backend) differential testing is the central
validation strategy. Granularity of the test depends on tier.

### 10.1 Tier 1 — bit-identical differential test

For each `(algorithm, schedule, tier-1 backend)` triple:

1. Compile algorithm against schedule, target backend.
2. Run with `input.bin`.
3. Compare output to `reference.bin`. **Must be bit-identical.**

This is the falsification rig for the model. Two independent axes
(schedule and backend) mean a green matrix falsifies two claims
simultaneously: that the algorithm/schedule split is real, and that
the middle-end / presentation-layer split is real. A red cell tells
you which boundary leaked.

`reference.bin` is generated once from a hand-written Rust
implementation kept under `examples/NN-name/reference/`. **Not** a
Nuc-compiled output (that would be "all backends wrong the same way").
Reference implementations are short, hand-audited, and committed to
the repo.

Bit-identity requires examples to be deterministic. v2 enforces this
by restricting numeric types to integers and deterministic floating
point where reductions don't reorder. Examples that reorder a
floating-point reduction are either restated as integer-equivalent or
excluded. See §12.

### 10.2 Tier 2 — compile-mandatory, run-best-effort

For each `(algorithm, schedule, tier-2 backend)` triple:

1. Compile must succeed.
2. Where a runtime is available (localhost MPI, slurm cluster in
   CI, OpenMPI in a container), run and compare. Bit-identity
   expected.
3. Where no runtime is available, compilation success + emitted-code
   inspection is the bar.

A tier-2 backend that compiles but never runs is not validating much.
The expectation is at least localhost MPI is in CI. Real-cluster runs
are out-of-band.

### 10.3 Tier 3 — compile-mandatory, run-in-Renode-where-supported

For each `(algorithm, schedule, tier-3 backend, target shim)` quad:

1. Compile must succeed against the shim.
2. Generated binary must pass `cargo check` under the target's
   `no_std` constraints.
3. **Default runtime check: Renode.** Renode is a multi-MCU
   instruction-set simulator supporting STM32 (multiple families),
   NRF52, ESP32 (xtensa + RISC-V), SiFive RISC-V cores, and others.
   It can co-simulate multiple MCUs in one session connected over
   UART/SPI/I2C/Ethernet. v2 treats Renode as the default tier-3
   runtime: a CI job spins up the appropriate `.resc` script, runs
   the generated binary, captures output via UART or memory dump,
   and diffs against `reference.bin`.
4. **Hardware-in-the-loop is a stretch goal**, not the default. A
   devboard on a CI runner (e.g. a Nucleo-H7 on a self-hosted
   runner) can validate timing-sensitive paths that Renode does not
   model accurately. v2 ships without HIL; contributions welcome.
5. Where Renode does not support the target MCU and no HIL exists,
   compile-only is the bar.

Tier 3's CI cost drops sharply once Renode is in the loop: no
hardware needed, no per-runner setup beyond the Renode container.
Multi-worker tier-3 schedules (workers on different MCUs connected
over SPI or Ethernet) become CI-testable in the same loop — Renode
can co-simulate them.

### 10.4 What's load-bearing

The tier-1 matrix is what falsifies the model. If tier 1 is green,
the model is sound — full stop. Tier 2 and 3 then become
*engineering* problems: emit the right code for the right target.
Tier 1's bit-identical claim is what makes the rest meaningful;
without it, all you have is "it builds."

CI runs the full tier-1 matrix on every commit. Tier 2/3 compile
checks run on every commit; tier 2/3 runtime checks run nightly
where harnesses exist. Failure of any tier-1 cell blocks merge.

## 11. Milestones

Sized for one person, not calendar-bounded. Tier 1 (M0–M6) is the
detailed plan. Tier 2 (M7+) and tier 3 (M8+) are scoped, not
fully designed — once tier 1 is green, the model is sound, and the
remaining tiers become engineering exercises whose detail comes later.

### Tier 1 — CPU-simulatable

- **M0 — Skeleton.** Rust workspace. `nucleus` binary that parses an
  algorithm file *and* a one-line schedule file for example 1, runs
  example 1 under `naive.sched.nuc` end-to-end producing reference
  output. The algorithm/schedule split is load-bearing from day one;
  no hello-world placeholder.
- **M1 — Single backend.** `pthreads-sync` backend. Examples 1–3 work
  end-to-end with `naive.sched.nuc`. ACFG, sync injection, transfer
  injection in place. Kernels declared in `.algo.nuc` and resolved
  against Rust function bodies in adjacent `kernels.rs`.
- **M2 — Static scheduling + Petri-net IR.** Transfer scheduling pass
  lowers (algo, sched) into a global Petri net; emits per-worker
  `EventList`s by projection. Boundedness and deadlock checks land as
  net properties. `nucleus --emit-pn` produces inspectable Graphviz.
  `blocked.sched.nuc` lands for examples 5, 7. Determinism test in CI.
- **M3 — Second backend.** `mp-tcp-bufsync`. Forces the capability
  matrix to be real (`capabilities.toml` lands as committable data).
  Examples 1–6 green on (naive ∪ blocked) × (pthreads-sync ∪
  mp-tcp-bufsync).
- **M4 — Async + buffering.** `pthreads-async` and `mp-tcp-event`.
  `pipelined.sched.nuc` lands. Examples 9, 11 work. `buffer=N`
  resolves end-to-end.
- **M5 — Distributed schedule + reuse.** `distributed.sched.nuc` lands.
  `reuse` loop option works. Examples 5–7 benefit measurably. Full
  tier-1 matrix still green.
- **M6 — Full tier-1 matrix.** Remaining tier-1 backends from §7.1.
  All 12 algorithms × required schedules × all tier-1 backends.

### Tier 2 — HPC cluster (scoped, not designed)

- **M7 — MPI blocking.** `mpi-blocking` backend. Examples 1–6 compile.
  Localhost MPI runtime in CI. Point-to-point only; no collectives.
- **M8 — MPI non-blocking.** `mpi-nonblocking` backend. Buffer/async
  schedules compile against MPI. Examples 9, 11 work over MPI.

### Tier 3 — Embedded (scoped, not designed)

- **M9 — Embedded skeleton.** `embedded-pattern` backend emits `no_std`
  Rust + event-list calls against a stub shim trait. Compile-only.
- **M10 — First Renode shim.** Reference shim for one MCU family
  (Cortex-M7 / STM32H7 likely). Renode-based runtime validation in
  CI for examples 1, 5, 9 (the ones most representative of embedded
  workloads). Single-MCU at first.
- **M11 — Multi-MCU Renode.** A tier-3 schedule with workers spread
  across two co-simulated MCUs (e.g. master STM32 + sensor STM32
  connected over SPI). Validates that the multi-worker embedded
  story is more than theoretical. Hardware-in-the-loop remains a
  stretch goal, not a milestone.

Each milestone ships a tagged release with a numbered example matrix.
A milestone is not done until its CI matrix is green. Tier-1 cells
are mandatory; tier-2/3 cells follow §10's compile-mandatory /
run-best-effort discipline.

## 12. Tech stack

Three tools, each doing one thing.

### 12.1 Nix flake — reproducible dev shell

One `flake.nix` at the repo root provides:

- A pinned Rust toolchain (rustc + cargo + clippy + rustfmt).
- `just`, the task runner (§12.3).
- `rust-analyzer` for IDE integration.
- Tier-specific tools added when their tier lands: an MPI
  implementation at M7, Renode at M10.

Principles:

- No verbose `shellHook` echoes. Enter the shell silently.
- MSRV is pinned in the flake, not in `Cargo.toml`. One pin, not two.
- CI enters `nix develop` first; no system-wide tool dependencies.

### 12.2 Cargo — build

One Rust workspace. Each backend is its own crate plus a sibling
`capabilities.toml`. The workspace `Cargo.toml` is the registry of
which backends exist; there is no in-code plugin registry. Adding a
backend means adding a crate, its capabilities file, and a workspace
member entry — three concrete things, no hidden machinery.

### 12.3 Just — task runner

One `justfile` at the repo root, kept deliberately short. Every
recipe has a one-line comment. Recipes do not bloat with
example/schedule/backend-specific one-offs — the `e2e` harness is
one entry point that runs the full matrix, parameterised by flags.

Reference shape (starting set; recipes added only when load-bearing):

```just
# Build all crates in the workspace.
build:
    cargo build --workspace

# Run unit tests.
test:
    cargo test --workspace

# Fast type-check without codegen.
check:
    cargo check --workspace

# Apply rustfmt.
fmt:
    cargo fmt --all

# Lint. Warnings are errors.
clippy:
    cargo clippy --workspace -- -D warnings

# Full end-to-end differential matrix.
# Compiles every (example × required schedule × supporting backend),
# runs the resulting binaries, diffs against reference.bin.
e2e:
    cargo run --release --bin nucleus-e2e

# Remove build artefacts.
clean:
    cargo clean
```

What the justfile is **not**:

- Not a place for `just run-stencil-on-pthreads-sync` style one-offs.
  Such queries are flags on the `e2e` binary, not new recipes — the
  matrix is data, not code.
- Not a build-system orchestrator. It invokes cargo; it doesn't
  reimplement it.

## 13. Open questions / risks

- **TCP backpressure vs. place capacity.** Application-level place
  capacity (`buffer=N`) must be matched by kernel-level socket buffer
  size, or a sender blocks below the layer the scheduler can see. v2
  computes the required `SO_SNDBUF`/`SO_RCVBUF` from the net and sets
  it in the generated `run.sh`. If the OS refuses (limits), compile
  fails loudly. No application-level credit protocol.
- **Bit-identical output across backends.** Trivial for integer
  algorithms; non-trivial once floating-point reductions enter (sum
  order matters). Either restrict examples to integer/deterministic
  FP, or compare with epsilon. Leaning toward integer-only for v2.
- **Greedy schedule quality.** Greedy linearisation may produce
  obviously-bad schedules for some examples. Acceptable for v2 as long
  as the resulting net is bounded and deadlock-free. Performance is
  not a v2 goal.
- **Petri-net library scope creep.** v2 needs ~500 lines: places,
  transitions, arcs, marking, firing simulator, reachability under
  acyclic firing, isomorphism check. If we find ourselves wanting
  coloured nets, hierarchical refinement, or model checkers, scope
  is wrong — back out, simplify the schedule, don't extend the net.
- **Where does `reuse` get its information?** Static stride analysis is
  feasible; data-dependent strides aren't. Restrict `reuse` to
  affine-indexed loops only and reject the rest.
- **Source single-file rule vs. example count.** 12 examples × N
  schedules is fine. If we ever need shared code across examples or
  across schedules of one example, reconsider — but not before then.
- **Schedule completeness checking.** A schedule that omits placement
  for some kernel must be a hard error. A schedule that over-specifies
  (places a kernel that doesn't exist in the algorithm) is also a hard
  error. The compiler must reject both before any codegen runs. This
  is what keeps the two files honest.
- **Algorithm changes that break schedules.** Renaming a kernel or a
  loop variable in the algorithm silently invalidates every schedule
  that references it. v2 accepts this — schedules name kernels by
  string, errors are immediate at next compile. Anything more clever
  (versioning, refactoring tools) is out of scope.
- **Rust edition / MSRV.** Pick one and stick to it. Probably edition
  2021, MSRV pinned to whatever stable was 6 months before M0.
- **Tier-2 / tier-3 maintenance burden.** Every backend is a Rust crate
  that must compile against every supported example and schedule.
  Tier 1 is seven backends. Tier 2 adds two. Tier 3 adds one
  presentation backend plus one shim per supported MCU family. The
  shim sprawl is real and unbounded; v2 caps at one reference shim
  and treats further shims as out-of-tree. State this so the
  community contribution model is clear from day one.
- **MPI ≠ embedded ≠ CPU.** The three tiers have different cost
  classes. A new tier-1 backend is days to weeks. A tier-2 backend is
  weeks to months (MPI library bindings, SPMD codegen, cluster CI).
  A tier-3 shim is months (per-MCU peripherals, DMA controllers,
  IRQ vector tables, memory layout, `no_std` constraints, real-time
  validation). Don't treat them as interchangeable line items in the
  capability matrix.
- **Capability mismatch.** Some (algorithm, schedule, backend) triples
  legitimately cannot work — e.g. a reduction schedule that demands
  `MPI_Allreduce`-style collective semantics on a backend that emits
  only point-to-point, or a stencil schedule requiring tightly-coupled
  memory on an MCU without TCM. v2 must report these as **early
  compile errors with named missing capabilities**, not as silent
  fallbacks or runtime failures.
- **Heterogeneous worker classes vs. simple form.** Adding worker
  classes and memory regions to §6.3.1 is a significant surface
  expansion. Risk: the simple form rots while the typed form bears
  all the testing. Mitigation: every tier-1 example uses the simple
  form; every tier-3 example uses the typed form; both forms are
  exercised by CI continuously.
- **The model claims "platform-agnostic" but only for an algorithm
  class.** Affine static, single-assignment, no data-dependent
  indexing, no recursion. v2 must clearly communicate this *up front*
  in user-facing docs, not just bury it in §3. A user who reaches for
  Nucleus expecting it to handle a sparse-matrix solver will be
  disappointed and won't come back.
- **`check` assertions are checkable, not prescriptive.** v2 has no
  cost model and does not adjust schedules to meet a `latency_max`.
  The assertion only tells the user whether their hand-written
  schedule meets the requirement. The seed of a v3 prescriptive
  `solve_latency_max` directive is here, but v2 explicitly does not
  ship it. Communicate this clearly in user docs — a user who
  assumes the compiler is optimising for their latency budget will
  be surprised when it isn't.

## 14. What this is, what it isn't

**Is:** A pre-compiler for a platform-portable distributed Rust
overlay where IO semantics, decomposition, and target are first-class
schedule directives — and the algorithm doesn't have to know.

**Isn't:** A research project. A thesis. A framework. A startup. A
runtime. A scheduler-in-the-traditional-sense. A replacement for MPI,
Halide, OpenMP, or Embassy. (It composes with them — each appears as a
backend or shim — but doesn't replace any of them.)

**Tested by:** A CLI, a test suite, a CI matrix. The tier-1 matrix is
the falsification rig: if it goes green after M3, the model is sound;
the rest is engineering. The tier-2 and tier-3 work after M6 is what
makes the project *worth* having built — but it cannot prove the model
on its own, because cluster CI and embedded HIL are too expensive to
exhaustively cover. The cheap CPU tier carries the formal claim; the
expensive tiers carry the practical payoff.

**Worth doing when:** The combination of static schedule + precise IO
+ target portability is more valuable than what existing tools
(Halide, MPI, Embassy, OpenMP) deliver individually. The bet is that
making IO and decomposition first-class schedule directives is the
missing piece. M3 tells us whether the bet is sound. M10 tells us
whether it's useful.
