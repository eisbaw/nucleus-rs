---
id: TASK-0287
title: >-
  e2e_example_05.rs: brace-balance scan for for-x body extraction (TASK-0282
  P2.2 hardening)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-24 18:20'
updated_date: '2026-05-24 18:49'
labels:
  - M5
  - reuse
  - testing
  - hardening
  - forward-carried-from-TASK-0282
dependencies:
  - TASK-0282
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0282 (cycle 110, commit 6984c64) added an AC#4 grep assertion to `nucleus/nucleus-compiler/tests/e2e_example_05.rs` that extracts the inner for-x body by finding `'for x in'` then the next `'{'` then the next `'\n        }'` (literal 8-space indent + close brace).

## Risk
Fragile to legitimate emit-shape changes:
- A future `block_transform` interposing a tile-outer loop above for-x shifts the indent.
- A partition-wrap or check-frame wrap adds an indent level.
- An inner sub-block with its own `\n        }` token would short-circuit the search and silently shrink the extracted body.

In the silent-shrink case the `<= 3` count assertion would still PASS (the truncated body contains fewer `img_in[` reads) — a regression-masking gap exactly the kind CLAUDE.md `feedback-silent-sibling-defect` memory warns about.

## Acceptance Criteria
1. Replace the textual `find('{')` + `find('\n        }')` walk with a brace-balance scan from `for_x_open_brace + 1` to the matching close brace at depth 0. Same complexity, no indent assumption.
2. Verify the new extraction still finds the same body shape on the current 05-stencil/reuse emit (test still PASSES after the refactor).
3. Add a negative-fixture regression: synthetic emit with an extra outer indent level — assert the test STILL extracts the correct for-x body (not a truncated parent / grandparent body).

## Dependencies
- TASK-0282 (Done).

## Honest scope
- Test robustness only — no behaviour change in the codegen itself. Cheap fix.

## Forward-carried from TASK-0282 architect P2.2
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Replace the literal-textual body extraction in e2e_example_05.rs with a brace-balance scan + add a synthetic-indent negative regression.

Implementation:
1. Refactor the body extraction in e2e_example_05.rs into a helper fn extract_for_x_body(emit: &str) -> &str that:
   - Finds 'for x in' (existing).
   - Finds the next '{' (existing).
   - Scans forward from there, tracking brace depth (starting at 1). Increment on '{', decrement on '}', stop when depth returns to 0. Return the slice [open+1..close-1].
   - Skip braces inside string literals (Rust source emit may contain {} in format-args; the current emit shape doesn't, but skip-string-literals is hygiene for the future).
2. Update the existing for-x body extraction site to call the helper.
3. Verify the existing 05-stencil/reuse e2e test still passes (the new helper finds the same body on the actual current emit).
4. Add a unit test extract_for_x_body_handles_nested_outer_indent — synthetic emit string with an extra outer for-loop wrap above the for-x. Assert the helper extracts the inner for-x body, NOT the outer for body. Counts img_in occurrences to verify the right slice is returned.

Gate: cargo test --workspace (819 prior baseline, +1 for new helper test), e2e + determinism preserved.

Honest scope: test robustness only. No codegen change. Cheap helper + regression pin.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE-111 REVIEW: qa-test-runner GO (820 tests pass; no new fmt drift on touched files). mped-architect GO (brace-balance scan correctness verified by argument: bottom-up depth tracking, slice [open+1, idx) exclusive, ASCII brace bytes cannot appear in UTF-8 continuation bytes). P3 observation: string-literal hole in scan documented as benign today (no for-x body emit contains string literals — verified empirically against 05-stencil reuse emits). Memory entry project-cross-backend-differential updated to reflect cycle-111 closure of P2.1+P2.2 forward-carries.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 111 LANDED in commit 91de823. extract_for_x_body(emit) helper using brace-balance scan from depth 1 past the open brace until depth 0 replaces the pre-TASK-0287 literal find of newline+8spaces+closing-brace. The existing 05-stencil/reuse e2e test calls the helper unchanged. Bite-verified regression test extract_for_x_body_handles_nested_outer_indent: synthetic 3-deep nest with let bad img_in read in the for-y suffix; helper correctly stops at the inner-x close brace, NOT silently extending into the for-y body. Orchestrator manually verified the bite (temporary revert to textual-find shape makes the test FAIL on the silent-truncation symptom). All 3 ACs met. Gate: 820 tests pass, e2e 92/79/0/13/0, determinism 92/79/0/13, clippy clean. Cycle-111 review (qa + architect) both GO.
<!-- SECTION:FINAL_SUMMARY:END -->
