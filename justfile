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

# Lint. Warnings are errors.
clippy:
    cd nucleus && cargo clippy --workspace -- -D warnings

# Full end-to-end differential matrix (stub binary at M0; real matrix lands at M1+).
e2e:
    cd nucleus && cargo run --release --bin nucleus-e2e

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

# Remove build artefacts.
clean:
    cd nucleus && cargo clean
