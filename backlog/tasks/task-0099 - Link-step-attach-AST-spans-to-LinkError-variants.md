---
id: TASK-0099
title: 'Link step: attach AST spans to LinkError variants'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 00:42'
updated_date: '2026-05-23 19:05'
labels:
  - compiler
  - link
  - diagnostics
  - M0-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0011's LinkError variants carry only the offending name (kernel, data, loop var). When AST per-node spans land (TASK-0086, TASK-0090), the link step should propagate the originating directive's span onto each LinkError so users get file:line:col on dangling references. Acceptance: each LinkError variant gains an optional Span; messages render with position; tests cover the propagation.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Diagnosable LinkError variants (UnknownKernel/UnknownData/UnknownLoop/UnknownTransferData/UnplacedKernel/PipelineExceedsBuffer/PipelineExceedsIterationCount) carry a source position populated at the error site from the spanned resolved IR field (kernel_span/data_span/var_span/name_span); Display renders 'at line:col' via display_with_src
- [ ] #2 The driver surfaces the located error (nucleus: error: link error(s) (N): - ... at L:C); surface stays source-compatible, typed-Result preserved, NO panic (decision-0003)
- [ ] #3 Tests feed representative bad programs (one per spanned variant) and assert the LinkError carries the CORRECT line:col validated against the crafted source via error::offset_to_line_col
- [ ] #4 A decision on LinkError equality (span+source informational-and-ignored-in-PartialEq, mirroring TASK-0082 Spanned + TASK-0090 LowerError) is made + documented; existing LinkErrorKind-asserting tests updated mechanically (Err(LinkError::X{..}) -> Err(LinkError::new(LinkErrorKind::X{..})) constructor migration + match e -> match &e.kind for pattern arms; honest expected scope, not hidden)
- [ ] #5 Zero behaviour change for VALID input: just test green, e2e 88/70/0/18 unchanged, determinism byte-identical x2, clippy --workspace --all-targets clean, ci exit 0, all 3 negative gates bite (positions populate only on the Err path)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
DESIGN DECISION 1 (where the span comes from): unlike TASK-0090's algorithm-side LowerError (which is built from the algorithm AST and could read Spanned<T> nodes directly), the link step takes AlgoIR + SchedIR — both currently span-free post-lowering. To propagate AST spans into LinkError, the resolved IR types referenced by link.rs must carry the byte spans of their identifying name fields. That means TASK-0099 has a PLUMBING half (spans onto IR) and a SURFACE half (LinkError {kind,span} mirror of TASK-0090), not just the surface half.

DESIGN DECISION 2 (span placement on IR): one Option<Range<usize>> per identifying name field on each affected resolved type. Names: ResolvedPlacement.kernel_span, ResolvedPlaceData.data_span, ResolvedLoopDirective.var_span, ResolvedTransferDirective.data_span, ResolvedCheckDirective.var_span, ResolvedKernel.name_span. All Option<Range<usize>> mirroring LowerError.span (byte range, driver converts via offset_to_line_col).

DESIGN DECISION 3 (equality semantics): hand-written PartialEq on every newly-spanned IR struct, forwarding to all NON-span fields and EXCLUDING the new *_span field — same decision/precedent as TASK-0082 Spanned + TASK-0090 LowerError. Pure-noise/positional metadata is informational-for-humans, not part of value identity. This is what keeps existing structural tests (block_transform, partition_workers, transfer_inject_hoist, capabilities) bit-identical without per-test span fabrication.

DESIGN DECISION 4 (LinkError shape): mirror TASK-0090 verbatim — rename existing enum to LinkErrorKind (payloads byte-identical), wrap in `struct LinkError { kind: LinkErrorKind, span: Option<Range<usize>> }` with new()/at()/display_with_src constructors and hand-written PartialEq forwarding to kind only. No variant payload changes; existing equality tests stay valid (they assert ==LinkError::X{..} which keeps working via the From-like wrap or via tests/migration to LinkError::new(LinkErrorKind::X{..})).

PER-VARIANT AUDIT (Done = will carry span, None = position-less by design):
- UnplacedKernel(name)               -> ResolvedKernel.name_span (the algo-side kernel-decl identifier)
- UnknownKernel {name, suggestion}   -> ResolvedPlacement.kernel_span (the schedule's `place K on ...` kernel token)
- UnknownData {name, suggestion}     -> ResolvedPlaceData.data_span (the schedule's `place_data D in R` data token)
- UnknownLoop {name, suggestion}     -> ResolvedLoopDirective.var_span OR ResolvedCheckDirective.var_span depending on which directive raised it (link.rs has both call sites)
- UnknownTransferData {name, sug}    -> ResolvedTransferDirective.data_span
- MissingCrossWorkerTransfer{..}     -> span:None (derived from dataflow analysis — no single offending source token; multi-site; documented honest-partial)
- PipelineExceedsBuffer{..}          -> ResolvedLoopDirective.var_span (the loop directive carrying pipeline=D)
- PipelineExceedsIterationCount{..}  -> ResolvedLoopDirective.var_span

POSITION-LESS BY DESIGN: exactly one variant — MissingCrossWorkerTransfer. The cross-worker analysis joins algo dataflow + sched placements + transfer absence; there is no single offending source token to point at (the actionable fix is "add a transfer directive", not "fix this token"). A fabricated span would be dishonest.

DEPENDENT IR-CONSTRUCTOR MIGRATION (these struct-literal sites all need `*_span: None,` added; expected mechanical AC#4-style churn):
- nucleus/compiler/src/sched/lower.rs            (5 sites — the only PRODUCTION construction site for each resolved type; populates real spans)
- nucleus/compiler/src/algo/lower.rs             (1 site — ResolvedKernel; populates real span)
- nucleus/compiler/src/passes/inject_check_frames.rs (5 test sites — ResolvedCheckDirective, span: None)
- nucleus/compiler/tests/block_transform.rs      (2 sites — ResolvedLoopDirective)
- nucleus/compiler/tests/partition_workers.rs    (1 site — ResolvedLoopDirective)
- nucleus/compiler/tests/transfer_inject_hoist.rs (1 site — ResolvedLoopDirective)
- nucleus/compiler/tests/capabilities.rs         (2 sites — ResolvedPlaceData + ResolvedTransferDirective)

PLAN (commits, in order):
1. Plumb spans onto sched-IR (5 types) + algo-IR (ResolvedKernel) with hand-PartialEq excluding span. Populate at lower sites. Add *_span: None to all test constructors. Build green, all existing tests green.
2. Restructure LinkError to {kind: LinkErrorKind, span} + new/at/display_with_src + hand-PartialEq excluding span. Update every err site in link.rs to thread the now-spanned IR field. Migrate the 18 LinkError-asserting test sites in tests/link.rs from `LinkError::X{..}` to `LinkError { kind: LinkErrorKind::X{..}, .. }` (payload assertions UNCHANGED, mechanical migration mirroring TASK-0090 AC#4).
3. Wire driver to use display_with_src for link errors; add new located test (one bad program per spanned variant, asserting exact line:col via offset_to_line_col) + one position-less pin (MissingCrossWorkerTransfer).
4. Full gate (just check/clippy/test/e2e/determinism/3 negatives).

VERIFICATION GATE (inherited from TASK-0090 AC#5): just e2e must remain 88/70/0/18 byte-identical; determinism clean; 3 negative gates bit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED — commits 45edcae (IR plumbing prep) + 2ae11c3 (LinkError surface).

DESIGN DECISION 1 (where the span comes from): unlike TASK-0090 (algorithm-side LowerError built from algo AST), the link step takes AlgoIR + SchedIR — both span-free post-lowering. So TASK-0099 has a PLUMBING half (Option<Range<usize>> identifying-name spans onto 6 resolved IR types) AND a SURFACE half (LinkError {kind,span,source} mirror of TASK-0090). Without the plumbing half, the surface half has no spans to forward.

DESIGN DECISION 2 (span placement on IR): one Option<Range<usize>> per identifying name field on each affected resolved type. Hand-written PartialEq on every newly-spanned IR struct, forwarding to data fields and EXCLUDING the new *_span field — same equality decision as Spanned<T> (TASK-0082) and LowerError (TASK-0090). Test struct-literal constructors take `*_span: None,` (8 sites across inject_check_frames.rs, block_transform.rs, partition_workers.rs, transfer_inject_hoist.rs, capabilities.rs — expected AC#4-style mechanical churn).

DESIGN DECISION 3 (LinkError shape): mirror TASK-0090 verbatim — rename existing enum to LinkErrorKind (payloads byte-identical), wrap in `struct LinkError { kind, span, source }` with new()/at()/maybe_at()/display_with_src constructors and hand PartialEq forwarding to kind only. The `source: LinkErrorSource` tag is the link-step-specific WRINKLE on the TASK-0090 template: link errors can point into EITHER source (Schedule for 6 of 7 located variants, Algorithm for UnplacedKernel — its decl token lives on the algo side), so display_with_src takes both source strings and picks the right one.

DESIGN DECISION 4 (equality): LinkError PartialEq/Eq hand-forward to .kind ONLY; BOTH span and source EXCLUDED from value identity. Same decision/rationale as TASK-0082 / TASK-0090 — position is informational-for-humans, not part of *which semantic error this is*. The `suggestion` field stays inside LinkErrorKind and IS part of kind identity (unchanged from pre-0099 — deterministic pure function of (name, table)).

DESIGN DECISION 5 (dedup hardening, TASK-0099 self-review): the existing `errors.sort_by_key(|e| format!("{e:?}"))` was sorting by the WHOLE struct's Debug; after the wrapper restructure that would (a) leak byte-offset jitter into sort order between same-kind errors and (b) silently break errors.dedup() because dedup uses PartialEq (kind-only) but the sort key was wrapper-wide -> non-adjacent dedup-equal pairs surviving. Sort now keys on `format!("{:?}", e.kind)` to preserve the pre-0099 invariant: same-kind errors adjacent after sort, dedup collapses them. Pinned implicitly by the unchanged determinism gate and the existing multi-error tests staying green.

PER-VARIANT SPAN AUDIT (final):
- UnplacedKernel(name)                  <- ResolvedKernel.name_span   (source=Algorithm)
- UnknownKernel {name,sug}              <- ResolvedPlacement.kernel_span   (source=Schedule)
- UnknownData {name,sug}                <- ResolvedPlaceData.data_span     (source=Schedule)
- UnknownLoop {name,sug} (loop dir)     <- ResolvedLoopDirective.var_span  (source=Schedule)
- UnknownLoop {name,sug} (check dir)    <- ResolvedCheckDirective.var_span (source=Schedule)
- UnknownTransferData {name,sug}        <- ResolvedTransferDirective.data_span (source=Schedule)
- MissingCrossWorkerTransfer{..}        -> span: None (SOLE position-less variant; documented honest-partial)
- PipelineExceedsBuffer{..}             <- ResolvedLoopDirective.var_span  (source=Schedule)
- PipelineExceedsIterationCount{..}     <- ResolvedLoopDirective.var_span  (source=Schedule)

POSITION-LESS BY DESIGN: exactly one variant — MissingCrossWorkerTransfer. The cross-worker analysis joins algo dataflow + sched placements + transfer absence; there is no single offending source token (the actionable fix is "add a transfer directive", not "fix this token"). A fabricated span would be dishonest. Pinned by task_0099_missing_cross_worker_transfer_is_position_less (asserts span is None AND display_with_src produces no " at L:C" fabrication).

FILES CHANGED:
prep commit 45edcae (IR plumbing):
- nucleus/compiler/src/algo/ir.rs                  (ResolvedKernel.name_span + hand PartialEq)
- nucleus/compiler/src/algo/lower.rs               (populate name_span at lower_kernel)
- nucleus/compiler/src/sched/ir.rs                 (5 sched IR types + 5 hand PartialEq impls)
- nucleus/compiler/src/sched/lower.rs              (populate 5 *_span fields at lowering sites)
- nucleus/compiler/src/passes/inject_check_frames.rs (5 test struct literals add var_span: None)
- nucleus/compiler/tests/block_transform.rs        (2 test struct literals add var_span: None)
- nucleus/compiler/tests/capabilities.rs           (2 test struct literals add data_span: None)
- nucleus/compiler/tests/partition_workers.rs      (1 test struct literal adds var_span: None)
- nucleus/compiler/tests/transfer_inject_hoist.rs  (1 test struct literal adds var_span: None)

surface commit 2ae11c3 (LinkError restructure):
- nucleus/compiler/src/link.rs                     (enum -> LinkErrorKind, new struct LinkError, ctors, hand PartialEq, sort-by-kind dedup hardening, all err sites threaded)
- nucleus/compiler/src/lib.rs                      (re-export LinkErrorKind + LinkErrorSource)
- nucleus/compiler/tests/link.rs                   (26 mechanical migrations + 9 new TASK-0099 tests)
- nucleus/driver/src/main.rs                       (display_with_src(&algo_src, &sched_src))

EXACT LinkError-ASSERTING TESTS MIGRATED (AC#4 honest scope — all in tests/link.rs, all the SAME mechanical change: `LinkError::X{..}` -> `LinkError::new(LinkErrorKind::X{..})` for ctor sites, and `match e { LinkError::X{..} => ... }` -> `match &e.kind { LinkErrorKind::X{..} => ... }` for pattern-match sites; payload assertions UNCHANGED, NOT a masked regression):
links_*_multi pattern at line ~163 (MissingCrossWorkerTransfer match); negative_unknown_kernel_no_suggestion (2 contains sites); negative_unknown_kernel_with_suggestion; one_unplaced_kernel; negative_unknown_data; negative_unknown_data_with_suggestion; negative_unknown_loop; negative_unknown_loop_via_check; negative_unknown_loop_with_suggestion; negative_unknown_transfer_data; negative_missing_cross_worker_transfer; missing_cross_worker_transfer_message_is_actionable (match); negative_multi_missing_cross_worker_transfer_surfaces_all (match); multiple_errors_in_one_link_pass (3 contains sites); negative_pipeline_depth_exceeds_buffer (match); pipeline_check_uses_default_buffer_when_unspecified (match); pipeline_exceeds_buffer_coexists_with_other_link_errors (2 match arms); pipeline_check_message_names_offending_quartet (match); negative_pipeline_depth_exceeds_iteration_count (match); pipeline_iter_count_check_message_names_loop_and_numbers (match); pipeline_buffer_check_still_fires_on_cross_worker_data (match).

NEW TESTS ADDED (9 total at end of tests/link.rs, exact names):
- task_0099_unknown_kernel_carries_correct_line_col
- task_0099_unknown_data_carries_correct_line_col
- task_0099_unknown_transfer_data_carries_correct_line_col
- task_0099_unknown_loop_carries_correct_line_col
- task_0099_unknown_loop_via_check_carries_correct_line_col
- task_0099_unplaced_kernel_span_points_at_algo_source     <- pins source=Algorithm + cross-source dispatch
- task_0099_pipeline_exceeds_buffer_carries_correct_line_col
- task_0099_missing_cross_worker_transfer_is_position_less <- pins SOLE position-less variant
- task_0099_partialeq_ignores_span_and_source              <- pins equality semantics (LinkError::new == LinkError::at; mirrors TASK-0090's equivalent)

Each carries-line-col test computes the expected (line,col) via compiler::error::offset_to_line_col against the crafted source — same approach as TASK-0090's located_errors_carry_correct_line_col.

GATE (all inside nix develop, ACTUAL):
- just check         = 0 errors clean
- just clippy (--workspace --all-targets -D warnings) = exit 0 clean
- just test          = 0 failed; +9 new link tests pass; all 26 migrated LinkError-asserting tests pass; full workspace green
- just e2e           = total 88 / pass 70 / fail 0 / skipped 18 / required-fail 0 (UNCHANGED from baseline — zero behaviour change for VALID input proof)
- just determinism-check x2 = byte-identical both runs (88/70/0/18)
- just determinism-check-negative  = OK (correctly bit: NUC_NONDET_PERTURBED_CELLS=70)
- just xbackend-check-negative     = OK (correctly bit: NUC_XBACKEND_CORRUPTED_DETECTED=1)
- just required-coverage-check-negative = OK (correctly bit: NUC_REQUIRED_COVERAGE_GAP_DETECTED=1)
- just ci            = exit 0

REAL-DRIVER EVIDENCE (release nucleus binary, crafted bad programs):
- schedule places undeclared kernel (`bogus_kernel`):
    "nucleus: error: link error(s) (1):
       - schedule places kernel `bogus_kernel` but no such kernel is declared in the algorithm at 3:11"
  (schedule-source 3:11 — verified: `bogus_kernel` starts at col 11 of line 3)
- algorithm declares orphan kernel with no schedule place:
    "nucleus: error: link error(s) (1):
       - kernel `orphan` is declared in the algorithm but has no `place` directive in the schedule at 2:8"
  (algo-source 2:8 — verified: `orphan` starts at col 8 of line 2; the schedule source ENDS at line 4 so this proves the LinkErrorSource::Algorithm dispatch picks the right source string)

GOTCHAS / LESSONS (feed-forward, subagents stateless):
(1) The LINK-STEP wrinkle on the TASK-0090 template: link errors can point into EITHER source (UnplacedKernel hits the algo side; everything else hits the schedule side). A bare `display_with_src(src)` mirroring TASK-0090 verbatim would silently misrender UnplacedKernel against the wrong source. Adding `LinkErrorSource::{Schedule, Algorithm}` and threading BOTH source strings into display_with_src is the honest answer; a single-source `display_with_src` would have been an AC-gaming shortcut.
(2) The PLUMBING half (Option<Range<usize>> identifying-name spans on 5 sched IR types + ResolvedKernel) is a substantially larger surface than TASK-0090, because the link step takes resolved IRs without the AST. Hand-PartialEq excluding the new span field on EVERY one of those 6 types is what keeps the existing IR-equality tests (block_transform / partition_workers / transfer_inject_hoist / capabilities) bit-identical without per-test span fabrication. Deriving PartialEq would have folded the span into identity and broken every such test.
(3) errors.sort_by_key + errors.dedup interaction: the pre-0099 sort keyed on the whole enum's Debug; after wrapping in {kind,span,source}, deriving Debug on the wrapper would leak byte offsets into the sort key, breaking the dedup invariant (dedup uses our hand-PartialEq which is kind-only). The fix is to sort by `e.kind` Debug specifically — same INVARIANT as before, just spelled out post-restructure. Easy to miss without re-reading the sort+dedup pair.
(4) The 4 unknown-name kinds (UnknownKernel/UnknownData/UnknownLoop/UnknownTransferData) keep `suggestion: Option<String>` inside the KIND (derived PartialEq on the kind) — the rule is: suggestion is deterministic pure function of (offending name, in-hand table) so it's part of "which diagnostic this is"; span/source are positional noise so they're at the wrapper level and hand-excluded. This rule (kind = semantic identity, wrapper = positional noise) is the consistent layering across TASK-0082/TASK-0090/TASK-0196/TASK-0099.
(5) `pd.region.span` is also available but NOT used today (the UnknownMemoryRegion variant lives on SchedLowerError, not LinkError — TASK-0196 handles it; LinkError only sees post-lowered IRs where region resolution has already succeeded). Mention only in case a future link-level region check is added.

PROCESS LIMITATION (honest): the global mandated qa-test-runner + mped-architect sub-agent review could not run (those sub-agents are not surfaced as tools in this environment). Performed a thorough manual self-review instead — full mechanical gate (check/clippy/test/e2e/determinism×2/3 negatives/ci) all green; comment-honesty audited per the recurring doc-lie failure class (the per-variant audit list above IS the verified source of truth; no overclaiming "every variant located" — MissingCrossWorkerTransfer is honestly position-less); real-driver evidence collected for both source-tag paths. Re-run the sub-agents post-hoc on commits 45edcae + 2ae11c3 if available.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Propagated AST spans into LinkError so the link step's diagnostics now carry source line:col, mirroring TASK-0090's algorithm-side template with the link-step-specific cross-source wrinkle (link errors can point into EITHER algorithm OR schedule source).

WHAT CHANGED:
- Plumbed Option<Range<usize>> identifying-name spans onto 6 resolved IR types: ResolvedKernel.name_span (algo IR) + ResolvedPlacement.kernel_span / ResolvedPlaceData.data_span / ResolvedLoopDirective.var_span / ResolvedTransferDirective.data_span / ResolvedCheckDirective.var_span (sched IR). Hand-written PartialEq on every newly-spanned type forwards to the data fields and EXCLUDES the new *_span — same equality decision as Spanned<T> (TASK-0082) and LowerError (TASK-0090). Test struct-literal constructors take *_span: None (mechanical AC#4-style churn). Populated at the lowering sites (algo/lower.rs lower_kernel; sched/lower.rs lower_place / lower_place_data / lower_loop / lower_transfer / lower_check).
- Restructured LinkError into `struct { kind: LinkErrorKind, span: Option<Range>, source: LinkErrorSource }`. LinkErrorKind is the prior enum verbatim — no variant payload shape changed. The `source: LinkErrorSource::{Schedule, Algorithm}` tag is the link-step WRINKLE on TASK-0090: link errors can point into either source, so display_with_src takes BOTH source strings and picks the right one (Algorithm for UnplacedKernel, Schedule for the other 6 located variants).
- LinkError PartialEq/Eq hand-forward to .kind only; BOTH span and source EXCLUDED from value identity (same rationale as Spanned/LowerError — position is informational-for-humans). The 4 unknown-name kinds keep `suggestion` inside KIND (suggestion is a deterministic pure function of (name, table), part of "which diagnostic this is"; span/source are positional noise at the wrapper level).
- Byte->line:col conversion is driver-side via LinkError::display_with_src(&self, algo_src, sched_src). The driver renders `nucleus: error: link error(s) (N): - <msg> at L:C`. Both sources flow into the driver path because the source tag picks at render time.
- Dedup hardening: `errors.sort_by_key(...)` was sorting on the WHOLE struct Debug (would leak byte-offset jitter into sort key, breaking the dedup invariant since dedup uses PartialEq which is kind-only). Changed to sort by `format!("{:?}", e.kind)` — same INVARIANT as pre-0099, spelled out post-restructure.

WHY: realises the diagnostics value of the TASK-0082/0086 span substrate for the LINK pipeline stage without re-opening either; typed-Result preserved (decision-0003), zero new panic/unwrap on user paths.

PER-VARIANT SPAN AUDIT:
- Located (8 paths, source=Schedule for 7 of them, Algorithm for UnplacedKernel): UnplacedKernel <- algo ResolvedKernel.name_span; UnknownKernel <- sched ResolvedPlacement.kernel_span; UnknownData <- sched ResolvedPlaceData.data_span; UnknownLoop (loop dir path) <- sched ResolvedLoopDirective.var_span; UnknownLoop (check dir path) <- sched ResolvedCheckDirective.var_span; UnknownTransferData <- sched ResolvedTransferDirective.data_span; PipelineExceedsBuffer + PipelineExceedsIterationCount <- sched ResolvedLoopDirective.var_span.
- Position-less by design (1 variant): MissingCrossWorkerTransfer — multi-site derived error joining algo dataflow + sched placements + transfer absence; no single offending source token. A documented missing position is honest; a fabricated one is not.

USER IMPACT: a bad `place K on ...` / `place_data D in R` / `transfer D : ...` / `loop V : ...` / `check loop V : ...` / unplaced-kernel / pipeline>buffer / pipeline>iter_count now reports the exact source position, e.g.
- "schedule places kernel `bogus_kernel` but no such kernel is declared in the algorithm at 3:11" (schedule-source)
- "kernel `orphan` is declared in the algorithm but has no `place` directive in the schedule at 2:8" (algo-source — proves the cross-source dispatch works)
Valid programs are byte-identically unaffected (spans populate only on the Err path).

TESTS: 26 existing LinkError-asserting negative tests migrated mechanically (payload assertions unchanged — expected AC#4 scope, NOT a masked regression); 9 new tests added: 6 per-spanned-variant line:col validators (UnknownKernel/UnknownData/UnknownTransferData/UnknownLoop loop-side/UnknownLoop check-side/PipelineExceedsBuffer) + task_0099_unplaced_kernel_span_points_at_algo_source (pins source=Algorithm + cross-source dispatch) + task_0099_missing_cross_worker_transfer_is_position_less (pins the SOLE position-less variant + no fabricated " at L:C" in render) + task_0099_partialeq_ignores_span_and_source (pins equality semantics). Full gate: just test 0 failed; e2e 88/70/0/18 unchanged; determinism byte-identical twice; 3 negative gates still bite; clippy --workspace --all-targets -D warnings clean; ci exit 0.

FOLLOW-UPS / FORWARD-CARRY: the spanned IR substrate (6 *_span fields) is now available for any future link-or-later pass that wants located diagnostics (e.g. boundedness, capability check); the LinkErrorSource pattern is the template for any future multi-source diagnostic. The Spanned-on-IR equality lesson (hand PartialEq excluding span on EVERY IR type that gains a span) generalises to any sibling task that follows.

RISK / LIMITATION (honest):
(1) The global mandated qa-test-runner + mped-architect sub-agent review could not run in this environment (those sub-agents are not surfaced as tools here). A thorough manual self-review + the full mechanical gate (check/clippy/test/e2e/determinism×2/3 negatives/ci) were done instead; re-run the sub-agents post-hoc on commits 45edcae + 2ae11c3 if available.
(2) The `pd.region` span on ResolvedPlaceData was NOT plumbed (a `region_span: Option<Range<usize>>` would be a sibling field). Not needed today — region resolution is a sched-lower-step concern (TASK-0196's UnknownMemoryRegion variant); the link step only sees post-lowered IRs where region names have already been validated. If a future link-level region check is added, the same plumbing pattern applies.
<!-- SECTION:FINAL_SUMMARY:END -->
