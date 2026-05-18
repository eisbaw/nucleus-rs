---
id: TASK-0015
title: Define Event types (Fire/Alloc/Push/Wait/Sync/Free)
status: Done
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 01:15'
labels:
  - M1
  - ir
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define the EventList contract from PRD §8.3. Six event variants. IterTile, Region, SyncKind, KernelId, DataId, WorkerId, SeqTag as supporting types.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes Event enum with 6 variants matching PRD §8.3 exactly.
- [ ] #2 IterTile is Vec<(IterVar, Range<i64>)>.
- [ ] #3 Region is an opaque newtype (an id assigned by the scheduler).
- [ ] #4 SyncKind has exactly one variant: Barrier.
- [ ] #5 Test: Event has Debug, Clone, PartialEq derives; round-trip through serde where useful.
- [ ] #6 Implementation notes record design questions (e.g. should Region carry the memory-region-id name for inspection, or remain pure opaque).
- [ ] #7 Implementation notes record honest limitations (e.g. no end-to-end-latency event; check directive's measurement points are TBD at this stage).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Design questions (recorded)

1. **Opaque u64 IDs vs interned strings**. Chose u64 newtypes for
   KernelId, DataId, WorkerId, IterVar, SeqTag, Region. Rationale:
   cheap eq/hash, no lifetime in the wire format, and the schedule
   pass already assigns indices. The human-readable name lives in a
   sidecar map (out of scope here). Trade-off: error messages need
   the sidecar to be human-readable.

2. **serde default-on vs feature-gated**. Default-on with the
   `serde` feature in compiler/Cargo.toml. Rationale: events ARE the
   inter-stage wire format (schedule -> backend codegen) and the
   shape of golden-test fixtures. Default-on so the common path is
   zero-friction; opt-out exists for size-constrained builds that
   don't need (de)serialisation. Pinned serde v1.0.219 to match the
   project's pinned-minor discipline (see syn/chumsky pins).

3. **SyncKind as enum despite a single variant**. Kept as enum
   (`enum SyncKind { Barrier }`), not a unit struct. PRD §8.3 says
   Rendezvous/Quorum may earn slots later; the enum shape means
   adding a variant doesn't break the public API. Costs one match
   arm in callers today.

4. **IterTile as named struct, not type alias**. Named struct wraps
   `Vec<(IterVar, Range<i64>)>` so we can hang `is_empty`, `rank`,
   `new`, `empty`, document the iteration-nest-order invariant, and
   add a manual Hash impl (Range<i64> doesn't implement Hash). The
   element type is the literal tuple, so pattern matching stays
   cheap.

5. **Sync.participants uses BTreeSet, not HashSet**. PRD types it
   as Set<WorkerId>; concretely we need (a) stable iteration order
   for deterministic codegen and (b) Hash for Event: Hash. HashSet
   gives neither.

6. **Region as opaque u64, not the memory-region name (AC #6
   directly asks)**. Decision: pure opaque, no name carried. The
   PRD §8.3 calls Region a backend-interpreted handle that the
   compiler doesn't know the representation of. Inspection tooling
   that wants `"shared_sram"` for a Region(2) goes through the
   schedule-side sidecar. Filed TASK-0104 for the inspection-side
   name map.

## Honest limitations

1. **Range bounds are i64**. PRD doesn't lock the integer type;
   schedule examples have plain integer arithmetic with negative
   intermediates (halo offsets). i64 is permissive. If the
   downstream backend wants u64 it can normalise.

2. **No span/source-location info on events**. Diagnostics that
   need to point at the algo or schedule line that produced an event
   need a sidecar. Filed TASK-0105.

3. **No latency/measurement event variant**. PRD §6.3.5's `check`
   measurement points are TBD; not modelled here. Filed TASK-0106.

4. **No validation in this module**. Push.dst != self, matched
   (src,dst,data,tile,seq) Push/Wait pairs, non-empty
   Sync.participants — all the responsibility of the
   *scheduler/validator* that constructs events. The type module
   is a contract, not a validator. Filed TASK-0107.

5. **No trait-surface exhaustiveness tests**. We test Debug-by-use,
   Clone-by-use (implicitly via roundtrip helpers), PartialEq, Hash,
   serde. We do NOT explicitly test Send/Sync/Ord-on-newtypes/Default
   coverage. Filed TASK-0108.

6. **No "invalid tile" doc**. The PRD doesn't define what counts as
   invalid (e.g. start >= end, duplicate IterVar in bounds). We
   neither reject nor describe such tiles. Construction-side
   validation is the scheduler's job; the type allows anything that
   typechecks.

7. **Externally-tagged JSON is the default serde shape**. Backends
   parsing this directly will see `{"Fire":{...}}` etc. We do not
   pin this in a snapshot test (too brittle to serde renaming
   knobs); one spot-check asserts the top-level tag prefix only.

## AC verification

- AC #1 (6 variants matching PRD §8.3 exactly): MET.
  event::Event has Fire, Alloc, Push, Wait, Sync, Free with the
  exact field names from PRD §8.3.

- AC #2 (IterTile is Vec<(IterVar, Range<i64>)>): MET.
  IterTile::bounds has type Vec<(IterVar, Range<i64>)>; named struct
  wrapper documented in module docs.

- AC #3 (Region is an opaque newtype, scheduler-assigned id): MET.
  Region(pub u64). Module docs explicitly call out
  "compiler-assigned, opaque to backend at the type level."

- AC #4 (SyncKind has exactly one variant, Barrier): MET.
  Kept as enum with the single Barrier variant for forward
  compatibility.

- AC #5 (Debug, Clone, PartialEq derives + serde round-trip): MET.
  All event types derive Debug/Clone/PartialEq/Eq/Hash (Hash is
  manual on IterTile because Range<i64> isn't Hash). Serde
  Serialize/Deserialize derives behind the default-on `serde`
  feature. Tests at nucleus/compiler/tests/event.rs:
  serde_roundtrip_{fire,alloc,push,wait,sync,free,empty_tile}.

- AC #6 (design questions in notes): MET — section above.

- AC #7 (honest limitations in notes): MET — section above.

## Verification

just check  -> pass
just clippy -> pass (-D warnings, one doc_lazy_continuation fixed)
just test   -> pass (119 = 94 prior + 25 new event tests)
just e2e    -> pass (stub binary)

## Follow-up tasks filed

- TASK-0104: schedule-side sidecar maps id->name for inspection.
- TASK-0105: span/source-location info on events for diagnostics.
- TASK-0106: latency/measurement event variant once §6.3.5 settles.
- TASK-0107: scheduler-side validation of event invariants.
- TASK-0108: exhaustive trait-surface tests for event types
  (Send/Sync/Ord/Default).
<!-- SECTION:NOTES:END -->
