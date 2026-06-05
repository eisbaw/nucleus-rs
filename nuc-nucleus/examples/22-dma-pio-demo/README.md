# Example 22 — DMA-async + PIO-sync transfers in one app

A Q8 fixed-point audio gain-apply — `out[i] = (samples[i] * gains[i])
>> 8` over an array of `N = 256` little-endian `i32` words — whose
**load-bearing schedule emits TWO structurally distinct cross-worker
transport paths from one Nucleus source**:

- the bulk audio frames (`samples` in, `out` back) cross as
  **DMA-async** edges (`mode=dma`);
- the per-sample gain coefficients (`gains`) cross as a **PIO-sync**
  edge (`mode=pio`).

This is the M11 demo that counters the "it's just toy memcpy" framing:
the embedded-pattern backend (TASK-0438.02) renders a genuinely
different code path for each mode, yet the whole app stays Renode
byte-exact because both modes ride the same UART fabric and are
value-equivalent.

## What this example stresses

| Axis        | What                                                                  |
| ----------- | --------------------------------------------------------------------- |
| Algorithmic | One pure scalar kernel (`apply_gain`, Q8 gain) + one for-loop + I/O — the same shape as 02-split-add. |
| Scheduling  | The first schedule with **per-edge transport-mode hints** (`mode=dma` / `mode=pio`). Two DMA-async edges (`samples`, `out`) + one PIO-sync edge (`gains`). |
| Backends    | embedded-pattern (tier 3): the `mode=dma` edges emit `shim.dma_link_arm` / `dma_link_recv_arm` + a completion spin loop; the `mode=pio` edge emits the unchanged `shim.link_push` / `shim.link_recv` byte loop. Proven byte-exact under the Renode multi-MCU co-sim. |

## The two transport modes

```
transfer samples : sync, mode=dma;   // bulk frame in   -> DMA-async
transfer gains   : sync, mode=pio;   // gain coeffs     -> PIO-sync
transfer out     : sync, mode=dma;   // bulk frame back -> DMA-async
```

**Why bulk = DMA, control = PIO.** This mirrors a real embedded audio
pipeline: you DMA a whole frame buffer (set up one descriptor, let the
stream run) but PIO a handful of control/coefficient words (a few
CPU-driven loads, no descriptor-setup overhead). The split is a
codegen/transport concern — the algorithm in `prog.algo.nuc` has no
knowledge of DMA, PIO, `host`, or `w0` (PRD §2 algorithm/schedule
split; PRD §6.3.4).

`mode=` is **orthogonal** to `sync`/`async`: it selects the transport
*code path*, not the completion semantics. All three edges stay `sync`
so the synchronous embedded shim can honour them without a capability
lie.

## HONEST caveat — Renode DMA is a timing MODEL, not a silicon engine

Under the Renode co-sim the `mode=dma` edges do **not** run a real
STM32H7 DMA stream. The default `dma_link_arm` delegates to the same
`link_push` UART transport, and `dma_link_poll` returns `true`
synchronously, so the emitted spin loop terminates on its first
iteration and the DMA bytes ride the **same UART fabric** as the PIO
bytes.

Consequently the proof this example carries is **value-correctness**
(byte-exact output), **NOT timing-correctness** (parent TASK-0438
AC#4). A real async DMA engine — where `dma_link_poll` reads the
silicon transfer-complete bit and the CPU does useful work while the
stream runs — is the explicit follow-up **TASK-0048.12**. See
TASK-0438.03.

## What this example does NOT stress

- Real cycle-accurate DMA timing (see the caveat above; TASK-0048.12).
- Distributed placement of `apply_gain` across many compute workers. A
  single `w0` runs the whole loop, exactly like 02-split-add.
- Buffered / `notify=event` async transfers.
- Tier 1 (pthreads/host) and tier 2 (MPI). This example is
  **tier-3-only** (multi-MCU Renode), like 14-hearing-aid's embedded
  schedule. It is intentionally **not** in `e2e-matrix.toml`'s
  `runnable_examples`, so the tier-1 `just e2e` baseline is unchanged.

## Files

```
22-dma-pio-demo/
  prog.algo.nuc                 # algorithm (audio gain-apply, 02-shaped)
  kernels.rs                    # Rust bodies (Q8 apply_gain, file-based I/O)
  schedules/
    dma_pio.sched.nuc           # the demo: host + w0, mode=dma + mode=pio
  reference/                    # hand-written, std-only reference
    Cargo.toml
    src/main.rs
  input.bin                     # 2048 bytes — 256 i32 LE samples ++ 256 gains
  reference.bin                 # 1024 bytes — expected out output
```

## I/O format

Binary little-endian `i32` words. `N = 256`; this matches `const N :
usize = 256;` in `prog.algo.nuc`.

- **`input.bin`** (2048 bytes):
  - bytes `[0      ..   4*N) ` — array `samples`, `N` LE `i32` words.
  - bytes `[4*N    .. 4*2*N) ` — array `gains`,   `N` LE `i32` words.
- **`reference.bin`** (1024 bytes):
  - bytes `[0      ..   4*N) ` — array `out = (samples*gains)>>8`.

The loads consume the input region **in fire order** (`load_samples`
then `load_gains`), so the on-disk layout is `samples ++ gains`. Under
Renode the host MCU fills these from the injected input region in the
same order.

### Fixture pattern (`--gen-input`)

```
samples[i] = (i as i32) - 128       // signed sweep [-128, 127]
gains[i]   = 200 + (i % 100) as i32  // [200, 299], straddles unity (256)
```

The gain band straddles Q8 unity (`256`), so some samples are
attenuated (`gain < 256`) and some amplified (`gain >= 256`) — a
non-trivial pattern where a dropped or swapped element shows up in the
output. The worst-case magnitude `|127 * 299| >> 8 = 148` stays well
inside `i32` (no overflow for the committed fixture; `wrapping_mul`
documents the contract regardless).

The fixtures are committed binaries (per
[`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)
§1), each well under the 10 KB cap that keeps them inspectable by hand
(`hexdump -C input.bin | less`).

## Running the byte-exact gate

```
just renode-multimcu 22-dma-pio-demo dma_pio
```

This generates the per-worker no_std firmware (embedded-pattern,
`--shim stm32h7`), cross-compiles each worker under `.#embedded`,
co-simulates two STM32H7 MCUs (host + w0) under `.#renode`, captures the
host saver's USART1 output, and `cmp`s it BYTE-EXACT against
`reference.bin` (1024 bytes). It is also wired as a third co-sim in the
standing `just renode-multimcu-gate` (alongside 02-split-add and
14-hearing-aid).

## Regenerating the fixtures

See the header of `reference/Cargo.toml` for the exact `cargo run`
commands (`--gen-input` then `--in/--out`).
