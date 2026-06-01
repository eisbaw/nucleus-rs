---
id: TASK-0413
title: >-
  Drop dead tcp_plan::XferId pub re-export + tighten over-wide tcp_plan::Plan
  field visibility (TASK-0412 cycle architect P2 silent-sibling)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 19:51'
updated_date: '2026-06-01 20:29'
labels:
  - tooling
  - dead-code
  - backend-common
  - silent-sibling
  - cycle-0412-followup
dependencies:
  - TASK-0412
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P2 silent-sibling finding from the TASK-0412 review (commit 947f7de). TASK-0412 dropped the dead ChanId pub re-export from event_plan+mpi_plan. The THIRD Plan substrate, nucleus/backend-common/src/tcp_plan/mod.rs:53, does `pub use plan::{Plan, XferId}` — and XferId (`pub type XferId = usize` at plan.rs:34) has ZERO external consumers (no backend references tcp_plan::XferId; grep-verified). So the XferId re-export is dead, exactly like ChanId was.

BUT XferId is NOT a trivial sibling of ChanId (this is why TASK-0412 correctly left it out of scope): tcp_plan::Plan is a `pub` struct whose field `pub xfer_ids: BTreeMap<DataId, XferId>` (plan.rs:~45) exposes XferId through genuinely-pub API. Narrowing `pub type XferId` to pub(crate) WITHOUT first tightening that field would be an E0446 private-in-public error. By contrast ChanId's exposing fields were already pub(crate)/private (event_plan field hygiene), which is why TASK-0412 could narrow freely.

Separately the architect noted tcp_plan::Plan looks over-widened: the whole struct is `pub` with all-pub fields, and the external consumers (mp-tcp-poll, mp-tcp-bufsync) do NOT reference `.xfer_ids` (grep `.xfer_ids` in nucleus/backends/ is empty) — unlike event_plan's deliberately pub(crate) field hygiene.

## Scope
1. Audit tcp_plan::Plan's pub field set vs what the mp-tcp-{poll,bufsync} backends actually read; tighten unused-externally fields to pub(crate) to match the event_plan precedent TASK-0412 cites.
2. Once xfer_ids (and any other XferId-exposing pub item) is pub(crate), drop XferId from the `pub use plan::{Plan, XferId}` re-export and narrow `pub type XferId` to pub(crate).
3. Verify: cargo build --workspace + clippy + cargo doc --workspace --no-deps warning count UNCHANGED (memory: feedback-visibility-tighten-doclink-trap — the just-ci gate does NOT build docs, so a narrowed doc-linked symbol breaks links silently). Run cargo doc before/after and diff the warning count.

## Honest scope / risk
- Higher risk than TASK-0412: this touches a genuinely-pub struct field set, not just a dead alias. The field-visibility audit (step 1) is the real work; do NOT narrow a field a backend actually reads.
- LOW / OPTIONAL: pure dead-surface + visibility hygiene, zero functional effect. Same class as TASK-0411 (dead-reexport removal) and TASK-0412 (ChanId). Do NOT narrow asymmetrically without the field audit.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DONE (commit 472e88b). Dropped dead tcp_plan::XferId re-export + narrowed pub type XferId -> pub(crate); tightened 7 over-wide tcp_plan::Plan fields (per_worker, names, sidecar, host_worker, xfer_ids, pair_tiles, accumulate_waits) to pub(crate), keeping used_workers pub (the ONLY field read across the crate boundary). All 3 Plan substrates (event_plan/mpi_plan/tcp_plan) now `pub use plan::Plan;` only.

VERIFIED: cargo build --workspace OK (E0616 would fire on any missed external field read — clean => none); clippy backend-common+mp-tcp-{poll,bufsync} clean; cargo doc --workspace --no-deps warning count UNCHANGED (14); 153 tests / 0 fail across the 3 crates; no [`XferId`] intra-doc-link (doc-link trap N/A). Zero functional effect (visibility cannot affect codegen; byte-identity confirmed by the passing poll-vs-bufsync emit differential test).

REVIEW GATE: mped-architect read-only GO. Independently reproduced (forced rebuild zero-warning + byte-identity test). Confirmed: both shims construct via Plan::build (not struct-literal => construction-safe), read only used_workers + pub methods; all 7 narrowed fields have in-crate readers (no dead_code); used_workers correctly left pub.

DOCSTRING ACCURACY (orchestrator double-checked the architect P3-2): the new struct docstring claims the shims drive the Plan through pub fn build/render_worker_program/render_run_sh/worker_name + read used_workers. VERIFIED ACCURATE by grepping ACTUAL call syntax in poll/bufsync src — those are exactly the 4 methods + 1 field the shims use. NOT a doc-lie.

ARCHITECT P3-1 (method-hygiene) -> filed TASK-0414. The architect named 3 internal-only pub fn (data_name/non_host_workers/ctrl_var) but UNDER-COUNTED: there are ~9 internal-only pub fn on Plan + the encode/walkers free fns. Deliberately NOT folded here (beyond TASK-0413 FIELD scope; needs a careful per-symbol external-caller audit). Gotcha forward-carried: Plan::non_host_workers/max_payload_bytes appear in a bufsync COMMENT backtick-span but are NOT external callers — audit by actual call syntax, not bare mentions.
<!-- SECTION:NOTES:END -->
