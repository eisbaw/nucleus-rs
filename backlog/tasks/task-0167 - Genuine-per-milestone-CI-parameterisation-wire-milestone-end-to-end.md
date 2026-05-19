---
id: TASK-0167
title: Genuine per-milestone CI parameterisation (wire --milestone end to end)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-18 22:23'
updated_date: '2026-05-19 02:45'
labels:
  - infra
  - tooling
  - M1
dependencies:
  - TASK-0057
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0057 found the CI milestone matrix was cosmetic (7 identical jobs; the e2e harness --milestone flag is accepted-but-ignored per nucleus/e2e/src/main.rs ~1526; nuc-nucleus/e2e-matrix.toml has no milestone dimension). The decorative matrix was removed and AC#3 of TASK-0057 honestly unchecked. This task is the REAL work AC#3 wanted: (1) make the e2e harness honour --milestone (subset the required cells by milestone); (2) add a milestone key to each cell in e2e-matrix.toml; (3) reinstate a CI matrix keyed on milestone that actually runs a different required set per milestone; (4) PRs to a milestone branch run that milestone tier. Until then PRD §11 milestone-gating is not real.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 e2e harness honours --milestone: required-set is subset by milestone, verified by a test
- [x] #2 e2e-matrix.toml carries a per-cell milestone key
- [x] #3 CI matrix runs a genuinely different required set per milestone (not identical jobs)
- [x] #4 A PR to a milestone branch runs that milestone tier (documented + matrix wired)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. e2e-matrix.toml: add per-cell `milestone` (string) to every [[required]] and [[skip]] entry. Scheme = PRD §11 "milestone whose acceptance owns the cell": M1 = pthreads-sync examples 1-3 naive + 02-split/split (the M1-era algo/schedule split keystone); M2 = pthreads-sync blocked + remaining 05/07 naive (M2 ships blocked.sched.nuc for 5,7 + determinism); M3 = ALL mp-tcp-bufsync cells (second backend lands M3) + ALL 04-prefix-sum + ALL 06-separable-filter (examples 4,6 only required from M3). Document the scheme in the manifest header.
2. main.rs structs: add `milestone: String` to Cell (the [[required]]/[[skip]] tables), keep deny_unknown_fields. Cell is used as a BTreeSet key for required/skip matching — milestone must NOT participate in identity matching (planned cells have no milestone). Solution: keep Cell {example,schedule,backend} as the identity triple; add a NEW struct (RequiredEntry/SkipEntry) carrying milestone + the triple, OR add `#[serde(default)] milestone` to Cell but exclude it from Ord/Hash/Eq. Chosen: introduce explicit MatrixEntry { #[serde(flatten? no-deny)] } — decide during impl; safest = separate RequiredEntry struct with milestone + triple, Manifest.required: Vec<RequiredEntry>, derive Cell triple for set ops.
3. milestone ordering: cumulative gate. milestone_rank(M1)=1,M2=2,M3=3. --milestone M<k> keeps required cells with rank <= k. Typed error (NOT panic) on unknown --milestone value or unknown per-cell milestone string. Document cumulative choice in --help + manifest + ci.yml.
4. plan_cells: thread milestone narrowing — a required cell with rank > requested is excluded from the planned/required set. cell_matches_filters: add milestone axis IN LOCKSTEP so required_coverage_gaps scopes identically (a typo'd M3 required cell run under --milestone M3 must STILL hard-fail with the triple named).
5. Replace the accepted-but-ignored block (~1668) with real validation: parse/validate --milestone -> typed error on unknown.
6. Tests: (a) milestone subsetting (M1 subset count < full, all pass-plan); (b) NEW coverage-gap regression mirroring typo_in_required_schedule_is_a_coverage_gap but with --milestone M3 + an M3-tagged typo'd required cell -> gap with triple named; (c) cumulative semantics (M3 includes M1 cells); (d) unknown --milestone -> typed err; (e) real manifest per-milestone has zero gaps.
7. justfile: add `e2e-milestone M` recipe delegating to nucleus-e2e --milestone. ci.yml: reinstate GENUINE matrix (M1/M2/M3) each running nix develop -c just e2e-milestone M<k> (different required sets), keep just-delegation SSOT; document PR-to-milestone-branch tier.
8. Gate: just test / e2e (28/24/0/rf0 unchanged) / per-milestone counts differ / determinism-check + negative / clippy -D warnings / ci.yml valid YAML. Commit per logical unit, no push.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
forward-carried from TASK-0163: when --milestone is wired end-to-end, each milestone subset of [[required]] MUST still pass the required_coverage_gaps() guard in nucleus/e2e/src/main.rs. The guard scopes by CLI filters via cell_matches_filters; --milestone is a NEW narrowing axis, so it must be added there (and to plan_cells) IN LOCKSTEP, else a milestone-tagged required cell with a typo'd/stale schedule re-introduces the exact silent-vanish blind spot (green CI, deleted gating cell). Add a regression test mirroring typo_in_required_schedule_is_a_coverage_gap but exercising the milestone filter.

Implemented (TASK-0167):
- e2e-matrix.toml: per-cell `milestone` key on all 24 [[required]] + 4 [[skip]]. Scheme documented in header = PRD §11 "milestone whose acceptance owns the cell": M1=pthreads-sync 1-3 naive + 02/split; M2=pthreads-sync 05/07 naive+blocked; M3=all mp-tcp-bufsync + all 04/06.
- main.rs: new Milestone(u8) typed (parse rejects non-M<k> / >M6 with typed error, never panic); RequiredEntry struct carries milestone (Cell stays the bare identity triple used for set-matching); SkipEntry gains milestone. Milestone parse-validated at manifest load + at --milestone CLI parse (fail loud).
- CUMULATIVE gate chosen + documented (manifest header, --help, ci.yml): --milestone M3 runs M1∪M2∪M3. Tight-tier semantics: --milestone narrows which cells EXECUTE to in-band required+skip (not just which gate), so a tier job is exactly its tier.
- TASK-0163 LOCKSTEP: shared milestone_in_gate() predicate used by BOTH plan_cells (required/skip flagging + execute-or-skip) AND required_coverage_gaps (coverage obligation + in-band skip exemption). Proven end-to-end: a typo'd M1 required cell under --milestone M3 (cumulative ⇒ M1 in band) hard-fails naming the triple.
- Tests: +6 (milestone parse, bad --milestone typed err, cumulative subsetting, THE lockstep regression mirroring typo_in_required_schedule_is_a_coverage_gap on the milestone axis, out-of-band-skip-does-not-exempt, per-tier zero-gaps, strictly-grow counts). e2e crate 25/0.
- justfile: e2e-milestone M recipe. ci.yml: genuine milestone matrix [M1,M2,M3] running just e2e-milestone (different sets) + unchanged full `gate` job. js-yaml valid.
GATE: just test workspace 0 failed; bare just e2e 28/24/0/skip4/req-fail0 UNCHANGED; per-milestone 4/8/28 cells (genuinely differ); determinism-check 28/24/0/4 byte-identical; negative arm bites; clippy -D warnings clean.

ORCHESTRATOR REVIEW GATE (phase3-ralph): qa-test-runner GO + mped-architect GO, both read-only. Numbers RE-RUN by reviewers (not transcribed): just test e2e-crate 25/0 (was 19, +6: typo_in_milestone_tagged_required_is_a_gap_under_that_milestone, out_of_band_skip_does_not_exempt_in_band_required, real_manifest_has_no_coverage_gaps_at_every_milestone, required_counts_strictly_grow_per_milestone, milestone_parse_*, arg_parser_rejects_bad_milestone) + the prior TASK-0163 tests still green; bare just e2e UNCHANGED 28/24/0/skip4/required-fail0; per-milestone GENUINELY DIFFERENT 4(M1)/8(M2)/24-required-28-executed(M3) all exit0; determinism byte-identical; negative bites x2; clippy clean; ci.yml valid YAML (jobs gate+milestone, matrix [M1,M2,M3]); no AI credit; tree clean. LOCKSTEP PROVEN: single milestone_in_gate() predicate is the SOLE scope comparator, consumed by both plan_cells AND required_coverage_gaps (architect traced both paths; qa reproduced — a transient typo on an M1-tagged [[required]] cell hard-failed under BOTH --milestone M1 AND M3 naming the exact triple, then reverted clean). Matrix GENUINELY non-cosmetic (the exact TASK-0057 defect fixed). Tag taxonomy real+documented (PRD §11 ownership; lopsided 4/4/16-required proves honest not reverse-fitted). Typed Milestone parse = typed error not panic-not-diagnostic. ORCHESTRATOR HARDENING (architect P2/P3, the recurring doc-misstates-a-number / stale-rustdoc class): corrected ci.yml comment "required 4/8/28" -> "4/8/24 required; M3 executes 28 incl 4 skip" (both occurrences); fixed main.rs:106 stale `Manifest::load` rustdoc -> required_milestones/skip_table. Verified inert (ci.yml YAML OK; cargo check --workspace Finished clean). Filed TASK-0182 (qa advisory: pre-existing flaky e2e build-dir/CWD race under rapid/concurrent runs — NOT a 0167 regression, encoded). TASK-0167 Done is HONEST: all 4 ACs met + independently verified (AC#4 gate-logic-verified; real-runner observation is the same standing limitation tracked under TASK-0057/0166).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Wired --milestone end to end so PRD §11 milestone-gating is genuine, not cosmetic.

What changed:
- nuc-nucleus/e2e-matrix.toml: per-cell `milestone` key on all 24 [[required]] + 4 [[skip]] entries; scheme documented in the header (PRD §11 "milestone whose acceptance task owns the cell": M1/M2/M3).
- nucleus/e2e/src/main.rs: typed Milestone(u8) (parse rejects non-M<k>/>M6 with a typed error — never panic, never silent default; validated at manifest load AND --milestone parse); RequiredEntry/SkipEntry carry milestone while Cell stays the bare identity triple for set-matching; cumulative tight-tier gate.
- justfile: `e2e-milestone M` recipe. .github/workflows/ci.yml: genuine milestone matrix [M1,M2,M3] each running `just e2e-milestone M<k>` (different required sets), plus the unchanged full-trust `gate` (just ci). js-yaml valid.

Decisions:
- CUMULATIVE (not exact): --milestone M3 = M1∪M2∪M3 — a regression gate must never drop an earlier tier. Documented in manifest header, --help, ci.yml.
- Tight tier: --milestone narrows which cells EXECUTE (in-band required+skip only), so a tier job is exactly its tier (M1=4 cells all pass exit0; M2=8; M3=28).

TASK-0163 lockstep (load-bearing): one shared milestone_in_gate() predicate drives BOTH plan_cells and required_coverage_gaps (coverage obligation + in-band skip exemption), so a typo'd/stale milestone-tagged required cell still hard-fails inside its tier. Proven end to end: a transient typo'd M1 required cell under --milestone M3 hard-failed naming the triple, then reverted clean. New regression test mirrors typo_in_required_schedule_is_a_coverage_gap on the milestone axis; +6 tests total (e2e crate 19→25, all green).

Gate (nix develop -c, all green): just ci EXIT 0; workspace tests 0 failed; bare just e2e 28/24/0/skip4/required-fail0 UNCHANGED; per-milestone required cells 4/8/28 (genuinely differ — matrix is real); determinism-check 28/24/0/4 byte-identical; determinism-check-negative bites; clippy --workspace -D warnings clean; ci.yml valid YAML (js-yaml, jobs=gate,milestone, matrix=[M1,M2,M3]).

Commits: 38f37e0 (harness+manifest), 4091172 (ci+justfile). No push (no remote). No AI credit.

Forward-carry (orchestrator, do not self-check): TASK-0057 AC#3 now satisfiable (genuine milestone matrix exists); TASK-0041 AC#1 (`just e2e --milestone M3` exits 0 — verified) and AC#4 (CI runs the M3 tier every commit) now satisfiable.

Limitations: no git remote/runner — the ci.yml milestone matrix is verified at gate-logic level (the exact `just e2e-milestone M<k>` commands run locally with the proven 4/8/28 split) but NOT observed on a real GitHub Actions runner (same standing limitation as TASK-0057). Skip milestone tags are all M3 (distributed cells are post-M3 work and not [[required]], so the tag only affects which run reports them SKIPPED — documented inline).
<!-- SECTION:FINAL_SUMMARY:END -->
