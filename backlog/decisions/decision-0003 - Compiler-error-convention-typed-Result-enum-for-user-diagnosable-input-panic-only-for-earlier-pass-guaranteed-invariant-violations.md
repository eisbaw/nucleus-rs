---
id: decision-0003
title: >-
  Compiler error convention: typed Result enum for user-diagnosable input, panic
  only for earlier-pass-guaranteed invariant violations
date: '2026-05-19 14:20'
status: accepted
---
## Context

The compiler pipeline already practices two distinct error
conventions, but the *rule* for which one applies where was unwritten
tribal knowledge — and the cost of leaving it tribal is a recurring
defect class (below).

The triggering finding (mped-architect review of acf8bab/8fad5d3,
TASK-0155): the driver pipeline mixes two conventions in adjacent
calls.

- `apply_block_transforms`
  (`nucleus-compiler/src/passes/block_transform.rs`) returns
  `Result<_, BlockTransformError>` — a `pub enum` whose variants carry
  enough context for a user-facing diagnostic — surfaced by the driver
  as a clean `nucleus: error:` stderr line, no backtrace.
- `inject_transfers`
  (`nucleus-compiler/src/passes/transfer_inject.rs:299` and `:1356`) `panic!`s
  with full context when a cross-worker `Wait` escapes the whole ACFG
  with no producing `Operation` — a broken **cross-pass invariant**
  (`build_acfg` lowers the producing kernel for every symbol the
  schedule records a producer for; a `Wait` only exists because
  `build_waits_for_op` found that symbol in `producers_by_data`).

Both are *individually correct*. The first is diagnosable user input;
the second is a state a valid program cannot reach unless an earlier
pass is buggy. The defect is not either call site — it is that the
selection rule existed only in reviewers' heads.

This is the same shape the codebase already standardised on. The
canonical written precedent is `nucleus-compiler/src/acfg.rs:620-624`: a doc
comment on `BuildAcfgError` explicitly states that the *other* `panic!`s
reachable from `build_acfg` (kernel with no placement at `:887`,
`.expect("kernel id assigned during pre-pass; link guarantees
existence")` at `:911`, `.expect("iter var collected during pre-pass")`
at `:805`) "are genuine link-pass invariant violations — `link` rejects
such programs first — so they stay `panic!`s, not variants here."
That rationale is correct and consistent across the codebase; it was
just never lifted out of one module into a project convention.

The recurring cost of leaving it tribal — the **panic-not-diagnostic
defect class**: this project has repeatedly shipped `panic!` on a path
a valid user program *can* reach, where a typed error belonged.
TASK-0170 (`SidecarError`) and TASK-0179 (`BuildAcfgError`'s
`NonConstLoopBound`: `for j : 0 .. i` lowers and links cleanly but
cannot be const-folded — diagnosable user input that was panicking)
each closed one such. The failure is almost always in that direction
(panicked where a typed error belonged), rarely the reverse.

The established typed-error family already in the tree (the pattern
this record names as precedent): `ParseErrorKind`
(`error.rs:29`), `CapError` (`capabilities.rs:420`), `SidecarError`
(`sidecar.rs:256`, TASK-0170), `DeadlockError`
(`passes/deadlock.rs:111`), `ContractError` (`contract.rs:93`),
`LinkError` (`link.rs:150`), `FireError` (`petri.rs:174`),
`BuildAcfgError` (`acfg.rs:626`, TASK-0179), `BoundednessError`
(`passes/boundedness.rs:83`), `BlockTransformError`
(`passes/block_transform.rs:121`). Each is a `pub enum` in its own
pass's module, returned via `Result`, surfaced by the driver as a
clean `nucleus: error:` stderr line.

Options weighed:

- (a) Leave it unwritten — rely on review to catch
  panic-not-diagnostic each time.
- (b) Document the convention in `transfer_inject`'s module docs only
  (the task's narrower option).
- (c) A decision record stating the rule project-wide, with
  `//!` pointers from the two module docs the finding named.

## Decision

Adopt (c). The convention, **descriptive of and consistent with the
code as it stands today** (no behaviour change — TASK-0155 is
explicitly docs-only):

**User-diagnosable error** — bad or unsupported user input,
capability mismatch, an unschedulable / deadlocking program, a
non-const loop bound, an unbounded net, a contract violation, etc.:
the state is reachable by a *valid, well-formed* program or schedule.
→ Return `Result<_, TypedError>` where `TypedError` is a `pub enum`
defined in that pass's own module, each variant carrying enough
context to produce an actionable diagnostic without the caller
threading extra state. The driver surfaces it as a single clean
`nucleus: error:` stderr line, no backtrace. Precedent: the
`ParseErrorKind` / `CapError` / `SidecarError` / `DeadlockError` /
`ContractError` / `LinkError` / `FireError` / `BuildAcfgError` /
`BoundednessError` / `BlockTransformError` family above.

**Compiler-invariant violation** — a state the *earlier* pipeline
(parse → link → lower → earlier passes) guarantees cannot occur for
valid IR: `panic!` / `.expect(...)` with a message that names the
invariant *and* which pass guarantees it. This is a bug-in-the-compiler
signal; it is never a user-facing path. Precedent:
`acfg.rs:620-624` (the written rationale), `acfg.rs:887`
(`panic!("kernel argument references ... not a declared data
symbol")`), `acfg.rs:911` (`.expect("kernel id assigned during
pre-pass; link guarantees existence")`), and the two
`transfer_inject` cross-pass-invariant panics
(`transfer_inject.rs:299`, `:1356`), whose messages already name the
invariant ("A Wait is only emitted when the schedule records a
producer for the symbol, so this is a malformed ACFG ... not a partial
test input").

**The deciding test** (apply this at every fallible site):

> *Can a valid, well-formed user program or schedule reach this
> state?*
>
> - **Yes** → typed `Result`. A `panic!` here is the recurring
>   panic-not-diagnostic defect, not an acceptable shortcut.
> - **No — it is reachable only via a compiler bug, or it asserts an
>   invariant an earlier pass already enforces** → `panic!` /
>   `.expect(...)`, and the message MUST state *which pass / which
>   invariant* guarantees it cannot happen for valid IR.

**Bias rule for the uncertain case:** when it is not obvious whether a
malformed-looking state is user-reachable or earlier-pass-excluded,
prefer the typed `Result`. The panic-not-diagnostic defect history
(TASK-0170, TASK-0179) shows the mistake is overwhelmingly
"panicked where a typed error belonged"; the reverse (a typed error
for a true compiler invariant) costs at most a slightly less terse
internal failure, never a lost user diagnostic.

Rationale, against the alternatives:

- **vs (a) — leave unwritten.** This is precisely the gate-trust /
  recurring-defect anti-pattern the project already rejects elsewhere
  (decision-0002's lineage: a property only enforced by manual review
  is, in practice, under-enforced). Two tasks (TASK-0170, TASK-0179)
  already paid to fix instances of exactly this. Rejected.
- **vs (b) — `transfer_inject` module docs only.** The rule is
  project-wide (ten typed-error enums across nine modules; invariant
  panics in `acfg` *and* `transfer_inject`). Burying the
  authoritative statement in one pass's module doc repeats the
  original mistake at a different scope — the next pass author would
  not find it. A decision record is the project-level home; module
  docs then *point* at it (AC#2). Adopted as (c).

## Consequences

- New record `backlog/decisions/decision-0003`; **zero code behaviour
  change** (TASK-0155 is docs-only). Proven by the gate: `just
  determinism-check` stays byte-identical and `just e2e` stays
  30/26/0/4 — a `//!` doc line does not perturb generated code.
- `transfer_inject.rs` and `block_transform.rs` module docs carry a
  one-line `//!` pointer to this record stating which side each pass
  is on (TASK-0155 AC#2): `transfer_inject` = invariant-panic side
  (`:299`/`:1356`); `block_transform` = user-diagnosable side
  (`BlockTransformError`). The pointers describe what those modules
  *actually do today* — comment-honesty is itself a reviewed defect
  class, so the references are accurate, not aspirational.
- A new `panic!` / `.expect(...)` on a path a valid user program can
  reach is, from now on, a **reviewable defect**, not a stylistic
  choice — the deciding test above is the objective criterion a
  reviewer applies. Inversely, an invariant `panic!` whose message
  does *not* name the guaranteeing pass is also a defect (it cannot be
  triaged as compiler-bug vs user-issue).
- This record is the reference point for the open error-handling /
  parser-quality backlog cluster: any pass that adds a fallible path
  follows the deciding test; any task that adds user-facing error
  surfaces adopts the typed-`pub enum` + driver `nucleus: error:`
  pattern rather than inventing a new mechanism.
- If a future change makes a currently-`panic!`'d invariant genuinely
  user-reachable (e.g. a grammar relaxation, as TASK-0179 was for
  triangular loop bounds), the fix is to convert that site to a typed
  `Result` variant — not to widen the panic. Such conversions
  reference this record.
- Closes TASK-0155 (AC#1: convention written in a decision record;
  AC#2: the two named modules reference it).
