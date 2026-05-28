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
    just test-release
    just check-textual-replace-on-codegen
    just check-include-str-coverage
    just check-narrative-doc-lie
    just check-mega-files
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
check-include-str-coverage:
    @echo "checking include_str! compile coverage..."
    @set -e; fail=0; \
    while IFS= read -r line; do \
        file=$(echo "$line" | cut -d: -f1); \
        target=$(echo "$line" | grep -oE 'include_str!\([^)]*\)' | head -1 | sed -E 's/include_str!\("([^"]+)"\)/\1/'); \
        base=$(basename "$target" .rs); \
        crate_dir=$(dirname "$file"); \
        if ! rg -q "mod ${base}\b" "$crate_dir" && \
           ! rg -q "include!\(\"${target}\"\)" "$crate_dir"; then \
            echo "FAIL: $file include_str!(\"$target\") has no matching 'mod ${base};' or 'include!(\"$target\")' in $crate_dir"; \
            fail=1; \
        fi; \
    done < <(rg -nH --type rust 'include_str!' nucleus/nucleus-compiler/src/ nucleus/backend-common/src/ nucleus/backends/*/src/ nucleus/mp-tcp-common/src/ 2>/dev/null || true); \
    if [ $fail -ne 0 ]; then \
        echo ""; \
        echo "(memory: feedback-include-str-compile-coverage — bare include_str! does not compile the file content; add 'mod <name>;' or 'include!(\"<path>\");' in the same crate so 'cargo test' compiles it)"; \
        exit 1; \
    fi; \
    echo "OK: every include_str! has compile coverage."

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
# `nucleus/nucleus-compiler/src`, `nucleus/backends/*/src`, and
# `nucleus/e2e/src` (the last one added cycle 190 / TASK-0342 — qa
# cycle-185b P3.1 surfaced the pre-cycle-190 exclusion as a
# documentation/expectation lag after TASK-0340 slice-10 carved
# e2e/src/main.rs from 7316→4716 LoC and added tests.rs at 2635 LoC,
# both still >1000 and neither in the fence). The following sub-trees
# remain DELIBERATELY excluded:
#   - `nucleus/driver`, `nucleus/mp-tcp-common`, `nucleus/test-common`,
#     `nucleus/nucleus` — currently NO file >1000 LoC by coincidence
#     of size, not by rule. Widen the scope when any grows past 1000.
#
# Allow-list canonical reproducer: `find
# nucleus/{backend-common,nucleus-compiler,backends,e2e}/src -name '*.rs'
# -exec wc -l {} \; | sort -rn | awk '$1 > 1000 {print $2}'`.
#
# Each entry is a TASK-0340 AC#2 split target — the allow-list shrinks
# as splits land. The allow-list is a printf-fed bash array (not a
# heredoc — a column-0 heredoc body crashes just's parser; see
# justfile-history). Adding/removing an entry is a one-line edit;
# per-file LoC numbers are deliberately NOT enumerated (architect
# cycle-176 P3.1 — they create drift debt with no automated guard).
#
# POSIX-shell portability (architect cycle-177 P1.1): the recipe uses
# `comm -23` against TEMP FILES, not bash process substitution
# `<(...)`. `just` defaults to `/bin/sh -cu`; on dash/ash/busybox-sh
# `<(...)` would syntax-error before either direction runs, leaving
# the regression-fence silently absent. The temp-file form via
# `mktemp` + `trap "rm -f ... " EXIT` is POSIX-portable.
check-mega-files:
    @echo "checking nucleus/**/src/*.rs for files exceeding 1000 LoC..."
    @set -eu; \
    set -o pipefail; \
    oversized_f=$(mktemp); \
    allow_f=$(mktemp); \
    trap "rm -f $oversized_f $allow_f" EXIT; \
    find nucleus/backend-common/src nucleus/nucleus-compiler/src nucleus/backends/*/src nucleus/e2e/src -name '*.rs' -exec wc -l {} \; \
        | awk '$1 > 1000 {print $2}' \
        | sort > $oversized_f; \
    printf '%s\n' \
        'nucleus/nucleus-compiler/src/passes/transfer_inject.rs' \
        'nucleus/nucleus-compiler/src/passes/reuse_inference.rs' \
        'nucleus/nucleus-compiler/src/sched/lower.rs' \
        'nucleus/nucleus-compiler/src/passes/halo_inference.rs' \
        'nucleus/nucleus-compiler/src/algo/lower.rs' \
        'nucleus/nucleus-compiler/src/passes/host_data_relay_inject.rs' \
        'nucleus/nucleus-compiler/src/sched/ir.rs' \
        'nucleus/backends/pthreads-async/src/multi_worker.rs' \
        'nucleus/e2e/src/main.rs' \
        'nucleus/e2e/src/tests.rs' \
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

# Tier-3 M9 compile-only acceptance (TASK-0047 AC#4). Generates the
# `embedded-pattern` no_std lib for the M9 acceptance examples (1 + 5,
# their naive schedules) and runs `cargo check --target
# thumbv7em-none-eabihf` on each generated project against the stub
# shim. SUCCEEDS iff every generated lib cross-compiles.
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
# The example set (1 + 5) is the M9 acceptance set fixed by PRD §11 M9 /
# TASK-0047 AC#4 (the two examples most representative of embedded
# workloads — elementwise + stencil). It is NOT per-example recipe bloat
# (PRD §12.3 anti-bloat): this is one milestone gate. M10 (Renode runtime,
# TASK-0048) extends it to a run-and-diff; until then compile-only is the
# bar (PRD §10.3 point 5).
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
    echo "OK: embedded-pattern no_std lib cross-compiles for examples 1 + 5 (thumbv7em-none-eabihf)."

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

# Remove build artefacts.
clean:
    cd nucleus && cargo clean
