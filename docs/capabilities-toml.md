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

All fields are required. Unknown fields are rejected (`deny_unknown_fields`).
Forward-compatible additions are flagged in §Limitations.

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
- Unknown elements rejected with `CapError::UnknownNotify`.

### `supports_async`

- Type: bool.
- Required.
- Semantics: when `false`, the schedule must not request `async` on
  any `transfer` directive. Asking yields
  `CapMismatch::AsyncNotSupported`.

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
- **Conditional capabilities.** The schema cannot express things like
  "async is supported, but only when `buffer >= 2`", or
  "`notify=event` only when `transport=tcp`". The flat-flag shape
  rejects this by design — the schedule × backend product is small
  enough that a real conflict shows up as a CapMismatch elsewhere.
  Future work could add a `restrictions = [ ... ]` array.
- **Schema versioning.** No `schema_version` field today. When a
  future field becomes mandatory in a back-incompatible way, a
  version field would help the parser distinguish. For now, the
  `deny_unknown_fields` policy plus the small field set keeps
  evolution mechanically inspectable. Filed as a follow-up.

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
