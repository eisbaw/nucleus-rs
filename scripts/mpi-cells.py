#!/usr/bin/env python3
"""Derive the MPI check-recipe target list from the e2e-matrix.

Single source of truth for "which (example, schedule) pairs does the
out-of-band `just check-mpi{,-nonblocking}` recipe exercise":

  1. The SET of cells comes from the `[[required]]` cells of
     nuc-nucleus/e2e-matrix.toml that carry the requested `backend`
     (mpi-blocking for M7, mpi-nonblocking for M8). This is the SAME
     surface the counted `just e2e-mpi` differential drives, so the two
     MPI coverage surfaces can no longer silently drift (TASK-0454: the
     recipes previously carried their own hardcoded duplicate list).

  2. The per-cell rank count `n` (the `mpiexec -n N` value, which the
     matrix TOML does NOT carry) is DERIVED from each example's schedule
     file by counting its declared `workers = { ... }` set — the project
     convention for the worker membership (see
     09-producer-consumer/pipelined's "workers explicitly in
     `workers = { ... }` is the project convention" note). `host` counts
     as a rank; an all-ranks-live `mpiexec -n N` is the worst case the
     matrix comment documents (NOT `-n 1`, which hides Send/Recv
     ordering bugs).

Fail-fast (MPED: never silent): ANY of these abort non-zero with a
contextual message BEFORE the recipe builds/runs anything —
  - a matrix cell whose schedule file is missing,
  - a schedule file with no parseable `workers = { ... }` set,
  - an empty worker set,
  - a duplicate (example, schedule) cell for the backend.
A missing schedule file is the divergence-rot tripwire: if the matrix
gains a cell the on-disk examples cannot satisfy, the recipe dies loud.

Output: one `example<TAB>schedule<TAB>n` line per cell, sorted
deterministically (example, then schedule), to stdout. Deterministic so
a recipe `for` loop and a test `diff` are stable.

Usage:
    mpi-cells.py <e2e-matrix.toml> <examples-root> <backend>
e.g.
    mpi-cells.py nuc-nucleus/e2e-matrix.toml nuc-nucleus/examples mpi-blocking
"""

import re
import sys
import tomllib
from pathlib import Path

# Matches the schedule's worker-membership declaration, e.g.
#   workers = { host, w0, w1, w2, w3 };
# Captures the comma-separated body between the braces. Anchored to the
# start-of-line `workers` keyword (after optional indentation) so a
# `workers` mention inside a comment line (which begins with `//`) is not
# matched.
_WORKERS_RE = re.compile(r"^\s*workers\s*=\s*\{([^}]*)\}", re.MULTILINE)


def die(msg: str) -> None:
    """Fail-fast: print a contextual error to stderr and exit non-zero."""
    print(f"mpi-cells.py: ERROR: {msg}", file=sys.stderr)
    sys.exit(1)


def worker_count(sched_path: Path) -> int:
    """Count the workers declared in a schedule's `workers = { ... }` set.

    The declared worker set IS the rank count for `mpiexec -n N`
    (`host` is rank 0). Fails loud if the file is missing, has no
    `workers = { ... }` line, or declares an empty set.
    """
    if not sched_path.is_file():
        die(
            f"divergence-rot: matrix references a schedule with no file "
            f"on disk: {sched_path} (the e2e-matrix mpi cell list and the "
            f"committed examples have drifted)"
        )
    text = sched_path.read_text(encoding="utf-8")
    m = _WORKERS_RE.search(text)
    if m is None:
        die(f"{sched_path}: no parseable `workers = {{ ... }}` declaration")
    members = [w.strip() for w in m.group(1).split(",") if w.strip()]
    if not members:
        die(f"{sched_path}: declared an empty `workers = {{ }}` set")
    return len(members)


def main() -> None:
    if len(sys.argv) != 4:
        die("usage: mpi-cells.py <e2e-matrix.toml> <examples-root> <backend>")
    matrix_path = Path(sys.argv[1])
    examples_root = Path(sys.argv[2])
    backend = sys.argv[3]

    if not matrix_path.is_file():
        die(f"matrix file not found: {matrix_path}")
    if not examples_root.is_dir():
        die(f"examples root not found: {examples_root}")

    with matrix_path.open("rb") as fh:
        matrix = tomllib.load(fh)

    cells = []
    seen = set()
    for cell in matrix.get("required", []):
        if cell.get("backend") != backend:
            continue
        example = cell.get("example")
        schedule = cell.get("schedule")
        if not example or not schedule:
            die(f"matrix cell missing example/schedule: {cell!r}")
        key = (example, schedule)
        if key in seen:
            die(f"duplicate matrix cell for {backend}: {example}/{schedule}")
        seen.add(key)
        n = worker_count(
            examples_root / example / "schedules" / f"{schedule}.sched.nuc"
        )
        cells.append((example, schedule, n))

    if not cells:
        die(f"no [[required]] cells found for backend {backend!r} in {matrix_path}")

    for example, schedule, n in sorted(cells):
        print(f"{example}\t{schedule}\t{n}")


if __name__ == "__main__":
    main()
