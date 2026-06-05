---
id: TASK-0450
title: >-
  Split pre-existing mega-file embedded-pattern/src/multimcu.rs (1478 LoC; just
  ci RED)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-05 04:44'
updated_date: '2026-06-05 07:31'
labels:
  - compiler
  - hygiene
  - megafile
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
embedded-pattern/src/multimcu.rs is 1478 LoC, over the 1000-LoC just check-mega-files fence and NOT on the allow-list — a PRE-EXISTING RED (confirmed at the TASK-0343.01.01 baseline, unrelated to that work). just ci has been RED on this file independently of the cheap pre-commit subset (memory: feedback-cheap-subset-blind-to-structural-fences). Split along the module-level docstring seams (emit_bin / multimcu.resc generation / UART-hub shim / input-offset layout) into cohesive sub-modules, OR allow-list with a one-line rationale if it is a single coherent unit. Sibling of the mega-file split cluster TASK-0340.x / TASK-0383 / TASK-0435 / TASK-0437.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 src/multimcu.rs (1478 LoC) split into a multimcu/ directory (mod.rs + cohesive submodules along the docstring seams: transport plan, event-scan helpers, input-offset layout, control-sync guard, .resc generation, boot-order) — every resulting .rs file <1000 LoC (check-mega-files scans recursively)
- [ ] #2 PURE refactor, ZERO behavior change: all external paths crate::multimcu::{TransportPlan,WorkerPlan,compute_input_offsets,compute_boot_order,render_multimachine_resc,verify_control_sync_subsumed} preserved via re-exports; internal visibilities widened MINIMALLY (pub(super)/pub(crate)), not blanket pub; lib.rs 'mod multimcu;' unchanged
- [ ] #3 just check-mega-files goes RED -> GREEN; multimcu.rs is NOT added to the allow-list; no stale allow-list entry introduced (direction-B clean)
- [ ] #4 FULL just ci passes GREEN end-to-end (the headline proof) — observed, not claimed
- [ ] #5 default just e2e stays 455/392/0/63/0 with the multi-MCU emit byte-identical; just renode-multimcu 02-split-add split still byte-exact (multimcu.rs is the multi-MCU transport path)
- [ ] #6 cargo doc --no-deps clean (intra-doc-links [`crate::multimcu::X`] survive the move; the gate builds no docs so this must be checked manually)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementer cycle (mechanical split, ZERO behavior change). src/multimcu.rs (1478 LoC) -> src/multimcu/{mod.rs 79, plan.rs 310, scan.rs 215, input_offsets.rs 334, control_sync.rs 294, resc.rs 155, boot_order.rs 188} — every file <1000 LoC. External crate::multimcu::{TransportPlan,WorkerPlan,UsartSlot,compute_input_offsets,render_multimachine_resc,verify_control_sync_subsumed} preserved via pub(crate) re-exports in mod.rs; visibilities widened MINIMALLY (cross-module callees -> pub(super); compute_input_offsets -> pub(crate); SeqEndpoints + 2 fields -> pub(super)/pub). compute_boot_order kept module-private (was a private fn pre-split; only TransportPlan::build calls it via super::) — NOT widened, NOT re-exported; mod.rs docstring link re-pathed to [boot_order::compute_boot_order]. Code-only line diff vs HEAD: only the intended visibility-keyword lines differ; zero body changes. DOC-LINK TRAP HIT (feedback-visibility-tighten-doclink-trap): the split perturbed the rustdoc cross-crate doc-graph and AWOKE a latent dead-href in backend-common/src/render/fire.rs:36 ([KernelSig](nucleus_compiler::sidecar::KernelSig) explicit-path link rendered an unresolvable 404 href under --no-deps), failing just check-doc-links. HEAD was green; my split made it render. Root-cause fix: converted that one cross-crate explicit-path link to a backtick code span (the gate-prescribed remediation), same form already used in scan.rs. Verified: just check-mega-files RED->GREEN; FULL just ci exit 0; clean just e2e 455/392/0/63/0; just renode-multimcu 02-split-add split BYTE-EXACT (1024 bytes == reference.bin). cargo doc --no-deps: 0 NEW warnings vs pre-split baseline (3 pre-existing private_intra_doc_links unchanged).

ORCHESTRATOR REVIEW GATE (independent, parallel qa-test-runner + mped-architect) — GO/GO, complementary coverage. qa RE-RAN full just ci end-to-end = exit 0 GREEN (check+clippy+test+test-release+all check-* fences incl check-mega-files+e2e+4 determinism/negative arms); e2e 455/392/0/63/0 reproduced 2x non-flake; just renode-multimcu 02-split-add split BYTE-EXACT 1024 bytes == reference.bin; clippy clean -D warnings. architect STATICALLY proved body byte-equivalence (union of new submodule items == old 1478-LoC file, zero items added; only non-plumbing delta is a rustfmt trailing comma on has_effectful_load wrapped signature, body identical; no .resc/USART/shim string altered), external surface crate::multimcu::{TransportPlan,WorkerPlan,UsartSlot,compute_input_offsets,render_multimachine_resc,verify_control_sync_subsumed} preserved via pub(crate) re-exports in multimcu/mod.rs, minimal pub(super)/pub(crate) widening (no blanket pub), compute_boot_order correctly kept module-private (was private at base 27f3bb2; tests reference only TransportPlan), module headers accurate. New files all <1000 LoC: mod 79/plan 310/scan 215/input_offsets 334/control_sync 294/resc 155/boot_order 188. multimcu.rs NOT allow-listed (direction-B clean). fire.rs:36 one-line doc-link fix (latent dead [KernelSig] explicit-path href -> backtick code span, the gates own prescribed remediation) is pure-doc, benign; silent-sibling sweep of ](nucleus_compiler::/](crate:: explicit-path links CLEAN (check-doc-links GREEN). just ci RED->GREEN restored. Commit 337e77d. P3 (out of scope, pre-existing): non-gating private_intra_doc_links warnings in lib.rs/mpi-*/net_soundness.rs predate this commit.
<!-- SECTION:NOTES:END -->
