---
id: TASK-0287
title: >-
  e2e_example_05.rs: brace-balance scan for for-x body extraction (TASK-0282
  P2.2 hardening)
status: To Do
assignee: []
created_date: '2026-05-24 18:20'
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
