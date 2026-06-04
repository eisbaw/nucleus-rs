---
id: TASK-0439
title: >-
  Doc-honesty sweep README + PRD count/feature drift + 'check-readme-counts'
  gate recipe
status: Done
assignee:
  - mark@radix63.dk
created_date: '2026-06-04 08:16'
updated_date: '2026-06-04 08:54'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Per WIP review (2026-06-04). README.md claims 'fourteen worked examples' (reality: 21 in nuc-nucleus/examples/). PRD §9 claims 12 algorithms (reality: 21). PRD §10.3 claims multi-MCU 'over SPI' (reality: UART, per TASK-0049.01 - Renode has no MCU-to-MCU SPI hub). The existing 'just check-narrative-doc-lie' (TASK-0339) does NOT police PRD/README counts.

Scope: (1) Sweep README + PRD reconciling example/backend counts and SPI->UART; (2) Add 'just check-readme-counts' recipe that greps the example directory cardinality and fails the gate on cardinality drift (same pattern as check-mega-files).

Why: Cheapest credibility win - prevents reviewer first-impression of 'fuzzy project'.

Estimated effort: ~half cycle, LOW priority. No code change; doc + 1 justfile recipe.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 README.md example-count claim (line ~28 'fourteen worked examples') reconciled to the real shipped count (21 dirs in nuc-nucleus/examples/)
- [x] #2 PRD stale '12' count sites reconciled to truth: §10.4/M6 line ~1133 'All 12 algorithms' and risks line ~1274 '12 examples' — to the §9 driving count (14) or shipped (21), each chosen to be TRUE in its context, not blindly substituted
- [x] #3 PRD §9 (14-row driving table) reconciled with the 21 shipped dirs: add an honest note that examples 15-21 are later extensions per §9's own 'added later' clause (do NOT fabricate table rows/stresses)
- [x] #4 PRD Nucleus-multi-MCU SPI overclaims fixed to UART: line ~1152 (master+sensor STM32 'connected over SPI') and line ~1084 ('over SPI or Ethernet'); per TASK-0049.01 reality (UART hub, no MCU-to-MCU SPI in Renode)
- [x] #5 PRD line ~1070 'Renode ... over UART/SPI/I2C/Ethernet' LEFT INTACT — it is a true statement about Renode's general capability, not a Nucleus claim (distinguish the two)
- [x] #6 new 'just check-readme-counts' recipe: derives example-dir cardinality from the filesystem, compares to a greppable declared count in README, FAILS on drift (both directions); wired into 'just ci' alongside check-mega-files/check-narrative-doc-lie
- [x] #7 MUTATION TEST: synthetic count drift (extra dir OR bumped declared count) makes 'just check-readme-counts' exit non-zero; revert -> green; result recorded
- [x] #8 cheap gate green (build/clippy/test/e2e 427/364/0/63/0) + new recipe + existing doc fences pass; renode-multimcu-gate byte-exact (structurally unaffected by doc edits) confirmed
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
PLAN (impl, cycle 2026-06-04):
1. README L28: "fourteen worked examples" -> "twenty-one worked examples" + add greppable sentinel HTML comment <!-- check-readme-counts: examples=21 --> next to the examples bullet. Keep descriptive range (elementwise-add..CNN..multi-MCU hearing aid) honest.
2. PRD L1133 (M6 milestone "All 12 algorithms"): -> "All 14 driving examples (§9)" — M6 meant the curated §9 driving set; NOT anachronistic 21.
3. PRD L1274 (risks single-file rule "12 examples"): -> "21 examples" — present-tense current count is honest here.
4. PRD §9 after table (~L1011): add ONE honest sentence noting examples 15-21 are later extensions per the "added later" clause; examples/ ships 21 total. No fabricated rows/stresses.
5. SPI->UART: L1152 (master+sensor STM32 over SPI) -> UART; L1084 (workers on different MCUs over SPI or Ethernet) -> UART. Per TASK-0049.01 (UART hub).
6. LEAVE L1070 (Renode general UART/SPI/I2C/Ethernet capability) INTACT.
7. Add just check-readme-counts recipe (model on check-mega-files: filesystem dir cardinality vs README sentinel, fail both directions, set -eu + mktemp + trap). Scope: README only (NOT PRD §9 curated 14). Wire into just ci near check-mega-files.
8. Mutation-test the gate; cheap subset green; doc fences pass.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
PRE-BRIEF (orchestrator 2026-06-04): exact reconciliation map. THREE distinct numbers in play — do NOT blind-substitute:
- STALE '12': PRD line 1133 ('All 12 algorithms', M6 milestone) + line 1274 ('12 examples × N schedules', risks/single-file-rule). These predate examples 13/14 entering the §9 table.
- CURATED 14: PRD §9 (lines 951-966) is a 14-ROW driving-example table (1..14). It is intentionally a curated subset; line 1009-1011 explicitly says 'Examples added later must justify...'. The architect's 'PRD claims 12' was imprecise — §9 is 14; the literal '12's are at 1133/1274.
- SHIPPED 21: ls -d nuc-nucleus/examples/*/ = 21 (14 driving + 7 later: 15-transpose,16-jacobi,17-spmv,18-multigather,19-histogram-unconstrained,20-index-cast-permute,21-jacobi-converge).
README line 28 'fourteen worked examples' -> 21.
SPI sites: line 1152 (master+sensor STM32 'connected over SPI', M11) + line 1084 ('workers on different MCUs connected over SPI or Ethernet') are NUCLEUS-multi-MCU claims -> fix to UART (TASK-0049.01). BUT line 1070-1071 ('Renode ... can co-simulate ... over UART/SPI/I2C/Ethernet') is a TRUE general statement about RENODE's capability — LEAVE IT. Distinguish 'Renode supports X' (true) from 'Nucleus multi-MCU uses X' (UART only).
GATE: model on check-mega-files (justfile:1037) = filesystem-truth vs declared, fail both directions. Scope to README example-count vs dir count ONLY (NOT PRD §9, which is a curated 14 ≠ 21 and would false-positive). Use a greppable sentinel in README (e.g. an HTML comment) for the declared count. Wire into 'just ci' at justfile:188-195. Mutation-test it.
No Rust change. Renode byte-exact is structurally unaffected by doc/justfile edits (no codegen touched) — qa reviewer to confirm once.

IMPL DONE (cycle 2026-06-04, commits cc4a2c7 docs + 2175c8e build).

EDITS (before -> after):
- README L28: "fourteen worked examples" -> "twenty-one worked examples (fourteen driving examples per PRD §9 plus seven later extensions, 15-21)"; added sentinel "<!-- check-readme-counts: examples=21 ... -->" after the reference.bin line.
- PRD M6 (was L1133): "All 12 algorithms × required schedules ..." -> "All 14 §9 driving examples × required schedules ...". M6 meant the curated §9 set; not anachronistic 21.
- PRD risks single-file rule (was L1274): "12 examples × N schedules" -> "21 examples × N schedules". Present-tense current count is honest here.
- PRD §9 (after "added later" clause, ~L1011): added one honest paragraph naming examples 15-21 as later extensions; no fabricated table rows/stresses.
- PRD §10.3 (was L1084): "workers on different MCUs connected over SPI or Ethernet" -> "over UART" (TASK-0049.01 UART hub).
- PRD M11 (was L1152): "master STM32 + sensor STM32 connected over SPI" -> "over UART".
- LEFT INTACT: PRD Renode general-capability line "UART/SPI/I2C/Ethernet" (now L1080 after §9 insertion) — true RENODE statement, not a Nucleus claim. Verified grep -i SPI shows ONLY this line remains.

GATE: just check-readme-counts (justfile, wired into just ci before check-doc-links). Filesystem truth = find nuc-nucleus/examples -mindepth1 -maxdepth1 -type d | wc -l; declared = grep sentinel in README. Fails both directions + missing-sentinel. Scope: README only (NOT PRD §9 curated 14 — comment in recipe explains).

MUTATION TEST evidence:
- Clean tree: exit 0 ("OK ... matches ... (21)").
- Direction A (mkdir 99-synthetic-mutation-test, actual=22 > declared=21): exit 1 (FAIL drift). rmdir -> exit 0.
- Direction B (sed sentinel ->99, declared=99 > actual=21): exit 1 (FAIL drift). revert -> exit 0.
- Synthetic dir confirmed removed; NOT committed.

CHEAP SUBSET (all green): just test 1369/0/3; just test-release 1367/0/3 (2-test delta = TASK-0291 debug_assert should_panic divergence, expected); just e2e total:427 pass:364 fail:0 skipped:63 required-fail:0 (EXACT baseline). Doc fences: check-narrative-doc-lie + check-include-str-coverage + check-textual-replace-on-codegen all OK.

renode-multimcu-gate NOT run (structurally unaffected by doc/justfile edits, no codegen touched) — qa reviewer to confirm once per brief.
<!-- SECTION:NOTES:END -->
