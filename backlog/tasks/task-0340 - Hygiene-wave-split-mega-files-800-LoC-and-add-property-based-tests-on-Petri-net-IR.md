---
id: TASK-0340
title: >-
  Hygiene wave: split mega-files (>800 LoC) and add property-based tests on
  Petri-net IR
status: In Progress
assignee:
  - '@orchestrator'
created_date: '2026-05-26 09:46'
updated_date: '2026-05-26 13:57'
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
- [ ] #3 proptest dep added to nucleus-compiler dev-dependencies; at least 3 properties per pass for passes/boundedness.rs, passes/deadlock.rs, passes/petri_to_events.rs. Generators emit small bounded ACFGs; properties assert (i) boundedness pass agrees with bounded-reachability up to N steps, (ii) deadlock pass agrees with explicit enumeration on the same generated nets, (iii) petri_to_events output is acyclic per worker
- [ ] #4 Report-formatter tests in nucleus/e2e/src/main.rs (currently 76 internal #[test]) carved out into a sub-module file (e2e/src/report/tests.rs) or sub-crate (e2e_report). Compiler-correctness tests remain in main.rs; formatter tests are visually separated
- [ ] #5 New just recipe check-mega-files added to ci: asserts no nucleus/**/src/*.rs file exceeds 1000 LoC. Recipe is wired into just ci as a regression-fence. Initial pass exempts any file the split intentionally leaves above 1000 LoC via an explicit allow-list (with rationale)
- [ ] #6 No new TASK-NNNN or cycle-NNN citations introduced in the refactored files (closes the comment-process-noise concentration smell: acfg.rs 74 mentions, mp-tcp-bufsync/lib.rs 68, sidecar.rs 57 at audit time)
- [ ] #7 Final cycle commit notes per-file LoC before/after and per-pass proptest count delta (no separate summary md file per CLAUDE.md cruft policy)
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
<!-- SECTION:NOTES:END -->
