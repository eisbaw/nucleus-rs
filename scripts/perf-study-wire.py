#!/usr/bin/env python3
"""Measure on-the-wire transfer volume for an mp-tcp-* Nucleus cell, by
parsing an strace socket-send log. TASK-0455.04 (c).

WHY strace AND NOT /proc/<pid>/io: the obvious least-intrusive method,
reading /proc/<pid>/io wchar deltas, was tried and FALSIFIED — Rust's
std::net::TcpStream writes go through the sendto(2)/sendmsg(2) syscalls,
whose payload bytes the kernel does NOT add to a task's wchar counter
(wchar counts only the write(2)/pwrite family). A run's measured wchar
was ~20 bytes for a transfer that actually moved kilobytes, so wchar
under-reports socket traffic by orders of magnitude. strace -e
trace=sendto,sendmsg captures the exact byte count on each call's return
value, which is the real application payload put on the socket.

METHOD (documented in docs/perf-study-methodology.md and the writeup):
  Run the cell once under
    strace -f -e trace=sendto,sendmsg -e signal=none -qq -o LOG run.sh ...
  Each completed send appears as a line ending in `= N` (N = bytes
  actually sent); under -f, a send interrupted by another thread's event
  appears as `<unfinished ...>` and its byte count lands on the matching
  `<... sendto resumed> ... = N` line, so grepping lines that END in
  `= <number>` counts each syscall's bytes EXACTLY ONCE. We bucket the
  byte sizes: small control frames (the backend's fixed-size length /
  sync tokens, <= CONTROL_MAX bytes) are reported separately from data
  payloads, and the DATA total is the figure compared against the
  whole-array baseline.

OVERHEAD / CAVEATS (carried into the doc):
  - strace ptraces every traced syscall, so it inflates wall time
    substantially (often 2-10x). That is why wire volume is measured in a
    SEPARATE run from timing — the byte counts are exact and unaffected;
    only the wall is perturbed, and we never read wall from the straced
    run.
  - sendto byte returns are application payload at the syscall boundary,
    NOT TCP segment bytes on the NIC (no headers/retransmits). For a
    loopback transfer this is the right "application bytes moved" figure,
    which is exactly what TASK-0453.22's narrowing reduces.
  - We count the SEND side (sendto/sendmsg); every cross-worker payload is
    sent exactly once, so the send-side sum is the wire volume without
    double counting. recvfrom on the other end mirrors it.

Prints one line of TSV-ish fields to stdout:
  data_bytes  control_bytes  total_bytes  n_data_sends  n_control_sends
With --histogram, additionally prints a per-payload-size send histogram
to STDERR (count x size = subtotal), so the writeup's "N x B + ..."
decomposition is reproduced from the measured log rather than asserted.
"""
import argparse, re, sys
from collections import Counter

# Sends at or below this size are the backend's fixed control / framing
# tokens (16-byte length prefixes, sync handshakes), not array data.
CONTROL_MAX = 16

# A send is COUNTED on the line that carries its byte return `= N`. Under
# strace -f, that is EITHER a one-shot completion line `sendto(...) = N`
# OR a resumption line `<... sendto resumed> ...) = N` (when another
# thread's event interrupted the call, the original line ends in
# `<unfinished ...>` and the byte count lands on the resumed line). BOTH
# forms contain the token `sendto`/`sendmsg` and END in `= <number>`, and
# the `<unfinished ...>` lines do NOT end in `= <number>` — so matching
# "mentions sendto/sendmsg AND ends in `= N`" counts each completed send
# exactly once. (An earlier regex required a literal `sendto(` and so
# SILENTLY DROPPED every resumed line, under-counting under load — the
# bug this comment guards against.)
LINE_RE = re.compile(r'\b(sendto|sendmsg)\b.*=\s+(\d+)\s*$')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--log", required=True)
    ap.add_argument("--histogram", action="store_true",
                    help="print a per-payload-size send histogram to stderr")
    args = ap.parse_args()
    data = control = 0
    nd = nc = 0
    sizes = Counter()
    with open(args.log, errors="replace") as f:
        for line in f:
            m = LINE_RE.search(line.rstrip("\n"))
            if not m:
                continue
            n = int(m.group(2))
            sizes[n] += 1
            if n <= CONTROL_MAX:
                control += n
                nc += 1
            else:
                data += n
                nd += 1
    # HARD-FAIL on zero DATA bytes (TASK-0187 lineage: a measurement step
    # that matched nothing must fail LOUD, not report a vacuous 0). A wire
    # cell ALWAYS moves array data over a socket; zero data sends means the
    # strace log was empty/truncated, the regex no longer matches the emit
    # shape, or strace was not actually tracing the worker processes. Any
    # of those is a broken measurement, and a green PASS must never be able
    # to carry an empty WIRE table (the runner's measure_wire double-checks
    # this too).
    if nd == 0 or data == 0:
        sys.exit(
            f"perf-study-wire: FAIL — parsed 0 data sends ({nd} data, "
            f"{nc} control) from {args.log}. A wire cell must move array "
            f"data; zero means an empty/truncated strace log or a stale "
            f"sendto/sendmsg regex (update LINE_RE, do not silently report 0).")
    if args.histogram:
        # Decompose the measured sends as `count x size = subtotal`,
        # smallest size first, so the writeup's per-size breakdown is
        # reproduced from the log, not asserted. Marked (control) for the
        # <= CONTROL_MAX framing tokens.
        sys.stderr.write(f"    send histogram ({args.log}):\n")
        for size in sorted(sizes):
            cnt = sizes[size]
            tag = " (control)" if size <= CONTROL_MAX else ""
            sys.stderr.write(
                f"      {cnt:>4} x {size:>8} B = {cnt*size:>10} B{tag}\n")
        sys.stderr.write(
            f"      data total = {data} B ({nd} sends), "
            f"control total = {control} B ({nc} sends)\n")
    total = data + control
    print(f"{data}\t{control}\t{total}\t{nd}\t{nc}")


if __name__ == "__main__":
    main()
