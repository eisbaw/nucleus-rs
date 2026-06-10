# Nucleus v2 task runner. See nuc-nucleus/PRD.md §12.3.
#
# Recipes invoke cargo against the workspace at nucleus/. The chosen
# convention is `cd nucleus && cargo ...` rather than threading
# `--manifest-path nucleus/Cargo.toml` through every recipe -- shorter
# to read, and the cwd is the natural project root for a developer who
# wants to drop into the workspace by hand. See TASK-0003 notes.
#
# Anti-bloat rule (PRD §12.3): no example/schedule/backend-specific
# recipes. Such queries are flags on the `e2e` binary, not new recipes.

# Shared by check-reference-independence (positive) and its -negative arm:
# the rg pattern for an `include!`/`include_str!`/`include_bytes!` whose
# path reaches OUTSIDE the reference crate (a `..` segment) — the
# generated-source common-mode vector the Cargo.toml scanner cannot see.
# Defined ONCE so the two arms cannot silently drift (a narrowing edit to
# one would otherwise escape the other; memory: feedback-silent-sibling-defect).
ref_include_pat := 'include(_str|_bytes)?!\s*\(\s*"[^"]*\.\.'

# Default: list available recipes.
default:
    @just --list

# Build all crates in the workspace.
build:
    cd nucleus && cargo build --workspace

# Run unit tests (dev profile — debug_asserts active).
test:
    cd nucleus && cargo test --workspace

# Run unit tests under --release. debug_assert! is stripped in
# release, so any `#[should_panic]` test that pins a debug_assert
# bite (or any other code path conditioned on debug_assertions) will
# diverge between profiles. Wired into `just ci` after `just test` so
# this skew is gate-visible (TASK-0291). Cost: a second cargo test
# build of the workspace.
test-release:
    cd nucleus && cargo test --workspace --release

# Fast type-check without codegen.
check:
    cd nucleus && cargo check --workspace

# Apply rustfmt.
fmt:
    cd nucleus && cargo fmt --all

# Check rustfmt without writing. Returns non-zero on drift so a
# developer can verify before committing — closes the asymmetric-
# strict gap with `just clippy` (TASK-0256). NOT wired into `just ci`
# per TASK-0069 closure (fmt is dev-side informational; clippy is the
# CI hard gate).
fmt-check:
    cd nucleus && cargo fmt --all -- --check

# Lint. Warnings are errors. --all-targets so test/bin-target lint
# rot is gate-visible and cannot silently re-accumulate behind a
# green `just ci` (decision-0002, TASK-0186).
clippy:
    cd nucleus && cargo clippy --workspace --all-targets -- -D warnings

# Full end-to-end differential matrix (every required cell, all
# milestones). This is the gate `just ci` runs and is UNCHANGED by
# the milestone work: bare `just e2e` = the full matrix.
e2e:
    cd nucleus && cargo run --release --bin nucleus-e2e

# Generative property-based cross-backend differential fuzzer
# (TASK-0453.01): synthesises k affine single-assignment integer
# programs from a seed and asserts byte-identity across all 7 tier-1
# backends + an in-process reference. Args are POSITIONAL: `just
# diff-fuzz 1 8` (seed=1, k=8); defaults seed=1 k=8.
diff-fuzz seed="1" k="8":
    cd nucleus && cargo run --release --bin diff-fuzz -- --seed {{seed}} --k {{k}}

# Per-milestone tier of the e2e matrix (TASK-0167). CUMULATIVE: `just
# e2e-milestone M3` runs the M1 ∪ M2 ∪ M3 required cells; `M1` runs
# only the M1 tier. A PR targeting a milestone branch runs that
# milestone's tier via this recipe (see .github/workflows/ci.yml
# matrix). Single source of truth: CI calls `nix develop -c just
# e2e-milestone M<k>`, identical to what a developer runs here. The
# anti-bloat rule (no example/schedule-specific recipes) is respected
# — milestone is a first-class gate dimension (PRD §11), not a one-off.
e2e-milestone M:
    cd nucleus && cargo run --release --bin nucleus-e2e -- --milestone {{M}}

# Tier-2 (M7) MPI differential e2e — the mpi-blocking arm of the
# cross-backend matrix, now COUNTED (TASK-0444). Enters the `.#mpi` dev
# shell (OpenMPI + rsmpi build deps + localhost mpiexec) and runs the
# nucleus-e2e harness with `--with-mpi`, which scopes the run to the
# `mpi_backends` tier (mpi-blocking) declared in e2e-matrix.toml INSTEAD
# of the default tier-1 `backends`. Each cell generates the SPMD MPI
# project, cross-builds it, launches it under `mpiexec -n N` (N = the
# schedule's used-worker count baked into run.sh — all ranks live), and
# byte-diffs output.bin against the example's reference.bin — the SAME
# oracle the tier-1 differential uses, so this is a real apples-to-
# apples cross-backend check. Self-contained: it enters `.#mpi` itself,
# so it runs from the default shell:  just e2e-mpi
#
# DELIBERATELY NOT in `just ci` / bare `just e2e`: the default shell has
# no MPI (TASK-0068 tiered-shell design). The harness HARD-FAILS if
# mpiexec is absent (no silent skip). The focused out-of-default-matrix
# sibling of `renode-multimcu-gate` (the tier-3 embedded gate); the
# mpi-blocking BACKEND crate itself is still built by `just ci` (a
# normal std workspace member emitting strings). This is the COUNTED
# matrix companion to the value-correctness gate `just check-mpi`.
e2e-mpi:
    nix develop .#mpi --command bash -c "cd nucleus && cargo run --release --bin nucleus-e2e -- --with-mpi"

# TASK-0446: the STANDING WIRED-PATH negative arm for the --with-mpi
# tier coverage guard — the mpi-tier sibling of
# `required-coverage-check-negative` (the tier-1 arm wired into `just
# ci`). The tier-1 arm anchors its synthetic typo'd `[[required]]` cell
# on a tier-1 backend and runs WITHOUT --with-mpi, so it does NOT
# exercise the mpi-tier branch of `run_inner`'s required-coverage
# hard-fail; the pure-function unit tests cover the LOGIC but not the
# WIRED call. A refactor could sever the mpi-tier hard-fail with unit
# tests + `just e2e-mpi` green. This recipe closes that standing-bite
# gap: under `.#mpi`, it runs the harness with --with-mpi AND
# NUC_REQUIRED_COVERAGE_NEGATIVE=1, which (TASK-0446 tier-aware
# injection) appends ONE synthetic mpi-tier `[[required]]` cell whose
# sentinel schedule cannot match any *.sched.nuc file. The wired
# mpi-tier `required_coverage_gaps` then yields a gap, `run_inner`
# returns Err, and the harness exits non-zero — SUCCEEDS iff the harness
# correctly FAILS. The gap check fires BEFORE any cell is built, so this
# is CHEAP (no rsmpi cross-builds; only needs the mpiexec startup probe
# the `.#mpi` shell satisfies). Same TASK-0188 belt-and-suspenders
# contract as the tier-1 arm: asserts NUC_REQUIRED_COVERAGE_GAP_DETECTED
# is present AND >=1 IN ADDITION to the exit-code inversion. DELIBERATELY
# NOT in `just ci` (needs `.#mpi`, same out-of-default-ci rule as
# `e2e-mpi` / `check-mpi`); run it alongside `just e2e-mpi`. A bundling
# meta-recipe that runs the whole mpi gate (incl. both falsifiers) in
# one command is tracked as TASK-0447.
# NB the body is wrapped in `nix develop .#mpi --command bash -c '...'`,
# so it must contain NO interior apostrophes (hence `trap "rm -f $out"
# EXIT` with `$out` expanded at trap-set time, not the tier-1 sibling's
# single-quoted `trap 'rm -f "$out"' EXIT`). Both are correct; the
# divergence is forced by the outer single-quote wrapping, not style.
required-coverage-check-negative-mpi:
    nix develop .#mpi --command bash -c 'cd nucleus && out=$(mktemp) && trap "rm -f $out" EXIT && { if NUC_REQUIRED_COVERAGE_NEGATIVE=1 cargo run --release --bin nucleus-e2e -- --with-mpi >"$out" 2>&1; then bit=0; else bit=1; fi; }; cat "$out"; n=$(grep -oE "^NUC_REQUIRED_COVERAGE_GAP_DETECTED=[0-9]+" "$out" | tail -n1 | cut -d= -f2); if [ -z "$n" ]; then echo "FAIL: NUC_REQUIRED_COVERAGE_GAP_DETECTED signal MISSING under --with-mpi — cannot prove the mpi-tier required-coverage guard detected the injected sentinel cell (TASK-0188/0446 harness contract broken)"; exit 1; fi; if [ "$n" -lt 1 ]; then echo "FAIL: NUC_REQUIRED_COVERAGE_GAP_DETECTED=$n — the mpi-tier required-coverage guard detected NO injection-attributable gap (TASK-0446)"; exit 1; fi; if [ "$bit" -eq 0 ]; then echo "FAIL: mpi-tier required-coverage guard did NOT exit non-zero on the injected sentinel mpi required cell (TASK-0446 wired --with-mpi path silently neutered)"; exit 1; else echo "OK: mpi-tier required-coverage guard correctly bit on the injected sentinel mpi required cell (--with-mpi)"; fi'

# TASK-0447: one-command tier-2 MPI gate. Bundles the previously-manual
# MPI verification sequence — positive acceptance (`check-mpi` M7 +
# `check-mpi-nonblocking` M8 + `e2e-mpi` cross-backend differential) AND
# BOTH required-coverage negative falsifiers (the tier-1
# `required-coverage-check-negative` + the mpi-tier
# `required-coverage-check-negative-mpi`) — into ONE fail-loud `just`
# prerequisite chain, so "run the MPI tier" is a single reproducible
# command that carries its own negative arms. Closes the TASK-0446 P2
# standing-bite DISCOVERABILITY gap: the mpi-tier coverage falsifier is
# out-of-default-`ci` (needs `.#mpi`), so a human could run `e2e-mpi`
# and forget the falsifier; bundling it here makes it un-forgettable.
# Any prerequisite exiting non-zero ABORTS the gate (just prerequisite
# semantics) — so `mpi-gate` FAILS if an mpi cell is silently skipped
# (caught by `e2e-mpi` / the mpi-tier coverage falsifier) or either
# required-coverage guard is bypassed (caught by the two falsifiers).
# Each prerequisite is self-contained: the four mpi arms self-enter
# `.#mpi`; `required-coverage-check-negative` is the tier-1 arm (default
# shell, also wired into `just ci`). The two CHEAP negative falsifiers
# run FIRST so a severed coverage guard fails fast BEFORE the heavy
# rsmpi cross-builds. Invoke from the default shell:  just mpi-gate
# DELIBERATELY NOT in `just ci` (needs `.#mpi`, same out-of-default-ci
# rule as every arm). Mirrors the tier-3 `renode-multimcu-gate` bundling
# pattern. If an mpi CI lane is ever added it MUST invoke this recipe so
# both required-coverage negative arms run (TASK-0446 P2 residual).
mpi-gate: required-coverage-check-negative required-coverage-check-negative-mpi check-mpi check-mpi-nonblocking e2e-mpi
    @echo "OK: tier-2 M7/M8 MPI gate PASSED — both required-coverage negative falsifiers bit (tier-1 + mpi-tier) AND check-mpi + check-mpi-nonblocking + e2e-mpi positive acceptance all GREEN (TASK-0447)."

# Verify PRD §1 / §10.1: same source + same backend = byte-identical
# emitted code. Builds every cell twice and diffs the generated files.
# TASK-0033.
determinism-check:
    cd nucleus && cargo run --release --bin nucleus-e2e -- --check-determinism

# Prove the determinism check actually BITES (TASK-0145 / TASK-0033
# AC#4 negative arm). Builds the driver with the
# `nondeterministic-test` feature so slot declarations emit in
# process-randomised HashMap order; the two determinism builds then
# differ. SUCCEEDS iff `--check-determinism` correctly FAILS (non-zero
# exit, naming the offending file). A green `determinism-check` is
# only meaningful because this one is too.
# TASK-0188: the safety invariant ("the falsifier actually perturbed
# >=1 tree") no longer rests SOLELY on the exit-code inversion below.
# The harness also emits an explicit machine-checkable stdout line
# `NUC_NONDET_PERTURBED_CELLS=<n>`; this recipe asserts that line is
# present AND n>=1 IN ADDITION to the exit-code check. If a future
# refactor drops the inversion, the count assertion still fails LOUD
# instead of silently re-neutering the falsifier. Combined output is
# captured to a temp file so the exit code still drives the `if`
# (cargo's status, NOT tee/grep's), then echoed so it stays visible.
determinism-check-negative:
    cd nucleus && out=$(mktemp) && trap 'rm -f "$out"' EXIT && { if NUC_NONDET_TEST=1 cargo run --release --bin nucleus-e2e -- --check-determinism >"$out" 2>&1; then bit=0; else bit=1; fi; }; cat "$out"; n=$(grep -oE '^NUC_NONDET_PERTURBED_CELLS=[0-9]+' "$out" | tail -n1 | cut -d= -f2); if [ -z "$n" ]; then echo "FAIL: NUC_NONDET_PERTURBED_CELLS signal MISSING — cannot prove the falsifier perturbed anything (TASK-0188; harness/recipe contract broken)"; exit 1; fi; if [ "$n" -lt 1 ]; then echo "FAIL: NUC_NONDET_PERTURBED_CELLS=$n — the determinism falsifier perturbed NOTHING (TASK-0188)"; exit 1; fi; if [ "$bit" -eq 0 ]; then echo "FAIL: determinism check did NOT detect injected nondeterminism"; exit 1; else echo "OK: determinism check correctly bit on injected nondeterminism"; fi

# Prove the CROSS-BACKEND e2e differential actually BITES (TASK-0178 /
# TASK-0041 AC#5 negative arm). Runs the full e2e matrix with
# NUC_XBACKEND_NEGATIVE=1, which deterministically corrupts the
# mp-tcp-EXCLUSIVE wire encode (enc_vec, copied into generated
# multi-process projects only — pthreads-sync emits no wire). A
# multi-process mp-tcp cell (02-split-add/split) then diverges from
# the hand-written reference.bin oracle while every pthreads-sync
# cell stays byte-identical: the harness exits non-zero with
# required-fail>0. SUCCEEDS iff the harness correctly FAILS — a green
# `e2e` differential is only meaningful because this one is too.
# Mirrors `determinism-check-negative`; deterministic (fixed source
# rewrite, no RNG/PID/clock) so it is non-flaky. Gate OFF by default:
# bare `e2e` is unchanged.
# TASK-0188: mirrors determinism-check-negative's hardening. The
# harness emits `NUC_XBACKEND_CORRUPTED_DETECTED=<n>` where n = required
# mp-tcp-bufsync cells that Failed at the Diff phase (corruption present
# AND the cross-backend differential genuinely detected it — NOT any
# unrelated required-fail). This recipe asserts that line present AND
# n>=1 IN ADDITION to the exit-code inversion, so a recipe refactor
# dropping the inversion fails LOUD rather than silently re-neutering.
xbackend-check-negative:
    cd nucleus && out=$(mktemp) && trap 'rm -f "$out"' EXIT && { if NUC_XBACKEND_NEGATIVE=1 cargo run --release --bin nucleus-e2e >"$out" 2>&1; then bit=0; else bit=1; fi; }; cat "$out"; n=$(grep -oE '^NUC_XBACKEND_CORRUPTED_DETECTED=[0-9]+' "$out" | tail -n1 | cut -d= -f2); if [ -z "$n" ]; then echo "FAIL: NUC_XBACKEND_CORRUPTED_DETECTED signal MISSING — cannot prove the differential detected injected corruption (TASK-0188; harness/recipe contract broken)"; exit 1; fi; if [ "$n" -lt 1 ]; then echo "FAIL: NUC_XBACKEND_CORRUPTED_DETECTED=$n — the cross-backend differential detected NO injected mp-tcp corruption (TASK-0188)"; exit 1; fi; if [ "$bit" -eq 0 ]; then echo "FAIL: cross-backend differential did NOT detect injected mp-tcp corruption"; exit 1; else echo "OK: cross-backend differential correctly bit on injected mp-tcp corruption"; fi

# Prove the [[required]]-coverage guard actually BITES on the WIRED
# `run_inner` path (TASK-0168 / TASK-0163 AC#3 negative arm). TASK-0163
# added `required_coverage_gaps`, but its 5 unit tests cover the pure
# function in isolation — none of them prove that `run_inner` still
# returns Err on a non-empty gap set. A future refactor could drop the
# `if !gaps.is_empty() { return Err }` wiring, all 5 unit tests would
# stay green, and `just ci` would not catch the silent re-introduction
# of the TASK-0163 false-negative. This recipe closes that gap by
# running the full harness with NUC_REQUIRED_COVERAGE_NEGATIVE=1, which
# deterministically appends ONE synthetic [[required]] entry whose
# schedule cannot match any discovered *.sched.nuc file. The wired
# `required_coverage_gaps` then yields a gap, `run_inner` returns Err
# naming the synthetic triple, and the harness exits non-zero. SUCCEEDS
# iff the harness correctly FAILS — a green `e2e` is only meaningful
# because this one is too. Mirrors `determinism-check-negative` /
# `xbackend-check-negative` exactly (env-flag seam, no committed broken
# manifest — AC#2; loud stderr WARNING at injection time;
# deterministic, non-flaky).
# TASK-0188 belt-and-suspenders contract: the harness emits
# `NUC_REQUIRED_COVERAGE_GAP_DETECTED=<n>` where n = gaps attributable
# to the injection (filtered by the sentinel schedule). This recipe
# asserts the line is present AND n>=1 IN ADDITION to the exit-code
# inversion, so a recipe refactor dropping the inversion fails LOUD
# rather than silently re-neutering the falsifier.
required-coverage-check-negative:
    cd nucleus && out=$(mktemp) && trap 'rm -f "$out"' EXIT && { if NUC_REQUIRED_COVERAGE_NEGATIVE=1 cargo run --release --bin nucleus-e2e >"$out" 2>&1; then bit=0; else bit=1; fi; }; cat "$out"; n=$(grep -oE '^NUC_REQUIRED_COVERAGE_GAP_DETECTED=[0-9]+' "$out" | tail -n1 | cut -d= -f2); if [ -z "$n" ]; then echo "FAIL: NUC_REQUIRED_COVERAGE_GAP_DETECTED signal MISSING — cannot prove the required-coverage guard detected the injected typo (TASK-0188; harness/recipe contract broken)"; exit 1; fi; if [ "$n" -lt 1 ]; then echo "FAIL: NUC_REQUIRED_COVERAGE_GAP_DETECTED=$n — the required-coverage guard detected NO injection-attributable gap (TASK-0188)"; exit 1; fi; if [ "$bit" -eq 0 ]; then echo "FAIL: required-coverage guard did NOT exit non-zero on injected typo'd required cell (TASK-0168 wired path silently re-neutered)"; exit 1; else echo "OK: required-coverage guard correctly bit on injected typo'd required cell"; fi

# TASK-0174 (B) / TASK-0038 AC#5: the REAL end-to-end reproduction of
# "an OS cap below the schedule requirement makes run.sh fail LOUD".
# Tries `unshare -Urn` + in-netns `sysctl -w net.core.wmem_max=4096`,
# then runs a generated mp-tcp-bufsync run.sh under that lowered cap
# and asserts it exits non-zero with the wire::apply_sock_buf clear
# error naming the OS cap. HONEST-SKIP: net.core.wmem_max is
# init_user_ns-owned (NOT per-netns namespaced), so the sandbox's
# user+net namespace cannot lower it — the script then SKIPs with a
# precise reason (exit 0, informational) rather than fake AC#5. It
# RUNS the genuine reproduction wherever the sysctl write is permitted
# (host root / privileged container / userns-sysctl-enabled CI). The
# fail-loud DECISION itself is proven deterministically and
# unconditionally by the mp-tcp-common pure-logic unit tests
# (check_effective_sock_buf) that `just test` runs — this recipe is
# the end-to-end arm. NOT wired into `just ci` because it skips in the
# sandbox; run it on a privileged host/CI to genuinely close AC#5.
sockbuf-cap-check:
    bash nuc-nucleus/sockbuf-cap-check.sh

# Aggregate verification gate. Single source of truth shared by CI
# (.github/workflows/ci.yml) and local developers: "what CI does" ==
# "what you can run here". Runs the full tier-1 gate in dependency
# order. just aborts a recipe on the first line whose command exits
# non-zero, so this is fail-fast with no silent continuation.
#
# Order rationale: cheapest/most-localised failures first (check →
# clippy → test) before the slower integration gates (e2e →
# determinism), so a broken build surfaces in seconds not minutes.
#
# NOTE (TASK-0162): the `clippy` step below intentionally calls the
# `clippy` recipe as the justfile defines it TODAY (workspace, not
# --all-targets). TASK-0162 will tighten that recipe to
# `--all-targets` after the pre-existing test-lint debt is paid; when
# it does, this aggregate inherits the stricter gate for free because
# it delegates rather than re-implements. Do not inline a different
# clippy invocation here.
ci:
    just check
    just clippy
    just test
    just test-release
    just check-textual-replace-on-codegen
    just check-include-str-coverage
    just check-reference-independence
    just check-reference-independence-negative
    just check-narrative-doc-lie
    just check-doc-citation-staleness
    just check-doc-citation-staleness-bare
    just check-doc-test-name-staleness
    just check-doc-cell-path-staleness
    just check-mega-files
    just check-readme-counts
    just check-doc-links
    just e2e
    just determinism-check
    just determinism-check-negative
    just xbackend-check-negative
    just required-coverage-check-negative

# Concurrency stress for the mp-tcp-bufsync port-handshake (TASK-0176
# AC#2). Runs `nucleus-e2e --backend mp-tcp-bufsync` 20 times in
# parallel under one shell and fails LOUD on any flaky failure. The
# rendezvous-file handshake (TASK-0176) eliminated the close-then-
# rebind TOCTOU window the old `__nuc_pick_port` helper had; this
# recipe is the high-confidence proof that the fix actually keeps
# concurrent mp-tcp cells flake-free under load (8 sequential samples
# was insufficient evidence per AC#2).
#
# Wired into a NIGHTLY scheduled CI job at .github/workflows/ci.yml's
# `port-stress` job (cron: '0 4 * * *'), gated by event_name in
# {schedule, workflow_dispatch} so per-PR walltime is unaffected
# (TASK-0252, closing TASK-0176 AC#2 as a steady-state guarantee, not
# a one-shot reading). Per-push / per-PR runs still skip the recipe
# because 20× parallel `nucleus-e2e` is too heavy for the standard
# walltime budget. Run manually anytime after touching the port
# handshake (mp-tcp-bufsync emit, run.sh generation, wire::apply_sock_buf).
#
# Each e2e invocation uses a unique per-RUN run-id (pid+nanos,
# TASK-0182) so scratch dirs do not collide; the per-cell run.sh uses
# its own pid-suffixed rendezvous dir so the port-handshake files do
# not collide either. Invokes the prebuilt `target/release/nucleus-e2e`
# binary directly (NOT `cargo run`) so the 20 parallel invocations do
# not contend on the per-package cargo build lock at all — pre-build
# once below to make sure the binary exists. On failure dumps EVERY
# child log (not just `head -1`), so the failing invocation's stderr
# is always in the tail output.
port-stress-check N="20":
    cd nucleus && cargo build --release --bin nucleus-e2e --quiet
    cd nucleus && fail=0; for i in $(seq 1 {{N}}); do ./target/release/nucleus-e2e --backend mp-tcp-bufsync >/tmp/nuc-port-stress-$$-$i.log 2>&1 & done; for j in $(jobs -p); do wait "$j" || fail=$((fail+1)); done; if [ "$fail" -gt 0 ]; then echo "FAIL: $fail of {{N}} parallel mp-tcp-bufsync e2e runs failed (TASK-0176 AC#2). Dumping all {{N}} child logs:"; for log in /tmp/nuc-port-stress-$$-*.log; do echo "===== $log ====="; tail -40 "$log"; done; exit 1; fi; rm -f /tmp/nuc-port-stress-$$-*.log; echo "OK: {{N}}/{{N}} parallel mp-tcp-bufsync e2e runs passed (TASK-0176 AC#2)"

# Regenerate every example's reference.bin via its standalone
# reference impl (docs/reference-impl-policy.md §3, §4). Maintainers
# run this when a kernel body changes and references must move in
# lockstep with the algorithm. Glob-discovers
# `nuc-nucleus/examples/*/reference/Cargo.toml` so a new example with
# a reference impl is picked up automatically — no per-example recipe
# bloat (PRD §12.3 anti-bloat rule).
#
# Two regen shapes are handled: most examples take the existing
# committed input.bin via --in/--out; examples 04-prefix-sum and
# 06-separable-filter also support --gen-input. This recipe regenerates
# ONLY reference.bin and uses the existing input.bin — that is the
# common maintenance flow when a kernel changes. To also regenerate
# input.bin (e.g. changing the test fixture shape) run the per-example
# `--gen-input` command from the Cargo.toml's header comment manually.
#
# Fails LOUD on first non-zero exit. Does NOT commit results — review
# diffs and commit yourself (TASK-0077: 'human review step'). 14-
# hearing-aid currently has no reference impl (M11; TASK-0054) so the
# glob skips it naturally.
regen-references:
    @set -e; for cargo_toml in nuc-nucleus/examples/*/reference/Cargo.toml; do \
        ex_dir=$(dirname $(dirname "$cargo_toml")); \
        ex_name=$(basename "$ex_dir"); \
        echo "=== regenerating $ex_name/reference.bin ==="; \
        cargo run --release --manifest-path "$cargo_toml" -- \
            --in  "$ex_dir/input.bin" \
            --out "$ex_dir/reference.bin"; \
    done; \
    echo "OK: all reference.bin files regenerated. Review with 'git diff' before committing (TASK-0077)."

# Catch the textual-replace-on-codegen-string defect class
# (memory: feedback-textual-replace-codegen-unsafe). Concrete prior
# instance: TASK-0269 P1.1 (cycle 103) — `abs.replace(iv_name,
# "0_i64")` corrupted sibling identifiers (`{iv}__tile`, `{iv}_partial`)
# because they contain the iv as a substring. The rule: never `String::
# replace` on a rendered Rust expression; build derived expressions
# structurally.
#
# Current baseline (TASK-0289 cycle-114a-orchestrator-hardening):
# 4 occurrences in source, all inside `//` comments cross-referencing
# the filed defect. Zero actual code uses. This check rg's the
# compiler/backend source tree and fails if any non-comment `.replace(`
# survives.
#
# False-positive policy: a legitimate `String::replace` on a known-safe
# input (e.g. a path component, a constant key) should be annotated
# with the comment `// ALLOW textual replace: <reason>` on the same
# line; the check exempts those.
check-textual-replace-on-codegen:
    @echo "checking for textual .replace( on codegen strings..."
    @hits=$(rg -nH --type rust '\.replace\(' \
        nucleus/nucleus-compiler/src/ \
        nucleus/backend-common/src/ \
        nucleus/backends/*/src/ \
        | grep -v '^[^:]*:[^:]*:\s*//' \
        | grep -v 'ALLOW textual replace' \
        || true); \
    if [ -n "$hits" ]; then \
        echo "FAIL: unannotated String::replace in compiler/backend code (memory: feedback-textual-replace-codegen-unsafe):"; \
        echo "$hits"; \
        echo ""; \
        echo "Build the derived expression structurally, or annotate the line with '// ALLOW textual replace: <reason>'."; \
        exit 1; \
    fi; \
    echo "OK: no textual .replace on codegen strings."

# Catch the include_str! compile-coverage defect class (memory:
# feedback-include-str-compile-coverage). Concrete prior instance:
# any `pub const X_SRC: &str = include_str!("foo.rs")` where `foo.rs`
# is NOT also referenced by `mod foo;` or `include!("foo.rs")` in the
# same crate ships uncompiled source — a rename or syntax error in
# foo.rs is invisible until a downstream user includes the const into
# their build.
#
# Today (TASK-0289 cycle-114a-orchestrator-hardening): 2 sites in
# source, both with coverage — mp-tcp-event's `runtime_src.rs` via
# `mod runtime_src;`, and mp-tcp-common's `wire_runtime.rs` via
# `pub mod wire { include!("wire_runtime.rs") }`.
#
# POSIX-shell portability (TASK-0415; cf. the doc-citation fences):
# `just` runs `/bin/sh -cu`, so this recipe feeds a `mktemp` temp file
# into a `while read` loop rather than a bash `done < <(...)` process
# substitution (a bashism dash/ash/busybox-sh reject). `mktemp` +
# `trap "rm -f ..." EXIT` is POSIX-portable.
check-include-str-coverage:
    @echo "checking include_str! compile coverage..."
    @set -e; fail=0; \
    inc_f=$(mktemp); \
    trap "rm -f $inc_f" EXIT; \
    rg -nH --type rust 'include_str!' nucleus/nucleus-compiler/src/ nucleus/backend-common/src/ nucleus/backends/*/src/ nucleus/mp-tcp-common/src/ 2>/dev/null > $inc_f || true; \
    while IFS= read -r line; do \
        [ -n "$line" ] || continue; \
        file=$(echo "$line" | cut -d: -f1); \
        target=$(echo "$line" | grep -oE 'include_str!\([^)]*\)' | head -1 | sed -E 's/include_str!\("([^"]+)"\)/\1/'); \
        base=$(basename "$target" .rs); \
        crate_dir=$(dirname "$file"); \
        if ! rg -q "mod ${base}\b" "$crate_dir" && \
           ! rg -q "include!\(\"${target}\"\)" "$crate_dir"; then \
            echo "FAIL: $file include_str!(\"$target\") has no matching 'mod ${base};' or 'include!(\"$target\")' in $crate_dir"; \
            fail=1; \
        fi; \
    done < $inc_f; \
    if [ $fail -ne 0 ]; then \
        echo ""; \
        echo "(memory: feedback-include-str-compile-coverage — bare include_str! does not compile the file content; add 'mod <name>;' or 'include!(\"<path>\");' in the same crate so 'cargo test' compiles it)"; \
        exit 1; \
    fi; \
    echo "OK: every include_str! has compile coverage."

# Enforce reference-oracle CODE-independence (docs/reference-impl-policy.md
# §2) mechanically. The differential argument's credibility hinges on the
# hand-written reference being independent of the compiler middle-end: if a
# reference shared compiler/backend code (or a Nuc-generated file), a
# common-mode bug could corrupt the generated code AND the reference the
# same way, and the byte-identity differential could NOT see it (PRD §10.1
# "all backends wrong the same way"; thesis ch10 sec:disc-shortcomings
# "Agreement could be common-mode"). §2 is a HARD RULE, but until now it
# was enforced only by a reviewer checklist (policy §6/§7 explicitly: "Not
# a CI-enforced check at M0"). This fence makes it mechanical (TASK-0453.08
# P8 / defence W3). Two guards:
#   1. check-reference-independence.awk on every reference Cargo.toml —
#      forbids a `workspace =` parent-link, any `path =`/`git =` dependency
#      (the only vector to an UNPUBLISHED Nucleus workspace crate; a
#      crates.io reference dep like byteorder never needs one), and any
#      Nucleus crate by name. An empty `[workspace]` table (the isolation
#      mechanism) and a `[[bin]] path = "src/main.rs"` target are allowed.
#   2. an rg guard on reference/src for `include!`/`include_str!`/
#      `include_bytes!` reaching OUTSIDE the crate (a `..` path) — the
#      generated-source-include vector the Cargo.toml fence cannot see.
# Coverage self-check: asserts ≥25 manifests scanned so a glob breakage
# cannot make the fence vacuously pass (memory:
# feedback-coverage-audit-undercount-recurring). Negative arm proves it
# bites: check-reference-independence-negative.
check-reference-independence:
    @echo "checking reference-oracle code-independence (policy §2)..."
    @set -e; \
    manifests=$(ls nuc-nucleus/examples/*/reference/Cargo.toml 2>/dev/null); \
    n=$(echo "$manifests" | grep -c .); \
    if [ "$n" -lt 25 ]; then \
        echo "FAIL: only $n reference Cargo.toml found (expected >=25); glob broke — fence would be vacuous"; \
        exit 1; \
    fi; \
    if ! awk -f check-reference-independence.awk $manifests; then \
        echo ""; \
        echo "A reference impl declares a dependency that reaches into the Nucleus workspace."; \
        echo "References MUST be standalone (docs/reference-impl-policy.md §2) so the"; \
        echo "differential cannot be defeated by a shared-code common-mode bug."; \
        exit 1; \
    fi; \
    inc=$(rg -n '{{ref_include_pat}}' nuc-nucleus/examples/*/reference/src/ 2>/dev/null || true); \
    if [ -n "$inc" ]; then \
        echo "FAIL: a reference src includes a file OUTSIDE its crate (generated-source vector, policy §2):"; \
        echo "$inc"; \
        exit 1; \
    fi; \
    echo "OK: all $n reference oracles are code-independent of the compiler (policy §2)."

# Prove the reference-independence fence actually BITES (TASK-0453.08
# AC, mirrors the *-check-negative convention: no committed broken
# manifest — poisons a temp copy). Builds four deliberately-dependent
# manifests (a path-dep on nucleus-compiler, a `workspace =` parent-link,
# a backend crate by name, and a `[patch.crates-io]` source override
# redirecting an innocuous crates.io name to a Nucleus path) plus a src
# file that include!s outside the crate, and asserts the SAME scanner used
# by the positive arm rejects each. SUCCEEDS iff the fence FAILS on every
# violation — a green check-reference-independence is only meaningful
# because this one is too.
check-reference-independence-negative:
    @set -e; \
    td=$(mktemp -d); trap 'rm -rf "$td"' EXIT; \
    fail=0; \
    printf '[package]\nname="evil"\n[dependencies]\nnucleus-compiler = { path = "../../../nucleus/nucleus-compiler" }\n' > "$td/path.toml"; \
    printf '[package]\nname="evil"\nworkspace = "../../../nucleus"\n' > "$td/ws.toml"; \
    printf '[package]\nname="evil"\n[dependencies]\nmp-tcp-event = "0.1"\n' > "$td/name.toml"; \
    printf '[package]\nname="evil"\n[dependencies]\nbyteorder = "1"\n[patch.crates-io]\nbyteorder = { path = "../../../nucleus/backend-common" }\n' > "$td/patch.toml"; \
    for c in path ws name patch; do \
        if awk -f check-reference-independence.awk "$td/$c.toml" >/dev/null 2>&1; then \
            echo "FAIL(negative): scanner did NOT bite on $c.toml violation"; fail=1; \
        else \
            echo "OK(negative): scanner bit on $c.toml"; \
        fi; \
    done; \
    mkdir -p "$td/src"; \
    printf 'fn main() { include!("../../../nucleus/generated.rs"); }\n' > "$td/src/main.rs"; \
    if rg -q '{{ref_include_pat}}' "$td/src/"; then \
        echo "OK(negative): src include-outside guard bit"; \
    else \
        echo "FAIL(negative): src include-outside guard did NOT bite"; fail=1; \
    fi; \
    if [ "$fail" -ne 0 ]; then exit 1; fi; \
    echo "OK: reference-independence fence bites on every injected violation."

# Catch the silent doc-link / HTML-tag breakage class (memory:
# feedback-visibility-tighten-doclink-trap). `just check` / `just clippy`
# build NO docs, so a visibility tighten or a renamed item that breaks an
# intra-doc-link — or angle-bracket prose rustdoc reads as an unclosed
# HTML tag — ships green and SILENT. The TASK-0340.11 epic drove both
# classes to zero across all 14 workspace crates. This arm has TWO
# independent guards, BOTH proven to bite (TASK-0340.11.02):
#
# 1. Two denied rustdoc lints on the whole-workspace doc build:
#      - broken_intra_doc_links: a [`Foo`] link whose target moved or was
#        renamed, or an unresolved explicit-path link [`X`](made_up::Item).
#      - invalid_html_tags: `<name>` / `slot_<id>` prose parsed as HTML.
#    Negative arm proven: a broken [`Foo`] link makes the build exit 101.
#
# 2. A dead-cross-crate-href grep on the rendered HTML — the PRIMARY (and
#    only) catch for the trap that bit the backend-common slice, which the
#    lint in (1) is STRUCTURALLY BLIND to. An explicit-path link
#    [`X`](other_crate::path) to a real dependency crate with no docs.rs
#    resolution — i.e. a WORKSPACE-SIBLING path-dep such as nucleus_compiler
#    — renders a literal href="other_crate::path" that 404s, and rustdoc
#    emits NO warning (it trusts the author-supplied path and does NOT
#    resolve it, EVEN under --workspace where the sibling IS co-documented).
#    Negative arm proven TASK-0340.11.02: [`DataId`](nucleus_compiler::event::DataId)
#    injected into backend-common keeps the build exit-0 yet renders
#    href="nucleus_compiler::event::DataId" — caught ONLY by this grep,
#    which makes the recipe exit 1 and prints the dead href.
#    Any rendered href of the form word::... is a guaranteed-dead
#    cross-crate link; real crates.io deps (serde, mio) resolve to
#    https://docs.rs/..., which the word::... pattern excludes.
#    Important: --workspace co-resolves AUTO-LINKED (bare [`X`] /
#    use-imported) cross-crate references to relative hrefs, but NOT the
#    explicit-path [`X`](crate::path) form — so the grep is genuinely the
#    catch for the explicit-path class, NOT redundant with the lint.
#
# Asserts rustdoc EXIT==0 BEFORE the grep — an empty grep on a failed
# (exit-101) build is VACUOUS, not clean (TASK-0340.11.03 method
# correction): the cargo doc line fails the recipe first, and `rm -rf
# target/doc` precedes it so no stale HTML can falsely pass the grep.
#
# Lint-set decision (TASK-0340.11.01/.02): gates the DEFAULT public-API
# doc surface only — NOT --document-private-items, so broken doc-links on
# PRIVATE items are deliberately NOT gated (a documented scope boundary).
# private_intra_doc_links and rustdoc::all (unescaped-backtick;
# backend-common still carries 2, a deferred decision) are also NOT denied.
check-doc-links:
    @echo "checking workspace cargo-doc (broken_intra_doc_links + invalid_html_tags + dead cross-crate hrefs)..."
    cd nucleus && rm -rf target/doc && RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links -D rustdoc::invalid_html_tags' cargo doc --no-deps --workspace
    @cd nucleus && dead=$(grep -rhoE 'href="[a-z_]+::[^"]*"' target/doc/ 2>/dev/null | sort -u || true); \
    if [ -n "$dead" ]; then \
        echo "FAIL: dead cross-crate intra-doc hrefs in rendered docs — the -D broken_intra_doc_links lint is blind to these (memory: feedback-visibility-tighten-doclink-trap):"; \
        echo "$dead"; \
        echo ""; \
        echo "A cross-crate intra-doc link to a non-workspace dep renders a literal-path href that 404s under --no-deps. Use a backtick code span for the out-of-crate reference, not a [link](dep::path)."; \
        exit 1; \
    fi; \
    echo "OK: workspace docs clean (0 broken-link / html-tag warnings, 0 dead cross-crate hrefs)."

# Catch the predictive-conclusion doc-lie defect class in narrative
# TOML (memory: feedback-comment-doc-lie-recurring 12+ firings;
# memory: feedback-silent-sibling-defect 13th firing as of cycle
# 169). Concrete motivating cycles: TASK-0338 cycle 169 + 169b
# closed two structurally identical stale blocks in
# nuc-nucleus/e2e-matrix.toml. TASK-0339 (this recipe) converts
# reactive cleanup into a gate-time check.
#
# Pattern set: predictive-claim phrasings whose conclusion rots when
# the predicted event lands. The author of cycle-N narrative often
# fails to back-edit the cycle-N-1 paragraph's predictive conclusion
# when cycle N answers the prediction.
#
# Per-line allow-list: a hit is OK if the same line contains one of:
#   - 'AT FILING TIME' marker (explicit past-tense framing)
#   - '# Cycle-<N> ...' paragraph-header prefix (any line starting
#     with `# Cycle-N` is recognized as a paragraph time-stamp;
#     widened cycle 170b after the file was observed to use
#     `# Cycle-N filing:`, `# Cycle-N update:`, `# Cycle-N PROMOTION:`,
#     `# Cycle-N first attempt:`, etc.)
#   - '# ALLOW narrative-doc-lie: <reason>' annotation
#
# Pattern set is case-insensitive (-i): sentence-initial variants
# `Pending cycle-N`, `Awaits TASK`, `Blocked by TASK` etc. fire too.
# Grep `ALLOW narrative-doc-lie` to find allow-annotated sites; their
# drift across file edits is OK as long as the annotation stays on
# the same line as the hit.
check-narrative-doc-lie:
    @echo "checking for predictive-conclusion doc-lies in narrative TOML..."
    @hits=$(rg -inH \
        -e 'BLOCKED by TASK' \
        -e 'CARRIED as \[\[skip\]\]' \
        -e 'still pending' \
        -e 'currently \[\[skip\]\]' \
        -e 'pending cycle-?[0-9]+' \
        -e 'Only .+ remains \[\[skip\]\]' \
        -e '[0-9]+ of [0-9]+ tier-1 backends' \
        -e '\bawaits\b' \
        -e '\bawaiting\b' \
        -e '\bgated on\b' \
        -e '\bnot yet\b' \
        nuc-nucleus/e2e-matrix.toml \
        | grep -v 'AT FILING TIME' \
        | grep -vE '^[^:]+:[0-9]+:# Cycle-?[0-9]+\b' \
        | grep -v 'ALLOW narrative-doc-lie' \
        || true); \
    if [ -n "$hits" ]; then \
        echo "FAIL: predictive-conclusion doc-lie candidates in narrative TOML (memory: feedback-comment-doc-lie-recurring, feedback-silent-sibling-defect cycle-169 hygiene rule):"; \
        echo "$hits"; \
        echo ""; \
        echo "Each hit is a paragraph whose predictive conclusion may have rotted when the predicted event landed."; \
        echo "Fix options (any one):"; \
        echo "  1. Convert the verb to past-tense + add 'AT FILING TIME' marker on the same line."; \
        echo "  2. Prefix the surrounding paragraph with '# Cycle-<N> filing:' / '# Cycle-<N> update:' / '# Cycle-<N> PROMOTION:' etc. (any '# Cycle-N ...' line is recognized as a paragraph time-stamp)."; \
        echo "  3. Annotate the line with '# ALLOW narrative-doc-lie: <reason>' (use only when the line is current-state-accurate or framed historical by a block header — explain which)."; \
        exit 1; \
    fi; \
    echo "OK: no predictive-conclusion doc-lies in narrative TOML."

# Doc-citation staleness fence (TASK-0370, cycle 220).
#
# A SECOND, ORTHOGONAL arm of the comment-doc-lie defence (memory:
# feedback-comment-doc-lie-recurring — the project's #1 recurring
# defect). Where `check-narrative-doc-lie` (above) scans PRESENT-TENSE
# PROSE in the one high-discipline narrative TOML, THIS recipe is an
# OBJECTIVE STRUCTURAL check: it verifies that every FULLY-QUALIFIED
# source citation of the form `nucleus/<...>.rs:N` (or `.rs:N-M` /
# `.rs:N..M`) resolves to a file that exists AND whose line count is
# >= the largest cited line. It catches two historically-recurring
# lie shapes objectively, with NO escape-hatch annotation required:
#   - cycle-138 STALE-LINE citations (file shrank below the cited line).
#   - cycle-181b SPLIT-FILE deixis (the cited `foo.rs` became the
#     directory `foo/` — file no longer exists at that path).
#
# WHY FULLY-QUALIFIED ONLY (the load-bearing zero-FP boundary — see
# TASK-0370 notes for the full empirical scoping):
#   - A BARE-BASENAME citation (`lib.rs:1010`, `multi_worker.rs:854`)
#     is AMBIGUOUS — every crate has a `lib.rs` (12+ in this tree), so
#     there is no single file to resolve against. Worse, cross-crate
#     prose ("pre-extraction pthreads-sync at lib.rs:991") names a
#     DIFFERENT crate's file than the citing crate's own — a mechanical
#     resolver MISATTRIBUTES the verdict. So bare basenames are
#     deliberately NOT validated. A path that starts with `nucleus/`
#     or `nuc-nucleus/` has EXACTLY ONE resolution; that is the only
#     class this fence trusts.
#   - The numeric line is ADVISORY by the cycle-138 rule (prefer
#     symbol anchors); this fence only bites the unambiguous failure
#     "cited line is past EOF" / "cited file no longer exists". It
#     does NOT (cannot) catch stale-CONTENT where the line still
#     exists but the code at it moved — that is the deferred breadth.
#
# WHY backlog/tasks IS EXCLUDED:
#   - Task markdown is an IMMUTABLE HISTORICAL RECORD (CLAUDE.md forbids
#     hand-editing it) and its citations are FILING-TIME-ACCURATE
#     provenance — e.g. `task-0340.01`'s own title encodes the then
#     1997-LoC `lib.rs` that is now 329 LoC. Those are not lies; they
#     are history. ~42 fully-qualified citations there are stale-by-
#     design. Validating them would force either FP-flood or a
#     forbidden history rewrite.
#
# COVERAGE / DEFERRED BREADTH (honest scope — TASK-0370 AC#1 reads
# "or a justified subset"):
#   - COVERED: fully-qualified `.rs:N` citations in source (`*.rs`),
#     `docs/`, `README*.md`, `PRD.md`, `nuc-nucleus/`.
#   - DEFERRED (filed as TASK-0382, depends on TASK-0370):
#       (i) bare-basename citation validation (needs a crate-scoped,
#           prose-aware resolver to stay zero-FP);
#      (ii) stale-CONTENT detection (line exists, code moved);
#     (iii) present-tense narrative-prose scanning of md / `.sched.nuc`
#           headers (FP-floods: 171 legitimate hits on backlog/tasks
#           alone with the existing pattern set).
#
# POSIX-shell portability (cf. check-mega-files): `just` runs
# `/bin/sh -cu`; this recipe avoids bash arrays / process substitution.
# It feeds a `mktemp` temp file into a `while read` loop and parses
# each citation with POSIX parameter expansion + a single `awk` for
# the range tail.
#
# SCAN SCOPE (TASK-0395, shared by all four doc-citation fences below):
# the source of citations is `git ls-files` (intentional, tracked
# content) piped through `xargs -0 rg`, NOT a bare `rg .` repo-root
# scan. `rg .` respects .gitignore but STILL scans any untracked,
# non-ignored scratch dir (an ad-hoc emit/`scratch/`/`foo-out/` dir) —
# so a stale citation string sitting in throwaway scratch would red
# `just ci` (TASK-0394 only ignored the conventional `tmp/`). Scanning
# git-tracked files makes the fences deterministic regardless of what
# untracked junk is in the worktree. TRADE-OFF (accepted): a citation
# in a BRAND-NEW untracked file is unchecked until `git add` — fine for
# a pre-commit fence (the file is not yet part of the repo, and the
# common case, an EDIT to an already-tracked file, is still scanned via
# its working-tree content). `--with-filename`/`--no-filename` are set
# explicitly because `xargs` may batch into a single-file final `rg`
# invocation, which would otherwise drop the `file:` prefix the parsers
# below require. Do NOT revert to `rg .` — it reintroduces the scratch
# footgun.
check-doc-citation-staleness:
    @echo "checking fully-qualified nucleus/*.rs:N citations resolve (exists + line in range)..."
    @set -eu; \
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "FAIL: doc-citation fence must run inside a git work-tree (it scans git ls-files; TASK-0395)"; exit 1; }; \
    cites_f=$(mktemp); \
    trap "rm -f $cites_f" EXIT; \
    git ls-files -z -- ':!backlog/tasks' \
        | xargs -0 -r rg --no-filename -N \
            -oe '(nucleus|nuc-nucleus)/[A-Za-z0-9_./-]+\.rs:[0-9]+([.-]+[0-9]+)?' \
            2>/dev/null \
        | sort -u > $cites_f || true; \
    fail=0; \
    while IFS= read -r cite; do \
        [ -n "$cite" ] || continue; \
        path=${cite%%:*}; \
        lines=${cite#*:}; \
        maxl=$(printf '%s' "$lines" | grep -oE '[0-9]+$' || true); \
        case "$maxl" in ''|*[!0-9]*) echo "  WARN (unparseable line range, not checked): $cite"; continue;; esac; \
        if [ ! -f "$path" ]; then \
            echo "  STALE (no such file): $cite"; \
            echo "    -> the cited file does not exist (likely split into a directory; cycle-181b deixis)."; \
            fail=1; \
            continue; \
        fi; \
        total=$(awk 'END{print NR}' "$path"); \
        if [ "$maxl" -gt "$total" ]; then \
            echo "  STALE (line past EOF): $cite  (file has $total lines)"; \
            fail=1; \
        fi; \
    done < $cites_f; \
    if [ "$fail" -ne 0 ]; then \
        echo ""; \
        echo "FAIL: stale fully-qualified source citation(s) (memory: feedback-comment-doc-lie-recurring cycle-138 stale-line / cycle-181b split-file deixis)."; \
        echo "Fix (cycle-138 rule, in order of preference):"; \
        echo "  1. Re-anchor the citation to a STABLE symbol/comment name (e.g. 'the \`emit_log_branch\` call in event_walker.rs') instead of a line number — line numbers rot on every edit."; \
        echo "  2. If a line number is genuinely needed, update it to the current line and re-grep to confirm post-edit."; \
        echo "  3. If the file was split, point at the new sub-module path."; \
        exit 1; \
    fi; \
    echo "OK: every fully-qualified nucleus/*.rs:N citation resolves to an in-range line."

# Bare-basename / partial-path doc-citation staleness fence (TASK-0382
# cycle 221; partial-path arm TASK-0382.01).
#
# The SIBLING of check-doc-citation-staleness (above) for the OTHER
# citation class: a BARE basename `<file>.rs:N` (e.g. `wait.rs:307`,
# `multi_worker.rs:174-186`) — or a partial path `<seg>/.../<file>.rs:N`
# (e.g. `sched/ir.rs:382`) — with no `nucleus/<crate>/...` path prefix.
# This is the BULK of in-source citations and the class TASK-0370
# DEFERRED as "intractable for zero-FP". It is tractable with a
# CRATE-SCOPED, PROSE-AWARE resolver whose every rule is biased toward
# SKIPPING (zero-FP-favouring): a citation is only validated when its
# resolution is UNAMBIGUOUS and the surrounding prose gives no reason
# to doubt the crate. Scanned in `*.rs` files only (where the citing
# file's crate is well-defined by its nearest-ancestor Cargo.toml).
#
# The capture also admits an optional INTERIOR-SLASH path prefix
# (`<seg>/.../<base>.rs:N`, segments `[A-Za-z0-9_]+`) — see PARTIAL-PATH
# below. The prefix is MORE disambiguating, not less, so honouring it
# only ever recovers coverage (it can turn a SKIP:ambiguous basename
# into an UNAMBIGUOUS suffix resolve); it never introduces an FP.
#
# ALGORITHM (per `<path>.rs:N` hit at FILE:LINENO, where `<path>` is a
# basename `<base>.rs` or a partial path `<seg>/.../<base>.rs`):
#   1. crate root = nearest ancestor dir of FILE with a Cargo.toml.
#   2. resolve `<path>.rs` under <crate-root>:
#        - PARTIAL PATH (has `/`): `find <crate-root> -path '*/<path>.rs'`
#          (SUFFIX match — the interior slash disambiguates, e.g.
#          `sched/ir.rs` resolves to the one `.../sched/ir.rs`, never
#          colliding with `algo/ir.rs`).
#        - BARE BASENAME (no `/`): `find <crate-root> -name <base>.rs`
#          (unchanged).
#        - 0 matches  -> SKIP (not-in-crate; e.g. a partial path whose
#          file was moved/renamed — `multi_worker/mod.rs:N` after the
#          TASK-0340.04 split — or a cross-crate cite whose suffix is
#          absent here. SAFE: moved-file staleness is out of scope, see
#          DEFERRED).
#        - >1 matches -> SKIP (ambiguous; e.g. bare `ir.rs` matches
#          both `algo/ir.rs` and `sched/ir.rs`).
#   3. CROSS-CRATE-PROSE GUARD: scan the citation line and the WIN lines
#      above it for the name of ANY OTHER crate, in BOTH dash-form
#      (`pthreads-sync`) AND Rust module-path underscore-form
#      (`pthreads_sync`). If found -> SKIP. This is the load-bearing
#      zero-FP rule: check_frame.rs (in backend-common) cites
#      `lib.rs:1010-1018` whose prose three lines up says
#      "pthreads-sync's pre-extraction comment" — a naive resolver
#      MISATTRIBUTES it to backend-common/src/lib.rs (92 lines) and
#      false-positives. The underscore variant is equally load-bearing:
#      `pthreads_sync::multi_worker::Plan::emit (multi_worker.rs:237)`
#      in pthreads-async names pthreads_sync ONLY in `::`-path form.
#   4. otherwise range-check N against the resolved file's line count.
#
# WHY A WINDOW (and why WIN is deliberately SMALL): the crate name and
# the citation often straddle a wrapped `///` line, so same-line-only
# misses it (empirically reproduced: WIN=1 false-positives the
# check_frame.rs lib.rs cite). But widening too far OVER-skips, because
# `e2e`, `nucleus`, `driver` double as common domain words — at WIN=6 a
# legitimate same-crate `expr.rs` cite is skipped because a sentence
# four lines up says "the 7 shipped e2e gather cells". WIN=3 is the
# measured sweet spot: zero false-positives AND minimal over-skip.
# Over-skipping is SAFE (a missed validation, never a false alarm);
# under-skipping risks an FP, so the tie always breaks toward SKIP.
#
# MAINTENANCE CONTRACT (the price of prose-dependence; architect
# cycle-221 P2). Unlike the FQ sibling — which derives the crate
# STRUCTURALLY from the path and cannot drift — this fence's zero-FP
# property is LOAD-BEARING on prose layout for the cross-crate bare
# `lib.rs:N` cites in `nucleus/backend-common/src/check_frame.rs`.
# Those cites are the historical pre-extraction provenance pointers
# (TASK-0052.04) naming pthreads-sync / mp-tcp-bufsync `lib.rs` sites,
# NOT backend-common's own short 92-line lib.rs. (Deliberately NOT
# enumerated by line number here: an approximate line list in a
# comment ROTS on every check_frame.rs edit — the exact line-rot this
# fence exists to catch. Locate them with
# `grep -nE 'lib\.rs:[0-9]' nucleus/backend-common/src/check_frame.rs`.)
# Today each names its crate within WIN lines, so all SKIP correctly.
# If you REWORD those docstrings, keep the crate name
# (`pthreads-sync`/`pthreads_sync`, dash or underscore) within WIN
# lines of the cite — otherwise this fence misattributes the bare
# `lib.rs:N` to backend-common's short lib.rs and FALSE-POSITIVES
# `just ci`. (Empirically reproduced: crate name pushed 4+ lines above
# the cite -> validate -> FP. Do NOT fully-qualify these cites to
# silence it: their line numbers are pre-extraction-historical and
# would then FAIL the FQ sibling's range check.)
#
# PARTIAL-PATH citations (`sched/ir.rs:N`, `multi_worker/mod.rs:N`):
# LANDED (TASK-0382.01, cycle-221 follow-up). The optional interior-slash
# prefix is now captured and resolved by SUFFIX (`find -path '*/<path>'`)
# under the crate root — see ALGORITHM step 2. Zero-FP is preserved: the
# suffix is strictly more disambiguating than the basename, and every
# 0-match (file moved/renamed, e.g. `multi_worker/mod.rs` after the
# TASK-0340.04 split) or >1-match goes to SKIP. CAVEAT: the segment class
# is `[A-Za-z0-9_]+` (no hyphen), so a cite whose FIRST segment is a
# HYPHENATED crate name (`nucleus-compiler/src/...`) is captured as its
# trailing run (`compiler/src/...`) and SKIPs on a segment-boundary
# mismatch — SAFE (unvalidated, never a false alarm). Moved-file
# staleness (the file moved/renamed → 0-match → SKIP) stays OUT of scope:
# the fence catches "line past EOF" (file shrank), not "file moved".
#
# DEFERRED (honest coverage limits — all SAFE skips, none can produce a
# false POSITIVE):
#   - The Implementation-Notes BARE-BASENAME-AS-LOCATION variant (a
#     prose `tests.rs` with NO `:N`, claiming a named test resides
#     there) needs symbol/test-name residence checking, not line-count.
#     Filed as TASK-0382.02 (harder zero-FP profile — prose ambiguity
#     about what token is a symbol name).
#   - STALE-CONTENT (line still exists, code moved) — see AC#2 decision
#     recorded on TASK-0382: out of mechanized scope; the cycle-138
#     prefer-a-symbol-anchor convention is the mitigation.
#
# POSIX-shell portability (cf. check-mega-files / the FQ sibling): `just`
# runs `/bin/sh -cu`. The crate-name list is a `set --` positional
# list (no bash arrays); the window slice is one `awk`; path/basename
# resolution is `find ... -path`/`-name`.
check-doc-citation-staleness-bare:
    @echo "checking bare-basename <file>.rs:N citations (crate-scoped, prose-aware)..."
    @set -eu; \
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "FAIL: doc-citation fence must run inside a git work-tree (it scans git ls-files; TASK-0395)"; exit 1; }; \
    win=3; \
    set -- backend-common nucleus-compiler driver e2e nucleus mp-tcp-common \
        test-common embedded-pattern mp-tcp-bufsync mp-tcp-event mp-tcp-poll \
        mp-uds-event openmp-rs pthreads-async pthreads-sync; \
    crates="$@"; \
    recs_f=$(mktemp); \
    trap "rm -f $recs_f" EXIT; \
    git ls-files -z -- '*.rs' \
        | xargs -0 -r rg --with-filename --no-heading -n \
            -oe '[A-Za-z0-9_]+(/[A-Za-z0-9_]+)*\.rs:[0-9]+([.-]+[0-9]+)?' \
            2>/dev/null > $recs_f || true; \
    fail=0; \
    while IFS= read -r rec; do \
        [ -n "$rec" ] || continue; \
        cite=$(printf '%s' "$rec" | grep -oE '[A-Za-z0-9_]+(/[A-Za-z0-9_]+)*\.rs:[0-9]+([.-]+[0-9]+)?$' || true); \
        [ -n "$cite" ] || continue; \
        rest=${rec%:"$cite"}; \
        lineno=${rest##*:}; \
        file=${rest%:*}; \
        base=${cite%%:*}; \
        lines=${cite#*:}; \
        maxl=$(printf '%s' "$lines" | grep -oE '[0-9]+$' || true); \
        case "$maxl" in ''|*[!0-9]*) continue;; esac; \
        croot=$(d=$(dirname "$file"); while [ "$d" != "." ] && [ "$d" != "/" ]; do if [ -f "$d/Cargo.toml" ]; then printf '%s' "$d"; break; fi; d=$(dirname "$d"); done); \
        [ -n "$croot" ] || continue; \
        cratename=$(basename "$croot"); \
        lo=$((lineno - win)); [ "$lo" -lt 1 ] && lo=1; \
        windowtext=$(awk -v a="$lo" -v b="$lineno" 'NR>=a && NR<=b' "$file" 2>/dev/null); \
        skip=0; \
        for c in $crates; do \
            [ "$c" = "$cratename" ] && continue; \
            cu=$(printf '%s' "$c" | tr '-' '_'); \
            case "$windowtext" in *"$c"*) skip=1; break;; esac; \
            case "$windowtext" in *"$cu"*) skip=1; break;; esac; \
        done; \
        [ "$skip" -eq 1 ] && continue; \
        case "$base" in \
            */*) matches=$(find "$croot" -path "*/$base" 2>/dev/null);; \
            *)   matches=$(find "$croot" -name "$base" 2>/dev/null);; \
        esac; \
        nmatch=$(printf '%s\n' "$matches" | grep -c . || true); \
        [ "$nmatch" -eq 1 ] || continue; \
        total=$(awk 'END{print NR}' "$matches"); \
        if [ "$maxl" -gt "$total" ]; then \
            echo "  STALE (line past EOF): $cite cited in $file:$lineno"; \
            echo "    -> resolves crate-scoped to $matches ($total lines); cited line $maxl is past EOF."; \
            fail=1; \
        fi; \
    done < $recs_f; \
    if [ "$fail" -ne 0 ]; then \
        echo ""; \
        echo "FAIL: stale bare-basename source citation(s) (memory: feedback-comment-doc-lie-recurring cycle-138 stale-line)."; \
        echo "Fix (cycle-138 rule, in order of preference):"; \
        echo "  1. Re-anchor the citation to a STABLE symbol/comment name instead of a line number — line numbers rot on every edit."; \
        echo "  2. If a line number is genuinely needed, update it to the current line and re-grep to confirm post-edit."; \
        echo "  3. If the cite means ANOTHER crate's file, name that crate in the same/adjacent line so the cross-crate-prose guard skips it."; \
        exit 1; \
    fi; \
    echo "OK: every crate-resolvable, prose-unambiguous bare-basename citation is in range."

# Doc test-name citation fence (TASK-0382.02, cycle-231).
#
# A THIRD orthogonal arm of the comment-doc-lie defence family
# (alongside check-narrative-doc-lie / check-doc-citation-staleness /
# -bare), catching a class NONE of those see: a back-tick-quoted
# reference to a UNIT TEST by name -- the project's `task<NNNN>_<desc>`
# convention -- in a `.rs` docstring/comment, where the named test no
# longer exists as a `fn` (renamed or deleted). This is the same FAMILY
# as the recurring "stale test-pin citation" doc-lie (memory:
# feedback-verbatim-copy-comment-doc-lie, cycle-197). NB cycle-197's OWN
# firing was a `module::descriptive_name`-shaped pin, which this fence
# does NOT cover -- the broader back-ticked `mod::name` test-pin class
# (~640 in-tree refs) is a candidate follow-up, not closed here. The
# existing fences validate `.rs:N` LINE citations or narrative TOML
# prose; none validate a back-ticked test-NAME.
#
# WHY THIS SHAPE IS ZERO-FP (the load-bearing boundary):
#   - Token class: `task` + >=3 digits + snake-tail, BACK-TICK QUOTED.
#     `task\d{3,}` is the project's test-naming convention; it is never
#     an ordinary English word nor a codegen string literal. Codegen
#     file names are `"kernels.rs"`-style DOUBLE-quoted literals (in the
#     fence's gitignore-respected scan space: ~150 such literals, ~610
#     `kernels.rs` mentions, ~2.3k bare `<base>.rs` mentions overall;
#     plus tens of thousands more in gitignored generated output the
#     fence never scans) -- and ZERO of them start with `task\d`. A
#     back-tick + `task\d{3,}` is unambiguously a test ref.
#   - Definitions are `fn task...` (NEVER back-tick-quoted), so the scan
#     cannot mistake a definition for a reference.
#   - Resolution is WORKSPACE-GLOBAL (not crate-scoped): test names are
#     globally unique by the `task<NNNN>` convention, so a ref is
#     satisfied by the test existing in ANY crate. Rule: the ref
#     (trailing `*` of a `task<NNNN>_*` glob stripped) must be a literal
#     PREFIX of some defined `fn task...` name. Prefix (not equality) so
#     an abbreviated cite (`task0306_ac3` -> fn `task0306_ac3_inner_...`)
#     and a glob-family cite (`task0299_*`) both resolve; a renamed cite
#     (no fn begins with it any longer) FAILS.
#   - SAFE asymmetry (cf. the bare fence): under-matching is SAFE (a
#     missed lie, never a false alarm). The prefix rule only ever FAILS
#     a cite that prefixes NO existing test -- which is exactly the lie.
#
# OUT OF SCOPE (honest limits -- all deferred, none can false-POSITIVE):
#   - The general symbol FILE-RESIDENCE variant TASK-0382.02 originally
#     framed ("`sym` in `base.rs`") is NOT built: the back-ticked-ident
#     + back-ticked-`base.rs` co-occurrence has ~16 in-tree instances,
#     but they are a MIX of true residence claims (`relay_one` defined
#     in `runtime.rs`) and non-residence co-occurrences (a symbol
#     EMITTED into a generated `main.rs`, or CALLED from `events.rs`) --
#     so a zero-FP residence checker must disambiguate defined-in vs
#     called-from vs emitted-into, which is the genuine hard part. Left
#     deferred on TASK-0382.02 (its open deliverable); the `task<NNNN>`
#     arm here is the feasible zero-FP subset.
#   - Stale e2e-CELL-name cites (`gather_2out_loop` renamed -- the actual
#     ec50108 lie) are a separate shape (validate vs e2e-matrix.toml,
#     not `fn` defs) -- filed as TASK-0392.
#   - `.md` (backlog history, excluded by design) and `.toml` (covered
#     by check-narrative-doc-lie) are out of scope; this fence is `.rs`
#     source docstrings/comments only.
check-doc-test-name-staleness:
    @echo "checking back-ticked task<NNNN> test-name citations resolve to a defined fn..."
    @set -eu; \
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "FAIL: doc-citation fence must run inside a git work-tree (it scans git ls-files; TASK-0395)"; exit 1; }; \
    defs=$(mktemp); refs=$(mktemp); \
    trap "rm -f $defs $refs" EXIT; \
    git ls-files -z -- '*.rs' \
        | xargs -0 -r rg --no-filename -o 'fn task[0-9]{3,}[_a-z0-9]*' 2>/dev/null \
        | sed 's/.*fn //' | sort -u > $defs || true; \
    git ls-files -z -- '*.rs' \
        | xargs -0 -r rg --with-filename -o -n --no-heading \
            '`task[0-9]{3,}[_a-z0-9]*\*?`' 2>/dev/null > $refs || true; \
    fail=0; \
    while IFS= read -r rec; do \
        [ -n "$rec" ] || continue; \
        tok=${rec##*:}; \
        loc=${rec%:*}; \
        name=$(printf '%s' "$tok" | tr -d '`*'); \
        [ -n "$name" ] || continue; \
        if ! grep -qE "^${name}" $defs; then \
            echo "  STALE test-name citation: $tok at $loc"; \
            echo "    -> no defined 'fn task...' begins with '$name' (renamed or deleted test)."; \
            fail=1; \
        fi; \
    done < $refs; \
    if [ "$fail" -ne 0 ]; then \
        echo ""; \
        echo "FAIL: stale back-ticked test-name citation(s) (memory: feedback-comment-doc-lie-recurring / feedback-verbatim-copy-comment-doc-lie 'test-pin citation')."; \
        echo "Fix (in order of preference):"; \
        echo "  1. Update the cite to the current test name (re-grep 'fn <name>' to confirm)."; \
        echo "  2. If the test was deleted, drop the citation or point it at the replacement."; \
        echo "  3. If it is a deliberate glob family cite, ensure at least one 'fn <prefix>...' still exists."; \
        exit 1; \
    fi; \
    echo "OK: every back-ticked task<NNNN> test-name citation resolves to a defined fn."

# Doc e2e-cell-PATH citation fence (TASK-0392, cycle-233).
#
# A FOURTH arm of the comment-doc-lie defence family (alongside
# check-narrative-doc-lie / check-doc-citation-staleness / -bare /
# check-doc-test-name-staleness), catching a class NONE of those see: a
# back-tick-quoted reference to an e2e differential CELL by its
# example-PATH name -- the project's `NN-name/variant` convention (e.g.
# `18-multigather/distributed`) -- in a `.rs` docstring/comment, where
# the named example or schedule no longer exists (example dir renamed,
# schedule file renamed/deleted). This is the SAME defect class as the
# ec50108 lie that motivated TASK-0382.02's family: the cell formerly
# aliased `gather_2out_loop` was renamed to `18-multigather/distributed`,
# and a prose ref kept the dead name. The sibling test-name fence
# validates `fn task<NNNN>` cites; this one validates `NN-name/variant`
# cell-path cites against the examples tree -- a DIFFERENT resolution
# target (schedule files, not `fn` defs).
#
# WHY THIS SHAPE IS ZERO-FP (the load-bearing boundary):
#   - Token class: BACK-TICK QUOTED `\d{2}-[a-z]...` (a letter right
#     after the `NN-` hyphen) + `/` + a LETTER-led tail. This is the
#     example-directory convention (`nuc-nucleus/examples/NN-word/`,
#     never `NN-NN`); the leading-digits + slash + letter-led shape is
#     never an ordinary English word, a Rust `::`-path, nor a codegen
#     `"basename.rs"` literal. A date-like `NN-DD/NNNN` false match is
#     blocked TWICE: by the letter-after-`NN-` gate AND the letter-after-
#     `/` gate (variants are always letter-led: `naive`, `distributed`,
#     `distributed-2d`, `reuse`).
#   - LOAD-BEARING trailing anchor: the regex is back-tick-DELIMITED at
#     BOTH ends, so the tail `[a-z][a-z0-9_-]*` cannot contain `.` or a
#     second `/`. THAT closing `` ` `` is what excludes the in-tree
#     suffixed/deeper siblings that would otherwise false-resolve or
#     mismatch -- e.g. `05-stencil/distributed.sched.nuc` (a `.`) and
#     `14-hearing-aid/schedules/embedded_multimcu.sched.nuc` (a second
#     `/`). A future maintainer relaxing the regex to also match a
#     `.sched.nuc` suffix or a deeper path MUST keep this exclusion in
#     mind or reintroduce FPs. In the gitignore-respected scan space
#     there are 7 unique cell-paths (12 citation sites) today and ALL
#     resolve -- it is a tight, additive class.
#   - Resolution: examples/NN-name/schedules/variant.sched.nuc is the
#     DOCUMENTED source-of-truth for which schedule files exist (see the
#     e2e-matrix.toml header). The rule -- the cited
#     `examples/<ex>/schedules/<var>.sched.nuc` MUST exist -- catches
#     BOTH directions: an example-dir rename (`<ex>` gone) AND a
#     schedule/cell rename (`<var>.sched.nuc` gone). Either is the lie.
#   - SAFE asymmetry (cf. the bare / test-name fences): the only way to
#     FAIL is a cite whose example-path does NOT resolve -- which is
#     exactly the stale-cell lie. A token that is genuinely not a cell
#     ref but happens to match the shape would only fail if it ALSO
#     fails to resolve as a real example path; the shape is specific
#     enough (verified zero such tokens in-tree) that this is not a
#     practical FP source. Under-matching (a cell ref the regex misses)
#     is a missed lie, never a false alarm.
#
# OUT OF SCOPE (honest limits -- deferred, cannot false-POSITIVE):
#   - The bare snake_case CELL-ALIAS shape (`gather_2out_loop`) -- the
#     literal ec50108 token -- is NOT covered: a bare snake_case alias is
#     not disambiguable from an ordinary symbol mention without a
#     curated alias map, so it stays deferred on TASK-0392 (its open
#     deliverable). The `NN-name/variant` arm here is the feasible
#     zero-FP subset.
#   - `.md` (backlog history, excluded by design) and `.toml` (the matrix
#     itself, covered by check-narrative-doc-lie) are out of scope; this
#     fence is `.rs` source docstrings/comments only.
check-doc-cell-path-staleness:
    @echo "checking back-ticked e2e cell-path citations (NN-name/variant) resolve to a schedule file..."
    @set -eu; \
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "FAIL: doc-citation fence must run inside a git work-tree (it scans git ls-files; TASK-0395)"; exit 1; }; \
    refs=$(mktemp); \
    trap "rm -f $refs" EXIT; \
    git ls-files -z -- '*.rs' \
        | xargs -0 -r rg --with-filename -o -n --no-heading \
            '`[0-9]{2}-[a-z][a-z0-9-]*/[a-z][a-z0-9_-]*`' 2>/dev/null > $refs || true; \
    fail=0; \
    while IFS= read -r rec; do \
        [ -n "$rec" ] || continue; \
        tok=${rec##*:}; \
        loc=${rec%:*}; \
        cell=$(printf '%s' "$tok" | tr -d '`'); \
        ex=${cell%%/*}; \
        var=${cell##*/}; \
        if [ ! -f "nuc-nucleus/examples/$ex/schedules/$var.sched.nuc" ]; then \
            echo "  STALE e2e cell-path citation: $tok at $loc"; \
            if [ ! -d "nuc-nucleus/examples/$ex" ]; then \
                echo "    -> no example dir 'nuc-nucleus/examples/$ex' (example renamed or removed)."; \
            else \
                echo "    -> no schedule 'nuc-nucleus/examples/$ex/schedules/$var.sched.nuc' (schedule/cell renamed or removed)."; \
            fi; \
            fail=1; \
        fi; \
    done < $refs; \
    if [ "$fail" -ne 0 ]; then \
        echo ""; \
        echo "FAIL: stale back-ticked e2e cell-path citation(s) (memory: feedback-comment-doc-lie-recurring; ec50108 cell-rename class)."; \
        echo "Fix (in order of preference):"; \
        echo "  1. Update the cite to the current 'NN-name/variant' (ls nuc-nucleus/examples/<ex>/schedules to confirm)."; \
        echo "  2. If the example/schedule was removed, drop the citation or point it at the replacement cell."; \
        exit 1; \
    fi; \
    echo "OK: every back-ticked e2e cell-path citation resolves to a schedule file."

# Mega-file regression-fence (TASK-0340 AC#5; slice 1 cycle 176, slice 2
# cycle 177).
#
# Asserts no file under the scoped sub-trees STRICTLY EXCEEDS 1000 LoC
# outside an explicit allow-list. The recipe checks BOTH directions:
#   (A) Direction "new mega-file appears" — any oversized file NOT in
#       the allow-list FAILS LOUD.
#   (B) Direction "allow-list entry becomes stale" — any allow-list
#       entry whose file is NO LONGER >1000 LoC (split landed, file
#       deleted, file shrank) FAILS LOUD.
#
# Direction (B) was added cycle 177 (architect cycle-176 P2.1
# fold-back) — the prior negative-filter shape (`grep -v` per allow-
# list entry) silently passed when an allow-listed file shrank below
# the threshold. Architect empirically reproduced this on cycle 176:
# replaced pthreads-async/multi_worker.rs (allow-listed, 1048 LoC)
# with a 500-LoC stub; recipe passed with stale exemption in force.
# The positive-enumeration shape introduced cycle 177 closes that
# silent-correctness-loss arm.
#
# Threshold semantics: the check is `$1 > 1000`, STRICTLY greater. A
# file at exactly 1000 LoC passes (AC#5 of TASK-0340 reads "no file
# exceeds 1000 LoC"; "exceeds" = strictly more than). When an allow-
# listed file shrinks to exactly 1000, direction-(B) reports it as
# stale and the recipe FAILS — the implementer removes the entry.
#
# Why 1000? Natural reading-fatigue boundary — a file beyond ~1000
# lines stops fitting in a single editor view. The 800-LoC threshold
# the 2026-05-25 audit used is a softer warning; 1000 is the hard
# fence.
#
# Scope: walks `nucleus/backend-common/src`,
# `nucleus/nucleus-compiler/src`, `nucleus/backends/*/src`,
# `nucleus/driver/src`, and `nucleus/e2e/src` (e2e added cycle 190 /
# TASK-0342 — qa cycle-185b P3.1 surfaced the pre-cycle-190 exclusion as
# a documentation/expectation lag after TASK-0340 slice-10 carved
# e2e/src/main.rs from 7316→4716 LoC and added tests.rs at 2635 LoC,
# both still >1000 and neither in the fence; driver/src added TASK-0388
# after main.rs grew to 1242 LoC — the "widen when any grows past 1000"
# trigger below fired, and main.rs was split into args.rs + dispatch.rs
# to land it under the fence). The following sub-trees remain
# DELIBERATELY excluded:
#   - `nucleus/mp-tcp-common`, `nucleus/test-common`, `nucleus/nucleus`
#     — currently NO file >1000 LoC by coincidence of size, not by rule.
#     Widen the scope when any grows past 1000.
#
# Allow-list canonical reproducer: `find
# nucleus/{backend-common,nucleus-compiler,backends,driver,e2e}/src -name '*.rs'
# -exec wc -l {} \; | sort -rn | awk '$1 > 1000 {print $2}'`.
#
# Each entry is a TASK-0340 AC#2 split target — the allow-list shrinks
# as splits land. The allow-list is a printf-fed positional list — i.e.
# POSIX `printf '%s\n' a b c ...` with positional args, NOT a bash array
# and NOT a heredoc (a column-0 heredoc body crashes just's parser; see
# justfile-history). Adding/removing an entry is a one-line edit;
# per-file LoC numbers are deliberately NOT enumerated (architect
# cycle-176 P3.1 — they create drift debt with no automated guard).
#
# POSIX-shell portability (architect cycle-177 P1.1; TASK-0415 cycle-244):
# the recipe uses `comm -23` against TEMP FILES, not bash process
# substitution `<(...)`. `just` defaults to `/bin/sh -cu`; on
# dash/ash/busybox-sh `<(...)` would syntax-error before either direction
# runs, leaving the regression-fence silently absent. The temp-file form
# via `mktemp` + `trap "rm -f ... " EXIT` is POSIX-portable. The recipe
# also avoids `set -o pipefail`: although `pipefail` was added to POSIX in
# Issue 8 (2024) and modern dash / bash-compat busybox accept it
# (empirically verified cycle-244), it is NOT supported on pre-2024 or
# non-bash-compat shells (e.g. a busybox built without
# CONFIG_ASH_BASH_COMPAT — the busybox-sh named above), AND its exit
# status is never consumed here (results are read from the temp file via
# `comm`, so the `find|awk|sort` pipeline status is irrelevant). It bought
# nothing; dropped for consistency with the sibling fences, none of which
# use it.
#
# Allow-list rationale (cycle-262 additions; see the printf list below):
#   - nucleus-compiler/src/event.rs — the EventList contract types
#     (Event / FireBinding / DataSlice / IterTile + serde); pre-existing
#     >1000 LoC (1036 at HEAD), untouched by cycle-262. Split → TASK-0437.
# NOTE (feedback-cheap-subset-blind-to-structural-fences): all three were
# found RED at cycle-262 HEAD — the cheap pre-commit subset does not run
# this fence, so they crossed 1000 LoC silently over prior cycles.
# passes/block_transform.rs (the strip-mine tile/seq/inner pass) was the
# third original offender; its ~380-LoC inline #[cfg(test)] mod tests was
# carved out to passes/block_transform/tests.rs (TASK-0437), dropping it
# to ~664 LoC, so its allow-list entry was removed (direction-B guard).
check-mega-files:
    @echo "checking nucleus/**/src/*.rs for files exceeding 1000 LoC..."
    @set -eu; \
    oversized_f=$(mktemp); \
    allow_f=$(mktemp); \
    trap "rm -f $oversized_f $allow_f" EXIT; \
    find nucleus/backend-common/src nucleus/nucleus-compiler/src nucleus/backends/*/src nucleus/driver/src nucleus/e2e/src -name '*.rs' -exec wc -l {} \; \
        | awk '$1 > 1000 {print $2}' \
        | sort > $oversized_f; \
    printf '%s\n' \
        'nucleus/nucleus-compiler/src/passes/reuse_inference.rs' \
        'nucleus/nucleus-compiler/src/sched/lower.rs' \
        'nucleus/nucleus-compiler/src/algo/lower.rs' \
        'nucleus/nucleus-compiler/src/passes/host_data_relay_inject.rs' \
        'nucleus/nucleus-compiler/src/sched/ir.rs' \
        'nucleus/backends/pthreads-async/src/multi_worker.rs' \
        'nucleus/e2e/src/tests.rs' \
        'nucleus/nucleus-compiler/src/event.rs' \
        | sort > $allow_f; \
    new_megafile=$(comm -23 $oversized_f $allow_f); \
    stale_allow=$(comm -23 $allow_f $oversized_f); \
    fail=0; \
    if [ -n "$new_megafile" ]; then \
        echo "FAIL (direction A): new mega-file(s) >1000 LoC outside the allow-list:"; \
        echo "$new_megafile" | sed 's/^/  /'; \
        echo ""; \
        echo "Fix options (in order of preference):"; \
        echo "  1. Split the file into cohesive sub-modules along seams already named by its module-level docstring (TASK-0340 AC#2 — preferred)."; \
        echo "  2. If the file is a single coherent unit that genuinely needs to be large, add to the allow-list (printf list) at justfile:check-mega-files with a one-line rationale for why this file is exempt."; \
        echo "  3. If the growth is from one cycle's worth of feature additions that can be deferred, revert / postpone."; \
        echo ""; \
        fail=1; \
    fi; \
    if [ -n "$stale_allow" ]; then \
        echo "FAIL (direction B): stale allow-list entr(ies) — file no longer >1000 LoC (split landed, file deleted, or file shrank):"; \
        echo "$stale_allow" | sed 's/^/  /'; \
        echo ""; \
        echo "Fix: remove the entry from the allow-list (printf list) at justfile:check-mega-files. The allow-list is the project's record of what the team has knowingly accepted — entries that no longer apply must come off so the fence stays meaningful."; \
        echo ""; \
        fail=1; \
    fi; \
    if [ $fail -ne 0 ]; then \
        echo "(memory: feedback-comment-doc-lie-recurring — large files concentrate comment-doc-lie risk."; \
        echo " memory: feedback-opacity-gate-rot — direction-B stale-entry path added cycle 177 in response to architect cycle-176 P2.1 empirical reproduction; each grep -v / printf-list entry is a per-file opacity gate that rots silently when the underlying file shrinks below threshold.)"; \
        exit 1; \
    fi; \
    echo "OK: no non-allow-listed nucleus/**/src/*.rs file exceeds 1000 LoC; no allow-list entry is stale."

# Doc-honesty gate (TASK-0439): the README example-count claim must match
# the on-disk reality. Same shape as check-mega-files — filesystem truth
# vs a declared value, fails BOTH directions (declared > actual and
# actual > declared) with a remediation message naming the file + sentinel.
#
# Filesystem truth: count of nuc-nucleus/examples/NN-name/ directories.
# Declared values:  TWO surfaces in README.md, BOTH machine-checked so the
#   human-facing word cannot silently drift from the machine sentinel
#   (TASK-0439 review P2):
#     (a) prose digit  — the `NN worked` count in the examples bullet;
#     (b) sentinel     — `<!-- check-readme-counts: examples=NN ... -->`.
#   All three (dir count, prose digit, sentinel) must agree.
#
# SCOPE: this polices ONLY the README example-count claim against the dir
# count. It deliberately does NOT police PRD §9 — §9 is an intentionally
# CURATED 14-row driving-example table (≠ 21 shipped dirs), so checking it
# against the directory count would be a guaranteed false-positive.
check-readme-counts:
    @echo "checking README.md example counts (prose + sentinel) against nuc-nucleus/examples/ dir count..."
    @set -eu; \
    actual=$(find nuc-nucleus/examples -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' '); \
    declared=$(grep -oE '<!-- check-readme-counts: examples=[0-9]+' README.md | grep -oE '[0-9]+$' || true); \
    prose=$(grep -oE '[0-9]+ worked' README.md | grep -oE '^[0-9]+' | head -n1 || true); \
    if [ -z "$declared" ]; then \
        echo "FAIL: no machine-readable count sentinel found in README.md."; \
        echo "  Expected a line of the form:"; \
        echo "    <!-- check-readme-counts: examples=$actual ... -->"; \
        echo "  Add it next to the examples bullet in README.md's Pointers section."; \
        exit 1; \
    fi; \
    if [ -z "$prose" ]; then \
        echo "FAIL: no greppable prose count ('NN worked examples') found in README.md."; \
        echo "  The examples bullet must lead with a digit, e.g. '$actual worked'."; \
        exit 1; \
    fi; \
    if [ "$declared" != "$actual" ] || [ "$prose" != "$actual" ]; then \
        echo "FAIL: README example-count drift."; \
        echo "  prose    (README.md 'NN worked'): $prose"; \
        echo "  sentinel (README.md comment):     $declared"; \
        echo "  actual   (nuc-nucleus/examples/ dirs): $actual"; \
        echo ""; \
        echo "Fix: update BOTH the prose digit ('$actual worked examples') AND the sentinel"; \
        echo "  <!-- check-readme-counts: examples=$actual ... -->"; \
        echo "in README.md to $actual, OR reconcile the examples/ directory set."; \
        exit 1; \
    fi; \
    echo "OK: README prose ($prose) + sentinel ($declared) match nuc-nucleus/examples/ dir count ($actual)."

# Tier-3 compile-only acceptance. Three arms, all real
# `cargo check --target thumbv7em-none-eabihf` cross-compiles against
# the stub shim. SUCCEEDS iff every generated lib cross-compiles.
#
#   1. M9 single-worker (TASK-0047 AC#4): the `embedded-pattern` no_std
#      lib for examples 1 + 5 (their naive schedules) at the out_dir
#      root.
#   2. M11 backend slice A multi-worker (TASK-0049.04): 02-split-add's
#      `split` schedule (host + w0, sync transfers a/b/c) emits ONE
#      no_std lib PER worker under out_dir/<worker>/, with Push/Wait/Sync
#      lowered to the stub-shim hooks (dma_push/dma_wait/irq_barrier).
#      This is the no_std-clean fixture that isolates the structural
#      backend change.
#   3. M11 backend slice A follow-up (TASK-0049.06): the REAL example-14
#      hearing-aid, via the SYNCHRONOUS sibling schedule
#      `embedded_multimcu_sync` (3 default-class workers fe/dsp/rf, sync
#      transfers, no async/buffer/event/named-regions) + the no_std-clean
#      `kernels.embedded.rs` (`--kernels`; mix2/denoise as fixed-array
#      `[i32; 16]` in/out). Three per-worker no_std libs cross-compile,
#      exercising array-typed pure-kernel Fire lowering (fixed-array
#      `.try_into()` args, alloc-free). The ASYNC `embedded_multimcu`
#      schedule is deliberately NOT cross-compiled — its async/buffer=3/
#      notify=event/heterogeneous-class demands are correctly REJECTED at
#      check_schedule_compat against the synchronous stub (admitting them
#      would be a capability lie). The async transport + the BIN/Renode
#      multi-MCU path are the separate slice TASK-0049.05.
#
# MUST be run under the embedded cross-compile shell, which provides the
# thumbv7em-none-eabihf rust-std on the pinned 1.83.0 toolchain:
#
#     nix develop .#embedded --command just check-embedded
#
# DELIBERATELY NOT wired into `just ci` / `just e2e`: the DEFAULT dev
# shell has NO thumbv7em-none-eabihf std (only `.#embedded` does), so the
# cross-check would hard-fail there. Same "tier-3 checks live outside the
# default tier-1 ci" rule TASK-0223 established for the Renode runtime.
# The embedded-pattern backend is likewise NOT in `e2e-matrix.toml`'s
# `backends` list — that list drives the tier-1 bit-identical RUNTIME
# differential (it runs + diffs host binaries), which is meaningless for
# a compile-only no_std backend (PRD §10.3 point 2 / §11 M9).
#
# The M9 example set (1 + 5) is fixed by PRD §11 M9 / TASK-0047 AC#4 (the
# two examples most representative of embedded workloads — elementwise +
# stencil). It is NOT per-example recipe bloat (PRD §12.3 anti-bloat):
# this is one milestone gate. M10 (Renode runtime, TASK-0048) extends it
# to a run-and-diff; until then compile-only is the bar (PRD §10.3
# point 5).
check-embedded:
    @echo "tier-3 M9 compile-only acceptance (embedded-pattern, TASK-0047 AC#4)"
    @set -eu; \
    cd nucleus && cargo build --release --bin nucleus --quiet; \
    for ex in 01-elementwise-add 05-stencil; do \
        out="target/embedded-m9/$ex"; \
        rm -rf "$out"; \
        echo "=== generating embedded-pattern no_std lib for $ex/naive ==="; \
        ./target/release/nucleus build \
            --algo "../nuc-nucleus/examples/$ex/prog.algo.nuc" \
            --sched "../nuc-nucleus/examples/$ex/schedules/naive.sched.nuc" \
            --backend embedded-pattern \
            --out "$out"; \
        echo "=== cargo check --target thumbv7em-none-eabihf ($ex) ==="; \
        (cd "$out" && cargo check --target thumbv7em-none-eabihf); \
    done; \
    echo "OK: embedded-pattern no_std lib cross-compiles for examples 1 + 5 (thumbv7em-none-eabihf)."; \
    echo ""; \
    echo "=== M11 backend slice A: multi-worker compile-only LIB (TASK-0049.04) ==="; \
    mout="target/embedded-m11/02-split-add"; \
    rm -rf "$mout"; \
    echo "=== generating per-worker no_std libs for 02-split-add/split (2 workers) ==="; \
    ./target/release/nucleus build \
        --algo "../nuc-nucleus/examples/02-split-add/prog.algo.nuc" \
        --sched "../nuc-nucleus/examples/02-split-add/schedules/split.sched.nuc" \
        --backend embedded-pattern \
        --out "$mout"; \
    for w in host w0; do \
        if [ ! -d "$mout/$w" ]; then \
            echo "FAIL: expected per-worker project $mout/$w (TASK-0049.04 N-projects layout)"; \
            exit 1; \
        fi; \
        echo "=== cargo check --target thumbv7em-none-eabihf (02-split-add/$w) ==="; \
        (cd "$mout/$w" && cargo check --target thumbv7em-none-eabihf); \
    done; \
    echo "OK: embedded-pattern emits + cross-compiles N per-worker no_std libs"; \
    echo "    with stub-shim Push/Wait/Sync transport (TASK-0049.04, thumbv7em-none-eabihf)."; \
    echo ""; \
    echo "=== M11 backend slice A follow-up: REAL example-14 multi-MCU LIB (TASK-0049.06) ==="; \
    eout="target/embedded-m11-ex14"; \
    rm -rf "$eout"; \
    echo "=== generating 3 per-worker no_std libs for 14-hearing-aid/embedded_multimcu_sync (fe/dsp/rf) ==="; \
    ./target/release/nucleus build \
        --algo "../nuc-nucleus/examples/14-hearing-aid/prog.embedded.algo.nuc" \
        --sched "../nuc-nucleus/examples/14-hearing-aid/schedules/embedded_multimcu_sync.sched.nuc" \
        --backend embedded-pattern \
        --kernels "../nuc-nucleus/examples/14-hearing-aid/kernels.embedded.rs" \
        --out "$eout"; \
    for w in fe dsp rf; do \
        if [ ! -d "$eout/$w" ]; then \
            echo "FAIL: expected per-worker project $eout/$w (TASK-0049.06 real-ex14 3-MCU layout)"; \
            exit 1; \
        fi; \
        echo "=== cargo check --target thumbv7em-none-eabihf (14-hearing-aid/$w) ==="; \
        (cd "$eout/$w" && cargo check --target thumbv7em-none-eabihf); \
    done; \
    echo "OK: embedded-pattern cross-compiles the REAL example-14 multi-MCU"; \
    echo "    hearing-aid (fe/dsp/rf, array-typed mix2/denoise pure kernels,"; \
    echo "    no_std-clean fixed-array args) — TASK-0049.06, thumbv7em-none-eabihf."; \
    echo ""; \
    echo "=== SECOND MCU FAMILY: nRF52840 single-worker BIN (P10, TASK-0453.10) ==="; \
    for ex in 01-elementwise-add 05-stencil 09-producer-consumer; do \
        nout="target/embedded-nrf/$ex"; \
        rm -rf "$nout"; \
        echo "=== generating embedded-pattern nRF52840 bin for $ex/naive (--shim nrf52840) ==="; \
        ./target/release/nucleus build \
            --algo "../nuc-nucleus/examples/$ex/prog.algo.nuc" \
            --sched "../nuc-nucleus/examples/$ex/schedules/naive.sched.nuc" \
            --backend embedded-pattern --shim nrf52840 \
            --out "$nout"; \
        echo "=== cargo check --target thumbv7em-none-eabihf (nrf52840 $ex) ==="; \
        (cd "$nout" && cargo check --target thumbv7em-none-eabihf); \
    done; \
    echo "OK: embedded-pattern cross-compiles the SECOND MCU family (nRF52840"; \
    echo "    Cortex-M4F, UARTE EasyDMA shim) for examples 1/5/9 — the same"; \
    echo "    NucleusShim trait + generic run<S>, swapping only the concrete"; \
    echo "    shim/memory-map/linker (P10, TASK-0453.10, thumbv7em-none-eabihf)."

# Tier-2 (M7) rsmpi build+run smoke (TASK-0063 AC#3). Builds the
# hand-written tests/mpi/rsmpi-smoke crate under the `.#mpi` shell
# (which provides OpenMPI + the libclang/bindgen build deps the `mpi`
# crate needs) and runs it under a localhost `mpiexec -n 2`. It proves
# the whole tier-2 foundation links end-to-end BEFORE the mpi-blocking
# backend (TASK-0045) emits a line: rsmpi compiles against the provided
# OpenMPI, MPI_Init/Finalize + Comm_rank/size + a blocking Send/Recv
# link and run, and the localhost SPMD launcher works (PRD §10.2).
# Self-contained: it enters `.#mpi` itself, so it runs from the default
# shell:  just check-mpi-smoke
#
# DELIBERATELY NOT wired into `just ci`: the DEFAULT dev shell has NO
# MPI (only `.#mpi` does), so this would hard-fail there — same
# tier-2/3-outside-default-ci rule as check-embedded / renode-*
# (TASK-0223). `--oversubscribe` lets the 2 ranks share however few
# cores the sandbox exposes. Fails LOUD if the rank-1 Send/Recv
# verification line is absent.
check-mpi-smoke:
    @echo "tier-2 M7 rsmpi build+run smoke (.#mpi, TASK-0063 AC#3)"
    @set -eu; \
    sm="$(pwd)/tests/mpi/rsmpi-smoke"; \
    out="$(mktemp)"; \
    trap 'rm -f "$out"' EXIT; \
    echo "=== building rsmpi-smoke (.#mpi) ==="; \
    nix develop .#mpi --command bash -c "cd '$sm' && cargo build --release --quiet"; \
    echo "=== running under localhost mpiexec -n 2 ==="; \
    nix develop .#mpi --command bash -c "mpiexec --oversubscribe -n 2 '$sm/target/release/rsmpi-smoke'" >"$out" 2>&1; \
    sort "$out"; \
    if grep -q 'rank 1 received sentinel OK' "$out"; then \
        echo "OK: rsmpi compiles against OpenMPI and a localhost -n 2 Send/Recv runs (M7 foundation verified)."; \
    else \
        echo "FAIL: expected 'rank 1 received sentinel OK' from the -n 2 launch (rsmpi/OpenMPI Send/Recv broken)"; \
        exit 1; \
    fi

# Tier-2 (M7/M8) MPI sub-communicator barrier build+run smoke
# (TASK-0045.02 AC#3). The COLLECTIVE sibling of check-mpi-smoke: builds
# the hand-written tests/mpi/barrier-smoke crate under the `.#mpi` shell
# and runs it under a localhost `mpiexec -n 3`. It proves the EXACT
# Comm_split + sub-communicator-barrier surface the two MPI backends'
# shared substrate (`backend_common::mpi_plan`) emits for a STRICT-SUBSET
# (host-excluding) `Event::Sync`: every rank calls `MPI_Comm_split`
# COLLECTIVELY, the excluded host (rank 0) passes `Color::undefined()`
# (-> None, no sub-comm barrier), the >=2 compute participants share a
# color, land in one sub-communicator, and `MPI_Barrier` on IT. The run
# is wrapped in `timeout` so a broken split (host wrongly joined the
# sub-group, or a participant skipped the collective) deadlocks and fails
# LOUD instead of hanging. Self-contained: it enters `.#mpi` itself, so
# it runs from the default shell:  just check-mpi-barrier-smoke
#
# DELIBERATELY NOT wired into `just ci`: the DEFAULT dev shell has NO MPI
# (only `.#mpi` does) — same tier-2/3-outside-default-ci rule as
# check-mpi-smoke / check-mpi / check-embedded / renode-* (TASK-0223).
# `--oversubscribe` lets the 3 ranks share however few cores the sandbox
# exposes.
check-mpi-barrier-smoke:
    @echo "tier-2 M7/M8 MPI Comm_split + sub-comm barrier smoke (.#mpi, TASK-0045.02 AC#3)"
    @set -eu; \
    sm="$(pwd)/tests/mpi/barrier-smoke"; \
    out="$(mktemp)"; \
    trap 'rm -f "$out"' EXIT; \
    echo "=== building barrier-smoke (.#mpi) ==="; \
    nix develop .#mpi --command bash -c "cd '$sm' && cargo build --release --quiet"; \
    echo "=== running under localhost mpiexec -n 3 (1 excluded host + 2 compute participants) ==="; \
    nix develop .#mpi --command bash -c "timeout 60 mpiexec --oversubscribe -n 3 '$sm/target/release/barrier-smoke'" >"$out" 2>&1 \
        || { echo "FAIL: barrier-smoke run failed/timed out under -n 3 (Comm_split + sub-comm barrier deadlock?)"; sort "$out"; exit 1; }; \
    sort "$out"; \
    if grep -q 'host world-rank 0 excluded from the compute barrier' "$out" \
        && [ "$(grep -c 'compute participant world-rank .* barrier OK' "$out")" -eq 2 ]; then \
        echo "OK: MPI_Comm_split + sub-communicator MPI_Barrier link and run host-excluding under -n 3 (TASK-0045.02 collective verified)."; \
    else \
        echo "FAIL: expected the excluded-host line + 2 compute-participant barrier-OK lines from the -n 3 launch (Comm_split/sub-comm barrier broken)"; \
        exit 1; \
    fi

# Tier-2 (M7) mpi-blocking acceptance (TASK-0045/.01). For each
# mpi-blocking cell of the e2e-matrix (see SCOPE), generates the SPMD MPI
# project from the example's schedule, cross-builds it under the `.#mpi`
# shell (rsmpi + OpenMPI), runs it under a localhost `mpiexec -n N`, and
# `cmp`s the output BYTE-EXACT against the example's committed
# reference.bin. This is STRONGER than the PRD §7.4 tier-2 ship bar
# (COMPILE): a simulator IS available (localhost MPI per §10.2), so we
# assert value-correctness too (§7.4 "where simulators ... exist, produce
# reference-matching output"). Self-contained: it enters `.#mpi` itself,
# so it runs from the default shell:  just check-mpi
#
# DELIBERATELY NOT wired into `just ci` and EXCLUDED from
# e2e-matrix.toml's `backends`: the generated project needs the `.#mpi`
# shell (the DEFAULT shell + the tier-1 bit-identical e2e RUNTIME matrix
# have no MPI). Same tier-2/3-outside-default-ci rule as check-embedded /
# renode-* (TASK-0223). The mpi-blocking BACKEND crate itself IS built by
# `just ci` (it is a normal std workspace member that only emits strings).
#
# SCOPE — DERIVED, not duplicated (TASK-0454): the (example, schedule)
# set is the `[[required]]` mpi-blocking cells of e2e-matrix.toml, read
# via `scripts/mpi-cells.py` (the SAME surface the counted `just e2e-mpi`
# differential drives — the two MPI coverage surfaces can no longer
# silently drift). The per-cell rank count N is derived from each
# schedule's declared `workers = { ... }` set (host counts as a rank), so
# there is no second hardcoded `n` table either. The script FAILS LOUD
# (non-zero, before any build) if the matrix names a schedule with no
# file on disk — the divergence-rot tripwire. Every run uses `mpiexec -n
# N` with N = the used-worker count, the WORST case (ALL ranks live; NOT
# -n 1, which hides Send/Recv ordering bugs — memory `16-jacobi`:
# deadlock-free != value-correct), wrapped in `timeout` so a
# standard-mode-send deadlock fails LOUD instead of hanging. (All
# mpi-blocking cells are sync schedules; the ASYNC-only distributed
# schedules belong to check-mpi-nonblocking — the driver's capability
# gate hard-rejects them here.)
# The driver release binary is built once up front so generation is cheap.
check-mpi:
    @echo "tier-2 M7 mpi-blocking acceptance (.#mpi, matrix-derived cells, all ranks live, TASK-0045/.01/.0454)"
    @set -eu; \
    nix develop .#mpi --command bash -c '\
        set -eu; \
        cd nucleus && cargo build --release --bin nucleus --quiet && cd ..; \
        cells="$(python3 scripts/mpi-cells.py nuc-nucleus/e2e-matrix.toml nuc-nucleus/examples mpi-blocking)"; \
        echo "--- matrix-derived mpi-blocking cells (example schedule n) ---"; echo "$cells"; \
        while IFS="$(printf "\t")" read -r ex sc n; do \
            out="nucleus/target/mpi-m7/$ex--$sc"; \
            rm -rf "$out"; \
            echo "=== generating mpi-blocking SPMD project for $ex/$sc (n=$n) ==="; \
            ./nucleus/target/release/nucleus build \
                --algo "nuc-nucleus/examples/$ex/prog.algo.nuc" \
                --sched "nuc-nucleus/examples/$ex/schedules/$sc.sched.nuc" \
                --backend mpi-blocking --out "$out" >/dev/null; \
            echo "=== cargo build --release ($ex/$sc) ==="; \
            ( cd "$out" && cargo build --release --quiet ); \
            echo "=== mpiexec -n $n (all ranks live) + byte-exact cmp ($ex/$sc) ==="; \
            o="$(mktemp)"; \
            NUC_INPUT_PATH="nuc-nucleus/examples/$ex/input.bin" NUC_OUTPUT_PATH="$o" \
                timeout 120 mpiexec --oversubscribe -n "$n" "$out/target/release/nuc-generated" \
                || { echo "FAIL: $ex/$sc run failed/timed out under -n $n (deadlock?)"; rm -f "$o"; exit 1; }; \
            if cmp -s "$o" "nuc-nucleus/examples/$ex/reference.bin"; then \
                echo "OK: $ex/$sc SPMD output is byte-exact vs reference.bin (mpiexec -n $n)"; \
            else \
                echo "FAIL: $ex/$sc SPMD output differs from reference.bin"; rm -f "$o"; exit 1; \
            fi; \
            rm -f "$o"; \
        done < <(printf "%s\n" "$cells"); \
        echo "OK: mpi-blocking value-correct — all matrix-derived cells byte-exact under mpiexec -n N (all ranks live)."'

# Tier-2 (M8) mpi-nonblocking acceptance (TASK-0046). The mpi-nonblocking
# backend widens the capability surface to async + buffer + notify=event
# and lowers Push to a non-blocking BUFFERED send (MPI_Ibsend, local
# completion) + Wait to a non-blocking receive (MPI_Imrecv/Irecv) +
# explicit MPI_Wait. That admits the async/buffered schedules mpi-blocking
# HARD-REJECTS at the capability gate. For each ASYNC target it generates
# the SPMD MPI project, cross-builds under `.#mpi`, runs it under a
# localhost `mpiexec -n N` (all ranks live — the WORST case; -n 1 hides
# ordering bugs, memory `16-jacobi`: deadlock-free != value-correct), and
# `cmp`s the output BYTE-EXACT against the example's committed
# reference.bin (a single-pass box blur / game-of-life step is invariant
# under the partition shape, so the same oracle applies). Self-contained;
# runs from the default shell:  just check-mpi-nonblocking
#
# TWO arms per target, to defeat the buffer-lifetime TIMING-LUCK trap
# (TASK-0046 AC#5; a use-after-free can pass by luck when the MPI eager
# protocol copies the send buffer immediately and masks a premature drop):
#  (1) DEFAULT eager protocol, `mpiexec -n N`.
#  (2) FORCED RENDEZVOUS: the same run with every BTL eager limit driven
#      to 128 bytes (`btl_{sm,self,vader,tcp}_eager_limit=128`; 0 is below
#      OpenMPI's self-BTL minimum of 80, 128 is below every array transfer
#      here which is >= 256 bytes), so the array messages take the
#      rendezvous path that reads the send buffer LATER. Buffered Ibsend
#      copies into the attached buffer regardless, so a correct backend
#      passes BOTH arms identically. NOTE: buffered Ibsend is structurally
#      immune to the eager-masks-use-after-free trap (the payload is
#      copied into the Universe-owned attach buffer before Wait returns),
#      so this arm's residual value is (a) no deadlock under rendezvous and
#      (b) the attach buffer holds every in-flight message through the
#      longer rendezvous handshake — NOT UAF coverage (which would matter
#      for standard MPI_Isend; see TASK-0046 notes).
# Each run is wrapped in `timeout` so a deadlock fails LOUD, not hangs.
#
# TARGETS — DERIVED, not duplicated (TASK-0454): the (example, schedule)
# set is the `[[required]]` mpi-nonblocking cells of e2e-matrix.toml,
# read via `scripts/mpi-cells.py` (the SAME surface the counted `just
# e2e-mpi` differential drives — the two MPI coverage surfaces can no
# longer silently drift). The per-cell rank count N is derived from each
# schedule's declared `workers = { ... }` set (host counts as a rank), so
# there is no second hardcoded `n` table; the script FAILS LOUD (non-zero,
# before any build) if the matrix names a schedule with no file on disk —
# the divergence-rot tripwire. The four async schedules, with N = the
# used-worker count, all ranks live:
#   05-stencil/distributed     (n=5, host + w0..w3; async img_in broadcast)
#   05-stencil/distributed-2d  (n=5; 2x2 grid + real worker<->worker halo)
#   09-producer-consumer/pipelined (n=3, host + producer + consumer; carries
#     a non-whole-world barrier {producer,consumer} EXCLUDING the host ->
#     MPI_Comm_split + sub-communicator barrier landed in TASK-0045.02,
#     wired here by TASK-0046.01; closes TASK-0046 AC#3 [examples 9 AND 11])
#   11-game-of-life/pipelined  (n=2, host + compute; async grid).
#
# DELIBERATELY NOT wired into `just ci` and EXCLUDED from e2e-matrix.toml's
# `backends`: the generated project needs the `.#mpi` shell (same tier-2-
# outside-default-ci rule as check-mpi / check-embedded / renode-*). The
# BACKEND crate itself IS built by `just ci` (a normal std member emitting
# strings).
check-mpi-nonblocking:
    @echo "tier-2 M8 mpi-nonblocking acceptance (.#mpi, matrix-derived async cells, default + forced-rendezvous, TASK-0046/.0454)"
    @set -eu; \
    nix develop .#mpi --command bash -c '\
        set -eu; \
        cd nucleus && cargo build --release --bin nucleus --quiet && cd ..; \
        rndv="--mca btl_sm_eager_limit 128 --mca btl_self_eager_limit 128 --mca btl_vader_eager_limit 128 --mca btl_tcp_eager_limit 128"; \
        cells="$(python3 scripts/mpi-cells.py nuc-nucleus/e2e-matrix.toml nuc-nucleus/examples mpi-nonblocking)"; \
        echo "--- matrix-derived mpi-nonblocking cells (example schedule n) ---"; echo "$cells"; \
        while IFS="$(printf "\t")" read -r ex sc n; do \
            out="nucleus/target/mpi-m8/$ex--$sc"; \
            rm -rf "$out"; \
            echo "=== generating mpi-nonblocking SPMD project for $ex/$sc (n=$n) ==="; \
            ./nucleus/target/release/nucleus build \
                --algo "nuc-nucleus/examples/$ex/prog.algo.nuc" \
                --sched "nuc-nucleus/examples/$ex/schedules/$sc.sched.nuc" \
                --backend mpi-nonblocking --out "$out" >/dev/null; \
            echo "=== cargo build --release ($ex/$sc) ==="; \
            ( cd "$out" && cargo build --release --quiet ); \
            for arm in "default:" "rendezvous:$rndv"; do \
                label="${arm%%:*}"; mca="${arm#*:}"; \
                echo "=== mpiexec -n $n ($label protocol) + byte-exact cmp ($ex/$sc) ==="; \
                o="$(mktemp)"; \
                NUC_INPUT_PATH="nuc-nucleus/examples/$ex/input.bin" NUC_OUTPUT_PATH="$o" \
                    timeout 120 mpiexec --oversubscribe $mca -n "$n" "$out/target/release/nuc-generated" \
                    || { echo "FAIL: $ex/$sc $label run failed/timed out under -n $n (deadlock?)"; rm -f "$o"; exit 1; }; \
                if cmp -s "$o" "nuc-nucleus/examples/$ex/reference.bin"; then \
                    echo "OK: $ex/$sc $label output byte-exact vs reference.bin (mpiexec -n $n)"; \
                else \
                    echo "FAIL: $ex/$sc $label output differs from reference.bin"; rm -f "$o"; exit 1; \
                fi; \
                rm -f "$o"; \
            done; \
        done < <(printf "%s\n" "$cells"); \
        echo "OK: mpi-nonblocking value-correct + deadlock-immune — matrix-derived async cells (-n N, all ranks live) x {default, forced-rendezvous}."'

# Tier-3 M10 firmware -> Renode -> UART template (TASK-0048). Builds the
# minimal STM32H7 (Cortex-M7) no_std UART firmware under tests/renode/
# uart-smoke/, runs it headless in Renode on the bundled stm32h743
# platform, captures USART1 to a file, and ASSERTS the sentinel — failing
# LOUD on mismatch. Self-contained: it enters the .#embedded shell (for
# the thumbv7em cross-compile) then the .#renode shell (for the Renode
# runtime), so it runs from the default shell:  just renode-uart-smoke
#
# DELIBERATELY NOT wired into `just ci`: it needs the .#embedded +
# .#renode shells (heavy ARM-std + Mono closures) — same tier-3-outside-
# default-ci rule as check-embedded / TASK-0223. This is the lib->bin +
# UART-capture TEMPLATE the embedded-pattern backend's M10 codegen
# (TASK-0048) will follow; here the firmware's "computation" is a
# constant sentinel (no codegen yet — that is M10 proper).
renode-uart-smoke:
    @echo "tier-3 M10 firmware -> Renode -> UART smoke (TASK-0048)"
    @set -eu; \
    fw="$(pwd)/tests/renode/uart-smoke"; \
    elf="$fw/target/thumbv7em-none-eabihf/release/uart-smoke"; \
    out="$(mktemp)"; log="$(mktemp)"; \
    trap 'rm -f "$out" "$log"' EXIT; \
    echo "=== cross-compiling firmware (.#embedded) ==="; \
    nix develop .#embedded --command bash -c "cd '$fw' && cargo build --release --quiet"; \
    echo "=== running in Renode (.#renode), capturing USART1 ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        -e "\$bin=@$elf" -e "\$uartFile=@$out" -e "include @$fw/run.resc" >"$log" 2>&1; \
    echo "=== captured USART1 ==="; cat "$out"; \
    if grep -q 'NUCLEUS-M10-OK' "$out"; then \
        echo "OK: Renode captured the firmware's USART1 sentinel (M10 lib->bin + capture template verified)."; \
    else \
        echo "FAIL: expected sentinel 'NUCLEUS-M10-OK' not found in captured USART1 output"; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Tier-3 M10 AC#1 DE-RISK: DMA-driven firmware -> Renode -> UART smoke
# (TASK-0048.11). Sibling of renode-uart-smoke, but the firmware under
# tests/renode/dma-uart-smoke/ emits over USART1 via a DMA1
# MemoryToPeripheral transfer (into USART1's TDR) instead of a polled CPU
# store loop. It PROVES, before the real async STM32H7 DMA shim (parent
# TASK-0048 AC#1) touches the working synchronous Usart1Shim, that Renode's
# bundled stm32h743 model drives DMA-to-peripheral USART1 TX end-to-end.
# Same tier-3-outside-default-ci rule (needs .#embedded + .#renode); a
# multi-char sentinel proves the DMA wrote every byte to the fixed TDR
# (non-incrementing destination), not just the first.
renode-dma-uart-smoke:
    @echo "tier-3 M10 AC#1 de-risk: DMA-driven firmware -> Renode -> UART smoke (TASK-0048.11)"
    @set -eu; \
    fw="$(pwd)/tests/renode/dma-uart-smoke"; \
    elf="$fw/target/thumbv7em-none-eabihf/release/dma-uart-smoke"; \
    out="$(mktemp)"; log="$(mktemp)"; \
    trap 'rm -f "$out" "$log"' EXIT; \
    echo "=== cross-compiling firmware (.#embedded) ==="; \
    nix develop .#embedded --command bash -c "cd '$fw' && cargo build --release --quiet"; \
    echo "=== running in Renode (.#renode), capturing USART1 ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        -e "\$bin=@$elf" -e "\$uartFile=@$out" -e "include @$fw/run.resc" >"$log" 2>&1; \
    echo "=== captured USART1 ==="; cat "$out"; \
    if grep -q 'NUC-DMA-OK' "$out"; then \
        echo "OK: Renode captured the DMA-transmitted USART1 payload (real-DMA USART1 TX is modellable; M10 AC#1 de-risk GO)."; \
    else \
        echo "FAIL: expected DMA payload 'NUC-DMA-OK' not found in captured USART1 output"; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Tier-3 M11 inter-MCU transport DE-RISK: two co-simulated STM32H7 MCUs
# wired by a UARTHub (TASK-0049.01). The sender bin transmits a sentinel
# over USART1 -> CreateUARTHub -> the receiver bin reads it off the hub
# (USART1 RX) and relays it out USART2, which the .resc captures. A
# captured sentinel proves Renode models WIRED MCU-to-MCU transport
# end-to-end with our own firmware — the gating prerequisite for M11
# cross-MCU codegen (parent TASK-0049). NB Renode has NO MCU-to-MCU SPI
# link, so M11's interconnect is a UART hub (user decision, TASK-0049.01).
# Same tier-3-outside-default-ci rule (needs .#embedded + .#renode).
renode-multimcu-uart-smoke:
    @echo "tier-3 M11 inter-MCU de-risk: 2x STM32H7 + UARTHub -> Renode -> relay capture (TASK-0049.01)"
    @set -eu; \
    fw="$(pwd)/tests/renode/multimcu-uart-smoke"; \
    snd="$fw/target/thumbv7em-none-eabihf/release/sender"; \
    rcv="$fw/target/thumbv7em-none-eabihf/release/receiver"; \
    out="$(mktemp)"; log="$(mktemp)"; \
    trap 'rm -f "$out" "$log"' EXIT; \
    echo "=== cross-compiling both firmwares (.#embedded) ==="; \
    nix develop .#embedded --command bash -c "cd '$fw' && cargo build --release --quiet"; \
    echo "=== running 2-machine co-sim in Renode (.#renode), capturing receiver USART2 ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        -e "\$senderBin=@$snd" -e "\$receiverBin=@$rcv" -e "\$uartFile=@$out" -e "include @$fw/run.resc" >"$log" 2>&1; \
    echo "=== captured receiver USART2 (relayed from the hub) ==="; cat "$out"; \
    if grep -q 'M11-LINK-OK' "$out"; then \
        echo "OK: the sentinel crossed the inter-MCU UARTHub and the receiver relayed it (M11 inter-MCU transport de-risk GO)."; \
    else \
        echo "FAIL: expected sentinel 'M11-LINK-OK' not found in the receiver's relay output"; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Tier-3 M11 GENERATED multi-MCU firmware -> Renode co-sim -> reference.bin
# diff (TASK-0049.05). Unlike renode-multimcu-uart-smoke (a hand-written
# 2-MCU sentinel relay), this GENERATES one no_std firmware per worker of a
# MULTI-worker schedule via the embedded-pattern bin-emit mode (`--shim
# stm32h7`), cross-compiles each under .#embedded, and co-simulates them as
# N STM32H7 machines wired by the GENERATED multimcu.resc under .#renode —
# REAL inter-MCU UART-hub transport (link_push = USART TX, link_recv =
# blocking USART RX), receivers-first start-gating. Each SAVER worker writes
# its USART1 to its own file backend ($<worker>Uart; single-saver schedules
# keep $uartFile for recipe-compat); the recipe reads the emitted
# output_captures.txt manifest, injects one var per saver pointing at a
# DISTINCT temp file, then CONCATENATES the per-saver files in MANIFEST order
# (= TransportPlan.output_captures decl-order) and `cmp`s the concatenation
# BYTE-EXACT against the example's reference.bin (PRD §10.3 point 3 value-
# correctness, end-to-end across real co-simulated MCUs).
#
# PARAMETERISED (PRD §12.3): positional EX (example dir) + SCHED (schedule
# stem under schedules/), defaulting to the proven 02-split-add/split
# (host+w0). The worker bin var names ($<worker>Bin) are derived from the
# generated per-worker project dirs, so the recipe is worker-name-agnostic.
# The ALGO + KERNELS are resolved FROM the schedule's `schedule for "<path>"`
# directive (relative to schedules/) + the repo kernels-naming convention
# (prog.algo.nuc->kernels.rs, prog.embedded.algo.nuc->kernels.embedded.rs),
# mirroring the e2e harness — so ex14 drives prog.embedded.algo.nuc /
# kernels.embedded.rs while 02-split-add stays on the default pair.
#   just renode-multimcu                      # 02-split-add/split (default)
#   just renode-multimcu 02-split-add split   # explicit
#   just renode-multimcu 14-hearing-aid embedded_multimcu_sync  # multi-saver
#
# Self-contained (enters .#embedded then .#renode); DELIBERATELY NOT in
# `just ci` — same tier-3-outside-default-ci rule as renode-embedded /
# renode-multimcu-uart-smoke (needs the .#embedded + .#renode shells).
renode-multimcu EX="02-split-add" SCHED="split":
    @echo "tier-3 M11 GENERATED multi-MCU {{EX}}/{{SCHED}} -> Renode co-sim -> reference.bin diff (TASK-0049.05/0049.10.08)"
    @set -eu; \
    exdir="$(pwd)/nuc-nucleus/examples/{{EX}}"; \
    sched="$exdir/schedules/{{SCHED}}.sched.nuc"; \
    input="$exdir/input.bin"; reference="$exdir/reference.bin"; \
    gen="$(mktemp -d)"; out="$(mktemp)"; log="$(mktemp)"; capdir="$(mktemp -d)"; \
    trap 'rm -rf "$gen" "$capdir"; rm -f "$out" "$log"' EXIT; \
    echo "=== resolving algo + kernels from the schedule's \`schedule for\` directive ==="; \
    rel="$(grep -E '^[[:space:]]*schedule[[:space:]].*for' "$sched" | grep -v '^[[:space:]]*//' | head -n1 | sed -n 's/.*"\([^"]*\)".*/\1/p')"; \
    if [ -z "$rel" ]; then echo "FAIL: no \`schedule for \"...\"\` directive in $sched"; exit 1; fi; \
    algo="$exdir/schedules/$rel"; \
    base="$(basename "$algo")"; \
    case "$base" in \
        prog.algo.nuc) kfile="kernels.rs" ;; \
        prog.*.algo.nuc) variant="${base#prog}"; variant="${variant%.algo.nuc}"; kfile="kernels${variant}.rs" ;; \
        *) kfile="kernels.rs" ;; \
    esac; \
    kernels="$exdir/$kfile"; \
    if [ ! -f "$algo" ]; then echo "FAIL: resolved algo not found: $algo"; exit 1; fi; \
    if [ ! -f "$kernels" ]; then echo "FAIL: resolved kernels not found: $kernels"; exit 1; fi; \
    echo "    algo=$algo"; echo "    kernels=$kernels"; \
    echo "=== generating multi-MCU bins (embedded-pattern --shim stm32h7) ==="; \
    cd nucleus && cargo build --release --bin nucleus --quiet; \
    ./target/release/nucleus build \
        --algo "$algo" \
        --kernels "$kernels" \
        --sched "$sched" \
        --backend embedded-pattern --shim stm32h7 \
        --out "$gen"; \
    cd ..; \
    binargs=""; \
    for wdir in "$gen"/*/; do \
        w="$(basename "$wdir")"; \
        echo "=== cross-compiling worker $w (.#embedded) ==="; \
        nix develop .#embedded --command bash -c "cd '$wdir' && cargo build --release --quiet"; \
        elf="$wdir/target/thumbv7em-none-eabihf/release/nuc-embedded-generated"; \
        if [ ! -f "$elf" ]; then echo "FAIL: worker $w ELF not produced: $elf"; exit 1; fi; \
        binargs="$binargs -e \$${w}Bin=@$elf"; \
    done; \
    manifest="$gen/output_captures.txt"; \
    if [ ! -f "$manifest" ]; then echo "FAIL: emit produced no capture manifest $manifest"; exit 1; fi; \
    echo "=== capture manifest (per-saver file_var, decl-order concat order): ==="; cat "$manifest"; \
    capargs=""; capfiles=""; n=0; \
    while IFS= read -r fv; do \
        [ -z "$fv" ] && continue; \
        cf="$capdir/$fv.bin"; : >"$cf"; \
        capargs="$capargs -e \$${fv}=@$cf"; \
        capfiles="$capfiles $cf"; \
        n=$((n+1)); \
    done < "$manifest"; \
    if [ "$n" -eq 0 ]; then echo "FAIL: capture manifest $manifest is empty"; exit 1; fi; \
    echo "=== running multi-MCU co-sim in Renode (.#renode), capturing $n saver USART1 file backend(s) ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        $binargs $capargs -e "\$input=@$input" -e "include @$gen/multimcu.resc" >"$log" 2>&1; \
    : >"$out"; \
    for cf in $capfiles; do \
        sz="$(wc -c < "$cf")"; \
        echo "    saver capture $(basename "$cf"): $sz bytes"; \
        cat "$cf" >>"$out"; \
    done; \
    captured="$(wc -c < "$out")"; expected="$(wc -c < "$reference")"; \
    echo "=== concatenated $captured bytes over $n saver USART1 file backend(s) (reference.bin is $expected bytes) ==="; \
    if [ "$captured" -ne "$expected" ]; then \
        echo "FAIL: concatenated $captured bytes, expected exactly $expected (reference.bin). The co-sim did not stream the full output region across all savers."; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi; \
    if cmp -s "$out" "$reference"; then \
        echo "OK: multi-MCU co-sim concatenated saver output is BYTE-EXACT identical to reference.bin ($expected bytes) — M11 {{EX}}/{{SCHED}} value-correctness verified (PRD §10.3 point 3)."; \
    else \
        echo "FAIL: concatenated saver output differs from reference.bin (first differing byte):"; \
        cmp "$out" "$reference" || true; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Tier-3 M11 multi-MCU VALUE-CORRECTNESS standing gate (TASK-0049.10.08
# AC#4). Runs the proven `renode-multimcu` co-sims as ONE fail-loud
# command — a STANDING gate, not a remembered manual invocation:
#
#   1. 02-split-add/split   — 2 STM32H7 MCUs (host + w0), sync transfers;
#      BYTE-EXACT 1024B vs reference.bin (the original M11 multi-MCU proof).
#   2. 14-hearing-aid/embedded_multimcu_sync — the REAL 3-MCU (fe/dsp/rf)
#      hearing-aid pipeline; BYTE-EXACT 512B (spk_out 256 ++ bt_out 256)
#      vs reference.bin. Unblocked by the staged-release boot-order
#      fixpoint (TASK-0049.05.03) which resolved the rf frame-3 stall;
#      this proves ex14 per-frame VALUES are correct end-to-end.
#   3. 22-dma-pio-demo/dma_pio — 2 STM32H7 MCUs (host + w0) exercising
#      BOTH transport shapes in one app: two `mode=dma` edges (samples,
#      out) emit dma_link_arm/dma_link_recv_arm + completion spin, one
#      `mode=pio` edge (gains) emits link_push/link_recv; BYTE-EXACT
#      1024B vs reference.bin (TASK-0438.03, the DMA-vs-PIO demo). The
#      modes are value-equivalent over the SAME UART fabric so the app
#      stays byte-exact — value-correctness, NOT timing (AC#4).
#
# `just` runs prerequisite recipes before the body, so all three
# `renode-multimcu` invocations run; each fails LOUD (exit 1) on any
# byte-count/diff mismatch, which fails this gate. No new co-sim logic
# lives here — it composes the
# parameterised recipe (PRD §12.3 anti-bloat: one standing gate, not
# per-example one-offs).
#
# DELIBERATELY NOT wired into `just ci` / `just e2e`: the DEFAULT dev shell
# has NO thumbv7em-none-eabihf std and NO Renode closure (only `.#embedded`
# + `.#renode` do; `renode-multimcu` enters them itself). Same
# tier-3-outside-default-ci rule as check-embedded / renode-embedded
# (TASK-0223). The 7 `embedded_multimcu` cells in e2e-matrix.toml stay
# `[[skip]]` for the same reason — the standing gate is THIS Renode recipe
# path, NOT the e2e matrix. The ci.yml `renode-multimcu` job runs this gate
# best-effort (PRD §10.3); it is NOT part of the merge-blocking `just ci`.
renode-multimcu-gate: (renode-multimcu "02-split-add" "split") (renode-multimcu "14-hearing-aid" "embedded_multimcu_sync") (renode-multimcu "22-dma-pio-demo" "dma_pio")
    @echo "OK: tier-3 M11 multi-MCU value-correctness gate PASSED — 02-split-add 2-MCU 1024B + 14-hearing-aid 3-MCU 512B + 22-dma-pio-demo 2-MCU 1024B (DMA+PIO) all BYTE-EXACT vs reference.bin (TASK-0049.10.08 AC#4 + TASK-0438.03)."

# Tier-3 M10 GENERATED firmware -> Renode -> reference.bin diff
# (TASK-0048.01 emission + TASK-0048.02 value-correctness + TASK-0048.03
# generalisation to examples 1/5/9). Unlike `renode-uart-smoke` (a hand-
# written sentinel firmware), this GENERATES the chosen example's firmware
# via the embedded-pattern backend's bin-emit mode (`--shim stm32h7`),
# cross-compiles it under .#embedded, injects that example's input.bin into
# the emulated MCU's axiSram, runs it headless in Renode under .#renode on
# the bundled stm32h743 platform, captures the RAW USART1 output bytes, and
# `cmp`s them BYTE-EXACT against THAT example's reference.bin — failing LOUD
# on mismatch. This is PRD §10.3 point 3 ("captures output ... diffs
# against reference.bin. Must be bit-identical").
#
# PARAMETERISED over the example (PRD §12.3 anti-bloat: ONE recipe, not
# three near-duplicates). The single positional argument `EX` selects the
# PRD §11 M10 example directory under nuc-nucleus/examples/; it defaults
# to 01-elementwise-add so the bare `just renode-embedded` preserves the
# original ex1 gate behaviour. just takes the argument POSITIONALLY (not
# `EX=...`):
#   just renode-embedded                       # ex1 (default)
#   just renode-embedded 05-stencil            # ex5: 2D blur3 stencil
#   just renode-embedded 09-producer-consumer  # ex9: two-stage pipe
# The firmware emit is example-agnostic (it streams the save Fire's output
# region as raw bytes; the load Fire fills from the injected region); only
# the --algo/--sched generated and the reference.bin diffed differ per
# example. The expected byte count is derived from each example's
# reference.bin, so ex9's 64-byte output and ex1/ex5's 1024-byte output are
# both handled with no per-example length constant.
#
# REAL INPUT PATH (TASK-0048.02): the .resc does `sysbus LoadBinary
# @input.bin 0x24000000` into axiSram (mapped in the platform, NOT in the
# firmware's memory.x — so the linker never collides). The firmware's
# Usart1Shim::alloc_in_region reads sequential slices of that region; for
# the M10 examples every naive schedule has exactly ONE effectful load, so
# the cursor starts at 0 and consumes the whole injected region (the
# multi-load concatenation-order assumption is only exercised by ex1's two
# loads — TASK-0048.06).
#
# Self-contained: enters .#embedded for the thumbv7em cross-compile then
# .#renode for the runtime, so it runs from the default shell.
#
# DELIBERATELY NOT wired into `just ci` (needs the .#embedded + .#renode
# shells) — same tier-3-outside-default-ci rule as check-embedded /
# renode-uart-smoke / TASK-0223. The generated project is its own empty
# [workspace] in a scratch dir (cleaned on exit), so it never enters the
# nucleus/ host workspace.
renode-embedded EX="01-elementwise-add":
    @echo "tier-3 M10 GENERATED embedded-pattern {{EX}} -> Renode -> reference.bin diff (TASK-0048.03)"
    @set -eu; \
    resc="$(pwd)/tests/renode/embedded/run.resc"; \
    exdir="$(pwd)/nuc-nucleus/examples/{{EX}}"; \
    input="$exdir/input.bin"; \
    reference="$exdir/reference.bin"; \
    gen="$(mktemp -d)"; out="$(mktemp)"; log="$(mktemp)"; \
    trap 'rm -rf "$gen"; rm -f "$out" "$log"' EXIT; \
    echo "=== generating {{EX}} bin (embedded-pattern --shim stm32h7) ==="; \
    cd nucleus && cargo build --release --bin nucleus --quiet; \
    ./target/release/nucleus build \
        --algo "$exdir/prog.algo.nuc" \
        --sched "$exdir/schedules/naive.sched.nuc" \
        --backend embedded-pattern --shim stm32h7 \
        --out "$gen"; \
    cd ..; \
    elf="$gen/target/thumbv7em-none-eabihf/release/nuc-embedded-generated"; \
    echo "=== cross-compiling generated firmware (.#embedded) ==="; \
    nix develop .#embedded --command bash -c "cd '$gen' && cargo build --release --quiet"; \
    echo "=== running in Renode (.#renode): inject input.bin, capture USART1 ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        -e "\$bin=@$elf" -e "\$uartFile=@$out" -e "\$input=@$input" -e "include @$resc" >"$log" 2>&1; \
    captured="$(wc -c < "$out")"; expected="$(wc -c < "$reference")"; \
    echo "=== captured $captured bytes over USART1 (reference.bin is $expected bytes) ==="; \
    if [ "$captured" -ne "$expected" ]; then \
        echo "FAIL: captured $captured bytes, expected exactly $expected (reference.bin). The firmware did not stream the full output region."; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi; \
    if cmp -s "$out" "$reference"; then \
        echo "OK: captured USART1 bytes are BYTE-EXACT identical to reference.bin ($expected bytes) — M10 {{EX}} value-correctness verified (PRD §10.3 point 3)."; \
    else \
        echo "FAIL: captured USART1 output differs from reference.bin (first differing byte):"; \
        cmp "$out" "$reference" || true; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Tier-3 SECOND-MCU-FAMILY (nRF52840) GENERATED embedded-pattern firmware ->
# Renode -> reference.bin byte-exact diff (P10, TASK-0453.10). The sibling
# of `renode-embedded` (STM32H7): same example-agnostic generate ->
# cross-compile -> co-simulate -> byte-exact-diff flow, but `--shim nrf52840`
# emits a Nordic Cortex-M4F firmware whose UARTE0 EasyDMA transmit Renode
# captures. PROVES the NucleusShim hardware-abstraction trait is a real
# portability seam: the SAME generic backend + `run<S>` lower against a
# SECOND family by swapping only the concrete shim + memory map + linker
# config. Single-worker examples only (1/5/9 naive); nRF multi-MCU is future
# work. Self-contained (enters .#embedded then .#renode); DELIBERATELY NOT in
# `just ci` — same tier-3-outside-default-ci rule as renode-embedded.
renode-embedded-nrf EX="01-elementwise-add":
    @echo "tier-3 SECOND-FAMILY nRF52840 GENERATED embedded-pattern {{EX}} -> Renode -> reference.bin diff (P10, TASK-0453.10)"
    @set -eu; \
    resc="$(pwd)/tests/renode/embedded-nrf/run.resc"; \
    exdir="$(pwd)/nuc-nucleus/examples/{{EX}}"; \
    input="$exdir/input.bin"; \
    reference="$exdir/reference.bin"; \
    gen="$(mktemp -d)"; out="$(mktemp)"; log="$(mktemp)"; \
    trap 'rm -rf "$gen"; rm -f "$out" "$log"' EXIT; \
    echo "=== generating {{EX}} bin (embedded-pattern --shim nrf52840) ==="; \
    cd nucleus && cargo build --release --bin nucleus --quiet; \
    ./target/release/nucleus build \
        --algo "$exdir/prog.algo.nuc" \
        --sched "$exdir/schedules/naive.sched.nuc" \
        --backend embedded-pattern --shim nrf52840 \
        --out "$gen"; \
    cd ..; \
    elf="$gen/target/thumbv7em-none-eabihf/release/nuc-embedded-generated"; \
    echo "=== cross-compiling generated firmware (.#embedded) ==="; \
    nix develop .#embedded --command bash -c "cd '$gen' && cargo build --release --quiet"; \
    echo "=== running in Renode (.#renode): inject input.bin, capture UARTE0 ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        -e "\$bin=@$elf" -e "\$uartFile=@$out" -e "\$input=@$input" -e "include @$resc" >"$log" 2>&1; \
    captured="$(wc -c < "$out")"; expected="$(wc -c < "$reference")"; \
    echo "=== captured $captured bytes over UARTE0 (reference.bin is $expected bytes) ==="; \
    if [ "$captured" -ne "$expected" ]; then \
        echo "FAIL: captured $captured bytes, expected exactly $expected (reference.bin). The firmware did not stream the full output region."; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi; \
    if cmp -s "$out" "$reference"; then \
        echo "OK: captured UARTE0 bytes are BYTE-EXACT identical to reference.bin ($expected bytes) — nRF52840 SECOND-FAMILY {{EX}} value-correctness verified (PRD §10.3 point 3)."; \
    else \
        echo "FAIL: captured UARTE0 output differs from reference.bin (first differing byte):"; \
        cmp "$out" "$reference" || true; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Reproducibility composite (P10, TASK-0453.10): run the nRF52840 byte-exact
# co-simulation over ALL THREE single-worker examples (1/5/9) the thesis
# claims, in one command, so "byte-exact on the same three examples" is a
# single reproducible invocation rather than three manual runs (architect
# P2-1). Fail-loud via `just` prerequisites. Same tier-3-outside-default-ci
# rule as renode-embedded-nrf.
renode-embedded-nrf-all: (renode-embedded-nrf "01-elementwise-add") (renode-embedded-nrf "05-stencil") (renode-embedded-nrf "09-producer-consumer")
    @echo "OK: nRF52840 SECOND-FAMILY byte-exact across all three single-worker examples (01/05/09) — P10, TASK-0453.10."

# Tier-3 M10 GENERATED check-loop firmware -> Renode -> assert the
# on_violation=log UART line (TASK-0048.04). Generates example 1's
# embedded_check schedule (a `check loop i : latency_max=1ns,
# on_violation=log` directive) via the embedded-pattern bin-emit mode
# (`--shim stm32h7`), cross-compiles it under .#embedded, runs it headless
# in Renode under .#renode on the bundled stm32h743 platform, captures
# USART1, and ASSERTS the violation line is present — proving the no_std
# SysTick monotonic clock ADVANCED across the loop body and the
# report_violation hook fired end-to-end (AC#3).
#
# WHY a violation IS expected: latency_max=1ns is deliberately tiny so any
# nonzero per-iteration SysTick reading trips the check. The reported ns
# figure is NOT physically meaningful under Renode (not cycle-accurate) —
# what is verified is lowering correctness + that the clock advances, not
# timing fidelity. If NO violation line appears, the clock did not advance
# under Renode (a real Renode-timing limitation to investigate / fall back
# to a different source) and this recipe FAILS LOUD.
#
# NOTE: the firmware streams ex1's raw output bytes over the SAME USART1,
# so the captured stream interleaves the ASCII violation line(s) with raw
# output bytes; the assertion greps for the `check loop ` ASCII prefix
# which raw i32 output cannot spuriously produce. A separate diagnostic
# channel is the TASK-0048.08 follow-up.
#
# Self-contained (enters .#embedded then .#renode); DELIBERATELY NOT in
# `just ci` — same tier-3-outside-default-ci rule as renode-embedded.
renode-embedded-check:
    @echo "tier-3 M10 GENERATED embedded check-loop -> Renode -> assert on_violation=log UART line (TASK-0048.04)"
    @set -eu; \
    resc="$(pwd)/tests/renode/embedded/run.resc"; \
    exdir="$(pwd)/nuc-nucleus/examples/01-elementwise-add"; \
    input="$exdir/input.bin"; \
    gen="$(mktemp -d)"; out="$(mktemp)"; log="$(mktemp)"; \
    trap 'rm -rf "$gen"; rm -f "$out" "$log"' EXIT; \
    echo "=== generating embedded_check bin (embedded-pattern --shim stm32h7) ==="; \
    cd nucleus && cargo build --release --bin nucleus --quiet; \
    ./target/release/nucleus build \
        --algo "$exdir/prog.algo.nuc" \
        --sched "$exdir/schedules/embedded_check.sched.nuc" \
        --backend embedded-pattern --shim stm32h7 \
        --out "$gen"; \
    cd ..; \
    elf="$gen/target/thumbv7em-none-eabihf/release/nuc-embedded-generated"; \
    echo "=== cross-compiling generated firmware (.#embedded) ==="; \
    nix develop .#embedded --command bash -c "cd '$gen' && cargo build --release --quiet"; \
    echo "=== running in Renode (.#renode): inject input.bin, capture USART1 ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        -e "\$bin=@$elf" -e "\$uartFile=@$out" -e "\$input=@$input" -e "include @$resc" >"$log" 2>&1; \
    echo "=== captured USART1 (raw output bytes + any violation lines) ==="; \
    strings "$out" | grep -a 'check loop' || true; \
    if strings "$out" | grep -qa 'check loop `i` violated latency_max=1 ns'; then \
        echo "OK: captured the on_violation=log UART line — the no_std SysTick clock ADVANCED across the loop body + the report_violation hook fired end-to-end (AC#3)."; \
    else \
        echo "FAIL: expected on_violation=log violation line not found in captured USART1."; \
        echo "      This means the SysTick clock did not advance under Renode (a real Renode-timing"; \
        echo "      limitation: AC#3 would be honestly BLOCKED — investigate / try a different clock source)."; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Tier-3 M10 GENERATED check-loop COUNT firmware -> Renode -> assert the
# on_violation=count program-exit UART summary reports a 255-or-256
# violation count (TASK-0048.08, PART 1; band per the WHY block below).
# Generates example 1's
# embedded_check_count schedule (`check loop i : latency_max=1ns,
# on_violation=count`) via the embedded-pattern bin-emit mode (`--shim
# stm32h7`), cross-compiles it under .#embedded, runs it headless in Renode
# under .#renode, captures USART1, and ASSERTS the summary line reports the
# expected near-trip-count number of occurrences.
#
# WHY 255-or-256 (the genuinely timing-INDEPENDENT invariant): latency_max=
# 1ns is deliberately tiny so EVERY iteration of ex1's `for i : 0..256`
# loop that the clock can RESOLVE violates. The count therefore equals the
# loop trip-count (256) MODULO AT MOST ONE iteration. The exception is NOT
# the clock-seeding iteration: `_check_elapsed =
# monotonic_ns().wrapping_sub(_check_start)`, so iteration 0's seeded
# `_check_start = 0` (`NucleusShim::monotonic_ns`'s first call returns 0 to
# seed `last_cvr` without a bogus initial span) is CANCELLED by the
# subtraction — the seeding does NOT cause the zero. The real cause is
# Renode's coarse, non-cycle-accurate SysTick stepping: SysTick DOES
# advance, but in quanta, so an iteration whose two clock reads fall within
# one un-stepped counter quantum (CVR unchanged → delta 0) measures elapsed
# 0 ns and so does NOT exceed 1ns (not counted). WHICH iteration (if any)
# lands in such a quantum is instruction-layout-dependent, so the count is
# 255 OR 256. EMPIRICALLY this fixture binary deterministically yields 255
# (one iteration measures 0 ns; verified earlier by a TOTAL_ITERS=256 +
# FIRST_ELAPSED_NS=0 diagnostic, since removed). What is robustly verified
# is: the loop runs all 256 iterations, the AtomicU32 counter increments
# per RESOLVED violation, and the program-exit summary flushes the EXACT
# deterministic count — NOT a specific ns figure. Asserting the 255-or-256
# band (not a hard 256) keeps the check independent of Renode's
# instruction-layout-sensitive non-cycle-accurate timing.
#
# NOTE: the count summary shares USART1 with ex1's raw output bytes (same
# documented wart as the log sink): the summary line is emitted AFTER `run`
# streams all raw output, distinguishable by the `check loop ` ASCII prefix
# which raw i32 output cannot spuriously produce. A separate diagnostic
# channel is the TASK-0048.09 follow-up.
#
# Self-contained (enters .#embedded then .#renode); DELIBERATELY NOT in
# `just ci` — same tier-3-outside-default-ci rule as renode-embedded-check.
renode-embedded-check-count:
    @echo "tier-3 M10 GENERATED embedded check-loop COUNT -> Renode -> assert 255-or-256 violations (TASK-0048.08)"
    @set -eu; \
    resc="$(pwd)/tests/renode/embedded/run.resc"; \
    exdir="$(pwd)/nuc-nucleus/examples/01-elementwise-add"; \
    input="$exdir/input.bin"; \
    gen="$(mktemp -d)"; out="$(mktemp)"; log="$(mktemp)"; \
    trap 'rm -rf "$gen"; rm -f "$out" "$log"' EXIT; \
    echo "=== generating embedded_check_count bin (embedded-pattern --shim stm32h7) ==="; \
    cd nucleus && cargo build --release --bin nucleus --quiet; \
    ./target/release/nucleus build \
        --algo "$exdir/prog.algo.nuc" \
        --sched "$exdir/schedules/embedded_check_count.sched.nuc" \
        --backend embedded-pattern --shim stm32h7 \
        --out "$gen"; \
    cd ..; \
    elf="$gen/target/thumbv7em-none-eabihf/release/nuc-embedded-generated"; \
    echo "=== cross-compiling generated firmware (.#embedded) ==="; \
    nix develop .#embedded --command bash -c "cd '$gen' && cargo build --release --quiet"; \
    echo "=== running in Renode (.#renode): inject input.bin, capture USART1 ==="; \
    nix develop .#renode --command renode --disable-xwt --console --plain \
        -e "\$bin=@$elf" -e "\$uartFile=@$out" -e "\$input=@$input" -e "include @$resc" >"$log" 2>&1; \
    echo "=== captured USART1 count summary line(s) ==="; \
    strings "$out" | grep -a 'check loop' || true; \
    if strings "$out" | grep -qaE 'check loop `i` violated latency_max=1 ns: 25[56] occurrence\(s\)'; then \
        n="$(strings "$out" | grep -aoE 'latency_max=1 ns: [0-9]+ occurrence' | grep -oE '[0-9]+' | tail -1)"; \
        echo "OK: count summary reports $n violations (expected 255-or-256: trip-count 256 minus at most one iteration whose two clock reads fall in one un-stepped Renode SysTick quantum, measuring 0 ns elapsed) — the AtomicU32 counter + program-exit USART1 sink fired end-to-end (TASK-0048.08)."; \
    else \
        echo "FAIL: expected count summary 'check loop \`i\` violated latency_max=1 ns: 25[56] occurrence(s)' not found in captured USART1."; \
        echo "      (latency_max=1ns => every RESOLVED iteration of ex1's 256-iteration loop violates; at most one iteration measures 0 ns when its two clock reads fall in one un-stepped Renode SysTick quantum.)"; \
        echo "--- renode log (for diagnosis) ---"; cat "$log"; \
        exit 1; \
    fi

# Remove build artefacts.
clean:
    cd nucleus && cargo clean

# Render the Marpit thesis deck (docs/presentation/slides.md) to a SINGLE
# self-contained HTML SPA: marp inlines the theme + nav JS, then bundle.mjs
# inlines the 8 animated SVGs as base64 data: URIs. Needs the `.#docs` shell.
slides:
    nix develop .#docs --command bash -c "set -euo pipefail; \
        marp docs/presentation/slides.md \
            --theme-set docs/presentation/themes/nucleus.css \
            -o docs/presentation/index.html --html; \
        node docs/presentation/bundle.mjs docs/presentation/index.html"
