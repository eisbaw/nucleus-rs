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

# Default: list available recipes.
default:
    @just --list

# Build all crates in the workspace.
build:
    cd nucleus && cargo build --workspace

# Run unit tests.
test:
    cd nucleus && cargo test --workspace

# Fast type-check without codegen.
check:
    cd nucleus && cargo check --workspace

# Apply rustfmt.
fmt:
    cd nucleus && cargo fmt --all

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
# Not in `just ci`: 20x parallel `nucleus-e2e` invocations are too
# heavy for the standard CI walltime budget. Run manually after any
# change touching the port handshake (mp-tcp-bufsync emit, run.sh
# generation, wire::apply_sock_buf), or on a nightly schedule.
#
# Each e2e invocation uses a unique per-RUN run-id (pid+nanos,
# TASK-0182) so scratch dirs do not collide; the per-cell run.sh
# uses its own pid-suffixed rendezvous dir so the port-handshake
# files do not collide either. Pre-builds the workspace once so the
# 20 parallel invocations do not serialise on the cargo build lock.
port-stress-check N="20":
    cd nucleus && cargo build --release --bin nucleus-e2e --quiet
    cd nucleus && fail=0; for i in $(seq 1 {{N}}); do cargo run --release --quiet --bin nucleus-e2e -- --backend mp-tcp-bufsync >/tmp/nuc-port-stress-$$-$i.log 2>&1 & done; for j in $(jobs -p); do wait "$j" || fail=$((fail+1)); done; if [ "$fail" -gt 0 ]; then echo "FAIL: $fail of {{N}} parallel mp-tcp-bufsync e2e runs failed (TASK-0176 AC#2). Last failing log tail:"; ls /tmp/nuc-port-stress-$$-*.log | head -1 | xargs tail -40; exit 1; fi; rm -f /tmp/nuc-port-stress-$$-*.log; echo "OK: {{N}}/{{N}} parallel mp-tcp-bufsync e2e runs passed (TASK-0176 AC#2)"

# Remove build artefacts.
clean:
    cd nucleus && cargo clean
