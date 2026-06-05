---
id: TASK-0451
title: 'Thesis writeup — Nucleus dissertation (paper/, LaTeX/LuaTeX, formal PhD)'
status: To Do
assignee: []
created_date: '2026-06-05 19:57'
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
