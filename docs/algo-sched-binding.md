# Algorithm ↔ schedule binding: name-by-string convention

TASK-0059 / PRD §13 risks list. Documents the rule that schedules
reference algorithm symbols by string name, the consequence (renames
invalidate schedules at next build, not silently), and the explicit
v2 decision against fuzzy-match / refactoring tooling.

## The convention

A schedule file (`*.sched.nuc`) names algorithm symbols by their
**string identifier**:

- `place blur3 on { w0 };` — `blur3` matches the algorithm's
  `kernel blur3 : (...) -> ... pure;` declaration by exact-equal
  string compare.
- `loop y : block=8;` — `y` matches the algorithm's `for y : 0..H { ... }`
  loop variable by exact-equal string compare.
- `transfer img_in : sync;` — `img_in` matches the algorithm's
  `data img_in : f32[H][W];` declaration by exact-equal string compare.

The schedule grammar (`docs/grammar-sched.md`) does not have a parallel
type system for these references. The link step
(`nucleus-compiler/src/link.rs`) is where the cross-file binding is
checked: every schedule symbol must resolve to exactly one algorithm
symbol of the right shape, or the build fails with a typed
`LinkErrorKind` (e.g. `UnknownKernel`, `UnknownData`, `UnknownLoop`).
TASK-0099 (cycle 74) added byte-range spans to these errors so the
diagnostic points at the offending source token, not just the symbol
name.

## Cascade behaviour

Renaming an algorithm symbol (`kernel blur3` → `kernel blur_3x3`)
**silently invalidates every schedule that references the old name**.
The first `nucleus build` against the now-mismatched schedule fails
loud with an `UnknownKernel { name: "blur3", ... }` error naming the
offending schedule directive's source location. This is the safest
failure mode for v2 — loud at next build, not silent and not at
runtime.

## v2 decisions (recorded)

- **Why string names, not stable IDs.** Stable IDs (e.g. `kernel #0042
  blur3`) would let the compiler track a renamed kernel through its
  identity rather than its name. v2 explicitly rejects this:
  schedules are written by humans, and an `#0042` opaque token in a
  user-visible schedule file is worse for the user than a clean rename
  cascade. A v3 with a refactoring tool (a la `cargo fix --rename`)
  could revisit, but is out of scope.
- **No `nucleus list-refs` tooling.** A `nucleus list-refs --kernel X`
  command (find every schedule that references kernel `X` in the
  current repo) was considered. **Rejected for v2.** Rationale:
  `git grep "place X "` in any half-decent editor returns the same
  answer in milliseconds; building a domain-specific lister adds a
  surface to maintain (CLI flag, output format, test coverage) that
  duplicates a tool every user already has. Reconsider if a real
  user-workflow surfaces where `git grep` is insufficient (e.g.
  multi-file rename support, semantic-aware match across `place foo`
  vs `transfer foo`).
- **No fuzzy-match suggestions in link errors.** The
  `nucleus-compiler/src/link.rs` `UnknownKernel` / `UnknownData` /
  `UnknownLoop` variants do carry a `suggestion: Option<String>`
  field (TASK-0096), populated by a string-similarity check against
  the algorithm's declared names. This is a *quality-of-life*
  diagnostic, not a refactoring tool: it suggests the closest
  reasonable spelling, never auto-applies.
- **No automatic rename refactoring.** v2 has no `nucleus rename
  kernel X Y` command. The user does the rename in both files; the
  link step verifies they're consistent at next build.

## Honest limitations

- A schedule that references a no-longer-existing algorithm symbol
  fails at the link step, but only when the user re-runs `nucleus
  build`. There is no IDE / file-watcher integration that flags the
  inconsistency in-editor before the build.
- The string-similarity suggestion is single-symbol-at-a-time. A
  renamed-but-still-typo'd schedule (`blur3` → `blu_r3` instead of
  `blur_3x3`) gets the same suggestion as a fresh typo, with no
  hint that the symbol was previously known by the old name.
- No cross-schedule consistency check: two schedules in the same
  example that both reference a missing kernel each get their own
  error, with no "this rename broke 3 schedules" summary.

These limitations are acceptable for v2's "one repo, hand-edited
schedules" workflow. A v3 with multi-author schedule libraries would
revisit.
