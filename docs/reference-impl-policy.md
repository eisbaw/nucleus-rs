# Reference Implementation Provenance Policy

Status: normative for M0 onward. Enforcement is informal until M2 (see §6).
Cross-reference: [PRD §10.1](../nuc-nucleus/PRD.md) (Tier 1 — bit-identical
differential test), [PRD §12.2](../nuc-nucleus/PRD.md) (Cargo workspace).

## 1. File layout

For every driving example `examples/NN-name/` (PRD §9), the
reference implementation lives at `examples/NN-name/reference/`.
Contents:

- A standalone Rust source file or a small Cargo project that
  implements the algorithm independently of Nucleus. Typical shape:
  - `examples/NN-name/reference/Cargo.toml` — manifest, own crate.
  - `examples/NN-name/reference/src/main.rs` — the implementation,
    plus a CLI that reads `--in INPUT.bin` and writes `--out
    OUTPUT.bin`.
- `examples/NN-name/input.bin` — committed binary input fixture.
- `examples/NN-name/reference.bin` — committed binary expected output.
- A regeneration command, documented in the example's `README.md`.
  Canonical form:

  ```
  cargo run --release \
    --manifest-path examples/NN-name/reference/Cargo.toml -- \
    --in    examples/NN-name/input.bin \
    --out   examples/NN-name/reference.bin
  ```

`input.bin` and `reference.bin` are tracked in Git as binaries. They
are small (kilobytes to a few megabytes) and change rarely; Git LFS
is not used in v2.

## 2. The independence rule (hard rule)

Reference implementations MUST NOT depend on:

- The Nucleus compiler crate.
- Any Nuc-generated source file (anything under `out/` or anything
  produced by `nucleus build`).
- Any backend crate (`pthreads-sync`, `mp-tcp-event`, `mpi-blocking`,
  any embedded shim, etc.).
- Any Nuc-internal IR, EventList, or Petri-net library.

Allowed: `std`, `core`, the Rust toolchain, and small,
well-known third-party crates only where they materially simplify
the reference (e.g. `byteorder` for endian-explicit IO). New
third-party dependencies require reviewer sign-off — the reference
is meant to be auditable, not feature-rich.

Rationale: if the reference shares code with any backend, then a
shared bug produces matching output across the entire (schedule ×
backend) matrix and the differential test cannot see it. The PRD
calls this failure mode "all backends are wrong the same way" (§10.1).

A `Cargo.toml` in a `reference/` directory MUST NOT have a
`workspace = ".."` link to `nucleus/Cargo.toml`. References are
standalone crates outside the Nucleus workspace by design.

## 3. Audit requirements

When the semantics of an algorithm change — anything that alters the
expected byte stream in `reference.bin` — the same commit MUST update:

1. The `.algo.nuc` source.
2. The reference implementation under `examples/NN-name/reference/`.
3. The committed `reference.bin`.

A commit that touches `examples/NN-name/*.algo.nuc` without touching
the matching `reference/` directory and `reference.bin` is suspect.
Reviewers must verify all three move together. Examples of changes
that count as semantic:

- New, renamed, or removed kernels referenced by the example.
- A change to a kernel body in `kernels.rs` that alters output bytes.
- A change to the algorithm's dataflow or iteration bounds.
- A change to the input layout in `input.bin`.

Examples of changes that do NOT count as semantic (no reference
update required):

- Comment-only edits in `.algo.nuc`.
- Whitespace, formatting, or rename of an unused symbol.
- Schedule-file changes (`*.sched.nuc`) — schedules are explicitly
  required to leave output bit-identical (PRD §10.1). A schedule
  change that alters `reference.bin` is a compiler bug, not a
  reference-update event.

## 4. Regeneration policy

`reference.bin` is regenerated only when algorithm semantics change
(see §3). Specifically:

- **Allowed regeneration:** the algorithm or its reference impl
  changed; the new `reference.bin` is produced by re-running the
  command in §1 and committed in the same PR.
- **Forbidden regeneration:** the reference impl was re-run and
  produced a different byte stream without any §3-class change. This
  is a bug in the reference impl (non-determinism, undefined
  behaviour, platform-dependent code, etc.) and a merge blocker. Do
  not commit the new `reference.bin` — fix the reference first.

Reviewers should ask, on any PR touching `reference.bin`: which
file in §3 justifies the byte change? If the answer is "none",
reject.

## 5. Determinism requirement

Reference impls MUST be bit-deterministic across:

- Repeated runs on the same machine.
- Different host CPU vendors and microarchitectures.
- Different OS / libc versions within the supported set.

Operational rules:

- Prefer integer arithmetic. Integer ops are bit-deterministic by
  language definition.
- Floating-point reductions are permitted only with a stated, fixed
  reduction order (e.g. strict left-to-right accumulation, with no
  use of `f32::sum` / `f32::reduce` that may reorder). The reference
  source must make the order explicit in code, not just in a
  comment.
- Avoid `HashMap`/`HashSet` iteration order, `std::time::*`-derived
  inputs, threading non-determinism, and any external state.
- No `#[cfg(target_arch = "...")]` branches that produce different
  output bytes.

A reference impl that violates determinism is a §4 blocker — fix
before the PR lands, do not paper over with epsilon comparisons.
Bit-identical is non-negotiable for Tier 1 (PRD §10.1).

The broader project-wide floating-point determinism policy
(reduction tree shape rules, denormal handling, fast-math
prohibitions) is being captured separately as TASK-0060. Until that
lands, the bullets above are the working rule.

## 6. Drift detection

- **M0 → M1 (informal):** the reviewer manually checks on every PR
  touching `*.algo.nuc`, `examples/NN-name/reference/`, or
  `reference.bin`, that all three are consistent. There is no CI
  check.
- **M2 onward (enforced):** CI runs a `reference-regen` job that, for
  each example, executes the §1 regeneration command and diffs the
  produced bytes against the committed `reference.bin`. Any
  difference fails the build with a message naming the example and
  pointing at this policy. A separate follow-up task (see §8) tracks
  building this CI hook.

The asymmetry — paper policy now, CI gate later — is deliberate. M0
has no CI matrix yet (PRD §11). Once the M2 Petri-net work lands and
the e2e harness exists, the drift check costs almost nothing to add.

## 7. What this policy is NOT

- **Not a CI-enforced check at M0.** Until M2, this is a reviewer
  checklist. Honest about the gap; not pretending the policy is
  mechanical.
- **Not a fuzzy-comparison policy.** Reference output is compared
  bit-for-bit against backend output. No epsilon, no ULPs, no
  tolerance. Examples whose algorithm cannot be made deterministic
  are excluded from Tier 1 (PRD §10.1, §13).
- **Not a description of the backend test harness.** That belongs to
  the e2e crate and the §10.1 differential matrix. This document
  only governs how `reference.bin` comes into existence.
- **Not a license to share code between reference and Nucleus.** Even
  a "tiny utility" shared with a backend crate violates §2.

## 8. Follow-up tasks tracked elsewhere

- CI hook to verify `reference.bin` freshness on PRs touching
  `*.algo.nuc` (lands at M2). Filed as TASK-0061.
- A `just regen-references` recipe (or `nucleus-e2e --regen-refs`
  flag) that re-runs every example's reference command in one shot.
  Filed as TASK-0062.
- Project-wide floating-point determinism policy. Filed as TASK-0060
  per task brief.
