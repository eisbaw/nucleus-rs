---
id: decision-0002
title: >-
  Project gate adopts clippy --all-targets so test/bin-target lint rot cannot
  silently re-accumulate
date: '2026-05-19 04:44'
status: accepted
---
## Context

The `clippy` justfile recipe (which `just ci` calls) ran
`cargo clippy --workspace -- -D warnings`. Without `--all-targets`,
clippy lints **only the default targets**: the `lib` and `bin`
targets. It does *not* lint integration-test crates
(`nucleus-compiler/tests/*.rs`), `#[cfg(test)]` modules compiled as test
binaries (e.g. `e2e/src/main.rs`'s test cfg), benches, or examples.

Consequence observed at the TASK-0154 review gate: lint rot
accumulated in test targets entirely invisible to `just ci`, which
stayed green the whole time. `cargo clippy --workspace --all-targets
-- -D warnings` failed with 6 pre-existing lints on clean master
(4× `clippy::len_zero` in `nucleus-compiler/tests/acfg_to_petri.rs`, 1×
`clippy::type_complexity` in `nucleus-compiler/tests/petri_to_events.rs`, 1×
`clippy::empty_line_after_doc_comments` in `e2e/src/main.rs`'s test
cfg). TASK-0186 AC#1 cleared all 6 at root cause; AC#3 is whether the
gate itself should adopt `--all-targets` so this class of rot cannot
re-accumulate undetected.

This is the same property the project already treats as first-class
in the TASK-0057/0163/0167 gate-trust lineage: **the gate must
actually catch what it claims to catch.** `determinism-check-negative`
and `xbackend-check-negative` exist precisely to prove the gate
*bites*. A clippy gate that silently exempts every test target is the
identical class of gap — it asserts "lint-clean" while a whole
category of code is unlinted.

Options weighed:

- (a) Keep `clippy` as-is (default targets only); rely on ad-hoc
  manual `--all-targets` runs.
- (b) `clippy` recipe passes `--all-targets`; `just ci` inherits it.
- (c) Add a *separate* `clippy-all` recipe, leave `clippy`/`ci`
  untouched, opt-in only.

## Decision

Adopt (b): the `clippy` recipe runs
`cargo clippy --workspace --all-targets -- -D warnings`. `just ci`
inherits it unchanged (no `ci`-recipe edit needed — `ci` already
calls `just clippy`; single source of truth, PRD §12.3).

Rationale, argued against PRD §12 and the gate-trust lineage:

- **vs (a) — status quo.** Disclosed-but-real gap. The project's own
  discipline (TASK-0163 required-cell guard, the two
  `*-check-negative` recipes) is that a gate which can be green while
  a property it claims is violated is a *broken gate*, not an
  acceptable trade-off. Test-target lint rot being invisible to
  `just ci` is exactly that. Rejected.
- **vs (c) — separate opt-in recipe.** PRD §12.3: "One justfile, kept
  deliberately short. Recipes do not bloat with one-offs." A
  `clippy-all` that nobody is gated on rots the same way the test
  targets just did — an opt-in lint that CI does not enforce is, in
  practice, unenforced. It also splits "lint" into two entry points,
  against the "three tools, each doing one thing; one entry point
  parameterised by flags" ethos (§12.3, mirrored by the `e2e`
  harness). Rejected.
- **(b) chosen** because it makes the gate honest with a one-flag
  change to the existing single lint recipe — no new recipe, no new
  entry point, no `ci`-recipe edit. It is the minimal change that
  closes the gap and keeps the justfile shape the PRD prescribes.

Cost weighed explicitly:

- The only non-default targets that exist today are **test targets**
  — there are zero `benches/` or `examples/` and zero `[[bench]]` /
  `[[example]]` manifest entries (verified). So `--all-targets`
  adds exactly the test-target lint pass, nothing speculative.
- Measured marginal cost (inside `nix develop`, warm lib artifacts):
  lib/bin clippy ≈ 5s; the `--all-targets` delta (check-compiling the
  test targets for the lint pass) ≈ **7s**. `just ci` already runs
  `just test` (≈46s, which fully compiles + runs every test target)
  and `just e2e` immediately after, so a bounded ~7s extra check pass
  is small relative to the legs already in the gate. `just test`
  builds test targets with codegen to *run* them; clippy's check
  artifacts do not fully substitute, so the ~7s is genuine but
  bounded and one-time-per-change, not a full rebuild.

## Consequences

- `justfile` `clippy` recipe is now
  `cd nucleus && cargo clippy --workspace --all-targets -- -D warnings`.
  `just ci` inherits `--all-targets` transitively (it calls
  `just clippy`); the `ci` recipe body is unchanged — single source
  of truth preserved.
- Test/bin-target lint rot is now a hard gate failure: it cannot
  silently re-accumulate behind a green `just ci` again. The
  TASK-0186 cleanup landing *before* this wiring (commit ordering) is
  the proof the wiring is honest — `just ci` is exit 0 *after*
  adoption only because the rot was actually cleared, not because the
  flag is cosmetic.
- CI compile time grows by a bounded ~7s for the extra test-target
  check pass; acceptable against the `just test` + `just e2e` legs
  already present. Revisit only if benches/examples are added and
  their lint cost becomes disproportionate (then scope per-target,
  do not regress to default-only).
- Forward guidance for the open clippy-policy tooling tasks
  (TASK-0065 / TASK-0070 / TASK-0074): the gate's clippy scope is now
  `--all-targets`; any future lint-policy work must preserve that
  scope (do not narrow back to default targets) and treat
  test-target lints as gate-blocking.
- Closes TASK-0186 AC#3.
