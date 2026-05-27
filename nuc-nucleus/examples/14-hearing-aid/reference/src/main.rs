//! Reference implementation + fixture generator for example
//! 14-hearing-aid (TASK-0054).
//!
//! Computes the hearing-aid pipeline a SECOND, independent way:
//! frame-by-frame functional composition with explicit local
//! variables for the intermediate values (mic_clean, mixed,
//! mixed_clean), vs the Nucleus kernel's loop body where the
//! denoise + mix2 calls are arguments-in-place. Both produce identical
//! output because the operations are deterministic i32 arithmetic and
//! function composition is order-independent for fixed operands. The
//! point of the oracle is to be "wrong in a different way if wrong at
//! all" (docs/reference-impl-policy.md §2). If the Nucleus output
//! matches this composition-with-locals reference bit-for-bit on all
//! 7 tier-1 backends, the pipeline is correct.
//!
//! Independence (policy §2): no dependency on Nucleus, no shared code
//! with `kernels.rs`, std only, no third-party crates, no threads,
//! no HashMap, no Instant-derived values.
//!
//! Two subcommands:
//!   --gen-input PATH   write the canonical `input.bin` (mic+bt
//!                      concatenated, N_FRAMES*SAMPLES_PER_FRAME i32
//!                      LE words each, mic-first), no python step.
//!   --in PATH --out PATH
//!                      read `input.bin`, run the pipeline, write
//!                      `reference.bin` (spk+bt_out concatenated, same
//!                      shape as input).
//!
//! N_FRAMES / SAMPLES_PER_FRAME are fixed at the values declared in
//! `prog.algo.nuc`. If those consts change, this binary must change
//! in the same commit (policy §3).
//!
//! Determinism rules (policy §5): integer arithmetic only;
//! `wrapping_add` makes overflow explicit; strict element-order.
//!
//! Input distribution (--gen-input)
//! ---------------------------------
//! mic[f][s] = ((f * 7 + s * 3 + 1) & 0x7F) - 64       // range [-64, 63]
//! bt[f][s]  = ((f * 11 + s * 5 + 2) & 0x7F) - 64      // range [-64, 63]
//! Distinct seeds for mic vs bt so the cross-mix produces non-trivial
//! output. Range stays well inside i32; the 3-wide denoise sum at
//! worst triples a value (|3 * 64| = 192, no overflow); the final
//! mix2+denoise nests one more wrapping_add (192 * 2 = 384, no
//! overflow).

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N_FRAMES: usize = 4;
const SAMPLES_PER_FRAME: usize = 16;
const BUFFER_LEN: usize = N_FRAMES * SAMPLES_PER_FRAME;
const BUFFER_BYTES: usize = BUFFER_LEN * 4;
const INPUT_BYTES: usize = 2 * BUFFER_BYTES;
const OUTPUT_BYTES: usize = 2 * BUFFER_BYTES;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 3 && args[1] == "--gen-input" {
        return gen_input(&args[2]);
    }
    if args.len() == 5 && args[1] == "--in" && args[3] == "--out" {
        return run_reference(&args[2], &args[4]);
    }
    eprintln!(
        "usage:\n  hearing-aid-reference --gen-input PATH\n  hearing-aid-reference --in IN_PATH --out OUT_PATH"
    );
    ExitCode::FAILURE
}

fn gen_input(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    // mic block first.
    for f in 0..N_FRAMES {
        for s in 0..SAMPLES_PER_FRAME {
            let v: i32 = (((f as i32) * 7 + (s as i32) * 3 + 1) & 0x7F) - 64;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    // bt block second.
    for f in 0..N_FRAMES {
        for s in 0..SAMPLES_PER_FRAME {
            let v: i32 = (((f as i32) * 11 + (s as i32) * 5 + 2) & 0x7F) - 64;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("--gen-input: cannot create {}: {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = f.write_all(&bytes) {
        eprintln!("--gen-input: write failed: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_reference(in_path: &str, out_path: &str) -> ExitCode {
    let bytes = match fs::read(in_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("--in: cannot read {}: {}", in_path, e);
            return ExitCode::FAILURE;
        }
    };
    if bytes.len() < INPUT_BYTES {
        eprintln!(
            "--in: file {} has {} bytes; need at least {}",
            in_path,
            bytes.len(),
            INPUT_BYTES
        );
        return ExitCode::FAILURE;
    }

    // Parse mic + bt as 2D i32 arrays.
    let mic = parse_buffer(&bytes, 0);
    let bt = parse_buffer(&bytes, BUFFER_BYTES);

    let mut spk = vec![vec![0i32; SAMPLES_PER_FRAME]; N_FRAMES];
    let mut bt_out = vec![vec![0i32; SAMPLES_PER_FRAME]; N_FRAMES];

    // Pipeline (independent control structure vs kernels.rs):
    //   1. Compute mic_clean per frame (outbound denoise).
    //   2. Assign bt_out := mic_clean.
    //   3. Compute mixed per frame (mic + bt).
    //   4. Compute mixed_clean per frame (inbound denoise).
    //   5. Assign spk := mixed_clean.
    // The kernels.rs version weaves these together in the for-loop
    // body via argument-in-place composition.
    for f in 0..N_FRAMES {
        let mic_clean = denoise(&mic[f]);
        for s in 0..SAMPLES_PER_FRAME {
            bt_out[f][s] = mic_clean[s];
        }
    }
    for f in 0..N_FRAMES {
        let mixed = mix2(&mic[f], &bt[f]);
        let mixed_clean = denoise(&mixed);
        for s in 0..SAMPLES_PER_FRAME {
            spk[f][s] = mixed_clean[s];
        }
    }

    // Write spk + bt_out (same layout as input: spk-first).
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for f in 0..N_FRAMES {
        for s in 0..SAMPLES_PER_FRAME {
            out_bytes.extend_from_slice(&spk[f][s].to_le_bytes());
        }
    }
    for f in 0..N_FRAMES {
        for s in 0..SAMPLES_PER_FRAME {
            out_bytes.extend_from_slice(&bt_out[f][s].to_le_bytes());
        }
    }
    let mut f = match fs::File::create(out_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("--out: cannot create {}: {}", out_path, e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = f.write_all(&out_bytes) {
        eprintln!("--out: write failed: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn parse_buffer(bytes: &[u8], start: usize) -> Vec<Vec<i32>> {
    let mut buf = vec![vec![0i32; SAMPLES_PER_FRAME]; N_FRAMES];
    for f in 0..N_FRAMES {
        for s in 0..SAMPLES_PER_FRAME {
            let off = start + (f * SAMPLES_PER_FRAME + s) * 4;
            buf[f][s] =
                i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        }
    }
    buf
}

fn mix2(a: &[i32], b: &[i32]) -> Vec<i32> {
    let mut out = Vec::with_capacity(SAMPLES_PER_FRAME);
    for i in 0..SAMPLES_PER_FRAME {
        out.push(a[i].wrapping_add(b[i]));
    }
    out
}

fn denoise(buf: &[i32]) -> Vec<i32> {
    let n = SAMPLES_PER_FRAME;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let l = buf[if i > 0 { i - 1 } else { 0 }];
        let c = buf[i];
        let r = buf[if i + 1 < n { i + 1 } else { n - 1 }];
        out.push(l.wrapping_add(c).wrapping_add(r));
    }
    out
}
