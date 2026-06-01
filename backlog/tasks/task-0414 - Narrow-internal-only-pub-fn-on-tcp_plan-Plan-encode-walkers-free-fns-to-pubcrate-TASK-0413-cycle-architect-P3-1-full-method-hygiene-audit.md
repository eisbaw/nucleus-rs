---
id: TASK-0414
title: >-
  Narrow internal-only pub fn on tcp_plan::Plan (+ encode/walkers free fns) to
  pub(crate) (TASK-0413 cycle architect P3-1, full method-hygiene audit)
status: To Do
assignee: []
created_date: '2026-06-01 20:28'
labels:
  - tooling
  - dead-code
  - backend-common
  - visibility-hygiene
  - cycle-0413-followup
dependencies:
  - TASK-0413
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3-1 follow-up from the TASK-0413 review (commit 472e88b). TASK-0413 tightened tcp_plan::Plan FIELD visibility; this is the parallel METHOD/free-fn hygiene to fully match the event_plan precedent (whose internal helpers are pub(crate) fn).

## Verified facts (orchestrator, this cycle)
The mp-tcp-{poll,bufsync} shims call EXACTLY these on the tcp_plan Plan (grep of actual call syntax, comments excluded): Plan::build, plan.worker_name, plan.render_worker_program, plan.render_run_sh, and read field plan.used_workers. NOTHING else.

So these tcp_plan::Plan pub fn are internal-only (callers only inside backend-common/src/tcp_plan) and CANDIDATES to narrow to pub(crate): data_name, non_host_workers, ctrl_var (the 3 the architect named), PLUS data_conn_var, relay_schedule, render_relay_phase, collect_pre_init, max_payload_bytes, render_events (the architect P3-1 UNDER-COUNTED — there are ~9, not 3). Also the pub fn FREE functions in tcp_plan/encode.rs (scalar_width, encode_expr, decode_expr, scalar_fn_suffix) and tcp_plan/walkers.rs (collect_xfer_data, relay_phase_insertion_point, collect_w2w_pushes, detect_wait_before_push_hazard, collect_barriers_by_tag) — note walkers is already pub(crate) mod so those are effectively crate-capped; check encode.rs mod visibility.

## Gotcha (forward-carried, reviewer-finding subtlety)
`Plan::non_host_workers` and `Plan::max_payload_bytes` appear in a mp-tcp-bufsync/src/lib.rs:257 COMMENT (backtick code-span) but are NOT external callers — they are used by the Plans OWN render_run_sh/render_worker_program internally. Audit by grepping ACTUAL call syntax (`plan\.<m>(` / `Plan::<m>(`), NOT bare backtick mentions, else you will mis-classify.

## Scope
1. Per-symbol external-caller audit (poll/bufsync src + any other consumer). Narrow each internal-only pub fn/free-fn to pub(crate). Keep build/worker_name/render_worker_program/render_run_sh pub (externally called) + the used_workers field pub.
2. Verify: cargo build --workspace + clippy + cargo doc --workspace --no-deps warning count UNCHANGED (baseline 14; memory feedback-visibility-tighten-doclink-trap — gate does NOT build docs, narrowing a doc-linked symbol breaks links silently). dead_code: each narrowed symbol must have an in-crate caller.
3. Update the tcp_plan::Plan struct docstring if the pub-method set changes (it currently lists build/render_worker_program/render_run_sh/worker_name as the boundary API — accurate today).

## Honest scope / priority
LOW / OPTIONAL: pure dead-/over-wide-surface hygiene, zero functional effect. Same class as TASK-0411/0412/0413. Higher symbol-count than 0413 but each is compiler-verified (E0616/private-fn errors are loud). Do NOT narrow a symbol an external shim calls.
<!-- SECTION:DESCRIPTION:END -->
