# nucleus/nucleus-compiler

Front-end + middle-end of the Nucleus v2 compiler. Reads `.algo.nuc` +
`.sched.nuc` source files; emits the IR contracts (per-worker `EventList`,
`NameTables`, `NameSidecar`) that the backend crates consume.

## What lives here

- `src/algo/` — algorithm-language parser, AST, IR (`AlgoIR`), lowering.
- `src/sched/` — schedule-language parser, AST, IR, lowering.
- `src/link/` — algorithm × schedule reconciliation (`LinkedIR`).
- `src/acfg/` + `src/passes/` — ACFG (Algorithmic Control Flow Graph)
  build + transforms: `apply_block_transforms`, `apply_partition_workers`,
  `inject_syncs`, `inject_transfers`, `inject_check_frames`,
  `acfg_to_petri`, `acfg_to_events`.
- `src/event.rs` — the inert `Event` enum the backends consume.
- `src/sidecar.rs` — `NameSidecar` (loop bounds, partition slices, data
  types, transfer buffer sizes — everything the projection needs).
- `src/name_tables.rs` — reverse name tables (`NameTables`).
- `src/petri.rs` + `src/boundedness.rs` — Petri net analysis +
  boundedness/deadlock passes.

## Driver

The `nucleus` binary lives in `nucleus/driver/`. It uses this crate's
`build_*` + `link` + `inject_*` + `acfg_to_*` functions to lower a
source pair down to the IR contract, then hands off to a backend crate
(`pthreads-sync`, `pthreads-async`, `mp-tcp-bufsync`, `mp-tcp-event`).

## Backend contract

Backends are AlgoIR/ACFG-FREE. They receive ONLY:
- `per_worker: BTreeMap<WorkerId, Vec<Event>>` — the projected event
  stream per worker.
- `names: NameTables` — reverse names for ids.
- `sidecar: NameSidecar` — everything the projection couldn't carry in
  the inert `Event` shape.

See `nucleus/backend-common/` for the shared codegen surface that all
backends consume.

## Tests

`cargo test -p nucleus-compiler` exercises parser → lower → link → acfg → events
plus the Petri net analyses. The e2e differential matrix
(`nucleus/e2e/`) gates the end-to-end bit-identicality invariant
across backends.
