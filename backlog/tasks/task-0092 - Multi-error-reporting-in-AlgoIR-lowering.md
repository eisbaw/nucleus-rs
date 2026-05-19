---
id: TASK-0092
title: Multi-error reporting in AlgoIR lowering
status: To Do
assignee: []
created_date: '2026-05-18 00:25'
updated_date: '2026-05-19 19:47'
labels:
  - M0
  - compiler
  - diagnostics
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
lower_algo currently aborts on the first LowerError. Mirror the multi-error follow-up filed for the parser (TASK-0079) so users see all violations in one compile cycle. Filed as follow-up from TASK-0009.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0082 (DONE): algo AST is Spanned<T>-wrapped (compiler/src/algo/span.rs); lower.rs projects .node and currently IGNORES spans. TASK-0090 wires spans into LowerError; multi-error lowering here can then carry a precise (line,col) per error via error::offset_to_line_col on the relevant Spanned node\u0027s span.start. Keep typed-Result, no panic (decision-0003).

Forward-carried from TASK-0090 (DONE, commit 1c4e90a): AlgoIR lowering errors are now LOCATED — LowerError = { kind: LowerErrorKind, span: Option<Range<usize>> }, span populated at each diagnosable err site from the offending Spanned. Multi-error reporting (this task) should collect Vec<LowerError> and the located span on each element gives a per-error line:col for free via LowerError::display_with_src (driver-side, source held by driver). NOTE: lower_algo currently early-returns on first Err; multi-error needs the recursion to accumulate rather than `?`-bail. The position substrate is done; this task is the accumulation/recovery design on top of it. Equality forwards to .kind only (span informational) so dedup/grouping of collected errors keys on the semantic kind, not the offset.

Forward-carried from TASK-0080/0081 (DONE, commits be43c33/12af9b9). Different layer (AlgoIR lowering, not parsing) and a different error type (LowerError, not ParseError), so do NOT reuse ParseErrors directly — but the SURFACING pattern is the template:

1. Driver multi-error surface: see nucleus/driver/src/main.rs parse_algo call site — header line + one indented located line per error, matching the established link/contract shape. Lowering should produce a Vec<LowerError> owner and the driver should iterate it the same way (currently lower_algo uses e.display_with_src(&algo_src); a multi-error LowerErrors owner should Display/iterate analogously).
2. Determinism discipline (load-bearing): NO HashMap/HashSet on the error path; collect+order deterministically; if you render any chumsky/auxiliary message, beware hash-iteration order (we had to root-cause-fix chumsky Simple Display — sorted expected set in error::chumsky_message). Dedup, if any, must be order-preserving Vec-based.
3. Recovery in lowering is a different problem (no chumsky combinator) — likely collect-and-continue across independent lowering units rather than parser recovery; the bounded+deterministic + "single clean error => exactly one" + no-spurious-cascade test discipline still applies.
4. Gate identically (e2e 30/26/0/4/0, determinism x2 byte-identical, negatives bite, clippy --all-targets, ci exit 0) and migrate negatives with strength preserved (return first, dedicated no-cascade test).
<!-- SECTION:NOTES:END -->
