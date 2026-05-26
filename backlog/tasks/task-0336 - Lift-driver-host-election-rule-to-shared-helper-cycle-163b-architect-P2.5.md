---
id: TASK-0336
title: Lift driver host-election rule to shared helper (cycle-163b architect P2.5)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-26 04:56'
updated_date: '2026-05-26 06:12'
labels: []
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

cycle-163b architect P2.5 fold-back finding: driver `nucleus/driver/src/main.rs` independently mirrors `Plan::build`'s host-election rule in THREE conditional wirings:

- cycle-160 `apply_host_mediation_inject` (CTRL-arm host mediation).
- cycle-162 `apply_safe_push_reorder` (slice 1 / Option D event-list-layer reorder).
- cycle-163 `apply_host_data_relay_inject` (slice 2 / Option B2 ACFG-layer routing).

Each site independently picks: 'worker literally named "host" filtered by used_workers, else used_workers.iter().next()'. The rule is currently respected at all 3 sites (cycle-163 QA verification GREEN), but the mirroring surface is exactly the `feedback-driver-must-mirror-backend-election-exactly` recurrence — adding a 4th wiring (e.g., a slice-3 threaded relay) or refactoring `Plan::build`'s rule risks drift.

## Acceptance criteria

### AC#1: shared helper

Lift the host-election rule to a single helper, e.g. `pub fn elect_host(used: &BTreeSet<WorkerId>, name_workers: &BTreeMap<String, WorkerId>) -> WorkerId` in `backend-common` (or wherever `Plan::build` itself can consume it). All 3 driver wirings + `Plan::build` call the helper instead of inlining the rule.

### AC#2: regression pin

Negative test: if the helper is removed and the rule re-inlined in 2 of the 4 sites with a divergence, the test catches the drift. Likely: a parameterised test exercising 'named host', 'first-used fallback', 'tied-name resolution' across both sites.

### AC#3: no behavioral change

`just e2e` baseline preserved (no cell regresses or promotes); host-election outcome for every existing cell is byte-identical pre/post-refactor.

## Cross-reference

- TASK-0329.01.02 cycle 163b architect P2.5 finding (parent fold-back commit).
- Memory `feedback-driver-must-mirror-backend-election-exactly` — load-bearing for this task's existence.
- `nucleus/driver/src/main.rs` — 3 wiring sites (search for `apply_host_mediation_inject`, `apply_safe_push_reorder`, `apply_host_data_relay_inject`).
- `nucleus/backends/mp-tcp-event/src/multi_worker.rs` `Plan::build` (lines around 153-160) — the source-of-truth rule that driver mirrors.

## Honest scope

LOW priority — currently zero defects (3-of-3 sites correct as of cycle 163b). This is hardening / fragility-reduction. Promote priority on first drift instance.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 164 implementer plan (2026-05-26)

**Helper design (two thin public wrappers + one private core, zero allocation):**

```rust
// nucleus/backend-common/src/host_election.rs (new module)
pub const HOST_NAME: &str = "host";

fn elect_host_core(
    named_host_in_used: Option<WorkerId>,
    smallest_used: Option<WorkerId>,
) -> Option<WorkerId> {
    named_host_in_used.or(smallest_used)
}

/// Backend view (NameTables.worker: WorkerId -> name).
pub fn elect_host_from_worker_names(
    worker_names: &BTreeMap<WorkerId, String>,
    used_sorted_asc: &[WorkerId],
) -> Option<WorkerId>;

/// Driver view (ACFG.name_workers: name -> WorkerId).
pub fn elect_host_from_name_workers(
    name_workers: &BTreeMap<String, WorkerId>,
    used_sorted_asc: &BTreeSet<WorkerId>,
) -> Option<WorkerId>;
```

**Rationale for two helpers over one generic:**
- Backends always have NameTables.worker; drivers always have ACFG.name_workers. The view is fixed at compile time, so a closure-based generic just adds callsite boilerplate without payoff.
- Two named helpers with view-specific signatures read more clearly at the callsite (one or two lines).
- Both delegate to elect_host_core where the rule `named_in_used.or(smallest_used)` lives ONCE. Doc lives ONCE on the core.

**File:** nucleus/backend-common/src/host_election.rs (new module). Hooked into lib.rs alongside check_frame / multi_worker_walker / project_skeleton / render. Backend-common is the right home per memory project-backend-common-crate.

**Conversion order:**
1. Helper + tests in backend-common (5 test cases per AC#2).
2. mp-tcp-event multi_worker.rs (single backend first — catches signature issues).
3. Remaining 3 backends (pthreads-sync, pthreads-async, mp-tcp-bufsync).
4. 3 driver sites (host_mediation_inject, host_data_relay_inject, safe_push_reorder).
5. Verification gate.

**Verified rule-equivalence at 7 sites (pre-conversion grep):**
- All 4 backends: `names.worker.iter().find(|(_, n)| n.as_str() == "host").map(|(w, _)| *w).filter(|w| used_workers.contains(w))` then `.or_else(|| used_workers.first().copied())`. Identical.
- All 3 driver sites: `acfg.name_workers.iter().find(|(n, _)| n.as_str() == "host").map(|(_, w)| *w).filter(|w| used.contains(w))` then `.or_else(|| used.iter().next().copied())`. Identical.
- Both forms produce identical results (BTreeMap.iter().find(key=k) is equivalent to BTreeMap.get(k); both `Vec<WorkerId>` and `BTreeSet<WorkerId>` for used are sorted-ascending so `.first()` == `.iter().next()`).

**Honest scope:**
- Pure refactor. AC#3 contract: byte-identical e2e output (every cell), test counts unchanged on production but +5 in backend-common.
- HOST_NAME constant lifted as a side effect (single source of truth for the literal too).
- test-common:329 (`r.names.worker.values().any(|n| n == "host")`) is a fixture-presence assertion, NOT an election; not converted.

## Cycle 164 results (2026-05-26)

**All 3 ACs met.**

**AC#1 - shared helper exists, all 7 sites converted:** `backend_common::host_election::{elect_host_from_worker_names, elect_host_from_name_workers, HOST_NAME}` (new module `nucleus/backend-common/src/host_election.rs`). Both wrappers delegate to a private `elect_host_core(named_host_in_used, smallest_used)` where the rule `named_in_used.or(smallest_used)` lives ONCE. Grep proof:
- `grep -rn 'n.as_str() == "host"' nucleus/backends/ nucleus/driver/src/main.rs` -> 0 production hits.
- `grep -rn 'used_workers.first().copied()\|used.iter().next().copied()' nucleus/backends/ nucleus/driver/src/main.rs` -> 0 production hits.
- HOST_NAME literal is centralised in the helper module; backends/driver no longer compare against `"host"` directly.

**AC#2 - regression test:** 11 test cases in `nucleus/backend-common/src/host_election.rs#tests` mod:
- Backend view (5): named-in-used wins / named-not-in-used falls back to smallest / no named -> smallest / empty used -> None / smallest wins on tie.
- Driver view (5): same 5 branches.
- Cross-view symmetry (1): `both_views_elect_the_same_host_for_the_same_input` proves the two public wrappers agree across all 4 branch shapes; defends directly against the recurrence this refactor retires (`feedback-driver-must-mirror-backend-election-exactly` - divergent re-inlining at one site only).

**AC#3 - byte-identity:** confirmed empirically by side-by-side emit + diff against parent commit `86886b5`:
- 05-stencil/distributed × mp-tcp-event: `diff -r parent head` -> empty.
- 09-producer-consumer/pipelined × mp-tcp-event (cycle-163 keystone, exercises BOTH `host_data_relay_inject` AND `safe_push_reorder` driver wirings): `diff -r parent head` -> empty.
- 02-split-add/split × pthreads-sync: `diff -r parent head` -> empty.

**Gates (`nix develop --command bash -c "just X"`):**
- `just build`: clean.
- `just clippy`: clean (`-D warnings`).
- `just test` (dev): 960 / 0 / 3 (= 949 baseline + 11 new helper tests).
- `just test-release`: 960 / 0 / 3 (dev == release; cycle-112 `feedback-qa-gate-misses-release-profile` precaution honoured).
- `just e2e`: 112 / 101 / 0 / 11 / 0 across 3 non-flake samples (4 total runs all matched baseline).

**Lessons (forward-carryable):**
1. **Two thin view-specific wrappers + one private core beats a single closure-based generic** when the views are heterogeneous BTreeMaps and the rule is small (a single `.or()`). The two-wrapper shape reads cleaner at the callsite, the core docstring is the canonical single source of truth for the rule, and the cross-view symmetry test is mechanically writeable. A closure-based generic would have added boilerplate at 7 sites without payoff.
2. **`cargo run --release --bin nucleus-e2e` returned in 0.02s and DID NOT rebuild** even after edits — memory `feedback-stale-release-binary-during-session` fires again. Workaround used here: `touch backend-common/src/host_election.rs && cargo build --release --workspace` before trusting the binary. Generic enough to add to that memory note: `cargo run --release` SOMETIMES misses the freshness check the way `cargo build --release` does too (per the existing memory). The robust workaround is `touch + cargo build --release --workspace` not just relying on cargo's mtime check.
3. **The e2e harness deletes its `target/e2e-matrix/run-*` dir on success** (`finalize_run_scratch(success=true)` at `nucleus/e2e/src/main.rs:831`). Means a successful e2e run leaves NO inspectable post-emit on disk — bit-identity comparisons must be done via direct `nucleus build --out DIR` invocations, not by spelunking the matrix dir. Useful project gotcha for future bit-identity verifications.
4. **Driver did not previously depend on `backend-common` directly** (only transitively via backends). Added `backend-common = { path = "../backend-common" }` to `nucleus/driver/Cargo.toml`. No circular risk: driver -> backend-common is a normal arrow (backend-common has no driver knowledge); pre-existing backends -> backend-common arrows are independent of this new one.
5. **Empirical sweep confirmed the rule was identical at all 7 sites** (no hidden `eq_ignore_ascii_case` or alternative tie-breaker found). The cycle-163b architect's P2.5 audit was structurally correct; no honest-failure surprise. `nucleus/test-common/src/lib.rs:329` `r.names.worker.values().any(|n| n == "host")` is a fixture-presence assertion, NOT an election site, so was not converted.

**Memory `feedback-driver-must-mirror-backend-election-exactly` status:** the recurrence surface on the canonical path (driver wirings vs backend Plan::build host election) is now structurally retired. A 4th driver wiring or a future refactor of Plan::build's rule that uses the helper cannot drift; a hand-rolled re-inlining (the original failure mode) would be caught by code review for not consuming `backend_common::elect_host_from_*` per cross-pass convention. Promote priority on first re-firing (none expected on the canonical path).

## Cycle 164b — parallel review fold-back

Parallel review gate (qa-test-runner + mped-architect, read-only) both returned GO. Architect P2.1 + P2.3 + P3.1 folded in this cycle; P2.2 acknowledged as re-discovery (no memory change).

### P2.1 — stale driver prose blocks duplicated the rule

Architect noted lines 481-502 + 545-548 of `nucleus/driver/src/main.rs` still inline the rule + cited stale file:line ranges (`mp-tcp-bufsync/src/lib.rs:331-338` and `mp-tcp-event/src/multi_worker.rs:153-160`). After cycle 164 those ranges contain helper calls, not the rule body — the prose duplicated what the helper docstring is now the canonical source for. `feedback-comment-doc-lie-recurring` + `feedback-opacity-gate-rot` on docs.

Cycle-164b shrinks the first block to keep WHY (degenerate-mediation risk → backend defensive rejection re-fires against the BACKEND-elected host) + drops the rule restatement + drops the stale file:line cite. Second block (545-548) shrinks to a one-liner pointing at the cycle-160 wiring above.

### P2.3 — e2e harness scratch lifecycle worth a memory entry

Architect verified at `nucleus/e2e/src/main.rs:831-851`: `finalize_run_scratch` deletes `target/e2e-matrix/run-*/` on success; retains + stderr-prints on failure. The implementer's cycle-164 L2 disclosure captured the success-case cleanup but not the failure-case retention. Filed as new memory `project-e2e-harness-scratch-lifecycle` covering both halves + the canonical bit-identity-verification workflow (direct `nucleus build --out DIR` invocations, not post-`just e2e` diff).

### P3.1 — sort invariant unenforced

The helper's docstring at line 49-60 declared `used_sorted_asc` MUST be ascending; the implementation trusted it. All current callers (4 backends, 3 driver wirings) build the slice from `BTreeMap::keys()` so sort-by-construction; but a future caller passing an unsorted Vec would silently elect the FIRST element instead of the smallest.

Cycle-164b adds `debug_assert!(used_sorted_asc.windows(2).all(|w| w[0] < w[1]))` to `elect_host_from_worker_names` + a `#[should_panic(expected = "must be strictly ascending")]` `#[cfg(debug_assertions)]` negative pin in the tests mod. Zero release cost, catches future caller drift in dev/test.

### P2.2 — implementer L1 lesson was a RE-DISCOVERY (no memory change)

Architect noted the cycle-164 L1 lesson ('cargo run --release ships stale binary') is already covered by the existing memory `feedback-stale-release-binary-during-session` — the implementer's framing inverted which case the memory documents (the memory already explicitly mentions `cargo run --release --bin nucleus-e2e`). Live instance of `feedback-implementer-disclosure-mechanism-wrong`.

**Disposition**: no memory file update needed. This addendum acknowledges the re-discovery so a fresh subagent reviewing the cycle history sees the disposition. Re-validates the orchestrator-hygiene rule: read the existing memory before forward-carrying an 'L1-class' disclosure.

### Cycle-164b gate results (3-sample non-flake)

- `just build`: clean
- `just clippy` (`-D warnings`): clean
- `just test` (dev): **961 / 0 / 3** (was 960 at cycle 164; +1 sort-assert negative pin)
- `just test-release`: **960 / 0 / 3** (intentional asymmetry: new pin is `#[cfg(debug_assertions)]` so it's compiled out of release; release count unchanged from cycle 164)
- `just e2e`: **112 / 101 / 0 / 11 / 0** (3 non-flake samples; bit-identity preserved)

### What this fold-back commit changes

- `nucleus/driver/src/main.rs`: trimmed 2 inline-rule comment blocks (lines 480-498 + 545-548) to remove rule restatement + stale file:line cites; preserved WHY of host-election in each pass.
- `nucleus/backend-common/src/host_election.rs`: added `debug_assert!` sort-invariant guard in `elect_host_from_worker_names` + 1 negative pin test.
- New memory: `project-e2e-harness-scratch-lifecycle.md` + MEMORY.md index entry.
- This tracker note.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 164 close: TASK-0336 host-election helper lifted to `backend_common::host_election`; all 7 production sites (4 backend Plan::build + 3 driver wirings) converted; 11 helper unit tests + cross-view symmetry test; AC#1/2/3 all met. Gates: 960/0/3 dev = release; 112/101/0/11/0 e2e across 3 non-flake samples. Byte-identity confirmed on 3 cells across 2 backends vs parent commit 86886b5. Memory feedback-driver-must-mirror-backend-election-exactly recurrence retired on the canonical path.
<!-- SECTION:FINAL_SUMMARY:END -->
