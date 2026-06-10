# Production case study: a VGA video-frame-strip stencil at realistic size

This document is the **production witness** for the Nucleus v2 compiler
(TASK-0455.03). Every example in the corpus (`nuc-nucleus/examples/`) is
toy-sized — 16×16 stencils, 16³ matmuls — and its runtime is dominated
by process startup rather than computation. That makes the corpus a good
*falsification rig* (it exercises the dataflow shapes) but a poor witness
that the tool does *real work* at a size where the per-run startup
floor no longer dominates. (Precisely that: the measured claim is
startup-floor insignificance; the 114 ms naive wall still includes
~78 MB of file IO + decode/encode alongside the 9-integer-op-per-pixel
blur, and the study does not decompose compute vs IO.)

This case study fills that gap with **one** realistic workload carried
end to end: it compiles, builds, runs, and is byte-identical to an
independent reference oracle across a single-worker baseline and two
distributed decompositions (one tier-1, one tier-2), at a problem size
where runtime is **>100× the process-startup floor**.

Reproduce all numbers below with:

```
just case-study
```

(runs `scripts/case-study-run.sh` inside the `.#mpi` Nix shell; prints
the cell timings, byte-diffs, gate RSS, and wire-span numbers — the
height-sweep table in §1 and the single-frame row were measured by
one-off runs of the same script at other heights, not by the default
invocation). The fixture lives under
`docs/case-study/`, **outside** `nuc-nucleus/examples/`, so the e2e
matrix never enumerates it.

---

## 1. The workload, and why this one

**Workload:** a single-pass 3×3 box-blur stencil — the same algorithm as
the toy example `05-stencil`, identical integer arithmetic
(`wrapping_add` sum of nine taps, truncating `/9`), scaled up.

**Realistic size:** a **32-frame VGA strip** — 32 standard VGA frames
(640×480) stacked vertically into one image of **W = 640, H = 15360**.
That is 9 830 400 pixels, **39 321 600 bytes (≈ 37.5 MiB) per array**.
This models the realistic production unit for an image pipeline: a
*stream* of frames (here ≈ ½ second of 60 fps VGA video) processed as one
batch, which is exactly how startup cost is amortised in practice.

### Why a single-pass stencil, and not jacobi or a pipeline

The choice is dictated by what the **landed** machinery actually carries
at realistic size, and is stated honestly as such:

- The single-pass stencil's distributed net is **loop-depth-0**: one
  `Push` and one `Wait` per shared buffer place, fired **once**, *not*
  inside an iteration `Repeat`. This is precisely the single-shot
  matched-pair subclass the **symbolic soundness gate** (TASK-0455.01,
  landed) proves bounded + deadlock-free **flat in N** — its
  peak-occupancy argument runs on the *rolled* ACFG and never expands the
  net. So this workload compiles at strip size in tens of MB of RSS (see
  §4) where the expanded-replay gate projected hundreds of GB.

- **Loop-carried** shapes (`16-jacobi`: an iterative stencil with a
  cross-worker halo exchange *per timestep*) and **pipelined** shapes
  (`09-producer-consumer`, `13-cnn`: pre-marked buffers inside a
  `Repeat`) still **expand** the net per iteration. At a realistic
  iteration count they OOM the gate (the coupling the thesis records in
  `sec:fw-quant`). They are therefore **deliberately not** this case
  study — choosing the single-pass stencil is an honest choice of a
  workload the gate carries, not a workaround.

### Why a frame *strip* and not a frame *loop*

Stacking 32 frames into one tall image keeps the net loop-depth-0 (a
single-shot blur over a bigger array). A frame *loop* (`for f in 0..32 {
… }`) would instead place the cross-worker transfer inside a `Repeat`,
expanding the net per iteration — the very shape the gate cannot carry at
realistic counts. The strip framing is the only way to scale a
single-pass stencil while staying inside the gate's flat-in-N class.

The 31 inter-frame seam rows blur across the 480-row frame boundary — a
benign, documented artifact. It is *not* a correctness problem for the
differential: the independent reference oracle blurs the identical strip,
so byte-identity holds regardless.

### Why this size and not a single VGA frame

This is the load-bearing size justification. A **single** VGA frame
(640×480) is too small to be a production witness:

| measurement (this dev machine)                | wall (min) |
| --------------------------------------------- | ---------- |
| nucleus startup floor (4×4 image, naive)      | **0.68 ms** |
| single VGA frame, naive / pthreads-async      | ≈ 4.5 ms   |

A single frame clears the floor by only ≈ 7×, far short of the ≥ 100×
the case study must show — the blur compute is sub-millisecond at VGA,
and the 4.5 ms is dominated by process spawn and 2.4 MB of I/O. Runtime
scales with problem size; the floor does not. Measured naive runtime
across image heights (each a VGA-frame multiple) confirms the crossover:

| height (VGA frames) | pixels   | naive wall | × floor |
| ------------------- | -------- | ---------- | ------- |
| 480 (1)             | 0.31 M   | ≈ 4.5 ms   | ≈ 7×    |
| 3840 (8)            | 2.46 M   | ≈ 29 ms    | ≈ 48×   |
| 7680 (16)           | 4.92 M   | ≈ 56 ms    | ≈ 93×   |
| **15360 (32)**      | **9.83 M** | **≈ 114 ms** | **≈ 168×** |
| 30720 (64)          | 19.66 M  | ≈ 239 ms   | ≈ 398×  |

**32 frames (15360×640)** is chosen: it clears ≥ 100× with margin on
every cell (the *smallest* multiple across all three cells is **168×**,
not the largest — see §5), while keeping the committed arrays at a size
(≈ 37.5 MiB) that the generator can reproduce in well under a second.

The startup floor is measured as the **minimum of 60 runs** of a trivial
4×4-image program (spawn + nucleus runtime init + trivial I/O). Min/min
is the like-with-like pairing (jitter only ever *adds* time to both
sides) — but note the direction honestly: a smaller floor denominator
makes the ×-multiple *larger*, so min-floor is the **flattering**
choice, not the conservative one (the wave-8 review corrected an
inverted claim here). Under the noisier *median* floor observed on this
machine (1.0–2.5 ms) the smallest cell's multiple drops to ≈ 46–110×,
straddling the 100× bar; under the min floor (0.68 ms, consistent with
the thesis's ≈ 1 ms figure) it is 168×. Both readings are reported so
the margin's sensitivity to the denominator is visible.

---

## 2. Schedules and matrix cells

Three cells are exercised (`docs/case-study/schedules/`):

| schedule       | backend            | tier | workers          | role                        |
| -------------- | ------------------ | ---- | ---------------- | --------------------------- |
| `naive`        | `pthreads-async`   | 1    | host             | single-worker baseline      |
| `distributed`  | `pthreads-async`   | 1    | host + w0..w3    | tier-1 distributed witness  |
| `distributed`  | `mpi-nonblocking`  | 2    | host + w0..w3 (mpiexec −n 5) | tier-2 distributed witness |

The **same** `distributed.sched.nuc` drives both distributed cells. Its
transfers are `img_in : async, buffer=2, notify=event` and `img_out :
sync`. `mpi-nonblocking` is a strict capability superset of
`pthreads-async` (async + buffer + event, plus barrier/blocking), so one
schedule targets both. Sync-only backends (`pthreads-sync`,
`mp-tcp-bufsync`, `mpi-blocking`, `openmp-rs`, …) reject this schedule
*loudly at the capability check, before codegen* — by design.

**Row-band partition (`loop y : partition=rows`):** the compute loop
walks `y ∈ 1..15359` (15358 interior rows). The `partition_rows` pass
splits 15358 across 4 workers with the numpy.array_split
floor-with-spillover policy: `15358 = 4·3839 + 2`, giving bands
**3840 / 3840 / 3839 / 3839**. The halo (one row each side) is inferred
from `blur3`'s 3×3 access pattern — it is not declared in the schedule.

---

## 3. Correctness: byte-identity against an independent oracle

The reference oracle (`docs/case-study/reference/`) is a standalone Rust
crate, `std` only, with **no dependency on any Nucleus crate or
generated source** (reference-impl-policy §2). The runner runs the policy
§2 independence scan (`check-reference-independence.awk`) over it before
trusting the differential. It is *re-derived*, not copied from the kernel
(a tap-array fold instead of the kernel's chained `.wrapping_add`), so a
transcription bug is unlikely to coincide.

The input frame (`docs/case-study/gen/`) is a separate `std`-only
generator producing a deterministic, bounded (`[0, 65535]`),
spatially-varying pattern — kept separate from the reference so the
*values* and their *expected blur* share no code.

**Result: all three cells produce output byte-identical to
`reference.bin`** (`cmp -s`), at the full 39 321 600-byte size. The MPI
cell is additionally exercised under `mpiexec --oversubscribe -n 5`
(all five ranks live) on loopback.

> Fixtures (`input.bin`, `reference.bin`, ≈ 37.5 MiB each) are **not
> committed** — they exceed the "a few MB" policy ceiling. The runner
> regenerates them every run from the committed `std`-only crates;
> reproducible by construction (no RNG, no clock).

---

## 4. Compile cost and gate cost — the keystone

`nucleus build` (parse → link → elaborate → lower → **soundness gate** →
project → emit), measured under `/usr/bin/time -v`:

| cell                          | gate+compile wall | peak RSS |
| ----------------------------- | ----------------- | -------- |
| naive / pthreads-async        | ≈ 0.04 s          | ≈ 88 MB  |
| distributed / pthreads-async  | ≈ 0.04–0.06 s     | ≈ 88 MB  |
| distributed / mpi-nonblocking | ≈ 0.06 s          | ≈ 88 MB  |

**The keystone result: the gate's RSS is FLAT in problem size.** The same
distributed schedule compiles at the toy 16×16 size and at the
15360×640 strip in the **same ≈ 88 MB** — because the symbolic gate
(TASK-0455.01) proves the loop-depth-0 net bounded on the *rolled* ACFG
and never builds the expansion. (The ≈ 88 MB is dominated by the driver
process itself, not the gate; the gate's own footprint is iteration-count
independent, as TASK-0455.01 measured at +356 KB over a 32 768× firing
increase for matmul.)

For scale: the **expanded** net for this strip would carry roughly two
Petri nodes per `blur3` firing over **9 798 404** interior pixels ≈ **19.6
million nodes**; projecting the per-node cost the single-replay gate
carried for the corpus matmul (TASK-0453 cycle 6, ~190–930 bytes per
node depending on net structure) puts that at roughly **4–12 GB** —
not prohibitive on a workstation, but three orders of magnitude more
memory than the symbolic path uses, growing linearly with frame count
where the symbolic gate stays flat. (Projection, not a measurement:
it extrapolates per-node costs measured on differently-structured
nets; the wave-8 review corrected an earlier ~100 GB figure here that
over-extrapolated by >10x.)

The compile is sub-second in all cases — a small fraction of the
downstream Rust build of the emitted project (seconds), consistent with
the thesis (`sec:res-quant-time`).

---

## 5. Runtime — and an honest scaling result

Min-of-9 wall time (after a warm-up), and its
multiple of the 0.68 ms startup floor:

| cell                          | runtime  | × floor |
| ----------------------------- | -------- | ------- |
| naive / pthreads-async        | 114 ms   | **168×** |
| distributed / pthreads-async  | 182 ms   | **268×** |
| distributed / mpi-nonblocking | 1535 ms  | **2257×** |

**The size justification is met:** the *smallest* multiple across all
cells is **168× ≥ 100×**, and runtime now scales with the problem, not
with startup.

**An honest scaling result, not a speedup claim.** The distributed
pthreads-async cell (182 ms) is **slower** than the single-worker naive
baseline (114 ms), and the MPI cell (1535 ms) is dramatically slower.
This is expected and is reported, not hidden:

- At this size the per-worker coordination + full-shaped buffer
  allocation (see §7) outweighs the parallel arithmetic, which is itself
  cheap (nine integer ops per pixel). The decomposition pays a real
  overhead the blur does not amortise on 4 loopback workers sharing one
  machine's memory bandwidth.
- The MPI cell runs five oversubscribed ranks on a **single host over
  loopback** — *not* a production cluster. It witnesses *correctness and
  deadlock-immunity at scale* under the buffered `MPI_Ibsend`/`Imrecv`
  path, **not** cluster performance. A credible runtime/scaling study on
  representative hardware is separate work (TASK-0455.04); this case
  study deliberately does not pre-empt it.

This case study's runtime claim is therefore narrow and exact: **the
workload runs at a size where compute dominates startup (≥ 100×) and is
byte-correct across decompositions** — not "the distributed schedule is
faster".

---

## 6. Transfer volume — the wire-narrowing win

With wire-level precise transfer (TASK-0453.22, landed), each
cross-worker edge whose inferred region is a contiguous row span
(`RecvBasis::Flat`) transmits only that band, not the whole array. The
runner extracts the actual narrowed `name[lo..hi].to_vec()` spans from
the generated tier-1 source:

| edge                    | narrowed (this schedule) | whole-array baseline | reduction |
| ----------------------- | ------------------------ | -------------------- | --------- |
| `img_in` host→workers   | 39 336 960 B (4 bands+halo) | 157 286 400 B (4× full array) | **75.0 %** |
| `img_out` workers→host  | 39 316 480 B (4 interior bands) | 157 286 400 B | **75.0 %** |
| **combined cross-worker** | **78 653 440 B** | **314 572 800 B** | **75.0 %** |

The whole-array baseline is what *every* backend transmitted before
TASK-0453.22 — a full ≈ 37.5 MiB copy to every worker on every edge. The
narrowing cuts that to one band (+ a one-row halo) per worker: a 4×
reduction on a 4-way row-band partition. (The narrowed `img_in` total
slightly exceeds one whole array because of the 1-row halo overlap
between adjacent bands; the *per-edge* payload is ≈ ¼ of the whole array,
which is the saving.)

---

## 7. Honest limitations — walls hit, and where they are filed

Per the case-study discipline, every wall is filed as its own task and
named here, not worked around silently:

1. **Per-worker memory is still full-shaped.** Wire *volume* is narrowed
   (§6), but each receiver still allocates the **whole** 39 MB
   destination `Vec` and pastes its band into it — the footprint half of
   the over-communication shortcoming is unfinished. This is why the
   distributed cell does not reduce memory pressure and contributes to
   its runtime overhead (§5). **Filed: TASK-0455.14** (band-shaped
   per-worker allocation).

2. **Loop-carried / pipelined shapes do not scale here.** This case study
   could *not* be a realistic iterative jacobi or a deep pipeline,
   because those still expand the gate's net per iteration (§1). That is
   the open keystone the thesis records (`sec:fw-quant`); the
   single-shot communicating gate (TASK-0455.01) lifted the wall only for
   the loop-depth-0 class. **Filed: TASK-0455.04** (runtime/scaling
   study, which depends on lifting the loop-carried gate) and the
   parent epic's multi-worker bounded break (S7 = TASK-0341.02.01.08).

3. **The distributed decomposition is slower than the baseline at this
   size** (§5). Not a defect — an honest scaling observation. A real
   speedup study needs representative hardware and the memory-footprint
   work above; **filed: TASK-0455.04**.

4. **MPI numbers are loopback, not cluster** (§5). The thesis keeps the
   cluster caveat; this case study reproduces it. No new task — this is a
   standing, documented scope limit of the tier-2 acceptance path.

5. **Strided / non-prefix edges are not narrowed.** This stencil's edges
   are all contiguous row spans, so all narrow. A 2D-grid
   (`partition=blocks2d`) partition would produce strided column bands
   that stay whole-array (sound, but unnarrowed). **Filed:
   TASK-0455.15** (pack-and-scatter for strided arms).

No wall was hidden by shrinking the case study to fit: the size was
chosen to *meet* the ≥ 100× bar (§1), and the slower-than-baseline
distributed runtime is reported as-is.

---

## 8. File map

```
docs/case-study/
  prog.algo.nuc            # the algorithm: 15360×640 single-pass 3×3 blur
  kernels.rs               # blur3 + load/save (i32, env-var IO)
  schedules/
    naive.sched.nuc        # single host
    distributed.sched.nuc  # host + w0..w3, partition=rows
  reference/               # INDEPENDENT oracle crate (std only, policy §2)
  gen/                     # INDEPENDENT input generator crate (std only)
scripts/case-study-run.sh  # the runner (just case-study)
```
