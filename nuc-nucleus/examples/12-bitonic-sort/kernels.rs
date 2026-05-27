// Kernel bodies for example 12-bitonic-sort.
//
// PRD §6.2.2: kernel bodies are real Rust functions in an adjacent
// file, compiled by the host toolchain unmodified. Nucleus does NOT
// interpolate text into them.
//
// Three kernels:
//   - `bitonic_sort(input)` — pure. Returns the sorted permutation
//                             of `input` (length N power-of-2). The
//                             compare-exchange network is implemented
//                             in straight-line Rust.
//   - `load_input()`        — () -> Vec<i32>, effectful. Reads N i32
//                             LE words from `input.bin` (or
//                             `$NUC_INPUT_PATH`).
//   - `save_output(v)`      — (Vec<i32>) -> (), effectful. Writes
//                             N i32 LE words to `output.bin` (or
//                             `$NUC_OUTPUT_PATH`).
//
// Why the network lives here and not in the algorithm
// -----------------------------------------------------
// See `prog.algo.nuc` header. Bitonic sort has log2(N) outer stages
// × variable-length inner stages × per-element bit-pattern-dependent
// pair assignment; expressing this at the v2 algorithm level would
// require either (a) ~21 hardcoded stage blocks for N=64, OR
// (b) variable loop bounds — both currently blocked by sublanguage
// rules. The honest workaround is to push the entire network into a
// single pure kernel.
//
// Why `Vec<i32>` and not `[i32; N]`
// ---------------------------------
// Per TASK-0103 (Done cycle 17): `Vec<i32>` IS the canonical
// convention for aggregate-typed kernel signatures. Length is
// checked at runtime against the algorithm's `const N`.
//
// I/O paths: same convention as the other examples — read
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else sibling filenames
// in cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Sort length. Mirrors `const N : usize = 64;` in `prog.algo.nuc`.
const N: usize = 64;

/// Bitonic sort: outer stage `k` (1-based size of bitonic subsequence
/// = 2^k), inner stage `j` (compare-exchange distance = 2^(j-1)).
/// Direction is determined by the bit `i & (1 << k)`: if 0, ascending;
/// if 1, descending. After log2(N) outer stages the whole array is
/// ascending.
///
/// This is the textbook iterative bitonic sort. Straight-line Rust;
/// no recursion. Indices computed from i and the stage bit masks.
pub fn bitonic_sort(input: Vec<i32>) -> Vec<i32> {
    assert_eq!(
        input.len(),
        N,
        "bitonic_sort: input has {} elements; need {}",
        input.len(),
        N
    );
    let mut a = input;
    let mut k = 2usize;
    while k <= N {
        let mut j = k >> 1;
        while j > 0 {
            for i in 0..N {
                let l = i ^ j;
                if l > i {
                    // Ascending iff bit (i & k) is 0; descending iff set.
                    let ascending = (i & k) == 0;
                    let need_swap = if ascending {
                        a[i] > a[l]
                    } else {
                        a[i] < a[l]
                    };
                    if need_swap {
                        a.swap(i, l);
                    }
                }
            }
            j >>= 1;
        }
        k <<= 1;
    }
    a
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = N * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {} (N={})",
        path,
        bytes.len(),
        need,
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

pub fn save_output(v: Vec<i32>) {
    assert_eq!(
        v.len(),
        N,
        "save_output: v has {} elements; need {}",
        v.len(),
        N
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    let mut bytes = Vec::with_capacity(N * 4);
    for w in v {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
