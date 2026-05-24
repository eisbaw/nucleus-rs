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

    let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
        "partition-aware must escalate on any PartitionKind, not just Workers",
    );
    assert!(
        matches!(err, HaloInferenceError::StridedAccessNotSupported { .. }),
        "expected StridedAccessNotSupported under partition=rows, got {err:?}"
    );
}
