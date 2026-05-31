# Example 18 — multigather (two loop-outputs on one channel)

The dedicated fixture for the **TASK-0389.01** FIFO-ordering residual:
two distinct loop-output arrays crossing the SAME worker→host channel.

## What it exercises

A single compute worker `w0` produces TWO arrays inside one loop:

```
for i : 0 .. N {
    p[i] <-- add(a[i], a[i]);   // p = 2a   (produced FIRST  -> lower rank)
    q[i] <-- add(p[i], a[i]);   // q = 3a   (produced SECOND -> higher rank)
}
save_pq(p, q);                  // host consumes BOTH after the loop
```

Under `schedules/distributed.sched.nuc` (`host` + `w0`), both `p` and
`q` cross the SAME `(w0 -> host)` channel as **loop outputs**: their
host-side `Push` is hoisted out past the `for i` `Repeat` (the
`splice_pushes_global` "cut" branch). `p` is produced before `q`, so
producer-rank order is `[p, q]`; the worker waits `{p, q}` (rank-sorted
by `build_waits_for_op`).

Before TASK-0389.01 the naive `splice_after_repeat` ("insert the Push
immediately after the Repeat") **reversed** co-hoisted Pushes — the host
sent `{q, p}` while the worker waited `{p, q}`. On the strict-FIFO
backends (`mp-tcp-bufsync` / `mp-tcp-poll`, via `wire::read_msg_expect`)
that is a fail-loud seq/tag mismatch:

```
wire: seq tag mismatch: receiver expected 2, wire delivered 3 —
Push/Wait pairing diverged between the two generated endpoints
```

The fix makes the host's textual (= wire-send) Push order equal
producer-rank order (append each new Push after any already-spliced
Pushes at the same insertion point + feed in producer-rank order), so
worker Wait order and host Push order coincide by construction for ANY
loop-output nesting.

## Files

```
prog.algo.nuc                 # one input `a`, two loop outputs `p`, `q`
kernels.rs                    # add / load_a / save_pq
schedules/naive.sched.nuc     # single-worker smoke test
schedules/distributed.sched.nuc  # host + w0, the residual fixture
input.bin                     # 256 bytes — 64 i32 LE words (`a`)
reference.bin                 # 512 bytes — `p = 2a` then `q = 3a`
reference/                    # independent std-only reference impl
```

## Fixtures

`input.bin` (64 i32 LE words):

```python
import struct
N = 64
buf = bytearray()
for i in range(N):
    buf += struct.pack('<i', (i * 7) - 19)
open('input.bin', 'wb').write(buf)
```

Regenerate `reference.bin`:

```
cargo run --release \
  --manifest-path nuc-nucleus/examples/18-multigather/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/18-multigather/input.bin \
  --out   nuc-nucleus/examples/18-multigather/reference.bin
```

`reference.bin` is byte-identical across all 7 tier-1 backends for both
schedules.
