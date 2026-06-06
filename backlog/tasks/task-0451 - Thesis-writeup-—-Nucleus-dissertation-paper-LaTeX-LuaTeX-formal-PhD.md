---
id: TASK-0451
title: 'Thesis writeup — Nucleus dissertation (paper/, LaTeX/LuaTeX, formal PhD)'
status: Done
assignee: []
created_date: '2026-06-05 19:57'
updated_date: '2026-06-06 02:10'
labels:
  - thesis
  - paper
  - latex
  - epic
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
EPIC. Produce a formal PhD dissertation under paper/ (LaTeX/LuaTeX, own nix flake + justfile) presenting Nucleus: a deterministic algorithm/schedule co-compiler with first-class IO semantics and a single Petri-net IR carrying a per-build boundedness/deadlock/conflict-free soundness gate. SHARED BRIEF for all children: (1) The thesis is a COHESIVE STANDALONE document — it MUST NOT reference internal TASK-NNNN ids or 'PRD section X' paragraphs; write it so a PL/FM examiner can read it cold. (2) Cite real literature via BibTeX (Halide/TVM/Tiramisu/Exo; MPI/OpenMP/Embassy; polyhedral/isl; Petri-net & dataflow scheduling; the author's 2013 v1). (3) Be HONEST: advantages AND shortcomings; state the affine/static/single-assignment/integer-only restriction class up front. (4) Plenty of TikZ figures. NARRATIVE: Problem = porting a parallel algorithm across CPU threads -> MPI cluster -> embedded MCU today means rewriting it because decomposition + IO semantics are hand-written per platform; IO is a second-class citizen and decomposition (not the algorithm) is the expensive part. Contribution = strict algorithm/schedule separation (decomposition/IO/target are first-class schedule directives the algorithm never sees), unified by a Petri-net IR whose boundedness+deadlock-freedom are a COMPILE-TIME gate, with the genericity claim made FALSIFIABLE by a cross-backend bit-identical differential test (one algorithm x many schedules x many backends -> byte-identical output). Realization = Rust pipeline (parse -> typed algorithm IR -> link -> ACFG -> transforms -> transfer/halo inference -> Petri-net IR -> soundness gate -> per-worker projection -> presentation codegen) over 10 backends / 3 tiers. Validation = tier-1 bit-identical matrix as the falsification rig + soundness gate + negative falsifiers; MPI value-correctness; embedded multi-MCU byte-exact Renode co-sim. ARTIFACT INVENTORY: 27 examples (nuc-nucleus/examples), 10 backends (7 CPU: pthreads-sync/async, mp-tcp-bufsync/event/poll, mp-uds-event, openmp-rs; 2 MPI: mpi-blocking/nonblocking; 1 embedded-pattern). Content source-of-truth (READ, do not cite): nuc-nucleus/PRD.md + the examples + the backends. CHAPTERS: 1 Introduction; 2 Background; 3 Prior art & literature; 4 Problem statement & solution space; 5 The Nuc language; 6 Architecture & methodology; 7 Backends & the target ladder; 8 Validation; 9 Results; 10 Discussion; 11 Future work & open problems; 12 Conclusion; appendices A example catalogue / B grammar / C capability matrix / D reproducibility. Children: C1 scaffold+outline, C2 intro/background/prior-art, C3 core (language+architecture+backends), C4 validation+results, C5 discussion+future+conclusion+appendices, C6 figures (TikZ), C7 review accuracy+references, C8 review prose+visual+peer+camera-ready, C9 (LOW) quantitative measurements addendum. Per-task: phase3 discipline (write -> mandatory independent review gate using the paper-* read-only agents). Full plan: ~/.claude/plans/prancy-discovering-tide.md
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
EPIC CLOSED — thesis-writeup arc complete. Content children .01-.05 (scaffold + ch1-12 + appendices A-D), .06 (17 TikZ figures), .10 (deep compilation walkthrough ch7b) all landed; review waves .07 (accuracy + references, whole draft vs live codebase) and .08 (prose/clarity/consistency/density + visual figure pass + skeptical PL/FM peer review + camera-ready) both GO. Dissertation is content-complete, twice-reviewed, and submission-ready: nix develop --command just build exit 0, 114 pages (grew from 111 after .08 added the StreamIt/Futhark/Legion related-work section + combine-operator paragraph + jargon glosses), zero undefined refs/citations, biber clean, 0 LaTeX errors, overfull boxes 29->7 (all <=15pt). All thesis content committed (latest 029282a). .09 (quantitative-measurements addendum: compile-time/code-size/scaling) DELIBERATELY LEFT OPEN as an optional/deferred future task per explicit user instruction — the thesis frames it as future work (ch11 sec:fw-quant) and is fully defensible without it; do NOT auto-start it. HUMAN PRE-SUBMISSION CHECKLIST (7 items, recorded in .08 final summary): (1) mped2013nuc 2013-thesis metadata unverifiable online; (2) petri1962kommunikation institution form TH vs TU Darmstadt; (3) title page institution/department/supervisor; (4) declaration date/signature; (5) personalise acknowledgements (currently neutral placeholder); (6) Appendix D artefact URL/DOI + licence; (7) defence-prep answers for reference-oracle independence (W3) + why generative testing is future work (W4).
<!-- SECTION:NOTES:END -->
