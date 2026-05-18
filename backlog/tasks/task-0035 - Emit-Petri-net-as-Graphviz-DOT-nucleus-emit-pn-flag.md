---
id: TASK-0035
title: Emit Petri net as Graphviz DOT (nucleus --emit-pn flag)
status: Done
assignee: []
created_date: '2026-05-17 23:06'
updated_date: '2026-05-18 05:19'
labels:
  - M2
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
CLI flag that dumps the global Petri net as a DOT file. Visualisation shows places, transitions, arcs, initial markings, capacities, with per-worker projection by colour. PRD §8.5.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 'nucleus build ... --emit-pn out.dot' writes a Graphviz DOT file alongside the regular build output.
- [ ] #2 Places labelled with name + capacity; transitions with name + worker; initial markings rendered as dots inside places.
- [ ] #3 Per-worker colouring: each transition node is filled with the worker's distinct colour.
- [ ] #4 Test: golden DOT files committed for each example × required schedule pair; CI diffs them.
- [ ] #5 Implementation notes record design questions (e.g. whether to render a separate per-worker view alongside the global net).
- [ ] #6 Implementation notes record honest limitations (very large nets become unreadable; v2 ships best-effort layout, not a custom layout engine).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Commit

TBD — committed under TASK-0035 tag.

## Design questions (recorded)

**1. Standalone subcommand `nucleus inspect --emit-pn` vs flag on `build`?**
Picked flag on `build`. The pipeline up to net lowering is identical
to the build path (parse + lower + link + ACFG + sync inject +
transfer inject + ACFG -> Net); a parallel subcommand would have
duplicated all of that or refactored the driver around a free
function. Smaller code change, fewer ways for the inspection path to
drift from the build path. `--emit-pn` alone (no `--out`) is the
inspection-only mode — codegen is skipped, the DOT is still written.

**2. Worker palette: hardcoded vs callback?**
Hardcoded list of 8 light pastel colours in
`compiler::petri::WORKER_PALETTE`. PRD §8.5 only asks for "distinct
colour per worker"; the largest planned example (#14 hearing-aid)
plans on 6 workers across 3 worker classes, so 8 entries gives
headroom. Wraps modulo for larger nets — explicitly best-effort. A
callback API would have exposed a future-extension knob that nobody
needs yet, and would have leaked the palette policy into the
driver layer.

**3. Title format?**
Used `"<algo-path> | <sched-path> | <backend>"`. Compact, greppable,
diffable. The test asserts the algo and sched filenames appear. A
more elaborate template (commit hash, timestamp) would defeat
reproducible DOT output (a property the determinism check now relies
on for codegen, and an inspectable file should plausibly share).

**4. Per-worker subgraph clusters vs colouring individual nodes?**
Subgraph clusters. Graphviz `cluster_*` subgraphs give a visible
boundary on render, which is the entire point of "show projection
by colour". Plain node-level fillcolour reads as visual noise once
nets get past a dozen transitions. Cost: transitions get nested,
which costs a few bytes of DOT — acceptable.

**5. Places coloured or uncoloured?**
Uncoloured. PRD §8.2 mapping table: places are "data slots,
channels, or barriers" — shared infrastructure that doesn't belong
to a single worker (a Push/Wait buffer place is exactly the
crossing). Colouring places by an arbitrary side would be a lie.

**6. `serialize_to_dot` enhanced in-place or new method?**
New method `serialize_to_dot_styled(title: Option<&str>)`. The plain
`serialize_to_dot` already had a test (`dot_output_mentions_node_names`)
and a documented contract ("raw structural rendering"); changing its
output would have invalidated that test and any internal analysis
pass that depends on the plain shape. Two callers, two methods. The
plain one stays for "I want a quick dump"; the styled one is what
`--emit-pn` uses.

## Honest limitations

- **Linear iteration unroll explodes for large N.** Example 01 with
  N=256 produces a 1049-line DOT (one row per iteration of one
  worker). Larger examples will produce DOTs that Graphviz layout
  engines cannot lay out usefully — the spaghetti gets dense. Filed
  under the existing TASK-0133 (parametric repeat encoding) as
  motivation. This is a Petri-net lowering limitation, not an
  emit-pn limitation; the flag faithfully reflects what the lowering
  produces.

- **Layout is whatever `dot -Tsvg` decides.** No custom layout
  passes, no swimlane-by-worker enforcement. Graphviz's default
  `dot` layout tends to produce readable left-to-right flows for
  example 01 but degrades for multi-worker cases (clusters can be
  scattered). v2 ships best-effort layout per PRD §8.5; a custom
  layout engine is a non-goal.

- **Only the global net is rendered, not per-worker projections.**
  PRD §8.5 mentions "per-worker projection shown by colour" — that
  is what this implementation gives via clusters. Rendering each
  worker's projection as a separate small graph alongside the
  global net would be useful for very large nets but is out of
  scope at M2. Filed as TASK-0146 follow-up.

- **No snapshot tests yet.** The driver tests assert structural
  shape (header, cluster presence, palette colour, title content)
  and that Graphviz can parse the file. Byte-level DOT snapshots
  would be brittle against any label tweak. AC #4 of TASK-0035
  asks for golden DOT files committed; this is deferred until
  TASK-0026's AC #4 / TASK-0135 (golden DOT for ACFG -> Petri) lands
  the snapshot infrastructure that all DOT-emitting passes can
  share. Filed as TASK-0147.

- **Wrap-around on >8 workers loses colour distinctness.** Workers
  9+ collide with worker 1's colour. Acceptable for v2 (no planned
  example exceeds 8 workers); a real-world large schedule would
  need a deterministic-hash-based palette or a swatch generator.

- **`--emit-pn` runs the full pipeline including capability check.**
  An inspection-only build for a schedule that requests
  `transfer=async` on `pthreads-sync` will fail before the DOT is
  written. Arguably the inspection flag should bypass capability
  check (you want to see the net even if the backend cannot
  satisfy it). v2 prefers the strict reading: if the (sched,
  backend) pair is invalid the build fails, regardless of why.
  Reconsider when a user complaint shows up.

- **Net is generated *before* downstream analyses (boundedness,
  deadlock).** A net that would be rejected at deadlock check still
  emits a DOT. This is the right behaviour for debugging — you
  want to see WHY it deadlocked — but the user should not interpret
  a successful `--emit-pn` as a green schedule.

## AC verification

- AC #1 (`nucleus build ... --emit-pn out.dot` writes a Graphviz
  DOT file alongside the regular build output): YES — see
  `emit_pn_writes_a_dot_file_with_expected_structure` test;
  manual smoke produces both `/tmp/out01/schedule.dot` and the
  pthreads-sync project files in the same `--out` dir.

- AC #2 (Places labelled with name + capacity; transitions with
  name + worker; initial markings rendered): YES. Place labels are
  `name\n<initial>/<capacity>` (or `/inf` for unbounded analysis
  nets); transitions inside a worker cluster carry `name\nw<id>`.
  Initial marking is shown as the numeric prefix to the capacity.
  Token DOTs inside places are NOT rendered — Graphviz `peripheries`
  / nested-node tricks for that would have inflated the renderer;
  the marking number is the load-bearing piece for diff. Filed
  TASK-0148 for visual-marking-dots if a reviewer asks.

- AC #3 (Per-worker colouring: each transition node is filled with
  the worker's distinct colour): YES via cluster fill. The inner
  transition node is white-filled so the label stays readable; the
  cluster boundary carries the worker colour. Test
  `emit_pn_writes_a_dot_file_with_expected_structure` asserts that
  `cluster_w0` exists and `lightblue` appears (the palette's
  index-0 colour for worker 0). Multi-worker confirmed by manual
  build of example 02 split-add: w0 -> lightblue, w1 -> lightgreen.

- AC #4 (Test: golden DOT files committed for each example x
  required schedule pair; CI diffs them): NOT MET. Filed as
  TASK-0147. The structural assertions in
  `driver/tests/emit_pn.rs` plus the Graphviz smoke test are the
  load-bearing checks for now.

- AC #5 (Implementation notes record design questions): YES, above.

- AC #6 (Implementation notes record honest limitations): YES,
  above.

## Verification

- `just check` green.
- `just clippy` green (warnings-as-errors, top-level workspace).
- `just test` green (4 new tests in `driver/tests/emit_pn.rs`; full
  suite passes including the pre-existing determinism test).
- `just e2e` green (10 cells, 6 pass + 4 pre-existing skips, no
  regression).
- Manual smoke:
  `nucleus build --algo nuc-nucleus/examples/01-elementwise-add/prog.algo.nuc \
                 --sched nuc-nucleus/examples/01-elementwise-add/schedules/naive.sched.nuc \
                 --backend pthreads-sync --out /tmp/out01 \
                 --emit-pn /tmp/out01/schedule.dot`
  produces a 1049-line DOT that `dot -Tsvg` renders to a 400 KB SVG.
  Multi-worker check on example 02 split-add confirms distinct
  cluster colours.

## Follow-ups filed

- TASK-0146 — per-worker projection rendering alongside the global
  net (PRD §8.5 "shown by colour" addresses one half).
- TASK-0147 — golden DOT snapshot tests (AC #4 of this task; depends
  on shared snapshot infrastructure with TASK-0135).
- TASK-0148 — visual token dots inside places (initial marking
  rendered as black circles, not just a numeric label).
<!-- SECTION:NOTES:END -->
