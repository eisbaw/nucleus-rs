---
id: TASK-0167
title: Genuine per-milestone CI parameterisation (wire --milestone end to end)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-18 22:23'
updated_date: '2026-05-19 02:33'
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
<!-- SECTION:NOTES:END -->
