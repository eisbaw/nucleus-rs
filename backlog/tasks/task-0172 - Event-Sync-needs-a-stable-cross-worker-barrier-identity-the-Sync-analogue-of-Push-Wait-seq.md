---
id: TASK-0172
title: >-
  Event::Sync needs a stable cross-worker barrier identity (the Sync analogue of
  Push/Wait seq)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 00:08'
updated_date: '2026-05-19 19:19'
labels:
  - M2
  - backend
  - contract
dependencies:
  - TASK-0124
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Discovered by TASK-0124. Event::Push/Wait carry a stable cross-worker SeqTag; Event::Sync carries only participants+kind, NO stable cross-worker identity. The pre-TASK-0124 multi_worker recovered barrier identity from a GLOBAL acfg tree walk (walk_assign_sync_ids); the EventList-only path cannot recover that from disjoint per-worker lists in general. TASK-0124's multi_worker assigns barrier id by per-worker PRE-ORDER Sync index, which is byte-identical to the old ids ONLY for UNIFORM barriers (every Sync has the same participant set — true for 02-split's three {host,w0} barriers). It VALIDATES uniformity and fails loud (EmitError::ContractGap) on a partial/non-uniform-barrier schedule rather than emit a wrong barrier graph. The robust fix is to give Event::Sync a stable id (the Sync analogue of TASK-0156 FireBinding / TASK-0159 Event::Loop / Push-Wait seq) so partial-barrier multi-worker schedules can be lowered correctly. Until then, partial-barrier multi-worker is a typed codegen error, not a wrong binary.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Event::Sync carries a stable cross-worker barrier identity (or an equivalent join key) so disjoint per-worker EventLists agree on barrier identity without a global ACFG walk
- [x] #2 pthreads-sync multi_worker uses that identity instead of the per-worker pre-order-index heuristic
- [x] #3 a partial/non-uniform-barrier multi-worker schedule lowers correctly (no ContractGap rejection)
- [x] #4 e2e 02-split + determinism stay byte-identical
- [x] #5 Full gate stays green incl. CROSS-BACKEND: just e2e EXACTLY 30/26/0/4/0, just determinism-check byte-identical x2, just determinism-check-negative + just xbackend-check-negative both still bite, clippy --workspace --all-targets clean, ci exit 0; 02-split byte-identical on BOTH pthreads-sync AND mp-tcp-bufsync (the contract is consumed by both backends — the new SyncTag must not perturb either's uniform-barrier output)
- [x] #6 The SyncTag is assigned where the GLOBAL barrier structure is visible (sync-injection pass / SyncPlaceholder, analogue of how XferPlaceholder.seq is assigned) and threaded through petri_to_events into Event::Sync — disjoint per-worker EventLists agree on barrier identity with NO backend global ACFG walk; Event's manual Hash impl (event.rs ~661) includes the new field; serde (if Sync is serialized in the sidecar/contract) updated
- [x] #7 AC#3 is proven by a GENUINE multi-worker partial/non-uniform-barrier test (Syncs with differing participant sets) that previously hit EmitError::ContractGap and now lowers to correct barrier code (multi-worker path — single-worker ignores the injected ACFG per the known backend limitation); the ContractGap + pre-order-index heuristic + uniformity-validation are removed, not bypassed
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
DESIGN: distinct SyncTag(pub u64) newtype in event.rs (mirrors SeqTag exactly: Debug/Clone/Copy/PartialEq/Eq/Hash/PartialOrd/Ord + serde transparent). Barriers and transfers are different identity domains; reusing SeqTag would conflate two id spaces and let a barrier alias a transfer in any future seq-keyed code. Distinct type is the conservative, type-safe choice.

ASSIGNMENT SITE: sync_inject pass. Add SyncPlaceholder.sync: SyncTag. Thread a deterministic monotonic counter (mirror transfer_inject State.fresh_seq / next_seq) through the inject_in_node/inject_in_sequence/wrap_repeat_body walk in DETERMINISTIC pre-order (same program -> same ids; no HashMap/HashSet iteration in assignment — the walk is over Vec children + BTreeSet participants, fully ordered). The 3 SyncPlaceholder construction sites each take a fresh tag. This is the GLOBAL barrier-structure-visible site, exactly analogous to XferPlaceholder.seq.

KEY CORRECTNESS POINT: a single barrier emitted by emit_sync is ONE SyncPlaceholder cloned into each participant EventList -> they all carry the SAME SyncTag (the cross-worker join key). Disjoint per-worker lists agree with NO global walk. This is the entire point of the task.

THREAD-THROUGH: petri_to_events.rs emit_sync copies s.sync into Event::Sync { sync: ... } (copy the seq:x.seq pattern). acfg_to_petri emit_sync needs no change (analysis net, no Event).

EVENT CONTRACT: add sync: SyncTag field to Event::Sync; add to manual Hash impl (event.rs ~661) — hash it like seq; serde derives auto-cover the new field (externally-tagged, new field added — note wire-format change, no serde(default) since every producer emits it now). Update event.rs module-doc bullet, the Event::Sync doc, PRD §8.3 Sync line, petri_to_events doc.

MULTI_WORKER (pthreads-sync): replace pre-order-index BarrierId with SyncTag. collect_barriers_preorder -> collect by SyncTag; barrier_participants keyed by SyncTag; emit bar_<tag>; render Sync arm uses ev.sync not *sync_idx. REMOVE the uniformity validation + the non-uniform ContractGap (keep EmitError::ContractGap variant — used widely elsewhere). Rewrite module doc ~28-53 to the contract-carried-id reality (no stale heuristic lie). For uniform 02-split: with deterministic pre-order tag assignment the three {host,w0} barriers get tags 0,1,2 — same numbering the old pre-order index produced -> byte-identical bar_0/1/2 names (must verify).

MP-TCP-BUFSYNC: same — use ev.sync as the barrier_cross token + identity; remove its non-uniform ContractGap + uniformity validation; KEEP the host-must-be-participant ContractGap (genuine topology limit, TASK-0175, NOT a partial-barrier rejection). Rewrite its inherited-caveat doc lines 45-57.

AC#3 TEST: add multi-worker partial-barrier test in pthreads-sync tests/multi_worker.rs (3 workers: host produces a,b; w0 consumes a; w1 consumes b; host sinks — yields barriers with differing participant sets {host,w0} and {host,w1}). Assert it now lowers (no ContractGap) AND the barrier wiring is correct (Barrier::new with right counts, right workers .wait() the right bar_<tag>). Must be multi-worker path. Also add a compiler-level assertion that two Syncs with differing participants carry distinct SyncTags and a shared barrier carries one tag across participants.

GATE per commit: just determinism-check x2 (30/26/0/4) BOTH backends byte-identical; just e2e 30/26/0/4/0; determinism-check-negative + xbackend-check-negative both bite; cargo clippy --workspace --all-targets -D warnings; just ci exit 0; just test counts + migrated-tests strength preserved.

COMMITS: (a) contract: SyncTag + Event::Sync field + Hash + SyncPlaceholder + sync_inject assignment; (b) petri_to_events thread-through; (c) pthreads-sync multi_worker consume + remove heuristic/ContractGap + doc; (d) mp-tcp-bufsync consume + doc; (e) partial-barrier test. Full gate before each.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED (commits b023e08, dfc00b7, 8d36e33, 46a00ec on master; PRD doc folded into dfc00b7).

DESIGN DECISIONS + RATIONALE:
- Distinct SyncTag(pub u64) newtype, NOT a reused SeqTag. Barriers and transfers are different identity domains; sharing the space would let a barrier alias a transfer in any seq-keyed code path. SyncTag mirrors SeqTag derives + serde(transparent), PLUS Default (SeqTag is not Default) — needed only so SyncPlaceholder keeps its Default derive; the SyncTag(0) default is never semantically meaningful (always overwritten by assign_sync_tags). Documented inline.
- ASSIGNMENT SITE: a deterministic post-pass assign_sync_tags() in sync_inject, walking the FINAL injected tree in pre-order. NOT during injection: the injection walk creates Syncs out of final-tree order (recurses children before inserting a Sequence-boundary Sync; insert(0,..)s the Repeat-entry Sync), so creation order != final-tree order. Final-tree pre-order == petri_to_events::walk order == the order each participant meets the barrier in its projected EventList == the order the OLD backend pre-order-index heuristic used. Hence a uniform-barrier program gets the same 0,1,2 numbering -> generated code byte-identical (verified). Walk is over Vec children + BTreeSet participants only (no HashMap/HashSet) -> reproducible.
- THREAD-THROUGH: petri_to_events::emit_sync copies s.sync into every participant Event::Sync (mirrors seq:x.seq). Manual Event Hash includes sync. serde derives auto-cover it; new REQUIRED field (no serde(default)) — internal stage-to-stage wire format, golden tests regenerated, which the determinism gate verifies.

MULTI_WORKER (both backends): barrier id is now Event::Sync.sync. pthreads-sync keys barrier_participants by SyncTag, emits bar_<tag.0>, render uses ev.sync; removed barrier_count, sync_idx threading, uniformity validation, non-uniform ContractGap. collect_barriers_preorder -> infallible collect_barriers_by_tag. mp-tcp-bufsync: same; the wire barrier_cross token is now sync.0 (host+worker agree by construction). EmitError::ContractGap variant RETAINED (used widely elsewhere, incl. unrelated single-worker-Sync reject and the mp-tcp host-must-participate TASK-0175 limit, which is re-documented as SEPARATE, not a partial-barrier reject).

MIGRATED TESTS (strength preserved/increased): compiler/tests/event.rs sample_sync now SyncTag(7); sync_constructor_smoke ADDS assert_eq!(sync, SyncTag(7)); serde_roundtrip_sync now also exercises the new field; order-irrelevant-eq test gives both Syncs equal tags. petri_to_events.rs / acfg_to_petri.rs Sync sites use ..Default::default() / `..` (participants/kind still asserted). No assertion weakened.

AC#3/#7 EVIDENCE: new pthreads-sync test partial_nonuniform_barrier_multi_worker_lowers_correctly. 3-worker (host,w0,w1) pipeline; sync_inject yields THREE barriers {host,w0}=tag0, {w0,w1}=tag1, {host,w1}=tag2 (verified by debug dump). Under OLD heuristic: w0 events [Sync{host,w0},Wait,Fire,Push,Sync{w0,w1}] idx0={host,w0}; w1 [Sync{w0,w1},Wait,Fire,Push,Sync{host,w1}] idx0={w0,w1} -> barrier#0 participant disagreement -> EmitError::ContractGap (genuinely the formerly-broken path). Now: each tag independent; test asserts carriers==participants per tag, 3 distinct tags, correct per-worker bar_<tag> wiring incl. the host-EXCLUDING {w0,w1} barrier (+ host does NOT wait it), builds + runs the 3-worker binary -> correct value 124. Multi-worker path (single-worker ignores injected ACFG per known limitation).

GATE (ACTUAL, all inside nix develop): workspace cargo test 426 passed / 0 failed; e2e 30/26/0/4 required-fail 0; determinism-check 30/26/0/4 byte-identical RUN x2 (across 4 separate runs this session), 02-split byte-identical on BOTH pthreads-sync AND mp-tcp-bufsync (explicit AC#5 cross-backend check observed); determinism-check-negative + xbackend-check-negative BOTH still bite; clippy --workspace --all-targets -D warnings clean; just ci exit 0.

GOTCHA/LIMITATION: qa-test-runner & mped-architect subagents (per CLAUDE.md) are NOT available as tools in this environment; ran the full equivalent verification gate manually + self-reviewed against MPED principles (no comment-lie: every touched doc rewritten to new reality; no panic-not-diagnostic regression per decision-0003; removed a workaround not a real guard). Honest: dual-agent review gate not executed because the agents are unavailable here, not skipped by choice.

ORCHESTRATOR review-gate close (phase3-ralph, BOTH reviewers GO — the deepest-regression cycle of the segment, dual gate run in full): qa-test-runner (independently re-ran, not trusted): determinism-check byte-identical x2 BOTH backends, e2e EXACTLY 30/26/0/4/0, determinism-check-negative + xbackend-check-negative BOTH still bite, AC#3 partial_nonuniform_barrier_multi_worker_lowers_correctly is a GENUINE 3-worker host-excluding {host,w0}/{w0,w1}/{host,w1} test that previously ContractGap-d and now lowers correctly (binary -> 124, host explicitly NOT waiting {w0,w1}), Event::Sync manual Hash hashes the new sync field, NO persisted Event JSON fixtures (serde bump in-memory only), pre-order heuristic+uniformity-validation+partial-barrier ContractGap REMOVED from BOTH backends (grep-proven, retained ContractGap only for genuinely-other cases incl. pre-existing TASK-0175 mp-tcp host-mediation), workspace 426/0, migrations strength-preserved/strengthened, clippy --all-targets clean, ci exit 0, assign_sync_tags deterministic (Vec/Box walk, no hash-order), no new panic. mped-architect (architecture+honesty, re-derived from code): SyncTag join-key invariant PROVABLY holds for ALL representable schedules incl. partial/non-uniform — tag is a per-barrier-node property assigned ONCE by a monotonic counter threaded through the final-tree walk; emit_sync clones the SAME (participants,sync) to every participant => backends first-sighting-wins is provably exact, the removed cross-worker validation was re-checking a now-single-source-of-truth-guaranteed invariant (correct MPED architecture); uniform byte-identical is a STRUCTURAL THEOREM (assign_sync_tags ≡ petri_to_events::walk identical pre-order) not coincidence — holds for ANY uniform program not just the e2e matrix; removed ContractGap genuinely safe (single/empty barriers elided upstream by participants.len()>=2; no other wrong-codegen path exposed); distinct SyncTag newtype vs SeqTag correct (separate identity domains, prevents aliasing); manual Hash consistent with derived Eq; serde required-field bump architecturally acceptable (no persisted/cross-process Event; mp-tcp sends sync.0 as bare u64 not a serialized struct) and correctly FLAGGED not hidden; both backends coherent (cross-backend differential stays GREEN); PRD §8.3 + multi_worker module doc rewritten to the new reality with zero stale "pre-order index" residue (comment/doc-lie class NOT repeated); all 7 ACs honestly met, none perfunctory/overclaimed. No NO-GO from either; no follow-up required (serde sharp-edge currently-safe + already in notes; petri_to_events `..` test coverage adequate-but-narrow per the dedicated AC#3 test, not a regression). TASK-0172 Done stands — the long-deferred keystone Event-contract barrier-identity is RESOLVED; partial/non-uniform multi-worker barriers now lower correctly.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Gave Event::Sync a stable cross-worker barrier identity (SyncTag) — the Sync analogue of Push/Wait SeqTag — so partial / non-uniform barrier multi-worker schedules lower correctly instead of being a typed codegen error.

WHAT CHANGED:
- compiler/event.rs: new distinct SyncTag(u64) newtype (SeqTag-shaped + serde transparent, plus Default so SyncPlaceholder keeps its derive); sync: SyncTag field on Event::Sync; field added to the manual Event Hash impl; module/Event docs updated.
- compiler/acfg.rs: sync: SyncTag on SyncPlaceholder.
- compiler/passes/sync_inject.rs: assign_sync_tags() — deterministic pre-order walk of the FINAL injected tree hands out monotonic tags (the analogue of XferPlaceholder.seq assignment; final-tree pre-order == projection order == old heuristic order, so uniform programs stay byte-identical).
- compiler/passes/petri_to_events.rs: emit_sync copies s.sync into every participant Event::Sync (mirrors seq:x.seq) — disjoint per-worker lists agree on barrier identity with NO global ACFG walk.
- backends pthreads-sync/multi_worker.rs + mp-tcp-bufsync/lib.rs: barrier id is now the contract SyncTag. Removed the per-worker pre-order-index heuristic, the uniform-barrier validation, and the non-uniform/partial-barrier EmitError::ContractGap (removed, not bypassed). collect_barriers_preorder -> infallible collect_barriers_by_tag; dropped barrier_count + sync_idx threading. The EmitError::ContractGap VARIANT is retained (still used for other genuine gaps incl. single-worker-Sync and the unrelated mp-tcp host-mediation TASK-0175 limit, re-documented as separate). Stale heuristic module docs rewritten to the contract-carried reality. PRD §8.3 updated (Sync now lists sync: SyncTag + prose on the partial-barrier consequence).
- pthreads-sync/tests/multi_worker.rs: partial_nonuniform_barrier_multi_worker_lowers_correctly — a 3-worker pipeline producing three barriers with three different participant sets (incl. a host-excluding {w0,w1}) that previously hit ContractGap; asserts the SyncTag is a genuine join key (carriers==participants per tag), distinct barriers => distinct tags, correct per-worker bar_<tag> wiring, and the generated 3-worker binary builds + produces the correct value.

USER IMPACT: partial/non-uniform multi-worker barrier schedules now compile to correct code on the pthreads-sync tier-1 backend instead of being rejected; uniform schedules (the tier-1 examples) are unaffected (byte-identical). mp-tcp-bufsync handles the host-mediated subset correctly; host-excluding barriers remain a typed limitation there (TASK-0175), now clearly separated from partial-barrier identity.

TESTS RUN (actual): workspace cargo test 426 passed / 0 failed; e2e 30/26/0/4 required-fail 0; just determinism-check 30/26/0/4 byte-identical x2 with 02-split byte-identical on BOTH pthreads-sync AND mp-tcp-bufsync; just determinism-check-negative + just xbackend-check-negative both still bite; clippy --workspace --all-targets -D warnings clean; just ci exit 0.

RISKS/FOLLOW-UPS: new required serde field on Event::Sync (internal stage-to-stage wire format; golden tests regenerated and gated by the determinism check). Honest limitation: the CLAUDE.md qa-test-runner / mped-architect review subagents are not available as tools in this environment — the full equivalent verification gate was run manually and a MPED-principle self-review performed (no comment-lie, no panic-not-diagnostic regression, workaround removed not a real guard); the dual-agent gate itself was not executed because the agents are unavailable here.
<!-- SECTION:FINAL_SUMMARY:END -->
