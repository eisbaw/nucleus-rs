// Kernel bodies for example 16-jacobi. Landed cycle 206 (TASK-0341.02
// AC#1 fixed-iteration language-sanity slice).
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does NOT interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Four kernels:
//   - `jacobi5_or_seed(pn, ps, pe, pw, sy, t)` — scalar
//                     `(i32, i32, i32, i32, i32, i32) -> i32`, pure.
//                     Returns `sy` (seed cell) when `t == 0`; returns
//                     `(pn + ps + pe + pw) / 4` (truncating) when
//                     `t >= 1`. Branches on `t` to fold the seed-
//                     staging case into the same Dataflow as the
//                     iteration body (same trick as 11-game-of-life's
//                     `step_or_seed`).
//   - `ident`       — scalar (i32) -> i32, pure. Identity copy. Used
//                     to extract `field[ITERS]` into `result`.
//   - `load_input`  — () -> Vec<i32>, effectful. Reads H*W i32 LE
//                     words from `input.bin` (row-major).
//   - `save_output` — (Vec<i32>) -> (), effectful. Writes H*W i32 LE
//                     words to the output path (row-major).
//
// Numeric type choice: i32 with truncating division
// -------------------------------------------------
// PRD §13 and docs/reference-impl-policy.md §5. Integer arithmetic is
// bit-deterministic across schedules and backends. The 4-tap sum of
// i32 cells is computed with `wrapping_add`; the divide-by-4 is
// integer truncating division. The reference impl uses the SAME
// expression so the differential test stays meaningful.
//
// The committed input pattern keeps all seed cells in 0..256 (see
// README), so the worst-case 4-tap sum is 4*255 = 1020, far inside
// the i32 range; `wrapping_add` documents the overflow contract but
// is never tripped by the fixture. After 4 Jacobi iterations on this
// fixture the values diffuse toward 0 (the Dirichlet zero-boundary
// acts as a low-pass sink); the exact bit pattern depends on the
// iteration order at the kernel arithmetic level only, not on
// schedule choices.
//
// Why `Vec<i32>` and not `[i32; H*W]`
// -----------------------------------
// TASK-0103 (Done cycle 17) convention. `field` is declared
// `i32[ITERS+1][H][W]` in the algorithm but it is NOT a kernel I/O
// type — only `seed` (declared `i32[H][W]` in/out via load_input) and
// `result` (declared `i32[H][W]` out via save_output) cross the
// aggregate-IO boundary. Both use `Vec<i32>` length H*W on the Rust
// side; the codegen flattens 2D index `seed[y][x]` to
// `seed[y * W + x]`.
//
// Contract pass (TASK-0012) expected behaviour against this file:
//   - PASS for `jacobi5_or_seed` — declared
//                              `(i32, ..., i32) -> i32`, six scalar
//                              params, scalar return.
//   - PASS for `ident`          — declared `(i32) -> i32`.
//   - `TypeMismatch` for `load_input`, `save_output` — declared
//                              aggregate `i32[H][W]`, matched against
//                              `Vec<i32>` (scalar-only matcher at M1).
//                              Loud failure, pinned by the contract
//                              test. Same pattern as every other
//                              aggregate-IO example.
//
// I/O paths: NUC_INPUT_PATH / NUC_OUTPUT_PATH override the
// conventional sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Dimensions used by the algorithm. Mirror `const H : usize = 8;`,
/// `const W : usize = 8;`, `const ITERS : usize = 4;` in
/// `prog.algo.nuc`. The doubled declaration is the v2 convention per
/// TASK-0103 (kernels.rs is plain Rust compiled unmodified; Nucleus
/// does not text-substitute algorithm consts into kernel bodies).
const H: usize = 8;
const W: usize = 8;
const N: usize = H * W;

/// One Jacobi-step kernel call, with a seed-fallback branch on `t`.
/// At `t == 0` the kernel returns the seed cell verbatim (the
/// pre-initialised zero field[ITERS] cells the algorithm reads
/// through `prev_*` are ignored). At `t >= 1` the kernel returns the
/// 4-tap average over the previous generation's north/south/east/west
/// neighbours (`wrapping_add` for the sum, `/ 4` for the truncating
/// integer divide).
pub fn jacobi5_or_seed(
    prev_n: i32,
    prev_s: i32,
    prev_e: i32,
    prev_w: i32,
    seed_yx: i32,
    t: i32,
) -> i32 {
    if t == 0 {
        seed_yx
    } else {
        let sum = prev_n
            .wrapping_add(prev_s)
            .wrapping_add(prev_e)
            .wrapping_add(prev_w);
        sum / 4
    }
}

/// Identity copy. Used to stage `field[ITERS][y][x]` into the flat
/// `result[y][x]` array that `save_output` consumes.
pub fn ident(v: i32) -> i32 {
    v
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = N * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {} (H*W = {}*{} = {})",
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
        let word = i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]);
        out.push(word);
    }
    out
}

pub fn save_output(data: Vec<i32>) {
    assert_eq!(
        data.len(),
        N,
        "save_output: expected {} elements (H*W = {}*{}), got {}",
        N,
        H,
        W,
        data.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(data.len() * 4);
    for v in &data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
