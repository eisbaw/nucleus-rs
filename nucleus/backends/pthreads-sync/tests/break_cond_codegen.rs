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
//! The break EMIT shape is verified at the render-string layer here; the
//! downstream `cargo build` + run + bit-identical differential is covered
//! by the `21-jacobi-converge` example promoted into the e2e matrix (epic
//! S5, TASK-0341.02.01.06 — the FIRST non-inert consumer). As of S5 the
//! break also CAPTURES the break generation into `__nuc_break_gen` and the
//! post-loop extraction reads the runtime `field[__nuc_final_gen]` rather
//! than the compile-time cap slice (TASK-0341.02.01.05.02), with a
//! cap-hit-not-converged stderr diagnostic (TASK-0341.02.01.05.03). The
//! `runtime_final_read_*` tests below pin those.

use nucleus_compiler::algo::{IrCmpOp, IrExpr, Purity, ResolvedType, ScalarType};
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
            purity: Purity::Pure,
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
        out.contains("break; }"),
        "a break_cond=Some loop must emit a `break;`; got:\n{out}"
    );
    // The break is guarded by the rendered Compare over the runtime
    // value. `acc[t]` is a full-rank gather load `acc[(t) as usize]`.
    // The break also CAPTURES the break generation into `__nuc_break_gen`
    // (TASK-0341.02.01.05.02) before the `break;` — `var` is `t` here.
    assert!(
        out.contains("if (acc[(t) as usize] < 1) { __nuc_break_gen = t; break; }"),
        "the break must be guarded by the rendered bool Compare and capture \
         the break generation; got:\n{out}"
    );
    // The capture variable is declared (sentinel -1) before the loop, and
    // the cap-hit observability block + `__nuc_final_gen` are emitted
    // after it (TASK-0341.02.01.05.02 / .05.03).
    assert!(
        out.contains("let mut __nuc_break_gen: i64 = -1;"),
        "the break-generation capture variable must be declared with the -1 \
         (did-not-converge) sentinel; got:\n{out}"
    );
    assert!(
        out.contains("[[nuc_converge]] did NOT converge")
            && out.contains("let __nuc_final_gen: i64 ="),
        "the cap-hit-not-converged observability diagnostic + __nuc_final_gen \
         resolution must be emitted after the loop; got:\n{out}"
    );
    // Ordering: the break must come AFTER the body Fire (the kernel call),
    // not before it — the final iteration is fully executed before the
    // loop terminates.
    let fire_pos = out
        .find("kernels::step")
        .expect("the body Fire must be emitted");
    let break_pos = out.find("break; }").expect("the break must be emitted");
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

/// A post-loop extraction Fire `out[i] <-- step(acc[CAP])` reading the
/// break-array `acc` at the COMPILE-TIME cap slice. `acc` is rank-1 here,
/// so `acc[CAP]` is a full-rank scalar load; `CAP` is the literal cap
/// `range.end - 1` of the break loop (7 for `0..8`).
fn extraction_fire(data: DataId, kernel: KernelId, out_data: DataId, cap: i64) -> Event {
    let read = DataSlice {
        data,
        indices: vec![IrExpr::IntLit(cap)],
    };
    let write = DataSlice {
        data: out_data,
        indices: vec![IrExpr::Ident("i".to_string())],
    };
    Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(read)],
            output: Some(write),
        },
    }
}

#[test]
fn runtime_final_read_rewrites_cap_index_to_break_gen() {
    // TASK-0341.02.01.05.02 AC#1+#2. A break loop writes `acc[t]`; a
    // POST-LOOP extraction reads the cap slice `acc[CAP]`. The backend
    // must REWRITE that read's outer index from the compile-time cap to
    // the runtime `__nuc_final_gen` (the captured break generation), so
    // on early-exit at k<CAP the converged generation `acc[k]` is read,
    // NOT the unwritten cap slice.
    let iv = IterVar(0);
    let acc = DataId(0);
    let out = DataId(1);
    let kernel = KernelId(0);
    let (mut names, mut sidecar) = fixtures(iv, acc, kernel);
    names.data.insert(out, "out".to_string());
    sidecar.data_types.insert(
        out,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![8],
        },
    );

    let cond = IrExpr::Compare(
        IrCmpOp::Le,
        Box::new(IrExpr::DataRef(nucleus_compiler::algo::IndexedRef {
            name: "acc".to_string(),
            indices: vec![IrExpr::Ident("t".to_string())],
        })),
        Box::new(IrExpr::IntLit(0)),
    );
    // Break loop over `0..8` -> cap = 7. The extraction reads `acc[7]`.
    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..8,
        body: vec![body_fire(acc, kernel)],
        block_tag: None,
        check_frame: None,
        break_cond: Some(cond),
    };
    let extraction = extraction_fire(acc, kernel, out, 7);

    let rendered = render_single_worker_main(&[loop_ev, extraction], &names, &sidecar)
        .expect("single-worker emit with a break loop + extraction must succeed");

    // The extraction read of the cap slice `acc[7]` must be rewritten to
    // the runtime `__nuc_final_gen`. It must NOT read the bare cap `7`.
    assert!(
        rendered.contains("acc[(__nuc_final_gen) as usize]"),
        "the post-loop extraction read of the cap slice must be rewritten to \
         the runtime break generation __nuc_final_gen; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("acc[(7) as usize]"),
        "the post-loop extraction must NOT read the hard-coded cap slice \
         acc[7] (the unwritten/stale-zero slice on early-exit); got:\n{rendered}"
    );
    // The in-loop self-read `acc[t]` (the break-loop body) must be
    // UNTOUCHED — it references the live loop var, not the cap.
    assert!(
        rendered.contains("acc[(t) as usize]"),
        "the in-loop self-read acc[t] must NOT be rewritten; got:\n{rendered}"
    );
}

#[test]
fn cap_hit_resolution_distinguishes_converged_from_cap_hit() {
    // TASK-0341.02.01.05.03 AC#1+#2. The cap-hit-not-converged case (the
    // -1 sentinel still set) must be OBSERVABLE and DISTINGUISHED from a
    // converged early-exit: a stderr diagnostic fires, and __nuc_final_gen
    // resolves to the cap (last computed generation) on cap-hit vs the
    // captured break gen on convergence.
    let iv = IterVar(0);
    let acc = DataId(0);
    let kernel = KernelId(0);
    let (names, sidecar) = fixtures(iv, acc, kernel);

    let cond = IrExpr::Compare(
        IrCmpOp::Le,
        Box::new(IrExpr::DataRef(nucleus_compiler::algo::IndexedRef {
            name: "acc".to_string(),
            indices: vec![IrExpr::Ident("t".to_string())],
        })),
        Box::new(IrExpr::IntLit(0)),
    );
    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..8,
        body: vec![body_fire(acc, kernel)],
        block_tag: None,
        check_frame: None,
        break_cond: Some(cond),
    };

    let rendered = render_single_worker_main(&[loop_ev], &names, &sidecar)
        .expect("single-worker emit with a break loop must succeed");

    // The branch distinguishing cap-hit (sentinel < 0) from converged
    // must exist, emit a stderr diagnostic, and resolve __nuc_final_gen
    // to the cap (7 = range.end - 1) on cap-hit.
    assert!(
        rendered.contains("if __nuc_break_gen < 0 {"),
        "a cap-hit branch keyed on the -1 sentinel must be emitted; got:\n{rendered}"
    );
    assert!(
        rendered.contains("eprintln!(\"[[nuc_converge]] did NOT converge"),
        "cap-hit must emit an observable stderr diagnostic (NOT a silent \
         stop-at-cap); got:\n{rendered}"
    );
    assert!(
        rendered.contains("if __nuc_break_gen < 0 { 7_i64 } else { __nuc_break_gen }"),
        "__nuc_final_gen must resolve to the cap (7) on cap-hit, else the \
         captured break gen; got:\n{rendered}"
    );
}
