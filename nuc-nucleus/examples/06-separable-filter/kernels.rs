// Kernel bodies for example 06-separable-filter.
//
// PRD §6.2.2: kernel bodies are real Rust functions in an adjacent
// file, compiled by the host toolchain unmodified. Nucleus does NOT
// interpolate text into them.
//
// Four kernels:
//   - `hblur_acc(acc, sample, pos, idx)` — Pass-1 horizontal 1x5
//                                          clamp-blur fold step.
//   - `vblur_acc(acc, sample, pos, idx)` — Pass-2 vertical 5x1
//                                          clamp-blur fold step.
//   - `load_image()`  — () -> Vec<i32>, effectful. Reads H*W i32 LE
//                        words from `input.bin` (or `$NUC_INPUT_PATH`).
//   - `save_image(v)` — (Vec<i32>) -> (), effectful. Writes H*W i32
//                        LE words to `output.bin` (or
//                        `$NUC_OUTPUT_PATH`).
//
// Why the clamp lives here and not in the algorithm
// --------------------------------------------------
// A shifted tap index `in[y][x-2]` underflows `usize` at x<2 (the
// generated `in[y][(x-2) as usize]` is out of bounds), and Nuc v2
// has no conditionals (PRD §6.2.4) to clamp it. So the algorithm
// keeps the proven rectangular reduction-accumulator shape and these
// kernels decide WHICH column/row contributes — i.e. they implement
// the 5-tap clamp-to-edge stencil. Same division of labour as
// example 04-prefix-sum (see TASK-0039 / TASK-0179).
//
// The tap rule (identical structure for both passes; horizontal
// clamps the COLUMN index to [0, W-1], vertical clamps the ROW index
// to [0, H-1]): for output position `pos`, the five taps are
// `pos-2 .. pos+2`, each individually clamped to the valid range.
// `acc` accumulates `sample` once for EACH of the five taps that
// clamps onto `idx`. Edge positions therefore see the edge sample
// counted multiple times (clamp-to-edge / replicate), which is the
// intended boundary policy (AC#5) — NOT skip-with-zero (that is
// 05-stencil's policy).
//
// Why `Vec<i32>` and not `[i32; H*W]`
// -----------------------------------
// Same reason as examples 01/02/03/04/05: the PRD const-in-Rust-
// generics flow is unresolved (TASK-0103). `Vec<i32>` carries length
// at runtime; we check it explicitly in `save_image`. Resolves when
// TASK-0103 picks a convention.
//
// Contract pass (TASK-0012): the scalar kernels PASS; the
// aggregate-typed I/O kernels (`load_image` `() -> i32[H][W]`,
// `save_image` `(i32[H][W]) -> ()`) surface the known `TypeMismatch`
// (aggregate matching not yet implemented). Loud, pinned, identical
// in spirit to examples 03/04/05.
//
// I/O paths: read `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else
// sibling filenames in cwd (same convention as the other examples).

use std::env;
use std::fs;
use std::io::Write;

/// Dimensions used by the algorithm. Mirrors `const H : usize = 16;`
/// / `const W : usize = 16;` in `prog.algo.nuc`. Single-source-of-
/// truth violation (TASK-0103); disappears when the const-flow
/// convention is picked.
const H: usize = 16;
const W: usize = 16;
const N: usize = H * W;

/// Radius of the 5-tap kernel: taps are pos-2 ..= pos+2.
const RADIUS: i32 = 2;

/// Clamp `v` to the inclusive range `[0, hi]`. Plain integer min/max
/// — the boundary policy the algorithm cannot express.
fn clamp_idx(v: i32, hi: i32) -> i32 {
    if v < 0 {
        0
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Pass-1 horizontal fold. Called for every (hy, hx, hk) with
/// `acc = tmp[hy][hx]` (pre-init 0), `sample = in_arr[hy][hk]`,
/// `pos = hx`, `idx = hk`. Adds `sample` once for each of the five
/// horizontal taps of `hx` (clamped to [0, W-1]) that lands on column
/// `hk`. Over the full `hk : 0..W` sweep this yields
/// `tmp[hy][hx] = sum over the 5 clamp-to-edge taps of in_arr[hy][.]`.
pub fn hblur_acc(acc: i32, sample: i32, pos: i32, idx: i32) -> i32 {
    let hi = W as i32 - 1;
    let mut v = acc;
    let mut off = -RADIUS;
    while off <= RADIUS {
        if clamp_idx(pos + off, hi) == idx {
            v = v.wrapping_add(sample);
        }
        off += 1;
    }
    v
}

/// Pass-2 vertical fold. Called for every (vy, vx, vm) with
/// `acc = out[vy][vx]` (pre-init 0), `sample = tmp[vm][vx]`,
/// `pos = vy`, `idx = vm`. Adds `sample` once for each of the five
/// vertical taps of `vy` (clamped to [0, H-1]) that lands on row
/// `vm`. Over the full `vm : 0..H` sweep this yields the separable
/// 5x5 box SUM of the clamp-padded input.
pub fn vblur_acc(acc: i32, sample: i32, pos: i32, idx: i32) -> i32 {
    let hi = H as i32 - 1;
    let mut v = acc;
    let mut off = -RADIUS;
    while off <= RADIUS {
        if clamp_idx(pos + off, hi) == idx {
            v = v.wrapping_add(sample);
        }
        off += 1;
    }
    v
}

pub fn load_image() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_image: cannot read {}: {}", path, e));
    let need = N * 4;
    assert!(
        bytes.len() >= need,
        "load_image: file {} has {} bytes; need at least {} (H*W = {}*{} = {})",
        path,
        bytes.len(),
        need,
        H,
        W,
        N
    );
    let mut out = Vec::with_capacity(N);
    for i in 0..N {
        let off = i * 4;
        let word = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push(word);
    }
    out
}

pub fn save_image(v: Vec<i32>) {
    assert_eq!(
        v.len(),
        N,
        "save_image: expected {} elements (H*W = {}*{}), got {}",
        N,
        H,
        W,
        v.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in &v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_image: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_image: write failed: {}", e));
}
