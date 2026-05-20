---
id: decision-0004
title: >-
  DataflowStmt RHS purity is unidirectional: grammar §2 note 5 is canonical, no
  RHS-pure enforcement
date: '2026-05-20 21:47'
status: accepted
---
## Context

TASK-0089 ("Enforce kernel-purity vs statement-form") landed enforcement
of one direction: an `EffectStmt` (bare-call statement) callee must be
`Purity::Effectful` — emit `LowerErrorKind::EffectCalleeNotEffectful`
otherwise. The implementation matches grammar §2 note 5 verbatim.

The OTHER direction — whether a `DataflowStmt` RHS `Call` must reference
a `Purity::Pure` kernel — was deferred to this record because three
spec carriers disagreed:

- **PRD §2 row at line 77** (delta-vs-2013-thesis table) reads, verbatim:
  > `Where-clauses could side-effect silently` | `` `where pure` mandatory; `where !effectful` opt-in. ``
  Read in isolation, this can be heard as a project-wide bidirectional
  purity claim — `where pure` mandatory on every dataflow position.
- **Grammar §2 note 5** (`docs/grammar-algo.md:137-139`) reads, verbatim:
  > **Bare-call statements (`EffectStmt`)** are only valid when the
  > called kernel is `effectful`. A pure kernel called for its value
  > thrown away is a warning (likely error) at type-check time.
  Only the `EffectStmt` direction is constrained. Note 5 is silent
  about the dataflow direction.
- **Every shipped example** (verified by grep across
  `nuc-nucleus/examples/`):
  - `01-elementwise-add/prog.algo.nuc:48-49`: `a <-- load_input(); b <-- load_input_b();` — both `effectful`.
  - `02-split-add/prog.algo.nuc:67-68`: `a <-- load_input(); b <-- load_input_b();` — both `effectful`.
  - `03-reduction/prog.algo.nuc:85`: `a <-- load_input();` — `effectful`.
  - `04-prefix-sum/prog.algo.nuc:97`: `in_arr <-- load_input();` — `effectful`.
  - `05-stencil/prog.algo.nuc:67`: `img_in <-- load_image();` — `effectful`.
  - `06-separable-filter/prog.algo.nuc:91`: `in_arr <-- load_image();` — `effectful`.
  - `07-matmul/prog.algo.nuc:81-82`: `a <-- load_a(); b <-- load_b();` — both `effectful`.
  - `13-cnn-inference/prog.algo.nuc:45`: `input <-- load_input();` — `effectful`.
  - `14-hearing-aid/prog.algo.nuc:68-69`: `mic_in[frame] <-- fe_capture(); bt_in[frame] <-- rf_receive();` — both `effectful`.

The empirical fact: 9/9 shipped examples use an `effectful` kernel on the
RHS of `<--`. A strict bidirectional rule would reject every one of
them. The TASK-0089 onboarding finding ("the strict reading contradicts
every shipped example AND grammar §2 note 5") surfaced this conflict
without resolving it; this record resolves it.

Options weighed:

- **(a) Bidirectional.** Make `DataflowStmt` RHS `Call` of an `effectful`
  kernel an error. Redesign IO across all 9 examples: introduce a
  separate IO-capture form distinct from kernels (per AC#2 — explicitly
  **not** by rewriting examples to add intermediate pure wrappers
  around effectful loads, which only hides the side-effect at the
  AlgoIR layer without removing it). Closes more of the 2013 sin
  (no effectful kernel can hide inside a dataflow expression).
- **(b) Unidirectional.** Grammar §2 note 5 is canonical: only
  `EffectStmt → Effectful` is enforced; `DataflowStmt` RHS purity is
  unconstrained at the algorithm-language level. The 2013 sin is
  closed in the *bare-call* direction, where its bite was — silent
  side-effects in `where` clauses (legacy 2013 syntax) — and explicitly
  declared open in the dataflow direction. PRD line 77 is tightened to
  match.

## Decision

Adopt **(b) — unidirectional**. Grammar §2 note 5 is the canonical
rule. The `DataflowStmt` RHS may call either a `Pure` or an
`Effectful` kernel; the algorithm-language well-formedness checker
neither requires nor warns about RHS purity.

Rationale, against (a):

- **Note 5 is a formal grammar artefact; PRD line 77 is informal table
  prose about the 2013-vs-v2 delta.** Where they disagree, the formal
  artefact is canonical. The same precedence holds in the test corpus:
  9/9 examples lower cleanly under (b); 0/9 under (a). The strict
  bidirectional interpretation was the looser reading of a table cell
  that summarised "kernels are now annotated `pure` / `effectful`" — it
  was not stating a position-dependent enforcement direction.

- **The 2013 sin's actual shape was the `where`-clause / kernel-body
  case** (legacy `kernel NAME(args) -> out where pure {{ ${out} = ... }}`
  syntax — see SKETCH.md:60-78 for the pre-v2 form). That syntax could
  silently side-effect *inside* a kernel body without being marked
  effectful. The v2 fix is the `pure | effectful` annotation on the
  `KernelDecl` itself (PRD §6.2.2 lines 251-275), with kernel bodies
  living in Rust source verified at `cargo build` time. The
  `EffectStmt → Effectful` rule (note 5) closes the orthogonal case
  where a bare statement invokes a pure-marked kernel for its
  meaningless discarded return value. Together these close the 2013
  sin at the points where it bit; the dataflow-RHS direction is a
  separate, weaker claim with no recorded 2013 evidence behind it.

- **Effectful-kernel-on-dataflow-RHS is a load-bearing v2 idiom**, not
  an accidental example pattern. `fe_capture()` (`14-hearing-aid`)
  returns the next audio frame as a value *and* advances the input
  source as a side-effect; `load_input()` (`01..07,13`) returns the
  input array *and* consumes the input stream. The value-and-effect
  semantics are exactly what the dataflow form expresses: the LHS
  binds the value, the schedule serialises the effect at the
  containing basic block (BB-order preservation for `effectful`
  kernels — PRD §6.2.2 lines 273-274). Forcing IO through a separate
  capture form would split kernel-as-Rust-function (PRD §6.2.2 line 243:
  "A kernel is a **real Rust function**") into two unrelated mechanisms
  without a recorded benefit.

- **Reorderability of dataflow statements is a schedule concern, not
  an algorithm-language well-formedness concern.** Inside a basic
  block the schedule preserves issue order for `effectful` kernels and
  may reorder `pure` kernels — the kernel's `Purity` annotation is the
  signal. The algorithm-language IR is a *declarative* dataflow
  description; checking RHS purity at lowering time would not change
  what reorderings are legal downstream (the schedule already reads
  `Purity` from the resolved kernel). The check would only reject
  every existing IO pattern with no compensating semantic gain.

- **(a) requires inventing surface syntax not present in the grammar.**
  No production in `docs/grammar-algo.md` §1 currently expresses
  "capture / IO statement distinct from `DataflowStmt`". Adding one
  is a substantial language change for a sin (silent dataflow-RHS
  effectfulness) that has not bitten in 9 examples and has no recorded
  bug instance. The cost is concrete; the benefit is hypothetical.

**Bias rule** (for the uncertain case): when the grammar and the PRD
disagree, the grammar wins. PRD prose is descriptive; the grammar is
the contract the parser and lowering passes implement. Any future
spec ambiguity should be resolved by tightening the PRD to match the
grammar (or by changing both in a recorded decision), never by silently
overcommitting the implementation past the grammar.

## Consequences

- **Zero code change.** TASK-0089 already implements the unidirectional
  rule. The `EffectCalleeNotEffectful` variant exists and fires on the
  enforced direction; no `DataflowCalleeNotPure` variant is introduced.

- **PRD §2 row at line 77 is tightened.** The cell text moves from the
  loose `` `where pure` mandatory; `where !effectful` opt-in. `` to a
  v2-accurate phrasing that names the actual closure mechanism (kernel
  `pure | effectful` annotation + `EffectStmt → Effectful` enforcement
  via grammar §2 note 5) and explicitly disclaims any bidirectional
  reading. See PRD diff in this commit.

- **`algo/ir.rs` module-doc** is updated to remove the "deferred —
  pending TASK-0201" hedge that TASK-0089 left in place, and to cite
  this decision record as the resolution. The pointer in the
  `EffectCalleeNotEffectful` variant doc (`ir.rs:289-291`) likewise
  loses its hedge.

- **Test docstrings** in `tests/algo_lower.rs:1746-1817` already
  describe the deferred-decision state correctly; the language stays
  load-bearing (the
  `pure_dataflow_with_effectful_rhs_load_lowers` test is the
  regression guard against any future strict-bidirectional
  reinterpretation) and is retargeted from "pending TASK-0201" to
  "fixed by decision-0004".

- **No example redesign.** Examples 01-07, 13, 14 continue to use
  effectful kernels directly on the RHS of `<--`. This is the
  canonical v2 IO idiom.

- **SKETCH.md is left untouched.** It is a pre-v2 brainstorming
  artefact (uses 2013-style `where pure {{ ... }}` syntax and `@y`
  iteration-variable prefixes that v2 dropped). It is not a normative
  spec carrier; modernising it is a separate housekeeping task if it
  ever becomes one.

- **The bidirectional rule remains *recordedly* rejected, not just
  unspecified.** If a future task wants to reintroduce
  `DataflowCalleeNotPure` enforcement, it must reopen this decision
  with new evidence (a recorded bug instance the unidirectional rule
  missed, a thesis-claim that depends on it, or a backend that
  requires it) and a concrete redesign for the 9 example IO sites.
  "Tighter sounds better" is not sufficient.

- **Closes TASK-0201** (AC#1 — unidirectional decided; AC#3 — PRD and
  AlgoIR module-doc updated; AC#4 — recorded as `decision-0004`;
  AC#5 — no code change forced). AC#2 ("if bidirectional, redesign
  IO") does not apply and is recorded as N/A in the task notes.
