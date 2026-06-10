# CLI reference: the `nucleus` driver

The compiler driver is the `nucleus` binary in the `nucleus/` workspace.
It has one subcommand today, `build`. This document is the
ergonomics-level reference (flags, defaults, exit codes, output
discipline); the end-to-end walkthrough is [`tutorial.md`](tutorial.md).

Inside the dev shell, invoke it via Cargo:

```
cd nucleus
cargo run --bin nucleus -- build --algo ... --sched ... --backend ... --out ...
```

`cargo run --bin nucleus -- --help` prints the same flag/backend summary
the source keeps in sync (the `print_help` function in
`nucleus/driver/src/args.rs`).

---

## `nucleus build`

Drives the full pipeline: parse algo + schedule -> lower -> link -> build
ACFG -> inject syncs -> inject transfers -> load backend capabilities ->
check schedule/backend compatibility -> Petri-net soundness gate ->
backend `emit(...)` -> write a `run.sh` to the output directory.

### Flags

| Flag | Required? | Meaning |
|---|---|---|
| `--algo FILE` | yes | The algorithm source (`*.algo.nuc`). |
| `--sched FILE` | yes | The schedule source (`*.sched.nuc`). |
| `--backend NAME` | yes | Target backend (see the list below). |
| `--out DIR` | yes\* | Output directory for the emitted Cargo project. \*Optional **only** when `--emit-pn` is given alone (inspection-only build). |
| `--kernels FILE` | no | Rust kernel bodies. Default: `kernels.rs` next to the algorithm file. |
| `--capabilities FILE` | no | Backend capabilities TOML. Default: see [capability resolution](#capability-resolution) below. |
| `--emit-pn FILE` | no | Write the global Petri net as Graphviz DOT (PRD §8.5). Makes `--out` optional. |
| `--shim NAME` | no | Tier-3 target shim (`stm32h7` / `nrf52840`); embedded-pattern backend only. Omit for the compile-only `no_std` lib. |
| `-h`, `--help` | no | Print usage and exit 0. |

An unknown flag, or a flag missing its value, is a hard error (exit 1)
— there are no silently-ignored arguments.

### Backends

| Name | Tier | Transport |
|---|---|---|
| `pthreads-sync` | 1 | shared-memory threads (single binary) |
| `pthreads-async` | 1 | shared memory + ring buffer |
| `openmp-rs` | 1 | rayon threads |
| `mp-tcp-bufsync` | 1 | OS processes over TCP loopback (sync) |
| `mp-tcp-event` | 1 | OS processes + TCP + mio reactor |
| `mp-tcp-poll` | 1 | OS processes + TCP + nonblocking poll |
| `mp-uds-event` | 1 | OS processes + Unix-domain sockets + mio |
| `mpi-blocking` | 2 | SPMD MPI (blocking); `.#mpi` dev shell |
| `mpi-nonblocking` | 2 | SPMD MPI (non-blocking buffered); `.#mpi` dev shell |
| `embedded-pattern` | 3 | `no_std` lib / Renode-runnable bin via `--shim` |

A single-binary backend (`pthreads-sync`) emits `nuc-generated` and is
run directly. A multi-process backend emits `run.sh`, which launches one
OS process per worker. Both honour `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH`.

### `--emit-pn` (inspection-only builds)

`--emit-pn FILE` writes the global Petri net (the soundness model) as a
Graphviz DOT file. The pipeline still runs up through transfer injection
to produce the net, but in this mode `--out` becomes optional: you can
inspect the net without triggering backend codegen. If both `--out` and
`--emit-pn` are given, both outputs are produced.

### Capability resolution

`--capabilities` always wins, and is the recommended, reproducible path
for any program built **outside** this repo.

When `--capabilities` is omitted, the driver
(`find_default_capabilities` in `nucleus/driver/src/main.rs`) walks **up
from the current working directory** looking for
`nucleus/backends/<backend>/capabilities.toml` (the in-repo canonical
layout). This is a convenience for in-repo use (the e2e tests refer to a
sibling backend crate by name); it is deliberately **not** load-bearing
for correctness.

Two practical consequences for external users:

- Outside the Nucleus repo, the CWD-walk finds nothing and the build
  fails with a message that names the walk and tells you to pass
  `--capabilities` explicitly. **Pass it explicitly** rather than
  relying on the walk.
- The resolved path is observable: run with `NUC_TRACE=1` to see a
  `find_default_capabilities: resolved ... by CWD-walk to <path>` trace
  line, so you always know which file was used.

---

## Exit codes

The driver uses a binary exit-code convention:

| Code | Meaning |
|---|---|
| `0` | Success (a build completed, or `--help` / inspection-only emit-pn). |
| `1` | Any error — bad arguments, a parse/lower/link error, a capability or compat rejection, a soundness-gate rejection, a codegen error, or a missing input file. |

There are no finer-grained exit codes today; distinguish error *classes*
by the message text on stderr, not by the code.

## Output-on-success discipline

- **stdout** carries success output only: on a completed build the
  driver prints `nucleus: ok` followed by the emitted file paths
  (`project_dir = ...`, `cargo_toml = ...`, `run_sh = ...`, etc.). An
  `--emit-pn` run prints `emit_pn = <path>`.
- **stderr** carries diagnostics: every error is printed as `nucleus:
  error: <message>` (and the process exits 1). The contract check is a
  best-effort warning surface — it prints `warning: contract check
  reported N issue(s)` to stderr and **proceeds** (it does not fail the
  build; aggregate-typed I/O still reports a non-fatal type mismatch
  until aggregate matching lands).
- A clean build writes nothing alarming to stderr. Errors are never
  swallowed — there are no silent failures.

---

## Known ergonomics gaps

These are documented limitations, not bugs:

- **One subcommand.** Only `build` exists; there is no `check`, `fmt`, or
  `run` subcommand. Use Cargo on the emitted project to build/run it.
- **No `--version`.** The driver does not print a version string.
- **CWD-dependent default capability lookup.** Surprising outside the
  repo; see [capability resolution](#capability-resolution). Mitigated by
  the explicit `--capabilities` flag and the `NUC_TRACE=1` trace line.
- **Coarse exit codes.** All failures share exit code 1; callers must
  parse stderr to classify.
