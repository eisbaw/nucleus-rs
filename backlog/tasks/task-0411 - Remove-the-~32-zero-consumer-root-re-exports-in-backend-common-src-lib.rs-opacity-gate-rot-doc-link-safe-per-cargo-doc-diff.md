---
id: TASK-0411
title: >-
  Remove the ~32 zero-consumer root re-exports in backend-common/src/lib.rs
  (opacity-gate-rot; doc-link-safe per cargo-doc diff)
status: To Do
assignee: []
created_date: '2026-06-01 09:50'
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
