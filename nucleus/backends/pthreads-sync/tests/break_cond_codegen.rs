//! Codegen-string tests for the `for..until` early-exit `break` on the
//! pthreads-sync single-worker sequential backend (epic S4,
//! TASK-0341.02.01.05.04).
//!
//! Asserts that an `Event::Loop` carrying `break_cond: Some(Compare(..))`
//! emits an `if (<lhs> <op> <rhs>) { break; }` as the LAST statement of
//! the rendered loop body, and that a plain `Event::Loop` with
//! `break_cond: None` emits NO break (byte-identical to the pre-S4
//! backend on the loop-body region — the regression guard).
//!
//! Why string-asserting and not compile-and-run: this slice is e2e-INERT
//! (there is no `for..until` example `.nuc` yet — that is a later slice),
//! so the codegen contract is verified at the render-string layer. The
//! downstream `cargo build` + run is covered when an example with a
//! `for..until` is promoted into the e2e matrix (parent .05 / S5).

use nucleus_compiler::algo::{IrCmpOp, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    ArgBinding, DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId,
};
use nucleus_compiler::sidecar::{KernelSig, LoopBound, NameSidecar};

use pthreads_sync::{render_single_worker_main, NameTables};

/// Minimal `(NameTables, NameSidecar)` for a single source loop `t` over
/// data `acc` with a unary `i32 -> i32` kernel `step`.
fn fixtures(iv: IterVar, data: DataId, kernel: KernelId) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.iter_var.insert(iv, "t".to_string());
    names.data.insert(data, "acc".to_string());
    names.kernel.insert(kernel, "step".to_string());

    let mut sidecar = NameSidecar::default();
    // Source loop bound `0 .. 8` so `render_loop_bounds` renders the
    // source form rather than the synthesised-tile concrete form.
    sidecar.loop_bounds.insert(
        iv,
        LoopBound {
            lo: IrExpr::IntLit(0),
            hi: IrExpr::IntLit(8),
        },
    );
    // `acc` is a rank-1 i32 array so `acc[t]` is a full-rank scalar slot.
    sidecar.data_types.insert(
        data,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![8],
        },
    );
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
        },
    );
    (names, sidecar)
}

/// A loop body `acc[t] <-- step(acc[t])` (one Fire writing a scalar slot).
fn body_fire(data: DataId, kernel: KernelId) -> Event {
    let slot = DataSlice {
        data,
        indices: vec![IrExpr::Ident("t".to_string())],
    };
    Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(slot.clone())],
            output: Some(slot),
        },
    }
}

#[test]
fn break_cond_some_emits_if_break_after_body() {
    // TASK-0341.02.01.05.04 AC#1. A source loop carrying a convergence
    // predicate `acc[t] < 1` emits `if (acc[...] < 1) { break; }` as the
    // last statement of the loop body.
    let iv = IterVar(0);
    let data = DataId(0);
    let kernel = KernelId(0);
    let (names, sidecar) = fixtures(iv, data, kernel);

    let cond = IrExpr::Compare(
        IrCmpOp::Lt,
        Box::new(IrExpr::DataRef(nucleus_compiler::algo::IndexedRef {
            name: "acc".to_string(),
            indices: vec![IrExpr::Ident("t".to_string())],
        })),
        Box::new(IrExpr::IntLit(1)),
    );

    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..8,
        body: vec![body_fire(data, kernel)],
        block_tag: None,
        check_frame: None,
        break_cond: Some(cond),
    };

    let out = render_single_worker_main(&[loop_ev], &names, &sidecar)
        .expect("single-worker emit with a break_cond must succeed");

    assert!(
        out.contains("{ break; }"),
        "a break_cond=Some loop must emit a `{{ break; }}`; got:\n{out}"
    );
    // The break is guarded by the rendered Compare over the runtime
    // value. `acc[t]` is a full-rank gather load `acc[(t) as usize]`.
    assert!(
        out.contains("if (acc[(t) as usize] < 1) { break; }"),
        "the break must be guarded by the rendered bool Compare; got:\n{out}"
    );
    // Ordering: the break must come AFTER the body Fire (the kernel call),
    // not before it — the final iteration is fully executed before the
    // loop terminates.
    let fire_pos = out
        .find("kernels::step")
        .expect("the body Fire must be emitted");
    let break_pos = out.find("{ break; }").expect("the break must be emitted");
    assert!(
        fire_pos < break_pos,
        "the break must be emitted AFTER the loop body, not before it"
    );
}

#[test]
fn break_cond_none_emits_no_break() {
    // TASK-0341.02.01.05.04 regression guard. A plain `for` loop
    // (break_cond=None) must emit NO break — byte-identical to the
    // pre-S4 backend for every existing schedule.
    let iv = IterVar(0);
    let data = DataId(0);
    let kernel = KernelId(0);
    let (names, sidecar) = fixtures(iv, data, kernel);

    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..8,
        body: vec![body_fire(data, kernel)],
        block_tag: None,
        check_frame: None,
        break_cond: None,
    };

    let out = render_single_worker_main(&[loop_ev], &names, &sidecar)
        .expect("single-worker emit without a break_cond must succeed");

    assert!(
        !out.contains("break;"),
        "a plain `for` loop (break_cond=None) must NOT emit any break; got:\n{out}"
    );
}

#[test]
fn break_cond_some_on_block_tagged_loop_fails_loud() {
    // TASK-0341.02.01.05.04 invariant. A strip-mined (block_tag=Some)
    // loop must NEVER carry a break_cond — the projection only sets it
    // from the untagged SOURCE Repeat. Reaching the backend with BOTH is
    // a projection-layer bug; the backend rejects it fail-loud rather
    // than silently dropping the break (the strip-mine arm has no emit
    // site for it).
    let iv = IterVar(0);
    let tile_iv = IterVar(1);
    let data = DataId(0);
    let kernel = KernelId(0);
    let (mut names, sidecar) = fixtures(iv, data, kernel);
    names.iter_var.insert(tile_iv, "tile".to_string());

    let cond = IrExpr::Compare(
        IrCmpOp::Lt,
        Box::new(IrExpr::Ident("t".to_string())),
        Box::new(IrExpr::IntLit(1)),
    );

    let inner = Event::Loop {
        iter_var: iv,
        range: 0..4,
        body: vec![],
        block_tag: Some(nucleus_compiler::event::BlockTag {
            block_n: 4,
            num_full: 4,
            is_partial: false,
        }),
        check_frame: None,
        break_cond: Some(cond),
    };
    let tile_loop = Event::Loop {
        iter_var: tile_iv,
        range: 0..4,
        body: vec![inner],
        block_tag: None,
        check_frame: None,
        break_cond: None,
    };

    let err = render_single_worker_main(&[tile_loop], &names, &sidecar)
        .expect_err("a block-tagged loop carrying a break_cond must fail loud");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("break_cond") && msg.contains("block_tag"),
        "the fail-loud message must name the break_cond + block_tag invariant; got: {msg}"
    );
}
