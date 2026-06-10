# Runtime performance + scaling study — results

This is the results writeup for TASK-0455.04: how the emitted programs'
wall-clock time varies with **worker count**, **problem size**, and
**backend** for three distributed Nucleus workloads, plus the **measured**
on-the-wire transfer-volume reduction from TASK-0453.22. It is the
empirical companion the thesis defers in `sec:fw-quant`.

**Measured:** 2026-06-11, repo at commit `2363fae` (+uncommitted study
scripts). Every table below is transcribed PROGRAMMATICALLY from one
retained artifact pair —
`nucleus/target/perf-study/artifacts/{results,wire}-20260611-001452.tsv`
— produced by a single green `just perf-study` pass (0 byte
divergences, 6 CAPSKIP). The matmul grid ran genuinely quiet (load
0.86–1.33); the machine picked up background load during the stencil
and reduction grids (2.1–4.7), and rows that exceed the runner's own
dirty threshold (load > workers+1.5) are flagged `*` in the tables and
weighed accordingly in the prose. Earlier measurement passes (whose
published figures could not be traced to a retained artifact — the
re-review finding that forced this rewrite) are superseded entirely by
this artifact.

**The methodology — machine spec, warm-up, repetition counts, min-of-N,
load recording, the input-independence argument, and the explicit list of
what this study cannot claim — is `docs/perf-study-methodology.md`. Read
it first; this document does not restate it.**

**What `just perf-study` reproduces, and what it does not.** A single
`just perf-study` (runner: `scripts/perf-study-run.sh`) reproduces, in one
pass: the full wall-clock grid of §2–§4 (every naive/distributed/mp-tcp
cell, min + median + spread + per-row load, asserted byte-exact first),
the §5 measured wire table **and its per-size send histogram**, the §6
CAPSKIP enumeration, and the §2 keystone **gate compile-wall + peak RSS**
(`time -v`). Two classes of number are **not** single-pass outputs and are
labelled where they appear: (a) the cross-doc reconciliation figures in
§3, which quote `docs/case-study.md` (a different runner, different shell,
different window); and (b) the run-to-run *absolute* wall-times, which
drift with machine load and core placement — the runner reproduces the
*shape* (ratios, crossover, ordering) deterministically, not the exact
millisecond, and the retained per-row TSV under
`nucleus/target/perf-study/artifacts/` is the evidence each published
figure summarises. This study does **not** measure worker RSS (§6.3,
methodology §7.5).

Headline, stated up front and bounded: **distribution pays on loopback
for the compute-bound $O(N^3)$ matmul — break-even at $N=64$, ≈1.4× at
$N=128$, ≈1.9× at $N=256$, ≈2.3× at $N=512$ (all w=4, shared-memory
workers) — and does not pay for the bandwidth-bound stencil and
reduction at any measured size; the message-passing cells carry a
roughly fixed ≈20 ms-per-worker process-spawn + TCP-setup overhead, so
their slowdown vs the naive baseline spans ≈2.2× (large cell, few
workers) to ≈29× (tiny cell, w=8) and amortises as compute grows.**
None of this is a cluster claim — it is all loopback on one laptop
(methodology §7).

---

## 1. The measured matrix

Three distributed workloads, each a parameterized copy of a committed
corpus example (same arithmetic, larger size), swept ≥ 3 octaves up from
the corpus toys:

| workload | algorithm | corpus toy | sizes swept | partition | worker counts |
| -------- | --------- | ---------- | ----------- | --------- | ------------- |
| matmul   | $C = A\times B$, $O(N^3)$ integer | $N{=}16$ | $N \in \{64,128,256\}$ (+512 keystone) | outer-$i$ row band | 2, 4, 8 |
| stencil  | 3×3 box blur, $O(N)$ integer | 16×16 | $H \in \{1920,7680,15360\}$ at $W{=}640$ | row band + halo | 2, 4, 8 |
| reduction| two-phase sum, $O(N)$ integer | $N{=}256$ | $N \in \{65536,\,2^{20},\,2^{24}\}$ | outer-$w$ band | 4 (fixed) |

(Reduction's worker count is fixed at 4 because its algorithm hardwires a
4-partial phase-2 tree; only its problem size scales. The worker-count
*axis* is carried by matmul and stencil.)

Backends: **`pthreads-async`** (tier-1, shared memory) is the always-present
arm; the **message-passing** arm is **`mp-tcp-bufsync`** (sync, for matmul
and reduction) or **`mp-tcp-event`** (async + buffered, for the async
stencil schedule) — both are OS processes over loopback TCP. Each size also
has a **single-worker naive baseline**. **Every cell that ran was
byte-identical to an independent generated reference oracle** (the runner's
hard gate; 0 byte divergences across the whole sweep).

All numbers below are **min-of-9 wall-clock ms after one warm-up**, with the
median and a spread figure `(median−min)/min`. The 1-minute load average at
the moment each row was timed is recorded; this machine is the maintainer's
interactive workstation. The runner flags a row dirty when
the 1-min load exceeds `workers + 1.5`. In the published artifact the
matmul grid is fully clean (load 0.86–1.33); the stencil and reduction
grids picked up background load mid-run (2.1–4.7), and their dirty rows
are flagged `*` in the tables rather than silently re-published — the
direction of those findings is load-insensitive (consistent across
every pass ever taken), the magnitudes carry the noted load. The
naive and distributed rows of each table were taken **back-to-back in
the same load window** to keep the
*ratios* valid even though the heterogeneous P/E-core + powersave absolute
wall-times still drift run-to-run (the spread column is the in-band
indicator).

---

## 2. Matmul — the crossover, and where distribution pays

This is the load-bearing scaling result. `pthreads-async`, transcribed
from `results-20260611-001452.tsv` (min/median ms, spread %, 1-min load;
all matmul rows clean — load 0.86–1.33, threshold ≥ workers+1.5):

| $N$ | naive w=1 | dist w=2 | dist w=4 | dist w=8 | best speedup (min/min) |
| --- | --------- | -------- | -------- | -------- | ---------------------- |
| 64  | **3.0**/3.1 (5%) | 3.1/3.4 (10%) | 3.0/3.2 (7%) | 2.7/3.4 (27%) | 1.11× (w=8 — within spread; no real win) |
| 128 | **5.3**/5.5 (4%) | 4.0/4.5 (10%) | 3.8/4.0 (4%) | 3.8/4.2 (10%) | **1.39×** (w=4) |
| 256 | **16.4**/18.6 (14%) | 12.5/13.3 (6%) | 8.6/10.3 (20%) | 9.9/10.3 (4%) | **1.91×** (w=4) |
| 512 | **104.2**/111.5 (7%) | — | 45.0/53.2 (18%) | — | **2.32×** (w=4) |

**The crossover from this clean artifact: $N=64$ is break-even (the
1.11× at w=8 sits inside that row's 27% spread), distribution pays from
$N=128$ (1.39×) and grows monotonically — 1.91× at $N=256$, 2.32× at
$N=512$.** The best worker count in this pass is **4 at every size that
pays** (an earlier, dirtier pass read w=8 best at $N=256$; this clean
artifact does not reproduce that, and the w=4-vs-w=8 gap at 256 — 8.6 vs
9.9 ms — is real but modest). The speedup band on this 10-physical-core
heterogeneous laptop is ≈1.4–2.3× and does not scale linearly with
workers.

Absolute wall-times drift run-to-run on this machine (heterogeneous
P/E cores, powersave governor); the ratios within this single
back-to-back pass are the robust quantity, and the pass behind every
number above IS the retained artifact named in §0 — re-running
`just perf-study` reproduces the shape (ordering, crossover, ratios
within spread), not the exact millisecond.

### The N=512 gate keystone (thesis `sec:fw-quant` anchor)

The $N=512$ four-worker distributed matmul — the exact shape the thesis
cites — **compiles in ≈0.03 s at ≈88 MB compiler RSS** (measured by the
runner via `time -v`: compile wall ≈0.03 s, peak RSS ≈88 200 KB across
runs — the runner prints the exact figure each pass), flat with the toy
size, because the symbolic soundness gate (TASK-0455.01) proves its
single-shot matched-pair net bounded without expanding it. (The thesis
quotes ≈7.2 MB for the gate's *own* footprint; the ≈88 MB is dominated by
the driver process, as the case study also records.) That this large
distributed decomposition compiles at all — where the expanded replay would
project over a hundred gigabytes (`sec:fw-quant`) — is what made *timing*
it possible; this study is the timing the gate-lift unblocked.

---

## 3. Stencil and reduction — distribution does not pay (bandwidth-bound)

Both workloads do a handful of integer ops per element and are
**bandwidth/coordination-bound, not compute-bound**, so the
decomposition's overhead is never amortised on loopback. Transcribed
from `results-20260611-001452.tsv`; the machine picked up background
load during these grids (2.1–4.7), and rows over the dirty threshold
(load > workers+1.5) carry `*` — the per-row direction below is
consistent with every prior pass including fully-clean ones, so the
dirt affects magnitudes, not the finding.

**Stencil** (`pthreads-async`, min/median ms (spread %), `*` = load-dirty):

| size (H×640) | naive w=1 | dist w=2 | dist w=4 | dist w=8 |
| ------------ | --------- | -------- | -------- | -------- |
| 1920  | **15.6**/16.7 (7%) | 21.5/23.6 (10%) | 27.0/33.3 (23%) | 18.8/23.5 (25%) |
| 7680  | **73.9**/80.8 (9%)\* | 111.5/121.7 (9%) | 85.8/89.1 (4%) | 92.0/128.5 (40%) |
| 15360 | **109.0**/118.5 (9%)\* | 163.6/175.2 (7%)\* | 149.9/165.0 (10%) | 138.8/149.7 (8%) |

Distribution is **slower at every stencil size** — the same direction
the production case study reports at 15360×640. The nine-integer-op
blur is too cheap to amortise per-worker coordination and the
full-shaped per-worker buffer allocation (the memory-footprint
residual, methodology §7.5).

**Reduction** (min/median ms (spread %); these rows ran at load
4.4–4.6, so all single-worker baselines are flagged `*`):

| $N$ | naive w=1 | dist w=4 (pthreads) | dist w=4 (mp-tcp-bufsync) |
| --- | --------- | ------------------- | ------------------------- |
| 65 536     | **3.3**/3.4 (2%)\* | 4.0/4.4 (10%) | 51.3/52.1 (2%) |
| 1 048 576  | **10.3**/11.1 (8%)\* | 14.0/15.6 (12%) | 64.0/68.3 (7%) |
| 16 777 216 | **81.0**/85.8 (6%)\* | 122.1/128.0 (5%) | CAPSKIP (§6) |

Reduction distribution **does not pay either, and gets worse with
size** (at $N=2^{24}$: 122.1 ms distributed vs 81.0 ms naive, both
under comparable load): a sum is one streaming pass over memory, and
splitting it adds a scatter + gather the trivial arithmetic cannot
offset. Note the dirty flags: the naive baselines here ran under
sustained load, which *inflates* them — i.e. the true distributed
penalty is, if anything, larger than these rows show.

**Conclusion across all three:** distribution paying is **specific to
the compute-bound matmul at $N \ge 128$**. For cheap-compute
bandwidth-bound kernels it does not pay on loopback at any size
measured — the honest, generalised form of the case study's single
slower-than-baseline data point.

### Cross-doc reconciliation (this study vs `docs/case-study.md`)

The two docs overlap at one workload, the 15360×640 single-pass
stencil:

| 15360×640 stencil cell | `docs/case-study.md` §5 | this artifact (§3) |
| ---------------------- | ----------------------- | ------------------ |
| naive / pthreads-async | ≈114 ms | 109.0 ms\* |
| distributed w=4 / pthreads-async | 182 ms | 149.9 ms |

Direction agrees (distributed slower than naive in both); magnitudes
differ within the documented drift (different shells, windows, loads —
and this artifact's naive row is load-dirty). The docs are bounded
against each other, not reconciled to the millisecond.

## 4. The message-passing backends: a fixed per-worker overhead that amortises (loopback)

Every `mp-tcp-*` cell is slower than its naive shared-memory baseline.
From `results-20260611-001452.tsv` (matmul, `mp-tcp-bufsync`, vs the
same-size naive min; mp-tcp timed via the **cargo-stripped**
`run-timed.sh`, so no `cargo build` no-op enters the sample — the P1
fix; all rows clean, load 0.86–1.33):

| $N$ | w=2 | w=4 | w=8 |
| --- | --- | --- | --- |
| 64  | 23.1 ms = 7.7× | 45.0 ms = 15.0× | 86.1 ms = 28.7× |
| 128 | 24.5 ms = 4.6× | 45.2 ms = 8.5×  | 86.3 ms = 16.3× |
| 256 | 35.6 ms = 2.2× | 51.8 ms = 3.2×  | 91.3 ms = 5.6×  |

The structure is plain in the rows: the mp-tcp wall is **approximately
a fixed ≈20–22 ms per worker** (w=2 ≈23 ms, w=4 ≈45 ms, w=8 ≈86 ms,
nearly independent of $N$ until $N=256$ where real payloads start to
add) — process spawn + per-channel TCP setup + loopback copies, paid
afresh every run. The ratio band is therefore **not one number**: it
spans ≈2.2× (largest cell, fewest workers) to ≈28.7× (tiny cell, most
workers) and shrinks as compute grows. The earlier published "≈5–7×
band" and before it "5–25×" were artifacts of contaminated and dirty
passes respectively; this table supersedes both.

It is the same effect the case study sees with its MPI cell (≈1.5 s vs
≈114 ms naive). The message-passing backends witness **correctness and
deadlock-immunity at scale** (every cell that ran was byte-exact),
**not** competitive single-host performance — and on a real cluster the
trade-off is entirely different, which is why no cluster claim is made
(methodology §7.1).

## 5. Measured wire-volume reduction (TASK-0453.22)

The narrowed transfer is **measured in real bytes on the wire** — summed
`sendto`/`sendmsg` payload bytes from an `strace` of the worker processes
(method + the rejected `/proc/io` route: methodology §5) — and compared
against the statically-computed whole-array baseline (`edges × workers ×
array_bytes`, the pre-narrowing behaviour). One cap-safe size per example
(the ratio is size-independent for a fixed partition shape):

| workload  | size      | backend        | w | **measured data on wire** | whole-array baseline | **reduction** |
| --------- | --------- | -------------- | - | ------------------------- | -------------------- | ------------- |
| matmul    | 64        | mp-tcp-bufsync | 4 | **98 304 B**              | 196 608 B            | **50.0 %**    |
| stencil   | 1920×640  | mp-tcp-event   | 4 | **9 840 768 B**           | 39 321 600 B         | **75.0 %**    |
| reduction | 65 536    | mp-tcp-bufsync | 4 | **262 144 B**             | 1 048 640 B          | **75.0 %**    |

Decoded (matmul, exactly): the 98 304 measured data bytes are `a` scatter
(4 workers × one $i$-band) + `c` gather (4 × one band) — both **narrowed** —
plus `b` broadcast (4 × the **whole** matrix), which stays whole-array
because `b` is indexed `[k][j]`, not by the partition variable $i$. So
matmul narrows to **50 %** (two of its three arrays band-narrowed, one
unavoidably whole), while stencil and reduction — whose every distributed
array *is* partition-indexed — narrow the full **75 %** a 4-way band gives.
These are the same percentages the case study reports for its stencil
(75 %), now **measured on the wire** rather than extracted statically, and
extended to two more workloads.

The numbers are exact and reproducible — the runner prints the per-size
send histogram for each wire cell. For matmul it is 8×4096 B band payloads
+ 4×16384 B whole-array `b` + 28×16 B control frames = **98 304 B data +
448 B control**. Every send of **≤ 16 B** (length prefixes, sync
handshakes — they happen to be 4 B and 16 B here) is classified **control
and excluded from the measured-data total**; only sends above 16 B count
as array data and enter the reduction ratio. Control framing is reported
separately and never folded into the data reduction, so the 50 %/75 %
figures are data-payload reductions, not data+framing.

---

## 6. Walls hit, filed as tasks

Per the case-study discipline, every wall is named here, not worked around
silently.

1. **The OS socket-buffer cap (`net.core.wmem_max` = 4 MiB) blocks large
   message-passing cells on this sandbox.** The `mp-tcp-*` backends
   `setsockopt(SO_SNDBUF)` to the largest per-channel payload; above 4 MiB
   the host panics (a precise, fail-loud diagnostic) and the cell cannot
   run *here* — the cap is un-raisable in this sandbox (memory:
   `env-netns-sysctl-limits`). The runner reads the cell's own emitted
   `export NUC_SO_BUF=<N>` line (the backend's authoritative socket-buffer
   request — the single source of truth, not a hand-derived estimate),
   compares it to the live cap, and records **CAPSKIP** deterministically
   rather than letting the cell flake. The **6 affected cells** are:

   - stencil 7680×640 at **w=2, w=4** (the w=8 band drops under the cap and
     *does* run);
   - stencil 15360×640 at **w=2, w=4, *and* w=8** — at this height even the
     8-way band's per-channel payload (emitted `NUC_SO_BUF=4 920 320` B)
     exceeds the 4 MiB cap, so 15360 is capped at **every** worker count;
   - reduction $2^{24}$ at w=4.

   The W-dependence is therefore size-dependent, not a flat "w≤4" rule: the
   cap bites whenever the largest *per-channel band* — which shrinks as
   workers rise but grows with problem size — clears 4 MiB. It is an
   *environment* wall, not a backend defect: the identical cell runs on a
   host whose `wmem_max` is raised. **Filed: TASK-0455.04.01** (sandbox
   wmem cap blocks large-payload mp-tcp scaling).

2. **Loop-carried / pipelined distributed shapes are not swept.** This
   study sweeps only the **single-shot matched-pair** distributed class the
   symbolic gate carries flat in $N$ (matmul/stencil/reduction distributed
   nets are all loop-depth-0). The loop-carried (`16-jacobi` halo-per-step)
   and pipelined (`09-producer-consumer`) shapes still expand the gate net
   per iteration and OOM at realistic counts — the open keystone the thesis
   records in `sec:fw-quant`. A single demonstration of that wall: building
   the 4-worker `16-jacobi/distributed` at a realistic iteration count is
   the expanded-replay path, not the symbolic one, and is the third-regime
   residual the gate does not yet reach. They are therefore deliberately
   excluded; this is the same honest scoping the case study makes. **No new
   task — this is the standing `sec:fw-quant` coupling and S7 =
   TASK-0341.02.01.08.**

3. **Per-worker memory is still full-shaped.** Wire volume narrows (§5) but
   every receiver still allocates the whole array and pastes its band in,
   so the distributed cells do not reduce memory pressure — a contributor to
   why distribution does not pay for the bandwidth-bound kernels (§3). This
   study measures wire bytes and wall time, **not** worker RSS. **Filed:
   TASK-0455.14** (band-shaped per-worker allocation).

---

## 7. What this study does NOT show

(The full list is methodology §7; the load-bearing ones, repeated here so
no claim is read out of context.)

- **It is not a cluster study.** All message-passing numbers are loopback
  on one host: no network latency, no real bandwidth limit, shared memory
  bandwidth a cluster would not have. **Nothing here transfers to cluster
  performance**, and the MPI tier-2 backends are deliberately not swept —
  another local launcher would not change the substrate; only real
  multi-node hardware would, and there is none here. The thesis keeps the
  cluster caveat; this study reproduces it.
- **One laptop, one config, heterogeneous P/E cores, unpinned (powersave +
  turbo) frequency.** Absolute wall-times carry a frequency-scaling and
  core-placement uncertainty the min-of-9 estimate reduces but does not
  remove. This is consistent with the worker-scaling not improving cleanly
  past ≈4 workers and with the run-to-run absolute drift §2 flags — but the
  cause (which worker landed on a P-core vs an E-core in a given run) was
  **not measured**, so it is offered as the plausible mechanism, not a
  demonstrated one.
- **Shared-machine noise.** The host ran other work during measurement;
  load is recorded per row and dirty rows are flagged `*` in the
  published tables (matmul fully clean at 0.86–1.33; stencil/reduction
  carried 2.1–4.7). Each table's naive and distributed rows were taken
  back-to-back in one window so within-table ratios are not
  cross-contaminated by load drift; flagged magnitudes carry the load.
- **The compute is cheap integer arithmetic** with **input-independent
  runtime** (verified: no data-dependent branches — methodology §6). The
  crossover findings are specific to such cheap-compute kernels and should
  not be generalised to "distribution rarely pays."
- **No runtime number bears on the correctness claims.** A slow but
  byte-identical program falsifies nothing; correctness was the hard gate
  (0 byte divergences across the whole sweep), performance the secondary
  observation.
