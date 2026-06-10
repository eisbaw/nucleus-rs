# Diagnostics UX audit

This document audits every user-facing error surface the `nucleus`
driver can emit, scoring each on four axes:

- **Names symbol?** Does the message name the offending symbol (data
  array, kernel, loop variable, worker)?
- **Source location?** Does it resolve a `line:col` (or byte span) in the
  user's source?
- **Fix hint?** Does it suggest a concrete remedy?
- **Leaks internals?** Does it print tracker IDs (`TASK-NNNN`) or raw
  `Debug` forms (`DataId(3)`) that mean nothing to an external user?

The audit was produced for TASK-0455.06. Cheap gaps (tracker-ID leaks in
user-facing strings) were fixed in place during that task; the expensive
gap (threading source spans onto post-link pass errors) is filed as a
follow-up — see [Filed follow-ups](#filed-follow-ups).

The driver surfacing template (one header line + one line per error, so
a user sees *all* errors at once) is shared across the parse / lower /
link paths in `cmd_build` (`nucleus/driver/src/main.rs`).

---

## Audit table

| Error surface | Phase | Names symbol? | Source location? | Fix hint? | Leaks internals? |
|---|---|---|---|---|---|
| `ParseErrors` (algo + sched) | parse | yes (token) | **yes** — each error carries 1-based `line:col` | partial (parser-level "expected X") | no |
| `LowerError` / `LowerErrors` | lower (algo) | yes | **yes** — byte span -> `line:col` via `display_with_src` | varies by variant | no |
| `SchedLowerError` / `SchedLowerErrors` | lower (sched) | yes | **yes** — `display_with_src` | varies | no |
| `LinkError` (+ `LinkErrorSource`) | link | yes | **yes** — span + algo/sched-source tag via `display_with_src` | varies | no |
| `ContractError` (non-fatal warning) | contract | yes (kernel + position) | partial (kernel name, no line) | partial | **fixed** (was `TASK-0012`) |
| `CapError` (capabilities load) | caps load | yes (the TOML field) | path of the TOML file | yes | no |
| `CapMismatch` (schedule/backend compat) | compat | yes (data symbol) | no | partial | no |
| `BuildAcfgError` | post-link pass | **yes** (loop var / kernel / lhs / rhs) | **no span** | varies | no |
| `BlockTransformError` | post-link pass | **yes** (loop var, block size) | **no span** | varies | no |
| `PartitionError` (workers) | post-link pass | **yes** (loop var, worker set) | **no span** | partial | no |
| `PartitionRowsError` | post-link pass | yes | **no span** | partial | no |
| `PartitionBlocks2dError` | post-link pass | yes | **no span** | partial | no |
| `SidecarError` | sidecar build | yes | **no span** | partial | no |
| `HaloInferenceError` | post-link pass | **yes** (kernel / loop var) | **no span** | partial | no |
| `ReuseInferenceError` | post-link pass | yes (loop var, slot range) | **no span** | partial | no |
| `SyncInjectError` | post-link pass | yes (worker sets) | **no span** | yes (now actionable) | **fixed** (was `TASK-0268`/`0365`/`0281`) |
| `TransferInjectError::SameSetSilentElisionRisk` | post-link pass | yes (data name + id) | **no span** | yes | **fixed** (was `TASK-0324`/`0325`) |
| `TransferInjectError::CumulativeWholeArrayFallback` | post-link pass | partial (`DataId` debug, no name) | **no span** | yes | **fixed** (was `TASK-0366`); name still `Debug` (filed) |
| `PetriAnalysisError::Boundedness` (gate) | soundness gate | partial (place) | no | partial | leaks transition `Debug` id |
| `PetriAnalysisError::Deadlock` (gate) | soundness gate | partial (place) | no | partial | leaks transition `Debug` id |
| `PetriAnalysisError::ConflictingChoice` (gate) | soundness gate | partial | no | partial | leaks transition `Debug` id |
| `EmitError::UnsupportedFeature` | codegen | varies (message-dependent) | no | partial | **fixed** (was `TASK-0045.03` in MPI arm) |
| `EmitError::ContractGap` | codegen | partial (name when present, else `DataId` debug) | no | partial | `DataId` debug only when name provably absent |
| `EmitError::AccumulatorShapeMismatch` | codegen | yes (data symbol) | no | yes | no |
| Driver I/O (`cannot read`, `could not find kernels.rs`) | driver | yes (path) | path | **yes** (names the override flag) | no |
| Capability CWD-walk failure | driver | yes (backend) | the CWD that was walked | **yes** (`--capabilities`) | no |

\* "no span" means the error has no `line:col`; the span substrate
(`Spanned<T>`) does not reach ACFG nodes / `XferPlaceholder`s, so
post-link pass errors carry symbol provenance but not a source location.

---

## What was fixed in place (TASK-0455.06)

Tracker IDs leaked into several user-facing `Display` / message strings.
A `TASK-NNNN` reference is meaningless to anyone outside this repo's
tracker; it was stripped from the surfaced text and kept only in the
adjacent code comment and the variant docstring (where it is genuine
developer provenance). The diagnostic content (symbol + reason + a
concrete `Fix:`) was preserved or improved:

- `ContractError::TypeMismatch` aggregate message — dropped `see
  TASK-0012 follow-ups` (`nucleus/nucleus-compiler/src/contract.rs`).
- `SyncInjectError::UncoveredCrossPartitionReducer` — dropped
  `TASK-0268` / `TASK-0365` / `TASK-0281`; added a concrete transfer-
  directive fix hint (`nucleus/nucleus-compiler/src/passes/sync_inject.rs`).
- `TransferInjectError::SameSetSilentElisionRisk` — dropped `TASK-0324`
  / `TASK-0325`; kept the data name + a fix hint
  (`.../transfer_inject/elision.rs`).
- `TransferInjectError::CumulativeWholeArrayFallback` — dropped
  `TASK-0366`; kept the `xN` risk + a `Fix:` hint
  (`.../transfer_inject/tiles.rs`). The test pin was updated to assert
  *absence* of any tracker ID plus presence of the fix hint.
- `EmitError::UnsupportedFeature` (MPI multi-worker check-loop) — dropped
  `Forward-linked to TASK-0045.03`
  (`nucleus/backend-common/src/mpi_plan/plan.rs`).

Wave-7 review fold-in (the architect found the de-ID sweep had missed
sibling arms of the SAME Display impls it edited — the recurring
silent-sibling class, plus this table's own rows were wrong until the
fold-in made them true):

- `SchedLowerErrorKind::{BlockPipelineConflict, UnrollNotDivisibleByBlock,
  CheckOnStripMinedLoop}` — dropped `TASK-0215`/`TASK-0144`/`TASK-0220`
  from the message strings (IDs kept in adjacent code comments).
- `PartitionBlocks2dError` (both arms) — dropped `TASK-0259`/`TASK-0262`.
- `SidecarError` (duplicate-loop-var arm) — dropped `TASK-0171`; row
  added to the table above (it was missing entirely — the documented
  coverage-undercount pattern, caught by the review).

`EmitError`'s own `Display` (in `backend-common/src/render/error.rs`) was
already clean — the leaks were all in the `String` payloads built at the
throw sites.

---

## Known-acceptable `Debug` forms

Some `ContractGap` sites print `data id {d:?}` — but ONLY where the
`NameTables` provably has no name for that `DataId` (the message is
literally "has no name in NameTables"). There is no resolvable name to
print, so the `Debug` form is the only available identifier; this is
acceptable. The MPI plan sites that print `({did:?})` already print the
resolved `name` alongside it, so the `Debug` id is a secondary technical
detail, not the primary identifier.

---

## Filed follow-ups

The expensive gaps — the ones needing real span-threading work, not a
one-line string edit — are filed as follow-up tasks:

- **Thread source spans onto post-link pass errors.** Every error in the
  `BuildAcfgError` -> `TransferInjectError` chain names its offending
  symbol but carries no `line:col`, because the `Spanned<T>` substrate
  (TASK-0082) never reaches ACFG nodes / `XferPlaceholder`s. Filed:
  **TASK-0455.06.01**.
- **Resolve `DataId` to a name in
  `TransferInjectError::CumulativeWholeArrayFallback`.** The throw site
  (`rewrite_cumulative_band_tiles`) has no `NameTables` threaded, so it
  prints `DataId(N)` debug. Thread the name table (or a `DataId -> name`
  view) to that pass. Filed: **TASK-0455.06.02**.
- **Resolve transition IDs in the Petri soundness-gate errors.** The
  `BoundednessError` / `DeadlockError` arms print `{:?}` transition ids,
  which are internal-engine identifiers. Filed: **TASK-0455.06.03**.
