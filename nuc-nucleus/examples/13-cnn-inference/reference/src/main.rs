//! Reference implementation of example 13-cnn-inference.
//!
//! Small CNN forward pass over a batch of i32 inputs:
//!   for each sample n in 0..B:
//!     feat1[n]  = conv3x3_relu_pool2(input[n], w1)   (C0=1  -> C1=8 channels;  28x28 -> 14x14)
//!     feat2[n]  = conv3x3_relu_pool2(feat1[n], w2)   (C1=8  -> C2=16 channels; 14x14 -> 7x7)
//!     output[n] = dense(feat2[n], w_cls)             (16*7*7=784 -> 10)
//!
//! Independent of Nucleus per docs/reference-impl-policy.md §2:
//! depends on `std` only; does not share any code with kernels.rs or
//! the compiler. The shape constants, weight formulae and arithmetic
//! contract are RE-IMPLEMENTED here — same algorithm, different
//! source. A bug in one file is not automatically duplicated in the
//! other (policy §2 rationale: "all backends wrong the same way").
//!
//! Input format (`input.bin`):
//!   B * C0 * H * W = 16 * 1 * 28 * 28 = 12544 i32 LE words.
//!   Row-major flattening: byte offset of input[n][c][y][x] is
//!     4 * (n * C0*H*W + c * H*W + y * W + x).
//!
//! Output format (`reference.bin`):
//!   B * N_CLASSES = 16 * 10 = 160 i32 LE words.
//!   Per-sample classifier logits, no softmax.
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only (`i32` with `wrapping_mul` / `wrapping_add`).
//!   - All reductions are strict left-to-right `for` loops.
//!   - No threads, no HashMap iteration, no Instant-derived values.
//!   - No FMA, no f32, no SIMD reorder.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

// Mirror prog.algo.nuc constants. Single-source-of-truth violation
// shared with kernels.rs is intentional (policy §2: independent
// implementation may re-declare the same shape).
const B: usize = 16;
const H: usize = 28;
const W: usize = 28;
const C0: usize = 1;
const C1: usize = 8;
const C2: usize = 16;
const N_CLASSES: usize = 10;
const H1: usize = H / 2;
const W1: usize = W / 2;
const H2: usize = H / 4;
const W2: usize = W / 4;

const SAMPLE_IN: usize = C0 * H * W;
// SAMPLE_F1 (C1*H1*W1) is intentionally not declared here because
// the generic `forward_conv_pool` derives it from `CIN/COUT/hin/win`.
// It IS declared in kernels.rs where it bounds an explicit assert.
const SAMPLE_F2: usize = C2 * H2 * W2;
const BYTES_PER_WORD: usize = 4;
const INPUT_ELEMS: usize = B * SAMPLE_IN;
const OUTPUT_ELEMS: usize = B * N_CLASSES;
const INPUT_BYTES: usize = INPUT_ELEMS * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = OUTPUT_ELEMS * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--in" => {
                i += 1;
                if i >= args.len() {
                    return die("--in requires a path argument");
                }
                in_path = Some(args[i].clone());
            }
            "--out" => {
                i += 1;
                if i >= args.len() {
                    return die("--out requires a path argument");
                }
                out_path = Some(args[i].clone());
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage: cnn-inference-reference --in INPUT.bin --out OUTPUT.bin"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                return die(&format!("unknown argument: {}", other));
            }
        }
        i += 1;
    }
    let in_path = match in_path {
        Some(p) => p,
        None => return die("missing required --in PATH"),
    };
    let out_path = match out_path {
        Some(p) => p,
        None => return die("missing required --out PATH"),
    };

    // Read and validate input. Strict length check — silent
    // acceptance of wrong-length files would mask fixture drift.
    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != INPUT_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (B*C0*H*W*4)",
            in_path,
            bytes.len(),
            INPUT_BYTES
        ));
    }
    let mut input = vec![0i32; INPUT_ELEMS];
    for k in 0..INPUT_ELEMS {
        let off = k * BYTES_PER_WORD;
        input[k] = i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]);
    }

    // Per-sample forward pass.
    let mut output = vec![0i32; OUTPUT_ELEMS];
    for n in 0..B {
        let sample_in_lo = n * SAMPLE_IN;
        let sample_in_hi = sample_in_lo + SAMPLE_IN;
        let sample_input: &[i32] = &input[sample_in_lo..sample_in_hi];

        let feat1 = forward_conv_pool::<C0, C1>(sample_input, H, W, H1, W1, weight_layer_1);
        let feat2 = forward_conv_pool::<C1, C2>(&feat1, H1, W1, H2, W2, weight_layer_2);

        // Dense classifier: per-class wrapping dot product, strict
        // left-to-right.
        for class in 0..N_CLASSES {
            let mut acc: i32 = 0;
            for k in 0..SAMPLE_F2 {
                acc = acc.wrapping_add(feat2[k].wrapping_mul(weight_classifier(class, k)));
            }
            output[n * N_CLASSES + class] = acc;
        }
    }

    // Encode and write output.
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for v in &output {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(out_bytes.len(), OUTPUT_BYTES);
    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&out_bytes) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }
    ExitCode::SUCCESS
}

/// Generic conv3x3 SAME-padded + ReLU + 2x2 maxpool, sample-at-a-time.
///
/// `CIN` and `COUT` are channel counts (compile-time generics so the
/// per-layer call sites are crisp without runtime channel parameters).
/// `hin`/`win` are the spatial input dims; `hout`/`wout` are the
/// post-pool dims (must be hin/2 and win/2). `weight_fn` returns the
/// (oc, ic, ky, kx) -> i32 weight; identical formula must hold in
/// kernels.rs's `w1` / `w2` (verified by reference.bin diff).
///
/// Order of operations: ALL conv outputs computed and ReLU'd first,
/// then maxpool. This is the same order as kernels.rs's two separate
/// loops (conv -> relu -> pool). Strict left-to-right reductions.
fn forward_conv_pool<const CIN: usize, const COUT: usize>(
    input: &[i32],
    hin: usize,
    win: usize,
    hout: usize,
    wout: usize,
    weight_fn: fn(usize, usize, usize, usize) -> i32,
) -> Vec<i32> {
    assert_eq!(input.len(), CIN * hin * win, "forward_conv_pool: input shape");
    assert_eq!(hout, hin / 2, "forward_conv_pool: pool stride");
    assert_eq!(wout, win / 2, "forward_conv_pool: pool stride");

    // Conv + ReLU intermediate.
    let mut relu = vec![0i32; COUT * hin * win];
    for oc in 0..COUT {
        for y in 0..hin {
            for x in 0..win {
                let mut acc: i32 = 0;
                for ky in 0..3usize {
                    for kx in 0..3usize {
                        let iy = y as i32 + ky as i32 - 1;
                        let ix = x as i32 + kx as i32 - 1;
                        for ic in 0..CIN {
                            let v = if iy < 0
                                || iy >= hin as i32
                                || ix < 0
                                || ix >= win as i32
                            {
                                0i32
                            } else {
                                input[ic * hin * win + iy as usize * win + ix as usize]
                            };
                            let w = weight_fn(oc, ic, ky, kx);
                            acc = acc.wrapping_add(v.wrapping_mul(w));
                        }
                    }
                }
                let r = if acc > 0 { acc } else { 0 };
                relu[oc * hin * win + y * win + x] = r;
            }
        }
    }

    // Maxpool 2x2 stride 2. Strict left-to-right max chain (i32::max
    // is order-independent, but spelling out the chain documents
    // intent).
    let mut out = vec![0i32; COUT * hout * wout];
    for oc in 0..COUT {
        for oy in 0..hout {
            for ox in 0..wout {
                let a = relu[oc * hin * win + (2 * oy) * win + (2 * ox)];
                let b = relu[oc * hin * win + (2 * oy) * win + (2 * ox + 1)];
                let c = relu[oc * hin * win + (2 * oy + 1) * win + (2 * ox)];
                let d = relu[oc * hin * win + (2 * oy + 1) * win + (2 * ox + 1)];
                let m = a.max(b).max(c).max(d);
                out[oc * hout * wout + oy * wout + ox] = m;
            }
        }
    }
    out
}

/// Conv layer 1 weights. MUST match `w1` in kernels.rs bit-for-bit
/// (the cross-file algorithmic contract; bug here = reference.bin
/// disagrees with the backend, caught by the e2e differential).
fn weight_layer_1(oc: usize, ic: usize, ky: usize, kx: usize) -> i32 {
    let mixed = oc.wrapping_mul(73)
        .wrapping_add(ic.wrapping_mul(19))
        .wrapping_add(ky.wrapping_mul(11))
        .wrapping_add(kx.wrapping_mul(7));
    (mixed % 5) as i32 - 2
}

/// Conv layer 2 weights. MUST match `w2` in kernels.rs.
fn weight_layer_2(oc: usize, ic: usize, ky: usize, kx: usize) -> i32 {
    let mixed = oc.wrapping_mul(101)
        .wrapping_add(ic.wrapping_mul(29))
        .wrapping_add(ky.wrapping_mul(13))
        .wrapping_add(kx.wrapping_mul(5));
    (mixed % 5) as i32 - 2
}

/// Classifier weights. MUST match `w_cls` in kernels.rs.
///
/// Modulus 11 (smallest M >= N_CLASSES=10 making `class*131 mod M`
/// injective on 0..9). Smaller moduli fold weight rows together;
/// M=10 would collide on class 0 and class 9 (both `0 mod 10`
/// since 131*10 % 10 = 0). Range [-5, 5].
fn weight_classifier(class: usize, k: usize) -> i32 {
    let mixed = class.wrapping_mul(131)
        .wrapping_add(k.wrapping_mul(37));
    (mixed % 11) as i32 - 5
}

fn die(msg: &str) -> ExitCode {
    eprintln!("cnn-inference-reference: {}", msg);
    ExitCode::FAILURE
}
