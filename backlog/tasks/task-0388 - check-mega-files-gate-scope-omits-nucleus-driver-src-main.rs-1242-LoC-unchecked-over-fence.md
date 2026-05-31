---
id: TASK-0388
title: >-
  check-mega-files gate scope omits nucleus/driver/src (main.rs 1242 LoC
  unchecked + over fence)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-31 11:06'
updated_date: '2026-05-31 12:40'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3.2 on TASK-0049.05. nucleus/driver/src/main.rs is 1242 LoC (grew from 1225 via the 0049.05 --shim print arm) but the `check-mega-files` recipe `find` scope does NOT include nucleus/driver/src, even though the recipe header comment lists nucleus/driver as a covered sub-tree (comment/scope mismatch = latent gate-rot, feedback-cheap-subset-blind-to-structural-fences). Fix: extend the find scope to cover nucleus/driver/src AND split main.rs below the 1000-LoC fence (the arg-parse, the lowering-pipeline orchestration, and the per-backend dispatch are natural seams). Pre-existing; not introduced by 0049.05 but nudged further over.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 check-mega-files find scope includes nucleus/driver/src; the gate now catches an over-fence driver file (tripwire-verified: a 1100-LoC driver/src file trips direction-A FAIL).
- [x] #2 main.rs split below the 1000-LoC fence (1242 -> 816); arg-parse -> args.rs (111), per-backend dispatch -> dispatch.rs (376); dispatch arms + arg-parse moved VERBATIM (byte-identical diff).
- [x] #3 Recipe header comment + canonical-reproducer find-set updated to include driver (no fresh comment/scope mismatch); driver dropped from the deliberately-excluded list.
- [x] #4 Behaviour-preserving: e2e 350/293/0/57/0 unchanged (byte-identical 7-backend); driver integration tests pass incl. task0048_05_shim_rejection (moved --shim check); build/clippy/test/test-release exit 0.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE OUTCOME (orchestrator-verified, GOx2 with e2e + check-mega-files tripwire independently re-run). Implemented in-thread. Commit b71e429.

FACTS: main.rs was 1242 LoC; check-mega-files find scope (justfile body) excluded nucleus/driver/src, so just ci was GREEN despite the over-fence file. The recipe HEADER (justfile:722-725) had deliberately excluded driver "until any grows past 1000" — that widen-trigger had FIRED unhandled. The two halves are COUPLED: extending scope without splitting => just ci RED.

SPLIT (seams from the module docstring): args.rs (BuildArgs/parse_build_args/print_help, pub(crate), 111 LoC) + dispatch.rs (dispatch_backend = the --shim validity check + 10-backend match, pub(crate), 376 LoC). cmd_build keeps the lowering-pipeline orchestration and ends with dispatch::dispatch_backend(...). The ONLY non-verbatim changes (behaviour-equivalent): shim param Option<String> -> Option<&str> (caller passes shim.as_deref()); match shim.as_deref() -> match shim; backend.as_str() -> match backend (param &str). Architect proved the moved blocks byte-identical via diff.

GATE-FIX: added nucleus/driver/src to the find scope (justfile body) + updated header scope-comment + canonical-reproducer brace-set; dropped driver from the deliberately-excluded list. Architect TRIPWIRE-tested liveness: a 1100-LoC driver/src file trips direction-A FAIL => scope genuinely enforced, not cosmetic.

VERIFICATION (re-run independently by QA + architect): build/clippy(0/0)/test(1192/0/3)/test-release(1191/0/3); driver tests 10/0 incl. task0048_05_shim_rejection both arms (moved --shim check) + emit_pn 4/4; check-mega-files GREEN with driver/src covered (main.rs 816, dispatch.rs 376, args.rs 111, all <1000); e2e 350/293/0/57/0 (independently re-run, 0 real FAIL = byte-identical 7-backend).

SILENT-SIBLING audit (architect): the 3 still-excluded sub-trees are all <1000 (mp-tcp-common 576, test-common 463, nucleus/nucleus has no src/) — no new over-fence file hiding. No moved-symbol call site missed (both print_help sites updated).

P3 advisory (NO action, pre-existing): dispatch.rs inherits the cycle-stamped per-arm comments verbatim from main.rs (a standing comment-doc-lie surface now concentrated in one file); not introduced by this commit.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE (GOx2). Closed the check-mega-files scope hole (driver/src now scanned, tripwire-verified) AND split the 1242-LoC main.rs below the fence (-> 816) into args.rs (111) + dispatch.rs (376) along the docstring-named seams. The dispatch match + arg-parse moved byte-identically (architect-diffed); only behaviour-equivalent re-typings (shim Option<&str>, match on &str). just ci is now honestly GREEN (was green only because the gate did not scan driver/src). No regression: e2e 350/293/0/57/0 (independently re-run, byte-identical 7-backend), driver tests pass incl. the moved --shim rejection. Commit b71e429.
<!-- SECTION:FINAL_SUMMARY:END -->
