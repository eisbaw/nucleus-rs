#!/usr/bin/env bash
# Walk the getting-started tutorial (docs/tutorial.md) end-to-end so it
# cannot rot silently (TASK-0455.06). Invoked by `just tutorial`.
#
# Steps, for TWO backends running the SAME algorithm under two different
# schedules:
#   1. nucleus build  docs/tutorial/prog.algo.nuc + naive.sched.nuc
#                      -> pthreads-sync   (single-binary, shared memory)
#   2. nucleus build  docs/tutorial/prog.algo.nuc + split.sched.nuc
#                      -> mp-tcp-bufsync  (two OS processes over TCP)
#   3. Run each generated program against the SAME input.bin.
#   4. Assert the two output.bin files are byte-identical to each other.
#
# That byte-identity across two radically different decompositions is
# the project's headline correctness property. If the tutorial program
# stops compiling, or the two backends diverge, this script exits
# non-zero and `just tutorial` fails.
#
# Run from the repo root (the `just` recipe does `cd nucleus` itself for
# the `cargo run` invocations, mirroring the e2e recipe convention).
set -euo pipefail

# Repo root = parent of this script's dir.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TUT="$ROOT/docs/tutorial"
WS="$ROOT/nucleus"

# All scratch under target/ so `just clean` reclaims it; unique per run.
SCRATCH="$WS/target/tutorial/run-$$"
OUT_NAIVE="$SCRATCH/naive-pthreads-sync"
OUT_SPLIT="$SCRATCH/split-mp-tcp-bufsync"
mkdir -p "$OUT_NAIVE" "$OUT_SPLIT"
trap 'rm -rf "$SCRATCH"' EXIT

build() {
    # build <sched> <backend> <out>
    ( cd "$WS" && cargo run --quiet --bin nucleus -- build \
        --algo "$TUT/prog.algo.nuc" \
        --sched "$1" \
        --kernels "$TUT/kernels.rs" \
        --backend "$2" \
        --out "$3" )
    ( cd "$3" && cargo build --release --quiet )
}

echo "tutorial: building naive schedule on pthreads-sync ..."
build "$TUT/schedules/naive.sched.nuc" pthreads-sync "$OUT_NAIVE"

echo "tutorial: building split schedule on mp-tcp-bufsync ..."
build "$TUT/schedules/split.sched.nuc" mp-tcp-bufsync "$OUT_SPLIT"

INPUT="$TUT/input.bin"
OUT1="$OUT_NAIVE/output.bin"
OUT2="$OUT_SPLIT/output.bin"

echo "tutorial: running pthreads-sync (single binary) ..."
NUC_INPUT_PATH="$INPUT" NUC_OUTPUT_PATH="$OUT1" \
    "$OUT_NAIVE/target/release/nuc-generated"

echo "tutorial: running mp-tcp-bufsync (run.sh, two processes) ..."
( cd "$OUT_SPLIT" && NUC_INPUT_PATH="$INPUT" NUC_OUTPUT_PATH="$OUT2" \
    bash run.sh "$INPUT" "$OUT2" )

echo "tutorial: diffing the two backend outputs ..."
if cmp -s "$OUT1" "$OUT2"; then
    echo "tutorial: PASS — pthreads-sync and mp-tcp-bufsync byte-identical ($(wc -c <"$OUT1") bytes)"
else
    echo "tutorial: FAIL — backend outputs DIVERGED" >&2
    cmp "$OUT1" "$OUT2" >&2 || true
    exit 1
fi
