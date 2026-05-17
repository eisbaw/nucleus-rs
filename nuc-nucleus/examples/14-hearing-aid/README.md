# Example 14 — Hearing-Aid Pipeline

A finite-frame test version of the dataflow inside a hearing aid:
analog front-end (microphone, speaker), DSP (noise reduction, mixing),
RF (Bluetooth receive/transmit).

## What this stresses

| Axis | What                                                                  |
| ---- | --------------------------------------------------------------------- |
| Workers     | Three heterogeneous *classes*, not just three names. First example to exercise the typed worker form (§6.3.1). |
| Dataflow    | Fork-and-merge: mic and BT converge in DSP; DSP forks to speaker and BT-out. No earlier example has a fork or a merge. |
| Bidirectional | RF both receives and transmits; FE both captures and emits.         |
| IO          | Peripheral IO wrapped in effectful kernels.                           |
| Tier-3 multi-MCU | The schedule that lives or dies on Renode co-simulation (M11).   |

## Required schedules

- `naive.sched.nuc` — one host worker, sequential frames. Tier-1 smoke test, defines reference output.
- `embedded_multimcu.sched.nuc` — three MCUs of three classes (FE, DSP, RF), `pipeline=3`, async-buffered transfers in both directions. Tier-3 / Renode target.

A `batch_parallel` schedule does not make sense here — the algorithm is
inherently sequential per frame (the speaker output at frame N depends
on mic+BT at frame N). The point of this example is *heterogeneous
spatial decomposition*, not data parallelism.

## Latency: checkable, not prescriptive

`embedded_multimcu.sched.nuc` ends with:

```
check frame : latency_max = 10ms;
```

This is a **runtime assertion**, not a compiler constraint. v2 has no
cost model and cannot schedule code to meet a latency budget. What it
*can* do: emit measurement code at iteration boundaries and verify
the actual wall-clock duration is within the budget. If the
assertion fires during tier-1 testing or Renode simulation, the
schedule needs revision by hand — increase pipeline depth, enlarge
a buffer, move a kernel to a different worker class, etc.

The seed of a future prescriptive `solve_latency_max` directive is
here. v2 ships the observation; the optimiser comes later (v3, if
ever).

## What this example does NOT exercise

State this so the example doesn't get over-claimed:

- **Continuous operation.** A real hearing aid runs forever. v2
  algorithms terminate. This example uses `N_FRAMES = 1000` for
  testability. A future `forever` construct or large-N substitution
  is a deployment concern.
- **Peripheral interrupts as first-class.** Effectful kernels work,
  but v2 doesn't model "this kernel blocks on a DMA-complete IRQ"
  natively. The backend / shim handles this. The model sees
  `fe_capture` as just an effectful function whose ordering is
  preserved.
- **Compiler-enforced deadlines.** The `check` directive is checkable
  only. See "Latency: checkable, not prescriptive" above. A v3
  prescriptive variant could be added without changing the source-
  level syntax shape.

If any of those becomes a load-bearing requirement, that's the signal
v3 needs a streaming / IRQ / cost-model language extension.

## Reference

`reference/` (TODO) contains a hand-written single-threaded Rust
implementation of the same pipeline. CI feeds canned `mic_in.bin` and
`bt_in.bin` and diffs `spk_out.bin` / `bt_out.bin` against the
reference.

The DSP body (`denoise`, `mix2`) must be bit-deterministic. Either
implement in fixed-point integer arithmetic or use a fixed FFT
implementation that does not reorder reductions. See PRD §10.1.

## Why no training / adaptation

A real hearing aid adapts gain to environment, adapts beam-forming to
speaker location, etc. All adaptive behaviour is excluded here for
the same reason ML training is excluded from example 13: it needs
state that crosses frames in ways the static-schedule + affine-access
model doesn't natively support. The example tests the fixed pipeline.
