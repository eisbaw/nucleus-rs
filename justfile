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
determinism-check-negative:
    cd nucleus && if NUC_NONDET_TEST=1 cargo run --release --bin nucleus-e2e -- --check-determinism; then echo "FAIL: determinism check did NOT detect injected nondeterminism"; exit 1; else echo "OK: determinism check correctly bit on injected nondeterminism"; fi

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
xbackend-check-negative:
    cd nucleus && if NUC_XBACKEND_NEGATIVE=1 cargo run --release --bin nucleus-e2e; then echo "FAIL: cross-backend differential did NOT detect injected mp-tcp corruption"; exit 1; else echo "OK: cross-backend differential correctly bit on injected mp-tcp corruption"; fi

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

# Remove build artefacts.
clean:
    cd nucleus && cargo clean
