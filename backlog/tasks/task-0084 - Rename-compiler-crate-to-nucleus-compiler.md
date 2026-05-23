---
id: TASK-0084
title: Rename compiler crate to nucleus-compiler
status: Done
assignee:
  - mark
created_date: '2026-05-18 00:05'
updated_date: '2026-05-23 20:47'
labels:
  - M1
  - infra
  - tooling
  - refactor
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
MPED architect review of TASK-0002 flagged: crate name 'compiler' is dangerously generic in grep terms. Once codebase grows, 'grep -rn compiler' will hit standard error messages, dep names, comments. Also closes the bin/crate-name-mismatch footgun (currently 'cargo run -p compiler' but 'cargo run --bin nucleus'). Renaming now is a cheap directory move + workspace-member edit + one Cargo.toml line; renaming later costs incrementally more per-import.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Crate directory renamed: nucleus/compiler/ -> nucleus/nucleus-compiler/.
- [x] #2 Workspace Cargo.toml members list updated accordingly.
- [x] #3 All imports/references in the codebase updated; cargo check + clippy + test all clean.
- [x] #4 Bin name 'nucleus' (in nucleus-compiler/Cargo.toml) unchanged — users still type 'nucleus build ...'.
- [x] #5 Test: 'just build && just test && just clippy' all green after the rename.
- [x] #6 Implementation notes record any imports that needed touching and any breakage encountered.
- [x] #7 Implementation notes record honest limitations (e.g. if some external doc or follow-up task references the old name, those need a follow-pass).
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. git mv nucleus/compiler -> nucleus/nucleus-compiler (preserves history).
2. nucleus/nucleus-compiler/Cargo.toml: package.name & lib.name = nucleus-compiler; KEEP [[bin]] name = nucleus.
3. nucleus/Cargo.toml workspace members: 'compiler' -> 'nucleus-compiler'.
4. 7 Cargo.toml dep updates: backend-common, driver, test-common, e2e (if any), backends/{pthreads-sync,mp-tcp-bufsync,pthreads-async,mp-tcp-event}: compiler -> nucleus-compiler with new path.
5. Rust source: every 'use compiler::' / 'compiler::' qualified path -> nucleus_compiler:: (underscore). Use careful sed.
6. Mandatory doc-lie audit: grep -rn 'compiler::|\bcompiler\b' for leftover comments. Update doc-comments, code comments naming the crate-path.
7. Example reference Cargo.toml comments ('no dependency on the compiler crate') -> 'nucleus-compiler crate'.
8. README updates: compiler/README.md (now nucleus-compiler/README.md) + 'cargo test -p compiler' -> 'cargo test -p nucleus-compiler'.
9. cargo doc --workspace --no-deps gate for stale rustdoc links.
10. Full gate: just check, just clippy, just test, just e2e (88/70/0/18 unchanged), just determinism-check + 3 negatives, port-stress 20/20.
11. Backlog/docs audit for stale references (per-file decide: update vs historical record).
12. Commit per logical unit; no AI co-author credit.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 76 (commit 71057ba): rename landed.

Mechanical surface:
- git mv nucleus/compiler -> nucleus/nucleus-compiler (78 files renamed, history preserved).
- nucleus/nucleus-compiler/Cargo.toml: package.name='nucleus-compiler', lib.name='nucleus_compiler'.
- Workspace members list: 'compiler' -> 'nucleus-compiler' (with TASK-0084 comment).
- 7 dependent Cargo.toml dep entries (backend-common, driver, test-common, all 4 backends): compiler -> nucleus-compiler with hyphen.
- 57 .rs files: 'use compiler::' / 'compiler::' -> 'use nucleus_compiler::' / 'nucleus_compiler::' (sed; underscore form for extern crate import).
- nucleus/Cargo.lock auto-regenerated.

Doc-lie audit (the recurring failure-class for this session per memory feedback-comment-doc-lie-recurring):
- 21 distinct comment/doc-comment sites in 16 files updated to name the new crate path. Categorized into 'crate-path references' (UPDATED) vs 'English / generic-concept usage' (LEFT).
- Examples of LEFT (concept, not crate): 'compiler-internal invariant', 'compiler bug', 'compiler-pass', 'the compiler does/treats/partitions', 'the nucleus pre-compiler' banner strings, PRD/SKETCH prose references (~12 mentions).
- Examples of UPDATED: crate doc headers, CARGO_MANIFEST_DIR comment, NameTables module doc + struct doc (4 sites), Cargo.toml path refs, e2e/src/main.rs (3 sites), test-common/src/lib.rs (3 sites), decision-0001 (3 sites), docs/check-loop-latency-max.md + docs/wire-protocol-v0.md (compiler::event::* paths), all 10 example reference/Cargo.toml comments, 4 example reference/src/main.rs main-doc comments, 6 example README path links.
- Bonus fix in mp-tcp-bufsync/tests/check_frame_emit.rs: doc comment pointed at 'nucleus/compiler/tests/check_frame_codegen.rs' but the file lives at backends/pthreads-sync/tests/ (file moved at TASK-0225 but the comment had not been updated). Corrected to the real path AND renamed the prefix.
- backlog/tasks/ historical references to 'cargo test -p compiler' INTENTIONALLY NOT REWRITTEN (workflow: tracker entries are dated records).

Surprise vs the task brief:
- Brief said [[bin]] section is in compiler/Cargo.toml. It is actually in driver/Cargo.toml (TASK-0020 had already promoted the bin out of compiler/ to its own crate). The bin name 'nucleus' was preserved either way — AC#4 holds.
- Brief said ~50 import sites. Actual: 2285 'compiler::' occurrences across 57 .rs files (so much wider; sed handled cleanly).

Gate (full tier-1 + cycle-72 stress baseline):
- just check / clippy / test: all clean.
- just e2e: 88/70/0/18 UNCHANGED (zero-behavior rename confirmed).
- just determinism-check: clean.
- just determinism-check-negative: BITES.
- just xbackend-check-negative: BITES.
- just required-coverage-check-negative: BITES.
- just port-stress-check 20: 20/20.
- cargo doc --workspace --no-deps: 72 pre-existing warnings unchanged, no new compiler:: doc-link warnings.

Honest limitations:
- backlog/tasks/*.md task bodies and progress notes retain 'compiler/...' references as dated historical record. A future 'grep -rn compiler' from the repo root will surface those — acceptable.
- nuc-nucleus/PRD.md and SKETCH.md use 'the compiler' generically ~12 times. These are conceptual; rewriting to 'the nucleus-compiler' would damage the prose. Left as-is.
- Generated-code banner strings 'Generated by the nucleus pre-compiler.' in backend-common/src/project_skeleton.rs were left unchanged: they are byte-identical with golden test fixtures and the cross-backend bit-identical differential oracle expects them verbatim. Changing them would cascade into test fixture rebaseline.

Cycle 76 review-gate hardening (commit below): the 139-file mechanical rename surfaced the recurring doc-lie failure class AT MAXIMUM AMPLITUDE this session — mped-architect found 3 missed sites, qa-test-runner found 2 MORE. Total 5 doc-comment / doc-link sites missed in the implementer's initial audit. All 5 fixed in-thread: (1) README.md:36 'nucleus/' tree description still said 'compiler/' (top-level repo entry point — most-visible miss); (2) examples/03-reduction/README.md:182 '(compiler/tests/e2e_example_03.rs)' (6 of 7 example READMEs were updated, 03 missed despite same shape); (3) decision-0002 and (4) decision-0003 retained 5+3 stale 'compiler/{src,tests}/...' path citations (decision-0001 was updated, 0002+0003 were missed despite same category); (5) e2e_example_01.rs:9-14 doc-comment + inject_check_frames.rs:12 doc-comment had stale 'compiler/tests' and 'compiler crate' refs (inside the renamed-directory itself — the implementer's audit grep missed because they were doc-comments in the just-moved files). Plus one minor implementer-claim correction: cargo doc warning count claim was '~72 unchanged' actual is 61 (favorable direction; doc-lie risk class but not behavior). Reliability signal: 7 cycles this session with MAJOR counts 0,0,3,3,3,4,3 — the doc-lie recurrence is firmly established. Process recommendation for future renames (per mped-architect MINOR-1): run grep TWICE — once for the keyword, once for keyword AND NOT new-name — would have caught all 5 missed sites in under a minute. qa-test-runner GO + mped-architect GO-with-conditions; all 5 sites + the count claim addressed in-thread.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Compiler crate renamed to nucleus-compiler in cycle 76 (commit 71057ba). All 7 ACs MET, full gate green (just check/clippy/test/e2e/determinism + 3 negatives all clean; just e2e 88/70/0/18 unchanged; just port-stress-check 20/20). The rename is zero-behavior. Mass-mechanical sed across 57 .rs files + careful per-file Cargo.toml edits + targeted doc-lie audit across 21 sites in 16 files (the recurring failure-class for this session; classified as crate-path-reference vs English-concept on each match). bin name 'nucleus' preserved (lives in driver/Cargo.toml, not compiler/ — TASK-0020 had already promoted it; brief was out-of-date on layout). Historical backlog/tasks/ references to 'compiler' intentionally left as dated records. PRD/SKETCH 'the compiler' generic prose left as conceptual.
<!-- SECTION:FINAL_SUMMARY:END -->
