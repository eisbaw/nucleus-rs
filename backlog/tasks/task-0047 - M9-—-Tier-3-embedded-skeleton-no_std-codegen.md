---
id: TASK-0047
title: M9 — Tier 3 embedded skeleton (no_std codegen)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-17 23:08'
updated_date: '2026-05-28 11:50'
labels:
  - M9
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
First tier-3 milestone: embedded-pattern backend emitting no_std Rust against a stub shim trait. Compile-only acceptance — no hardware or simulator yet. PRD §11. Placeholder.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/embedded-pattern/ crate lands; emits no_std code.
- [ ] #2 Shim trait NucleusShim defined: methods for alloc-in-region, dma-push, dma-wait, irq-barrier.
- [ ] #3 Generated code compiles against a stub shim that does nothing (just satisfies the trait).
- [ ] #4 Test: 'cargo check --target thumbv7em-none-eabihf' succeeds for examples 1, 5 under M9 backend.
- [ ] #5 Implementation notes record design questions (e.g. shim trait shape; whether shims provide async or sync semantics).
- [ ] #6 Implementation notes record honest limitations (no DMA, no IRQ, no real timing; just compile-only).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Toolchain prereq satisfied by TASK-0062 (commit 9787412 / 2026-05-21): `nix develop .#embedded` provides thumbv7em-none-eabihf rust-std on the pinned 1.83.0 toolchain. AC#2 of TASK-0062 already verified a no_std hello-world cross-builds to ARM ELF inside that shell. Start this skeleton inside .#embedded, not the default shell.

== Implementation Plan (cycle start) ==
Goal: M9 generic `embedded-pattern` backend emitting a no_std LIB crate
against a do-nothing STUB `NucleusShim` trait; compile-only via
`cargo check --target thumbv7em-none-eabihf` (run under `.#embedded`).

Design crux (the std-bound kernel problem):
- tier-1 kernels.rs are std-bound (file I/O, Vec) and CANNOT compile no_std.
- The PURE kernel bodies (add, blur3) are no_std-clean by inspection.
- Resolution: the embedded backend does NOT copy kernels.rs. Instead it
  emits a SELF-CONTAINED no_std lib (lib.rs) that:
    * defines `NucleusShim` (alloc_in_region/dma_push/dma_wait/irq_barrier),
    * includes ONLY the pure kernel fn(s) verbatim (extracted from the
      source kernels.rs by reusing the algorithm's kernel signatures — the
      pure kernels are tiny; we re-emit them as `mod kernels` with no_std-
      clean bodies pulled from the source), and
    * emits `run<S: NucleusShim>(shim, ...)` lowering the EventList.
  Effectful Fires (load_*/save_*) map to shim hooks (input-fill / output-
  drain), NOT to the std kernel bodies.

Event-list lowering (single-worker ex1/ex5; events = Fire + Loop only,
no Push/Wait/Sync/Alloc/Free in naive):
- Whole-array output Fire with NO inputs (`a <-- load_input()`)  => shim
  input-fill hook (`shim.alloc_in_region`+`dma_wait` style no-op fill).
- Output-less Fire (`save_output(c)`)                            => shim
  output-drain hook.
- Indexed-output Fire inside a Loop (`c[i] <-- add(...)`)        => call
  the pure kernel, write into a fixed `[i32; N]` array.
- Event::Loop => Rust `for v in (lo)..(hi)` (reuse render_loop_bounds).
- Data arrays => fixed-size `[i32; N]` locals (N = product(dims) from
  sidecar.data_types). Alloc-free, no_std-clean.

Reuse: backend_common shared renderers (data_name, render_fire_args,
render_fire_output_assign, render_loop_bounds, rust_scalar_type, EmitError,
RenderCtx). The backend crate itself is std (runs on host); only the
EMITTED lib is no_std.

Deliverables:
1. backends/embedded-pattern/{Cargo.toml, capabilities.toml, src/lib.rs}
2. NucleusShim trait + stub shim in emitted lib.
3. driver dispatch arm + unknown-backend list + nucleus build --help.
4. nucleus/Cargo.toml workspace member.
5. justfile `check-embedded` recipe (runs under .#embedded; NOT in `just ci`).
6. capabilities.toml tier=3 surface accepting ex1/ex5 naive.

Gate: host (build/clippy/test/test-release/e2e must stay 280/246/0/34/0)
+ embedded cross-check (cargo check --target thumbv7em-none-eabihf green).
NOT adding embedded-pattern to e2e-matrix.toml backends list (runtime
differential is wrong for a compile-only no_std backend).

HONEST PARTIAL fallback: if ex5 (2D flatten) doesn't fit safely, land ex1
fully + file ex5 follow-up.

== Implementation complete (awaiting independent review gate) ==
STATUS: In Progress — all 6 ACs met + both gate surfaces green; left In
Progress for the orchestrator's independent review (per implementer
brief). DO NOT self-mark Done.

Files created:
- backends/embedded-pattern/Cargo.toml         (std host crate, no_std emit)
- backends/embedded-pattern/capabilities.toml  (tier=3, minimal ex1/ex5 surface)
- backends/embedded-pattern/src/lib.rs          (emit + event lowering)
- backends/embedded-pattern/src/skeleton.rs     (no_std lib + Cargo.toml templates + NucleusShim/StubShim source)
- backends/embedded-pattern/src/kernel_extract.rs (verbatim pure-fn extraction)
- backends/embedded-pattern/src/tests.rs        (ex1/ex5 emit-shape + multi-worker-reject tests)
Files modified:
- nucleus/Cargo.toml          (+ workspace member)
- nucleus/driver/Cargo.toml   (+ embedded-pattern dep)
- nucleus/driver/src/main.rs  (+ dispatch arm, + --help line, + unknown-backend list)
- justfile                    (+ check-embedded recipe; NOT in `just ci`)
- nucleus/Cargo.lock          (crate registration)
Tracker: TASK-0048/0049 forward-carry notes; TASK-0361 scope-limit follow-up filed.

NucleusShim trait (AC#2) FINAL shape (canonical: skeleton.rs NUCLEUS_SHIM_SRC):
  fn alloc_in_region(&mut self, region: usize, bytes: usize) -> *mut u8;
  fn dma_push(&mut self, chan: usize, src: *const u8, len: usize);
  fn dma_wait(&mut self, chan: usize);
  fn irq_barrier(&mut self, tag: u32);

Emit design:
- DATA -> fixed `[T; N]` no_std locals (N = product(sidecar.data_types[d].dims)); alloc-free.
- PURE kernel (called by an INDEXED-output Fire) -> extracted VERBATIM from
  kernels.rs into `mod kernels`; the indexed Fire lowers to a kernels::<k>(..)
  call writing the array slot (shared backend_common render_fire_args /
  render_fire_output_assign — identical index flatten to tier-1).
- EFFECTFUL load (top-level whole-array-output Fire, no inputs) ->
  shim.alloc_in_region + shim.dma_wait (region fill; stub no-op).
- EFFECTFUL save (top-level output-less Fire) -> shim.dma_push + shim.dma_wait
  (region drain; stub no-op).
- Event::Loop -> Rust `for` (shared render_loop_bounds).
- Push/Wait/Sync, Alloc/Free, block_tag, check_frame -> precise
  UnsupportedFeature rejections w/ forward links (none occur in naive single-worker).
Classification is STRUCTURAL (output.indices), NOT a purity lookup — the
Event/sidecar contract deliberately drops purity (KernelSig DIVERGENCE HAZARD).

capabilities.toml (AC chosen surface): tier=3, transport=embedded-dma,
notify=[barrier,blocking], supports_async=false, supports_buffer=false,
max_buffer=1, worker_classes=[default], memory_regions=[heap]. Accepts ex1/ex5
naive; the full IRQ+DMA+async surface (PRD §7.3) lands with the M10 shim.

AC#5 design questions recorded: (a) shim methods SYNCHRONOUS (dma_push
enqueue / dma_wait block) vs an async completion-future/callback — M10
decides once a concrete MCU shim exists (recorded in NUCLEUS_SHIM_SRC
docstring + skeleton.rs). (b) effectful-kernel-as-shim-hook mapping
(input-fill vs output-drain) documented in lib.rs.
AC#6 honest limits recorded: no DMA / no IRQ / no real timing (StubShim
no-ops); compile-only (no_std LIB, no panic_handler/entry/linker — M10's
job); irq_barrier defined but UNEXERCISED by ex1/ex5 (no Sync in naive);
generated lib computes on ZERO-FILLED inputs (input-fill hook is a no-op).

VERIFICATION (measured, not adjectives):
HOST GATE (default `nix develop`):
  just build       -> Finished, clean
  just clippy      -> Finished, clean (fixed 1 doc_lazy_continuation in kernel_extract.rs)
  just test        -> all crates ok; embedded-pattern 13/0 in-crate
  just test-release-> all crates ok
  just e2e         -> 280/246/0/34/0 (sample 1) AND 280/246/0/34/0 (sample 2) — baseline PRESERVED, non-flaky
  just check-textual-replace-on-codegen / check-include-str-coverage / check-mega-files -> all OK
EMBEDDED CROSS-CHECK (`nix develop .#embedded --command just check-embedded`):
  rustc 1.83.0, thumbv7em-none-eabihf std present.
  ex1 (01-elementwise-add/naive): cargo check --target thumbv7em-none-eabihf -> Finished (PASS)
  ex5 (05-stencil/naive):         cargo check --target thumbv7em-none-eabihf -> Finished (PASS)
  Recipe overall: "OK: embedded-pattern no_std lib cross-compiles for examples 1 + 5".

Per-AC status: #1 MET (crate lands, emits no_std). #2 MET (4-method trait).
#3 MET (compiles vs StubShim). #4 MET (ex1 AND ex5 cargo-check-green for
thumbv7em-none-eabihf). #5 MET (design questions recorded). #6 MET (honest
limits recorded). NO examples deferred — both ex1 and ex5 landed (the
2D-flatten case fit the same structural pattern cleanly).

=== Cycle 236b review-gate closure (orchestrator) ===
Parallel read-only review gate on 7382e28:
- qa-test-runner: GO. Host gate green (build; clippy clean incl. no doc_lazy_continuation; test 1064/0/3; test-release 1063/0/3 — the -1 is the expected debug_assert #[should_panic] divergence). e2e 280/246/0/34/0 stable 2 runs. Embedded cross-check (just check-embedded under .#embedded, rustc 1.83.0 + thumbv7em-none-eabihf): BOTH ex1 + ex5 cargo check --target reach Finished. Structural isolation correct: embedded-pattern 0x in e2e-matrix.toml, NOT in just ci, thumbv7 check confined to .#embedded. Backend registered (dispatch arm + --help + unknown-backend list). The TASK-0012 aggregate-typed-I/O advisory warnings are pre-existing (shared contract.rs), not from this backend.
- mped-architect (read-only): GO. Pure-vs-effectful structural classification (by output.indices) verified SOUND on both examples' lowered EventLists; the whole-array-pure-compute-with-inputs case is really rejected as typed UnsupportedFeature (lib.rs:511), not mislowered. No panic-not-diagnostic (all unsupported shapes typed EmitError; the only .expect are in tests; writeln!().ok() is the established infallible-String convention). Doc-lies CLEAN (6 spot-checks incl. irq_barrier-unexercised, zero-filled-inputs, capability surface). Scope honest (tests non-vacuous, multi-worker reject bites, all 6 ACs genuinely met, nothing AC-gamed). Registration complete. Forward-carries to TASK-0048/0049 accurate.

P2-1 FOLDED IN-CYCLE (feedback-implementer-disclosure-mechanism-wrong recurrence): kernel_extract.rs docstring claimed the brace-matcher uniformly returns None->ContractGap on miscount; actually a stray CLOSING brace in a string/char/comment returns Some(TRUNCATED) caught only at downstream cargo check, while stray-open/genuine-imbalance returns None. Docstring corrected to state both directions; TASK-0361 item 2 note appended. Does NOT affect M9 (add/blur3 have no literal braces). P3-1 (scalar save-arg .as_ptr()) appended to TASK-0361. Re-ran post-fix: build OK, clippy clean, embedded-pattern 13/0.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
M9 LANDED: new tier-3 backend nucleus/backends/embedded-pattern emits compile-only no_std Rust against a NucleusShim stub trait (alloc_in_region/dma_push/dma_wait/irq_barrier). Data -> fixed [T;N] no_std arrays; pure compute kernels (add/blur3) extracted verbatim into a no_std lib; effectful I/O kernels -> shim hooks; Event::Loop -> for-loop; Push/Wait/Sync/multi-worker -> typed UnsupportedFeature rejects (M10/M11 forward-linked). Acceptance: just check-embedded runs cargo check --target thumbv7em-none-eabihf on ex1+ex5 under .#embedded — both Finished. no_std LIB (not bin) so no panic_handler/entry/linker (that's M10). Capability surface tier=3 minimal (accepts ex1/ex5 naive; full IRQ+DMA+async lands with M10 shim). Registered in workspace/driver/--help/unknown-list; correctly absent from e2e-matrix + just ci. Host e2e baseline 280/246/0/34/0 preserved. Both reviews GO. AC#1-6 met. Commit 7382e28 + cycle-236b doc fold-back. Scope limits filed TASK-0361.
<!-- SECTION:FINAL_SUMMARY:END -->
