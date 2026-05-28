# `check loop V : latency_max=T` — runtime semantics, scope, and honest limits

Status: descriptive, normative for what v2 actually measures. Authored under
TASK-0052.03 after TASK-0052.01-.05 landed end-to-end.

Cross-references:

- [PRD §6.3.5 "Runtime assertions"](../nuc-nucleus/PRD.md) (lines 533-582 of
  `nuc-nucleus/PRD.md` at time of writing). The source of truth for the
  source-language shape.
- [`grammar-sched.md` §1 (EBNF), §4.3 (alignment record)](grammar-sched.md).
- TASK-0052 (parent) and sub-tasks 0052.01 (parser/IR), 0052.02 (panic
  codegen), 0052.04 (log/count codegen), 0052.05 (multi-worker codegen),
  0052.03 (this doc).
- TASK-0079 (grammar-vs-example reconciliation, done), TASK-0106 (open:
  decide on a measurement Event variant if/when end-to-end latency is
  promoted to a first-class concept), TASK-0220 (strip-mining + check
  combination, rejected at sched-lower), TASK-0222 (shared codegen
  templates), TASK-0042.01 (pthreads-async codegen consumer), TASK-0048
  (tier-3 Renode forward-carry).

## 1. What v2 actually measures

`check loop V : latency_max = T` lowers to: at every iteration of the
EventList `Event::Loop` whose `iter_var` joins to source variable `V`,
the backend wraps the loop body in

```rust
let _check_start = std::time::Instant::now();
// ... user body ...
let _check_elapsed = _check_start.elapsed().as_nanos();
if _check_elapsed > T_NS_u128 { /* on_violation action */ }
```

Authoritative sites (single source of truth for the rendered template):

- `nucleus/backend-common/src/check_frame.rs` — shared `emit_log_branch`,
  `emit_count_branch`, reporter struct, atomic-static helpers. Three
  tier-1 backends (`pthreads-sync`, `pthreads-async`, `mp-tcp-bufsync`)
  consume the same helpers; drift becomes structurally impossible per
  TASK-0222.
- `nucleus/backends/pthreads-sync/src/lib.rs:694..758` — single-worker
  emit (the panic branch is inline; log/count delegate to the shared
  helpers).
- `nucleus/backend-common/src/multi_worker_walker.rs:300..355` —
  multi-worker emit; shared across pthreads-sync + pthreads-async +
  mp-tcp-bufsync.

The measurement is **per-iteration wall-clock on the worker that runs
the loop body**. That is the only thing v2 guarantees.

## 2. Per-iteration, NOT end-to-end pipeline latency

This is the single biggest gotcha for users coming from real-time
systems literature.

A pipelined schedule with `loop frame : pipeline=3` runs `frame=0..N`
overlapped across stages — at steady state, stage `S_k` is executing
`frame=i+k` while stage `S_{k+1}` is executing `frame=i+k-1`. The
**end-to-end latency** that a real-time engineer typically cares about
("when did data X enter the pipeline, when did it exit?") is the sum
of one-iteration-on-each-stage minus the overlap, plus inter-worker
queueing time on each Push/Wait edge.

`check loop V : latency_max = T` does NOT measure that. It measures one
iteration of `V`'s body on whatever worker the per-worker projection
attaches it to. For a pipelined `frame`, every participating worker
runs its own `Instant::now() ... elapsed()` over its own slice; the
check fires per-stage, not per-frame-end-to-end.

To get end-to-end latency v2 would need a correlation key threaded
through Push/Wait events (each Push carries the source iteration's
`Instant`; each Wait subtracts at the consumer side). That requires a
new EventList variant or a sidecar. The slot is open as TASK-0106
("Decide on latency/measurement event variant once §6.3.5 measurement
points settle"). Until TASK-0106 closes, `check loop V` is the wrong
tool for end-to-end and the documentation must say so loudly.

**Practical guidance:** if your schedule's `check loop V` directive
fires with `pipeline=K`, the violation is "stage took too long", not
"frame missed its deadline". A stage exceeding its slice-of-the-budget
is *evidence* of a deadline miss but is not a deadline miss itself.

## 3. Clock-resolution caveat per tier

PRD §6.3.5 names three clocks. Each carries its own caveats.

### Tier 1 — `std::time::Instant`

- Linux: `clock_gettime(CLOCK_MONOTONIC)` (ns granularity, but the
  underlying timekeeping infrastructure has its own jitter; not
  RT-grade absent kernel preemption disabled).
- Windows: typically `QueryPerformanceCounter`, often a 100 ns
  granularity floor.
- macOS: `mach_absolute_time`, ns-class.

`Instant::now()` and `.elapsed().as_nanos()` are *not* a real-time
clock. Calling them on a contended core under heavy GC / page-fault /
context-switch pressure can themselves take double-digit microseconds.
For tier-1 testing this is acceptable — the gate is "did the iteration
miss the budget on a normal Linux host" — but a tight `latency_max =
1us` on tier-1 is measuring "Instant + scheduler + cache state", not
just "kernel time".

Three tier-1 backends use this path identically: pthreads-sync,
pthreads-async, mp-tcp-bufsync. The fourth tier-1 backend in the matrix
(mp-tcp-event) inherits the same `Instant` substrate when its
multi-worker codegen lands.

### Tier 2 — `MPI_Wtime`

`MPI_Wtime` is a wall-clock with implementation-defined resolution
(`MPI_Wtick` reports it). Typical implementations are microsecond-class
on most hardware. Determinism: not strictly monotonic across MPI ranks
on heterogeneous clusters — `check loop V` measured on rank A and
rank B is not a comparable quantity without synchronization. Not
implemented in v2; this paragraph is forward-carry for when the MPI
backend lands.

### Tier 3 — backend-specific monotonic clock

The PRD says "a backend-specified monotonic clock". On STM32H7 (the
v2 Renode target, TASK-0048) the obvious choice is DWT_CYCCNT at the
HCLK rate (480 MHz ⇒ ~2 ns ticks, 32-bit wraps every ~9 s) or SysTick
+ overflow counter for longer ranges. Both have well-known issues
(DWT_CYCCNT halts under debug-step; SysTick interrupt cost biases
measurements of itself). The Drop-guard summary path used by tier-1
`on_violation=count` is also problematic on bare-metal — Rust Drops at
`fn main()` return do not fire if the MCU just halts. Tier-3 backends
will need a different sink (UART line on first violation, RTT channel,
in-flash counter dumped on watchdog reset). TASK-0048's forward-carry
covers this.

RESOLVED (TASK-0048.04 + TASK-0048.08): the embedded-pattern backend
uses SysTick (not DWT_CYCCNT — DWT may not advance under Renode's
non-cycle-accurate timing), exposed as `NucleusShim::monotonic_ns`.
`on_violation=count` lowers to a module-scope `AtomicU32` counter (NOT
`AtomicU64`, which is absent on `thumbv7em-none-eabihf`); the summary
sink is the cortex-m-rt `#[entry]`, which flushes a one-line USART1
summary AFTER `run` returns and BEFORE the `loop {}` spin — the
bare-metal program-exit equivalent of the tier-1 Drop-guard. A SEPARATE
physical diagnostic channel (2nd UART / RTT / SWO) so the summary is not
interleaved with raw USART1 output is the deferred PART-2 follow-up
(TASK-0048.09). `on_violation=panic` stays rejected (it bricks the MCU).

## 4. `on_violation` trade-offs

Three actions, three different operational profiles. Pick by what you
want a violation to *do*, not by what reads nicest.

### `panic` (default)

- **Tier-1 emit:** inline `if _check_elapsed > T { panic!("...") }`
  with the user's `loop_var`, the measured ns, and the threshold ns
  in the message — see TASK-0052.02 AC#3.
- **Process effect:** the generated `Cargo.toml` sets `panic = "abort"`
  in `[profile.release]` (the only profile the emitted `run.sh`
  builds — `cargo build --release`). A panic from any worker thread
  therefore `SIGABRT`s the whole process. Exit code is the abort
  signature (not 101). Under `panic = "unwind"` (NOT the default
  for emitted projects, and would require editing the generated
  `Cargo.toml`) a worker-thread panic propagates via
  `JoinHandle::join().expect()` to the host and exits 101 with empty
  stdout — the cross-backend differential treats either as a clean
  assertion signal because stdout is empty in both cases. The
  `[profile.dev]` profile inherits the rustc default (unwind), but
  the `dev` profile is never used by the e2e pipeline; if a user
  runs `cargo build` (dev) on the emitted project by hand they get
  unwind semantics, which is a documented difference from the
  `run.sh`/`run.sh release` path the matrix tests.
- **Multi-worker:** each worker thread panics independently. The first
  one to violate wins; the abort takes the rest down with it. Workers
  that violated but had not yet executed the comparison at SIGABRT
  time are not reported.
- **When to pick this:** safety-critical assertions where "this MUST
  hold" is the contract; tier-1 testing where loud failure is the
  point.

### `log`

- **Tier-1 emit:** inline `if _check_elapsed > T { eprintln!("warning:
  ...", _check_elapsed) }`. Stderr, NOT stdout (the cross-backend
  differential compares stdout/`output.bin` only; eprintln stays
  determinism-safe by construction).
- **Multi-worker:** N workers `eprintln!` on N threads to one shared
  `stderr` file descriptor. Lines do not interleave intra-line on
  Linux (write() of <= PIPE_BUF is atomic for pipes/FIFOs, and
  `eprintln!` issues one write), but order across threads is
  unspecified. A user grepping for "warning:" sees all of them; a user
  expecting timestamped event order is reading the wrong signal.
- **Iteration cost:** an `eprintln!` per *violating* iteration. A loop
  that violates every iteration produces an `eprintln!` storm. There
  is no rate-limiting in v2.
- **When to pick this:** development and CI surfaces where you want
  the violation observable but don't want to bail out; throwaway
  diagnostic instrumentation.

### `count`

- **Tier-1 emit:** file-scope `static NUC_CHECK_COUNT_<id>:
  AtomicU64`, an `fn main`-local Drop-guard `_nuc_check_reporter_<id>`,
  and a per-violation `fetch_add(1, Relaxed)`. The summary line prints
  at the guard's Drop — i.e. when `fn main()` returns.
- **Multi-worker pthreads-sync / pthreads-async:** SHARED static
  across worker threads. One aggregated summary line for the whole
  run. Verified by TASK-0052.05 multi-worker codegen.
- **Multi-worker mp-tcp-bufsync:** PROCESS-LOCAL static. Each worker
  process gets its own counter and its own Drop summary. For an
  N-worker schedule the stderr ends up with N "violated ... K
  occurrence(s)" lines, one per process. Cross-process aggregation is
  the open design question on TASK-0052.05; not in v2.
- **`std::process::exit()` interaction:** `exit()` does NOT run Drops.
  The Count summary is therefore lost if generated code (or any
  user-supplied kernel) calls `exit()`. The v2 generated runtime does
  not call `exit()`, so this is a theoretical hole today, not a known
  bug. If you add an external `exit()` call paths in a kernel, the
  Count summary is no longer trustworthy.
- **Quiet-on-zero:** the Drop body gates on `n > 0` before printing,
  so a clean run prints NOTHING on stderr. This is what keeps the
  cross-backend differential indifferent to the presence of `Count`
  in a schedule.
- **When to pick this:** measuring violation *rate* over a long run
  without affecting control flow; batch-validation regression suites.

## 5. Future grammar slots

`grammar-sched.md` §1 defines `CheckAssert` as

```
CheckAssert ::= 'latency_max'  '=' TimeLit
              | 'on_violation' '=' ViolationKind ;
```

The same `CheckAssert` slot accepts further metrics without grammar
breakage. PRD §6.3.5 explicitly enumerates two reserved future
variants:

- **`jitter_max = T`** — per-iteration deviation from the running
  mean (or from a declared period). Implementation shape: a second
  field on `nucleus_compiler::event::CheckFrame`; the `let _check_elapsed`
  bookkeeping in `check_frame.rs` extends to maintain an EWMA or
  Welford running variance; one more `match` arm.
- **`throughput_min = R`** — iterations per second floor. Shape:
  total elapsed `Instant` from first to last iteration, divided by
  iteration count, compared at loop exit (not per-iteration).

A future per-transfer `buffer_max` (PRD §6.3.5) does NOT slot into
`check loop`; it would be a sibling `check transfer X` clause. The
grammar reserves `check` + qualifier (`loop` | `transfer`) so this is
not a breaking change. TASK-0079 chose to require the `loop` qualifier
explicitly precisely to keep this room.

**This is checkable, not prescriptive.** v2 does not adjust the
schedule to *meet* `latency_max`; it only checks whether the
hand-written schedule does. A future `solve_latency_max` directive
with the same surface shape but a v3 cost model is the reserved
prescriptive sibling (PRD §6.3.5 paragraph "This is checkable, not
prescriptive"). v2 ships the observation; the optimiser is v3 or
never.

## 6. Strip-mining interaction

When a schedule combines `loop V : block=N` (strip-mine) and `check
loop V : latency_max = T`, the source variable `V` is decomposed by
`block_transform` into outer/inner tile loops. The inner
`Event::Loop` carries `block_tag.is_some()`. The projection pass
`inject_check_frames` skips strip-mined inner loops by design (the
strip-mining is implementation detail, not a source-visible loop).

A naive lowering would silently drop the user's check — exactly the
"silent loss of an assertion" failure mode that TASK-0052.02's
review gate was hardened against. The chosen v2 behaviour is to
reject the combination at sched-lower with `CheckOnStripMinedLoop`,
naming both the check and the block directive and pointing at the
two actionable choices (drop one or the other). See TASK-0220 for
the design discussion of what the assertion *should* mean
post-strip-mine; v2 does not pick.

## 7. Examples in-tree that exercise this directive

As of cycle 42 (TASK-0052.03 close), the only in-tree user is

- `nuc-nucleus/examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc`
  line 105:
  `check loop frame : latency_max = 10ms;`

Example 14 is NOT in the e2e required matrix
(`nuc-nucleus/e2e-matrix.toml:65` lists the runnable set — example 14
is excluded because its tier-3 Renode target is M11 future work).
No tier-1 e2e cell exercises `check loop` today.

The README of example 14 (`nuc-nucleus/examples/14-hearing-aid/README.md`
lines 27-45) explains the directive at the right level: it states the
physical meaning ("frame latency budget"), notes the
checkable-not-prescriptive framing, and points at the seed of a future
prescriptive `solve_latency_max`. **However**, line 32 of that README
quotes the directive as `check frame : latency_max = 10ms;` — without
the `loop` keyword, i.e. the pre-TASK-0079 form. The schedule file
itself is conformant; only the README quotation is stale. Filing as
the follow-up below.

AC#2 of TASK-0052.03 — "every example using `check loop` has README
text explaining what the assertion is checking and what a violation
means physically" — is *currently* satisfied by example 14's README in
spirit (the explanatory paragraph is present and physically grounded
at "frame latency budget"). Future examples authored to use `check
loop` (TASK-0106-track end-to-end work, or a tier-1 cell promoting
example 14 to the required matrix) must include analogous explanatory
text per this AC.

## 8. What is intentionally NOT documented here

- **CPU-cycle counting.** RDTSC, DWT_CYCCNT, `perf_event_open` — v2
  uses Instant only. Cycle-level reasoning is a TASK-0048 / TASK-0106
  forward-carry, not a current capability.
- **Pre-iteration "warm-up" rounds.** Real-time benchmarking
  conventions discard the first N iterations to avoid cache-cold and
  branch-predictor-cold bias. v2's `check loop V` does NOT discard
  anything; the first iteration is measured the same as the
  thousandth. A user concerned about cold-start should write their
  algorithm with a discard prologue at the kernel level.
- **Recursive / nested `check loop`.** A nested loop with its own
  `check` is supported (each gets its own `CheckFrame` and its own
  `_check_start` shadowing scope), but the cumulative-cost story
  ("the outer check fires, but how much of its budget did the inner
  check's body account for?") is not analysed. The user reads two
  independent assertions; the runtime does not correlate them.

## 9. Follow-ups identified

These were surfaced while writing this doc; none are blockers for
TASK-0052.03 itself.

- **Example 14 README:** line 32 quotes the stale `check frame`
  form; bring it in line with the schedule file's conformant `check
  loop frame` form (one-line edit in README only). Filing as a
  separate task — the README quotation must read identically to the
  schedule file or the docs go out of sync the moment someone reads
  the README before opening the schedule. See "Filed follow-up" in
  TASK-0052.03 implementation notes.
- The "filed follow-up" referenced above lives in the backlog as
  TASK-0247 (see TASK-0052.03 notes for the filing record).
