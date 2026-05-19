---
id: TASK-0177
title: De-fragilize wrap_single_worker module-path coupling (mp-tcp)
status: To Do
assignee: []
created_date: '2026-05-19 01:02'
labels:
  - M3
  - backend
  - tech-debt
dependencies:
  - TASK-0036
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0036 (non-blocking follow-up): mp-tcp-bufsync wrap_single_worker does replacen("mod kernels;", ...) on the SHARED pthreads-sync single-worker renderer output to redirect the kernels module #[path]. Correct today (render_main_rs provably emits the literal `mod kernels;` token) but fragile-by-coupling: a future change to the shared renderer module spelling would silently break the #[path] redirect. Replace the string-replace with an explicit module-path injection parameter on the shared single-worker renderer (render_single_worker_main / RenderCtxPub) so the coupling is a typed API, not a brittle token match.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Shared single-worker renderer takes an explicit kernels module-path parameter (no string replacen)
- [ ] #2 mp-tcp + pthreads-sync both use the typed API; byte-identical output preserved
<!-- AC:END -->
