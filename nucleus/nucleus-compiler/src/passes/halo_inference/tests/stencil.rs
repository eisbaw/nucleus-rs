//! Direct affine-decomposer + full-pipeline stencil pins for halo
//! inference (TASK-0460 split; shared fixtures live in the `tests` root).
use super::*;

// ---- Direct affine-decomposer tests ----
//
// Moved to [`crate::passes::common`] tests in cycle 82 (TASK-0261
// prerequisite). Halo inference still exercises the helper
// transitively via the full-pipeline tests below; keeping the
// helper-level coverage in the helper's own module avoids a
// duplicate test surface that would skew the per-pass test count.

// ---- Full-pipeline tests via apply_halo_inference ----

/// Tiny ACFG builder: pulls `linked` through build_acfg. Avoids
/// the partition/transfer passes — halo inference works on the raw
/// `build_acfg` output (post-block-transform is fine but not needed
/// for these synthetic algorithms with no block= directive).
fn build_acfg_and_apply(linked: &LinkedIR) -> Result<ACFG, HaloInferenceError> {
    let acfg = crate::acfg::build_acfg(linked).expect("acfg build");
    apply_halo_inference(linked, acfg)
}

#[test]
fn positive_3point_stencil_along_y() {
    // for y : 1..15 { out[y] <-- K(grid[y-1], grid[y], grid[y+1]) }
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(1),
        hi: ir_int(15),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call(
                "K",
                vec![
                    data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(1))]),
                    data_ref("grid", vec![ir_id("y")]),
                    data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                ],
            ),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
    let k_id = *acfg.name_kernels.get("K").unwrap();
    let y_iv = *acfg.name_iter_vars.get("y").unwrap();
    assert_eq!(
        acfg.halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(1)
    );
    // No other keys.
    assert_eq!(acfg.halo_widths.len(), 1);
}

#[test]
fn positive_9point_stencil_two_axes() {
    // for y : 1..15 { for x : 1..15 {
    //   out[y][x] <-- K(grid[y-1][x-1], grid[y][x], grid[y+1][x+1])
    // } }
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(1),
        hi: ir_int(15),
        until: None,
        body: vec![IrStmt::For {
            var: "x".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            until: None,
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y"), ir_id("x")]),
                rhs: ir_call(
                    "K",
                    vec![
                        data_ref(
                            "grid",
                            vec![ir_sub(ir_id("y"), ir_int(1)), ir_sub(ir_id("x"), ir_int(1))],
                        ),
                        data_ref("grid", vec![ir_id("y"), ir_id("x")]),
                        data_ref(
                            "grid",
                            vec![ir_add(ir_id("y"), ir_int(1)), ir_add(ir_id("x"), ir_int(1))],
                        ),
                    ],
                ),
            }],
        }],
    }];
    let linked = build_linked(stmts, vec![16, 16]);
    let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
    let k_id = *acfg.name_kernels.get("K").unwrap();
    let y_iv = *acfg.name_iter_vars.get("y").unwrap();
    let x_iv = *acfg.name_iter_vars.get("x").unwrap();
    assert_eq!(
        acfg.halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(1)
    );
    assert_eq!(
        acfg.halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&x_iv))
            .copied(),
        Some(1)
    );
    // Outer map has one entry (the kernel K); inner map has two
    // (one per axis).
    assert_eq!(acfg.halo_widths.len(), 1);
    assert_eq!(acfg.halo_widths.get(&k_id).map(|m| m.len()), Some(2));
}

#[test]
fn positive_mixed_access_widest_wins() {
    // for y : 2..14 {
    //   out[y] <-- K(grid[y-2], grid[y], grid[y+1])
    // }
    // halo on y should be max(2, 0, 1) = 2.
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(2),
        hi: ir_int(14),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call(
                "K",
                vec![
                    data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(2))]),
                    data_ref("grid", vec![ir_id("y")]),
                    data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                ],
            ),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
    let k_id = *acfg.name_kernels.get("K").unwrap();
    let y_iv = *acfg.name_iter_vars.get("y").unwrap();
    assert_eq!(
        acfg.halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(2)
    );
}

#[test]
fn no_halo_pure_constant_index() {
    // for y : 0..4 { out[y] <-- K(grid[3]) }
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(0),
        hi: ir_int(4),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call("K", vec![data_ref("grid", vec![ir_int(3)])]),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
    assert!(acfg.halo_widths.is_empty());
}

#[test]
fn no_halo_bare_iv() {
    // for y : 0..4 { out[y] <-- K(grid[y]) } — halo 0.
    //
    // The contract (brief): "the (kernel, iv) entry is either
    // missing OR maps to 0." The implementation chooses to record
    // an explicit 0-width entry on every (kernel, iv) pair the
    // detector inspects — this makes the sidecar's keyset a
    // useful "every iv this kernel touches" index for the Stage 2
    // consumer (TASK-0263). A non-touching iv simply has no entry.
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(0),
        hi: ir_int(4),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call("K", vec![data_ref("grid", vec![ir_id("y")])]),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
    let k_id = *acfg.name_kernels.get("K").unwrap();
    let y_iv = *acfg.name_iter_vars.get("y").unwrap();
    // Width must be 0 (or absent — both satisfy the contract; we
    // emit explicit 0). The lenient form (.unwrap_or(0)) documents
    // the contract; the structural form below pins TODAY'S
    // implementation choice as a sentinel.
    let width = acfg
        .halo_widths
        .get(&k_id)
        .and_then(|m| m.get(&y_iv))
        .copied()
        .unwrap_or(0);
    assert_eq!(width, 0);

    // TASK-0307 cycle-123 structural sentinel (TASK-0305 cycle-122
    // Option B defence). Pins the implementation choice of recording
    // explicit `Some(0)` for every inspected (kernel, iv) pair at
    // the `classify_index` emit site (search for
    // `per_iv.entry(iv).or_insert(0)` in this file). A future
    // walker regression that silently DROPS entries for bare-iv
    // accesses would make the `== 0` `.unwrap_or(0)` narrative pins
    // in `tests/sidecar_halo.rs` (specifically `task0299_06` and
    // `task0303_07` — both `assert == 0`) pass vacuously: no entry
    // → `.unwrap_or(0)` → 0 ≡ 0. (`task0303_05` is the sibling
    // with the same idiom but `assert == 1` — strict-positive, so
    // contract-form-independent BY CONSTRUCTION, NOT vacuous-pass-
    // prone. Sentinel is moot there.) This single sentinel catches
    // the silent-skip at the contract boundary, without coupling
    // downstream tests to the explicit-0 representation (preserves
    // Option B contract).
    assert_eq!(
        acfg.halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(0),
        "structural sentinel: halo_inference must emit an \
         explicit `Some(0)` entry for every inspected (kernel, iv) \
         pair (today's contract-form choice — Option B per \
         TASK-0305). A silent-skip regression here would let the \
         `== 0` `.unwrap_or(0)` narrative pins in \
         tests/sidecar_halo.rs (specifically `task0299_06` and \
         `task0303_07`) pass vacuously."
    );
}

#[test]
fn negative_data_dependent_stride() {
    // for y : 0..4 { out[y] <-- K(grid[lookup[y]]) }
    // — index expression is grid[(lookup[y])] which is a DataRef inside the index. Reject.
    // Add a `lookup` data symbol to the algorithm.
    let mut linked = build_linked(
        vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            until: None,
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref("grid", vec![data_ref("lookup", vec![ir_id("y")])])],
                ),
            }],
        }],
        vec![16],
    );
    linked.algo.data.insert(
        "lookup".to_string(),
        ResolvedData {
            name: "lookup".to_string(),
            ty: t_arr(ScalarType::I32, vec![16]),
        },
    );
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let err = apply_halo_inference(&linked, acfg).unwrap_err();
    match err {
        HaloInferenceError::DataDependentStride {
            kernel,
            ref_name,
            ax_idx,
            is_scatter_rmw,
        } => {
            assert_eq!(kernel, "K");
            assert_eq!(ref_name, "grid");
            assert_eq!(ax_idx, 0);
            // TASK-0373: the LHS here is the affine `out[y]`, so this
            // is a pure GATHER read, not a scatter RMW.
            assert!(
                !is_scatter_rmw,
                "affine LHS `out[y]` ⇒ pure gather, is_scatter_rmw must be false"
            );
        }
        other => panic!("expected DataDependentStride, got {other:?}"),
    }
}

#[test]
fn negative_strided_access() {
    // for y : 0..4 { out[y] <-- K(grid[2*y + 1]) } — coefficient 2, reject.
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(0),
        hi: ir_int(4),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call(
                "K",
                vec![data_ref(
                    "grid",
                    vec![ir_add(ir_mul(ir_int(2), ir_id("y")), ir_int(1))],
                )],
            ),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let err = apply_halo_inference(&linked, acfg).unwrap_err();
    match err {
        HaloInferenceError::StridedAccessNotSupported { coefficient, .. } => {
            assert_eq!(coefficient, 2);
        }
        other => panic!("expected StridedAccessNotSupported, got {other:?}"),
    }
}

#[test]
fn negative_two_iter_vars_in_one_index() {
    // for y : 0..4 { for x : 0..4 {
    //   out[y][x] <-- K(grid[y + x]) — two iter-vars in one index. Reject.
    // } }
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(0),
        hi: ir_int(4),
        until: None,
        body: vec![IrStmt::For {
            var: "x".to_string(),
            lo: ir_int(0),
            hi: ir_int(4),
            until: None,
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y"), ir_id("x")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref("grid", vec![ir_add(ir_id("y"), ir_id("x"))])],
                ),
            }],
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let err = apply_halo_inference(&linked, acfg).unwrap_err();
    match err {
        HaloInferenceError::MultipleIterVarsInIndex { iter_vars, .. } => {
            assert_eq!(iter_vars, vec!["x".to_string(), "y".to_string()]);
        }
        other => panic!("expected MultipleIterVarsInIndex, got {other:?}"),
    }
}

#[test]
fn negative_negated_iv_rejected() {
    // for y : 0..4 { out[y] <-- K(grid[-y]) } — coefficient -1, reject.
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(0),
        hi: ir_int(4),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call(
                "K",
                vec![data_ref("grid", vec![IrExpr::Neg(Box::new(ir_id("y")))])],
            ),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let err = apply_halo_inference(&linked, acfg).unwrap_err();
    match err {
        HaloInferenceError::StridedAccessNotSupported { coefficient, .. } => {
            assert_eq!(coefficient, -1);
        }
        other => panic!("expected StridedAccessNotSupported, got {other:?}"),
    }
}

#[test]
fn determinism_same_input_yields_same_map() {
    // Run the same input through the pass twice; assert the
    // resulting halo_widths maps are byte-identical (well, value-
    // identical — BTreeMap implements PartialEq).
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(1),
        hi: ir_int(15),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call(
                "K",
                vec![
                    data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(1))]),
                    data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                ],
            ),
        }],
    }];
    let linked = build_linked(stmts, vec![16]);
    let acfg1 = build_acfg_and_apply(&linked).expect("first run");
    let acfg2 = build_acfg_and_apply(&linked).expect("second run");
    assert_eq!(acfg1.halo_widths, acfg2.halo_widths);
}

#[test]
fn nested_call_inside_kernel_arg_recurses() {
    // for y : 1..15 {
    //   out[y] <-- K(inner(grid[y-1], grid[y+1]))
    // }
    // The OUTER call K has no DataRef args (its arg is a Call).
    // Halo inference must still scan the INNER call's args and
    // record halo against `inner`'s KernelId, not K's.
    let mut linked = build_linked(
        vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(1),
            hi: ir_int(15),
            until: None,
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![ir_call(
                        "inner",
                        vec![
                            data_ref("grid", vec![ir_sub(ir_id("y"), ir_int(1))]),
                            data_ref("grid", vec![ir_add(ir_id("y"), ir_int(1))]),
                        ],
                    )],
                ),
            }],
        }],
        vec![16],
    );
    // Add the `inner` kernel + its placement so link succeeds.
    linked.algo.kernels.insert(
        "inner".to_string(),
        ResolvedKernel {
            name: "inner".to_string(),
            params: vec![t_scalar(ScalarType::I32), t_scalar(ScalarType::I32)],
            ret: Some(t_scalar(ScalarType::I32)),
            purity: Purity::Pure,
            combine: None,
            name_span: None,
        },
    );
    linked.sched.places.insert(
        "inner".to_string(),
        ResolvedPlacement {
            kernel: "inner".to_string(),
            target: ResolvedPlaceTarget::One("w0".to_string()),
            kernel_span: None,
        },
    );
    // Re-link to refresh kernel_workers / placements.
    let linked = link(linked.algo, linked.sched).expect("re-link");
    let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
    let inner_id = *acfg.name_kernels.get("inner").unwrap();
    let k_id = *acfg.name_kernels.get("K").unwrap();
    let y_iv = *acfg.name_iter_vars.get("y").unwrap();
    // Halo recorded against the INNER kernel (the one whose args
    // touch the DataRefs).
    assert_eq!(
        acfg.halo_widths
            .get(&inner_id)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(1)
    );
    // The outer K has no halo entry.
    assert_eq!(
        acfg.halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        None
    );
}

#[test]
fn no_halo_call_outside_loop_scope() {
    // K(grid[3]) at top level — no enclosing for, no halo entries.
    // (The grid[3] is pure-constant anyway, but the absence of a
    // for nest is itself a stricter no-halo signal.)
    let stmts = vec![IrStmt::Effect {
        callee: "K".to_string(),
        args: vec![data_ref("grid", vec![ir_int(3)])],
    }];
    // K's purity must be effectful for an Effect statement.
    let mut linked = build_linked(stmts, vec![16]);
    linked.algo.kernels.get_mut("K").unwrap().purity = Purity::Effectful;
    // re-link to take the updated purity.
    let linked = link(linked.algo, linked.sched).expect("re-link with effectful K");
    let acfg = build_acfg_and_apply(&linked).expect("halo inference succeeds");
    assert!(acfg.halo_widths.is_empty());
}

