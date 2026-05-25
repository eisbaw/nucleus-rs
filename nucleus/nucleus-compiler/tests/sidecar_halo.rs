//! Tests for `NameSidecar::halo_widths` (TASK-0260, Stage 1).
//!
//! Pins both halves of the invariant:
//!
//! 1. A 3x3 stencil algorithm (example 05) produces non-trivial halo
//!    entries for both axes after `apply_halo_inference`. The pipeline
//!    must populate the ACFG sidecar AND the NameSidecar (the codegen
//!    contract surface) so the Stage 2 consumer (TASK-0263,
//!    transfer_inject extension) has a path to read it.
//!
//! 2. A non-stencil algorithm (example 01 / naive) produces an EMPTY
//!    halo_widths map — Stage 1 records nothing where there's nothing
//!    to record. This pins the additive-only contract: existing
//!    examples remain byte-identical under codegen because no consumer
//!    has wired through yet (the field is observationally inert).
//!
//! 3. The serde round-trip preserves `halo_widths` AND an older
//!    payload (synthesised by dropping the field) deserialises with
//!    `halo_widths` defaulting to an empty map.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use nucleus_compiler::{
    algo::{lower_algo, parse_algo},
    apply_block_transforms, apply_halo_inference, apply_halo_inference_partition_aware,
    apply_partition_blocks2d, apply_partition_rows, apply_partition_workers, build_acfg,
    build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
    HaloInferenceError,
};

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("two ancestors above compiler crate")
}

/// Run the full lower-link-inject pipeline for a given example/schedule.
/// Mirrors `sidecar_buffer.rs::lower` but includes the three partition
/// passes + `apply_halo_inference` so the sidecar is populated as the
/// driver does it.
fn lower(
    ex_rel: &str,
    sched_rel: &str,
) -> (nucleus_compiler::link::LinkedIR, nucleus_compiler::ACFG) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(ex_rel);
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).expect("read algo");
    let sched_src = fs::read_to_string(ex.join(sched_rel)).expect("read sched");

    let algo_ir = lower_algo(&parse_algo(&algo_src).expect("parse_algo")).expect("lower_algo");
    let sched_ir =
        lower_sched(&parse_sched(&sched_src).expect("parse_sched")).expect("lower_sched");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block_transforms");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");
    let acfg = apply_partition_rows(&linked, acfg).expect("partition_rows");
    let acfg = apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d");
    let acfg = apply_halo_inference(&linked, acfg).expect("halo_inference");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    (linked, acfg)
}

#[test]
fn stencil_3x3_produces_halo_one_on_both_axes() {
    // Example 05 / 3x3 stencil, naive schedule. The algorithm reads
    // img_in[y±1][x±1] (and centre + edges); halo inference must
    // record (blur3, y) -> 1 AND (blur3, x) -> 1.
    let (linked, acfg) = lower("05-stencil", "schedules/naive.sched.nuc");

    let kid = *acfg.name_kernels.get("blur3").expect("blur3 in ACFG");
    let y_iv = *acfg.name_iter_vars.get("y").expect("y in ACFG");
    let x_iv = *acfg.name_iter_vars.get("x").expect("x in ACFG");

    let y_halo = acfg
        .halo_widths
        .get(&kid)
        .and_then(|m| m.get(&y_iv))
        .copied();
    let x_halo = acfg
        .halo_widths
        .get(&kid)
        .and_then(|m| m.get(&x_iv))
        .copied();
    assert_eq!(
        y_halo,
        Some(1),
        "halo_widths[blur3][y] must be 1 (offsets -1, 0, +1)"
    );
    assert_eq!(
        x_halo,
        Some(1),
        "halo_widths[blur3][x] must be 1 (offsets -1, 0, +1)"
    );

    // The codegen-contract surface (NameSidecar) must mirror the ACFG
    // sidecar verbatim.
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert_eq!(
        sidecar.halo_widths, acfg.halo_widths,
        "NameSidecar.halo_widths must mirror ACFG.halo_widths"
    );
    assert_eq!(
        sidecar
            .halo_widths
            .get(&kid)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(1)
    );
    assert_eq!(
        sidecar
            .halo_widths
            .get(&kid)
            .and_then(|m| m.get(&x_iv))
            .copied(),
        Some(1)
    );
}

#[test]
fn elementwise_add_records_only_zero_halos() {
    // Example 01 (elementwise-add): kernel reads `a[i]` and `b[i]` —
    // both bare-iv reads, halo offset 0. The implementation records
    // an explicit 0-width entry per (kernel, iter-var) the detector
    // inspects (see `halo_inference.rs::no_halo_bare_iv` for the
    // contract rationale). The MAX halo width across all entries
    // must therefore be 0 — equivalent to "no halo needed".
    let (linked, acfg) = lower("01-elementwise-add", "schedules/naive.sched.nuc");
    let max_halo = acfg
        .halo_widths
        .values()
        .flat_map(|m| m.values().copied())
        .max()
        .unwrap_or(0);
    assert_eq!(
        max_halo, 0,
        "elementwise-add reads only `a[i]`/`b[i]` (offset 0); max halo width must be 0; got map {:?}",
        acfg.halo_widths
    );

    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert_eq!(
        sidecar.halo_widths, acfg.halo_widths,
        "NameSidecar.halo_widths must mirror ACFG.halo_widths"
    );
}

#[cfg(feature = "serde")]
#[test]
fn halo_widths_serde_roundtrip() {
    // Round-trip the NameSidecar through serde JSON; the halo_widths
    // map must survive byte-for-byte.
    let (linked, acfg) = lower("05-stencil", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert!(
        !sidecar.halo_widths.is_empty(),
        "fixture must produce non-trivial halo for the round-trip test"
    );
    let json = serde_json::to_string(&sidecar).expect("serialise NameSidecar");
    let back: nucleus_compiler::NameSidecar =
        serde_json::from_str(&json).expect("deserialise NameSidecar");
    assert_eq!(
        back.halo_widths, sidecar.halo_widths,
        "halo_widths must survive serde JSON round-trip"
    );
}

#[cfg(feature = "serde")]
#[test]
fn halo_widths_serde_default_on_missing_field() {
    // An "old" wire payload that omits the `halo_widths` field must
    // deserialise to an empty map (TASK-0260 backward-compat contract,
    // mirroring TASK-0233's transfer_buffer_for_seq).
    //
    // We synthesise the "old" payload by round-tripping a real
    // NameSidecar through JSON and then stripping the `halo_widths`
    // key from the JSON object string. The result still has every
    // OTHER field, so non-serde-default fields like `data_types`
    // remain present and the test isolates the additive-compat claim
    // to the new field alone.
    let (linked, acfg) = lower("01-elementwise-add", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let value: serde_json::Value =
        serde_json::to_value(&sidecar).expect("serialise NameSidecar to Value");
    let mut obj = value
        .as_object()
        .expect("NameSidecar serialises to JSON object")
        .clone();
    obj.remove("halo_widths");
    let pruned = serde_json::Value::Object(obj);
    let stripped_json = serde_json::to_string(&pruned).expect("re-serialise");
    let back: nucleus_compiler::NameSidecar =
        serde_json::from_str(&stripped_json).expect("payload without halo_widths must deserialise");
    assert!(
        back.halo_widths.is_empty(),
        "missing halo_widths field must default to empty map"
    );
}

// ----------------------------------------------------------------------
// TASK-0275: partition-policy-aware (B) entry-point pins.
//
// These pins lock in the per-error fatality contract of
// `apply_halo_inference_partition_aware`:
//
// - Non-affine kernel-arg index UNDER a partitioned iv → FATAL.
//   transfer_inject (cycle 83) would silently emit wrong-output tiles
//   without the halo entry it cannot recover; we must fail loud.
//
// - Non-affine kernel-arg index UNDER an UN-partitioned iv (example 11
//   `step_or_seed` Mod-wrap shape) → ADVISORY. transfer_inject does
//   not fire on un-partitioned ivs, so the missing halo entry is
//   harmless. Recording the error in the advisory bucket keeps the
//   diagnostic visible to NUC_TRACE consumers but lets compilation
//   proceed. This is the load-bearing case (B) preserves over the
//   naive (A) strict mirror that would have newly-rejected example 11.
//
// - Fully-affine body under a partitioned iv → CLEAN: Ok((acfg, []))
//   with the recovered halo width committed.
//
// Hand-built LinkedIR mirrors the cycle-88 TASK-0271 reuse template
// (tests/sidecar_reuse.rs::task0271_strict_rejects_non_affine_reuse_body
// + sibling); the same scaffold (data + kernel + workers + places +
// loops) is reused across the three arms with only the kernel-arg
// index expression / the `partition=` option toggling between them.

/// Common scaffold: build a `LinkedIR` for halo (B) tests with one
/// kernel `K`, one data symbol `grid` (1D, length 64), one out symbol
/// `out`, one worker `w0`, one placement of K on w0, and a single
/// `for y in 0..16` loop containing `out[y] = K(grid[<idx_expr>])`.
/// The caller chooses the index expression (affine vs non-affine) and
/// whether to attach a `partition=` option to the `y` loop directive.
fn build_linked_for_partition_test(
    y_idx_expr: nucleus_compiler::algo::IrExpr,
    y_partition: Option<nucleus_compiler::sched::PartitionKind>,
) -> nucleus_compiler::link::LinkedIR {
    use nucleus_compiler::algo::{
        AlgoIR, IndexedRef, IrExpr, IrStmt, Purity, ResolvedData, ResolvedKernel, ResolvedType,
        ScalarType,
    };
    use nucleus_compiler::sched::{
        ResolvedLoopDirective, ResolvedLoopOption, ResolvedPlaceTarget, ResolvedPlacement,
        ResolvedWorker, SchedIR, DEFAULT_WORKER_CLASS,
    };

    let mut data = BTreeMap::new();
    data.insert(
        "grid".to_string(),
        ResolvedData {
            name: "grid".to_string(),
            ty: ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![64],
            },
        },
    );
    data.insert(
        "out".to_string(),
        ResolvedData {
            name: "out".to_string(),
            ty: ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16],
            },
        },
    );
    let mut kernels = BTreeMap::new();
    kernels.insert(
        "K".to_string(),
        ResolvedKernel {
            name: "K".to_string(),
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
            purity: Purity::Pure,
            name_span: None,
        },
    );
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: IrExpr::IntLit(0),
        hi: IrExpr::IntLit(16),
        body: vec![IrStmt::Dataflow {
            lhs: IndexedRef {
                name: "out".to_string(),
                indices: vec![IrExpr::Ident("y".to_string())],
            },
            rhs: IrExpr::Call {
                callee: "K".to_string(),
                args: vec![IrExpr::DataRef(IndexedRef {
                    name: "grid".to_string(),
                    indices: vec![y_idx_expr],
                })],
            },
        }],
    }];
    let algo = AlgoIR {
        consts: BTreeMap::new(),
        data,
        kernels,
        stmts,
    };

    let mut workers: BTreeMap<String, ResolvedWorker> = BTreeMap::new();
    workers.insert(
        "w0".to_string(),
        ResolvedWorker {
            name: "w0".to_string(),
            class: DEFAULT_WORKER_CLASS.to_string(),
        },
    );
    let mut places: BTreeMap<String, ResolvedPlacement> = BTreeMap::new();
    places.insert(
        "K".to_string(),
        ResolvedPlacement {
            kernel: "K".to_string(),
            target: ResolvedPlaceTarget::One("w0".to_string()),
            kernel_span: None,
        },
    );
    let mut loops: BTreeMap<String, ResolvedLoopDirective> = BTreeMap::new();
    if let Some(p) = y_partition {
        loops.insert(
            "y".to_string(),
            ResolvedLoopDirective {
                var: "y".to_string(),
                options: vec![ResolvedLoopOption::Partition(p)],
                var_span: None,
            },
        );
    }
    let sched = SchedIR {
        algo_path: String::new(),
        worker_classes: BTreeMap::new(),
        memory_regions: BTreeMap::new(),
        workers,
        places,
        place_data: BTreeMap::new(),
        loops,
        transfers: BTreeMap::new(),
        checks: BTreeMap::new(),
    };
    link(algo, sched).expect("link must succeed")
}

#[test]
fn task0275_partition_aware_rejects_strided_under_partitioned_iv() {
    // Body: `for y in 0..16 { out[y] = K(grid[y*2]) }` with
    // `loop y : partition=workers;`. The `y*2` index is StridedAccess
    // (coefficient 2, not +1) — strictly *affine* but the first-cut
    // detector rejects coefficient != 1. Under (B), the y-loop carries
    // a `partition=` directive → transfer_inject's halo consumer would
    // fire and need the halo width, which the walker cannot recover.
    // The partition-aware entry point MUST return Err on this case.
    //
    // See `task0275_partition_aware_rejects_non_affine_under_partitioned_iv`
    // below for the matching `NonAffineIndex` (Mod-shape) variant —
    // that pin closes the AC#3 literal "non-affine halo on a
    // partitioned iv → FATAL" wording (example-11 shape).
    use nucleus_compiler::algo::{IrBinOp, IrExpr};
    use nucleus_compiler::sched::PartitionKind;

    let stride_idx = IrExpr::BinOp(
        IrBinOp::Mul,
        Box::new(IrExpr::Ident("y".to_string())),
        Box::new(IrExpr::IntLit(2)),
    );
    let linked = build_linked_for_partition_test(stride_idx, Some(PartitionKind::Workers));
    let acfg = build_acfg(&linked).expect("acfg build");

    let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
        "partition-aware must reject strided kernel-arg index when the enclosing iv is \
         partitioned (transfer_inject would silently emit wrong tiles)",
    );
    match err {
        HaloInferenceError::StridedAccessNotSupported {
            kernel,
            ref_name,
            coefficient,
            ..
        } => {
            assert_eq!(kernel, "K");
            assert_eq!(ref_name, "grid");
            assert_eq!(coefficient, 2);
        }
        other => panic!("expected StridedAccessNotSupported, got {other:?}"),
    }
}

#[test]
fn task0275_partition_aware_rejects_non_affine_under_partitioned_iv() {
    // Body: `for y in 0..16 { out[y] = K(grid[y % 4]) }` with
    // `loop y : partition=workers;`. The `y % 4` index is the genuine
    // `NonAffineIndex` shape — it mentions `y` but `affine_decompose`
    // rejects `Mod` (and `Div`) unconditionally (see `passes::common`
    // contract). This is structurally the example-11
    // (`11-game-of-life`) `step_or_seed` shape that motivated TASK-0275
    // in the first place: `grid[(t + ITERS) % (ITERS + 1)]`. Pins the
    // literal AC#3 "non-affine halo on a partitioned iv → FATAL"
    // wording — sibling `_rejects_strided_` above covers the
    // coefficient!=1 arm.
    use nucleus_compiler::algo::{IrBinOp, IrExpr};
    use nucleus_compiler::sched::PartitionKind;

    let mod_idx = IrExpr::BinOp(
        IrBinOp::Mod,
        Box::new(IrExpr::Ident("y".to_string())),
        Box::new(IrExpr::IntLit(4)),
    );
    let linked = build_linked_for_partition_test(mod_idx, Some(PartitionKind::Workers));
    let acfg = build_acfg(&linked).expect("acfg build");

    let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
        "partition-aware must reject NonAffineIndex (Mod-shape) when the enclosing iv is \
         partitioned (this is the example-11 step_or_seed shape under a hypothetical \
         partition= — transfer_inject would silently emit wrong tiles)",
    );
    match err {
        HaloInferenceError::NonAffineIndex {
            kernel, ref_name, ..
        } => {
            assert_eq!(kernel, "K");
            assert_eq!(ref_name, "grid");
        }
        other => panic!("expected NonAffineIndex, got {other:?}"),
    }
}

#[test]
fn task0275_partition_aware_accepts_non_affine_under_unpartitioned_iv() {
    // Same Mod-shape body as the matching rejection arm above
    // (`grid[y % 4]` — genuine `NonAffineIndex`), but the `y` loop has
    // NO `partition=` directive. This precisely mirrors the example-11
    // (`11-game-of-life`) `step_or_seed` shape: schedule carries zero
    // `partition=` on the iv whose index the affine detector cannot
    // fold (Mod). Under (B), this is ADVISORY: transfer_inject does
    // not fire on un-partitioned ivs, so the missing halo entry is
    // harmless. The partition-aware entry point returns Ok and routes
    // the typed error into the advisory bucket. This is the
    // load-bearing case that distinguishes (B) from a naive (A) strict
    // mirror — the latter would newly-reject example 11's two cells.
    use nucleus_compiler::algo::{IrBinOp, IrExpr};

    let mod_idx = IrExpr::BinOp(
        IrBinOp::Mod,
        Box::new(IrExpr::Ident("y".to_string())),
        Box::new(IrExpr::IntLit(4)),
    );
    let linked = build_linked_for_partition_test(mod_idx, None);
    let acfg = build_acfg(&linked).expect("acfg build");

    let (acfg_out, advisory) = apply_halo_inference_partition_aware(&linked, acfg).expect(
        "partition-aware must NOT reject NonAffineIndex when the enclosing iv is un-partitioned \
         (transfer_inject won't fire — the missing halo is harmless; this is the example-11 \
         contract preserved)",
    );
    assert_eq!(
        advisory.len(),
        1,
        "the single NonAffineIndex error must surface in the advisory bucket; got {advisory:?}"
    );
    match &advisory[0] {
        HaloInferenceError::NonAffineIndex { .. } => {}
        other => panic!("expected advisory NonAffineIndex, got {other:?}"),
    }
    // The walker recovered no widths for this body (the only index was
    // rejected); the committed sidecar therefore has an empty
    // halo_widths map. This pins the partial-commit contract for the
    // (B) entry point: errors that go advisory still preserve whatever
    // widths the walker COULD recover (vacuously empty here).
    assert!(
        acfg_out.halo_widths.values().all(|m| m.is_empty()),
        "no widths recoverable for this body; got {:?}",
        acfg_out.halo_widths
    );
}

#[test]
fn task0275_partition_aware_accepts_clean_affine_under_partitioned_iv() {
    // Body: `for y in 0..16 { out[y] = K(grid[y+1]) }` with
    // `loop y : partition=workers;`. The `y+1` index is fully affine
    // (coefficient +1, offset 1). Under (B), this is CLEAN: no typed
    // error from the walker, the partition-aware entry point returns
    // Ok with an empty advisory vector AND the recovered halo width
    // (1) committed to the sidecar. Pins the no-false-positive contract
    // — the shipped 05-stencil/distributed schedule (partition=workers
    // on its y-loop with fully-affine `blur3` body) is the production
    // analogue.
    use nucleus_compiler::algo::{IrBinOp, IrExpr};
    use nucleus_compiler::sched::PartitionKind;

    let affine_idx = IrExpr::BinOp(
        IrBinOp::Add,
        Box::new(IrExpr::Ident("y".to_string())),
        Box::new(IrExpr::IntLit(1)),
    );
    let linked = build_linked_for_partition_test(affine_idx, Some(PartitionKind::Workers));
    let acfg = build_acfg(&linked).expect("acfg build");

    let (acfg_out, advisory) = apply_halo_inference_partition_aware(&linked, acfg)
        .expect("partition-aware must accept fully-affine body under partitioned iv");
    assert!(
        advisory.is_empty(),
        "no errors expected on a clean affine body; got {advisory:?}"
    );
    let kid = *acfg_out.name_kernels.get("K").expect("K in ACFG");
    let y_iv = *acfg_out.name_iter_vars.get("y").expect("y in ACFG");
    assert_eq!(
        acfg_out
            .halo_widths
            .get(&kid)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(1),
        "recovered halo width for K[y] must be 1 (offset +1)"
    );
}

#[test]
fn task0275_partition_aware_rejects_strided_under_partition_rows() {
    // Same shape as `_rejects_strided_under_partitioned_iv` but with
    // `PartitionKind::Rows` instead of `Workers`. Pins that
    // `iv_is_partitioned` treats every `ResolvedLoopOption::Partition(_)`
    // variant as escalating — a regression that narrowed the match arm
    // to only `Partition(Workers)` would silently let a partition=rows
    // schedule produce wrong tiles on a non-affine body.
    use nucleus_compiler::algo::{IrBinOp, IrExpr};
    use nucleus_compiler::sched::PartitionKind;

    let stride_idx = IrExpr::BinOp(
        IrBinOp::Mul,
        Box::new(IrExpr::Ident("y".to_string())),
        Box::new(IrExpr::IntLit(2)),
    );
    let linked = build_linked_for_partition_test(stride_idx, Some(PartitionKind::Rows));
    let acfg = build_acfg(&linked).expect("acfg build");

    let err = apply_halo_inference_partition_aware(&linked, acfg)
        .expect_err("partition-aware must escalate on any PartitionKind, not just Workers");
    assert!(
        matches!(err, HaloInferenceError::StridedAccessNotSupported { .. }),
        "expected StridedAccessNotSupported under partition=rows, got {err:?}"
    );
}

// ----------------------------------------------------------------------
// TASK-0299: pinning test for 06-separable-filter's distributed schedule.
//
// 06-separable-filter/schedules/distributed.sched.nuc:19-21 carries the
// load-bearing narrative claim that halo_inference produces
// halo_widths[hblur_acc][hy] = 0 and that transfer_inject does NOT
// extend per-tile transfer ranges. The claim is true today by
// inspection of the algorithm: hblur_acc(tmp[hy][hx], in_arr[hy][hk],
// hx, hk) reads hy at offset 0 only (same with vblur_acc on vy).
//
// Without this test, the claim is a comment-doc-lie waiting to happen:
// a future kernel-surface change introducing a non-zero hy offset (e.g.
// `in_arr[hy-1][hk]` for a vertical-blur fold) would silently break the
// claim while the e2e cell catches only the wrong output. This test
// makes the claim a structural invariant — a future kernel-surface
// change with non-zero halo on hy fails LOUD and forces the schedule
// comment to be updated in the same commit. Defends against the
// feedback-comment-doc-lie-recurring pattern.
//
// Scope narrowing: the schedule header is a TWO-PART conjunction —
// "halo_widths[hblur_acc][hy] = 0 AND transfer_inject does NOT extend
// per-tile transfer ranges". This test pins ONLY the first conjunct
// (the halo_inference output). A regression that breaks the second
// conjunct without touching the first (e.g. a future transfer_inject
// that unconditionally extends tile ranges even when halo=0) would not
// trip this test; the e2e bytes would catch it iff the over-extension
// changes the output. Pinning the second conjunct is a separate
// fixture (assert transfer_inject's per-tile ranges, not halo widths)
// and is filed as a follow-up if the project wants narrative-coverage
// parity across both halves of the claim.

#[test]
fn task0299_06_separable_filter_distributed_halo_widths_pinned_to_zero() {
    // Contract degree of freedom: halo_inference's contract permits an
    // explicit 0-width entry OR omission (see the "TASK-0305 cycle-122
    // project decision (Option B)" paragraph in halo_inference.rs —
    // search for `absent ≡ explicit-0` + its `no_halo_bare_iv` test).
    // This pinning test treats both as "halo == 0" — robust to that
    // choice; the ONLY failure mode it pins is "halo > 0".
    //
    // Soundness floor (TASK-0305 cycle-122 decision, Option B): the
    // `== 0` `.unwrap_or(0)` form admits a vacuous pass on a future
    // silent-skip regression in the production walker (no entries
    // emitted for hblur_acc/vblur_acc → unwrap → 0 ≡ 0). TASK-0307
    // cycle-123 LANDED the structural sentinel (search for
    // `fn no_halo_bare_iv` in halo_inference.rs — the in-module test
    // now carries a `copied() == Some(0)` assertion alongside the
    // existing `.unwrap_or(0)` contract-form check). The sentinel
    // closes the vacuous-pass arm at the contract boundary without
    // coupling THIS narrative pin to the explicit-0 representation.
    let (_linked, acfg) = lower("06-separable-filter", "schedules/distributed.sched.nuc");

    let hblur_id = *acfg
        .name_kernels
        .get("hblur_acc")
        .expect("hblur_acc in ACFG");
    let vblur_id = *acfg
        .name_kernels
        .get("vblur_acc")
        .expect("vblur_acc in ACFG");
    let hy_iv = *acfg.name_iter_vars.get("hy").expect("hy in ACFG");
    let vy_iv = *acfg.name_iter_vars.get("vy").expect("vy in ACFG");

    let hblur_hy = acfg
        .halo_widths
        .get(&hblur_id)
        .and_then(|m| m.get(&hy_iv))
        .copied()
        .unwrap_or(0);
    let vblur_vy = acfg
        .halo_widths
        .get(&vblur_id)
        .and_then(|m| m.get(&vy_iv))
        .copied()
        .unwrap_or(0);

    assert_eq!(
        hblur_hy, 0,
        "halo_widths[hblur_acc][hy] must be 0; the schedule header at \
         nuc-nucleus/examples/06-separable-filter/schedules/distributed.sched.nuc:19-21 \
         depends on this. acfg.halo_widths = {:?}",
        acfg.halo_widths
    );
    assert_eq!(
        vblur_vy, 0,
        "halo_widths[vblur_acc][vy] must be 0 (mirror property on the \
         vertical pass; though pass 2 stays on host today per the \
         schedule's HONEST SCOPE note, the algorithm-level claim is \
         symmetric to hblur_acc[hy]). acfg.halo_widths = {:?}",
        acfg.halo_widths
    );

    // Defensive: max halo across the WHOLE algorithm must be 0. Catches
    // a future regression that introduces a non-zero halo on ANY
    // (kernel, iv) pair, even one the named lookups above don't cover.
    let max_halo = acfg
        .halo_widths
        .values()
        .flat_map(|m| m.values().copied())
        .max()
        .unwrap_or(0);
    assert_eq!(
        max_halo, 0,
        "06-separable-filter is a rectangular-accumulator separable filter; \
         no kernel argument reads at non-zero iv offset. max halo width \
         must be 0; got map {:?}",
        acfg.halo_widths
    );
}

// ----------------------------------------------------------------------
// TASK-0303: sibling-sweep follow-up to TASK-0299 cycle 119. Pins two
// more load-bearing halo-narrative claims that the cycle-119 architect
// review identified as structurally identical to TASK-0299's narrative
// but not yet covered by a structural test. Both use the same idioms
// (real-file load via the `lower()` helper, .unwrap_or(0) per the
// halo_inference contract degree of freedom).
//
// Defends against feedback-silent-sibling-defect — pinning the narrative
// at ONE site (TASK-0299) while leaving structurally-identical
// narratives unpinned at sibling sites is the named pattern this task
// closes.

#[test]
fn task0303_05_stencil_distributed_2d_halo_widths_pinned_to_one() {
    // 05-stencil/schedules/distributed-2d.sched.nuc:53 carries the
    // load-bearing narrative claim `halo_y = halo_x = 1 (inferred from
    // blur3's 3x3 access)`. This is the precondition for the halo-strip
    // Push/Wait synthesis pass (TASK-0289 cycle 114a) that the same
    // header attributes to its own design rationale at lines 19-31 —
    // halo=1 is what determines that exactly one row/column of cross-
    // worker halo gets synthesised per (neighbour, axis).
    //
    // Halo inference is partition-independent (it walks AlgoIR, not
    // SchedIR), so an existing test `stencil_3x3_produces_halo_one_on_both_axes`
    // at lines 70-125 covers the 05-stencil naive schedule and would
    // also catch an algorithm-level regression. This test is the
    // NARRATIVE TIE — a future failure here names `distributed-2d`
    // specifically so the failure message points the reader at the
    // schedule header whose narrative just broke, not just at the
    // algorithm.
    //
    // The claim's exact text is at distributed-2d.sched.nuc:53. If a
    // future kernel-surface change toggles blur3 to a 5x5 access
    // pattern (or a 1x3 / 3x1), the schedule comment becomes a
    // comment-doc-lie and this test fails LOUD, forcing the comment to
    // be updated in the same commit. Defends against
    // feedback-comment-doc-lie-recurring on this sibling narrative.
    //
    // Contract degree of freedom (TASK-0305 cycle-122 decision, Option
    // B): halo_inference's contract permits an explicit entry OR
    // omission (see the "TASK-0305 cycle-122 project decision
    // (Option B)" paragraph in halo_inference.rs — search for
    // `absent ≡ explicit-0`). This test uses `.unwrap_or(0)` and
    // asserts `blur3_y == 1`. The assert is
    // robust UNDER EITHER contract form — a value of 1 must be
    // explicitly present. Soundness floor: a regression that silently
    // produced NO entries for blur3 would surface as `unwrap_or(0) →
    // 0 ≠ 1` and fail LOUD. So this >0 pin is
    // contract-form-independent BY CONSTRUCTION — no vacuous-pass arm
    // here (unlike the `== 0` pins in task0299 / task0303_07, which
    // DO admit vacuous-pass under silent-skip).
    let (_linked, acfg) = lower("05-stencil", "schedules/distributed-2d.sched.nuc");

    let blur3_id = *acfg.name_kernels.get("blur3").expect("blur3 in ACFG");
    let y_iv = *acfg.name_iter_vars.get("y").expect("y in ACFG");
    let x_iv = *acfg.name_iter_vars.get("x").expect("x in ACFG");

    let blur3_y = acfg
        .halo_widths
        .get(&blur3_id)
        .and_then(|m| m.get(&y_iv))
        .copied()
        .unwrap_or(0);
    let blur3_x = acfg
        .halo_widths
        .get(&blur3_id)
        .and_then(|m| m.get(&x_iv))
        .copied()
        .unwrap_or(0);

    assert_eq!(
        blur3_y, 1,
        "halo_widths[blur3][y] must be 1; the schedule header at \
         nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc:53 \
         claims `halo_y = halo_x = 1 (inferred from blur3's 3x3 access)` \
         and the halo-strip synthesis (TASK-0289) depends on it. \
         acfg.halo_widths = {:?}",
        acfg.halo_widths
    );
    assert_eq!(
        blur3_x, 1,
        "halo_widths[blur3][x] must be 1; same narrative claim as above. \
         acfg.halo_widths = {:?}",
        acfg.halo_widths
    );
}

#[test]
fn task0303_07_matmul_distributed_halo_widths_pinned_to_zero() {
    // 07-matmul/schedules/distributed.sched.nuc:25-26 carries the
    // load-bearing narrative claim `no halo, no cross-worker carry, no
    // reduction across i`. The bit-identical e2e cells across all 4
    // tier-1 backends rest on this narrative being true.
    //
    // Scope (structural vs behavioural — narrowed deliberately):
    //   This test pins ONLY the halo_widths value (the narrative's
    //   first half — "no halo across i"). It does NOT pin the
    //   downstream BEHAVIOUR that the schedule comment also names
    //   (cycle-118 TASK-0301 axis-mapping filter producing empty
    //   bounds on i for b, leading to whole-array broadcast). That
    //   behaviour lives in transfer_inject, exercised by a different
    //   fixture; a regression there with correct halo_widths would
    //   pass this test and be caught only by the e2e bytes.
    //
    // The algorithm reads `madd(c[i][j], a[i][k], b[k][j])`: c, a, b
    // each use only bare-iv index expressions at offset 0. So
    // halo_widths is EITHER omitted entirely for every (kernel, iv) pair
    // OR is explicit-0 for every inspected (kernel, iv). The claim
    // "max halo over i is 0" is the strongest pin and trivially
    // satisfied by either contract form.
    //
    // Soundness floor (TASK-0305 cycle-122 decision, Option B): the
    // contract degree of freedom DOES admit a vacuous pass on a future
    // silent-skip regression. If halo_inference were to stop emitting
    // entries for madd entirely (a walker bug), both `madd_i == 0`
    // (.unwrap_or(0) → 0 ≡ 0) and `max_halo == 0` (empty map .max() →
    // .unwrap_or(0) → 0 ≡ 0) would pass vacuously. Accepted per
    // contract; the alternative (a strict key-exists assertion) would
    // narrow the contract's permitted representations (see the
    // "TASK-0305 cycle-122 project decision (Option B)" paragraph
    // in halo_inference.rs — search for `absent ≡ explicit-0`) and
    // is rejected on test-coupling grounds. The vacuous-pass risk is
    // judged unlikely: today's production walker at `classify_index`
    // (search for `per_iv.entry(iv).or_insert(0)` in halo_inference.rs)
    // always emits an explicit-0 entry for every inspected (kernel, iv)
    // pair. TASK-0307 cycle-123 LANDED the structural sentinel
    // (search for `fn no_halo_bare_iv` in halo_inference.rs — the
    // in-module test now carries a `copied() == Some(0)` assertion
    // alongside the existing `.unwrap_or(0)` contract-form check):
    // closes the vacuous-pass arm at the contract boundary without
    // coupling THIS narrative pin to the explicit-0 representation.
    //
    // If a future kernel-surface change introduces a non-zero i-axis
    // offset (e.g. `a[i+1][k]` for some fused stencil-matmul), the
    // schedule comment becomes a comment-doc-lie, the partition=workers
    // tile bounds on a would need a halo extension that transfer_inject
    // doesn't synthesise for matmul today, and bit-identical would
    // break. This test catches the FIRST failure (the narrative lie) at
    // halo-inference time, not at e2e-byte time.
    let (_linked, acfg) = lower("07-matmul", "schedules/distributed.sched.nuc");

    let madd_id = *acfg.name_kernels.get("madd").expect("madd in ACFG");
    let i_iv = *acfg.name_iter_vars.get("i").expect("i in ACFG");

    let madd_i = acfg
        .halo_widths
        .get(&madd_id)
        .and_then(|m| m.get(&i_iv))
        .copied()
        .unwrap_or(0);

    assert_eq!(
        madd_i, 0,
        "halo_widths[madd][i] must be 0; the schedule header at \
         nuc-nucleus/examples/07-matmul/schedules/distributed.sched.nuc:25-26 \
         depends on `no halo across i` for the cycle-118 axis-mapping \
         filter (TASK-0301) to lower bit-identical across all 4 tier-1 \
         backends. acfg.halo_widths = {:?}",
        acfg.halo_widths
    );

    // Defensive: max halo across the WHOLE matmul algorithm must be 0
    // (the narrative is "no halo" without qualifying the axis). Catches
    // a regression on any (kernel, iv) pair, including the un-named
    // {j, k} axes of madd that the narrative covers implicitly.
    let max_halo = acfg
        .halo_widths
        .values()
        .flat_map(|m| m.values().copied())
        .max()
        .unwrap_or(0);
    assert_eq!(
        max_halo, 0,
        "07-matmul is a triple-nested loop with bare-iv index expressions \
         only (madd(c[i][j], a[i][k], b[k][j])); no kernel argument reads \
         at non-zero iv offset. max halo width must be 0; got map {:?}",
        acfg.halo_widths
    );
}
