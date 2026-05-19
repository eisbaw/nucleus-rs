---
id: TASK-0036
title: 'mp-tcp-bufsync backend crate (second backend, tier 1)'
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 00:12'
labels:
  - M3
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Multi-process over TCP loopback, sync blocking, buffered. PRD §7.1. Workers are OS processes; transport is std::net::TcpStream; sync = blocking recv. Forces capability matrix to be real.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/mp-tcp-bufsync/ is a crate with capabilities.toml.
- [ ] #2 Emit produces N Rust binaries (one per worker) plus a run.sh that launches them and wires them up over loopback.
- [ ] #3 Workers connect via a deterministic handshake; ports either auto-allocated or passed via env.
- [ ] #4 Each Push lowers to a length-prefixed write on the appropriate socket; each Wait to a blocking read.
- [ ] #5 Test: synthetic two-worker pingpong matches pthreads-sync output bit-for-bit.
- [ ] #6 Implementation notes record design questions (e.g. handshake protocol; whether to use SO_REUSEADDR; how to handle Bind errors).
- [ ] #7 Implementation notes record honest limitations (no buffer-pool reuse; one allocation per transfer at M3; perf is not a goal).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
[forward-carried from TASK-0124] The (per_worker EventList, NameTables, NameSidecar) contract is now PROVEN sufficient for a tier-1 backend: pthreads-sync consumes ONLY compiler::event + compiler::sidecar + the inert IrExpr grammar the EventList carries (NO AlgoIR/LinkedIR/ACFG), single- AND multi-worker, byte-identical e2e+determinism. mp-tcp-bufsync should consume the SAME tuple. Reuse the pattern: driver builds acfg_to_events(&acfg)+build_sidecar(&linked,&acfg).map_err(...)? + reverse NameTables; backend emit() takes (&per_worker,&names,&sidecar,kernels,out). Use an EmitError::ContractGap fail-loud variant for any missing contract fact (never default). CAVEATS that apply identically to the 2nd backend: (1) Event::Sync carries NO stable cross-worker barrier identity — TASK-0172; multi-worker barrier-id recovery is a per-worker pre-order-index heuristic valid only for UNIFORM barriers (fail loud otherwise). (2) block_transform DEFERS absolute-index rebinding (LO+tile*N+inner) to codegen; the EventList faithfully carries the tiled nest so the backend MUST rebind or an accumulator double-counts. TASK-0124 rebinds only the evenly-divisible case; non-divisible/partial-tile is TASK-0173. Any EventList-consuming backend inherits both.
<!-- SECTION:NOTES:END -->
