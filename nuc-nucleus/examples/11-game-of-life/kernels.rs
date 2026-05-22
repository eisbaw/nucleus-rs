// Kernel bodies for example 11-game-of-life.
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does NOT interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Four kernels:
//   - `step_or_seed(pl, pm, pr, sm, t)` — scalar
//                     `(i32, i32, i32, i32, i32) -> i32`, pure.
//                     Returns `sm` (the seed cell) when `t == 0`;
//                     returns `pl + pm + pr` (wrapping_add) when
//                     `t >= 1`. The Nuc-level single-Dataflow
//                     constraint forces the seed-boundary case into
//                     the same loop body as the iteration body; the
//                     branch on `t` is the smallest expression of
//                     "use seed_m at t=0, three-tap sum at t>=1".
//   - `ident`       — scalar (i32) -> i32, pure. Identity copy.
//                     Used to extract `grid[ITERS]` into `result`
//                     (see `prog.algo.nuc` for the rationale).
//   - `load_input`  — () -> Vec<i32>, effectful. Reads N i32 LE
//                     words from `input.bin`.
//   - `save_output` — (Vec<i32>) -> (), effectful. Writes N i32 LE
//                     words to the output path.
//
// Numeric type choice: i32
// ------------------------
// PRD §13 and docs/reference-impl-policy.md §5. Integer arithmetic is
// bit-deterministic by Rust's language definition; floating-point is
// not. The 1D additive cellular automaton here grows by at most a
// factor of 3 per generation from a worst-case unit seed (3^8 ≈ 6561
// for ITERS=8), well inside the i32 range from the committed seed
// pattern (see README §"Input pattern"). `wrapping_add` documents the
// overflow contract so a pathological seed cannot panic the program;
// it does not actually wrap on the committed input.
//
// Why `Vec<i32>` and not `[i32; N]`
// ---------------------------------
// See examples 01..07/09/13. TASK-0103 is the open PRD question for
// aggregate-type matching; until it lands, aggregate kernel I/O uses
// `Vec<i32>` with a runtime length assertion in `save_output`.
//
// Contract pass (TASK-0012) behaviour expected against this file
// --------------------------------------------------------------
// `check_kernels_contract` is scalar-only at present. It will:
//   - PASS for `step_or_seed` — declared `(i32, i32, i32, i32, i32) ->
//                                i32`, five scalar params, scalar
//                                return.
//   - PASS for `ident`        — declared `(i32) -> i32`, signature matches.
//   - REPORT `TypeMismatch` with "aggregate type matching is not yet
//     implemented" for `load_input` and `save_output` because their
//     Nuc-side declarations are aggregate (`i32[N]`). Loud failure,
//     not silent acceptance — same pattern as every other example;
//     not a bug here.
//
// I/O paths
// ---------
// Read paths from environment variables when set, falling back to
// conventional sibling filenames in the cwd. This is what the
// pthreads-sync / pthreads-async / mp-tcp-bufsync emitted host
// program threads in via `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH`.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 32;` in
/// `prog.algo.nuc`. The duplication is the single-source-of-truth
/// violation called out in TASK-0103 and shared with every other
/// example.
const N: usize = 32;

/// Iterated three-tap 1D stencil step, with a seed-fallback path for
/// the `t == 0` boundary case (see `prog.algo.nuc` for why the
/// boundary lives in the kernel rather than the algorithm).
///
/// - `pl`, `pm`, `pr` — three cells from the previous generation
///                       (read by the algorithm from
///                       `grid[(t+ITERS)%(ITERS+1)][...]`). When
///                       `t == 0`, these are reads of pre-initialised
///                       zero cells and are ignored.
/// - `sm`             — the corresponding cell of `seed`. Only used
///                       when `t == 0` (where it becomes the result —
///                       grid[0][i] = seed[i] by definition).
/// - `t`              — the current generation index. 0 means "seed
///                       case"; >=1 means "step case".
///
/// Implementation:
///   - `t == 0`: return `sm` directly (no arithmetic; the seed IS
///               grid[0]).
///   - `t >= 1`: return `pl.wrapping_add(pm).wrapping_add(pr)` (the
///               three-tap sum, wrapping_add to document the
///               overflow contract). The committed input + ITERS=8
///               stays far below `i32::MAX`, so wrap is never tripped
///               in practice.
///
/// Two explicit `wrapping_add` calls catch a bug that drops one
/// operand or uses `+` (which would panic in debug on overflow rather
/// than wrap).
pub fn step_or_seed(pl: i32, pm: i32, pr: i32, sm: i32, t: i32) -> i32 {
    if t == 0 {
        sm
    } else {
        pl.wrapping_add(pm).wrapping_add(pr)
    }
}

/// Identity copy. The cheapest pure kernel that exists; present here
/// to stage grid[ITERS] -> result inside the single-assignment shape.
/// Compiles to a single move under release.
pub fn ident(v: i32) -> i32 {
    v
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn save_output(data: Vec<i32>) {
    assert_eq!(
        data.len(),
        N,
        "save_output: expected {} elements, got {}",
        N,
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

/// Read `count` little-endian i32 words from `path`, starting at
/// element offset `start` (i.e. byte offset `start * 4`).
fn read_i32_le_slice(path: &str, start: usize, count: usize) -> Vec<i32> {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = (start + count) * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {}",
        path,
        bytes.len(),
        need
    );
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = (start + i) * 4;
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
