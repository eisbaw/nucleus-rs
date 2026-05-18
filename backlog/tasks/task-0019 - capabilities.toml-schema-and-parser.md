---
id: TASK-0019
title: capabilities.toml schema and parser
status: Done
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 01:59'
labels:
  - M1
  - backend
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §7.4: each backend declares its capabilities as a sibling text file. Define the schema and write a parser; the schedule-vs-backend compatibility check uses this.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs/capabilities-toml.md describes the schema: transport, notify, supports_async, supports_buffer, max_buffer, worker_classes, memory_regions.
- [ ] #2 compiler crate parses capabilities.toml from each backend crate's root.
- [ ] #3 Mismatch between schedule demand and backend capability is a compile-time error with the offending field named.
- [ ] #4 Test: every backend's capabilities.toml round-trips through the parser.
- [ ] #5 Test: a curated set of schedule/backend pairs is checked; expected acceptances and rejections both verified.
- [ ] #6 Implementation notes record design questions (e.g. how to express forward-compatible capability extensions).
- [ ] #7 Implementation notes record honest limitations (e.g. schema cannot express conditional capabilities like 'async only when buffer>=2').
- [ ] #8 1,2,3,4,5,6,7
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions

- **Transport as closed enum vs free string.** Closed enum (serde
  rename_all = "kebab-case"). Loud failure on typo (`tpc` -> error).
  Trade-off: adding a transport (e.g. `rdma`) requires compiler
  recompile. Acceptable since the set is small and PRD-listed.

- **`NotifyMode` is a superset of `sched::NotifyKind`.** PRD §6.3.4
  only lists `event`/`poll` as schedule surface; capabilities also
  accept `barrier`/`blocking`/`irq`. Lets a backend declare modes
  the schedule cannot yet request. Conversion is one-way: the schedule
  uses `NotifyKind::From -> NotifyMode` on the capability check side.
  If a backend wants a truly custom mode, add a variant — string
  free-form was rejected for the same reason as transport.

- **Conditional capabilities.** Cannot express things like "async
  works only when buffer >= 2" or "notify=event only when
  transport=tcp". Documented in the schema doc and limitations
  section. Filed as TASK-0121.

- **Schema versioning.** No `schema_version` field. Filed as TASK-0120.
  For now, `deny_unknown_fields` is loud on additions.

- **String-scraping in error classification.** `toml`/serde does not
  expose a structured "which enum variant failed" hook. The
  `classify_de_error` function pattern-matches on the error message
  to route to UnknownTransport / UnknownNotify / MissingField /
  UnknownField. Worst case it falls through to `ParseFailed` carrying
  the verbatim message. Fragile but localised; if `toml` >= 0.8 lands
  in MSRV later, the classification can be tightened.

- **MSRV constraint forced `toml@=0.7.8`.** Cargo 1.83.0 doesn't
  support edition2024, which `toml@0.8+` transitively requires
  (indexmap >= 2.7). Pinning `toml = "=0.7.8"` and `indexmap = "=2.6.0"`
  in Cargo.lock keeps things buildable. Bumping MSRV unblocks newer
  toml.

- **Tempfile vs hand-rolled temp file.** Same MSRV issue as toml's
  transitive deps — `tempfile` pulls in `getrandom@0.4.x` (edition2024).
  Avoided the dep by hand-rolling a 30-line `TempToml` RAII wrapper
  in the tests. Trade-off: less polished than `tempfile`, but zero
  added dependencies.

- **Synthetic default class -> "default" on the capability side.**
  Schedule simple-form workers get class `__default` in SchedIR
  (PRD §6.3.1, TASK-0010). On the capability side, "default" is the
  PRD §7.4 example value. The check translates one to the other in
  `check_schedule_compat`. Documented in schema doc and code.

## Honest limitations

1. **Conditional capabilities can't be expressed.** The flat-flag
   shape rejects "async only when buffer >= 2" etc. Will need a
   `restrictions = [ ... ]` block or similar. TASK-0121.

2. **No schema_version.** Forward-compatible field additions are
   *rejected* today (`deny_unknown_fields`). Need version-gating
   to relax. TASK-0120.

3. **Error classification scrapes message strings.** Fragile;
   tightly coupled to toml/serde's message format. Tests cover the
   patterns we route on; outside that, errors fall through to
   `ParseFailed` carrying the verbatim message.

4. **No transport-vs-schedule cross-check.** PRD §6.3 has no
   schedule surface that names a transport. The field is parsed and
   exposed for codegen to branch on but isn't validated against
   anything from the schedule.

5. **Worker-class translation is hard-coded "default" <-> __default.**
   If the schedule simple form ever gains a different synthetic name,
   or capabilities want to declare "__default" verbatim, the
   translation needs updating.

6. **No real backend's capabilities.toml exists yet to test against.**
   AC #4 ("every backend's capabilities.toml round-trips") is satisfied
   structurally — the round-trip tests parse-serialise-reparse a
   hand-built struct matching the PRD §7.4 example. Once tier-1
   backend crates land (M3 onward, TASK-0036) their capabilities.toml
   files become the real integration test. The capability-check pass
   is not wired into the build driver yet (TASK-0118 covers
   codegen-time integration).

7. **`Vec<NotifyMode>` / `Vec<String>` accept duplicates.** No
   uniqueness check — `notify = ["event", "event"]` parses, the
   `BTreeSet` lookup in `check_schedule_compat` folds duplicates
   anyway. Cosmetic; not filing.

## AC verification

- AC#1 — docs/capabilities-toml.md exists, covers transport, notify,
  supports_async, supports_buffer, max_buffer, worker_classes,
  memory_regions plus name and tier. Also documents the
  CapMismatch variants emitted by the check.

- AC#2 — `load_capabilities(path: &Path) -> Result<Capabilities,
  CapError>` reads a capabilities.toml. Exposed via `crate::capabilities`
  and re-exported from `compiler::lib`.

- AC#3 — `check_schedule_compat(caps, sched) -> Result<(), Vec<CapMismatch>>`
  reports one error per mismatched field; the offending data symbol /
  region / class name is named in the variant payload. Display impl
  produces a human message.

- AC#4 — Round-trip test (`round_trip_serialize_then_parse`) plus
  variant-coverage tests (`round_trip_all_notify_variants`,
  `round_trip_all_transport_variants`). No real backend capabilities.toml
  exists yet — the synthetic round-trip is the test surface until
  TASK-0036+.

- AC#5 — 9 negative compat tests + 3 positive cover one CapMismatch
  variant each plus the "all six at once" multi-error case. Positive
  tests verify clean pass on (a) default schedule, (b) all-features-on
  schedule, (c) buffer=1 with supports_buffer=false (edge case).

- AC#6 — Design questions captured here.

- AC#7 — Honest limitations captured here.

## Follow-up tasks filed

- TASK-0120: schema_version field for forward-compatible
  capabilities.toml evolution.
- TASK-0121: restrictions[] for conditional capability expressions
  (e.g. "async only when buffer >= 2").

## Verification

- `just check` — pass
- `just clippy` — pass (-D warnings)
- `just test` — pass (180+ tests including 21 new capabilities tests)
- `just e2e` — pass (stub at M0; no changes)

## Files touched

- docs/capabilities-toml.md (new)
- nucleus/compiler/src/capabilities.rs (new)
- nucleus/compiler/src/lib.rs (module + re-exports)
- nucleus/compiler/Cargo.toml (toml@=0.7.8 dep, serde no longer optional)
- nucleus/Cargo.lock (toml + indexmap@=2.6.0 pin via cargo update)
- nucleus/compiler/tests/capabilities.rs (new, 21 tests)
<!-- SECTION:NOTES:END -->
