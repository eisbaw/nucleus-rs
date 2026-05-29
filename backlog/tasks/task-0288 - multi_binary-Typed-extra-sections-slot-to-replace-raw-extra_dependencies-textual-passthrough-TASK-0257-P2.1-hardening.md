---
id: TASK-0288
title: >-
  multi_binary: Typed extra-sections slot to replace raw extra_dependencies
  textual passthrough (TASK-0257 P2.1 hardening)
status: To Do
assignee: []
created_date: '2026-05-24 19:27'
updated_date: '2026-05-29 23:20'
labels:
  - backend-common
  - project_skeleton
  - hardening
  - forward-carried-from-TASK-0257
dependencies:
  - TASK-0257
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0257 (cycle 112, commit ca3bbdb) lifted the multi-binary project skeleton into backend_common::project_skeleton::multi_binary. The Cargo.toml emitter takes `extra_dependencies: Option<&str>` — a raw textual passthrough into a `[dependencies]` section.

## Risk
Today two callers: mp-tcp-bufsync passes None, mp-tcp-event passes a mio block. When a third multi-binary backend lands (mp-tcp-uring, mp-tcp-tokio, embedded?), the slot may need to carry [dev-dependencies] / [build-dependencies] / multiple sections / target-specific deps. The current shape silently lets the new caller jam a multi-section TOML string through a parameter named "dependencies", which becomes a documentation-vs-behaviour divergence.

## Acceptance criteria
1. Replace `extra_dependencies: Option<&str>` with a typed shape — either `extra_sections: &[(SectionHeader, &str)]` or a small `CargoDeps` struct with explicit fields per section type.
2. mp-tcp-event's MIO_DEPENDENCY_BLOCK is updated to construct the typed form. e2e byte-identical preserved (the emitted Cargo.toml bytes don't change).
3. Test added in project_skeleton::multi_binary_tests for the typed shape (e.g. building Cargo.toml with [dependencies] + [dev-dependencies]).

## Dependencies
- TASK-0257 (Done).
- Trigger: a third multi-binary backend or a section other than [dependencies]. Don't lift pre-emptively — wait for the second caller pattern.

## Honest scope
- LOW priority. Today two callers + one section type don't justify the abstraction; do this when the third caller materialises and the textual-passthrough rot starts to bite.

## Forward-carried from TASK-0257 architect P2.1
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0044.03.02 (event_plan lift, cycle of commit b1c2ffe): the cycle-197b architect P3.1 RendezvousDirStrategy idea (parameterise render_run_sh_multi so mp-uds-event need not substring-swap the shared template output) is now subsumed at the EVENT-BACKEND layer by the EventTransport::render_run_sh_post trait method in backend_common::event_plan — TCP returns the shared run.sh unchanged, UDS does the /tmp-rooted mktemp swap (still a single ALLOW-annotated String::replace on a FIXED bash-literal block, fail-loud on needle-miss). So the run.sh divergence is no longer duplicated across the two event backends, but it is STILL a post-hoc substring swap rather than a typed slot in render_run_sh_multi itself. If TASK-0288 lands a typed extra-sections / typed-rendezvous-strategy slot in the shared multi_binary template, EventTransport::render_run_sh_post on the UDS impl becomes a candidate to retire (replace the swap with a typed RendezvousDirStrategy::TmpMktemp arg). Single consumer today (UDS only); low priority.
<!-- SECTION:NOTES:END -->
