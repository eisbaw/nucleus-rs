// Kernel bodies for the VGA frame-strip stencil production case study.
//
// Identical arithmetic to nuc-nucleus/examples/05-stencil/kernels.rs;
// the ONLY differences are the dimensions (H=15360, W=640 vs 16,16).
// H = 15360 = 32 VGA frames (32*480) stacked vertically — see
// prog.algo.nuc's size-justification header.
// PRD §6.2.2: kernel bodies are real Rust functions in an adjacent
// source file. Nucleus does NOT interpolate text into these bodies —
// the H/W duplication against prog.algo.nuc is the documented v2
// trade-off (TASK-0103); kernels.rs is plain Rust compiled by the host
// toolchain unmodified.
//
// Three kernels:
//   - blur3(p0..p8)       — scalar nine-input 3x3 box blur, pure.
//   - load_image()        — () -> Vec<i32>, effectful. Reads H*W i32 LE
//                           words from $NUC_INPUT_PATH (or input.bin).
//   - save_image(img)     — (Vec<i32>) -> (), effectful. Writes H*W i32
//                           LE words to $NUC_OUTPUT_PATH (or output.bin).
//
// I/O paths come from env vars so the generated program is
// location-independent — this is how the runner script points each
// backend at the same input and diffs their outputs against the
// independent reference.

use std::env;
use std::fs;
use std::io::Write;

/// Frame-strip dimensions (32 VGA frames). Mirrors `const H : usize =
/// 15360;` and `const W : usize = 640;` in prog.algo.nuc. The
/// duplication is the v2 convention (TASK-0103): kernels.rs is plain
/// Rust; Nucleus does not text-substitute algorithm consts into kernel
/// bodies.
const H: usize = 15360;
const W: usize = 640;
const N: usize = H * W;

/// 3x3 box blur of nine neighbouring pixels.
///
/// Sums the nine inputs left-to-right, top-to-bottom and divides by 9
/// (truncating integer division). `wrapping_add` keeps overflow defined
/// (deterministic two's-complement wraparound); the case study's
/// generated `input.bin` stays inside a safe range so no wraparound
/// actually happens. The reference oracle uses the SAME expression for
/// the differential test (docs/reference-impl-policy.md §5).
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
