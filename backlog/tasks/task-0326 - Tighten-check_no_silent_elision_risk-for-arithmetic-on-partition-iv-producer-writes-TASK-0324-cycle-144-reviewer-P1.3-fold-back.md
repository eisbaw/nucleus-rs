---
id: TASK-0326
title: >-
  Tighten check_no_silent_elision_risk for arithmetic-on-partition-iv producer
  writes (TASK-0324 cycle-144 reviewer P1.3 fold-back)
status: To Do
assignee: []
created_date: '2026-05-25 14:19'
labels:
  - compiler
  - transfer_inject
  - validator-coverage
  - M6
  - forward-carried-from-TASK-0324
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0324 cycle-144 reviewer (mped-architect) P1.3: the per-axis discriminator in `check_no_silent_elision_risk` skips axes where the producer's write index is not a bare Ident — including the case where the producer writes with an ARITHMETIC expression involving a partition iv (e.g. `tmp[hy*2][hx]`, `tmp[hy+1][hx]`).

Semantically these accesses ARE partition-sliced (the worker writes only its own transformed range), but the structural check (`ident_iv_in_set` returns None on non-Ident) treats the axis as whole-array → no constraint on the consumer's axis-k read → potential silent elision the same way as the underlying TASK-0324 case.

## Cycle-144 disclosure (in-thread)

The validator at `transfer_inject.rs` (the `p_iv = ident_iv_in_set(...)` branch in the per-axis loop) carries an explicit `CONSERVATIVELY-NOT-REJECTED` comment naming this task. No in-tree schedule today exercises arithmetic producer-side indices on a partition iv.

## Acceptance criteria

### AC#1: enrich the discriminator

Extend `ident_iv_in_set` (or its consumer) to also detect arithmetic expressions involving a partition iv. The simplest sufficient extension: walk the `IrExpr` tree for any `IrExpr::Ident` referencing an iv in `partition_iter_vars`; if found, mark the axis as partition-sliced and require the consumer's axis-k read to ALSO contain only that same partition iv (i.e. same arithmetic shape — or at minimum, same set of referenced ivs).

### AC#2: positive + negative tests

- Positive fixture: producer writes `tmp[hy*2][hx]` (or similar) on {w0..w3} with hy partitioned; consumer (same set) reads `tmp[hy*2][hx]` → expect `Ok` (read shape matches write shape, worker reads its own slice).
- Positive fixture: producer writes `tmp[hy*2][hx]` on {w0..w3} with hy partitioned; consumer reads `tmp[other_iv][hx]` → expect `Err`.

### AC#3: alignment with halo machinery

Halo widths (TASK-0260) extend a worker's local tile by the kernel's access pattern. If the producer writes with arithmetic but the access stays within the halo-extended tile, the elision IS safe. The validator must NOT over-reject in that case — either honour halo widths in the discriminator OR document the conservative-reject path with a halo-aware escape valve.

## Dependencies

- TASK-0324 (cycle-144 base validator). This task tightens its discriminator on a path the cycle-144 implementer intentionally left under-conservative pending an in-tree need.

## Honest scope

- LOW priority. Dormant path. Filed so the under-conservative comment at transfer_inject.rs in the cycle-144 fold-back has a tracker anchor (per TASK-0319 / future-audit discipline: every 'conservatively not rejected' code comment needs a tracker reference).
<!-- SECTION:DESCRIPTION:END -->
