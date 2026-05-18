---
id: TASK-0004
title: Document reference-implementation provenance policy
status: Done
assignee: []
created_date: '2026-05-17 23:02'
updated_date: '2026-05-17 23:40'
labels:
  - M0
  - docs
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §10.1 says reference.bin comes from hand-written Rust under examples/NN/reference/. Write the explicit policy: where reference impls live, how they're committed, how to regenerate reference.bin, who audits changes. This closes the open item from earlier reviews.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs/reference-impl-policy.md describes file layout, audit requirements, regeneration command.
- [ ] #2 Policy specifies that reference impls are NOT Nuc-compiled — they are independent.
- [ ] #3 Policy specifies what happens when an algorithm's semantics change (reference must be updated in the same commit).
- [ ] #4 Test: docs file lints clean under any markdown linter we adopt.
- [ ] #5 Implementation notes record design questions (e.g. how to detect reference drift in CI).
- [ ] #6 Implementation notes record honest limitations (e.g. policy is on-paper; enforcement is informal until M2 CI lands).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation notes for TASK-0004

### Design questions resolved

- **When to enforce drift detection: M0 vs M2.**
  Settled on M0 = paper policy / reviewer checklist, M2 = CI-enforced
  regenerate-and-diff. Reasons:
    (a) M0 has no CI matrix at all (PRD §11 — M0 is just the
        skeleton + example 1). Adding a one-off drift-check CI now
        means writing CI plumbing that exists only to check one
        example.
    (b) The M2 e2e harness already needs the bytes of input.bin and
        reference.bin to run the differential matrix. Adding a
        regen-and-diff step on the same harness is a tiny extension,
        not a separate pipeline.
    (c) Honest about the M0 gap rather than pretending a paper policy
        is mechanically enforced.
  Trade-off: between M0 and M2 a careless reviewer can let a stale
  reference.bin land. Mitigation is the §3 checklist; this is a known
  hole, documented as such. Filed as TASK-0076.

- **How to handle reference-impl bugs found post-merge.**
  The policy is silent on this on purpose — once a reference bug
  ships and reference.bin reflects it, every backend's reference-
  matching test starts passing against a wrong target. The §4
  rule that 'reference re-run with different bytes is a bug, not a
  regen event' covers detection only when someone reruns. There is
  no detection mechanism for a stable-but-wrong reference. Mitigations
  considered:
    (a) A second, independently written reference. Rejected — doubles
        maintenance for marginal gain; the v2 examples are small
        enough to audit by inspection.
    (b) Property-based tests on the reference (e.g. 'sum-of-stencil
        equals known closed form'). Worth doing per-example where a
        property exists, but no project-wide policy is warranted yet.
  Leaving this as a known limitation; revisit if a reference bug
  ever ships.

- **Standalone Cargo project vs. single-file Rust.**
  Allowed both in §1. A single .rs file is enough for a sum reducer;
  a small Cargo project is appropriate when --in/--out CLI plumbing,
  byteorder handling, etc. add up. The regeneration command in the
  README is what makes the choice invisible to reviewers.

- **Where the policy file lives.**
  Picked /docs/reference-impl-policy.md rather than nuc-nucleus/. The
  nuc-nucleus/ directory is the v2 design space (PRD.md, SKETCH.md);
  policy that governs the *repo* (where reference bins live, how PRs
  are reviewed) belongs at the repo root under /docs/. This also
  matches the MPED principle that documentation lives near the
  code it governs — here, near examples/ rather than near the design
  PRD.

- **Link to PRD §10.1: anchor or section reference?**
  Used a plain link to the PRD file plus a parenthetical naming the
  section. GitHub auto-generates heading anchors but the rules for
  em-dashes and digits in section titles are fragile (e.g.
  '### 10.1 Tier 1 — bit-identical differential test' renders to
  something like #101-tier-1--bit-identical-differential-test, with
  the em-dash collapsed). A plain file link + named section is more
  robust to small heading edits than an anchor.

### Honest limitations and scope cuts

- **Policy is on paper; enforcement is informal until M2.**
  Reviewers must self-police the three-files-together rule (§3).
  Realistic, given M0 has no CI.

- **No enforcement of the §2 independence rule beyond review.**
  A reference Cargo.toml that, say, path-depends on nucleus/compiler
  would compile and produce 'correct' bytes silently. The only
  defence is a reviewer reading Cargo.toml. TASK-0076 can extend its
  scope to grep reference/Cargo.toml files for forbidden path deps —
  noted in that task description's intent but not formally required.

- **No coverage of the floating-point determinism policy beyond
  bullet points in §5.** Punted to TASK-0060 per the task brief.
  The §5 bullets are enough to keep an integer-only M0–M1 example
  set honest; nothing in the planned M0–M1 example list (1: add,
  2: add-split, 3: reduction) needs FP to be deterministic.

- **No fixtures or template provided.** The policy describes the
  shape of examples/NN-name/reference/ but does not commit a
  template or skeleton crate. First real reference impl (lands with
  example 1 at M1, TASK-0010 or similar) establishes the template
  by example.

- **No markdown linter is yet adopted in the repo.** AC #4 calls for
  'lints clean under any markdown linter we adopt' — vacuously true
  today. Spot-checked render with pandoc (pandoc OK on the file).
  A real markdownlint config is project-wide tooling, out of scope
  for this task.

### AC verification

- AC #1 (file layout, audit requirements, regeneration command):
  covered in §1, §3, §4 respectively.
- AC #2 (reference impls are NOT Nuc-compiled — independent):
  covered in §2 as a hard rule with explicit forbidden dependencies.
- AC #3 (algorithm semantics change → reference and reference.bin
  updated in same commit): covered in §3.
- AC #4 (file lints clean under any markdown linter we adopt):
  vacuously satisfied — no linter adopted yet. Pandoc renders the
  file without errors as a sanity check.
- AC #5 (implementation notes record design questions): these notes.
- AC #6 (implementation notes record honest limitations): above.

### Follow-up tasks filed

- TASK-0076: CI hook to verify reference.bin freshness (M2 gate).
- TASK-0077: Tooling to regenerate every reference.bin in one
  command.
- TASK-0060: FP determinism policy — referenced by §5 of the doc;
  not filed in this task (the task brief implied it pre-exists or
  will be filed elsewhere).

### Commits

- 9dd6e72  docs(M0): add reference-impl provenance policy (TASK-0004)
- 45ae9ce  docs(M0): correct follow-up task IDs in reference-impl
            policy (TASK-0004)
<!-- SECTION:NOTES:END -->
