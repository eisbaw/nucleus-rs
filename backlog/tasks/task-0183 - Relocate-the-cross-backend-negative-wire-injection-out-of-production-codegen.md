---
id: TASK-0183
title: Relocate the cross-backend-negative wire injection out of production codegen
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 02:55'
updated_date: '2026-05-19 06:51'
labels:
  - M3
  - backend
  - tech-debt
dependencies:
  - TASK-0178
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect-style seam concern, parallel to TASK-0157 (which tracks the same for TASK-0145's NUC_NONDET_TEST). The TASK-0178 NUC_XBACKEND_NEGATIVE perturbation lives inline as maybe_corrupt_wire in mp-tcp-bufsync production lib.rs, called on the wire.rs emission critical path of every shipping build. It is deterministic (fixed source rewrite), value-gated (=='1'), loud-bannered and anchor-guarded (panics if wire_runtime drifts), so it is SAFE — but the seam is not clean: production codegen carries a self-corruption branch. Move it to a #[doc(hidden)] test hook or perform it harness-side (post-process the emitted mp-tcp tree) so production codegen has no test-only branch. Keep behaviour identical; just relocate the seam.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 mp-tcp-bufsync production codegen path contains no test-only corruption branch
- [x] #2 xbackend-check-negative still bites 100% (>=3 consecutive runs, non-flaky)
- [x] #3 Loud-banner + value-gate + anchor-drift-panic safety properties preserved
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Delete maybe_corrupt_wire from mp-tcp-bufsync/src/lib.rs entirely (fn ~1153-1183 + stale doc ~1108-1152); change call site :139 to write pristine WIRE_RUNTIME_SRC. Rewrite the now-stale doc comment so nothing lies about injection location.
2. Add maybe_corrupt_wire_for_xbackend(tree) harness-side in e2e/src/main.rs mirroring maybe_perturb_for_nondet_test: exact-1 gate, loud stderr banner, ANCHOR/CORRUPT consts, replacen on emitted src/wire.rs. mp-tcp-EXCLUSIVE: only invoked for mp-tcp-bufsync cells; missing wire.rs / anchor-drift => Err (hard, gate-visible, not silent skip).
3. Thread a corrupted: bool through CellResult; call corruption in run_cell AFTER nucleus build, BEFORE cargo build, for mp-tcp-bufsync cells only.
4. Add matrix-wide zero-corruption guard in run() under NUC_XBACKEND_NEGATIVE=1: print NUC_XBACKEND_CORRUPTED_APPLIED=<n>; if 0 applied => loud FATAL eprintln + return Ok(0) so inverting recipe fires FAIL. Keep TASK-0188 NUC_XBACKEND_CORRUPTED_DETECTED=<n> (results-derived, unchanged definition).
5. Update trace.rs comment to point at new harness location.
6. Add e2e tests mirroring TASK-0187: corruption mutates synthetic wire.rs (>=1); strict no-op when env unset; Err when gate set but wire.rs missing; extend explicit_count_signal test for the xbackend applied signal.
7. Gate: xbackend-check-negative x5 verbatim; e2e 30/26/0/4/0 + signal absent unset; determinism-check byte-identical x2; determinism-check-negative x5; test/clippy/ci; grep-prove + behaviour-equivalence (pristine wire.rs byte-identical to WIRE_RUNTIME_SRC).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0157 (its determinism-negative analogue, DONE commit e449cac): reuse the SAME clean seam. TASK-0157 deleted the inline branch from pthreads-sync codegen and moved the perturbation harness-side into nucleus/e2e/src/main.rs (maybe_perturb_for_nondet_test, called in check_cell_determinism after both builds, before the diff; perturbs ONE of the two trees so they diverge). For TASK-0183 the harness is the e2e RUN matrix (run_cell), not determinism -- but the principle holds: if the e2e harness is the sole consumer of NUC_XBACKEND_NEGATIVE, post-process the emitted mp-tcp tree harness-side (apply the maybe_corrupt_wire rewrite to wire.rs after nucleus build, before compile/run) so mp-tcp-bufsync production lib.rs carries no self-corruption branch. Keep the exact-"1" gate, loud banner, and the anchor-drift guard (fail LOUD if the wire_runtime anchor is gone -- TASK-0157 used an explicit "codegen layout drifted" Skipped message for the same never-silently-neuter-the-falsifier reason). Verify >=3 consecutive bites per AC#2.

CORRECTION to the forward-carried-from-TASK-0157 seam pattern: do NOT copy the src/main.rs-targeting perturbation verbatim — that exact choice is a partial-silent-neuter bug (TASK-0187): mp-tcp-bufsync emits src/bin/ not src/main.rs, so a src/main.rs-only perturbation silently Skips all mp-tcp cells. Any harness-side relocation here MUST perturb a file every backend emits (e.g. Cargo.toml) or be backend-layout-aware, AND hard-FAIL (not Skip) when zero perturbations happened under the negative env gate. Coordinate with / depend on TASK-0187 so both negative seams use the same provably-biting, never-silently-neuterable harness pattern.

## Forward-carried from TASK-0187 (commit 706065d) — REUSE THIS CORRECTED SEAM

TASK-0187 relocated/hardened the SIBLING NUC_NONDET_TEST perturbation harness-side in nucleus/e2e/src/main.rs. When you move maybe_corrupt_wire harness-side, mirror that exact pattern AND its corrected invariant:

1. **Perturb a layout-agnostic file.** TASK-0187 moved off src/main.rs (pthreads-only) onto Cargo.toml (every backend emits it) because the mp-tcp layout has no main.rs. For xbackend the corruption is mp-tcp-EXCLUSIVE (wire.rs), so target wire.rs in the emitted mp-tcp tree — but ASSERT it exists and treat absence as a hard fail, do NOT let a missing-file Err become a silent Skipped.

2. **THE RECIPE-INVERSION GOTCHA (critical).** justfile xbackend-check-negative, like determinism-check-negative, INVERTS the harness exit code: `if HARNESS; then echo FAIL+exit1; else echo OK`. Harness exit 0 => recipe FAIL; harness exit non-zero => recipe OK. So to make a zero-corruption run a LOUD gate FAIL, the harness must exit CLEAN (0) under the gate — exiting non-zero would let the recipe invert a no-op into a false OK. (TASK-0187 first implemented this backwards and caught it via a live demo; do not repeat.)

3. **Track a corruption-applied count** (analogue of DetCellResult.perturbed threaded through every constructor) and add the zero-corruption guard: under NUC_XBACKEND_NEGATIVE=1, if zero cells were actually corrupted -> loud FATAL + return Ok(0) so the recipe fires its FAIL branch. A falsifier must PROVABLY bite (gate-trust lineage TASK-0145/0157/0163/0167/0178/0187).

4. **Add a unit/integration test** asserting the corruption mutated >=1 tree AND a test modelling the recipe inversion (see TASK-0187 tests in nucleus/e2e/src/main.rs: zero_perturbation_guard_makes_negative_recipe_fail).

5. Env-gate not cfg!/feature (nested cargo --features does not reliably rebuild against the shared target cache — still holds). Keep loud banner + exact-"1" value gate + anchor-drift detection.

Forward-carried from TASK-0187 review gate: TASK-0188 will add an explicit machine-checkable corrupted-cell-count assertion to xbackend-check-negative (justfile:85) so its safety invariant does not rest solely on exit-code inversion. When implementing the harness-side relocation here, coordinate with / depend on TASK-0188 so the xbackend negative seam uses the explicit-signal pattern, not just the inverting recipe.

## Forward-carried from TASK-0188 (commit 6c703c1) — INHERIT THE EXPLICIT-SIGNAL CONTRACT (supersedes prior carries on the recipe seam)

TASK-0188 hardened BOTH negative gates so the "falsifier actually touched something" safety invariant no longer rests SOLELY on the exit-code inversion. When you relocate maybe_corrupt_wire harness-side, you MUST preserve the post-0188 contract, not just the pre-0188 zero-corruption guard:

1. **The explicit machine-checkable stdout line is now part of the recipe contract.** The e2e run() path prints `NUC_XBACKEND_CORRUPTED_DETECTED=<n>` on STDOUT, ONLY when NUC_XBACKEND_NEGATIVE=1, where n = required mp-tcp-bufsync cells Failed at Phase::Diff (corruption present AND differential detected it — NOT any unrelated required-fail). justfile:85 captures combined harness output to a temp file (cargo exit status still drives the `if`), then asserts the line is present AND n>=1 IN ADDITION to the exit-code inversion. Your relocation MUST keep emitting this exact line with the same key and the same precise definition; do not regress to exit-code-only. If you change WHERE corruption is applied (harness-side post-process), the detection count is still "required mp-tcp-bufsync cell diverged from reference.bin at Diff" — recompute it from results, keep the conjuncts.

2. **Stream + capture mechanism (reuse verbatim).** Signal on stdout via println! (semantically a RESULT line; loud diagnostics stay on stderr). Recipe pattern: `out=$(mktemp); trap rm; { if NUC_XBACKEND_NEGATIVE=1 cargo run ... >"$out" 2>&1; then bit=0; else bit=1; fi; }; cat "$out"; n=$(grep -oE ... | cut -d= -f2); [ -z "$n" ]||[ "$n" -lt 1 ] => loud FAIL exit 1; else exit-code inversion.` The `>"$out" 2>&1` keeps cargo's status (not tee/grep's) driving the `if`. Absent-signal AND zero-count are BOTH loud FAILs independent of exit code.

3. **Keep gating strict.** The line must NOT appear under bare `just e2e` (verified post-0188: e2e standalone stays exactly total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0, zero signal lines). A harness-side relocation must keep this no-op-when-unset property.

4. **Prior carries (zero-corruption guard, recipe-inversion gotcha, layout-agnostic perturb) still apply** — TASK-0188 ADDS the explicit-signal backstop on top; it does not replace them. Net: a future recipe refactor dropping the inversion fails LOUD via the count assertion instead of silently re-neutering the falsifier. Model both via the e2e test `explicit_count_signal_makes_negative_recipes_fail_loud_independent_of_exit_code` (extend it if you move the seam).

## TASK-0183 implementation evidence (commit c48b7d3)

### AC#1 — production codegen corruption-branch-free
- Deleted maybe_corrupt_wire fn + its 45-line stale doc from mp-tcp-bufsync/src/lib.rs; call site :139 now writes pristine mp_tcp_common::WIRE_RUNTIME_SRC.
- grep-proof: only matches in mp-tcp-bufsync/src are COMMENTS (relocation notes, lines 142-147/1118-1133) + unrelated emitted NUC_TCP_PORT_* runtime strings (lines 500/532). No maybe_corrupt_wire, no wrapping_add, no executed std::env::var("NUC_XBACKEND_NEGATIVE").
- Behaviour-equivalence PROVEN: a normal `nucleus build` for 02-split-add/split mp-tcp emits src/wire.rs byte-identical to wire_runtime.rs (WIRE_RUNTIME_SRC is include_str! of it): 11925 bytes, equal=True, corruption present=False.

### AC#2 — xbackend-check-negative bites 100% (>=5 consecutive, non-flaky)
Ran 5/5 consecutive, all identical:
  RUN 1: APPLIED=13 DETECTED=1 -> "OK: cross-backend differential correctly bit on injected mp-tcp corruption" exit 0
  RUN 2: APPLIED=13 DETECTED=1 -> OK exit 0
  RUN 3: APPLIED=13 DETECTED=1 -> OK exit 0
  RUN 4: APPLIED=13 DETECTED=1 -> OK exit 0
  RUN 5: APPLIED=13 DETECTED=1 -> OK exit 0
DETECTED=1 == the pre-relocation TASK-0188 baseline (only 02-split-add/split ships array data over the wire; scalar protocols unaffected by the enc_vec last-byte tweak) — IDENTICAL behaviour.

### AC#3 — safety preserved (harness-side)
- Exact-"1" gate, loud stderr WARNING banner, anchor-drift hard-failure all moved verbatim to maybe_corrupt_wire_for_xbackend. panic! -> typed Err (caller maps to Failed(Compile); zero-corruption guard forces CLEAN exit so the inverting recipe FAILs loud — never a silent neuter). Env-gate not cfg!/feature (unchanged reasoning).
- 4 new e2e tests green: xbackend_corrupt_rewrites_wire_rs_under_gate / _is_strict_noop_when_env_unset / _errs_when_gate_set_but_wire_rs_missing / _errs_when_anchor_drifted. Extended explicit_count_signal_makes_negative_recipes_fail_loud_independent_of_exit_code for the xbackend APPLIED/DETECTED contract.

### Full gate (all green, inside nix develop)
- just e2e standalone: total 30 / pass 26 / fail 0 / skipped 4 / required-fail 0; NUC_XBACKEND_CORRUPTED grep -c = 0 (signal absent when gate unset — strict no-op).
- just determinism-check: 30/26/0/4 byte-identical x2, exit 0 (pristine wire.rs preserves determinism).
- just determinism-check-negative: 5/5 NUC_NONDET_PERTURBED_CELLS=26 + OK (sibling seam unaffected).
- cargo test -p e2e: 34 passed 0 failed. just test workspace: 388 passed 0 failed.
- clippy --workspace --all-targets -- -D warnings: clean. just ci: exit 0.

### Gotchas / feed-forward (subagents are stateless)
- DETECTION signal kept correct with corruption now harness-side: NUC_XBACKEND_CORRUPTED_DETECTED is still recomputed from results (required && mp-tcp-bufsync && Failed{Diff}), conjuncts intact — moving WHERE corruption is applied does not change "a required mp-tcp cell diverged from reference.bin at Diff".
- wire.rs is mp-tcp-EXCLUSIVE: maybe_corrupt_wire_for_xbackend is invoked ONLY for cell.backend=="mp-tcp-bufsync"; a pthreads cell legitimately has no wire.rs and is never touched / never Errs.
- Zero-corruption guard DIRECTION: under the gate, APPLIED==0 => loud FATAL eprintln + return Ok(0) (CLEAN), because xbackend-check-negative INVERTS exit code; exiting non-zero would invert a no-op into a false OK (the TASK-0187 backwards-first lesson, not repeated).
- Anchor-drift now harness-side: panic! became a typed Err -> Failed(Compile) -> zero-corruption guard -> recipe FAILs loud. Gate-visible, never silent.
- justfile xbackend-check-negative UNCHANGED (its TASK-0188 capture+dual-assert already consumes the stdout DETECTED line; APPLIED added as an extra backstop, no recipe edit needed).
- The "no test-fault-injection in production codegen" production-readiness theme (TASK-0157/0187/0188/0183) is now COMPLETE: both negative seams (NUC_NONDET_TEST, NUC_XBACKEND_NEGATIVE) are harness-side, provably-biting, explicit-signal-backed, never-silently-neuterable. No new follow-up/forward-carry needed — the relocation surfaced no gap.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, no blocking findings, no follow-up needed. qa-test-runner: xbackend-check-negative 5/5 (APPLIED=13 DETECTED=1 + verbatim OK); e2e 30/26/0/4/0 + both signals absent unset; determinism byte-identical x2 + determinism-negative 5/5 unaffected; emitted wire.rs BYTE-IDENTICAL to pristine wire_runtime.rs (SHA256 e4b2c972..., 11925 bytes) on a normal build; codegen grep-proven branch-free; cargo test workspace 388/0 (e2e 34, +4 new); clippy --all-targets clean; ci exit 0; corrupted-flag threading trustworthy (single let mut + shorthand, no misset constructor). mped-architect: AC#1 branch-free+behaviour-equivalent by code-path proof; TASK-0188 DETECTED contract preserved EXACTLY (key+conjuncts+gate-only+stdout) + new APPLIED backstop sound; zero-corruption guard correct CLEAN-Ok(0) direction; pthreads correctly never Err for lacking wire.rs (no false gate failure); panic->Err anchor-drift downgrade LOST NO safety (more gate-visible); comments honest; Done honest with independently-reproduced 5/5. BOTH independently verified the no-test-injection-in-production-codegen theme (TASK-0157/0187/0188/0183) is genuinely COMPLETE: both negative seams harness-side, provably-biting, explicit-signal-backed, never-silently-neuterable. TASK-0183 Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Relocated the NUC_XBACKEND_NEGATIVE cross-backend wire-corruption falsifier entirely out of mp-tcp-bufsync production codegen and harness-side into nucleus-e2e, behaviour-identical, preserving the TASK-0188 explicit-signal contract. Final of the test-injection-relocation thread (TASK-0157/0187/0188/0183) — the "no test-fault-injection in production codegen" production-readiness theme is now complete: both negative seams are harness-side, provably-biting, explicit-signal-backed, never-silently-neuterable.

Changes:
- mp-tcp-bufsync/src/lib.rs: deleted maybe_corrupt_wire + its 45-line stale doc; wire.rs now emitted byte-identical to mp_tcp_common::WIRE_RUNTIME_SRC; zero env read on any codegen path. Stale doc replaced with an accurate relocation note (comment-honesty).
- e2e/src/main.rs: added maybe_corrupt_wire_for_xbackend (sibling of maybe_perturb_for_nondet_test) — exact-"1" gate, loud banner, ANCHOR/CORRUPT replacen, anchor-drift/missing-wire => typed Err. Called in run_cell post-nucleus-build / pre-cargo-build for mp-tcp-bufsync cells ONLY (wire.rs is mp-tcp-exclusive). Threaded CellResult.corrupted through all 19 constructors. Added matrix-wide zero-corruption guard + NUC_XBACKEND_CORRUPTED_APPLIED stdout signal; preserved NUC_XBACKEND_CORRUPTED_DETECTED (TASK-0188) verbatim, recomputed from results. 4 new tests + extended the explicit-count-signal test.
- compiler/src/trace.rs: updated the env-var inventory comment to the new harness location.
- justfile UNCHANGED (its dual-assert already consumes the DETECTED stdout line).

User impact: production codegen carries no test-only branch; normal builds emit pristine wire.rs (proven byte-identical, determinism preserved). Falsifier behaviour unchanged: xbackend-check-negative still bites 100%.

Tests/gate (all green, nix develop): xbackend-check-negative 5/5 APPLIED=13 DETECTED=1 + verbatim OK; e2e standalone 30/26/0/4/0 with signal absent (grep -c 0); determinism-check 30/26/0/4 byte-identical x2; determinism-check-negative 5/5 NUC_NONDET_PERTURBED_CELLS=26 + OK; cargo test -p e2e 34/0; just test 388/0; clippy --workspace --all-targets -D warnings clean; just ci exit 0; grep-proof + behaviour-equivalence (11925-byte pristine wire.rs).

Risks/follow-ups: none — the relocation surfaced no gap; no new forward-carry. Commit c48b7d3 (code only; task .md CLI-managed/unstaged).
<!-- SECTION:FINAL_SUMMARY:END -->
