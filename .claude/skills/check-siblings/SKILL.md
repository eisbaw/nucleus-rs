---
name: check-siblings
description: Audit a symbol for silent-sibling defects. Given a symbol or pattern, lists every call site, marks which were touched in the current diff, and flags untouched sites that look structurally identical. Use after fixing a defect at one call site, before claiming closure. Defends against the `feedback-silent-sibling-defect` recurrence.
allowed-tools: Bash, Read, Grep
user-invocable: true
---

# check-siblings

The Nucleus codebase has a documented recurring defect class: a fix
lands at one visible call site while a structurally identical sibling
silently skips it. The memory file
`feedback-silent-sibling-defect.md` lists concrete prior instances
(cycles 93/95/97/98).

This skill is a structured grep-audit for that class.

## Inputs

The user provides either:

- a **symbol name** (function, type, field — `partition_pairs`,
  `inject_transfers`, `XferPlaceholder`) — the skill finds every
  reference and reports;
- a **pattern** in quotes (`"data_producers.get"`,
  `"halo_widths.entry"`) — the skill greps it literally.

If unclear, ask the user once for the exact symbol/pattern.

## What this skill does

1. Run `rg -nH '<symbol>' nucleus/` (or the explicit pattern). Sort
   hits by file.
2. Run `git diff --name-only` AND `git log --name-only -5` to learn
   which files the current cycle has touched.
3. Cross-reference: tag each hit as `[touched-in-diff]`,
   `[touched-recent-commits]`, or `[untouched]`.
4. For each `[untouched]` hit, read 5-10 surrounding lines and report
   the local context to the user — so they can judge whether the
   sibling needs the same fix.
5. If the symbol/pattern is `BTreeMap` / `BTreeSet` field of an ACFG
   sidecar, also check the standard destructure-and-rebuild pattern
   in `nucleus/nucleus-compiler/src/passes/*.rs` — every pass that
   destructures the ACFG must mention the field, OR a compile error
   fires; report any pass that does NOT.

## Output format

```
## Sibling audit: <symbol>

### Touched (this cycle / recent commits)
- path/to/file.rs:LINE  <one-line context>

### Untouched call sites (audit candidates)
- path/to/file.rs:LINE  <one-line context>
  Verdict: looks-same | different-shape | needs-eyes

### Recommendation
GO (every site addressed) | INVESTIGATE (N sites flagged
needs-eyes) | NO-GO (M structurally-identical siblings untouched)
```

## When NOT to use

- The symbol is a syntactic primitive (`unwrap`, `clone`) — too noisy.
- The diff is a wholesale rewrite of the file — every line is "touched".
- The user already greped all call sites manually and just wants a
  spot-check; do the spot-check inline, no skill needed.

## Limits

This skill cannot reason about WHETHER an untouched sibling needs
the same fix — that's an architectural judgement. It surfaces the
candidates; the user (or a follow-up architect agent) decides.

The skill also cannot detect "structurally-identical sibling that
uses a DIFFERENT symbol" (renamed-but-equivalent code). That class
needs an architect review, not a grep.
