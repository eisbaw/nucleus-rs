# Runtime performance + scaling study — methodology

This document is the **single source of truth** for how the runtime
performance and scaling study (TASK-0455.04) is measured: machine spec,
warm-up and repetition discipline, the point estimate and spread, how the
load context is recorded, the argument that the measured kernels are
input-independent, and — explicitly — the list of claims this study
**cannot** make. The numbers themselves live in `docs/perf-study.md`; the
runner is `scripts/perf-study-run.sh` (+ `scripts/perf-study-fixtures.sh`,
`scripts/perf-study-wire.py`, `scripts/perf-study-wirebase.py`), reproduced
by `just perf-study`.

This study deliberately mirrors `docs/case-study.md`'s discipline,
including its review-corrected floor-sensitivity reporting (report both a
min and a median reading where a denominator enters a ratio).

**Measurement provenance:** the figures in `docs/perf-study.md` were
measured on 2026-06-10 with the repo at commit `2363fae`, in the pinned
Nix dev shell. The raw per-row TSVs are retained on every green run under
`nucleus/target/perf-study/artifacts/{results,wire}-<stamp>.tsv`.

---

## 1. Machine spec

All numbers in `docs/perf-study.md` were measured on a single
developer laptop — **this is the binding caveat of the whole study**
(§7):

| property      | value |
| ------------- | ----- |
| CPU           | Intel Core Ultra 7 165U |
| cores         | 10 physical (2 P-cores w/ SMT + 8 E-cores), **14 logical threads** |
| topology      | heterogeneous P/E cores, single socket, single NUMA node |
| max freq      | 4.9 GHz (P-core turbo), 0.4 GHz min |
| governor      | `powersave` (intel_pstate), turbo **enabled** (`no_turbo=0`) |
| RAM           | ~32 GB |
| kernel        | Linux 6.19.6 |
| toolchain     | the repo's pinned Nix dev shell (`flake.nix` `rustChannel`) |

Two spec facts are load-bearing measurement caveats and are carried into
every reading:

- **Heterogeneous cores.** The 165U is not a uniform 10-core machine; it
  has fast P-cores and slower E-cores. A worker placed on an E-core is
  slower than the same worker on a P-core, and the Linux scheduler — not
  the study — decides placement. This adds a structural, non-Gaussian
  source of run-to-run variance to any multi-worker timing that the
  min-of-N estimate (§3) mitigates but does not remove.
- **`powersave` governor + turbo.** Clock frequency is demand- and
  thermal-driven, not pinned. We did **not** pin frequency (it would
  require root and is not reproducible across the dev shell), so absolute
  wall-times carry a frequency-scaling uncertainty. We report relative
  shapes (scaling curves, crossover points, wire reductions), which are
  far less sensitive to absolute clock than to the *ratios* between cells
  measured back-to-back under the same governor state.

---

## 2. Warm-up policy

Each timed cell is **built before it is timed** (the generated Rust
project is `cargo build --release`'d first), so no compile cost ever
enters a wall-time sample. Before the timed runs, **one warm-up run is
executed and discarded** — it pages in the binary, warms the OS file
cache for the input, and warms the CPU from any idle clock state. Every
timing row in `docs/perf-study.md` is therefore a **cache-warm** reading
(the `cache` column records this explicitly as `warm`).

**Symmetric launch (both backend arms time pure execution).** The
shared-memory (`pthreads-async`) arm times its single binary directly. The
message-passing (`mp-tcp-*`) arm launches via an emitted multi-process
`run.sh` — but that script begins with a `cargo build --release --quiet`
line, which on a warm tree is a ≈28 ms no-op fingerprint check on this
machine. Timing `bash run.sh` would therefore charge the mp-tcp arm
25–40 % of a ≈100 ms sample for cargo work the pthreads arm never pays —
**asymmetric arms** (the defect TASK-0455.04's review found). The runner
now, after the untimed byte-checked correctness run has built every
binary, strips that single cargo line into a `run-timed.sh`
(`strip_cargo_from_run_sh`, hard-failing if the line is not found exactly
once) and times *that*. Both arms now time pure process launch + execution
with no compiler in the loop.

Wire-volume measurement (§5) is taken in a **separate** run under
`strace`, never folded into a timing run, because `strace` ptraces every
traced syscall and inflates wall time. The byte counts it yields are
exact and unaffected; only the wall would be perturbed, so no wall is
ever read from a straced run.

---

## 3. Repetitions, point estimate, and spread

- **Repetitions.** Each runtime cell is timed **N = 9** times after the
  warm-up (the same N the case study uses for its runtime rows). The
  startup-floor-style stable denominators are not re-measured here because
  this study reports *relative* shapes rather than a floor multiple; where
  a baseline enters a ratio we report the ratio against both the min and
  the median baseline reading (the floor-sensitivity discipline carried
  over from the case study's wave-8 correction).
- **Point estimate: minimum of N.** Scheduler preemption, E-core
  placement, IO jitter, and background load only ever *add* time to a run,
  so the **minimum** of the N samples is the most reproducible estimate of
  the work itself. This is the identical rationale `scripts/case-study-run.sh`
  documents for its min-of-9 runtime and min-of-60 floor.
- **Spread.** Alongside the min, each row reports the **median** and a
  spread figure `(median − min) / min` in percent. A large spread is the
  honest signal that a row was noisy (contended machine, E-core
  placement); it is reported, not smoothed away. Where the min and median
  would lead to different qualitative conclusions (e.g. a crossover that
  holds on the min but not the median), `docs/perf-study.md` says so.

---

## 4. Load-context recording and dirty-row handling

This machine is the maintainer's interactive workstation; per the study's
ground rules it **occasionally builds other projects** while measurements
run. Therefore:

- **Every row records its 1-minute load average** (the `loadavg` column)
  at the moment it was timed. Nothing is hidden behind an "idle machine"
  assumption.
- A row is flagged **`LOAD-DIRTY`** by the runner when the 1-minute load
  average exceeds `(worker count + 1.5)` — i.e. when the machine carried
  meaningfully more runnable tasks than the cell itself uses. Dirty rows
  are re-measured when the machine quiets, and `docs/perf-study.md`
  reports whether a given table was taken clean or carries dirty rows.
  Because the point estimate is the **minimum** of 9 runs, a transient
  spike during one of the 9 samples does not corrupt the estimate — but a
  *sustained* high load biases even the minimum upward, which is exactly
  what the dirty flag catches.
- The `powersave`/turbo and heterogeneous-core facts of §1 mean even a
  load-clean row carries residual frequency- and placement-variance; the
  spread column is the in-band indicator of that residual.

---

## 5. Wire-volume measurement method (and the method that failed)

The narrowed-transfer claim (TASK-0453.22) is quantified in **measured
bytes on the wire**, not estimated, for the message-passing backends
(`mp-tcp-bufsync`, `mp-tcp-event`), whose workers are separate OS
processes communicating over loopback TCP sockets.

**Method actually used: `strace` socket-send byte counting.** The cell is
run once under
`strace -f -e trace=sendto,sendmsg -e signal=none -qq -o LOG run.sh …`.
Each completed send appears as a log line ending in `= N` (N = bytes
actually placed on the socket); under `-f`, a send interrupted by another
thread's event is split into a `<unfinished ...>` line and a matching
`<... sendto resumed> … = N` line, so the byte count always lands on a
line ending in `= N` and counting those lines counts each send **exactly
once**. We bucket the sizes: small fixed-size control/framing tokens
(≤ 16 bytes — length prefixes, sync handshakes) are reported separately,
and the **data** total is what we compare against the whole-array
baseline. (`scripts/perf-study-wire.py` carries a guard comment about a
real bug found during development: an earlier regex required a literal
`sendto(` and silently dropped every *resumed* line, under-counting under
load; the fix matches "mentions `sendto`/`sendmsg` and ends in `= N`".)

**Method tried first and rejected: `/proc/<pid>/io` wchar deltas.** The
obvious least-intrusive route — read each worker's `wchar` (bytes handed
to write-family syscalls) and subtract known file IO — was implemented and
**falsified empirically**: a transfer that demonstrably moved kilobytes
showed `wchar_total = 20` bytes, because Rust's `TcpStream` writes go
through `sendto(2)`, whose payload the kernel does **not** add to `wchar`
(only the `write(2)`/`pwrite` family lands there). The `/proc/io` route
under-reports socket traffic by orders of magnitude and was discarded.
This is recorded so the choice of the heavier `strace` method is
justified, not arbitrary.

**Overhead and honest caveats of the chosen method:**

- `strace` ptraces every traced syscall, inflating wall time (commonly
  2–10×). This is why wire volume is a **separate** run from timing.
- `sendto` byte returns are **application payload at the syscall
  boundary**, not TCP segment bytes on the NIC (no headers, no
  retransmits). For a loopback transfer this is the right "application
  bytes moved" figure — exactly the quantity TASK-0453.22's narrowing
  reduces — but it is not a packet capture.
- We count the **send** side (`sendto`/`sendmsg`); every cross-worker
  payload is sent once, so the send-side sum is the wire volume with no
  double-counting.

**Baseline side: an honest asymmetry (narrowed = measured, baseline =
computed).** The narrowed-transfer figure — the quantity TASK-0453.22's
narrowing actually produces — is **measured** on the wire, as the task
requires. The whole-array baseline it is compared against is **computed
statically** (`edges × workers × array_bytes`: each distributed data array
sent in full to every receiver on its edge), because the pre-TASK-0453.22
binary that broadcast whole arrays is not checked out anywhere to measure.
This is a deliberate, disclosed asymmetry, not a both-sides-measured
claim: the reduction percentage is `1 − measured/computed`, so it is exact
on the numerator and analytic on the denominator.

The rigorous alternative — **not taken here** — is the git-worktree A/B
route the backend differential uses elsewhere: check out the pre-.22
commit into a second worktree, build the same cell, and `strace` *its*
sends to get a measured baseline. We did not do that because the
whole-array baseline is a closed-form, uncontested quantity (every edge
sends the full array — there is nothing to discover by measuring it), so
the worktree build + strace would cost a second toolchain checkout to
re-confirm an arithmetic identity. If the baseline were in any doubt the
A/B route would be mandatory; here the computed figure is reported as
computed and the measured figure as measured.

`scripts/perf-study-wirebase.py` also extracts the emitted
`name[lo..hi].to_vec()` spans from the generated source. That is a genuine
cross-check **only for the `pthreads` backends**, which emit those slice
spans; the `mp-tcp-*` backends this study measures on the wire slice
through a different wire path and emit **0** such spans, so for them the
span count is 0 and the **measured** strace bytes are the sole authority
(the script prints the span count so a 0 is visible, never mistaken for
corroboration).

---

## 6. Input-independence of the measured kernels (verified, not assumed)

The standing perf discipline (memory:
`feedback-perf-measure-worst-case-and-task-solution-can-be-workaround`)
requires worst-case inputs — *unless* the kernel's runtime is provably
input-independent, in which case that must be **verified and documented**
rather than hand-waved. For this study's three workloads it is verified:

- **All three compute kernels are straight-line integer arithmetic with
  no value-dependent control flow.** Grepping the measured kernels
  (`madd`, `blur3`, `accumulate`, `combine`) for `if`/`match`/`while`/
  `break`/`continue` over data values returns **nothing** — the only
  branches anywhere in the kernel files are command-line argument parsing
  and file-IO error handling, neither of which depends on the *values*
  being computed.
- **Iteration counts are fixed by the problem size, not the data.** The
  matmul triple loop runs `N³` `madd`s, the stencil runs over every
  interior pixel once, and the reduction folds every element once. There
  is no early exit, no data-dependent loop bound, and no data-dependent
  memory-access pattern (every index is an affine function of the loop
  variables, identical for every input).
- Consequently the wall-clock time is a function of the **problem size and
  the schedule**, not of the input *values*. The deterministic input
  generators (`gen/`) still produce spatially/positionally varying,
  bounded data so the byte-identity differential is meaningful, but for
  *timing* purposes any same-shaped input would yield the same runtime
  within noise. **There is no adversarial "worst-case input" to seek for
  these kernels** — the worst case and the average case coincide, which is
  the documented justification for not constructing one.

(The contrast is the data-dependent-control-flow workloads —
`21-jacobi-converge`, the gather/scatter examples — which are explicitly
**out of scope** here, both because they are not in the swept set and
because their gate cost at scale is a separate open problem; see
`docs/perf-study.md` "walls" and the thesis `sec:fw-quant`.)

---

## 7. What this study CANNOT claim (explicit scope limits)

These are not hedges; they are hard boundaries of the measured substrate.
`docs/perf-study.md` repeats the relevant one beside every claim.

1. **Loopback is not a cluster.** Every message-passing number was taken
   with all worker processes on **one machine**, communicating over
   loopback TCP. Loopback has no network latency, effectively unbounded
   bandwidth relative to a real NIC, and shared memory-bandwidth contention
   a cluster would not have. **No claim here transfers to cluster
   performance.** The thesis keeps the cluster caveat (`sec:fw-quant`); this
   study reproduces it and does not pre-empt the real-cluster study the
   thesis defers. This holds equally for the MPI tier-2 backends, which are
   deliberately **not swept** — adding another local launcher would not
   change the substrate; only real multi-node hardware would, and there is
   none here.
2. **One machine, one configuration.** A single laptop CPU, one kernel,
   one governor, measured once. Nothing here is a cross-machine or
   cross-architecture claim.
3. **Heterogeneous P/E cores + unpinned frequency** (§1) inject variance
   that the min-of-N estimate reduces but cannot eliminate; absolute
   wall-times carry a frequency-scaling uncertainty.
4. **Shared-machine noise** (§4): the host is the maintainer's interactive
   workstation; load-dirty rows are flagged and re-measured, but the
   machine was not isolated.
5. **Per-worker memory footprint is not reduced** (the half of the
   over-communication shortcoming TASK-0453.22 did not address): every
   receiver still allocates the whole array and pastes its band in, so the
   distributed cells do not reduce memory pressure even where they reduce
   wire volume. This study measures wire volume and wall time, **not**
   resident memory of the worker processes.
6. **Compute is cheap integer arithmetic.** These kernels do a handful of
   integer ops per element; they are deliberately bandwidth/coordination-
   bound, not compute-bound, so the crossover findings are specific to
   cheap-compute kernels and should not be read as "distribution rarely
   pays" in general.
