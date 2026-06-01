---
id: TASK-0415
title: >-
  check-include-str-coverage uses bashism (done < <(...)) despite POSIX /bin/sh
  portability claim (TASK-0395 cycle architect P3-1)
status: To Do
assignee: []
created_date: '2026-06-01 21:07'
labels:
  - tooling
  - ci
  - doc-lie
  - posix
  - cycle-0395-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3-1 from the TASK-0395 review (commit 89025a4). The doc-fence recipe headers (justfile ~504/676/992) advertise POSIX-shell portability (just runs /bin/sh -cu; no bash arrays / process substitution). But check-include-str-coverage (justfile ~326) uses `done < <(...)` process substitution, a BASHISM that dash/posh would reject. This is a doc-vs-code inconsistency in the comment-doc-lie class (CLAUDE.md recurring defect #1): either the recipe is not actually POSIX (and the portability comment overclaims) or just is running these under bash, not /bin/sh, on this system.

NOT introduced by TASK-0395 (pre-existing) and that recipe uses explicit nucleus/.../src roots so it has NO untracked-scratch footgun — purely a portability-claim consistency nit. Fix options: (1) rewrite the `< <(...)` as a POSIX `while read` over a mktemp temp file (like the doc-citation fences do), or (2) if just genuinely runs bash here, correct the recipe-header comments that claim /bin/sh POSIX-cleanliness. Verify which by checking just's shell setting (settings / shell:) and testing under dash. LOW / OPTIONAL: zero functional effect today (the CI host has a bash-compatible sh).
<!-- SECTION:DESCRIPTION:END -->
