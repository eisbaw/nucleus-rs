# `capabilities.toml` schema

Each backend crate ships a sibling `capabilities.toml` text file. It is
the **declared, committable contract** of what the backend can do. The
compiler's capability-check pass (TASK-0019, used at codegen time per
TASK-0118) loads this file and rejects any schedule whose demands fall
outside the backend's declared capabilities. Forward-compatible because
the file is text, diffable, and reviewable without rebuilding the
compiler. Maps to PRD §7 (presentation layers) and PRD §7.4 (capability
matrix).

This is not a runtime config. It is the static declaration the compiler
uses to fail-fast on `(schedule, backend)` mismatches.

## Example

```toml
# capabilities.toml for the mp-tcp-event backend
name            = "mp-tcp-event"
tier            = 1
transport       = "tcp"
notify          = ["event"]
supports_async  = true
supports_buffer = true
max_buffer      = 1024
worker_classes  = ["default"]
memory_regions  = ["heap"]
```

## Fields

All fields below (`name` … `memory_regions`) are required. The three
topology/mediation flags in §"Topology / mediation flags" are OPTIONAL
and default to `false` (so an off-tree or pre-TASK-0455.09 file parses
unchanged and selects no host-mediation passes); every in-tree backend
declares them explicitly. Unknown fields are rejected
(`deny_unknown_fields`). Forward-compatible additions are flagged in
§Limitations.

### `name`

- Type: string.
- Required.
- Semantics: backend identifier. Must match the crate name (e.g.
  `mp-tcp-event` for the `mp-tcp-event` backend crate). Used in error
  messages and to disambiguate when multiple backends are scanned.

### `tier`

- Type: integer.
- Required. Allowed values: `1`, `2`, `3`.
- Semantics: which target tier the backend belongs to. PRD §7:
  - `1` — CPU-simulatable (commodity hardware, the falsification rig).
  - `2` — HPC cluster (MPI).
  - `3` — embedded (`no_std`, per-MCU shims).
- Other values are rejected at parse time with `CapError::InvalidTier`.

### `transport`

- Type: string enum.
- Required. Allowed values: `shared-memory`, `tcp`, `uds`, `mpi`,
  `embedded-dma`.
- Semantics: the wire / channel that cross-worker data transfers ride
  on. Maps directly to backend codegen (a `Push` event is a memcpy on
  `shared-memory`, a `socket.write` on `tcp`, an `MPI_Isend` on `mpi`,
  a DMA descriptor enqueue on `embedded-dma`).
- Other values are rejected with `CapError::UnknownTransport`.

### `notify`

- Type: array of string enums.
- Required (may be empty for backends that never emit a notification).
- Allowed element values: `event`, `poll`, `barrier`, `blocking`, `irq`.
- Semantics: the notification modes the backend supports. The
  schedule's `transfer D : notify=X` must have `X` present in this
  array, otherwise `CapMismatch::NotifyModeNotSupported`.
- Note: PRD §6.3.4 today only lists `event` and `poll` as schedule
  surface options. The wider set (`barrier`, `blocking`, `irq`) is
  declared here so a backend can advertise what *it* can do regardless
  of whether v2 schedules can request it yet.
- Tier note (TASK-0455.02): the tier-3 `embedded-pattern` backend
  declares `notify = ["event", "poll", "barrier", "blocking"]`. On its
  `mode=dma` ring path it is the FIRST backend to HONOUR the per-seq
  `notify` choice: `event` completes via the IRQ-driven
  `dma_link_irq_wait` hook, `poll`/absent via the `dma_link_poll`
  busy-spin. (`barrier`/`blocking` remain for the existing sync barrier +
  blocking-recv paths.) Declaring both `event` and `poll` keeps the cap
  surface consistent with what the renderer actually emits.
- Unknown elements rejected with `CapError::UnknownNotify`.

### `supports_async`

- Type: bool.
- Required.
- Semantics: when `false`, the schedule must not request `async` on
  any `transfer` directive. Asking yields
  `CapMismatch::AsyncNotSupported`.
- Tier note (TASK-0455.02): the tier-3 `embedded-pattern` backend
  declares `supports_async = true`. On its `mode=dma` transport path an
  `async` transfer arms a DMA descriptor (the producer is not blocked at
  the arm site); a `mode=pio` transfer ignores it. See `supports_buffer`
  for the depth-2 descriptor ring this enables.

### `supports_buffer`

- Type: bool.
- Required.
- Semantics: when `false`, the schedule must not request `buffer=N`
  with `N > 1`. (A request of `buffer=1` is "no extra buffering", the
  default of `TransferPolicy`, and always allowed.) Asking yields
  `CapMismatch::BufferNotSupported`.

### `max_buffer`

- Type: `u32`.
- Required.
- Semantics: the largest `buffer=N` the backend can satisfy. The
  schedule's `buffer=N` must satisfy `N <= max_buffer`. Otherwise
  `CapMismatch::BufferTooLarge`.
- Combined semantics with `supports_buffer`:
  - `supports_buffer = false` AND `max_buffer = 0` is the canonical
    "no buffering at all" state.
  - `supports_buffer = true` AND `max_buffer = 1` means the backend
    supports the buffer keyword but only the default depth; not very
    useful but legal.
  - `supports_buffer = false` AND `max_buffer > 1` is allowed and
    treated as "no in-flight buffering" — the `supports_buffer` check
    fires first.
- Tier note (TASK-0455.02): the tier-3 `embedded-pattern` backend
  declares `supports_buffer = true`, `max_buffer = 2`. A `mode=dma`
  transfer crossing inside a pipelined loop body lowers to a DEPTH-2
  descriptor ring (the producer arms transfer `i+1` while `i` is in
  flight — the bare-metal double-buffered DMA pipeline). `max_buffer = 2`
  is the honest cap: `buffer=3..` (e.g. example 9's `buffer=4`) STILL
  rejects with `BufferTooLarge`; deeper rings are follow-up work, not a
  capability lie. On a `mode=pio` (or absent-`mode=`) transfer the buffer
  fact is ignored (no ring).

### `worker_classes`

- Type: array of strings.
- Required (may be empty).
- Semantics: names of worker classes the backend can map onto.
  - The literal name `"default"` means the backend can host the
    simple-form worker shape (PRD §6.3.1: simple form is "equivalent
    to the typed form with a single default worker class"). All
    tier-1 backends should declare `"default"`.
  - Tier-3 backends typically declare class names matching the typed
    schedule's `worker_class` decls (e.g. `["control_core",
    "compute_core"]`).
- The schedule's resolved worker classes must each be in this list.
  Otherwise `CapMismatch::WorkerClassNotSupported`. The synthetic
  `__default` class (from the simple worker form, see SchedIR's
  `DEFAULT_WORKER_CLASS`) matches against the capability list entry
  `"default"`.

### `memory_regions`

- Type: array of strings.
- Required (may be empty).
- Semantics: names of memory regions the backend supports.
  - Tier-1 backends typically declare `["heap"]`.
  - Tier-3 backends declare physical region names matching the
    schedule's `memory_region` decls (`["tcm_per_core",
    "shared_sram"]`).
- The schedule's resolved memory regions (the `region` field of every
  `place_data` directive) must each be in this list. Otherwise
  `CapMismatch::MemoryRegionNotSupported`.

> **What `memory_regions` does and does NOT do today (TASK-0455.16).**
> This list is a backend **admission gate**, not a codegen input. The
> resolved region of a `place_data D in R` directive is checked against
> this list (reject if absent) and then **not threaded any further** —
> no backend yet consumes a region placement. That is deliberate, not a
> dropped fact: the `Event::Alloc` / `Event::Free` / `Region` contract
> surface in `nucleus-compiler` is a **reserved** surface emitted by no
> pass (the thesis reserves it for a future GPU/NPU tier), and the only
> `place_data`-using schedule in the example corpus
> (`14-hearing-aid/embedded_multimcu.sched.nuc`) is *rejected* by this
> gate precisely because the embedded backend declares only `["heap"]`,
> not `sram_shared`. The gate's whole job today is to reject placements a
> backend cannot honour. When an *accepted* `place_data` first lands, the
> intended lowering is a per-`DataId` region sidecar fact the backend
> render reads (the `XferFacts` precedent), NOT `Alloc`/`Free` events —
> see the `nucleus_compiler::event` module-doc section "DELIBERATELY
> RESERVED: `Alloc` / `Free` / `Region`".

## Topology / mediation flags

Three booleans (TASK-0455.09) declare the backend's wire-topology facts
that decide **which host-mediation compiler passes run**. They used to be
hard-coded as three separate backend-NAME lists in the driver
(`driver/src/main.rs`); a new platform had to remember up to three lists
and a miss was a silent topology mismatch (the silent-sibling failure
class). They are now declared once, here, per backend.

Each flag selects exactly one pass. They are NOT a schedule-compat axis
(`check_schedule_compat` never reads them) — they drive the driver's
internal pass selection only.

The capability file is **authoritative**: a build with an explicit
`--capabilities` file that omits these flags gets NO mediation passes,
even for a backend name that the deleted driver lists used to match
unconditionally. That is the intended semantics (one source of truth,
serde-default `false`), but it IS a behaviour change for hand-rolled
out-of-tree capability files aimed at the `mp-tcp-*` / `mp-uds-event`
backends — such files must now declare the flags explicitly (see the
per-backend table below).

### `star_topology_host_mediation`

- Type: bool. Optional, defaults to `false`.
- Semantics: `true` iff the backend has a host-mediated **star**
  topology with no native worker-to-worker barrier channel, so every
  host-EXCLUDING barrier must be re-routed through the elected host. The
  driver runs `apply_host_mediation_inject` for these backends. `false`
  for backends whose barrier primitive handles host-excluding barriers
  natively: shared-memory `std::sync::Barrier` (pthreads-*, openmp-rs),
  MPI `Comm_split` sub-comm barrier (mpi-*), the embedded stub.

### `host_data_relay`

- Type: bool. Optional, defaults to `false`.
- Semantics: `true` iff the backend has no native worker-to-worker DATA
  channel, so every worker-to-worker `Push`/`Wait` pair is relayed
  through the elected host. The driver runs
  `apply_host_data_relay_inject`. **Implies `star_topology_host_mediation`**
  — a transfer cannot be relayed through a host that is not a mediating
  hub. `validate` rejects the contradiction loudly
  (`CapError::InconsistentTopologyFlags`).

### `reorderable_push`

- Type: bool. Optional, defaults to `false`.
- Semantics: `true` iff the backend's wait primitive is per-(seq)
  DEMUXED (an inbound queue keyed by sequence, not a strict per-pair FIFO
  stream), so a hoistable worker-to-worker `Push` can be safely moved
  ahead of a preceding `Wait` to break the host-relay wait-before-push
  deadlock. The driver runs `apply_safe_push_reorder`. The strict-FIFO
  transports (mp-tcp-bufsync, mp-tcp-poll) MUST NOT set this — moving a
  push ahead of a wait would race host's own w2w waits on the shared
  stream. **Implies `star_topology_host_mediation`** (the reorder only
  matters on the host-relay path); `validate` enforces it.

### Per-backend declaration (the equivalence the driver test pins)

The values below reproduce EXACTLY the old driver name-list selection.
The exhaustive equivalence is asserted in
`driver/tests/task0455_09_capability_pass_selection.rs` (each of the 10
backends: capability-driven selection == old name-list selection).

| backend            | `star_topology_host_mediation` | `host_data_relay` | `reorderable_push` |
| ------------------ | ------------------------------ | ----------------- | ------------------ |
| pthreads-sync      | false                          | false             | false              |
| pthreads-async     | false                          | false             | false              |
| openmp-rs          | false                          | false             | false              |
| mpi-blocking       | false                          | false             | false              |
| mpi-nonblocking    | false                          | false             | false              |
| embedded-pattern   | false                          | false             | false              |
| mp-tcp-bufsync     | true                           | false             | false              |
| mp-tcp-poll        | true                           | false             | false              |
| mp-tcp-event       | true                           | true              | true               |
| mp-uds-event       | true                           | true              | true               |

## Compatibility check (`check_schedule_compat`)

The compiler calls `check_schedule_compat(caps, sched)` to verify a
schedule is satisfiable on the chosen backend. It walks the schedule's
`transfer`, worker-class, and memory-region declarations and emits one
of the following per mismatch:

| Variant                            | Cause                                                                |
| ---------------------------------- | -------------------------------------------------------------------- |
| `AsyncNotSupported`                | `transfer D : async` and `supports_async = false`.                   |
| `BufferNotSupported`               | `transfer D : buffer=N`, `N > 1` and `supports_buffer = false`.      |
| `BufferTooLarge`                   | `transfer D : buffer=N`, `N > max_buffer`.                           |
| `NotifyModeNotSupported`           | `transfer D : notify=M` where `M` is not in `caps.notify`.           |
| `WorkerClassNotSupported`          | Schedule declares worker class not in `caps.worker_classes`.         |
| `MemoryRegionNotSupported`         | `place_data D in R` where `R` is not in `caps.memory_regions`.       |

The check is **batch**: all mismatches are reported in one pass,
sorted for deterministic diffs, deduped on Debug-equality. Callers
get `Result<(), Vec<CapMismatch>>`.

## Design questions captured

- **Transport as enum vs free string.** Currently a closed enum.
  Adding a new transport (e.g. `rdma`, `shared-memory-numa`) requires
  a compiler change. Could relax to free string and rely on the
  backend codegen to reject unknowns — but then a typo
  (`"tpc"`) goes undetected here. Closed enum is the loud-failure
  choice; revisit if the set of transports churns.
- **Extending `notify` with backend-specific modes.** Same trade-off
  as transport. PRD §6.3.4 only lists `event`/`poll` on the schedule
  side; the capability side accepts a wider set so the schedule
  surface can grow without recompiling the capability parser. If a
  backend wants a truly custom notify ("`my_proprietary_doorbell`"),
  add it to the enum.
- **Conditional capabilities.** The schedule-compat axes
  (`supports_async`, `notify`, …) are flat flags: the schema cannot
  express "async is supported, but only when `buffer >= 2`". That
  flat-flag shape is by design — the schedule × backend product is small
  enough that a real conflict shows up as a CapMismatch elsewhere.
  Future work could add a `restrictions = [ ... ]` array. The
  topology/mediation flags (TASK-0455.09) DO carry the schema's first
  CROSS-FIELD consistency rule (`host_data_relay` / `reorderable_push`
  each imply `star_topology_host_mediation`), enforced in
  `Capabilities::validate` at load time with
  `CapError::InconsistentTopologyFlags`. That rule is a load-time
  validity check on the cap-file itself, NOT a schedule-conditional
  capability.
- **Schema versioning.** The `schema_version` field (TASK-0120) gates
  parsing rules per version; `SUPPORTED_SCHEMA_VERSIONS` is the accepted
  set (currently `[1]`). When a future field becomes mandatory in a
  back-incompatible way, the loader revs that list and per-version-gates
  the changed parsing. The three topology/mediation flags were added
  back-compatibly (optional, defaulting to `false`) so they did NOT need
  a version bump — they stay on `schema_version = 1`.

## Limitations

- The compatibility check does not yet validate `transport` against
  the schedule — there is no schedule-side directive that demands a
  specific transport (PRD §6.3 schedule sublanguage chooses
  capabilities via `transfer` options, not transport name). The
  field exists so codegen can branch on it; the check just records
  the chosen transport.
- The check assumes the schedule is fully lowered (`SchedIR`). It
  does not consult the algorithm IR — that's the link step's job
  (TASK-0011). Capability checking happens after linking, on the
  resolved schedule.
- Order-dependent capabilities (e.g. "max_buffer=128 only when
  notify=event") cannot be expressed.
- Conflicts between schedule options (e.g. `sync, async` on the same
  transfer) are not this pass's problem; they're caught earlier in
  lowering (see TASK-0119).
