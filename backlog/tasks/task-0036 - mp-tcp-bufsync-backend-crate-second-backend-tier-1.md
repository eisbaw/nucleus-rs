---
id: TASK-0036
title: 'mp-tcp-bufsync backend crate (second backend, tier 1)'
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
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
