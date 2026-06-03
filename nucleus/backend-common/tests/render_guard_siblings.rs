//! Render-layer fail-loud coverage for the FIVE defense-in-depth
//! SIBLINGS of the gather-index-load guards pinned by TASK-0374
//! (`render_gather_negative.rs`). TASK-0379, rigour follow-up to the
//! native-gather landing TASK-0341.03.01 (architect P3.1).
//!
//! WHY THIS EXISTS: TASK-0374 unit-pinned the four fail-loud arms of
//! `render_gather_index_load`. Other render entry points carry
//! fail-loud guards that had no unit test:
//!   - `render_int_expr`'s `IrExpr::Call` arm — was a fail-loud guard;
//!     TASK-0430 (X1') turned it into an EMITTING arm (a PURE kernel
//!     call in index position now renders `kernels::<callee>(<args>)`),
//!     so its test is now a POSITIVE render pin, not a fail-loud one;
//!   - `render_const_expr` — the `IrExpr::DataRef | Call` loop-bound
//!     arm (loop-bound position stays fail-loud — a computed bound is
//!     unimplemented);
//!   - `render_flat_index` — its OWN three guards (empty,
//!     missing-ResolvedType, rank mismatch).
//!
//! REACHABILITY — be honest about it. The remaining guards are
//! DEFENSE-IN-DEPTH and, UNLIKE TASK-0374's partial-rank arm (which IS
//! source-reachable — `x[col[i]]` with a 2D `col` lowers fine), they
//! are mostly NOT reachable from valid source today:
//!   - a kernel call in LOOP-BOUND position is rejected at lowering
//!     (`allow_gather = false` for bounds); a PURE kernel call in
//!     SUBSCRIPT position is now ADMITTED (TASK-0430) and reaches the
//!     emitting `render_int_expr` Call arm from real source;
//!   - a `DataRef`/`Call` in loop-bound position is rejected at
//!     lowering (`allow_gather = false` for bounds), so
//!     `render_const_expr`'s arm is unreachable from real source;
//!   - `render_flat_index`'s three guards sit BEHIND shape-valid
//!     callers (every production caller passes a non-empty, shape-
//!     resolved, rank-matched slice), so they fire only on a
//!     contract-violating IR a future refactor might construct.
//!
//! Their VALUE is therefore not "catch a user error" but "pin the
//! guard BEHAVIOUR so a future refactor cannot silently delete a
//! guard and re-introduce a wrong-offset / latent-panic emission".
//! Each test names the exact `file:line` guard it pins.
//!
//! TASK-0381 (completeness follow-up, architect P3 sibling sweep)
//! extends the inventory to the FOUR remaining fail-loud guards in
//! `fire.rs`, reached through the public arg/output renderers:
//!   - `render_fire_arg`'s `ArgBinding::Nested` arm (fire.rs:376) — a
//!     nested kernel call in argument position; reached via the public
//!     `render_fire_args`. Unlike the gather siblings above, this guard
//!     IS source-reachable (lowering builds `ArgBinding::Nested`
//!     faithfully; fire.rs:376 is the sole rejection site) — see the
//!     test comment;
//!   - `classify_data_slice`'s missing-`ResolvedType` ContractGap
//!     (fire.rs:427) and over-indexed UnsupportedFeature (fire.rs:438),
//!     reached via the public `render_fire_output_assign`.
//!
//! The fourth guard, `classify_data_slice`'s scalar-data-indexed arm
//! (fire.rs:446), is STRUCTURALLY MASKED: the over-indexed check at
//! fire.rs:435 (`indices.len() > dims.len()`) fires FIRST for any
//! `dims == []` with `>= 1` index, and `indices.len() == 0` is
//! forbidden by the `debug_assert!` at fire.rs:421 + the caller
//! contract. It therefore has no direct unit test; instead a MASKING
//! test pins the observable routing (scalar data + 1 index ->
//! over-indexed, NOT scalar-data), so a future re-ordering of the two
//! checks would be caught here. (fire.rs:346 no_std fixed-array
//! mismatch is already covered by `fire_args_nostd.rs:200` — not a
//! gap.)
//!
//! Fixture style mirrors `render_gather_negative.rs` /
//! `fire_args_nostd.rs`: build a minimal `(NameTables, NameSidecar)`
//! and opt IN to exactly the table entries each arm's reachability
//! ordering requires (see the `render_flat_index` tests — the
//! ordering of the empty / data_name / ResolvedType / rank checks is
//! load-bearing).

use nucleus_compiler::algo::{IndexedRef, IrExpr, Purity, ResolvedType, ScalarType};
use nucleus_compiler::event::{ArgBinding, DataId, DataSlice, KernelId};
use nucleus_compiler::name_tables::NameTables;
use nucleus_compiler::sidecar::{KernelSig, NameSidecar};

use backend_common::render::{
    render_const_expr, render_fire_args, render_fire_output_assign, render_flat_index,
    render_int_expr, EmitError, RenderCtx,
};

/// A `(NameTables, NameSidecar)` carrying ZERO data symbols. Each test
/// then opts IN to exactly the entries its arm's reachability ordering
/// requires, so a missing name / type is a deliberate fixture choice.
fn empty_fixtures() -> (NameTables, NameSidecar) {
    (NameTables::default(), NameSidecar::default())
}

/// A `(NameTables, NameSidecar)` where data `did` is NAMED `name` and
/// has a resolved shape `dims` (scalar `i32`).
fn fixtures_with_data(did: DataId, name: &str, dims: Vec<usize>) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.data.insert(did, name.to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.data_types.insert(
        did,
        ResolvedType {
            scalar: ScalarType::I32,
            dims,
        },
    );
    (names, sidecar)
}

/// A `(NameTables, NameSidecar)` where kernel `kid` is NAMED `name`
/// and has the given positional SCALAR param types (TASK-0431). Used to
/// pin the index-position arg cast: a scalar param drives the
/// `(arg) as <ty>` cast in `render_int_expr`'s `Call` arm exactly as it
/// does in `render_fire_arg`.
fn fixtures_with_kernel_sig(
    kid: KernelId,
    name: &str,
    params: Vec<ScalarType>,
) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.kernel.insert(kid, name.to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.kernel_sigs.insert(
        kid,
        KernelSig {
            params: params
                .into_iter()
                .map(|s| ResolvedType {
                    scalar: s,
                    dims: vec![],
                })
                .collect(),
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
            purity: Purity::Pure,
        },
    );
    (names, sidecar)
}

/// Convenience: an `i32`-valued ident index expression.
fn ident(n: &str) -> IrExpr {
    IrExpr::Ident(n.to_string())
}

// --------------------------------------------------------------------
// render_int_expr — the `IrExpr::Call` arm (TASK-0430, X1')
// --------------------------------------------------------------------

#[test]
fn int_expr_pure_kernel_call_in_index_emits_call_no_sig_degrades_bare() {
    // TASK-0430 (X1'). A PURE kernel call `f(k)` in array-subscript index
    // position is now EMITTED as `kernels::f(k)` (was rejected fail-loud
    // pre-TASK-0430). The lowering pass is the gate (subscript-only,
    // pure-callee-only); render just emits.
    //
    // TASK-0431 DEGRADATION PIN: with EMPTY fixtures the callee name `f`
    // is absent from `names.kernel`, so the sig lookup yields `None` and
    // the per-param cast is SKIPPED — the bare arg `k` is emitted, NOT
    // `(k) as <ty>`. This mirrors `render_fire_arg`'s fallback (it casts
    // only when `param_ty` is `Some` and scalar) and is panic-free on a
    // missing-sig contract regression.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "f".to_string(),
        args: vec![ident("k")],
    };

    let rendered = render_int_expr(&expr, &ctx)
        .expect("a pure kernel call in index position must render (TASK-0430)");
    assert_eq!(
        rendered, "kernels::f(k)",
        "a Call-in-index with NO resolvable sig must emit the bare arg \
         (`kernels::<callee>(<args>)`, no cast); the surrounding subscript \
         applies its own `as usize` cast"
    );
}

#[test]
fn int_expr_call_in_index_recurses_args_no_sig_degrades_bare() {
    // A two-arg call with a binop arg pins that the arg list recurses
    // through `render_int_expr` (each arg rendered, joined with `, `).
    // Empty fixtures => no sig => bare args (TASK-0431 degradation).
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "g".to_string(),
        args: vec![
            ident("k"),
            IrExpr::BinOp(
                nucleus_compiler::algo::IrBinOp::Add,
                Box::new(ident("j")),
                Box::new(IrExpr::IntLit(1)),
            ),
        ],
    };

    let rendered = render_int_expr(&expr, &ctx).expect("two-arg call in index must render");
    assert_eq!(rendered, "kernels::g(k, (j + 1))");
}

#[test]
fn int_expr_call_in_index_casts_iter_var_arg_to_i32_param() {
    // TASK-0431 — the FIX. A bare iter-var arg `i` (rendered `i64` in the
    // generated host source) passed to an `i32`-param PURE index kernel
    // `shift(i)` would hit E0308 at build of the generated crate without
    // a cast. `render_int_expr`'s `Call` arm now resolves the callee's
    // `KernelId` (inverting `names.kernel` by name), reads the sidecar
    // `KernelSig`, and casts each scalar arg `(arg) as <param_ty>` —
    // mirroring `render_fire_arg`'s scalar path EXACTLY.
    let kid = KernelId(7);
    let (names, sidecar) = fixtures_with_kernel_sig(kid, "shift", vec![ScalarType::I32]);
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "shift".to_string(),
        args: vec![ident("i")],
    };

    let rendered = render_int_expr(&expr, &ctx)
        .expect("a pure index kernel call with a resolvable i32-param sig must render");
    assert_eq!(
        rendered, "kernels::shift((i) as i32)",
        "an iter-var (i64) arg to an i32-param index kernel must be cast `(i) as i32` \
         so the generated crate typechecks (TASK-0431)"
    );
}

#[test]
fn int_expr_call_in_index_cast_is_noop_shape_for_i32_gather_arg() {
    // TASK-0431 — the cast is a SEMANTIC no-op for the SHIPPED cells.
    // `bucket(input[i])`'s arg lowers to a gather DataRef (`input[i]`,
    // already `i32`). Mirroring `render_fire_arg`, the cast is ALWAYS
    // applied when the param is scalar (it does not try to detect an
    // already-i32 arg) — so the emission is `(input[...]) as i32`, an
    // inert no-op. This pins that the redundant cast is intentional and
    // matches the Fire-arg rule (relevant to the clippy `unnecessary_cast`
    // discussion: the cast lives in GENERATED source, compiled by rustc
    // in the e2e harness, not clippy-gated as part of `just clippy`).
    let kid = KernelId(3);
    // input is a rank-1 i32 array so `input[i]` is a full-rank scalar load.
    let did = DataId(0);
    let (mut names, mut sidecar) = fixtures_with_data(did, "input", vec![8]);
    names.kernel.insert(kid, "bucket".to_string());
    sidecar.kernel_sigs.insert(
        kid,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
            purity: Purity::Pure,
        },
    );
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "bucket".to_string(),
        args: vec![IrExpr::DataRef(IndexedRef {
            name: "input".to_string(),
            indices: vec![ident("i")],
        })],
    };

    let rendered = render_int_expr(&expr, &ctx)
        .expect("bucket(input[i]) with a resolvable i32-param sig must render");
    assert_eq!(
        rendered, "kernels::bucket((input[(i) as usize]) as i32)",
        "an i32 gather-load arg to an i32-param index kernel is cast `(...) as i32` — \
         a semantic no-op, matching render_fire_arg's always-cast-scalar-param rule"
    );
}

#[test]
fn int_expr_call_in_index_skips_cast_for_nonscalar_param() {
    // TASK-0431 DEGRADATION: a NON-scalar param type (an aggregate
    // `i32[4]`) is not a cast target — the arg is emitted BARE, no
    // `as <ty>`. Mirrors `render_fire_arg` (which only casts when the
    // param `is_scalar()`); panic-free on an aggregate-param sig.
    let kid = KernelId(5);
    let mut names = NameTables::default();
    names.kernel.insert(kid, "agg".to_string());
    let mut sidecar = NameSidecar::default();
    sidecar.kernel_sigs.insert(
        kid,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![4], // aggregate, NOT scalar
            }],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
            purity: Purity::Pure,
        },
    );
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "agg".to_string(),
        args: vec![ident("k")],
    };

    let rendered = render_int_expr(&expr, &ctx).expect("a non-scalar-param call must still render");
    assert_eq!(
        rendered, "kernels::agg(k)",
        "a non-scalar (aggregate) param must NOT receive a scalar `as <ty>` cast"
    );
}

// --------------------------------------------------------------------
// render_const_expr — the `DataRef | Call` loop-bound arm
// (expr.rs:201-203)
// --------------------------------------------------------------------

#[test]
fn const_expr_data_ref_in_loop_bound_is_unsupported_feature() {
    // expr.rs:201-203. A data read `n[k]` used as a loop BOUND is
    // rejected fail-loud (a bound must be a static const expression).
    // NOT source-reachable: loop bounds lower with `allow_gather =
    // false`, so a DataRef never reaches this renderer from real
    // source. Tables irrelevant — the arm matches on the IR shape.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::DataRef(IndexedRef {
        name: "n".to_string(),
        indices: vec![ident("k")],
    });

    let err = render_const_expr(&expr, &ctx)
        .expect_err("a data-ref in loop-bound position must fail loud (expr.rs:201-203)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("const expression") && msg.contains("loop bound"),
                "expr.rs:201-203 message must name a const/loop-bound context: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a DataRef-in-loop-bound, got {other:?}"),
    }
}

#[test]
fn const_expr_kernel_call_in_loop_bound_is_unsupported_feature() {
    // expr.rs:201-203, the `Call` half of the SAME arm (bonus). A
    // kernel call as a loop bound hits the identical fail-loud path as
    // the DataRef case — pinned separately so a future refactor that
    // splits the combined `DataRef | Call` pattern cannot drop one half
    // silently.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let expr = IrExpr::Call {
        callee: "f".to_string(),
        args: vec![ident("k")],
    };

    let err = render_const_expr(&expr, &ctx)
        .expect_err("a kernel call in loop-bound position must fail loud (expr.rs:201-203)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("const expression") && msg.contains("loop bound"),
                "expr.rs:201-203 message must name a const/loop-bound context: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a Call-in-loop-bound, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// render_flat_index — its own three guards (fire.rs:519/529/538)
// --------------------------------------------------------------------

#[test]
fn flat_index_empty_indices_is_unsupported_feature() {
    // fire.rs:519-522. A non-indexed (whole-array) slice has no flat
    // offset. This is the FIRST check, before any table lookup, so
    // empty fixtures hit it.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: DataId(0),
        indices: vec![], // non-indexed reference
    };

    let err = render_flat_index(&slice, &ctx).expect_err(
        "render_flat_index on a non-indexed reference must fail loud (fire.rs:519-522)",
    );
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("non-indexed reference"),
                "fire.rs:519-522 message must name the non-indexed reference: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for an empty-indices slice, got {other:?}"),
    }
}

#[test]
fn flat_index_missing_resolved_type_is_contract_gap() {
    // fire.rs:529-536. REACHABILITY ORDERING (load-bearing):
    //   * indices.len() == 1 returns Ok EARLY at fire.rs:524-526 — so
    //     we MUST use >= 2 indices to get past it;
    //   * fire.rs:528 calls `data_name`, which itself ContractGaps
    //     ("has no name in NameTables") FIRST if the name is absent —
    //     so the data NAME must be PRESENT;
    //   * the ResolvedType must be ABSENT to hit THIS guard.
    // So: name present in NameTables.data, NO entry in
    // sidecar.data_types.
    let did = DataId(0);
    let mut names = NameTables::default();
    names.data.insert(did, "grid".to_string());
    let sidecar = NameSidecar::default(); // deliberately NO data_types entry
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("y"), ident("x")], // rank 2 → past the len==1 early return
    };

    let err = render_flat_index(&slice, &ctx).expect_err(
        "a rank>=2 slice over data with no ResolvedType must fail loud (fire.rs:529-536)",
    );
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("no ResolvedType") && msg.contains("grid"),
                "fire.rs:529-536 message must name the missing ResolvedType for `grid`: {msg}"
            );
        }
        other => panic!("expected ContractGap for a missing-ResolvedType slice, got {other:?}"),
    }
}

#[test]
fn flat_index_rank_shape_mismatch_is_unsupported_feature() {
    // fire.rs:538-544. The name AND ResolvedType are present (so we
    // pass :528 and :529), the index list has >= 2 entries (so we pass
    // the :524 early return), but the index COUNT disagrees with the
    // declared rank: 2 indices over a rank-3 datum (`i32[3][3][3]`).
    // That is a sub-array, not a scalar slot, so it is rejected rather
    // than silently emitting a wrong row-major offset.
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "grid", vec![3, 3, 3]); // rank 3
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("y"), ident("x")], // rank 2 index over rank-3 data
    };

    let err = render_flat_index(&slice, &ctx).expect_err(
        "a rank/shape mismatch (2 indices over rank-3 data) must fail loud (fire.rs:538-544)",
    );
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("rank/shape mismatch") && msg.contains("grid"),
                "fire.rs:538-544 message must name the rank/shape mismatch for `grid`: {msg}"
            );
            // The diagnostic carries the actual dims + index count so a
            // future shape change is attributable.
            assert!(
                msg.contains("[3, 3, 3]") && msg.contains("indices=2"),
                "fire.rs:538-544 message must name the dims and index count: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a rank/shape mismatch, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// Positive controls — the guards must NOT reject valid shapes
// --------------------------------------------------------------------

#[test]
fn flat_index_full_rank_renders_row_major_positive_control() {
    // POSITIVE CONTROL for the rank>=2 path: `grid[y][x]` over an
    // `i32[3][4]` datum is FULL-RANK, so the three guards pass and the
    // shared flattener emits the row-major stride form `(y*4 + x) as
    // usize` (D1 = 4 for the first axis, stride 1 for the last). The
    // exact string is snapshot-pinned so a future flattener change is
    // caught here, not silently in e2e.
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "grid", vec![3, 4]);
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("y"), ident("x")],
    };

    let rendered =
        render_flat_index(&slice, &ctx).expect("a full-rank rank-2 slice must render Ok");
    assert_eq!(
        rendered, "((y) * 4 + (x)) as usize",
        "full-rank rank-2 `grid[y][x]` over i32[3][4] must render the row-major flat offset"
    );
}

#[test]
fn flat_index_missing_name_is_contract_gap() {
    // BONUS: the `data_name` guard (fire.rs:31, hit via :528). A rank-2
    // slice whose DataId is absent from NameTables.data fails loud at
    // the name lookup BEFORE the ResolvedType check — pinning the
    // ordering nuance documented on the missing-ResolvedType test.
    let (names, sidecar) = empty_fixtures(); // no name for DataId(0)
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: DataId(0),
        indices: vec![ident("y"), ident("x")], // rank 2 → reaches data_name at :528
    };

    let err = render_flat_index(&slice, &ctx)
        .expect_err("a rank>=2 slice whose DataId has no name must fail loud (fire.rs:31)");
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("has no name in NameTables"),
                "fire.rs:31 message must name the missing NameTables entry: {msg}"
            );
        }
        other => panic!("expected ContractGap for a missing-name slice, got {other:?}"),
    }
}

// ====================================================================
// TASK-0381 — the FOUR remaining fire.rs fail-loud guards, reached
// through the public arg/output renderers.
// ====================================================================

// --------------------------------------------------------------------
// render_fire_arg — the `ArgBinding::Nested` arm (fire.rs:376), via the
// public `render_fire_args`.
// --------------------------------------------------------------------

#[test]
fn fire_arg_nested_kernel_call_is_unsupported_feature() {
    // fire.rs:376. A nested kernel call in ARGUMENT position
    // (`f(g(k))`) is rejected fail-loud — the backend renders flat
    // argument lists, not nested call expressions. Direct sibling of
    // the Call-in-index guard (expr.rs:72), the most glaring omission.
    // UNLIKE the other guards in this file, this one IS source-
    // reachable: the lowering layer FAITHFULLY lowers a nested call to
    // `ArgBinding::Nested` (acfg/build.rs:276) — it does NOT flatten or
    // reject — so fire.rs:376 is the SOLE rejection site and produces a
    // user-facing error for grammar-admitted source. Example 14 hand-
    // splits `denoise(mix2(a, b))` into two statements specifically to
    // avoid tripping it (examples/14-hearing-aid/prog.algo.nuc). The arm
    // matches on the binding shape alone and ignores the (absent) kernel
    // signature, so empty fixtures suffice.
    let (names, sidecar) = empty_fixtures();
    let ctx = RenderCtx::new(&names, &sidecar);

    let nested = ArgBinding::Nested {
        callee: "g".to_string(),
        args: vec![ArgBinding::Scalar(ident("k"))],
    };

    let err = render_fire_args(KernelId(0), std::slice::from_ref(&nested), &ctx)
        .expect_err("a nested kernel call in argument position must fail loud (fire.rs:376)");
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("nested kernel call") && msg.contains("argument"),
                "fire.rs:376 message must name a nested kernel call in an argument: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for a Nested arg binding, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// classify_data_slice — its missing-ResolvedType + over-indexed guards
// (fire.rs:427 / :438), via the public `render_fire_output_assign`.
// --------------------------------------------------------------------

#[test]
fn fire_output_missing_resolved_type_is_contract_gap() {
    // fire.rs:427. `render_fire_output_assign` resolves the data name
    // (fire.rs:71) then classifies the slice; `classify_data_slice`
    // fails loud when the named datum has no `ResolvedType` in the
    // sidecar. REACHABILITY ORDERING: the NAME must be PRESENT (else
    // both fire.rs:71 and classify's own `data_name` at fire.rs:425
    // ContractGap "has no name" FIRST); the ResolvedType must be ABSENT
    // to hit THIS guard. Sibling of the flat-index missing-ResolvedType
    // guard (fire.rs:529).
    let did = DataId(0);
    let mut names = NameTables::default();
    names.data.insert(did, "out".to_string());
    let sidecar = NameSidecar::default(); // deliberately NO data_types entry
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("i")],
    };

    let err = render_fire_output_assign(&slice, "kernels::f(i)", &ctx).expect_err(
        "an output slice over data with no ResolvedType must fail loud (fire.rs:427)",
    );
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("no ResolvedType") && msg.contains("out"),
                "fire.rs:427 message must name the missing ResolvedType for `out`: {msg}"
            );
        }
        other => panic!("expected ContractGap for a missing-ResolvedType output, got {other:?}"),
    }
}

#[test]
fn fire_output_over_indexed_is_unsupported_feature() {
    // fire.rs:438. The name AND ResolvedType are present, but the index
    // COUNT exceeds the declared rank: 2 indices over a rank-1 datum
    // (`i32[4]`). That is a contract bug the contract pass should have
    // rejected; the backend fails loud with context rather than
    // emitting a wrong offset. Sibling of the flat-index rank-mismatch
    // guard (fire.rs:539).
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "out", vec![4]); // rank 1
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("y"), ident("x")], // 2 indices over rank-1 data
    };

    let err = render_fire_output_assign(&slice, "kernels::f(y, x)", &ctx).expect_err(
        "an over-indexed output slice (2 indices over rank-1 data) must fail loud (fire.rs:438)",
    );
    match err {
        EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("over-indexed") && msg.contains("out"),
                "fire.rs:438 message must name the over-indexing of `out`: {msg}"
            );
            assert!(
                msg.contains("dims=[4]") && msg.contains("indices=2"),
                "fire.rs:438 message must name the dims and index count: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature for an over-indexed output, got {other:?}"),
    }
}

#[test]
fn fire_output_scalar_data_indexed_is_masked_by_over_indexed_guard() {
    // fire.rs:446 is STRUCTURALLY MASKED — this test pins WHY, so a
    // future re-ordering of the two checks is caught here rather than
    // silently changing which diagnostic fires.
    //
    // The dedicated scalar-data-indexed guard (`dims.is_empty()`,
    // fire.rs:444-449) can only fire when control reaches it, i.e.
    // AFTER the over-indexed check at fire.rs:435 passes
    // (`indices.len() <= dims.len()`). With `dims == []` that requires
    // `indices.len() == 0` — but `classify_data_slice` `debug_assert!`s
    // a non-empty index list (fire.rs:421) and its callers never pass
    // an empty one. So for ANY reachable input (>= 1 index over scalar
    // data), the over-indexed check at fire.rs:435 fires FIRST
    // (`1 > 0`), and the scalar-data arm is dead. We therefore assert
    // the OBSERVABLE behaviour: scalar data (`dims == []`) indexed once
    // routes to the OVER-INDEXED diagnostic, NOT the scalar-data one.
    let did = DataId(0);
    let (names, sidecar) = fixtures_with_data(did, "s", vec![]); // rank 0 (scalar)
    let ctx = RenderCtx::new(&names, &sidecar);

    let slice = DataSlice {
        data: did,
        indices: vec![ident("i")], // 1 index over scalar data
    };

    let err = render_fire_output_assign(&slice, "kernels::f(i)", &ctx).expect_err(
        "indexing scalar data must fail loud — via the over-indexed guard (fire.rs:435), \
         which masks the dedicated scalar-data guard (fire.rs:446)",
    );
    match err {
        EmitError::UnsupportedFeature(msg) => {
            // The over-indexed guard wins; its message (not the
            // scalar-data "indexed with N expressions" message) proves
            // the masking ordering.
            assert!(
                msg.contains("over-indexed") && msg.contains("dims=[]"),
                "fire.rs:435 (masking fire.rs:446) must report over-indexing of scalar data: {msg}"
            );
            assert!(
                !msg.contains("scalar data `s` indexed"),
                "the dedicated scalar-data message (fire.rs:447) must NOT fire — it is masked: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature (over-indexed) for indexed scalar data, got {other:?}"),
    }
}
