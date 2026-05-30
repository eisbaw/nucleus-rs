---
id: TASK-0368
title: >-
  Reconcile PRD section 8 Petri-net framing with production reality
  (check_bounded / check_deadlock_free are production-dead)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-30 11:08'
updated_date: '2026-05-30 22:49'
labels:
  - docs
  - PRD
  - petri-net
  - honesty
  - cycle-213-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (F3, honesty). VERIFIED: check_bounded (passes/boundedness.rs) and check_deadlock_free (passes/deadlock.rs) have ZERO production call sites — every non-test reference is a doc-comment. acfg_to_net runs ONLY under the --emit-pn inspection branch (driver/main.rs). Net soundness in the shipping compiler is enforced STRUCTURALLY (TtoP-arc elision + ad-hoc ACFG guards), NOT by the Petri analyses. PRD section 8 bills the Petri net as the central technical contribution with "analyses fall out as standard properties; failures are compile errors" — true of the TEST suite, not the shipping path. This is a PRD-vs-code framing gap. Cross-ref TASK-0219 (dead-code status accepted; no wire-in task filed). DECISION task: pick one and execute.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decision recorded (in this task) between: (A) wire check_bounded + check_deadlock_free into the production compile pipeline as a real gate (acfg_to_net + analyses run on every build, failures => compile error), OR (B) downgrade PRD section 8 framing to "inspection/spec artifact; soundness enforced structurally" and update any other doc claiming the analyses are a shipping gate
- [x] #2 The chosen option is executed: either the wire-in lands with a test proving an unbounded/deadlocking net is REJECTED at build time, OR the PRD + related docstrings are corrected and a grep shows no remaining "compile error / shipping gate" claim about the Petri analyses
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
AC#1 DECIDED (user): Option A — wire check_bounded + check_deadlock_free into the production compile pipeline as a HARD gate. Orchestrator de-risk already ran the analyses in REPORT mode over 288 shipping nets (17 examples x schedules x 7 tier-1 backends); all passed bounded+deadlock_free, so the hard gate rejects nothing shipping (e2e must stay 322/265/0/57/0).

AC#2 EXECUTION PLAN:
1. nucleus-compiler: add a small new module passes/net_soundness.rs (pub fn check_net_sound(net: &Net) -> Result<(), PetriAnalysisError>) that calls derive_firing_order, then check_bounded, then check_deadlock_free, mapping each into typed enum PetriAnalysisError { Boundedness(BoundednessError), Deadlock(DeadlockError) } with Display. Re-export check_net_sound + PetriAnalysisError from lib.rs. Library function (not inline driver) so the negative test exercises the exact gate.
2. Driver wire-in at main.rs ~line 679-681: AFTER the emit_pn DOT block, BEFORE the out_dir dispatch. recompute let net = acfg_to_net(&acfg); then check_net_sound(&net).map_err(|e| format!("petri-net soundness check failed: {e}"))?; runs on EVERY build (--emit-pn and --out). Do not change emit_pn behaviour; no .unwrap()/.expect() on analysis path.
3. Negative tests (tests/net_soundness.rs): synthetic unbounded net (2-token push into cap-1 place) -> PetriAnalysisError::Boundedness; synthetic deadlocking net (unmatched wait) -> PetriAnalysisError::Deadlock. Templated from boundedness.rs two_token_push_into_cap1_place_is_rejected + deadlock.rs unmatched_wait_is_detected_as_deadlock. Docstring states honestly: provably-dead tripwire on shipping schedules (structural guards prevent any valid schedule producing a bad net); pinned at function level.
4. PRD/docstring reconcile (honest, no NEW overclaim): PRD section 8.1 diagram (~175-176), section 2 table (~81), section 8.2 (~750-754), section 8.4 (~867-870), section 8.6 (~897-899). State the analyses run as a per-build gate; keep the METHOD nuance: exact-replay over a deterministic firing order, sound for v2 restricted statically-ordered nets, NOT a general reachability/coverability engine. Update driver main.rs header pipeline summary to include the soundness gate. Tighten boundedness.rs e2e_example_02_split_never_overflows_capacity to assert Ok(()) (stale InvalidFiringOrder caveat resolved — gate now depends on it).
5. Gate: nix develop -c just build && just clippy && just test && just test-release && just e2e. e2e MUST stay 322/265/0/57/0; regression => wiring bug, diagnose not relax. Also run check-narrative-doc-lie since editing docs.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-215 IMPLEMENTATION (Option A wire-in). Commits 8903076 (code) + 8d0419d (PRD).

INSERTION POINT: driver/main.rs right AFTER the --emit-pn DOT block, BEFORE the out_dir let-else dispatch. Gate runs on EVERY build (--emit-pn-only AND --out) and AFTER the DOT dump so an unsound net still writes its DOT for debugging then errors. acfg_to_net recomputed (the emit_pn net is if-let-scoped); construction O(net), replay O(firing_order) - cheap.

NEW SURFACE: passes/net_soundness.rs::check_net_sound(&Net)->Result<(),PetriAnalysisError>; enum PetriAnalysisError{Boundedness(BoundednessError),Deadlock(DeadlockError)}+Display+Error. Re-exported from lib.rs. Driver stringifies via map_err(|e| format!("petri-net soundness check failed: {e}")).

KEY DESIGN SUBTLETY (caught by the negative test, NOT trusted from narrative): blindly composing check_bounded?;check_deadlock_free? MISLABELS a deadlocking net as Boundedness, because check_bounded runs first and returns InvalidFiringOrder on a stall. Fix: the gate maps only CapacityExceeded/UnknownTransition to Boundedness; InvalidFiringOrder (a stall, per that variant own docstring deadlock-territory-not-boundedness) FALLS THROUGH to check_deadlock_free which returns the precise Stalled, mapped to Deadlock. Fall-through is sound: if bounded returned InvalidFiringOrder the first FireError was NotEnabled, so deadlock replay hits the same stall (an earlier overflow would have made bounded return CapacityExceeded instead).

STALE-CAVEAT TIGHTENING: boundedness.rs e2e_example_02_split_never_overflows_capacity now asserts Ok(()) (was tolerating InvalidFiringOrder; TASK-0028-era caveat resolved by TASK-0136/0139). Gate depends on it.

PRD EDITS (honest, no new overclaim - pattern #1): section 2 table line, section 8.1 diagram (also removed implicit liveness-is-gated overclaim - liveness is MODELLED not gated), section 8.2 deadlock/buffer bullets, section 8.4 (CORRECTED doc-lie: deadlock names stalled-transition+deficit-place NOT the cycle), section 8.6 firing-order bullet. driver main.rs header pipeline summary updated. Method nuance preserved everywhere: exact-replay over ONE deterministic firing order, sound for v2 statically-ordered nets, NOT a reachability/coverability engine.

GATE NUMBERS (all green): build+clippy clean (fixed one doc_lazy_continuation - recurring pattern, + at col5 of //!); dev 1147/0/3 (+4 baseline 1143); release 1146/0/3 (+4 baseline 1142); e2e 322/265/0/57/0 UNCHANGED (gate rejects nothing shipping, matches 288-net REPORT sweep). check-narrative-doc-lie + check-include-str-coverage + check-textual-replace-on-codegen all OK. cargo doc: only pre-existing CumulativeWholeArrayFallback warning (TASK-0366, not mine); my intra-doc-links resolve. Inline smoke: 16-jacobi/distributed x mp-tcp-event builds (nucleus: ok), gate reachable+accepting on hardest shape.

HONEST LIMITATIONS: (1) gate is a provably-dead-today tripwire on shipping schedules - structural inject-pass guards mean no valid schedule produces an unsound net (cannot e2e-test the reject path through the driver; pinned at function level only). (2) exact-replay not full reachability. (3) deadlock diagnostic names stall point not full cycle.

CYCLE-216 IMPLEMENTATION COMPLETE + ARCHITECT GO; AC#2 GATE-VERIFICATION PENDING (batched). AC#1 (decision): Option A chosen by user — wire check_bounded+check_deadlock_free as a per-build compile gate. Commits 8903076 (net_soundness.rs check_net_sound + PetriAnalysisError + driver gate after emit-pn before out_dir dispatch, on FINAL acfg; 4 tests; 02-split test tightened to assert Ok) + 8d0419d (PRD section 2/8.1/8.2/8.4/8.6 reconciled) + f41c7b1 (architect P3 docstring softening). ORCHESTRATOR DE-RISK: ran a report-mode sweep over all 17 examples x schedules x 7 tier-1 backends (288 nets) — every one bounded+deadlock-free, so the hard gate rejects nothing shipping (02-split InvalidFiringOrder caveat is STALE). mped-architect read-only review = GO: independently verified every changed PRD claim TRUE vs code (incl. the real section-8.4 cycle->stall doc-lie correction + the section-8.1 liveness-overclaim removal), fall-through error-labelling provably sound (no false-accept; DeadlockError::CapacityExceeded dead in the composition), single shippable entry point gated (e2e harness shells out to the driver — no silent sibling), no panic on the gate path, 02-split tightening rationale grounded. Implementer-reported gate (NOT yet orchestrator-re-run, per batched-QA cadence): dev 1147/0/3, release 1146/0/3, e2e 322/265/0/57/0. AC#2 stays unticked + task In Progress until the batched gate re-runs these numbers (memory feedback-batch-qa-gate-not-per-task).

AC#2 GATE-VERIFIED + DONE (cycle-217 batched gate, orchestrator-run x2). The batched verification (held per the new batch-QA cadence) ran build+clippy clean, dev test 1149/0/3, release 1148/0/3 (the net_soundness 4 tests + check_net_sound gate all green), and e2e reproduced 2x. The Petri soundness gate (check_bounded+check_deadlock_free per build, commits 8903076/8d0419d/f41c7b1) rejects NOTHING shipping — the e2e total moved 322/265/0/57/0 -> 329/272/0/57/0 only because the co-landed 17-spmv/gather cells (TASK-0341.03.01) added +7 PASS; every one of the 329 cells passes the soundness gate. Architect review was GO (independently verified PRD honesty, fall-through soundness, no silent-sibling, no panic) at landing. AC#1 (decision Option A) + AC#2 (executed: wire-in + negative tests + PRD reconcile) both met.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-215. AC#1: Option A (wire-in) recorded as user-decided. AC#2: executed - check_net_sound gate wired into driver on every build (commit 8903076), negative tests prove rejection of unbounded + deadlocking nets at the function level, PRD section 8/section 2 + docstrings reconciled honestly with the now-true behaviour (commit 8d0419d). Gate green: e2e 322/265/0/57/0 unchanged, dev 1147/0/3, release 1146/0/3, clippy clean. Gate is exact-replay over one deterministic firing order (sound for v2 statically-ordered nets, not a general reachability engine) and a provably-dead-today tripwire on shipping schedules - pinned at function level. Independent orchestrator review gate (qa-test-runner + mped-architect) to run after.

DONE cycle-217. check_bounded + check_deadlock_free now run as a per-build compile gate (check_net_sound) on the final ACFG of every build, making PRD section 8 literally true of the shipping compiler. Provably-dead tripwire on shipping schedules (structural guards mean no valid schedule produces an unsound net; gate rejects nothing across all 329 e2e cells); negative tests pin the reject path at the function level. PRD section 2/8 reconciled honestly (incl. correcting a real section-8.4 doc-lie). Gate exact-replay over one deterministic firing order, sound for v2 restricted nets.
<!-- SECTION:FINAL_SUMMARY:END -->
