#!/usr/bin/env bash
# TASK-0174 (B): the REAL end-to-end netns reproduction of TASK-0038
# AC#5 — an OS that caps SO_SNDBUF below the schedule requirement
# (forced by lowering net.core.wmem_max) makes a generated
# mp-tcp-bufsync run.sh fail LOUD with the wire::apply_sock_buf clear
# error naming the OS cap.
#
# Strategy (no privileged container / no host root required IF the
# kernel permits it):
#   1. `unshare -Urn` — new USER + NET namespace; our uid maps to
#      root-in-userns, granting CAP_NET_ADMIN over the FRESH netns.
#   2. `sysctl -w net.core.wmem_max=4096 rmem_max=4096` IN that netns.
#   3. Generate a real 02-split-add/split mp-tcp-bufsync project (its
#      NUC_SO_BUF requirement is the 64 KiB floor, >> 4096).
#   4. Run its run.sh inside the netns; assert it exits non-zero AND
#      the wire::apply_sock_buf clear error (naming net.core.wmem_max
#      / rmem_max) appears.
#
# HONEST-SKIP CONTRACT: if step 2 is not permitted in this
# environment (it ISN'T in the Nucleus Nix dev sandbox — see the
# probe below), this script exits 0 with a SKIPPED line and a precise
# reason. It does NOT fake AC#5. A SKIP here is informational, exactly
# like the distributed-cell [[skip]] discipline in e2e-matrix.toml
# and the TASK-0166 no-runner standing limitation. Only a genuine
# end-to-end reproduction may close TASK-0038 AC#5.
#
# Exit codes:
#   0  = either the reproduction genuinely PASSED (clear error seen
#        under the lowered cap), OR it was honestly SKIPPED because
#        the sandbox forbids lowering net.core.wmem_max.
#   1  = the harness RAN the reproduction and it did NOT fail loud as
#        required (a real regression — the fail-loud path is broken).
set -u

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$here/.." && pwd)"
ws="$repo_root/nucleus"
example="$here/examples/02-split-add"
algo="$example/prog.algo.nuc"
sched="$example/schedules/split.sched.nuc"
kernels="$example/kernels.rs"

say() { printf '%s\n' "$*"; }

skip() {
    say "SKIPPED: sockbuf-cap-check — $1"
    exit 0
}

# ---- PROBE: can we lower net.core.wmem_max in a user+net ns? --------
# net.core.wmem_max is owned by the INITIAL network namespace's user
# namespace (init_user_ns), NOT namespaced per-netns. Even as
# root-in-userns with CAP_NET_ADMIN over a fresh netns the kernel
# checks the capability against init_user_ns for this global core
# sysctl, so the write is EPERM without real host root or a
# privileged container (`docker run --sysctl net.core.wmem_max=...`).
if ! command -v unshare >/dev/null 2>&1; then
    skip "no \`unshare\` binary on PATH; cannot create a user+net namespace"
fi

# SUBTLETY (verified empirically): `sysctl -w net.core.wmem_max=4096`
# prints "Operation not permitted" but STILL EXITS 0 in this build —
# its exit code does NOT propagate the EPERM. So we must NOT trust
# `sysctl ... && ...`; instead we WRITE then READ BACK the value and
# only treat it as permitted if the readback actually changed. This
# is the fail-fast, no-false-positive probe.
probe="$(unshare -Urn sh -c '
    sysctl -w net.core.wmem_max=4096 >/dev/null 2>&1
    v="$(cat /proc/sys/net/core/wmem_max 2>/dev/null)"
    if [ "$v" = "4096" ]; then echo NS_SYSCTL_OK; else echo "NS_SYSCTL_EPERM(readback=$v)"; fi
' 2>/dev/null || echo NS_UNSHARE_FAIL)"

if [ "$probe" != "NS_SYSCTL_OK" ]; then
    skip "cannot lower net.core.wmem_max in a \`unshare -Urn\` user+net \
namespace (probe=$probe). net.core.wmem_max is init_user_ns-owned, \
NOT per-netns namespaced, so CAP_NET_ADMIN in a fresh netns is \
insufficient — it needs real host root or a privileged container \
(\`docker run --sysctl net.core.wmem_max=4096 ...\`) or a CI runner \
with userns sysctl write enabled. The fail-loud DECISION is proven \
deterministically by the mp-tcp-common pure-logic unit tests \
(check_effective_sock_buf, TASK-0174 layer A); this end-to-end \
netns reproduction is ready and will run wherever the sysctl write \
is permitted."
fi

# ---- The sysctl write IS permitted here: run the genuine repro -----
say "sockbuf-cap-check: netns sysctl write permitted — running the real reproduction"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# Generate the project OUTSIDE the netns (cargo/network for the
# registry is irrelevant — deps are vendored/locked, but emitting +
# building before entering the netns keeps the netns step minimal).
( cd "$ws" && cargo run --quiet --bin nucleus -- build \
    --algo "$algo" --sched "$sched" --kernels "$kernels" \
    --backend mp-tcp-bufsync --out "$scratch" ) \
    || { say "FAIL: nucleus build of 02-split-add/split failed"; exit 1; }

( cd "$scratch" && cargo build --release --quiet ) \
    || { say "FAIL: cargo build of the generated project failed"; exit 1; }

# The generated run.sh resolves input/output relative to CWD; the
# example's input.bin lives with the example, so stage it into the
# scratch project (the e2e harness does the same).
cp "$example/input.bin" "$scratch/input.bin" \
    || { say "FAIL: could not stage input.bin into the scratch project"; exit 1; }

# Now run run.sh INSIDE the lowered-cap netns. We RE-VERIFY the cap
# actually took (sysctl masks EPERM in its exit code) and abort the
# run as inconclusive if it did not — never report a pass/fail off a
# cap that was not really lowered.
out="$(unshare -Urn sh -c "
    sysctl -w net.core.wmem_max=4096 >/dev/null 2>&1
    sysctl -w net.core.rmem_max=4096 >/dev/null 2>&1
    rb=\$(cat /proc/sys/net/core/wmem_max 2>/dev/null)
    if [ \"\$rb\" != 4096 ]; then echo \"__CAP_NOT_LOWERED__ readback=\$rb\"; exit 0; fi
    ip link set lo up 2>/dev/null
    cd '$scratch' && bash run.sh input.bin output.bin 2>&1
")"
rc=$?

if printf '%s' "$out" | grep -q "__CAP_NOT_LOWERED__"; then
    skip "the in-netns net.core.wmem_max write did not actually take \
($(printf '%s' "$out" | grep -o '__CAP_NOT_LOWERED__ readback=[0-9]*')); \
\`sysctl\` masks the EPERM in its exit code. net.core.wmem_max is \
init_user_ns-owned, not per-netns — needs host root / privileged \
container / userns-sysctl-enabled CI. Pure-logic proof (layer A) \
covers the decision; this end-to-end arm is ready for such a host."
fi

say "---- run.sh output (under net.core.wmem_max=4096) ----"
say "$out"
say "---- run.sh exit code: $rc ----"

if [ "$rc" -eq 0 ]; then
    say "FAIL: run.sh exited 0 under a lowered net.core.wmem_max — the \
SO_*BUF fail-loud path did NOT bite (regression in wire::apply_sock_buf)."
    exit 1
fi

if printf '%s' "$out" | grep -q "socket buffer too small" \
   && printf '%s' "$out" | grep -q "net.core.wmem_max"; then
    say "OK: run.sh failed LOUD with the wire::apply_sock_buf clear error \
naming the OS cap (net.core.wmem_max / rmem_max) — TASK-0038 AC#5 \
genuinely reproduced end-to-end."
    exit 0
fi

say "FAIL: run.sh exited non-zero ($rc) but WITHOUT the expected \
wire::apply_sock_buf clear error naming net.core.wmem_max. The \
failure must be the buffer-cap fail-loud, not an unrelated error."
exit 1
