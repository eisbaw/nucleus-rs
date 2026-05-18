// Kernel bodies for example 05-stencil.
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does NOT interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Three kernels:
//   - `blur3(p0..p8)` — scalar nine-input box-blur, pure.
//   - `load_image()` — () -> Vec<i32>, effectful. Reads H*W i32 LE
//                      words from `input.bin` (or `$NUC_INPUT_PATH`).
//   - `save_image(img)` — (Vec<i32>) -> (), effectful. Writes H*W
//                         i32 LE words to `output.bin` (or
//                         `$NUC_OUTPUT_PATH`).
//
// Why `Vec<i32>` and not `[i32; H*W]`
// -----------------------------------
// Same reason as examples 01 / 02 / 03: PRD const-in-Rust-generics
// flow is unresolved (TASK-0103). `Vec<i32>` carries length at
// runtime; we check it explicitly in `save_image`. Trade-off: shape
// errors become runtime panics rather than compile-time mismatches.
// Resolves when TASK-0103 picks a convention.
//
// The algorithm declares `img_in / img_out : i32[H][W]`. On the Rust
// side these are single flat `Vec<i32>` of length H*W = 256, laid out
// row-major. The codegen (pthreads-sync) flattens the 2D index
// `img_in[y][x]` to `img_in[y * W + x]` at compile time using the
// data shape. `load_image` just returns the flat vector in file
// order — that file order MUST match the row-major layout. The
// committed `input.bin` (see README) is generated row-major by
// construction.
//
// Why integer division (and not e.g. round-half-to-even)
// ------------------------------------------------------
// PRD §10.1 wants bit-deterministic output across schedules and
// backends. Rust's `i32 / i32` is truncating integer division —
// deterministic, no rounding mode, no platform variation. A "true
// average" via float would be order-of-summation sensitive and
// reorderable (which is precisely what schedules will reorder). We
// take the precision hit. The reference impl uses the SAME
// expression so the differential test stays meaningful.
//
// Contract pass (TASK-0012) expected behaviour against this file:
//   - PASS for `blur3`            — declared `(i32, ..., i32) -> i32`,
//                                   nine scalar params, scalar return.
//   - `TypeMismatch` for `load_image`, `save_image` — declared
//                                   aggregate `i32[H][W]`, matched
//                                   against `Vec<i32>` (scalar-only
//                                   matching at M1). Loud failure,
//                                   pinned by the contract test.
//
// I/O paths: same convention as examples 01 / 02 / 03. Read from
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else conventional
// sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Dimensions used by the algorithm. Mirrors `const H : usize = 16;`
/// and `const W : usize = 16;` in `prog.algo.nuc`. Single-source-of-
/// truth violation (TASK-0103); disappears when the const-flow
/// convention is picked.
const H: usize = 16;
const W: usize = 16;
const N: usize = H * W;

/// 3x3 box blur of nine neighbouring pixels.
///
/// Sums the nine inputs in left-to-right, top-to-bottom order and
/// divides by 9 (integer truncating division). The sum order is
/// fixed by left-to-right evaluation in Rust — and even if it were
/// not, integer `wrapping_add` is associative under two's-complement
/// wraparound, so reordering would be safe. The reference impl uses
/// the same expression for the differential test.
///
/// Overflow note: each pixel fits in i32, and nine pixels sum to at
/// most ~9 * i32::MAX. That overflows i32. We use `wrapping_add` to
/// keep behaviour defined; the committed `input.bin` stays inside a
/// safe range (pixel values <= 16 * 16 * 7 = 1792), so no wraparound
/// actually happens in practice. The choice documents intent and
/// makes pathological inputs panic-free.
pub fn blur3(
    p0: i32,
    p1: i32,
    p2: i32,
    p3: i32,
    p4: i32,
    p5: i32,
    p6: i32,
    p7: i32,
    p8: i32,
) -> i32 {
    let sum = p0
        .wrapping_add(p1)
        .wrapping_add(p2)
        .wrapping_add(p3)
        .wrapping_add(p4)
        .wrapping_add(p5)
        .wrapping_add(p6)
        .wrapping_add(p7)
        .wrapping_add(p8);
    sum / 9
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

pub fn save_image(img: Vec<i32>) {
    assert_eq!(
        img.len(),
        N,
        "save_image: expected {} elements (H*W = {}*{}), got {}",
        N,
        H,
        W,
        img.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(img.len() * 4);
    for v in &img {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_image: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_image: write failed: {}", e));
}
