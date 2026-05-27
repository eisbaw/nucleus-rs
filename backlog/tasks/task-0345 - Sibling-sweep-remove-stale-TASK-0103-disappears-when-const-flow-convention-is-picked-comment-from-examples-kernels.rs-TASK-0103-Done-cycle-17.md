---
id: TASK-0345
title: >-
  Sibling sweep: remove stale TASK-0103 'disappears when const-flow convention
  is picked' comment from examples/*/kernels.rs (TASK-0103 Done cycle 17)
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-27 10:34'
updated_date: '2026-05-27 12:05'
labels:
  - examples
  - docs
  - comment-doc-lie
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect cycle-199 P3.4 follow-up. TASK-0103 (PRD §6.2.2 example kernels.rs uses Nuc consts as Rust generics) was CLOSED cycle 17 (2026-05-22). However seven examples' kernels.rs files still carry the stale claim 'Single-source-of-truth violation (TASK-0103); disappears when the const-flow convention is picked':

- nuc-nucleus/examples/03-reduction/kernels.rs:54
- nuc-nucleus/examples/04-prefix-sum/kernels.rs:67
- nuc-nucleus/examples/05-stencil/kernels.rs:61
- nuc-nucleus/examples/06-separable-filter/kernels.rs:60
- nuc-nucleus/examples/07-matmul/kernels.rs:73
- nuc-nucleus/examples/08-histogram/kernels.rs:57
- nuc-nucleus/examples/10-wavefront/kernels.rs:43 (inherited from template in cycle 199)

The 'convention is picked' phrasing reads as 'future work' — but TASK-0103 closed by deciding the const-flow convention IS the Vec<i32> + runtime length-check pattern these examples already use. So the comment self-contradicts: it claims a violation that the closed task accepted as the canonical pattern.

This is a feedback-comment-doc-lie-recurring + feedback-silent-sibling-defect double-pattern: TASK-0103's closure didn't sweep the seven inherited comments, and every new example (most recently 10-wavefront) inherits the lie by template.

Fix: rewrite each occurrence to either (a) remove the comment entirely (since the Vec<i32> pattern IS the convention now) or (b) cite TASK-0103 as the DECISION, not a violation. Then check if the README's reference to 'Why Vec<i32>' section also needs updating.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 203 — SWEPT (9 sites)

Rewrote the stale "Resolves when TASK-0103 picks a convention" / "Single-source-of-truth violation (TASK-0103); disappears when the const-flow convention is picked" framing across the 7 kernels.rs sites originally enumerated + 2 sibling README sites discovered in-cycle:

- nuc-nucleus/examples/03-reduction/kernels.rs (2 paragraphs: "Why Vec<i32>" header + const N reference)
- nuc-nucleus/examples/04-prefix-sum/kernels.rs (2 paragraphs)
- nuc-nucleus/examples/05-stencil/kernels.rs (2 paragraphs)
- nuc-nucleus/examples/05-stencil/README.md (1 paragraph — silent-sibling discovered in-cycle)
- nuc-nucleus/examples/06-separable-filter/kernels.rs (2 paragraphs)
- nuc-nucleus/examples/07-matmul/kernels.rs (2 paragraphs)
- nuc-nucleus/examples/07-matmul/README.md (1 paragraph — silent-sibling discovered in-cycle)
- nuc-nucleus/examples/08-histogram/kernels.rs (2 paragraphs)
- nuc-nucleus/examples/10-wavefront/kernels.rs (2 paragraphs)

New canonical framing:
- "Why Vec<i32>" header paragraphs: "Per TASK-0103 (Done cycle 17): `Vec<i32>` + runtime length check IS the canonical convention for aggregate-typed kernel signatures. The PRD §6.2.2 sketch `Box<[[f32; W]; H]>` did not compile as plain Rust (W and H are not Rust constants); `Vec<i32>` with explicit length checks is the resolution."
- const-N reference paragraphs: "The doubled declaration is the v2 convention per TASK-0103 (Done cycle 17): kernels.rs is plain Rust compiled by the host toolchain unmodified — Nucleus does not text-substitute algorithm consts into kernel bodies."

### In-cycle silent-sibling discovery

The original task description listed only 7 kernels.rs sites. A final grep verification found 2 README sites (07-matmul/README.md, 05-stencil/README.md) carrying the same stale-claim shape. Treated as in-scope per the `feedback-silent-sibling-defect` recurrence guard.

### Verification (cycle 203)

- just build: clean.
- just clippy: clean -D warnings.
- just test: clean (no test logic touched; only comments + docstrings).
- just test-release: clean.
- just e2e: 238/211/0/27/0 — UNCHANGED from cycle-202 baseline (expected; doc-only edits).

### No new follow-ups filed

The doc-lie class is now swept from the examples tree. The cycle-12 cycle-201 silently-fixed-bitonic instance (TASK-0044.06 cycle 200 used the new framing from the start) confirmed the fix is straightforward; cycle-203 brings the prior 9 sites into line.

Future-safety: any new example added by template-copy from the existing 7 will inherit the corrected framing. The `feedback-comment-doc-lie-recurring` recurrence guard should catch a recurrence at review-gate time.
<!-- SECTION:NOTES:END -->
