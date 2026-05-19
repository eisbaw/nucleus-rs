---
id: TASK-0186
title: 'e2e bin: empty-line-after-doc-comment clippy lint fails under --all-targets'
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 04:26'
updated_date: '2026-05-19 04:47'
labels:
  - compiler
  - tooling
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
cargo clippy --workspace --all-targets -- -D warnings fails. Pre-existing on clean master, NOT introduced by TASK-0154 (verified: cycle commits 36a27c2/8adcc6c touch zero e2e/test files; review gate confirmed). The project gate (just clippy / just ci) does NOT pass --all-targets so it is currently green, but the TEST targets have accumulated lint rot invisible to the gate. Known offenders found by the TASK-0154 review gate: nucleus/e2e/src/main.rs (~2256, empty-line-after-doc-comment on a commented-out doc block) PLUS pre-existing test-target lints in nucleus/compiler/tests/acfg_to_petri.rs and nucleus/compiler/tests/petri_to_events.rs (~5 lints). AC#1 cannot be satisfied by fixing only the e2e line — ALL --all-targets lints must be cleared. Fix each at root cause (convert/rephrase, not blanket #[allow] unless genuinely warranted). Then decide whether the project gate (just clippy / just ci) should adopt --all-targets so test-target lint rot is caught going forward (the architect review flagged this gate gap as real).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 cargo clippy --workspace --all-targets -- -D warnings is clean
- [x] #2 just ci still exit 0
- [x] #3 Decide and document (decision record or PRD note) whether just clippy / just ci should pass --all-targets; if yes, wire it into the justfile gate so test-target lint rot is caught
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0154 review gate (qa-test-runner + mped-architect, both GO): the --all-targets failure is broader than the single e2e doc-comment line originally noted — also ~5 lints in compiler/tests/acfg_to_petri.rs and compiler/tests/petri_to_events.rs. All are pre-existing on clean master (git blame: e2e doc region from 946159f6 / 8875bba TASK-0167; not this cycle). AC#1 (--all-targets clean) is the real bar — do not tick it by fixing only one file. The gate deliberately omits --all-targets today (honest, disclosed) but that is a genuine gap: test-target lints are invisible to just ci — hence the new AC#3 gate-adoption decision.

TASK-0186 complete. Full final lint set fixed under cargo clippy --workspace --all-targets -- -D warnings (exactly the 6 forward-carried; ZERO cascade after fixing):
- compiler/tests/acfg_to_petri.rs:484,504,527,544 clippy::len_zero -> assert!(!net.transitions.is_empty()); message arg "non-empty program" preserved at 484. Behaviour-preserving (cargo test green).
- compiler/tests/petri_to_events.rs:1085 clippy::type_complexity -> added type aliases DataCheck + SidecarExpectation (no #[allow]).
- e2e/src/main.rs:2253-2256 clippy::empty_line_after_doc_comments -> stale orphaned /// block (described a skip-entry test; now precedes fn cell + TASK-0163 banner, documents nothing) demoted to plain // comment, content kept verbatim (intent unconfirmed -> not deleted).
Gotcha: nix develop runs from REPO ROOT (flake there), not nucleus/ -> recipes must cd nucleus.
AC#3 decision = YES, documented in backlog/decisions/decision-0002. Argued vs PRD §12.3 (one short justfile, no opt-in one-off) + TASK-0057/0163/0167 gate-trust lineage (gate must catch what it claims; test-target rot invisible to just ci is that exact class). Rejected separate clippy-all (unenforced lint rots identically). Wired: justfile clippy recipe now --all-targets; ci unchanged (already calls just clippy = single source of truth).
CI cost finding: only test targets exist (zero benches/examples, zero [[bench]]/[[example]]). Measured warm: lib/bin clippy ~5s, --all-targets delta ~7s, just test ~46s. just test DOES fully compile test targets to run them but with codegen; clippy check artifacts do not fully substitute so the ~7s is real but bounded, small vs existing just test/just e2e legs in just ci.
Gate (all green): clippy --all-targets clean; cargo test --workspace all pass 0 failed; e2e 30/26/0/4 required-fail 0 (the just ci tail showing pass:25 fail:1 was the xbackend-check-negative leg biting CORRECTLY, not the e2e leg); determinism byte-identical; determinism-check-negative + xbackend-check-negative still bite; just clippy exit 0; just ci exit 0. Wiring honest by commit order: cleanup 0ffd293 BEFORE wiring 4680988.
Commits: 0ffd293 (cleanup), 4680988 (decision-0002 + justfile wiring). No AI/co-author credit (verified).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cleared all clippy --all-targets lint rot (pre-existing, gate-invisible) and made the gate catch it going forward.

What changed:
- Root-cause fixes (no blanket #[allow]): 4x clippy::len_zero in compiler/tests/acfg_to_petri.rs (assert! .len()>0 -> !is_empty()); clippy::type_complexity in compiler/tests/petri_to_events.rs (added DataCheck + SidecarExpectation type aliases); clippy::empty_line_after_doc_comments in e2e/src/main.rs (stale orphaned /// block -> plain // comment, content preserved).
- decision-0002 (accepted): project gate adopts clippy --all-targets, argued against PRD §12.3 and the TASK-0057/0163/0167 gate-trust lineage.
- justfile clippy recipe -> cargo clippy --workspace --all-targets -- -D warnings. ci recipe unchanged (calls just clippy; single source of truth).

Why: just ci was green while a whole category (test/bin targets) was unlinted — the same broken-gate class the project already guards with *-check-negative. Adoption closes it with a one-flag change, no new recipe.

User impact: test/bin-target lint rot is now a hard gate failure; cannot silently re-accumulate. CI grows a bounded ~7s test-target check pass (no benches/examples exist).

Tests/gate (all green, actual numbers): clippy --all-targets clean; cargo test --workspace all pass 0 failed; just e2e 30/26/0/4 required-fail 0; determinism byte-identical; determinism-check-negative + xbackend-check-negative still bite; just clippy exit 0; just ci exit 0 (green AFTER adoption only because rot was cleared first — commit order 0ffd293 then 4680988 is the honesty proof).

Risks/follow-ups: none blocking. Forward-carried to TASK-0065/0070/0074: gate clippy scope is now --all-targets; future clippy-policy work must not narrow it back.
<!-- SECTION:FINAL_SUMMARY:END -->
