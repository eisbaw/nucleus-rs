---
id: TASK-0433
title: 'Front-end: reject (or r#-escape) DSL identifiers colliding with Rust keywords'
status: Done
assignee:
  - '@me'
created_date: '2026-06-03 03:33'
updated_date: '2026-06-03 04:52'
labels:
  - compiler
  - frontend
  - panic-not-diagnostic
  - codegen
  - cycle-248
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0431 cycle-248 architect P3-2 (real latent defect CLASS, not a one-off; surfaced when an example data symbol named `in` generated `let mut in = ...` and failed rustc — worked around by rename in->src). ROOT CAUSE: the DSL KEYWORDS list (nucleus/nucleus-compiler/src/algo/parser.rs:155-175) rejects only DSL grammar words (const/data/kernel/for/scalar-types); it does NOT reject Rust keywords. So a data/kernel/worker identifier named in/let/mut/match/move/ref/loop/fn/type/as/self/crate/... is ADMITTED by the front-end and then emitted as `let mut <kw> = ...` by the `let mut {name}` codegen present in EVERY backend (tcp_plan, event_plan, mpi_plan, pthreads-*, openmp-rs, embedded-pattern). The failure surfaces as a confusing rustc parse/type error pointing at GENERATED source the user never wrote, not at their .nuc line — the project panic-not-diagnostic / usability-footgun class. FIX (pick one): (a) fail-loud front-end check listing Rust strict (and ideally reserved) keywords, emitting an EmitError/parse diagnostic at the .nuc identifier site; or (b) r#-escape identifiers in the data_name / `let mut {name}` codegen path so any identifier is legal. Prefer (a) for a clearer diagnostic, or (b) for max DSL freedom. Add a negative test (a .nuc with `data in : ...`) proving the check bites with a .nuc-site diagnostic, not a generated-crate compile error. LOW priority (single observed instance) but blast radius is every identifier x every backend.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Front-end rejects (or codegen-escapes) any DSL identifier — data / kernel / loop-var / worker name — that collides with a Rust strict or reserved keyword, so it can never reach an unescaped let mut {name} / kernels::{name} / worker codegen site
- [x] #2 The collision is reported with a diagnostic anchored at the .nuc identifier source span (or, if r#-escaping is chosen, the raw-identifier-INCOMPATIBLE keywords crate/self/super/Self are still rejected with such a diagnostic) — NOT surfaced as a rustc error in generated source
- [x] #3 A negative test (e.g. a .nuc with 'data in : i32[4];') proves the check bites at the source site; existing examples/tests swept for pre-existing collisions (none remain) and full just ci green with e2e unchanged at 420/363/0/57/0
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation plan (cycle-248 impl): Strategy (a) fail-loud front-end reject — localizes fix to the two parser ident() chokepoints; (b) would touch every backend let-mut codegen AND still need to reject crate/self/super/Self. Plan: (1) add a single shared sorted-slice RUST_RESERVED const (strict+reserved keyword set) as a SEPARATE concept from grammar KEYWORDS, with a distinct codegen-collision diagnostic. (2) wire it into algo/parser.rs ident() (covers data/kernel/loop-var) and sched/parser.rs ident() (covers worker/class/region names — these reach codegen too). (3) overlap with grammar KEYWORDS (const/for and scalar types overlap with Rust const/for plus none): grammar reject is checked first and wins for those, RUST_RESERVED only fires for the rest — no double-handle, distinct messages. (4) negative tests: data in / kernel match / data crate / data self FAIL at source span; positive: in_ / match_thing still ACCEPTED (prefix guard inherited from start.chain(cont)). Determinism: array-backed slice .contains(), no HashSet iteration. Swept all 85 example .nuc + inline test snippets: ZERO pre-existing collisions (the in->src rename already cleared the only one). e2e expected UNCHANGED at 420/363/0/57/0.

LANDED (strategy a, fail-loud reject). New single-source-of-truth module nucleus-compiler/src/reserved.rs: sorted-slice RUST_RESERVED (53 Rust strict+reserved keywords) + is_rust_reserved() + collision_message(); array-backed .contains() (no HashSet — determinism invariant). Wired into BOTH ident() chokepoints: algo/parser.rs (data/kernel/loop-var) and sched/parser.rs (worker/worker_class/memory_region). Grammar KEYWORDS checked FIRST so overlap (const/for; sched also in/loop/for/async/true/false) keeps the grammar message; RUST_RESERVED fires only for the rest. Raw-incompatible four (crate/self/super/Self) ARE in RUST_RESERVED so they reject regardless. Sweep of all 85 example .nuc + inline test snippets: ZERO pre-existing collisions. Docs grammar-algo.md note-4 + grammar-sched.md note-7 updated (were lying that only grammar words are reserved). Tests: algo_parser.rs rust_keyword_identifier_rejected_at_source_site (data in / kernel match / data crate / data self, col-exact span + Rust-reserved-word message + NOT grammar message) + rust_keyword_prefix_identifier_still_accepted (in_/match_thing/crater positive) + rust_keyword_for_loop_var_is_rejected_with_preexisting_grammar_parity; sched_parser.rs rust_keyword_worker_name_rejected_at_source_site; reserved.rs 4 unit tests (sorted/unique, known collisions, near-misses, message). Gate: just ci EXIT 0; e2e 420/363/0/57/0 UNCHANGED. AC1/2/3 all met. SUBTLETY/LIMITATION (honest): for the for-loop-VARIABLE position (for VAR :), chumsky 0.9 error-merge surfaces the more-consuming downstream {-mismatch error instead of the ident() custom message, so the for-var collision is REJECTED (never admitted to AST -> never reaches codegen, AC1 holds) but the diagnostic is NOT anchored at VAR. This is PRE-EXISTING (a grammar keyword for const : behaves identically) and pinned by a parity test; diagnostic-anchoring follow-up filed TASK-0434. Orchestrator: independent review gate pending; do not treat this Done as authoritative until reviewed.

REVIEW GATE (cycle 249, orchestrator-independent parallel read-only): qa-test-runner GO + mped-architect GO.

qa NUMBERS (re-run, not transcribed): build OK; clippy clean -D warnings incl forced recompile of reserved.rs/algo+sched parser.rs (no doc_lazy_continuation on new doc edits); just test 1284/0/3 dev; just test-release 1282/0/3 (dev->release delta 2 = pre-existing TASK-0291 debug_assert should_panic); just e2e UNCHANGED 420/363/0/57/0 (front-end validation only, zero example collisions); full just ci EXIT 0 (mega-files OK: reserved.rs 199, algo/parser.rs 961, sched/parser.rs 930 — none crossed 1000; doc-citation/include-str/textual-replace all OK; all 4 negative/determinism arms bit correctly). All 8 new tests pass incl the slice-sorted-and-unique guard.

architect COMPLETENESS: PASS. Chokepoint coverage COMPLETE (silent-sibling cleared): exactly two fn ident() in the whole compiler (algo/parser.rs:449 + sched/parser.rs:209), BOTH guarded grammar-KEYWORDS-first then is_rust_reserved; verified every identifier role routes through guarded ident() (data/kernel/const/for-var/lvalue/bare-call + sched worker/class/region/place targets); the only unguarded ident_chars (algo:804) is an error-only path returning Err, never an AST ident. Reserved set conservatively complete (53 entries: all strict 2015/2018 incl dyn/async/await + reserved abstract..yield + try/gen; raw-incompatible four crate/self/super/Self present + unit-pinned); no dangerous omission. Determinism honored (sorted &[&str] + slice::contains, no HashSet). Grammar overlap coherent (grammar-first ordering verified both sites). For-var limitation HONEST + correctly scoped: for-var collision genuinely rejected (never reaches AST/codegen, AC#1 holds), unanchored-diagnostic is pre-existing+identical for grammar keywords (parity test asserts equal message), filed TASK-0434. No AC-gaming.

P1/P2: none. P3 (3, all comment/doc-lie class, FOLDED IN-THREAD commit 0924490): (P3-1) reserved.rs docstring mis-classified dyn as contextual (it is strict 2018+) and listed raw/macro_rules as included though they are NOT in the slice -> rewrote the strict-vs-contextual note accurately (verified slice membership: gen/union/try/dyn/async/await PRESENT, raw/macro_rules ABSENT). (P3-2) grammar-algo.md note-4 over-claimed universal source-span anchoring -> added the for-var carve-out caveat citing TASK-0434. (P3-3) union over-inclusion rationale clarified (sole genuine contextual over-inclusion). Fold-back gate: build+clippy+check-doc-citation-staleness+check-doc-links all green (doc-only edits, no code-logic change so test/e2e unaffected).
<!-- SECTION:NOTES:END -->
