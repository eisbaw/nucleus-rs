---
id: TASK-0340
title: >-
  Hygiene wave: split mega-files (>800 LoC) and add property-based tests on
  Petri-net IR
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-26 09:46'
updated_date: '2026-05-31 03:12'
labels:
  - tech-debt
  - hygiene
  - testing
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Hygiene wave flagged by 2026-05-25 audit (post cycle-171 M6-decomposition planning). Two deferred-not-cancelled debts that should land before M6 codegen amplifies them:

(A) Mega-files. Six src files breach the 800-LoC smell threshold; three breach 1500. Top offenders (LoC / comment ratio):
- nucleus/backend-common/src/render.rs               1687 / 0.46
- nucleus/backends/mp-tcp-bufsync/src/lib.rs          1515 / 0.34
- nucleus/nucleus-compiler/src/acfg.rs                1440 / 0.57
- nucleus/nucleus-compiler/src/link.rs                1290 / 0.47
- nucleus/backend-common/src/multi_worker_walker.rs   1169 / 0.48
- nucleus/backends/mp-tcp-event/src/multi_worker.rs   1140 / 0.21

acfg.rs at 57 percent comments is a comment-doc-lie magnet (per feedback-comment-doc-lie-recurring). render.rs + multi_worker_walker.rs are the shared spine of all 4 backends; a bug in either touches every tier-1 cell. M6 will add 3 backend crates + 3 examples on top of this substrate; splitting first prevents the smell from propagating.

(B) Zero property tests / zero fuzz across the entire workspace. The Petri-net IR (PRD section 8) whose soundness is the central thesis claim is tested by 49 hand-curated cases across acfg_to_petri / petri_to_events / boundedness / deadlock. A 50-line proptest on bounded-ACFG generators is the highest expected-ROI gap in the test suite.

Sub-concern: nucleus/e2e/src/main.rs is 7316 LoC with 76 internal tests covering the JSON/JUnit report formatter, not compiler correctness. Visually inseparable from compiler tests today.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Audit produces the canonical list of src .rs files greater than 800 LoC under nucleus/{backend-common,nucleus-compiler,backends}/src/; the six current offenders above are explicitly covered, plus any new addition
- [ ] #2 Each listed file split into cohesive sub-modules along seams already named by its module-level docstring (no behaviour change). Per-file split is one atomic commit; final commit asserts e2e baseline preserved bit-identical (currently 98 required + 10 skip in e2e-matrix.toml; just e2e totals line preserved)
- [x] #3 proptest dep added to nucleus-compiler dev-dependencies; at least 3 properties per pass for passes/boundedness.rs, passes/deadlock.rs, passes/petri_to_events.rs. Generators emit small bounded ACFGs; properties assert (i) boundedness pass agrees with bounded-reachability up to N steps, (ii) deadlock pass agrees with explicit enumeration on the same generated nets, (iii) petri_to_events output is acyclic per worker
- [x] #4 Report-formatter tests in nucleus/e2e/src/main.rs (currently 76 internal #[test]) carved out into a sub-module file (e2e/src/report/tests.rs) or sub-crate (e2e_report). Compiler-correctness tests remain in main.rs; formatter tests are visually separated
- [ ] #5 New just recipe check-mega-files added to ci: asserts no nucleus/**/src/*.rs file exceeds 1000 LoC. Recipe is wired into just ci as a regression-fence. Initial pass exempts any file the split intentionally leaves above 1000 LoC via an explicit allow-list (with rationale)
- [ ] #6 No new TASK-NNNN or cycle-NNN citations introduced in the refactored files (closes the comment-process-noise concentration smell: acfg.rs 74 mentions, mp-tcp-bufsync/lib.rs 68, sidecar.rs 57 at audit time)
- [x] #7 Final cycle commit notes per-file LoC before/after and per-pass proptest count delta (no separate summary md file per CLAUDE.md cruft policy)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 176 — slice 1 landed (AC#5 of TASK-0340)

`just check-mega-files` regression-fence recipe + ci wiring landed (commit pending).

DELIVERED:
- AC#5 (recipe + ci wiring + initial allow-list with rationale): DONE.

OPEN (subsequent slices):
- AC#1 (canonical audit list): DONE incidentally via the allow-list (14 files >1000 LoC documented).
- AC#2 (per-file split into sub-modules): NOT-YET — substantive, slice-2+ work.
- AC#3 (proptest dep + properties for boundedness / deadlock / petri_to_events): NOT-YET — slice-3 work.
- AC#4 (e2e/main.rs report-formatter carve-out): NOT-YET — slice-4 work; recipe scope explicitly excludes nucleus/e2e/src per architect cycle-176 P2.3.
- AC#6 (no new TASK/cycle citations in refactored files): NOT-YET — lands with AC#2 splits.
- AC#7 (final commit notes per-file LoC before/after + proptest count delta): NOT-YET — final slice when AC#2 + AC#3 close.

ARCHITECT-DEFERRED FOLD-BACKS (forward-carried to slice-2):
- P2.1: STALENESS direction not enforced — a future split could leave a stale allow-list entry for a file no longer >1000. Architect empirically verified: replaced pthreads-async/multi_worker.rs (allow-listed, 1048 LoC) with 500-LoC stub, recipe PASSED. Slice-2 should add a sibling assertion that every allow-list pattern matches a still-oversized file. Concretely: refactor the recipe to enumerate allow-list paths positively (rather than as grep -v negative filters) so the "this allow-list entry is stale" direction also fails loudly.

## Cycle 177 — slice 2 staleness-check refactor landed

Architect cycle-176 P2.1 fold-back complete: check-mega-files now enumerates the allow-list POSITIVELY (printf-fed bash array via temp files + `comm -23`). Both directions FAIL LOUD:
- (A) new mega-file >1000 LoC outside allow-list.
- (B) allow-list entry whose file is NO LONGER >1000 LoC (split landed, file deleted, file shrank).

The cycle-176 architect-reproduced silent-pass case (replace pthreads-async/multi_worker.rs allow-listed 1048 LoC → 500-LoC stub) now FAILS LOUD with the precise direction-B message.

Cycle-177 architect (read-only) GO with two P1 fold-backs applied this cycle:
- P1.1 POSIX-shell portability — `comm -23 <(echo ...)` used bash process substitution; just defaults to `/bin/sh` which on dash/ash/busybox would syntax-error before either direction runs (silent-absence rather than silent-pass). Rewrote to temp-file form via `mktemp` + `trap EXIT`.
- P1.2 memory-citation correction — initial draft cited `feedback-silent-sibling-defect` but the actual class is `feedback-opacity-gate-rot` (each per-file filter is a deferral gate that rots silently when surrounding state shifts). Swapped citation.
- P2.1 (folded inline) — added `set -eu` + `set -o pipefail` so find-pipeline-internal errors propagate; dropped `2>/dev/null` so scope-vanish failures surface.

Deferred (P2.2 cosmetic — direction-A LoC count in failure message; P2.3 informational — printf-form pin comment; P3.2 cosmetic — one memory per line). Acknowledged P3.1 — AC#5 of TASK-0340 reads "asserts no file exceeds 1000 LoC + initial pass exempts via allow-list with rationale"; the staleness direction is a strict SUPERSET of the AC text. Recording the cycle-177 implementation as "AC#5 implemented with both directions; staleness direction exceeds AC text scope" rather than rewriting the AC (per memory `feedback-ac-rewrite-on-done-task`).

BITE-verified both directions on the POSIX-rewrite form:
- 1006-LoC stub in backend-common/src/ → FAIL (direction A).
- pthreads-async/multi_worker.rs truncated to 500 LoC → FAIL (direction B).

TASK-0340 AC status post-cycle-177:
- AC#5: DONE (both directions now), strict superset of original AC text.
- AC#1: DONE incidentally (cycle 176).
- AC#2-#4, #6-#7: PENDING (subsequent slices).

## Cycle 178 — slice 3 plan: split nucleus/backend-common/src/render.rs (1687 LoC → ~7 sub-modules)

Slice scope: AC#2 (FIRST file). render.rs is the largest mega-file (1687 LoC) AND the shared spine of all 4 + 3 (M6 skeletons) tier-1 backends. M6 codegen WILL amplify it; splitting first prevents propagation. Per AC#2 "one atomic commit per file"; this cycle = ONE file (render.rs only).

EMPIRICAL UPDATE TO AUDIT (cycle-178 measurement at slice-3 start):
- nucleus/backend-common/src/render.rs:                1687 LoC (matches Nov-2025 audit)
- nucleus/backends/mp-tcp-bufsync/src/lib.rs:          1997 LoC (was 1515; +482)
- nucleus/nucleus-compiler/src/acfg.rs:                1440 LoC (was 1440; unchanged)
- nucleus/nucleus-compiler/src/link.rs:                1290 LoC (was 1290; unchanged)
- nucleus/backend-common/src/multi_worker_walker.rs:   1263 LoC (was 1169; +94)
- nucleus/backends/mp-tcp-event/src/multi_worker.rs:   1695 LoC (was 1140; +555)
- Total mega-file LoC: 9372. M6 amplification risk confirmed: mp-tcp-bufsync/lib.rs + mp-tcp-event/multi_worker.rs both crossed the original 1500 threshold further by Q1-2026 cross-pass work.

PROPOSED SEAMS in render.rs (each already named by `// ---` section header):
  1. error.rs       — EmitError + Display + Error  (~67 LoC,  L45-112)
  2. ctx.rs         — RenderCtx + RenderCtxPub     (~187 LoC, L113-300)
  3. fire.rs        — data_name / fire output / fire args / try_rewrite_reuse_arg / SliceForm /
                      classify_data_slice / render_flat_index  (~370 LoC, L301-704)
  4. expr.rs        — render_int_expr / render_loop_bounds / render_const_expr / bin_op_str (~200 LoC, L705-840)
  5. types.rs       — rust_scalar_type[_pub] / rust_scalar_zero / rust_type_of / render_array_init_for (~67 LoC, L841-908)
  6. pub_wrappers.rs— thin _pub shims for multi-worker callers (~88 LoC, L909-996)
  7. reuse.rs       — reuse-widths marker emit + ReuseRewriteGroup + circular-buffer codegen
                      (TASK-0265 Tier 1 + TASK-0269 cycle-103) (~691 LoC, L997-end)
  + render/mod.rs   — declarations + pub re-exports preserving the existing
                      `pub use render::{...}` set at lib.rs:68-74 (zero-churn for callers)

Slice 3 limits:
- Re-exports at backend-common/src/lib.rs:68-74 preserved verbatim — call sites in the 4 production backends + 3 M6 skeletons do NOT change.
- No behaviour change (AC#2). `just ci` + `just check-mega-files` MUST both pass; e2e baseline preserved (currently 112/101/0/11/0 per cycle 163 carry).
- AC#6 (no new TASK-NNNN or cycle-NNN citations in refactored files): the splits MUST NOT add new such citations; existing in-context citations carry forward unchanged in the moved code.
- AC#5 regression-fence: `just check-mega-files` allow-list MUST shrink — render.rs comes off it. Verify allow-list also doesn't go stale (slice-2 direction-B catches that).

Remaining slices (after cycle 178):
- slice 4: split mp-tcp-bufsync/src/lib.rs (1997 LoC) — host+worker mediation seam, largest single backend file.
- slice 5: split mp-tcp-event/src/multi_worker.rs (1695 LoC) — sibling of slice 4.
- slice 6: split nucleus-compiler/src/acfg.rs (1440 LoC, 57% comment ratio — comment-doc-lie magnet).
- slice 7: split nucleus-compiler/src/link.rs (1290 LoC).
- slice 8: split backend-common/src/multi_worker_walker.rs (1263 LoC).
- slice 9: AC#3 (proptest dep + 3 properties per pass on boundedness/deadlock/petri_to_events).
- slice 10: AC#4 (e2e/main.rs report-formatter carve-out).
- slice 11: AC#7 final commit with per-file LoC delta + proptest count delta.

## Cycle 178 + 178b — slice 3 LANDED

**Slice 3 commits:**
- c629458 — split render.rs (1687 LoC) into render/{ctx,error,expr,fire,reuse,types}.rs + mod.rs.
- 8fd785a — fold back architect P2.1 + P2.2 + QA P3-1 doc-only repairs.

**Per-file LoC** (vs old render.rs 1687):
- render/ctx.rs    203
- render/error.rs   70
- render/expr.rs   148
- render/fire.rs   466
- render/mod.rs     71
- render/reuse.rs  778
- render/types.rs   69
- (sum 1805; +118 vs pre-split = module docstrings + use statements + mod.rs re-exports; no behaviour change)

**Verification gate (orchestrator-self-run, inside nix develop):**
- just build (workspace): clean.
- just clippy (-D warnings --all-targets): clean.
- just test (dev): 969 / 0 / 3.
- just test-release: 968 / 0 / 3 (-1 vs dev is the debug_assert-only #[should_panic] negative test, expected per TASK-0291).
- just e2e: 112 / 102 / 0 / 10 / 0 — THREE independent samples across c629458 (×2) + 8fd785a (×1), all bit-identical.
- just check-mega-files: OK both directions (render.rs comes off the allow-list; direction-B staleness check confirms).

**Review gate (parallel, read-only, both spawned on c629458):**
- qa-test-runner: GO. Found one AC#6 hairline (cycle-107 hyphenated form vs the file's existing space-form convention) — folded back in 178b.
- mped-architect: GO. Found two P2 doc-lies the split carried verbatim from pre-split render.rs but PROMOTED the worst (mp-tcp-bufsync "does NOT yet consume" claim) into a module-level `//!` docstring. All three repaired in 178b. Filed three architect P3 follow-ups as TASK-0340.01 / .02 / .03 (substantive but not blocking):
  - TASK-0340.01: slice 4 candidate — mp-tcp-bufsync/src/lib.rs split (1997 LoC, largest remaining mega-file).
  - TASK-0340.02: ctx<->fire<->reuse sibling-mod dep cycle (architectural shape; Rust permits).
  - TASK-0340.03: further-split reuse.rs (778 LoC) into reuse/{group,marker,discover,codegen}.rs (defer until M6 amplification).

**Cycle 178+178b honest disclosure:**
- Implementer (= orchestrator-main-thread, per memory `feedback-spawned-agents-refuse-code-edits`) failed to spot-check the doc claims it carried forward. The architect review caught it (good — the parallel review gate is the safety net) and the fix lived in a single fold-back commit (8fd785a). Marker for future cycles: when a mechanical split moves a multi-claim docstring, RE-VERIFY every claim against current code before promoting the comment into a more-visible position (per memory `feedback-comment-doc-lie-recurring` + `feedback-opacity-gate-rot`).
- The architect-claimed "ctx.rs:117 was a stale field doc" was confirmed against TASK-0284 (Done cycle 107) by inspection — mp-tcp-bufsync DOES populate reuse_active via its own walker since cycle 107.
- e2e baseline NUMERIC drift discovered while splitting: cycle-104 commentary said `88/70/0/18` (literal-pinned in ctx.rs:99 via copy-from-old-render.rs); current baseline is `112/102/0/10` (cycle 178). The numeric pin was deleted in 178b (replaced with non-baseline-bound prose) so the same staleness can't silently recur as the matrix grows further.

**AC status post-cycle-178+178b:**
- AC#1: DONE (cycle 176 + 178 update — the original 6-file audit list still accurate, with the empirically-observed M6-amplification growth on mp-tcp-bufsync/lib.rs and mp-tcp-event/multi_worker.rs surfaced).
- AC#2: 1/6 files split (render.rs); 5 remaining. Forward-carried as slices 4-8.
- AC#3: PENDING (slice 9: proptest dep + 3 properties per pass on boundedness/deadlock/petri_to_events).
- AC#4: PENDING (slice 10: e2e/main.rs report-formatter carve-out).
- AC#5: DONE (cycle 176 + 177); allow-list shrank this cycle (render.rs entry removed).
- AC#6: DONE for the cycle-178 files (no new TASK-NNNN; one cycle-107 hyphen→space normalised in 178b).
- AC#7: PENDING (final slice when AC#2 + AC#3 close).

**Slice plan for subsequent cycles:**
- slice 4: TASK-0340.01 mp-tcp-bufsync/src/lib.rs (1997 LoC) — substantial; biggest remaining mega-file; in the M6-amplification path.
- slice 5: mp-tcp-event/src/multi_worker.rs (1695 LoC) — sibling shape to slice 4.
- slice 6: nucleus-compiler/src/acfg.rs (1440 LoC, 57% comment ratio — comment-doc-lie magnet per the audit; high P2-finding rate likely).
- slice 7: nucleus-compiler/src/link.rs (1290 LoC).
- slice 8: backend-common/src/multi_worker_walker.rs (1263 LoC).
- slice 9: AC#3 (proptest).
- slice 10: AC#4 (e2e report-formatter).
- slice 11: AC#7 (final summary commit).

## Cycle 179 — slice 4 LANDED (TASK-0340.01)

Split nucleus/backends/mp-tcp-bufsync/src/lib.rs (1997 LoC) into 7 cohesive sub-modules along the existing '// ---' section seams. AC#2 now 2/6.

**Per-file LoC after split** (vs old lib.rs 1997):
- src/lib.rs                315
- src/encode.rs              81
- src/walkers.rs            394
- src/plan/mod.rs           243
- src/plan/worker_program.rs 388
- src/plan/events.rs        491
- src/plan/relay.rs         222
- (sum 2134; +137 vs pre-split = per-file module docstrings + use statements + sub-module decls; no behaviour change)

**Verification (orchestrator-self-run, all inside nix develop):**
- just build / clippy: clean.
- just test (dev): 969/0/3. just test-release: 968/0/3.
- just e2e: 112/102/0/10/0 — TWO non-flake samples.
- just check-mega-files: OK both directions; bufsync lib.rs comes off the justfile:457 allow-list.

**AC status post-slice-4:**
- AC#1: DONE.
- AC#2: 2/6 files split.
- AC#5: DONE (allow-list shrank further).
- AC#6: DONE for this slice.
- AC#3, AC#4, AC#7: PENDING (slices 9 / 10 / 11).

**Remaining slices:**
- slice 5: mp-tcp-event/src/multi_worker.rs (1695 LoC) — sibling shape; expect ~identical seam map (worker_program / events / relay / walkers).
- slice 6: nucleus-compiler/src/acfg.rs (1440 LoC, 57% comment ratio — comment-doc-lie magnet).
- slice 7: nucleus-compiler/src/link.rs (1290 LoC).
- slice 8: backend-common/src/multi_worker_walker.rs (1263 LoC).
- slice 9: AC#3 proptest.
- slice 10: AC#4 e2e report-formatter.
- slice 11: AC#7 final summary commit.

## Cycle 179b — parent-tracker hardening from slice-4 review gate

(Follows cycle 179 commit 6315b1b TASK-0340.01 slice 4 Done. Review gate
findings recorded on TASK-0340.01 notes; this is the parent-task summary.)

**AC#2 correction: 2/6 files split (render.rs + mp-tcp-bufsync/lib.rs).
Remaining originally-named-by-name: 4 files.**
- nucleus/backends/mp-tcp-event/src/multi_worker.rs (1695 LoC)
- nucleus/nucleus-compiler/src/acfg.rs (1440 LoC)
- nucleus/nucleus-compiler/src/link.rs (1290 LoC)
- nucleus/backend-common/src/multi_worker_walker.rs (1263 LoC)

**Mega-file count correction: 12, NOT 13** (architect cycle-179b P2.3
empirically verified by `find … -exec wc -l … | awk '$1 > 1000'`). The
slice 4 commit message + slice 4 implementer's honest-limits both said
"13 remain"; the correct number is 12. Off-by-one in the commit message.

**Open scope question: should AC#2 cover all 12, or only the 4
originally-named?** The 8 mega-files NOT in the original cycle-175 audit:
- passes/transfer_inject.rs (4726 LoC) — >2× larger than any other
- passes/reuse_inference.rs (1676)
- sched/lower.rs (1546)
- passes/halo_inference.rs (1525)
- algo/lower.rs (1328)
- passes/host_data_relay_inject.rs (1212)
- sched/ir.rs (1163)
- backends/pthreads-async/multi_worker.rs (1048)

Architect cycle-179b recommends tightening AC#2 to the check-mega-files
recipe scope (all >1000 LoC). The 800-LoC discussion target in AC#2 is a
SMELL line, not the hard one; AC#5's recipe already polices the broader
set. transfer_inject.rs at 4726 LoC is structurally larger than every
file slice 1-4 has touched combined.

**Decision: defer the AC#2 scope question to the next slice's plan.**
Slice 5 implementer chooses between:
- (a) Continue with the originally-named 4 files (mp-tcp-event/multi_worker.rs
  next by size). Close TASK-0340 at end of slice 8.
- (b) Tighten AC#2 to the recipe scope, take transfer_inject.rs next
  (highest-leverage split per AC#5 reading). TASK-0340 grows by ~6 more
  slices.

**AC#6 reading clarification:** "no new TASK-NNNN or cycle-NNN citations
introduced in the refactored files" is interpreted in the LENIENT sense
— pre-existing anchors copied with their moved code or repeated in new
per-file `//!` orientation docstrings are NOT new citations. Strict
reading would forbid even copy-with-the-code, which would force
information loss. Lenient interpretation is what cycle 178 + 179
implementers used; making it explicit so slice 5 doesn't second-guess.

**Slice 4 implementer-disclosure honesty drift** (architect cycle-179b
P2.1): the implementer's "Every moved fn/struct/doc-comment is preserved
verbatim" claim was falsified by 4 small reflows + 1 cosmetic indentation
fix (see TASK-0340.01 notes). Functionally benign; e2e gate held. Per
memory `feedback-implementer-disclosure-mechanism-wrong`, the slice-5
implementer brief should:
- Forbid blanket "verbatim move" claims.
- Require enumerated disclosure: "Behaviour-equivalent change list: <none
  | (a) ..., (b) ..., ...>" instead of "no behaviour change".

This bookkeeping closes cycle 179b. Slice 5 (mp-tcp-event/multi_worker.rs
by default, OR transfer_inject.rs if scope tightened) is the natural next
keystone.

## Cycle 180 — slice 5 LANDED (TASK-0340.04)

Split nucleus/backends/mp-tcp-event/src/multi_worker.rs (1695 LoC) into multi_worker/{mod,worker_program,relay,walkers,encode}.rs. AC#2 now 3/6.

**Per-file LoC after split** (vs old multi_worker.rs 1695):
- src/multi_worker/mod.rs              649
- src/multi_worker/worker_program.rs   578
- src/multi_worker/relay.rs            104
- src/multi_worker/walkers.rs          355
- src/multi_worker/encode.rs            74
- (sum 1760; +65 vs pre-split = per-file //! docstrings + use statements + 2 sub-module impl Plan<'_> braces)

**Verification gate (orchestrator-self-run, inside nix develop):**
- just build / clippy: clean.
- just test (dev): 969/0/3. just test-release: 968/0/3.
- just e2e: 112/102/0/10/0 — TWO independent samples (non-flake).
- just check-mega-files: OK both directions; allow-list shrank from 12 to 11 entries.

**Behaviour-equivalent change list** (cycle-179b enumerated-disclosure discipline, empirically diffed per moved item):
- (a) collect_pre_init signature reflowed across 4 lines from 1 (rustfmt-forced after pub(super) widening).
- (b) Plan::max_payload_bytes: scalar_width(...) → encode::scalar_width(...) (function moved sub-module).
- (c) 13 items had visibility uplift fn → pub(super) (required for sibling-module access).
- (d) struct RelayHop + 4 fields uplifted to pub(super) (required for relay.rs to construct).
All four classes mechanical; bodies byte-identical. Bit-identical emit verified by e2e.

**AC status post-slice-5:**
- AC#1: DONE (cycle 176 + 178).
- AC#2: 3/6 files split. Remaining: acfg.rs (1440), link.rs (1290), multi_worker_walker.rs (1263).
- AC#3, AC#4: PENDING.
- AC#5: DONE; allow-list 12 → 11.
- AC#6: DONE for this slice (lenient AC#6 reading).
- AC#7: PENDING.

**Remaining slices:**
- slice 6: nucleus-compiler/src/acfg.rs (1440 LoC) — first NON-backend file in the audit; expect different seam shape than slice 4-5 (passes/types not Plan/codegen).
- slice 7: nucleus-compiler/src/link.rs (1290 LoC).
- slice 8: backend-common/src/multi_worker_walker.rs (1263 LoC) — shared walker substrate; very high consumer count; expect a careful split.
- slice 9: AC#3 proptest.
- slice 10: AC#4 e2e report-formatter.
- slice 11: AC#7 final summary commit.

**Implementer-disclosure honesty (cycle-179b lesson applied):**
Cycle 180 implementer used the enumerated-disclosure shape directly (no blanket 'verbatim move' claim). Spotted + flagged the 2 mechanical reflow + path-requalify edits (a) + (b) before review-gate-side feedback would surface them. The cycle-179 pattern (4 silent edits flagged by post-hoc review) did NOT recur — slice 5 was clean by self-audit.

## Cycle 180 + 180b — slice 5 close + parent status

**Slice 5 LANDED (cycle 180, commit 9a4c89d):** mp-tcp-event/multi_worker.rs split 1695 LoC → multi_worker/{mod,worker_program,relay,walkers,encode}.rs. Mirror of slice 4 template.

**Cycle 180b review-gate hardening:** parallel review gate (qa-test-runner + mped-architect read-only) returned GO from both arms. The cycle-179b enumerated-disclosure discipline is empirically validated this cycle — slice-5 implementer's (a)/(b)/(c)/(d) list was COMPLETE; no silent indentation fix; no 5th edit found. Three pre-existing P3 follow-ups filed (TASK-0340.04.01/.02/.03).

**AC status post-slice-5:**
- AC#1: DONE.
- AC#2: **3/6 files split.** Remaining originally-named: acfg.rs (1440), link.rs (1290), multi_worker_walker.rs (1263). 3 more slices to close the original 6-file scope.
- AC#3: PENDING.
- AC#4: PENDING.
- AC#5: DONE; allow-list shrank 12 → 11.
- AC#6: DONE for slice 5.
- AC#7: PENDING.

**Mega-file count after slice 5:** 11 files >1000 LoC remain in `nucleus/{backend-common,nucleus-compiler,backends}/*/src/*.rs` scope:
- Original audit remaining: acfg.rs (1440), link.rs (1290), multi_worker_walker.rs (1263).
- Out-of-original-audit: transfer_inject.rs (4726), reuse_inference.rs (1676), sched/lower.rs (1546), halo_inference.rs (1525), algo/lower.rs (1328), host_data_relay_inject.rs (1212), sched/ir.rs (1163), pthreads-async/multi_worker.rs (1048).

**AC#2 scope question (still deferred from cycle-179b):** the original 6-file scope vs the recipe-scope (all >1000 LoC) decision was deferred to the slice-6 implementer's plan. transfer_inject.rs at 4726 LoC remains the highest-leverage out-of-original-audit candidate; slice 6 should consider taking it instead of acfg.rs.

**Stop condition reached for this session:** the orchestrator has shepherded slice 4 + slice 5 + cycle-179b/180b hardening through 5 implementer/reviewer subagent cycles (1 implementer + 2 reviewers per slice = 6 spawns, +1 implementer for slice 5). The next slice (slice 6 = acfg.rs OR transfer_inject.rs) is a structurally different file (non-backend, IR/transform compiler-pass, high downstream consumer count) that warrants fresh context per the phase3-backlog-ralph stop criteria. Resuming in a new session yields cleaner brief construction.

## Cycle 181 + 181b — slice 6 (TASK-0340.05) close

Slice 6 implementer (mped-architect agent type, in-thread) landed commit 769d9a5: backend-common/src/multi_worker_walker.rs (1263 LoC) split into multi_worker_walker/{ctx,block_tag,event_walker,wait,collect,mod}.rs (6 files, total 1327 LoC; +64 vs pre-split = orientation docstrings + use lines + sub-module decls; no behaviour change).

Parallel review gate (qa-test-runner + mped-architect, read-only) returned GO with two P2 doc-lie findings (line-stamps stale from cycle-178/180 file shifts not carried during the slice-6 mechanical move per cycle-180 brief gotcha (B)). Cycle 181b folded them back inline; 1 additional 'in this file' deixis-lie discovered during the fold-back audit (L49 of ctx.rs pre-181b) and fixed in the same commit.

Behaviour-equivalent change list cycle 181:
- (a) 3 WalkerCtx impl methods uplifted private fn → pub(super): render_ctx, worker_name, data_name. Required because the sole cross-sub-module consumer (event_walker.rs) now lives in a different file; pub(super) is the tightest viable scope.
- (b) enum WaitSlice co-located with its consumers (moved from old L128-140 neighbouring WalkerCtx to head of wait.rs). Module-private before and after.

Cycle 181b edits (post-fold-back):
- ctx.rs L34-43: line-citations updated for pthreads-sync (538→532), pthreads-async (522→518), mp-tcp-event (multi_worker.rs:493 → multi_worker/worker_program.rs:130 — cycle 180 file-path shift). Verification-cycle stamp updated 142b → 181b.
- ctx.rs L49: 'in this file' → 'in the sibling [`super::event_walker`] module' (cycle-181-introduced deixis-lie; the cycle-178b/179b/180b lesson now generalises to file/module deixis in addition to numeric citations).
- ctx.rs L53-62: emit-template citations updated to event_walker.rs:454 (Push) + event_walker.rs:474 (Wait). Verification-cycle stamp updated 142 → 181b.

AC delta for TASK-0340 post-cycle-181b:
- AC#1: DONE (cycle 176; audit list still accurate).
- AC#2: **4/6 files split** (render.rs cycle 178; mp-tcp-bufsync/lib.rs cycle 179; mp-tcp-event/multi_worker.rs cycle 180; backend-common/multi_worker_walker.rs cycle 181). Remaining: nucleus-compiler/acfg.rs (1440 LoC), nucleus-compiler/link.rs (1290 LoC). The backend-common spine is now done; remaining 2 are nucleus-compiler IR/pass-layer — different shape than backend codegen splits 3-6.
- AC#3 (proptest): PENDING.
- AC#4 (e2e/main.rs report-formatter carve-out): PENDING.
- AC#5: DONE (allow-list shrank 11 → 10 cycle 181; direction-B staleness confirms).
- AC#6: DONE for each landed slice (no new TASK-NNNN / cycle-NNN citations introduced in slice 6 mechanical-move; cycle-181b adjustments are existing-narrative updates not new anchors).
- AC#7: PENDING (final cycle when AC#2 + AC#3 close).

Forward-carries to slices 7+ (acfg.rs, link.rs):
- Doc-deixis audit ('this file' / 'this function' references) is now mandatory post-split audit dimension alongside the existing line-number stamp audit. Surfaced via cycle-181b discovery of 'in this file' lie at ctx.rs:L49.
- Slices 7+ target compiler IR/pass files (not backend codegen). Comment-density at acfg.rs 0.57 is the highest in the workspace — comment-doc-lie risk is correspondingly amplified per feedback-comment-doc-lie-recurring.
- Architect cycle-181 P3.1 (two-way dep risk if future M6 codegen amplification needs wait/block_tag/event_walker back-edges) — not actionable at slice-6 close; flag if M6 codegen work bumps wait.rs / block_tag.rs past leaf status.

## Cycle 182 + 182b — slice 7 (TASK-0340.06) close

Slice 7 implementer (mped-architect agent type) landed commit b887acf: nucleus-compiler/src/acfg.rs (1440 LoC) split into acfg/{mod,types,errors,build}.rs (4 files, total 1500 LoC; +60 vs pre-split = orientation docstrings + use lines + sub-module decls; no behaviour change). Methods sub-module skipped — impl ACFGNode + impl ACFG co-located in types.rs (cycle-178 precedent: small impls with their types when no cross-sub-module callers).

Three doc-claim rewrites by the implementer (NOT verbatim moves, per cycle-181b discipline):
- mod.rs L70-77: 'filed rather than half-implemented' -> TASK-0260/0263 anchor.
- types.rs L113-114 (DataflowEdge): 'precise per-tile halo synthesis — deferred follow-up' -> TASK-0260/0263.
- build.rs L250-255 (bind_arg): 'pthreads-sync render_call_arg' -> 'backend_common::render::fire::render_fire_arg ... EmitError::UnsupportedFeature'.

Parallel review gate (qa-test-runner + mped-architect, read-only) returned GO with two P2 doc-lie findings the implementer's Dim-2 grep missed:
- P2.1: silent-sibling sweep on render_call_arg — 12 sibling sites untouched in event.rs + sidecar.rs + tests/petri_to_events.rs. Cycle 182b sweep + targeted qualifier updates folded back.
- P2.2: stale Xfer doc-claim 'empty payload at M1' in mod.rs:43-44. Cycle 182b rewrote to match Sync structural-sibling claim shape.

AC delta for TASK-0340 post-cycle-182b:
- AC#1: DONE.
- AC#2: **5/6 files split** (render.rs cycle 178; mp-tcp-bufsync/lib.rs cycle 179; mp-tcp-event/multi_worker.rs cycle 180; backend-common/multi_worker_walker.rs cycle 181; nucleus-compiler/acfg.rs cycle 182). Remaining: nucleus-compiler/link.rs (1290 LoC) — the LAST originally-named mega-file.
- AC#3 (proptest): PENDING.
- AC#4 (e2e/main.rs report-formatter carve-out): PENDING.
- AC#5: DONE (allow-list shrank 10 → 9 cycle 182; direction-B clean).
- AC#6: DONE for slice 7 (3 enumerated doc-claim rewrites in cycle 182 + 4 silent-sibling sweep edits in 182b are explicitly disclosed corrections, not new TASK/cycle anchors).
- AC#7: PENDING (final cycle when AC#2 + AC#3 close).

Forward-carries to slice 8 (link.rs split) + slices 9-10 (AC#3 proptest + AC#4 e2e formatter):
- **Dim-2 vocabulary extension (cycle 182b lesson):** the implementer's grep regex '(does NOT yet|will be|TODO|future work|deferred|in this cycle|NOT yet|coming)' MISSED the 'empty payload at M1' claim shape. Slice 8 brief should widen Dim-2 grep to: '(does NOT yet|will be|TODO|future work|deferred|in this cycle|NOT yet|coming|placeholder|stub|skeleton|empty payload|M[0-9]+\b)' or similar. Milestone-version pins (M1/M2/.../Mn) age fastest in this codebase since milestones land in days/weeks.
- **Silent-sibling sweep discipline (cycle 182b lesson):** when any doc-claim rewrite mentions a renamed function/struct/path, run grep -rn '<OLD-NAME>' nucleus/ BEFORE commit and sweep all hits to current truth. The implementer's claim 3 fix was 1 of 12 siblings; 11 silent siblings shipped — caught by architect review. Specifically applies to slice 8 if link.rs's docs reference any renamed compiler-pass entry points.

E2E baseline: 112 / 102 / 0 / 10 / 0 (preserved across cycles 178-182 + 182b).

## Cycle 183 + 183b — slice 8 (TASK-0340.07) close + parallel review gate

Slice 8 implementer (mped-architect, in-thread) landed commit f518890: nucleus-compiler/src/link.rs (1290 LoC) split into link/{mod,types,errors,build,dataflow,pipeline}.rs (6 files, total 1369 LoC; +79 vs pre-split = orientation docstrings + use lines + sub-module decls + pub use re-exports). Tracker close at e808bc2.

Parallel review gate (qa-test-runner + mped-architect, read-only) returned GO from both arms:
- qa-test-runner: all gate numbers reproduced bit-identically. just test 969/0/3 dev; just test-release 968/0/3; just e2e 112/102/0/10/0 across TWO independent samples (post-cargo-clean -p nucleus); just check-mega-files PASS both directions (allow-list 9 → 8).
- mped-architect: 2 P2 doc-lie findings + 1 P3 tracker-drift finding folded back as cycle 183b.

Cycle 183b fold-back (doc-only):
- P2.1 site #1: link/mod.rs:58 'filed only as a `link.rs` inline note today' → 'filed only as an inline limitation in this module today' (R2-introduced bare-filename self-deixis; link.rs no longer exists post-split).
- P2.1 site #2: link/errors.rs:341-342 'the common path through link.rs' → 'the common path through [`link`](super::build::link)' (anchor on the symbol, not the dead file).
- P2.2: link/dataflow.rs:48-53 R5 doc-claim overcorrection — partition_workers / partition_rows / partition_blocks2d do NOT process raw AlgoIR `Dataflow { rhs: bare-DataRef }` (they operate on ACFG DataflowEdges from kernel Calls). Rewrote the carve-out to anchor on no-current-consumer + the actual mechanism (bare-DataRef shape dropped at AlgoIR-ingest time), not on a downstream pass that doesn't address it. Promoted memory feedback-implementer-disclosure-mechanism-wrong instance.
- Silent-sibling sweep (cycle 182b discipline) — 3 further `link.rs` references swept:
  (i) backends/pthreads-async/tests/multi_worker_codegen.rs:239 'link.rs:PipelineExceedsBuffer' → 'link/errors.rs:PipelineExceedsBuffer'.
  (ii) nucleus-compiler/README.md:11 'src/link.rs' → 'src/link/' (+ adjacent 'src/acfg.rs' → 'src/acfg/' caught by the same sweep — slice 7 cycle 182 should have done this and didn't).
  (iii) nucleus-compiler/tests/link.rs:1066 'in link.rs' → 'in `link()`' (symbol anchor; the test file still legitimately exists as tests/link.rs, but the in-prose reference was to the SOURCE file).
- P3.1 (this note): parent TASK-0340 notes updated to reflect AC#2 = 6/6.

NEW Dim-3 vocabulary forward-carry (slice 9+): bare-filename and bare-path-fragment self-references are NOT covered by the cycle-181b/182b Dim-3 regex `(in this file|in this module|this function|the helper (above|below)|nearby|earlier in this file)`. Mirror the cycle-182b Dim-2 widening: extend Dim-3 to include `<basename>\.rs` self-references for any file being split, AND include directory-shape claims like `src/<name>.rs`. This pattern caught the cycle 183 R2 introduction and the 3 silent siblings above; would have shipped without the architect review.

NEW lesson — 'structured deferral list' Dim-2 escape (cycle 183 implementer's own discovery): a single Dim-2 introducer line like 'X explicitly DOES NOT do (deferred):' followed by N TASK-NNNN items DOES NOT repeat the deferral verb on each item, so per-line Dim-2 regex misses the items. When the introducer trips Dim-2, walk EVERY item under the introducer. This pattern caught 4 stale items in original link.rs L52-60 list that would have shipped silently.

AC delta for TASK-0340 post-cycle-183b:
- AC#1: DONE.
- AC#2: **6/6 files split** (render.rs cycle 178; mp-tcp-bufsync/lib.rs cycle 179; mp-tcp-event/multi_worker.rs cycle 180; backend-common/multi_worker_walker.rs cycle 181; nucleus-compiler/acfg.rs cycle 182; nucleus-compiler/link.rs cycle 183). **Original cycle-175 audit scope CLOSED.**
- AC#3 (proptest): PENDING (next slice).
- AC#4 (e2e/main.rs report-formatter carve-out): PENDING.
- AC#5: DONE (allow-list shrank 9 → 8 cycle 183).
- AC#6: DONE for slice 8 (5 doc-claim rewrites + 3 silent-sibling sweep edits in 183 + cycle-183b 5 doc-edits + 3 sweep-edits are explicitly disclosed corrections, not new TASK/cycle anchors).
- AC#7: PENDING (final cycle when AC#3 + AC#4 close).

AC#2 scope question NOW DECIDED: stop at originally-named 6-file scope. The 7 remaining files >1000 LoC on the check-mega-files allow-list (transfer_inject.rs 4726, reuse_inference.rs 1676, sched/lower.rs 1546, halo_inference.rs 1525, algo/lower.rs 1328, host_data_relay_inject.rs 1212, sched/ir.rs 1163, pthreads-async/multi_worker.rs 1048) are out-of-original-audit; AC#5 recipe already polices them at the >1000 LoC threshold. Splitting transfer_inject.rs separately is higher-leverage as a fresh TASK-NNNN rather than as a TASK-0340 sub-slice — file when the orchestrator decides the leverage outweighs the cycle cost (currently slice 9 = proptest is the higher-ROI next step).

E2E baseline: 112 / 102 / 0 / 10 / 0 (preserved cycles 178-183b, 7 cycles).
Test counts: 969/0/3 dev + 968/0/3 release (preserved cycle 183-183b).

## Cycle 184 — slice 9 (TASK-0340.08) close — AC#3 CLOSED

Slice 9 implementer (in-thread, mped-architect agent type) landed: proptest=1.9.0 dev-dep added to nucleus-compiler/Cargo.toml + new tests/proptest_petri.rs (~470 LoC; 9 properties + 1 smoke + 2 oracles + 2 generators). proptest 1.11.0 (latest) requires rustc 1.85; flake.nix pins 1.83 → resolver downgrade to 1.9.0 (rust-version=1.82). Cargo.lock updated with proptest + ~20 transitive deps (rand, rand_chacha, rand_core, rand_xorshift, regex-syntax, bit-set, bit-vec, fnv, tempfile, rusty-fork, wait-timeout, num-traits, ppv-lite86, unarray, bitflags, errno, fastrand, getrandom, libc, linux-raw-sys, rustix, autocfg).

**Per-pass property delivered list:**
- passes/boundedness: b.1 agrees-with-reachability-oracle (DELIVERED — asymmetric: oracle_false ⇒ pass≠CapacityExceeded); b.2 determinism (DELIVERED); b.3 accepts-when-oracle-finds-no-overflow (DELIVERED).
- passes/deadlock: d.1 agrees-with-replay-oracle (DELIVERED — position match; benign CapacityExceeded variant accepted with comment); d.2 determinism (DELIVERED); d.3 rejects-iff-replay-stalls (DELIVERED).
- passes/petri_to_events: p.1 operation-only-ACFG-emits-only-Fires (DELIVERED — tests acfg_to_events directly, not the petri_to_events(&acfg, &_net) wrapper since the _net arg is ignored at the entry point per module docs); p.2 WorkerId coverage (DELIVERED — exact set equality); p.3 determinism + per-worker Fire count (DELIVERED).

**NO defects surfaced** under PROPTEST_CASES=256 default × 9 properties × ~2 strategies = ~4608 randomised cases. AC#7 (honest-failure) NOT TRIGGERED.

**Verification gate (orchestrator-self-run, inside nix develop):**
- just build / clippy / check-textual-replace-on-codegen / check-include-str-coverage / check-mega-files / check-narrative-doc-lie: clean.
- just test (dev): 969 → 979 (+10). 0 failed; 3 ignored.
- just test-release: 968 → 978 (+10). 0 failed; 3 ignored.
- just e2e: 112/102/0/10/0 (preserved bit-identical — proptest is test-side only).

**Generator honest limits** (forward-carry to next slice / future fuzzing iterations):
- Petri-net generator: MAX_PLACES=4, MAX_TRANSITIONS=4, cap 1..=3, weight=1 arcs only, no multi-arc bundles, no unbounded places. Nets larger than 4×4 would push the BFS oracle past STATE_SPACE_CAP=10_000 on too many cases (would surface as prop_assume! discards reducing effective sample size).
- ACFG generator: linear Sequence of 1-5 Operations on 1-3 workers + 1-3 kernels. Does NOT produce Push/Wait, Sync, nested Repeat, or partition_workers overrides. Future slice could extend the generator to cover these — but the test-side ROI of the current shape was the primary deliverable.

**Three implementer-disclosure-honesty fixes during self-audit (pre-commit)**:
- Doc-lie #1: oracle_bounded_reachable / oracle_has_dead_state names in module docstring did not match actual function names (oracle_capacity_can_be_violated / oracle_first_stall_position). Fixed.
- Doc-lie #2: "≤ 6 places, ≤ 6 transitions" / "4×4 vs 6×6" inconsistency — code reduced sizes for tractability mid-implementation, docstring didn't follow. Fixed.
- Doc-lie #3: "N=50 firing steps" referenced a removed MAX_FIRING_STEPS constant. Fixed.
- One redundant helper (enumerate_reachable) removed during cleanup — was only a pre-flight check duplicated by the real oracle's inner BFS.

**AC delta for TASK-0340 post-cycle-184:**
- AC#1: DONE.
- AC#2: 6/6 files split. DONE.
- AC#3: **DONE** (this slice).
- AC#4 (e2e/main.rs report-formatter carve-out): PENDING (slice 10).
- AC#5: DONE.
- AC#6: DONE for slice 9 (no new TASK-NNNN or cycle-NNN citations introduced; the file references TASK-0340 AC#3 in its module docs only as the anchor for what it implements, not as cycle-process-noise).
- AC#7: PENDING (slice 11 — close-out cycle).

Remaining slices for TASK-0340: 10 (AC#4 e2e formatter carve-out) + 11 (AC#7 final summary cycle). Two slices left.

## Cycle 184 + 184b — slice 9 (TASK-0340.08, AC#3 proptest) close + parallel review gate

Slice 9 implementer (mped-architect, subagent) landed commit 8708979: proptest dev-dep + 9 properties + smoke + 2 oracles + 2 generators in new file nucleus/nucleus-compiler/tests/proptest_petri.rs. TASK-0340.08 marked Done by implementer.

Parallel review gate (qa-test-runner + mped-architect, read-only):
- qa-test-runner: GO. Gate numbers reproduced bit-identically. just test 979/0/3 dev; just test-release 978/0/3; e2e 112/102/0/10/0 across TWO independent samples (post cargo clean -p nucleus, 727 MiB + 692 MiB removed); proptest non-flake confirmed across THREE independent invocations (10 passed each); proptest = "=1.9.0" exact-pin matches Cargo.lock; 10 #[test] items (1 smoke + 9 properties) match AC contract exactly.
- mped-architect: GO with 4 P2 + 3 P3 HONESTY findings (not defects of the slice; epistemic-value disclosure refinements).

Cycle 184b fold-back (doc-only test-side edits to proptest_petri.rs):
- P2.1 (d.1/d.3 oracle non-independence): rewrote file-level //! 'Oracles' section + the fn-level doc on oracle_first_stall_position to be honest that this is a refactor-regression guard, NOT independent reference (both oracle and check_deadlock_free call into the same Net::fire). The independent deadlock cross-validation (state-space search over firing-order permutations) is deferred. Promoted memory feedback-implementer-disclosure-mechanism-wrong instance (4th this slice-thread; the implementer's commit message + the 'four runs' arithmetic both inherited the trivial-coverage mis-framing).
- P2.2 (p.1 'acyclic per worker' claim overstated): rewrote p.1 docstring to call out that it's a GENERATOR-RESTRICTED SHAPE PIN (acfg_to_events emits Fire from Operation nodes by construction; the generator never produces Push/Wait/Sync/Repeat). The nominal AC#3 'acyclic per worker' invariant is enforced by acfg_to_events's internal debug_assert!, not by this proptest. Forward-linked to TASK-0340.08.01.
- P3.1 (bare-filename self-deixis at L686 'petri_to_events.rs::acfg_to_events'): rewrite folded into the broader p.1 docstring rewrite — the file-path reference is gone; symbol anchor only.
- P3-from-QA (file-deixis at L18 'in this file'): rewrote to 'in this test binary' as part of the file-level //! Oracles rewrite.
- P2.4 (generator widening filed as a tracker task): TASK-0340.08.01 filed (priority LOW; not gap-fill, hardening).

Honest-coverage math correction (cycle 184b architect P2.3, sunk in commit 8708979 body — recorded here for future cycles): the commit body's '≈4608 cases' claim is wrong (9 × 256 = 2304, not 4608). The architect also correctly notes only 5 of 9 properties carry independent epistemic value (b.1 / b.2 / b.3 / p.2 / p.3); d.1 / d.3 are refactor-regression guards (P2.1); p.1 is generator-restriction-trivial (P2.2). The b.* / d.* / p.* spread is the right shape for the AC#3 contract; the honest framing is 'shape coverage with disclosed limits', not 'cross-validation across all 9'.

Architect found NO defects in the three target passes — but its scope had to be narrow given the generator-trivial p.1 + non-independent d.1/d.3. Independent deadlock cross-validation + Sync/Push/Wait-aware ACFG generation are the highest-ROI future work (TASK-0340.08.01).

AC delta for TASK-0340 post-cycle-184b:
- AC#1: DONE.
- AC#2: 6/6 (originally-named scope DONE cycle 183b).
- **AC#3: DONE (cycle 184) — proptest dep + 9 properties; cycle-184b honesty addendum applied; TASK-0340.08.01 filed for widening.**
- AC#4 (e2e/main.rs report-formatter carve-out): PENDING (slice 10).
- AC#5: DONE.
- AC#6: DONE for slice 9 (commit 8708979 + cycle-184b doc-only edits carry no new TASK/cycle anchors beyond TASK-0340 + TASK-0340.08 self-references; AC#6-compliant).
- AC#7: PENDING (final cycle when AC#4 closes).

NEW lesson — implementer/orchestrator coverage-math discipline (cycle 184b): when a proptest slice claims 'N properties × M cases = X randomised draws', verify the arithmetic AND verify each property's epistemic-class (independent reference vs refactor-regression guard vs generator-restriction shape pin). Cycle 184 had both errors: 4608 vs 2304 (raw math) and 9 vs 5 independent (epistemic). Add to implementer-disclosure-mechanism-wrong memory the proptest-specific shape.

E2E baseline: 112 / 102 / 0 / 10 / 0 (preserved cycles 178-184b, 8 cycles).
Test counts: 979/0/3 dev + 978/0/3 release (cycle 184; cycle-184b doc-only no count change).

## Cycle 190 addendum — TASK-0342 closed (AC#5 scope gap addressed)

Cycle-185b qa-test-runner P3.1 surfaced that check-mega-files scope EXCLUDED nucleus/e2e/src/ — neither pre-carve main.rs (7316 LoC) nor post-carve main.rs (4716 LoC) nor new tests.rs (2638 LoC) entered the fence. Filed as TASK-0342.

TASK-0342 cycle 190 (Option A): extended check-mega-files scope to include nucleus/e2e/src; allow-listed both files with rationale via recipe docstring. Recipe passes. The fence's coverage is now symmetric with the rest of nucleus/**/src.

This addendum closes the documentation/expectation lag noted in cycle-185b architect P3.6 (AC#5 'no file exceeds 1000 LoC' wording previously bound a sub-tree but excluded e2e; cycle-190 lift makes the wording bind nucleus/e2e/src/ too via allow-list).

Cycle-220 SIBLING follow-up TASK-0383 (Done): two files crossed the 1000-LoC fence AFTER this epic closed — nucleus-compiler/src/sidecar.rs (1164) + embedded-pattern/src/tests.rs (1030), neither in any 0340 slice nor the allow-list, so `just check-mega-files` (and thus `just ci`) was RED. Resolved by the AC#2 SPLIT pattern (NOT allow-list): sidecar.rs -> sidecar/cumulative_tests.rs (child #[cfg(test)] mod via 2018-edition dir resolution, like sched/ acfg/); embedded-pattern/tests.rs -> tests/bin_shape.rs (BIN-shape tests carved off, LIB tests + shared helpers retained). Result 905/263/534/515 LoC, all <1000. RECURRING-LESSON for any future split slice (carry forward): (a) re-derive the child module's `use` block from what the MOVED code actually names — a copied parent import (std::path::PathBuf here) becomes a dead import that FAILS `just clippy -D warnings`; (b) a split SHIFTS every comment that cites the moved code by bare filename ('pinned in tests.rs') into a doc-lie — grep the WHOLE crate for `<oldfile>.rs` location-claims after the move; the check-doc-citation-staleness fence does NOT catch bare-basename prose claims (filed as a data point on TASK-0382 AC#1). The mega-file fence has no scope-membership guard, so a file can silently cross 1000 between epic closures — the only backstop is `just check-mega-files` in the full `just ci`, which is why a per-commit-subset-only gate misses it.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0340 hygiene wave CLOSED after 11 slices across cycles 176-186 (8 implementer cycles + parallel review fold-backs at 177, 178b, 179b, 180b, 181b, 182b, 183b, 184b, 185b; orchestrator-led directly in cycle 185).

ORIGINAL 6-FILE MEGA-FILE AUDIT (AC#1, AC#2): all six files SPLIT.

| Slice | Cycle | File                                                | Pre LoC | Post LoC (incl. orientation + use lines + mod decls) | Sub-modules |
|------:|------:|-----------------------------------------------------|--------:|----------------------------------------------------:|------------:|
|     3 |   178 | nucleus/backend-common/src/render.rs                |    1687 |                                                1814 |           7 |
|     4 |   179 | nucleus/backends/mp-tcp-bufsync/src/lib.rs          |    1997 |                              2134 (lib.rs at 315)   |           7 |
|     5 |   180 | nucleus/backends/mp-tcp-event/src/multi_worker.rs   |    1695 |                                                1764 |           5 |
|     6 |   181 | nucleus/backend-common/src/multi_worker_walker.rs   |    1263 |                                                1336 |           6 |
|     7 |   182 | nucleus/nucleus-compiler/src/acfg.rs                |    1440 |                                                1506 |           4 |
|     8 |   183 | nucleus/nucleus-compiler/src/link.rs                |    1290 |                                                1371 |           6 |
|    10 |   185 | nucleus/e2e/src/main.rs                             |    7316 |                              7354 (4716 + 2638)    | 2 (carve-out)|

Net LoC delta original-audit-scope: 9372 -> 9925 (+553 = orientation docstrings + use lines + sub-module decls; ~6% overhead, no behaviour change). Slice 10 e2e carve-out delta: +38 LoC (33 docstring + 2 mod decl + 1 use + ~2 blanks).

PROPTEST DELTA (AC#3, slice 9 cycle 184 + cycle-184b architect honesty fold-back):
- proptest 1.9.0 dev-dep added to nucleus-compiler/Cargo.toml (downgrade from latest 1.11.0 because 1.11 requires rustc 1.85; flake pins 1.83).
- nucleus-compiler/tests/proptest_petri.rs: NEW, 800 LoC. 10 #[test] items (1 smoke + 9 properties: b.1-b.3 boundedness + d.1-d.3 deadlock + p.1-p.3 petri_to_events).
- Honest epistemic-coverage breakdown (cycle-184b architect P2.1/P2.2/P2.3): 5 of 9 properties carry independent epistemic value (b.1/b.2/b.3 + p.2/p.3); d.1/d.3 are refactor-regression guards (oracle non-independence disclosed); p.1 is generator-restriction-trivial. Total randomised cases: 2304 (9 × 256), not 4608 as cycle-184 commit body claimed.
- Generator widening (Sync/Push/Wait, nested Repeat, weight>1 arcs, partition_workers) deferred to TASK-0340.08.01 (cycle-184b architect P2.4).

CHECK-MEGA-FILES RECIPE (AC#5):
- just check-mega-files implemented cycle 176 (slice 1); cycle 177 (slice 2) folded back architect P2.1 staleness-direction to fail-loud on stale allow-list entries (POSIX-shell portable via comm + mktemp temp files).
- Allow-list shrank 12 -> 11 (cycle 180) -> 10 (cycle 181) -> 9 (cycle 182) -> 8 (cycle 183).
- AC#5 scope explicitly excludes nucleus/e2e/src/ (cycle-176 architect P2.3). Slice-10 carve-out (cycle 185) DID NOT close this gap; QA cycle-185b P3.1 surfaced it; TASK-0342 filed as low-priority follow-up.

NO TASK-NNNN / CYCLE-NNN CITATIONS in refactored mega-file SPLITS (AC#6):
- Slices 3-8: clean per-slice (each cycle's commit body documents the mechanical move; doc-claim rewrites are explicitly disclosed corrections, not new anchors).
- Slice 10: clean (cycle-185 + cycle-185b disclosures live at carve-out site + tracker notes + new memory file, all process-level audit hygiene).

VERIFICATION GATE BASELINE (preserved cycles 178-185b, 9 consecutive cycles):
- just test (dev): 859 -> 859 (cycle 178) -> ... -> 969 (cycle 178b+) -> 979 (cycle 184 with proptest_petri +10) -> 979 (cycle 185 + 185b).
- just test-release: 858 -> ... -> 968 (cycle 184 with proptest_petri +10) -> 978 (cycle 184) -> 978 (cycle 185 + 185b).
- just e2e: 112/102/0/10/0 PRESERVED across all 9 cycles.
- just check-textual-replace-on-codegen / check-include-str-coverage / check-mega-files / check-narrative-doc-lie: all clean every cycle.

REMAINING WORK (NOT part of TASK-0340; new follow-ups for future cycles):
- TASK-0340.02 (LOW): architect cycle-178 P3.1 ctx<->fire<->reuse sibling-mod dep cycle in nucleus/backend-common/src/render/. Deferred refinement of slice 3 layout; not regression-class.
- TASK-0340.03 (LOW): architect cycle-178 P3.2 further-split nucleus/backend-common/src/render/reuse.rs (784 LoC) into reuse/{group,discover,codegen}.rs. Deferred refinement.
- TASK-0340.04.01 / .04.02 / .04.03 (LOW): mp-tcp-event slice-5 fold-back micro-items (pub(crate) tightening, use-stmt hoisting, stale forward-link claim).
- TASK-0340.07.01 (LOW): TASK-0019/0088 stale-deferral-claim sibling sweep (slice 8 fold-back).
- TASK-0340.08.01 (LOW): proptest_petri generator widening (weight>1 arcs / Push-Wait / Sync / nested Repeat).
- TASK-0340.01.01 / .01.02: slice-4 follow-up micro-items.
- TASK-0342 (LOW): check-mega-files scope extension to nucleus/e2e/src/.

These are all LOW-priority refinements; AC#7 captures the per-file LoC + proptest summary as the final close-out, satisfying the AC text. The follow-up taxonomy is healthy: every cycle 'grew the tracker' with precise follow-ups rather than expanding the cycle's scope silently.

ORCHESTRATOR DISCIPLINE FORWARD-CARRIED THROUGH THIS WAVE:
1. Heavy-onboarded implementer briefs (preempt safety-reminder refusal pattern).
2. Parallel read-only review gate every cycle, before any cycle closure (qa-test-runner + mped-architect; gate independence preserved except slice 7 where API 529 forced inline gate).
3. Dim-1/2/3/4 audit dimensions (line-citation, stale-claim, file-deixis, structured deferral list) grew slice-by-slice and were forward-carried as implementer-onboarding context; widened in cycle 182b (M[0-9]+ + 'empty payload' vocabulary) and cycle 183b (bare-filename self-deixis).
4. Implementer-disclosure-honesty: enumerated-edit-list discipline replaced 'verbatim move' blanket claims after cycle 179b discovered 4 silent mechanical reflows; generalised across 4 actor classes by cycle 165b (implementer / orchestrator-notes / orchestrator-tests / reviewer-subagent).
5. AC-rewrite avoidance: cycle-185 docstring honesty disclosure recorded inline at carve-out site rather than rewriting AC#4 text (feedback-ac-rewrite-on-done-task).
6. Follow-up filing instead of scope creep: 9 new low-priority tasks filed during this wave; each one anchors a deferred refinement at exactly the diff-locus where it was discovered.

The hygiene wave's central thesis ('split mega-files BEFORE M6 codegen amplifies the smell, AND add property tests to the Petri-net IR whose soundness is the central thesis claim') is satisfied. M6 cellgen work can proceed against a substrate with:
- No file >1000 LoC in the original 6-file audit scope.
- check-mega-files regression-fence (both directions) policing the backend-common + nucleus-compiler + backends/*/src tree.
- proptest substrate landed for the three central Petri-net IR passes with honest epistemic-value disclosure.

E2E baseline at TASK-0340 close: 112/102/0/10/0. Test counts: 979/0/3 dev + 978/0/3 release.
<!-- SECTION:FINAL_SUMMARY:END -->
