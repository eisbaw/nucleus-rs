---
id: TASK-0288
title: >-
  multi_binary: Typed extra-sections slot to replace raw extra_dependencies
  textual passthrough (TASK-0257 P2.1 hardening)
status: Done
assignee: []
created_date: '2026-05-24 19:27'
updated_date: '2026-06-10 19:31'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Replace the raw extra_dependencies: Option<&str> textual passthrough in backend_common::project_skeleton with a typed slot that captures the REAL usage (verified across all 9 callers): an OPTIONAL single [dependencies] section whose body is verbatim TOML text (real bodies include comment lines + one entry like mio = {...}, so a named-entry map would NOT round-trip byte-identically and is speculative generality).

Design: introduce a small typed struct ExtraDependencies (wraps the verbatim [dependencies] section body) with a constructor + the renderer owning the [dependencies] header. None-equivalent = no struct. This (a) makes the section header owned by the renderer not the caller string, killing the "jam [dev-dependencies]/multiple sections through a param named dependencies" footgun, and (b) preserves verbatim comment+entry bodies byte-for-byte.

Callers (single_binary): pthreads-async(None x2), pthreads-sync(None), openmp-rs(None|Some rayon), mpi-blocking(Some MPI_DEP x2), mpi-nonblocking(Some MPI_DEP x2). Callers (multi_binary): mp-tcp-bufsync(None x2), mp-tcp-event(None+Some mio), mp-tcp-poll(None x2), mp-uds-event(None x2+Some mio). Migrate every call site mechanically.

AC#1 typed shape; AC#2 mp-tcp-event MIO block reconstructed via typed form, emit byte-identical; AC#3 new test for typed shape. Emit-path: A/B diff -r one mp-tcp cell + one mpi cell before/after to /tmp (campaign oracle). Verify: cargo test -p backend-common + each touched backend crate; clippy --workspace --all-targets.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0044.03.02 (event_plan lift, cycle of commit b1c2ffe): the cycle-197b architect P3.1 RendezvousDirStrategy idea (parameterise render_run_sh_multi so mp-uds-event need not substring-swap the shared template output) is now subsumed at the EVENT-BACKEND layer by the EventTransport::render_run_sh_post trait method in backend_common::event_plan — TCP returns the shared run.sh unchanged, UDS does the /tmp-rooted mktemp swap (still a single ALLOW-annotated String::replace on a FIXED bash-literal block, fail-loud on needle-miss). So the run.sh divergence is no longer duplicated across the two event backends, but it is STILL a post-hoc substring swap rather than a typed slot in render_run_sh_multi itself. If TASK-0288 lands a typed extra-sections / typed-rendezvous-strategy slot in the shared multi_binary template, EventTransport::render_run_sh_post on the UDS impl becomes a candidate to retire (replace the swap with a typed RendezvousDirStrategy::TmpMktemp arg). Single consumer today (UDS only); low priority.

IMPLEMENTED (cycle, not yet committed — left In Progress per orchestrator policy). Replaced raw extra_dependencies: Option<&str> textual passthrough with a typed CargoDependencies<a> newtype in backend_common::project_skeleton (a single OPTIONAL [dependencies] section body; renderer owns the [dependencies] header). Constructors: ::none() and ::section_body(body). Shared private render_into() helper used by BOTH single_binary::render_cargo_toml and multi_binary::render_cargo_toml (de-dups the prior two inline interpolations into one source of truth incl. trailing-newline normalisation).

Design rationale (no speculative generality): all 9 callers pass either None or a verbatim [dependencies] body; the mio bodies interleave comment lines + one entry, so a named-entry map would NOT round-trip comments byte-identically. Newtype keeps body verbatim while making the SECTION typed -> a future caller cannot jam [dev-dependencies]/multi-section TOML through a param named dependencies. Second-section support deferred until a real caller needs it (documented).

AC#1 DONE (typed shape). AC#2 DONE (mp-tcp-event MIO_DEPENDENCY_BLOCK now CargoDependencies::section_body(...); emit byte-identical). AC#3 DONE (new cargo_dependencies_tests mod: default_equals_none, section_body_is_interpolated_verbatim_under_one_owned_header, dependencies_section_is_byte_identical_across_both_layouts, trailing_newline_normalised_in_multi_binary_too).

Migrated all 9 call sites across 9 backends. Fixed stale extra_dependencies doc-lies in openmp-rs src + tests/skeleton.rs + mp-tcp-event/mp-uds-event MIO block docstrings.

VERIFICATION: backend-common lib 31/31 (dev+release); all 9 touched backend crates green; clippy --workspace --all-targets exit 0; cargo doc -p backend-common 0 warnings (fixed 2 private-item intra-doc-link warnings on render_into). A/B emit oracle: diff -r before/after IDENTICAL on 4 cells (mp-tcp-event Some-MIO multi_binary, mpi-blocking Some-MPI single_binary, pthreads-sync None single_binary, mp-tcp-bufsync None multi_binary). Touched files (11, all in ownership): backend-common/src/project_skeleton.rs; backends/{pthreads-sync,pthreads-async,openmp-rs,mpi-blocking,mpi-nonblocking,mp-tcp-bufsync,mp-tcp-event,mp-tcp-poll,mp-uds-event}/src/lib.rs; backends/openmp-rs/tests/skeleton.rs. NOTE: forward-carried UDS RendezvousDirStrategy retire-candidate (impl-notes) NOT done — out of scope, separate run/sh concern.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Typed CargoDependencies<lifetime> slot (none()/section_body()) replaces the raw Option<&str> extra_dependencies passthrough; one private render_into serves single_binary + multi_binary (de-duplicated); all 9 backend call sites migrated; A/B emit-oracle byte-identical. Review fold-in 38fea10 added the body-validation assert (no smuggled [section] headers — the so_buf precedent) making the docstring contract true. Deliberately NO dev-dependencies support (no real caller; no speculative generality). Note: this task predates structured ACs (description-body criteria only) — all three description criteria met, including the explicitly-justified deviation on dev-dependencies. backend-common 31/31 dev+release with 4 new typed-slot tests; clippy + cargo doc clean. Landed 996b00c + 38fea10; architect GO; wave gate green.
<!-- SECTION:FINAL_SUMMARY:END -->
