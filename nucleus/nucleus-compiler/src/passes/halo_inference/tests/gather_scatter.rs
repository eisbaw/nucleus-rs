//! TASK-0373 (B') data-dependent READ split + TASK-0384 scatter
//! soundness boundary + whitebox guards (TASK-0460 split; shared
//! fixtures live in the `tests` root).
use super::*;

// ---- TASK-0373 (B') data-dependent READ split + TASK-0384 scatter
// ---- soundness boundary. Pin every arm of the gather/scatter
// ---- distinction:
//
// - `task0373_partitioned_pure_gather_read_stays_advisory` — a pure
//   gather (`y[i] <-- K(x[col_idx[i][k]])`, AFFINE LHS) under
//   `partition=workers(i)` classifies ADVISORY. Whole-array
//   broadcast of `x` serves the data-dependent read; the cell now
//   compiles instead of being fail-loud rejected (the 17-spmv/
//   distributed_gather unblock).
//
// - `task0384_input_index_partitioned_scatter_rmw_admits` — a scatter
//   RMW (`h[input[i]] <-- inc(h[input[i]])`, DATA-DEPENDENT LHS)
//   under `partition=workers(i)` ADMITS (advisory). The scatter
//   target `h` is never affinely indexed by the partitioned iv, so it
//   replicates whole-array and the TASK-0343 element-wise-sum combine
//   is sound (the 08-histogram/distributed.scatter shape). TASK-0373
//   shipped this fixture as fatal — TASK-0384 lands the combine and
//   relaxes it.
//
// - `task0384_bin_partitioned_scatter_rmw_stays_fatal` — a scatter RMW
//   whose target `h` is ALSO affinely indexed by the partitioned iv
//   (`h[i]` with `i` partitioned) STAYS FATAL. NOTE (review P2): in
//   that fixture `h` is broadcast WHOLE-ARRAY (the `h[input[i]]`
//   access makes dim 0 opaque), NOT banded — the unsoundness is the
//   cross-band affine self-read `h[i]` that does not decompose under
//   replicate-then-element-wise-sum. The discriminator rejects it via
//   the conservative "any affine partitioned-iv index into the
//   target" rule. This is the
//   `scatter_target_replicates_whole_array == false` arm.
//
// - `task0390_lhs_index_banding_keeps_scatter_fatal` — bite-test for
//   the LHS-INDEX arm: a scatter target buried affinely inside ANOTHER
//   array's LHS index (`foo[histogram[j]] <-- ...`, `j` partitioned)
//   keeps `scatter_target_replicates_whole_array == false`. Proves the
//   `lhs.indices` descent bites via an A-only-replicates vs
//   A+B-does-not differential (unreachable in today's grammar).
//
// - `task0373_unpartitioned_scatter_rmw_read_stays_advisory` — the
//   same scatter with NO partition directive stays advisory
//   (single-worker needs no transfer): the fatal classification is
//   specifically partition-gated, not a blanket scatter reject.

/// Build a LinkedIR for a 1-D gather/scatter fixture over the data
/// symbols `out`/`x`/`col_idx` (gather) or `h`/`input` (scatter),
/// declared `i32[n]` / `i32[n][m]` as needed. Mirrors `build_linked`
/// but declares the data-dependent-index symbols the canonical
/// helper does not. The kernel is declared `pure` (a gather/scatter
/// compute step). `extra_data` names additional `i32[dim]` symbols.
fn build_linked_gather(
    stmts: Vec<IrStmt>,
    kernel: &str,
    out_name: &str,
    out_dims: Vec<usize>,
    extra_data: &[(&str, Vec<usize>)],
) -> LinkedIR {
    build_linked_gather_arity(stmts, kernel, 1, out_name, out_dims, extra_data)
}

/// Arity-parameterised sibling of [`build_linked_gather`] (TASK-0384):
/// declares `kernel` with `arity` i32 params so a fixture whose call
/// passes more than one arg (e.g. the bin-partition probe
/// `inc(h[input[i]], h[i])`) links cleanly.
fn build_linked_gather_arity(
    stmts: Vec<IrStmt>,
    kernel: &str,
    arity: usize,
    out_name: &str,
    out_dims: Vec<usize>,
    extra_data: &[(&str, Vec<usize>)],
) -> LinkedIR {
    let mut data = BTreeMap::new();
    data.insert(
        out_name.to_string(),
        ResolvedData {
            name: out_name.to_string(),
            ty: t_arr(ScalarType::I32, out_dims),
        },
    );
    for (name, dims) in extra_data {
        data.insert(
            (*name).to_string(),
            ResolvedData {
                name: (*name).to_string(),
                ty: t_arr(ScalarType::I32, dims.clone()),
            },
        );
    }
    let mut kernels = BTreeMap::new();
    kernels.insert(
        kernel.to_string(),
        ResolvedKernel {
            name: kernel.to_string(),
            params: vec![t_scalar(ScalarType::I32); arity],
            ret: Some(t_scalar(ScalarType::I32)),
            purity: Purity::Pure,
            combine: None,
            name_span: None,
        },
    );
    let algo = AlgoIR {
        consts: BTreeMap::new(),
        data,
        kernels,
        stmts,
        // Decl order is inert for halo inference (this fixture builds
        // `data` directly, not from source); empty (TASK-0049.10.06).
        data_decl_order: Vec::new(),
    };
    let mut places: BTreeMap<String, ResolvedPlacement> = BTreeMap::new();
    places.insert(
        kernel.to_string(),
        ResolvedPlacement {
            kernel: kernel.to_string(),
            target: ResolvedPlaceTarget::One("w0".to_string()),
            kernel_span: None,
        },
    );
    let mut workers: BTreeMap<String, ResolvedWorker> = BTreeMap::new();
    workers.insert(
        "w0".to_string(),
        ResolvedWorker {
            name: "w0".to_string(),
            class: crate::sched::DEFAULT_WORKER_CLASS.to_string(),
        },
    );
    let sched = SchedIR {
        algo_path: String::new(),
        worker_classes: BTreeMap::new(),
        memory_regions: BTreeMap::new(),
        workers,
        places,
        place_data: BTreeMap::new(),
        loops: BTreeMap::new(),
        transfers: BTreeMap::new(),
        checks: BTreeMap::new(),
    };
    link(algo, sched).expect("link must succeed for TASK-0373 gather/scatter fixtures")
}

/// AC#3 (advisory arm): pure gather `out[i] <-- K(x[col_idx[i][k]])`
/// under `partition=workers(i)`. The data-dependent READ of `x` is
/// served by whole-array broadcast ⇒ advisory, not fatal.
#[test]
fn task0373_partitioned_pure_gather_read_stays_advisory() {
    // for i : 0..8 { for k : 0..3 {
    //     out[i] <-- K(x[col_idx[i][k]]) } }
    let stmts = vec![IrStmt::For {
        var: "i".to_string(),
        lo: ir_int(0),
        hi: ir_int(8),
        until: None,
        body: vec![IrStmt::For {
            var: "k".to_string(),
            lo: ir_int(0),
            hi: ir_int(3),
            until: None,
            body: vec![IrStmt::Dataflow {
                // AFFINE LHS out[i] ⇒ pure gather, not scatter.
                lhs: lhs("out", vec![ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref(
                        "x",
                        vec![data_ref("col_idx", vec![ir_id("i"), ir_id("k")])],
                    )],
                ),
            }],
        }],
    }];
    let mut linked = build_linked_gather(
        stmts,
        "K",
        "out",
        vec![8],
        &[("x", vec![8]), ("col_idx", vec![8, 3])],
    );
    // Partition the OUTER i axis (the gather row band).
    linked
        .sched
        .loops
        .insert("i".to_string(), loop_partition_workers("i"));
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let (_acfg, advisory) = apply_halo_inference_partition_aware(&linked, acfg).expect(
        "TASK-0373: a partitioned PURE GATHER read must be advisory \
         (whole-array broadcast serves it), not fail-loud rejected",
    );
    assert_eq!(
        advisory.len(),
        1,
        "expected one advisory DataDependentStride, got: {advisory:?}"
    );
    assert!(
        matches!(
            &advisory[0],
            HaloInferenceError::DataDependentStride {
                is_scatter_rmw: false,
                ..
            }
        ),
        "advisory[0] must be a PURE-GATHER DataDependentStride \
         (is_scatter_rmw == false), got: {:?}",
        advisory[0]
    );
}

/// TASK-0384 (admit arm — was the TASK-0373 fatal arm, RELAXED):
/// scatter RMW `h[input[i]] <-- inc(h[input[i]])` under
/// `partition=workers(i)` — the INPUT-INDEX partition. `h` (the
/// scatter target) is never affinely indexed by the partitioned iv
/// `i` (its write index `input[i]` is data-dependent ⇒ whole-array
/// replicate), so the replicate-per-worker + element-wise-sum combine
/// is SOUND and the scatter is now ADVISORY, not fatal. This is the
/// exact 08-histogram/distributed.scatter shape (TASK-0384).
///
/// HISTORY: TASK-0373 (distributed gather) shipped this same fixture
/// as `..._stays_fatal` — at that point the cross-worker data-
/// dependent WRITE was unhandled. TASK-0384 lands the combine (it is
/// the TASK-0343 accumulator), so the canonical input-index shape is
/// admitted; only a BIN partition stays fatal (see
/// `task0384_bin_partitioned_scatter_rmw_stays_fatal`).
#[test]
fn task0384_input_index_partitioned_scatter_rmw_admits() {
    // for i : 0..8 { h[input[i]] <-- inc(h[input[i]]) }
    let stmts = vec![IrStmt::For {
        var: "i".to_string(),
        lo: ir_int(0),
        hi: ir_int(8),
        until: None,
        body: vec![IrStmt::Dataflow {
            // DATA-DEPENDENT LHS h[input[i]] ⇒ scatter RMW; `h` is
            // NOT affinely indexed by `i`.
            lhs: lhs("h", vec![data_ref("input", vec![ir_id("i")])]),
            rhs: ir_call(
                "inc",
                vec![data_ref("h", vec![data_ref("input", vec![ir_id("i")])])],
            ),
        }],
    }];
    let mut linked = build_linked_gather(
        stmts,
        "inc",
        "h",
        vec![16],
        &[("input", vec![8])],
    );
    // Partition the INPUT INDEX `i` (the i-band, the sound shape).
    linked
        .sched
        .loops
        .insert("i".to_string(), loop_partition_workers("i"));
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let (_acfg, advisory) = apply_halo_inference_partition_aware(&linked, acfg).expect(
        "TASK-0384: an INPUT-INDEX-partitioned scatter RMW must ADMIT \
         (advisory) — `h` replicates whole-array (its write index is \
         data-dependent), so replicate-per-worker + element-wise-sum \
         is sound",
    );
    assert_eq!(
        advisory.len(),
        1,
        "expected one advisory DataDependentStride, got: {advisory:?}"
    );
    assert!(
        matches!(
            &advisory[0],
            HaloInferenceError::DataDependentStride {
                is_scatter_rmw: true,
                ..
            }
        ),
        "advisory[0] must be a SCATTER-RMW DataDependentStride \
         (is_scatter_rmw == true) but ADVISORY because the scatter \
         target replicates whole-array, got: {:?}",
        advisory[0]
    );
}

/// TASK-0384 (fatal arm — the soundness boundary). A scatter RMW
/// whose target `h` is ALSO affinely indexed by the partitioned iv
/// STAYS FATAL. `scatter_target_replicates_whole_array` returns false
/// because the affine `h[i]` access (with `i` partitioned) trips the
/// "affine partitioned-iv index into the target" predicate.
///
/// MECHANISM (review P2 — be precise, do NOT claim `h` is banded): in
/// THIS fixture the data-dependent `h[input[i]]` access marks `h`
/// dim 0 OPAQUE (sticky in `transfer_inject::record_access_per_dim`),
/// so the transfer layer would actually broadcast `h` WHOLE-ARRAY —
/// it does NOT band-partition it. The unsoundness is therefore NOT
/// "a worker drops out-of-band scatters"; it is the cross-band affine
/// self-read `h[i]`: under replicate-then-element-wise-sum each worker
/// reads its OWN partial's `h[i]` (zero outside its scatter slice),
/// not the global accumulated value, so the combine is wrong. FATAL
/// is the correct outcome, and the discriminator reaches it via the
/// conservative "any affine partitioned-iv index into the target"
/// rule — a SUPERSET of the truly-unsound set (see the helper
/// docstring). The genuine band-partition shape (`h[b]`, `b`
/// partitioned, no data-dependent dim) is also rejected by the same
/// rule but is not expressible in a canonical scatter today.
///
/// Fixture: `h[input[i]] <-- inc(h[input[i]], h[i])` — the RHS
/// `h[input[i]]` fires `DataDependentStride` + stamps
/// `is_scatter_rmw == true`; the extra affine `h[i]` arg is what flips
/// the canonical-shape admit (the test above) back to FATAL. The
/// two-arg `inc` makes the affine self-read explicit (the canonical
/// 08-histogram example has no such access — this is the boundary the
/// fixture probes).
#[test]
fn task0384_bin_partitioned_scatter_rmw_stays_fatal() {
    // for i : 0..8 { h[input[i]] <-- inc(h[input[i]], h[i]) }
    let stmts = vec![IrStmt::For {
        var: "i".to_string(),
        lo: ir_int(0),
        hi: ir_int(8),
        until: None,
        body: vec![IrStmt::Dataflow {
            // Data-dependent WRITE (scatter) ...
            lhs: lhs("h", vec![data_ref("input", vec![ir_id("i")])]),
            rhs: ir_call(
                "inc",
                vec![
                    // ... RHS data-dependent self-read (fires the
                    // DataDependentStride error + is_scatter_rmw) ...
                    data_ref("h", vec![data_ref("input", vec![ir_id("i")])]),
                    // ... AND an AFFINE read h[i] with `i`
                    // partitioned — band-partitions the target.
                    data_ref("h", vec![ir_id("i")]),
                ],
            ),
        }],
    }];
    // `inc` declared 2-param so the affine h[i] arg links cleanly.
    let mut linked = build_linked_gather_arity(
        stmts,
        "inc",
        2,
        "h",
        vec![16],
        &[("input", vec![8])],
    );
    // Partition the index `i` — but here `i` ALSO affinely indexes
    // the scatter target, so it is a bin-band, NOT the sound
    // input-index shape.
    linked
        .sched
        .loops
        .insert("i".to_string(), loop_partition_workers("i"));
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
        "TASK-0384: a scatter RMW whose target is ALSO affinely \
         indexed by the partitioned iv (band-partition) must STAY \
         FATAL — replicate-per-worker + element-wise-sum is unsound \
         when the target is band-partitioned",
    );
    assert!(
        matches!(
            err,
            HaloInferenceError::DataDependentStride {
                is_scatter_rmw: true,
                ..
            }
        ),
        "expected a SCATTER-RMW DataDependentStride (is_scatter_rmw == \
         true) classified FATAL, got: {err:?}"
    );
}

/// TASK-0390 — bite-test for the LHS-INDEX banding arm of
/// `algo_target_has_affine_partitioned_index` (halo_inference/partition_policy.rs,
/// the `lhs.indices.iter().any(expr_bands_target)` clause). That arm makes
/// the discriminator descend a write LHS's index sub-expressions so a
/// scatter target that appears AFFINELY inside ANOTHER array's LHS
/// index — `foo[ histogram[j] ] <-- ...`, `j` partitioned — keeps the
/// scatter `scatter_target_replicates_whole_array == false` (FATAL).
/// The arm is additive-conservative (can only keep MORE scatters
/// FATAL) and is unreachable in today's grammar, so it had no
/// dedicated coverage — this is the "prove-the-guard-bites" pin
/// (recurring class: TASK-0374/0379/0381).
///
/// It bites via a self-contained DIFFERENTIAL, so the proof does not
/// rely on a one-off "delete the line and re-run" experiment. Case A:
/// statement A ALONE (canonical `histogram[input[i]]` scatter, `i`
/// partitioned) replicates histogram whole-array (the data-dependent
/// dim is opaque) so `scatter_target_replicates_whole_array == true`.
/// Case A+B: adding B = `foo[ histogram[j] ] <-- inc(foo[j])` with `j`
/// partitioned introduces exactly ONE new banding source — `histogram`
/// appearing affinely inside `foo`'s LHS index. B's own LHS ref (`foo`,
/// not the target) and its RHS (`inc(foo[j])`, no `histogram` access)
/// are both non-banding, so the flip to `== false` is attributable
/// SOLELY to the LHS-index arm.
///
/// Empirically confirmed: disabling the `lhs.indices` clause (a
/// `false &&` short-circuit on it) makes the A+B assertion fail while
/// the case-A baseline still passes — the differential and the manual
/// disable agree.
#[test]
fn task0390_lhs_index_banding_keeps_scatter_fatal() {
    // Statement A — canonical scatter `histogram[input[i]] <-- inc(...)`.
    // Data-dependent LHS index ⇒ histogram dim 0 opaque ⇒ replicate.
    let stmt_a = || IrStmt::For {
        var: "i".to_string(),
        lo: ir_int(0),
        hi: ir_int(8),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("histogram", vec![data_ref("input", vec![ir_id("i")])]),
            rhs: ir_call(
                "inc",
                vec![data_ref("histogram", vec![data_ref("input", vec![ir_id("i")])])],
            ),
        }],
    };
    // Statement B — `foo[ histogram[j] ] <-- inc(foo[j])`, `j`
    // partitioned. `histogram` (the scatter target) appears AFFINELY
    // (`[j]`) buried inside `foo`'s LHS index: the LHS-index arm's
    // exclusive trigger.
    let stmt_b = || IrStmt::For {
        var: "j".to_string(),
        lo: ir_int(0),
        hi: ir_int(8),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("foo", vec![data_ref("histogram", vec![ir_id("j")])]),
            rhs: ir_call("inc", vec![data_ref("foo", vec![ir_id("j")])]),
        }],
    };

    // Case A — statement A only, `i` partitioned: replicate (sound
    // input-index scatter). Establishes the differential baseline.
    let mut linked_a = build_linked_gather_arity(
        vec![stmt_a()],
        "inc",
        1,
        "histogram",
        vec![16],
        &[("input", vec![8]), ("foo", vec![16])],
    );
    linked_a
        .sched
        .loops
        .insert("i".to_string(), loop_partition_workers("i"));
    assert!(
        scatter_target_replicates_whole_array(&linked_a, "histogram"),
        "TASK-0390 baseline: the canonical input-index scatter alone \
         must replicate histogram whole-array (no affine partitioned-iv \
         index into the target)"
    );

    // Case A+B — statements A + B, `i` AND `j` partitioned: the buried
    // `histogram[j]` inside foo's LHS index bands the target, so it
    // must NOT replicate. Statement A is byte-identical to case A, so
    // the only change is statement B's LHS-index banding.
    let mut linked_ab = build_linked_gather_arity(
        vec![stmt_a(), stmt_b()],
        "inc",
        1,
        "histogram",
        vec![16],
        &[("input", vec![8]), ("foo", vec![16])],
    );
    linked_ab
        .sched
        .loops
        .insert("i".to_string(), loop_partition_workers("i"));
    linked_ab
        .sched
        .loops
        .insert("j".to_string(), loop_partition_workers("j"));
    assert!(
        !scatter_target_replicates_whole_array(&linked_ab, "histogram"),
        "TASK-0390: a scatter target appearing affinely (`histogram[j]`, \
         `j` partitioned) inside ANOTHER array's LHS index must keep the \
         scatter FATAL (replicates == false) — the LHS-index arm of \
         algo_target_has_affine_partitioned_index. If this fails, the \
         `lhs.indices` descent has regressed and a band-partitioned \
         target could be unsoundly admitted as whole-array replicate."
    );
}

/// AC#3 (boundary): the same scatter RMW with NO partition directive
/// stays advisory — the fatal classification is partition-gated, not
/// a blanket scatter reject (single-worker needs no transfer).
#[test]
fn task0373_unpartitioned_scatter_rmw_read_stays_advisory() {
    let stmts = vec![IrStmt::For {
        var: "i".to_string(),
        lo: ir_int(0),
        hi: ir_int(8),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("h", vec![data_ref("input", vec![ir_id("i")])]),
            rhs: ir_call(
                "inc",
                vec![data_ref("h", vec![data_ref("input", vec![ir_id("i")])])],
            ),
        }],
    }];
    // No partition directive anywhere.
    let linked = build_linked_gather(
        stmts,
        "inc",
        "h",
        vec![16],
        &[("input", vec![8])],
    );
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let (_acfg, advisory) = apply_halo_inference_partition_aware(&linked, acfg).expect(
        "TASK-0373: an UNPARTITIONED scatter RMW read stays advisory \
         (no transfer needed single-worker)",
    );
    assert_eq!(
        advisory.len(),
        1,
        "expected one advisory DataDependentStride, got: {advisory:?}"
    );
    assert!(
        matches!(
            &advisory[0],
            HaloInferenceError::DataDependentStride {
                is_scatter_rmw: true,
                ..
            }
        ),
        "advisory[0] must be a SCATTER-RMW DataDependentStride \
         (is_scatter_rmw == true) but advisory because unpartitioned, \
         got: {:?}",
        advisory[0]
    );
}

/// TASK-0397 white-box: `UnknownKernelInCall` is defensively
/// unreachable from a link-valid IR — `name_kernels` is built (by
/// `build_acfg`) from the same kernel set the calls reference and
/// `apply_halo_inference` is then GIVEN that `acfg`, so by the time
/// the walk runs every callee resolves (an unknown call was
/// already rejected at lowering). It is a KEPT link-invariant
/// tripwire (panic-not-diagnostic policy: a typed error, not a
/// `panic!`/`unreachable!`). Unlike the ConstOverflow/ShapeOverflow
/// negate arm — which a cycle-234 review WRONGLY called unreachable
/// (see `algo_lower.rs`, the negate arm IS reachable by a computed
/// `i64::MIN`) — this one genuinely cannot be reached by any `.nuc`
/// input, so it is proven-to-bite at the unit boundary instead: feed
/// the walk a deliberately INCOMPLETE `name_kernels` (empty) and
/// assert the guard fires for the missing callee.
#[test]
fn unknown_kernel_in_call_guard_bites_whitebox() {
    let name_kernels: BTreeMap<String, KernelId> = BTreeMap::new();
    let name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    let consts: BTreeMap<String, ResolvedConst> = BTreeMap::new();
    let ctx = WalkCtx {
        name_kernels: &name_kernels,
        name_iter_vars: &name_iter_vars,
        consts: &consts,
    };

    // One `<--` whose RHS calls a kernel absent from `name_kernels`.
    let stmts = vec![IrStmt::Dataflow {
        lhs: lhs("out", vec![ir_id("i")]),
        rhs: ir_call("ghost", vec![data_ref("grid", vec![ir_id("i")])]),
    }];

    let mut out: BTreeMap<KernelId, BTreeMap<IterVar, u64>> = BTreeMap::new();
    let mut errors: Vec<HaloErrorWithScope> = Vec::new();
    collect_from_stmts(&stmts, &[], &ctx, &mut out, &mut errors);

    assert!(
        errors.iter().any(|(e, _, _)| matches!(
            e,
            HaloInferenceError::UnknownKernelInCall { callee } if callee == "ghost"
        )),
        "an incomplete name_kernels must raise UnknownKernelInCall for the \
         missing `ghost` callee; got {errors:?}"
    );
}

/// TASK-0402 white-box: `UnknownLoopVar` is the lone untested sibling
/// in the `UnknownLoopVar` guard family. `reuse_inference`
/// (`sidecar_reuse.rs`), `partition_workers`, `partition_rows`,
/// `partition_blocks2d` (TASK-0400), and `block_transform`
/// (`tests/block_transform.rs`) all bite-test theirs; this test
/// closes the family (a `feedback-silent-sibling-defect` completion).
///
/// Like the reuse sibling, it is a KEPT link-invariant tripwire
/// (panic-not-diagnostic policy: a typed error, not `unreachable!`).
/// For link-valid IR the iv set collected from the body `for`-loop
/// scope is always a subset of `name_iter_vars`: `build_acfg`'s
/// `collect_iter_var_names` and this pass's scope walk traverse the
/// SAME `IrStmt::For` nodes, so every body `for` var is present in
/// `name_iter_vars`. The guard therefore fires only on an
/// inconsistently-constructed `(LinkedIR, ACFG)` pair. We reproduce
/// exactly that: build a link-valid pair, then DELETE `y` from
/// `acfg.name_iter_vars` while the `for y` loop stays in the body —
/// the walk still collects `y` from scope but cannot resolve it.
///
/// The index is `grid[y + 1]` (affine, coefficient +1) on purpose: a
/// non-unit coefficient exits earlier at `StridedAccessNotSupported`
/// (the `coeff != 1` arm) before the `name_iter_vars` lookup, so the
/// failing index MUST be coeff-1 for this guard to be the one that
/// fires.
#[test]
fn unknown_loop_var_guard_bites_whitebox() {
    // for y : 1..15 { out[y] <-- K(grid[y + 1]) }
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(1),
        hi: ir_int(15),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call(
                "K",
                vec![data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))])],
            ),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let mut acfg = crate::acfg::build_acfg(&linked).expect("acfg build");

    // Precondition: the link-valid ACFG carries `y`, so the deletion
    // below is a genuine poison and not a no-op on an absent key.
    assert!(
        acfg.name_iter_vars.contains_key("y"),
        "precondition: link-valid ACFG must carry the `y` iter-var"
    );
    // Poison: drop `y` from name_iter_vars while the `for y` loop
    // stays in the body — the inconsistent `(LinkedIR, ACFG)` pair.
    acfg.name_iter_vars.remove("y");

    let err = apply_halo_inference(&linked, acfg)
        .expect_err("a name_iter_vars missing the body iv must fail closed");
    match err {
        HaloInferenceError::UnknownLoopVar { var } => {
            assert_eq!(var, "y", "UnknownLoopVar must name the missing iv");
        }
        other => panic!("expected UnknownLoopVar, got {other:?}"),
    }
}
