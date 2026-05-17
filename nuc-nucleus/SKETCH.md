# Nuc / Nucleus — revival sketch

Starting point for a modern redo of the 2013 thesis (Chapter 4). Not a plan
for re-implementing the original — a sketch of the smallest thing worth
building now, with the lessons from the original baked in.

## What was right in 2013 — keep

- **Communication is a consequence of decomposition.** Express the
  decomposition; let the tool infer transfers and sync. This framing is
  still correct.
- **Unified source, annotations carry the mapping.** One file per
  application, annotations push code to threads/devices.
- **ACFG as IR.** Control-flow tree + `sync` + `xfer` nodes, with the
  semantic split between control sync and data coherency. Clean.
- **Presentation layer as the only target-specific code.** Everything
  above it stays hardware-agnostic.

## What was wrong — fix

1. **Title oversold scope.** The running example (panorama seam-finding)
   never ran end-to-end because unaligned-vector reuse was unimplemented.
   New scope: pick *one* end-to-end example and make it actually work
   before claiming anything.
2. **Single backend.** "Generic presentation layer" with N=1 backend is
   unfalsified. New rule: ship two backends from day one (e.g. OpenMP
   threads + WebGPU compute, or pthreads + CUDA). If the second one is
   painful, the abstraction is wrong.
3. **Aliasing punted.** Split-binding policy is unsound under aliasing
   (§4.3.14.3). Either pull in a polyhedral library (isl) for real array
   dataflow, or restrict the language to a SAFE subset where aliasing is
   structurally impossible (e.g. single-assignment arrays + explicit views).
4. **Where-clauses + "BB order doesn't matter" is a semantic crack.**
   If where-bodies can side-effect, total order matters. New rule:
   where-clauses must be pure, or annotated `!effectful` to pin their
   order. No third option.
5. **Hive-specific case study aged out.** Silicon Hive ISPs are gone.
   Pick a still-relevant heterogeneous target: GPU + CPU is the
   uncontroversial default; FPGA via HLS or NPU offload if more ambitious.

## Smallest interesting thing to build first

A pre-compiler that takes a single annotated source file and produces:

- A CPU host program (C or Rust).
- A GPU kernel (WGSL or CUDA) for any block-annotated loop nest.
- Inferred buffer transfers and a barrier between host and device.

That's the 2013 thesis's core claim, on hardware that exists in 2026.
If this works for one non-trivial kernel (e.g. a 3x3 separable convolution
with reuse), it's worth writing up. If it doesn't, the framing is wrong
and that's also worth knowing early.

## Minimal Nuc-like surface (strawman)

```
// stencil.nuc — strawman syntax
data img_in  : f32[H][W]
data img_out : f32[H][W]

img_in <-- host::load_image();

for y : 1 .. H-2  block=32 {
for x : 1 .. W-2  vectorize=8 reuse {
    img_out[@y][@x] <-- gpu::blur3x3(
        img_in[@y-1][@x-1], img_in[@y-1][@x], img_in[@y-1][@x+1],
        img_in[@y  ][@x-1], img_in[@y  ][@x], img_in[@y  ][@x+1],
        img_in[@y+1][@x-1], img_in[@y+1][@x], img_in[@y+1][@x+1]
    ) where pure {{
        ${out} = (${1}+${2}+${3}+${4}+${5}+${6}+${7}+${8}+${9}) * (1.0/9.0);
    }};
}}

host::save_image(img_out);
```

Differences from 2013 Nuc:
- `where pure` / `where !effectful` is mandatory — kills the BB-order trap.
- `reuse` is a first-class loop option (the thing that didn't ship).
- Types are explicit (`f32`) so the GPU backend doesn't have to guess.

## Open questions worth answering before writing code

- Is `block_integral` worth keeping, or do you always pay the body-clone cost
  and let the optimiser handle it?
- How to express "this access is aligned" without polluting the source?
  (2013 thesis suggested `: aligned` postfix — still seems right.)
- Should ACFG be a tree (2013) or a proper graph? Tree was a nice
  simplification but ruled out things like forever-loops cleanly.

## What this folder is not

Not a port of the 2013 C++ source. Not a backlog. A blank-page redo
informed by what the original got right and wrong.
