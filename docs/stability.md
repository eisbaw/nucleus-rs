# Language stability note (v0.1)

This note states which parts of the Nucleus surface are **stable at
v0.1** and which **may change**. The contract for the *syntax* is the
grammar appendix:

- [`docs/grammar-algo.md`](grammar-algo.md) — the algorithm sublanguage
  (`*.algo.nuc`).
- [`docs/grammar-sched.md`](grammar-sched.md) — the schedule sublanguage
  (`*.sched.nuc`).

Those two EBNF documents are *descriptive* (the reference parser is
hand-written against them, with conformance asserted by behavioural
tests, not grammar derivation). They are nonetheless the authoritative
statement of what the parser accepts. Where this note and the grammar
disagree, the grammar wins for syntax and the PRD
([`nuc-nucleus/PRD.md`](../nuc-nucleus/PRD.md)) wins for semantics.

v0.1 is a thesis-grade research compiler. "Stable" here means *we intend
not to break it without a deliberate version bump and a migration note*
— it is not a backwards-compatibility guarantee across major versions.

---

## Stable at v0.1

These surfaces are exercised by the worked-example matrix and the
cross-backend bit-identical differential; changing them would break that
matrix, so they are stable.

### Algorithm sublanguage

- The **algorithm / schedule split** itself: an algorithm names no
  worker, transfer, or backend.
- `const NAME : TYPE = VALUE;` declarations.
- `data NAME : TYPE[DIMS];` array declarations, single-assignment.
- `kernel NAME : (ARGS) -> RET purity;` with purity `pure` or
  `effectful`.
- Dataflow statements: `lhs <-- kernel(args);`, including indexed
  `c[i] <-- ...`.
- `for IV : LO .. HI { ... }` counted loops.
- Scalar integer kernels (`i32` etc.) — the determinism-critical core.

### Schedule sublanguage

- `schedule for "PATH" { ... }` binding to an algorithm file.
- `workers = { host, w0, ... };`.
- `place KERNEL on WORKER;` (and `on { w0, w1, ... }` for distributed
  placement).
- `transfer DATA : sync;` — a transfer is **required** for every data
  symbol that crosses a worker boundary (omitting it is a hard error).
- The `host` worker as the I/O / file-system owner by convention.

### Tooling / interface

- The `nucleus build` CLI (flags, exit-code convention, output-on-success
  discipline) — see [`cli-reference.md`](cli-reference.md).
- `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` as the runtime I/O wiring for
  generated programs.
- The single-binary vs `run.sh` distinction for single-process vs
  multi-process backends.
- Little-endian fixed-width integer encoding as the on-disk data format
  for the differential.

---

## May change

These surfaces are newer, narrower, or known-incomplete. Treat them as
provisional.

### Kernel/type contract

- **Aggregate type matching.** The contract pass is scalar-only today;
  aggregate-typed I/O kernels (`i32[N]`) report a *non-fatal* type
  mismatch and the build proceeds. The accepted Rust spelling for
  array arguments (`Vec<i32>` today) may change when aggregate matching
  lands. See the grammar's "Limitations" sections and the PRD §6.2.2
  open item.
- **Const propagation into kernels.** The algorithm's `const N` is
  duplicated by hand in `kernels.rs` today. When the codegen passes
  consts through to Rust, that duplication goes away — the kernel
  signature surface may tighten.
- **Floating-point determinism.** Only opt-in fixed-order reductions
  (`combine=fsum`) are bit-reproducible; general f32 reduction ordering
  is not part of the stable contract.

### Schedule transforms

- Loop options beyond placement/transfer — `block=`, `vectorize=`,
  `pipeline=`, `partition=` (rows / blocks2d / workers), halo / reuse
  inference, `notify=`, `buffer=N`, transport hints (`mode=pio|dma`) —
  are real but evolving; their interaction order and exact spellings may
  change.
- Async / buffered transfer semantics (`async`, `buffer=N`) vs the
  stable `sync` baseline.
- The `check loop` latency directive (single-worker only on several
  backends; multi-worker check-loops are rejected loud on MPI).

### Backends and tiers

- The tier-2 (MPI) and tier-3 (embedded / Renode, multi-MCU) backends
  and the `--shim` selector are research surfaces; the set of backends,
  their capability flags (`capabilities.toml` schema), and the shim
  names may change.
- The Petri-net soundness gate is an exact-replay tripwire over v2's
  restricted statically-ordered nets — not a general reachability
  engine. Its acceptance envelope may widen.

### Diagnostics

- Error message *text* is not a stable interface. Match on stderr
  prefixes (`nucleus: error:`) and exit codes, not on message bodies.
  Post-link pass errors do not yet carry source `line:col` — see
  [`diagnostics-audit.md`](diagnostics-audit.md).

---

## Explicitly out of scope for v2

Per the PRD and README: backward pass / autodiff, collectives, an
auto-tuner, and a general polyhedral optimizer are out of scope. They are
not "may change" — they are absent by design.
