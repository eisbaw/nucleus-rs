# Case study fixture — VGA frame-strip stencil

This directory is the **production witness** fixture for the Nucleus v2
compiler (TASK-0455.03). It is a realistic-size (32-frame VGA strip,
15360×640) single-pass 3×3 box-blur stencil, carried end to end across a
single-worker baseline and two distributed decompositions (tier-1
pthreads-async, tier-2 mpi-nonblocking) and byte-diffed against an
independent reference oracle.

It lives **outside** `nuc-nucleus/examples/` on purpose, so the e2e
matrix (which enumerates only `nuc-nucleus/examples/<NN-name>/`) does not
pick it up — it is exercised by its own runner instead.

**The full writeup, with all sizes / schedules / cells / compile / gate /
runtime / memory / transfer-volume numbers and the honest limitations,
is `../case-study.md`.**

## Run it

```
just case-study
```

Runs `scripts/case-study-run.sh` inside the `.#mpi` Nix shell (which
carries both the tier-1 cargo toolchain and `mpiexec`).

## Files

| path                       | what                                                       |
| -------------------------- | ---------------------------------------------------------- |
| `prog.algo.nuc`            | the algorithm (15360×640 single-pass 3×3 blur)             |
| `kernels.rs`               | `blur3` + load/save kernel bodies (i32, env-var IO)        |
| `schedules/naive.sched.nuc`       | single `host` baseline                              |
| `schedules/distributed.sched.nuc` | `host` + `w0..w3`, `partition=rows`                 |
| `reference/`               | independent reference oracle crate (`std` only, policy §2) |
| `gen/`                     | independent input-frame generator crate (`std` only)       |

## Fixtures are generated, not committed

`input.bin` and `reference.bin` (≈ 37.5 MiB each) exceed the
reference-impl-policy "a few MB" ceiling, so they are **not** committed.
The runner regenerates them every run from the two `std`-only crates;
reproducible by construction (no RNG, no clock):

```
# input frame (deterministic)
cargo run --release --manifest-path docs/case-study/gen/Cargo.toml -- \
    --out docs/case-study/input.bin

# expected output via the independent oracle
cargo run --release --manifest-path docs/case-study/reference/Cargo.toml -- \
    --in  docs/case-study/input.bin \
    --out docs/case-study/reference.bin
```

If the algorithm's H/W constants change, the `reference/` and `gen/`
crates' constants must change in the same commit (policy §3).
