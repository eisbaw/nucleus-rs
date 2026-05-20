---
id: TASK-0203
title: >-
  Add test: EffectStmt to declared-but-failed-kernel cascade short-circuit
  (asserts the lower.rs:784-790 comment claim)
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-19 22:33'
updated_date: '2026-05-20 05:21'
labels:
  - compiler
  - diagnostics
  - tests
  - follow-up
  - M0
  - doc-lie-audit
dependencies:
  - TASK-0092
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
lower.rs:784-790 (Stmt::Effect arm purity check) asserts in a comment that if the kernel was declared but its body failed to lower, the existing is_cascade_of_failed_decl UnknownIdent suppression collapses the error to the root declaration failure. The existing test effect_stmt_to_unknown_kernel_stays_unknown_ident only covers the never-declared path. The declared-but-failed-body path is asserted in the comment but NOT measured — the exact comment-doc-lie class this project keeps recurring on. Filed from TASK-0089 architecture review (Finding #4, 2026-05-20). NOTE: this test will FAIL until TASK-0092's case-1 transitive-poison one-line fix lands (because the cascade-decl path currently does not poison the kernel name in failed_decls); so the test is the right discriminator to land ALONGSIDE the transitive fix.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 New test in compiler/tests/algo_lower.rs: a kernel signature contains e.g. i32[BAD_CONST] so the kernel lowering fails (kernel poisoned in failed_decls); a downstream bare-call bad_kernel(); produces EXACTLY one error (the upstream BAD_CONST root failure, NOT also the UnknownIdent cascade); located line:col pinned
- [ ] #2 Asserts both: (a) no UnknownIdent cascade for the bare-call; (b) the root failure has the correct kind and span; (c) no EffectCalleeNotEffectful spuriously emitted (purity check naturally short-circuits when kernel is not in ir.kernels)
- [ ] #3 just test passes; just ci exit 0; no behaviour change for valid input; clippy --workspace --all-targets clean
- [ ] #4 Lands AFTER (or as part of) the TASK-0092 case-1 transitive-poison one-line fix; depends on TASK-0092 for that reason — without the fix this test would FAIL by design
<!-- AC:END -->
