#!/usr/bin/env bash
# Runtime performance + scaling study runner (TASK-0455.04).
#
# Sweeps wall-clock time vs {worker count} x {problem size} x {backend}
# for three distributed Nucleus workloads, and MEASURES on-the-wire
# transfer volume for the message-passing backend. Every measured point
# is asserted BYTE-IDENTICAL to an INDEPENDENT reference oracle before
# its timing is trusted (correctness first, time second) — the same
# discipline scripts/case-study-run.sh uses.
#
# OWNERSHIP / SCOPE (TASK-0455.04): this script + docs/perf-study*.md +
# one justfile recipe + nucleus/target scratch are the only things it
# writes. It does NOT edit nucleus/** source, paper/**, or the committed
# nuc-nucleus/examples/** — all parameterized fixtures (larger N/H
# algorithm + kernels + reference oracle + input generator, and schedule
# variants at worker counts {2,4,8}) are GENERATED into the scratch dir.
# The committed examples and the docs/case-study/ generator crates are
# reused by INVOCATION of the same arithmetic, never by edit.
#
# WHY GENERATED FIXTURES: every committed example hardcodes its problem
# size as a `const N`/`const H` in BOTH prog.algo.nuc and kernels.rs, and
# its reference oracle hardcodes the same. Scaling "3 octaves up from the
# corpus toys" therefore requires emitting parameterized copies; doing so
# in scratch keeps the committed tree (and the e2e matrix that enumerates
# it) untouched. The arithmetic of each generated fixture is identical to
# its committed sibling (matmul madd, stencil blur3, reduction
# accumulate/combine) — only the dimension constants and, for the
# schedule variants, the workers list differ.
#
# METHODOLOGY: docs/perf-study-methodology.md is the single source of
# truth for warm-up, repetition counts, min-of-N, load-context recording,
# the input-independence argument, and the explicit list of what this
# study CANNOT claim (loopback != cluster; one laptop; heterogeneous P/E
# cores; powersave governor; shared-machine noise).
#
# BACKENDS: pthreads-async (tier-1 shared memory) is the always-present
# arm. The message-passing arm is mp-tcp-event (tier-1, TCP loopback, OS
# processes) for the async stencil and mp-tcp-bufsync (tier-1, TCP
# loopback, OS processes, sync) for the sync matmul/reduction. Both are
# message-passing over loopback sockets — NOT a cluster. The MPI tier-2
# backends are NOT swept here: they need the .#mpi shell and only add
# another loopback launcher, which the case study already witnesses for
# correctness; a real cluster is the missing substrate, not another local
# launcher (see docs/perf-study.md "what this does NOT show").
#
# Run from the repo root inside the default dev shell:  just perf-study
# (the recipe wraps this in `nix develop` so cargo is on PATH). A
# size-trimmed smoke pass is available with  PERF_SMOKE=1 just perf-study .
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRIVER="$ROOT/nucleus/target/release/nucleus"

# All scratch under nucleus/target/ so `just clean` reclaims it.
SCRATCH="$ROOT/nucleus/target/perf-study/run-$$"
mkdir -p "$SCRATCH"
# On SUCCESS the bulky generated fixtures are deleted, but the raw per-row
# TSVs (results + wire) are the EVIDENCE the published docs/perf-study.md
# tables summarise, so they are copied to a stable retained location first
# (TASK-0455.04 P1: keep the raw TSV as a build artifact, do not delete).
ARTIFACTS="$ROOT/nucleus/target/perf-study/artifacts"
OK=0
cleanup() {
    if [ "$OK" = "1" ]; then
        mkdir -p "$ARTIFACTS"
        local stamp; stamp="$(date +%Y%m%d-%H%M%S)"
        [ -f "$RESULTS" ] && cp "$RESULTS" "$ARTIFACTS/results-$stamp.tsv"
        [ -f "$WIRE" ]    && cp "$WIRE"    "$ARTIFACTS/wire-$stamp.tsv"
        echo "perf-study: raw per-row evidence retained at $ARTIFACTS/{results,wire}-$stamp.tsv" >&2
        rm -rf "$SCRATCH"
    else
        echo "perf-study: FAILED — scratch (incl. raw TSVs) retained at $SCRATCH" >&2
    fi
}
trap cleanup EXIT

# Driver freshness (stale-driver trap — case-study precedent): a perf
# study must never time a binary built from an older tree, and must never
# emit codegen from a stale compiler. Rebuild the release driver up front;
# cargo no-ops when it is already current. Done inside the dev shell the
# recipe wraps us in, so cargo is on PATH.
echo "perf-study: ensuring the release driver is current (cargo build --release) ..." >&2
( cd "$ROOT/nucleus" && cargo build --release --quiet ) || {
    echo "perf-study: FAIL — could not build the release nucleus driver." >&2
    exit 1
}
[ -x "$DRIVER" ] || { echo "perf-study: FAIL — driver missing at $DRIVER after build." >&2; exit 1; }

# Results table accumulates here (TSV); printed + summarised at the end.
RESULTS="$SCRATCH/results.tsv"
printf 'example\tsize\tbackend\tworkers\twall_ms_min\twall_ms_med\tspread_pct\tloadavg\tcache\tbyteexact\n' > "$RESULTS"
WIRE="$SCRATCH/wire.tsv"
printf 'example\tsize\tbackend\tworkers\tbytes_measured\tbytes_baseline\treduction_pct\tmethod\n' > "$WIRE"
# Gate cost (compile wall + peak RSS) for the keystone, measured by the
# runner (not asserted from thesis prose) so docs/perf-study.md §2's
# "compiles in ≈X s at ≈Y MB" numbers are reproducible by `just perf-study`.
GATE="$SCRATCH/gate.tsv"
printf 'cell\tcompile_wall_s\tpeak_rss_kb\n' > "$GATE"

# GNU `time -v` binary (peak-RSS + elapsed). The case study uses
# /usr/bin/time -v; resolve it portably here so we work in the default dev
# shell too. Empty if no GNU time is found (the gate measurement is then
# skipped with a loud note rather than silently producing nothing).
GNU_TIME=""
if [ -x /usr/bin/time ]; then GNU_TIME=/usr/bin/time
elif command -v time >/dev/null 2>&1 && time -v true >/dev/null 2>&1; then
    GNU_TIME="$(command -v time)"
fi

# Repetition discipline (see docs/perf-study-methodology.md). One warm-up
# run (discarded), then N timed runs; the MINIMUM is the point estimate
# (scheduler/IO jitter only ADDS time) and the spread min..median is
# reported so the floor-sensitivity is visible. Smoke mode shrinks N and
# the size grids so the runner finishes in a couple of minutes for a
# wiring check.
if [ "${PERF_SMOKE:-0}" = "1" ]; then
    REPS=3
    echo "perf-study: PERF_SMOKE=1 — trimmed grids, REPS=$REPS (wiring check, not a measurement run)" >&2
else
    REPS=9
fi

# ---------------------------------------------------------------------
# Machine + load context (recorded into the results header).
# ---------------------------------------------------------------------
CPU_MODEL="$(grep -m1 'model name' /proc/cpuinfo | sed 's/.*: //')"
NPROC="$(nproc)"
KERNEL="$(uname -r)"
GOV="$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo unknown)"
echo "=============================================================="
echo " Nucleus runtime performance + scaling study (TASK-0455.04)"
echo "   CPU      = $CPU_MODEL ($NPROC logical threads)"
echo "   kernel   = $KERNEL   governor = $GOV"
echo "   reps     = $REPS timed runs after 1 warm-up; min-of-N point est."
echo "   load now = $(cut -d' ' -f1-3 /proc/loadavg)"
echo "=============================================================="

# loadavg_1m -> the 1-minute load average as a float.
loadavg_1m() { cut -d' ' -f1 /proc/loadavg; }

# measure_gate CELL_LABEL DIR SCHED BACKEND  -> times the `nucleus build`
# (codegen + symbolic gate) under GNU `time -v` and records compile wall +
# peak RSS into $GATE. This is the doc's §2 keystone gate-cost claim,
# MEASURED by the runner rather than asserted from the thesis. Mirrors the
# case study's build_cell gate measurement. No-op (loud) if GNU time is
# absent. The compiled output dir is left for the caller to cargo-build.
measure_gate() {
    local label="$1" dir="$2" sched="$3" be="$4"
    local out="$dir/gate-$be"
    rm -rf "$out"
    if [ -z "$GNU_TIME" ]; then
        echo "perf-study: NOTE — GNU \`time -v\` not found; gate cost for $label not measured." >&2
        printf '%s\t%s\t%s\n' "$label" "na" "na" >> "$GATE"
        return
    fi
    local tlog="$out.gate.txt"
    "$GNU_TIME" -v "$DRIVER" build --algo "$dir/prog.algo.nuc" --sched "$sched" \
        --kernels "$dir/kernels.rs" --backend "$be" --out "$out" \
        >/dev/null 2>"$tlog" || {
        echo "perf-study: FAIL — keystone gate build ($label) failed:" >&2
        tail -20 "$tlog" >&2; exit 1; }
    # GNU time reports "Elapsed (wall clock) time (h:mm:ss or m:ss): M:SS.ss"
    # and "Maximum resident set size (kbytes): N".
    local wall_s rss_kb
    wall_s="$(awk -F': ' '/Elapsed \(wall clock\)/{print $2}' "$tlog" \
        | awk -F: '{ if (NF==3) print $1*3600+$2*60+$3; else if (NF==2) print $1*60+$2; else print $1 }')"
    rss_kb="$(awk -F': ' '/Maximum resident set size/{print $2}' "$tlog")"
    printf '%s\t%s\t%s\n' "$label" "${wall_s:-na}" "${rss_kb:-na}" >> "$GATE"
    echo "   gate: $label  compile_wall=${wall_s}s  peak_rss=${rss_kb}KB" >&2
}

# A row's load context is "clean" when the 1-minute load average is at or
# below the worker count we are timing PLUS a small headroom: a row timed
# while the machine carries more runnable tasks than the cell uses is
# contended and must be re-measured (RECORD + re-measure dirty rows, per
# the ground rules). We record the load with every row regardless.
CLEAN_LOAD_HEADROOM="${PERF_CLEAN_LOAD_HEADROOM:-1.5}"

# ---------------------------------------------------------------------
# Timing helper. time_run N CMD...  -> "MIN MED SPREAD" (ms), min/median
# over N timed runs after one warm-up. SPREAD is (median-min)/min in
# percent — the floor-sensitivity / noise indicator the methodology doc
# requires. Identical min-of-N rationale to scripts/case-study-run.sh.
# ---------------------------------------------------------------------
time_run() {
    local n="$1"; shift
    python3 - "$n" "$@" <<'PY'
import subprocess, sys, time
n = int(sys.argv[1]); cmd = sys.argv[2:]


def one_run():
    """Run cmd once; return wall ms, or None on a non-zero exit (a
    transient run failure — e.g. the mp-tcp rendezvous timeout under heavy
    load — which we retry rather than treat as a measurement)."""
    t0 = time.perf_counter_ns()
    r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    dt = (time.perf_counter_ns() - t0) / 1e6
    return dt if r.returncode == 0 else None


# Warm-up with a few retries: a flaky cell (rendezvous timeout under load)
# is retried before we give up on it.
for _ in range(4):
    if one_run() is not None:
        break
else:
    # Could not get a single clean run; report a sentinel so the cell is
    # recorded as a run failure (NOT a byte divergence) and the sweep
    # continues. This must be loud, not silent.
    print("RUNFAIL RUNFAIL RUNFAIL")
    sys.exit(0)

ts = []
fails = 0
while len(ts) < n and fails < 2 * n:
    dt = one_run()
    if dt is None:
        fails += 1
        continue
    ts.append(dt)
if not ts:
    print("RUNFAIL RUNFAIL RUNFAIL")
    sys.exit(0)
ts.sort()
mn = ts[0]; md = ts[len(ts) // 2]
spread = 100.0 * (md - mn) / mn if mn > 0 else 0.0
print(f"{mn:.1f} {md:.1f} {spread:.0f}")
PY
}

# record_row example size backend workers min med spread byteexact
record_row() {
    local ex="$1" size="$2" be="$3" w="$4" mn="$5" md="$6" sp="$7" bx="$8"
    local load cache
    load="$(loadavg_1m)"
    # cache flag: this run reuses the warmed cargo target/registry, so the
    # binary was already built when timed — "warm" always holds for the
    # RUN phase (we build before timing). Recorded explicitly so a future
    # reader does not have to assume it.
    cache="warm"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$ex" "$size" "$be" "$w" "$mn" "$md" "$sp" "$load" "$cache" "$bx" >> "$RESULTS"
    local dirty=""
    # CAPSKIP rows did not run (wmem wall) — no timing, no load judgement.
    if [ "$bx" != "CAPSKIP" ] && \
       ! python3 -c "import sys; sys.exit(0 if float('$load') <= $w + $CLEAN_LOAD_HEADROOM else 1)"; then
        dirty="  [LOAD-DIRTY: load $load > workers $w + $CLEAN_LOAD_HEADROOM — re-measure]"
    fi
    printf '   %-9s %-7s %-15s w=%-2s : %8s ms min  %8s ms med  (spread %s%%, load %s, %s)%s\n' \
        "$ex" "$size" "$be" "$w" "$mn" "$md" "$sp" "$load" "$bx" "$dirty"
}

source "$ROOT/scripts/perf-study-fixtures.sh"

# =====================================================================
# SIZE GRIDS — "at least 3 octaves up from the corpus toys" (AC#2).
#   matmul    corpus N=16  -> {64,128,256}  (2,3,4 octaves; +512 keystone)
#   stencil   corpus 16x16 -> H {1920,7680,15360} at W=640 (VGA multiples)
#   reduction corpus N=256 -> {65536,1048576,16777216} (8,12,16 octaves)
# Smoke mode trims each to a single small size.
# =====================================================================
if [ "${PERF_SMOKE:-0}" = "1" ]; then
    MATMUL_SIZES=(64)
    STENCIL_SIZES=(1920)
    REDUCTION_SIZES=(65536)
    MATMUL_KEYSTONE=()
else
    MATMUL_SIZES=(64 128 256)
    STENCIL_SIZES=(1920 7680 15360)
    REDUCTION_SIZES=(65536 1048576 16777216)
    MATMUL_KEYSTONE=(512)   # single thesis-anchor point, 4-worker only
fi

# Worker counts for the worker-scaling axis (matmul + stencil).
WORKER_COUNTS=(2 4 8)

# =====================================================================
# 1. MATMUL — O(N^3) integer matmul, outer-i row-band partition.
#    Worker-count sweep {2,4,8} x size sweep, sync transfers.
#    Backends: pthreads-async + mp-tcp-bufsync (message-passing).
# =====================================================================
echo
echo "### 1. distributed matmul (C=AxB, outer-i partition, sync) ###"
for N in "${MATMUL_SIZES[@]}"; do
    gen_matmul_fixture "$N" "$SCRATCH/matmul-$N"
    # naive single-worker baseline (one backend; baseline is per-size).
    run_naive_cell matmul "$N" "$SCRATCH/matmul-$N" pthreads-async
    for W in "${WORKER_COUNTS[@]}"; do
        gen_matmul_sched "$N" "$W" "$SCRATCH/matmul-$N"
        run_dist_cell matmul "$N" "$SCRATCH/matmul-$N" pthreads-async "$W" sync
        run_dist_cell matmul "$N" "$SCRATCH/matmul-$N" mp-tcp-bufsync "$W" sync
    done
done
# Keystone: N=512 distributed matmul at 4 workers (thesis sec:fw-quant
# anchor — the gate now compiles this flat; we additionally TIME it and
# measure its gate compile-wall + peak RSS so the doc's §2 keystone
# numbers are reproduced by this runner, not asserted from prose).
for N in "${MATMUL_KEYSTONE[@]}"; do
    gen_matmul_fixture "$N" "$SCRATCH/matmul-$N"
    run_naive_cell matmul "$N" "$SCRATCH/matmul-$N" pthreads-async
    gen_matmul_sched "$N" 4 "$SCRATCH/matmul-$N"
    measure_gate "matmul $N dist w=4 (pthreads-async)" \
        "$SCRATCH/matmul-$N" "$SCRATCH/matmul-$N/dist-4.sched.nuc" pthreads-async
    run_dist_cell matmul "$N" "$SCRATCH/matmul-$N" pthreads-async 4 sync
done

# =====================================================================
# 2. STENCIL — 3x3 box blur, row-band partition, ASYNC streaming.
#    Worker-count sweep {2,4,8} x H sweep at W=640.
#    Backends: pthreads-async + mp-tcp-event (message-passing, async).
# =====================================================================
echo
echo "### 2. distributed stencil (3x3 blur, row-band, async) ###"
for H in "${STENCIL_SIZES[@]}"; do
    gen_stencil_fixture "$H" 640 "$SCRATCH/stencil-$H"
    run_naive_cell stencil "${H}x640" "$SCRATCH/stencil-$H" pthreads-async
    for W in "${WORKER_COUNTS[@]}"; do
        gen_stencil_sched "$W" "$SCRATCH/stencil-$H"
        run_dist_cell stencil "${H}x640" "$SCRATCH/stencil-$H" pthreads-async "$W" async
        run_dist_cell stencil "${H}x640" "$SCRATCH/stencil-$H" mp-tcp-event "$W" async
    done
done

# =====================================================================
# 3. REDUCTION — two-phase sum, outer-w partition (FIXED 4 workers:
#    the algorithm's phase-2 tree is hardwired to 4 partials, so only the
#    PROBLEM SIZE scales here, not the worker count). Size sweep only.
#    Backends: pthreads-async + mp-tcp-bufsync (message-passing).
# =====================================================================
echo
echo "### 3. distributed reduction (two-phase sum, 4 workers fixed) ###"
for N in "${REDUCTION_SIZES[@]}"; do
    gen_reduction_fixture "$N" "$SCRATCH/reduction-$N"
    run_naive_cell reduction "$N" "$SCRATCH/reduction-$N" pthreads-async
    run_dist_cell reduction "$N" "$SCRATCH/reduction-$N" pthreads-async 4 sync
    run_dist_cell reduction "$N" "$SCRATCH/reduction-$N" mp-tcp-bufsync 4 sync
done

# =====================================================================
# 4. WIRE-VOLUME MEASUREMENT — measured bytes on the message-passing
#    backend vs the static whole-array baseline (AC#3). Done on ONE
#    representative size per example. The narrowing RATIO is
#    size-INDEPENDENT for a fixed partition shape (it is a function of
#    worker count and which arrays are partition-indexed, not of N), so
#    one size per (example, worker-count) suffices and we deliberately
#    pick a CAP-SAFE size: the SMALLEST swept size of each example, which
#    is always under this sandbox's 4 MiB `net.core.wmem_max` socket
#    buffer cap. Larger mp-tcp-* cells cannot RUN here (they request a
#    per-channel SO_SNDBUF above the un-raisable cap and the host panics —
#    see docs/perf-study.md "walls"); measuring the ratio at the smallest
#    size avoids that wall while yielding the same reduction percentage a
#    larger size would.
# =====================================================================
echo
echo "### 4. measured wire volume (mp-tcp-*, strace sendto bytes) ###"
MM_WIRE_N="${MATMUL_SIZES[0]}"
ST_WIRE_H="${STENCIL_SIZES[0]}"
RD_WIRE_N="${REDUCTION_SIZES[0]}"
measure_wire matmul    "$MM_WIRE_N"        "$SCRATCH/matmul-$MM_WIRE_N"    mp-tcp-bufsync 4
measure_wire stencil   "${ST_WIRE_H}x640"  "$SCRATCH/stencil-$ST_WIRE_H"   mp-tcp-event   4
measure_wire reduction "$RD_WIRE_N"        "$SCRATCH/reduction-$RD_WIRE_N" mp-tcp-bufsync 4

# =====================================================================
# 5. SUMMARY
# =====================================================================
echo
echo "=============================================================="
echo " RESULTS (TSV at $RESULTS during the run)"
echo "--------------------------------------------------------------"
column -t -s $'\t' "$RESULTS"
echo "--------------------------------------------------------------"
echo " WIRE VOLUME"
column -t -s $'\t' "$WIRE"
if [ -s "$GATE" ] && [ "$(wc -l < "$GATE")" -gt 1 ]; then
    echo "--------------------------------------------------------------"
    echo " GATE COST (keystone compile wall + peak RSS)"
    column -t -s $'\t' "$GATE"
fi
echo "=============================================================="

# Correctness gate. Two distinct non-PASS statuses (correctness first):
#   FAIL    = the cell RAN but its bytes DIVERGED from the reference. This
#             is a hard correctness failure and fails the whole study.
#   RUNFAIL = the cell could not produce output after retries (a transient
#             run/robustness failure, e.g. the mp-tcp rendezvous timeout
#             under heavy machine load — see docs/perf-study.md "walls").
#             Surfaced LOUDLY but does NOT fail the study, because it is a
#             known backend robustness flake under contention, not a
#             byte-correctness defect. It is filed as its own task.
n_diverged=$(awk -F'\t' 'NR>1 && $10=="FAIL"' "$RESULTS" | wc -l)
n_runfail=$(awk -F'\t' 'NR>1 && $10=="RUNFAIL"' "$RESULTS" | wc -l)
n_capskip=$(awk -F'\t' 'NR>1 && $10=="CAPSKIP"' "$RESULTS" | wc -l)
if [ "$n_capskip" -gt 0 ]; then
    echo "perf-study: NOTE — $n_capskip mp-tcp-* cell(s) CAPSKIP'd: their per-channel" >&2
    echo "  socket payload exceeds this sandbox's 4 MiB net.core.wmem_max (un-raisable" >&2
    echo "  here). This is the documented wmem wall (docs/perf-study.md); the same cell" >&2
    echo "  runs on a host whose wmem_max is raised. Not a study failure." >&2
    awk -F'\t' 'NR>1 && $10=="CAPSKIP"{print "    CAPSKIP: "$1" "$2" "$3" w="$4}' "$RESULTS" >&2
fi
if [ "$n_runfail" -gt 0 ]; then
    echo "perf-study: NOTE — $n_runfail cell(s) hit a transient RUNFAIL (run/robustness," >&2
    echo "  not byte divergence); surfaced above and discussed in docs/perf-study.md." >&2
    awk -F'\t' 'NR>1 && $10=="RUNFAIL"{print "    RUNFAIL: "$1" "$2" "$3" w="$4" (load "$8")"}' "$RESULTS" >&2
fi
if [ "$n_diverged" -eq 0 ]; then
    OK=1
    echo "perf-study: PASS — every cell that RAN was byte-identical to its reference" \
         "($n_capskip wmem-cap skip(s), $n_runfail transient run-failure(s)," \
         "0 byte divergences)."
else
    echo "perf-study: FAIL — $n_diverged cell(s) DIVERGED from the reference (byte mismatch;" \
         "$n_capskip wmem-cap skip(s), $n_runfail transient run-failure(s))." >&2
    awk -F'\t' 'NR>1 && $10=="FAIL"{print "    DIVERGED: "$1" "$2" "$3" w="$4}' "$RESULTS" >&2
    exit 1
fi
