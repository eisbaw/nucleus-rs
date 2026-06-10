//! TASK-0341.02.02.01 (B') partition-policy-aware fatality pins
//! (TASK-0460 split; shared fixtures live in the `tests` root).
use super::*;

// ---- TASK-0341.02.02.01 (B') partition-policy-aware regression
// ---- pins. The cycle-209 refinement narrows the (B) fatality
// ---- predicate from "any iv in enclosing scope is partitioned"
// ---- to "the failing index expression itself references a
// ---- partitioned iv". Pin both edges:
//
// - `bprime_modwrap_nonpartitioned_axis_stays_advisory` is the
//   16-jacobi/distributed shape: a non-affine Mod-wrap on a
//   non-partitioned axis SHARES the lexical scope with a
//   partitioned iv on a DIFFERENT axis. Cycle 208 demonstrated
//   the (B) rule rejected this; cycle 209's (B') rule classifies
//   it advisory (the precise correctness condition).
//
// - `bprime_modwrap_partitioned_axis_stays_fatal` is the
//   complement: a non-affine Mod-wrap ON the partitioned axis
//   must STILL fire fatal. Verifies that the (B') refinement
//   does not silently weaken correctness when the gap really
//   matters.
//
// - `bprime_strided_on_partitioned_iv_stays_fatal` verifies the
//   per-variant rule applies symmetrically to
//   `StridedAccessNotSupported`: a stride-2 read on the
//   partitioned iv still rejects.
//
// - `bprime_modwrap_no_partition_at_all_stays_advisory` is the
//   11-game-of-life regression pin: under naive/pipelined
//   schedules (no partition directive anywhere) the Mod-wrap
//   error stays advisory (it did under (B); it must continue
//   to under (B')).



fn ir_mod(l: IrExpr, r: IrExpr) -> IrExpr {
    IrExpr::BinOp(IrBinOp::Mod, Box::new(l), Box::new(r))
}

/// 16-jacobi/distributed shape: `grid[(t + 3) % 4][y]` inside
/// `for t { for y { ... } }` with `y` partitioned. The failing
/// index is at axis 0 (the Mod wrap) and references only `t`.
/// Under (B') this stays advisory; under the pre-cycle-209 (B)
/// rule it would have been fatal.
#[test]
fn bprime_modwrap_nonpartitioned_axis_stays_advisory() {
    // for t : 0..5 { for y : 1..7 { out[t][y] <-- K(grid[(t+3)%4][y]) } }
    let stmts = vec![IrStmt::For {
        var: "t".to_string(),
        lo: ir_int(0),
        hi: ir_int(5),
        until: None,
        body: vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(1),
            hi: ir_int(7),
            until: None,
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("t"), ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref(
                        "grid",
                        vec![ir_mod(ir_add(ir_id("t"), ir_int(3)), ir_int(4)), ir_id("y")],
                    )],
                ),
            }],
        }],
    }];
    let mut linked = build_linked(stmts, vec![5, 8]);
    // Partition the y axis, NOT the t axis.
    linked
        .sched
        .loops
        .insert("y".to_string(), loop_partition_workers("y"));
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let (acfg, advisory) = apply_halo_inference_partition_aware(&linked, acfg)
        .expect("(B') must classify this advisory — y partitioned but failing axis is t");
    // Exactly one advisory error (the NonAffineIndex on axis 0).
    assert_eq!(
        advisory.len(),
        1,
        "expected one advisory error, got: {advisory:?}"
    );
    assert!(
        matches!(
            &advisory[0],
            HaloInferenceError::NonAffineIndex { ax_idx: 0, .. }
        ),
        "advisory[0] = {:?}",
        advisory[0]
    );
    // The y-axis read at index 1 of `grid` is affine (bare `y`,
    // halo 0) — halo_widths[K][y] should be recorded with width
    // 0. The t-axis carried no halo because the Mod wrap was
    // unfoldable and the iv is non-partitioned.
    let k_id = *acfg.name_kernels.get("K").unwrap();
    let y_iv = *acfg.name_iter_vars.get("y").unwrap();
    assert_eq!(
        acfg.halo_widths
            .get(&k_id)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(0)
    );
}

/// Complement of the above: the partitioned iv IS `t` (the axis
/// the Mod wrap is on). Now (B') must classify fatal — the
/// partition impact on axis 0 is real, and a halo cannot be
/// inferred.
#[test]
fn bprime_modwrap_partitioned_axis_stays_fatal() {
    let stmts = vec![IrStmt::For {
        var: "t".to_string(),
        lo: ir_int(0),
        hi: ir_int(5),
        until: None,
        body: vec![IrStmt::For {
            var: "y".to_string(),
            lo: ir_int(0),
            hi: ir_int(8),
            until: None,
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("t"), ir_id("y")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref(
                        "grid",
                        vec![ir_mod(ir_add(ir_id("t"), ir_int(3)), ir_int(4)), ir_id("y")],
                    )],
                ),
            }],
        }],
    }];
    let mut linked = build_linked(stmts, vec![5, 8]);
    // Partition the t axis (the WRAP axis).
    linked
        .sched
        .loops
        .insert("t".to_string(), loop_partition_workers("t"));
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
        "(B') must classify fatal — t is partitioned and the failing index references t",
    );
    assert!(
        matches!(err, HaloInferenceError::NonAffineIndex { ax_idx: 0, .. }),
        "expected NonAffineIndex on axis 0, got: {err:?}"
    );
}

/// `StridedAccessNotSupported` on the partitioned iv must still
/// fire fatal. Symmetric to the Mod-wrap fatal case but on the
/// strided variant, exercising a different error-push site.
#[test]
fn bprime_strided_on_partitioned_iv_stays_fatal() {
    // for y : 0..15 { out[y] <-- K(grid[2*y]) } with y partitioned.
    let stmts = vec![IrStmt::For {
        var: "y".to_string(),
        lo: ir_int(0),
        hi: ir_int(15),
        until: None,
        body: vec![IrStmt::Dataflow {
            lhs: lhs("out", vec![ir_id("y")]),
            rhs: ir_call(
                "K",
                vec![data_ref("grid", vec![ir_mul(ir_int(2), ir_id("y"))])],
            ),
        }],
    }];
    let mut linked = build_linked(stmts, vec![32]);
    linked
        .sched
        .loops
        .insert("y".to_string(), loop_partition_workers("y"));
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let err = apply_halo_inference_partition_aware(&linked, acfg).expect_err(
        "(B') must classify fatal — y is partitioned and the failing index references y",
    );
    assert!(
        matches!(err, HaloInferenceError::StridedAccessNotSupported { .. }),
        "expected StridedAccessNotSupported, got: {err:?}"
    );
}

/// 11-game-of-life regression pin: Mod-wrap on iv with NO
/// partition directive anywhere on any iv. Stays advisory under
/// both (B) and (B'). Confirms the cycle-209 refinement did not
/// silently regress the canonical preserved case.
#[test]
fn bprime_modwrap_no_partition_at_all_stays_advisory() {
    let stmts = vec![IrStmt::For {
        var: "t".to_string(),
        lo: ir_int(0),
        hi: ir_int(5),
        until: None,
        body: vec![IrStmt::For {
            var: "i".to_string(),
            lo: ir_int(0),
            hi: ir_int(32),
            until: None,
            body: vec![IrStmt::Dataflow {
                lhs: lhs("out", vec![ir_id("t"), ir_id("i")]),
                rhs: ir_call(
                    "K",
                    vec![data_ref(
                        "grid",
                        vec![ir_mod(ir_add(ir_id("t"), ir_int(4)), ir_int(5)), ir_id("i")],
                    )],
                ),
            }],
        }],
    }];
    let linked = build_linked(stmts, vec![5, 32]);
    // No partition directives anywhere.
    let acfg = crate::acfg::build_acfg(&linked).expect("acfg build");
    let (_acfg, advisory) = apply_halo_inference_partition_aware(&linked, acfg)
        .expect("no partition anywhere ⇒ advisory");
    assert_eq!(
        advisory.len(),
        1,
        "expected one advisory error (the Mod-wrap NonAffineIndex), got: {advisory:?}"
    );
    assert!(matches!(
        advisory[0],
        HaloInferenceError::NonAffineIndex { ax_idx: 0, .. }
    ));
}

