---
id: TASK-0411
title: >-
  Remove the ~32 zero-consumer root re-exports in backend-common/src/lib.rs
  (opacity-gate-rot; doc-link-safe per cargo-doc diff)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 09:50'
updated_date: '2026-06-01 10:50'
labels:
  - hardening
  - dead-code-audit
  - review-pass
  - cycle-237-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0407 architect-review P2. backend-common/src/lib.rs has 35 root pub-use re-exports; only THREE have crate-root-path code consumers (EmitError, elect_host_from_name_workers, elect_host_from_worker_names). The other ~32 are reachable AND actually consumed only via the submodule path (backend_common::render::X, backend_common::check_frame::X), so the root pub-use lines are redundant-for-reachability dead weight on an INTERNAL (unpublished) workspace crate -- the feedback-opacity-gate-rot / feedback-visibility-tighten-doclink-trap shape. TASK-0407 KEPT them citing the doc-link-trap, but the architect EMPIRICALLY disproved that justification for the 32: a clean cargo doc --no-deps --workspace yields only the 10 pre-existing warnings (none in backend-common), and the ONLY crate-root-resolving intra-doc link among the re-exported names is [EmitError] at lib.rs:30 -- and EmitError stays (it is root-consumed). SCOPE: remove the ~32 zero-root-consumer pub-use re-exports (keep the 3 root-consumed + EmitError doc-link). METHOD: before removing each, re-grep workspace for backend_common::<name> root-path use (silent-sibling discipline -- a future consumer may have been added); run cargo doc --no-deps --workspace BEFORE and AFTER and diff the warning set (must not grow); the gate does NOT build docs so this is the only catch for a broken intra-doc link. Run from nucleus/. LOW-MEDIUM leverage; pure dead-weight removal, no behaviour change. Sibling of TASK-0410 (dead error-variant removal).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation (cycle-237 followup): re-derived keep/remove set via workspace consumer grep (comment lines filtered, target/ excluded). Root-path consumers: EmitError=11, elect_host_from_name_workers=6, elect_host_from_worker_names=5 — KEPT (pub use render::EmitError; pub use host_election::{elect_host_from_name_workers, elect_host_from_worker_names}). All other ~32 names (whole check_frame block, project_skeleton render_cargo_toml/render_run_sh, HOST_NAME, all render::* except EmitError) had ZERO root-path consumers — REMOVED. HOST_NAME had zero references anywhere in consumer crates. No brace-import use backend_common::{...} exists anywhere; every root consumer is a single-name use. GOTCHA verified-not-assumed: in-crate intra-doc links to removed names (sanitize_loop_var, RenderCtx, RenderCtxPub, render_fire_args_nostd, HOST_NAME, etc.) resolve via their DEFINING module, not the root re-export, so removal is doc-link-safe. cargo doc --workspace --no-deps: 10 generated warnings (2+2+4+2) BEFORE and AFTER, zero unresolved links. Gate: build clean, clippy -D warnings exit 0 (forced fresh recompile of backend-common), test 1237/0, test-release 1236/0, e2e 385/328/0/57/0 — all unchanged (no behaviour change).

DONE: removed 32 zero-root-consumer re-exports; kept the 3 root-consumed (EmitError + 2 elect_*). Gate green, cargo-doc warnings 10->10 (no growth).

ORCHESTRATOR REVIEW GATE (cycle-239, batched with TASK-0410): qa GO + architect GO on cc4e078, ZERO blocking findings. qa: forced-fresh clippy exit 0 (14 crates recompiled, no unused-import from the 32 removals), test 1237/1236, e2e 385/328/0/57/0 x2, cargo doc summed-generated 10 BEFORE/AFTER no growth + ZERO unresolved links (doc-link-trap cleared -- the gate does not build docs so this was the load-bearing check). architect INDEPENDENTLY re-derived the keep/remove set: HOST_NAME genuinely unused everywhere (4 hits all inside host_election.rs); write_file removed because each backend has its OWN private fn write_file; no use backend_common::{...} brace-import exists to hide a root consumer; the 3 KEPT (EmitError 11, elect_host_from_name_workers 6, elect_host_from_worker_names 5 CODE consumers) correct. Doc-link safety confirmed via rustdoc scope semantics (in-crate links to removed names resolve via the defining submodule, not the deleted root re-export; cross-module block_tag.rs [RenderCtxPub] resolves via its local use crate::render::). 3rd-edit convenience comment ACCURATE (raw grep -c gives 12/7 but the +1 each is a comment/doc line; comment correctly states the 11/6/5 CODE-consumer counts). ARCHITECT MINOR NOTE (no action): a future naive re-auditor running grep -c backend_common::EmitError will see 12 and may wrongly "correct" the accurate 11 -- the delta is the comment line. Pure dead-weight removal, behaviour-preserving.
<!-- SECTION:NOTES:END -->
