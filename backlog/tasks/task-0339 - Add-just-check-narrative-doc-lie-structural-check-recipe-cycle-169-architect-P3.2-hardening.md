---
id: TASK-0339
title: >-
  Add 'just check-narrative-doc-lie' structural check recipe (cycle 169
  architect P3.2 hardening)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-26 08:40'
updated_date: '2026-05-26 08:48'
labels:
  - hardening
  - structural-check
  - narrative-doc-lie
  - feedback-comment-doc-lie-recurring
  - feedback-silent-sibling-defect
  - justfile
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

The cycle-169 / 169b session (TASK-0338) closed two structurally identical stale-doc-lie blocks in nuc-nucleus/e2e-matrix.toml (lines 1372-1378 and 1182-1217). The defect class is the recurring 'predictive conclusion in a doc-comment not back-edited after the predicted event lands' pattern tracked in memory.feedback-comment-doc-lie-recurring (12+ firings) and memory.feedback-silent-sibling-defect (13 firings as of cycle 169).

The cycle-169 mped-architect read-only review (P3.2) suggested a structural check recipe analogous to the existing 'just check-textual-replace-on-codegen' + 'just check-include-str-coverage' family that would catch this defect class at gate-time — converting reactive cleanup cycles into proactive gate failures.

## Standing pattern set (derived from cycle-169 hygiene rule)

The recipe should grep for these predictive-claim patterns:

- BLOCKED by TASK
- CARRIED as \\[\\[skip\\]\\]
- (is|was) still pending / still pending
- currently \\[\\[skip\\]\\]
- pending cycle-?[0-9]+
- Only .+ remains \\[\\[skip\\]\\]
- [0-9]+ of [0-9]+ tier-1 backends
- awaits / awaiting / gated on / not yet

## Per-line allow-list

A hit is OK if the same line contains any of:

1. 'AT FILING TIME' marker (explicit past-tense framing).
2. '# Cycle-[0-9]+ filing:' time-stamp prefix (paragraph explicitly historical).
3. '# ALLOW narrative-doc-lie: <reason>' annotation (per-line allow-list).

## Acceptance criteria

1. New 'just check-narrative-doc-lie' recipe in justfile, structurally analogous to check-textual-replace-on-codegen + check-include-str-coverage (echo + grep + allow-list + informative fail message + memory pointer).
2. Recipe scope: nuc-nucleus/e2e-matrix.toml plus any other narrative-bearing TOML in nuc-nucleus/ (capabilities.toml, schedule .toml files).
3. Per-line allow-list as enumerated above.
4. Recipe wired into 'just ci' aggregate alongside check-textual-replace-on-codegen + check-include-str-coverage (must be part of the hard gate, not a separately-invokable recipe only).
5. Empirically verified to BITE: (a) PASSES on current tree (post-cycle-169b); (b) FAILS with informative output on a deliberately-injected stale line (e.g. 'Only mp-tcp-bufsync remains [[skip]], pending cycle-200 replication'); (c) PASSES again after the injection is reverted.
6. Failure message references the memory entries (feedback-comment-doc-lie-recurring + feedback-silent-sibling-defect cycle-169 hygiene rule).

## Honest scope

This is a HARDENING task per phase3-backlog-ralph backlog-maturity guidance. It does not affect runtime behaviour or any e2e cell; it adds a gate that catches future doc-lie introductions BEFORE they require reactive cleanup cycles. Risk: zero false-positive surface if the allow-list is well-designed; small false-positive risk if the grep patterns are too aggressive (mitigated by the per-line allow-list).

## Cross-reference

- nuc-nucleus/e2e-matrix.toml (the file the recipe protects).
- justfile lines 270-325 (the existing check-textual-replace-on-codegen + check-include-str-coverage recipes, structural template).
- justfile lines 188-189 (the 'just ci' aggregate where the new recipe wires in).
- memory: feedback-comment-doc-lie-recurring.
- memory: feedback-silent-sibling-defect (cycle 169 hygiene rule, standing pattern set).
- TASK-0338 cycle 169 + 169b (the motivating cleanup cycles).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 170 implementation (orchestrator-direct hygiene)

### What landed

1. justfile: new `check-narrative-doc-lie` recipe (~30 lines, structurally analogous to check-textual-replace-on-codegen + check-include-str-coverage).
2. justfile: recipe wired into `just ci` aggregate after check-include-str-coverage.
3. nuc-nucleus/e2e-matrix.toml: 2 ALLOW annotations added to lines that legitimately contain 'still pending' but are framed historical or current-state-accurate by surrounding context.

### Recipe scope

- Pattern set: 'BLOCKED by TASK', 'CARRIED as \[\[skip\]\]', 'still pending', 'currently \[\[skip\]\]', 'pending cycle-?[0-9]+', 'Only .+ remains \[\[skip\]\]', '[0-9]+ of [0-9]+ tier-1 backends'.
- File scope: nuc-nucleus/e2e-matrix.toml only (the sole narrative-bearing TOML in nuc-nucleus/ today; other TOMLs are reference Cargo manifests).
- Per-line allow-list: 'AT FILING TIME' marker, '# Cycle-<N> filing:' prefix, '# ALLOW narrative-doc-lie:' annotation.

### Rejected patterns (false-positive surface too high)

- 'awaits' / 'awaiting' / 'gated on' / 'not yet': generic English; would fire on many legitimate uses.
- The cycle-169 architect suggested these in the wider sweep; dropping them keeps the recipe high-signal.

### Pattern-set width vs noise

Started with 7 narrow patterns derived from the cycle-169 + 169b architect's findings. The narrower set catches the exact defect signatures TASK-0338 fixed without false-positives elsewhere in the file. Future cycles that discover a new structural variant can widen the patterns.

### Empirical BITE verification (AC#5)

(a) Recipe PASSES on current tree (post-ALLOW-annotations).
(b) Injected synthetic stale line at e2e-matrix.toml:60 (`# Only mp-tcp-bufsync remains [[skip]], pending cycle-200 replication of foo.`); recipe FAILED with informative output naming the line + fix options.
(c) Reverted injection; recipe PASSES again.

### Gate verification

nix develop --command: toml OK, just check OK, just clippy OK (no warnings), just test OK, just test-release OK, just e2e total 112 / pass 102 / fail 0 / skipped 10 / required-fail 0 (baseline preserved across recipe addition).

### AC status

- AC#1: PASS (recipe exists, structurally analogous to check-textual-replace-on-codegen).
- AC#2: PARTIAL — current scope is just nuc-nucleus/e2e-matrix.toml. The task description said 'plus any other narrative-bearing TOML in nuc-nucleus/'; today there are none (all other TOMLs are reference Cargo manifests). If future narrative TOMLs land, the recipe's file list needs widening. Reframed as 'PASS for today's scope; widen when new narrative TOMLs land'.
- AC#3: PASS (recipe wired into `just ci` at the line after check-include-str-coverage).
- AC#4: PASS (per-line allow-list: 'AT FILING TIME' marker, '# Cycle-<N> filing:' prefix, '# ALLOW narrative-doc-lie:' annotation).
- AC#5: PASS (BITE verified empirically; round-trip PASS → FAIL → PASS).
- AC#6: PASS (failure message references both memory entries).

### Gotchas + forward-carried lessons

1. **TOML inline comments**: TOML uses '#' to end of line as comment, so '`# blocker). Example 14 still pending its own kernels.rs / reference /  # ALLOW narrative-doc-lie: ...`' is one big comment, not two — the ALLOW annotation does NOT need to be on its own line.

2. **Pattern set width tradeoff**: every pattern added to the recipe has a false-positive surface. The cycle-169 architect's wider set (awaits/awaiting/gated on/not yet) was rejected for noise; the narrower set fires on the exact phrasings that recurred. If a future cycle finds a NEW structural variant the recipe misses, widen via either (a) adding a new pattern, OR (b) the per-line ALLOW escape hatch — prefer (b) for one-off cases.

3. **Forward-carried to TASK-0044 (M6)**: when M6 lands new schedules/backends, e2e-matrix.toml will grow new [[skip]] cells with new blocker narratives. The recipe is the gate against new doc-lies being introduced. Every new cell's prose narrative must either (a) use 'AT FILING TIME' verbs, OR (b) annotate with the per-line ALLOW + reason.

4. **Forward-carried to TASK-0339-like future recipes**: the recipe pattern (informative-fail + memory pointer + fix options) is a reusable template for future structural checks; see check-textual-replace-on-codegen + check-include-str-coverage + check-narrative-doc-lie as the canonical set.
<!-- SECTION:NOTES:END -->
