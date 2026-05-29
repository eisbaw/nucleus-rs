// no_std-clean kernel bodies for example 14-hearing-aid — the EMBEDDED
// (per-frame) algorithm shape `prog.embedded.algo.nuc`, selected via the
// driver `--kernels` flag (TASK-0049.06).
//
// WHY A SEPARATE KERNELS FILE (decision recorded TASK-0049.06):
//
//   The tier-1 `kernels.rs` (this dir) backs the 7 passing M6 naive
//   e2e cells (bulk Vec<i32> IO over std::fs). It is `std`-bound by
//   design and MUST stay untouched. The embedded-pattern backend
//   cross-compiles its extracted PURE kernel bodies under
//   `no_std` (`thumbv7em-none-eabihf`), so it needs an alloc-free,
//   fixed-array variant. This file provides exactly that for the
//   embedded (per-frame) algorithm shape.
//
// HOW THE BACKEND USES THIS FILE (read before editing — the two kernel
// classes are treated DIFFERENTLY):
//
//   - PURE kernels (mix2, denoise): the embedded-pattern backend
//     extracts the `pub fn` body VERBATIM into the generated `no_std`
//     lib's `mod kernels` (PRD §6.2.2 — Nucleus copies the user's text
//     unchanged, it does not interpolate). So mix2/denoise MUST be
//     no_std-clean: fixed-size `[i32; SAMPLES_PER_FRAME]` in/out, NO
//     `Vec`, NO `alloc`, NO `std::` — only inherent integer ops
//     (`wrapping_add`) and array indexing, which are `core`-available.
//     The fixed-array signature matches the algorithm declaration
//     `(i32[SAMPLES_PER_FRAME], ...) -> i32[SAMPLES_PER_FRAME]` and the
//     backend's fixed-`[T; N]` data layout (each `i32[N_FRAMES][SPF]`
//     datum is a flat `[i32; N_FRAMES * SPF]` local; a single-index
//     `mic_in[frame]` read slices out one contiguous `[i32; SPF]` row).
//
//   - EFFECTFUL kernels (fe_capture, rf_receive, fe_emit, rf_transmit):
//     the backend does NOT extract these — they map to NucleusShim DMA
//     hooks (an input fills a region via `alloc_in_region`; an output
//     drains via `dma_push`). The bodies below exist ONLY to satisfy
//     `check_kernels_contract` (which requires a `pub fn` of the right
//     arity for every declared kernel) and are deliberately trivial.
//     They are NOT compiled into the firmware. They are written
//     no_std-clean too (no `std::`, no `Vec`) so the whole file is an
//     honest representation of the embedded world, even though only the
//     pure bodies are extracted.
//
// AGGREGATE-CONTRACT NOTE (expected, not a defect): mix2/denoise have
// aggregate (`[i32; SPF]`) signatures. `check_kernels_contract` only
// matches SCALAR signatures and emits a `TypeMismatch` warning for any
// aggregate-typed kernel (the known TASK-0012 gap) — the SAME warning
// the tier-1 kernels.rs triggers for its `Vec<i32>` IO kernels. The
// driver surfaces this as a non-fatal warning and proceeds.
//
// Numeric choice: i32 + wrapping_add (PRD §10.1), identical semantics
// to the tier-1 kernels.rs mix2/denoise so a future shared reference
// impl can compare bit-for-bit.

// SAMPLES_PER_FRAME is 16 in prog.embedded.algo.nuc. The PURE kernels
// below are SELF-CONTAINED — they use the array length literal `16`
// inline (and `buf.len()` for loop bounds) rather than a file-level
// `const SAMPLES_PER_FRAME`. This is DELIBERATE and load-bearing: the
// embedded-pattern backend extracts each `pub fn` body VERBATIM
// (kernel_extract::extract_pub_fn) but does NOT carry file-level
// `const` declarations into the generated `mod kernels`. A signature
// like `[i32; SAMPLES_PER_FRAME]` would therefore reference an
// undefined name once extracted (E0425), so the no_std-clean kernels
// MUST be const-free / self-contained. Effectful kernel bodies (below)
// are NOT extracted, but they follow the same rule for consistency.

// ------------------------- pure kernels (EXTRACTED verbatim) --------

/// Per-sample i32::wrapping_add of two 16-sample frames (mic + BT into
/// the speaker path). Fixed-array, alloc-free, const-free — no_std-clean
/// AND self-contained for the embedded backend's verbatim extraction.
pub fn mix2(a: [i32; 16], b: [i32; 16]) -> [i32; 16] {
    let mut out = [0i32; 16];
    let mut i = 0;
    while i < a.len() {
        out[i] = a[i].wrapping_add(b[i]);
        i += 1;
    }
    out
}

/// 3-wide sliding sum with edge replication — the deterministic integer
/// spectral-smoothing stand-in (NO FFT, NO division; PRD §10.1). Same
/// expression as the tier-1 kernels.rs denoise, rewritten fixed-array,
/// alloc-free AND const-free for no_std verbatim extraction.
pub fn denoise(buf: [i32; 16]) -> [i32; 16] {
    let n = buf.len();
    let mut out = [0i32; 16];
    let mut i = 0;
    while i < n {
        let l = buf[if i > 0 { i - 1 } else { 0 }];
        let c = buf[i];
        let r = buf[if i + 1 < n { i + 1 } else { n - 1 }];
        out[i] = l.wrapping_add(c).wrapping_add(r);
        i += 1;
    }
    out
}

// ------------------------- effectful kernels (NOT extracted) --------
//
// These map to NucleusShim DMA hooks in the generated firmware and are
// NOT compiled into it. The bodies exist only to satisfy the kernel
// contract's "a `pub fn` of the right arity exists per declared kernel"
// requirement. Trivial + no_std-clean.

/// Per-frame mic capture (simulated I2S microphone). In firmware this
/// maps to a DMA fill via NucleusShim::alloc_in_region; this body is
/// not extracted.
pub fn fe_capture() -> [i32; 16] {
    [0i32; 16]
}

/// Per-frame Bluetooth receive (simulated radio). Maps to a DMA fill in
/// firmware; this body is not extracted.
pub fn rf_receive() -> [i32; 16] {
    [0i32; 16]
}

/// Per-frame speaker emit (simulated DAC). Maps to a DMA drain
/// (dma_push) in firmware; this body is not extracted.
pub fn fe_emit(_frame: [i32; 16]) {}

/// Per-frame Bluetooth transmit (simulated radio). Maps to a DMA drain
/// in firmware; this body is not extracted.
pub fn rf_transmit(_frame: [i32; 16]) {}
