//! `render_fire_args` (tier-1 std `.to_vec()`) vs
//! `render_fire_args_nostd` (embedded no_std `.try_into().unwrap()`)
//! divergence pin (TASK-0049.06).
//!
//! WHY THIS EXISTS: the embedded-pattern backend cross-compiles its
//! generated kernel calls under `no_std` (`thumbv7em-none-eabihf`). A
//! contiguous-prefix sub-array argument (e.g. `mic_in[frame]` of an
//! `i32[N_FRAMES][16]` datum — an array-typed PURE kernel param) was
//! lowered by the shared `render_fire_args` as `<slice>.to_vec()`, which
//! needs `alloc`/`Vec` and does NOT cross-compile under `no_std`. That
//! was the GAP-C blocker the M11 real-example-14 cross-compile surfaced.
//!
//! The fix added `render_fire_args_nostd` (SubArrayForm::FixedArray):
//! sub-array args render as `<slice>.try_into().unwrap()` (a `core`,
//! alloc-free `[T; N]`), inferred from the kernel signature's `[T; N]`
//! param at the call site.
//!
//! This file PINS the divergence so a future refactor cannot silently
//! collapse the two forms (which would either (a) break tier-1's
//! `Vec<T>` kernel ABI, or (b) re-introduce `.to_vec()` into the no_std
//! lowering and break the embedded cross-compile). The two forms MUST
//! differ on a sub-array arg and MUST be identical on a scalar arg.

use nucleus_compiler::algo::{IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{ArgBinding, DataId, DataSlice, KernelId};
use nucleus_compiler::name_tables::NameTables;
use nucleus_compiler::sidecar::{KernelSig, NameSidecar};

use backend_common::render::{render_fire_args, render_fire_args_nostd, EmitError, RenderCtx};

/// Build a `(NameTables, NameSidecar)` describing one data symbol `D`
/// of shape `dims` and one kernel `k` whose i-th param type is
/// `param_ty`.
fn fixtures(
    data: DataId,
    data_dims: Vec<usize>,
    kernel: KernelId,
    param_ty: ResolvedType,
) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.data.insert(data, "D".to_string());
    names.kernel.insert(kernel, "k".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.data_types.insert(
        data,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: data_dims,
        },
    );
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![param_ty],
            ret: None,
        },
    );
    (names, sidecar)
}

#[test]
fn sub_array_arg_diverges_vec_vs_fixed_array() {
    // D : i32[4][16]; the arg `D[frame]` indexes only the OUTER axis →
    // a contiguous `[i32; 16]` sub-array (partial-prefix rank). The
    // kernel param is `[i32; 16]` (fixed array, the no_std convention).
    let data = DataId(0);
    let kernel = KernelId(0);
    let (names, sidecar) = fixtures(
        data,
        vec![4, 16],
        kernel,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16],
        },
    );
    let ctx = RenderCtx::new(&names, &sidecar);

    // `D[frame]` — one outer index over rank-2 data.
    let inputs = vec![ArgBinding::Data(DataSlice {
        data,
        indices: vec![IrExpr::Ident("frame".to_string())],
    })];

    let vec_form = render_fire_args(kernel, &inputs, &ctx).expect("vec form renders");
    let nostd_form = render_fire_args_nostd(kernel, &inputs, &ctx).expect("nostd form renders");

    // Tier-1 std: owned Vec via `.to_vec()`. The start expression is
    // `((frame) * 16) as usize` (classify_data_slice bakes the stride
    // into the `start` and casts to usize).
    assert_eq!(
        vec_form, "D[((frame) * 16) as usize..((frame) * 16) as usize + 16usize].to_vec()",
        "tier-1 sub-array arg must materialise as `.to_vec()` (Vec<T>)"
    );
    // no_std/embedded: owned fixed array via `.try_into().unwrap()`,
    // same `start..start + sub_len` window, alloc-free materialisation.
    assert_eq!(
        nostd_form,
        "D[((frame) * 16) as usize..((frame) * 16) as usize + 16usize].try_into().unwrap()",
        "no_std sub-array arg must materialise as `.try_into().unwrap()` ([T; N], alloc-free)"
    );
    // The whole point: the two forms DIVERGE on a sub-array arg, and the
    // no_std form is alloc-free (no `.to_vec()`).
    assert_ne!(
        vec_form, nostd_form,
        "the two arg forms must diverge on a sub-array arg"
    );
    assert!(
        !nostd_form.contains(".to_vec()"),
        "the no_std form must NOT use `.to_vec()` (needs alloc): {nostd_form}"
    );
}

#[test]
fn scalar_arg_identical_vec_vs_fixed_array() {
    // D : i32[16]; the arg `D[i]` is a FULL-rank index → a scalar slot.
    // Scalar args render IDENTICALLY in both forms (only the sub-array
    // materialisation differs). The kernel param is the scalar i32.
    let data = DataId(0);
    let kernel = KernelId(0);
    let (names, sidecar) = fixtures(
        data,
        vec![16],
        kernel,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![],
        },
    );
    let ctx = RenderCtx::new(&names, &sidecar);

    // `D[i]` — full-rank index over rank-1 data → a scalar.
    let inputs = vec![ArgBinding::Data(DataSlice {
        data,
        indices: vec![IrExpr::Ident("i".to_string())],
    })];

    let vec_form = render_fire_args(kernel, &inputs, &ctx).expect("vec form renders");
    let nostd_form = render_fire_args_nostd(kernel, &inputs, &ctx).expect("nostd form renders");

    assert_eq!(
        vec_form, "D[(i) as usize]",
        "a full-rank scalar arg renders as `D[(i) as usize]`"
    );
    assert_eq!(
        vec_form, nostd_form,
        "scalar (full-rank) args MUST render identically in both forms — only \
         sub-array materialisation differs"
    );
}

#[test]
fn nostd_sub_array_length_mismatch_is_contract_gap() {
    // TASK-0049.07: data `D : i32[4][16]` indexed `D[frame]` yields a
    // contiguous 16-element sub-array (`sub_len = 16`), but the kernel
    // param is typed `[i32; 8]` (`dims = [8]`, so `N = 8`). The two
    // sidecar tables disagree on the array length. The historical
    // emission `<slice>.try_into::<[i32; 8]>().unwrap()` of a 16-length
    // slice would return `Err` and PANIC on-device at runtime; this is
    // now caught at emit time as a typed `EmitError::ContractGap`.
    let data = DataId(0);
    let kernel = KernelId(0);
    let (names, sidecar) = fixtures(
        data,
        vec![4, 16], // data trailing-dim product = 16
        kernel,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![8], // kernel-sig array length = 8 (mismatch!)
        },
    );
    let ctx = RenderCtx::new(&names, &sidecar);

    let inputs = vec![ArgBinding::Data(DataSlice {
        data,
        indices: vec![IrExpr::Ident("frame".to_string())],
    })];

    // The no_std form must FAIL LOUD (typed error), not emit a latent
    // runtime-panicking `.try_into().unwrap()`.
    let err = render_fire_args_nostd(kernel, &inputs, &ctx)
        .expect_err("a sub_len/N mismatch must be a typed EmitError, not a latent panic");
    match err {
        EmitError::ContractGap(msg) => {
            // The lengths + data name are the load-bearing diagnostic.
            // Assert the specific phrasings ("length 16" / "length 8")
            // rather than a bare `contains('8')` — a stray digit
            // elsewhere in the message must not be able to satisfy this
            // (TASK-0049.07 architect P3.3).
            assert!(
                msg.contains("length 16") && msg.contains("length 8"),
                "message must name both lengths (sub_len 16, N 8): {msg}"
            );
            assert!(
                msg.contains("'D'") && msg.contains("'k'"),
                "message must name the data arg and kernel: {msg}"
            );
            assert!(
                msg.contains("TASK-0049.07"),
                "message must be greppable to the tracker id: {msg}"
            );
        }
        other => panic!("expected EmitError::ContractGap, got {other:?}"),
    }

    // The tier-1 std (`SubArrayForm::Vec`) path is UNTOUCHED: it still
    // emits `.to_vec()` and lets the compiler catch the mismatch at
    // build time as E0308 — no emit-time check, by design.
    let vec_form = render_fire_args(kernel, &inputs, &ctx)
        .expect("tier-1 vec form still renders (mismatch caught later by rustc E0308)");
    assert!(
        vec_form.contains(".to_vec()"),
        "tier-1 path must still emit `.to_vec()`: {vec_form}"
    );
}

#[test]
fn nostd_sub_array_length_match_still_renders() {
    // TASK-0049.07 positive case: data `D : i32[4][16]` indexed
    // `D[frame]` yields `sub_len = 16`; the kernel param is `[i32; 16]`
    // (`dims = [16]`, `N = 16`). The lengths AGREE, so the emit-time
    // check is a no-op and the rendered `.try_into().unwrap()` is
    // identical to the pre-TASK-0049.07 emission — proving the check
    // bites ONLY on a mismatch.
    let data = DataId(0);
    let kernel = KernelId(0);
    let (names, sidecar) = fixtures(
        data,
        vec![4, 16],
        kernel,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16], // matches the 16-length sub-array
        },
    );
    let ctx = RenderCtx::new(&names, &sidecar);

    let inputs = vec![ArgBinding::Data(DataSlice {
        data,
        indices: vec![IrExpr::Ident("frame".to_string())],
    })];

    let nostd_form = render_fire_args_nostd(kernel, &inputs, &ctx)
        .expect("matching lengths must still render the fixed-array form");
    assert_eq!(
        nostd_form,
        "D[((frame) * 16) as usize..((frame) * 16) as usize + 16usize].try_into().unwrap()",
        "a shape-matched no_std sub-array arg renders the unchanged `.try_into().unwrap()`"
    );
}
