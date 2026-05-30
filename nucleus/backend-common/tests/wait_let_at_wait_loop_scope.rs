//! TASK-0356 cycle 222: characterization + boundary pin for the
//! let-at-wait EMIT scope hazard (cycle-220 architect P3.2).
//!
//! ## The hazard
//!
//! The let-at-wait classifier (`collect_let_at_wait_data`, sibling
//! `collect.rs`) descends into `Event::Loop` bodies via
//! `collect_let_at_wait_inner` (a whole-array Wait buried inside a
//! loop body IS classified let-at-wait — pinned on the classifier
//! side by `collect_let_at_wait_data.rs::
//! whole_array_wait_inside_event_loop_body_included`).
//!
//! For a let-at-wait datum the walker's `render_wait_assign`
//! (`wait.rs`) emits `let <name> = <rhs>;` (declare-and-assign) AT THE
//! WAIT'S SCOPE — the per-backend pre-init pass deliberately OMITS the
//! outer `let mut <name>: Vec<..> = vec![0; N];` line for these data,
//! so the in-scope `let` is the ONLY declaration of `<name>`.
//!
//! When the Wait sits inside an `Event::Loop` body, that `let <name>`
//! lands INSIDE the emitted `for { ... }` block. If a downstream
//! consumer (a `Fire` whose kernel-arg reads the data, or a `Push` of
//! the data) sits at the ENCLOSING outer scope, it references
//! `<name>` AFTER the loop closes — i.e. out of scope. The generated
//! Rust does not compile.
//!
//! ## What this file establishes (OUTCOME-MATRIX branch d — now guarded)
//!
//! TASK-0356 AC#2 asked for either (a) an `EmitError` contract-gap or
//! (b) a correct outer-scope `let mut` fallback. TASK-0356 (cycle 222)
//! characterized the ACTUAL cycle-222 behaviour as NEITHER (the walker
//! had no scope-tracking, so it emitted the broken-scope Rust and
//! returned `Ok`). TASK-0364 (this landing) closes that with OPTION (a):
//! a typed `EmitError::ContractGap` fail-loud guard.
//!
//! - `render_worker_events` now returns `Err(EmitError::ContractGap)`
//!   for the at-risk shape — `collect::check_let_at_wait_scope_safety`,
//!   called once at the walker entry, detects a let-at-wait Wait whose
//!   value is consumed at an enclosing (non-dominated) scope. Pinned by
//!   `at_risk_shape_emits_scope_gap_error`.
//! - The previously-characterized BROKEN emit (`let <name>` inside the
//!   `for` block, consumer reading `<name>` after it — rustc `E0425`,
//!   confirmed non-compiling during cycle-222) is now UNREACHABLE: the
//!   guard fails loud BEFORE any code is emitted, so there is no
//!   broken-footprint string to pin anymore.
//!
//! The shipped-code protection is UNCHANGED and is still the UPSTREAM
//! CO-LOCATION INVARIANT enforced by the transfer-injection pass: every
//! cross-worker `Wait` is placed into the SAME sequence (same scope) as
//! its consuming `Operation`, immediately before it. A consumer at the
//! outer scope therefore gets its Wait at the outer scope, never in a
//! nested loop body. The at-risk shape this file hand-builds is NOT
//! producible by `inject_transfers` today. TASK-0364's guard is the
//! second line of defence for a FUTURE pass that breaks the invariant
//! (e.g. a hoist) — it converts a latent miscompile into a fail-loud
//! contract gap. OPTION (a) (typed `EmitError`) was chosen over the
//! classifier-side OPTION (A) (scope-aware exclusion) precisely because
//! the shape is non-producible: failing loud carries near-zero
//! regression risk and matches the project's panic-not-diagnostic
//! response to contract gaps, whereas a silent classifier transform
//! would alter a code path no shipped schedule exercises. The classifier
//! is therefore UNCHANGED (it still includes `buf`; see
//! `classifier_includes_in_loop_whole_array_wait_for_at_risk_shape`).
//!
//! Enforcement site (verified against code, cycle 222, NOT just the
//! docstring claim): `nucleus-compiler/src/passes/transfer_inject.rs`,
//! `inject_in_sequence` — the consumer-side Wait is pushed via
//! `out.push(ACFGNode::Xfer(w))` into the SAME `out` Vec the consumer
//! `Operation` is appended to (the `ACFGNode::Operation(op)` arm:
//! Waits from `build_waits_for_op` are pushed immediately before
//! `out.push(child.clone())` for the Operation). Descent into nested
//! blocks happens by a recursive `inject_in_node_with_tile` ->
//! `inject_in_sequence` call, so a consumer nested in a loop body
//! gets its Wait inside THAT nested sequence — never a Wait in a
//! nested loop with its consumer in the enclosing scope. The module
//! docstring states the same: "Insert the Wait immediately before O
//! in O's enclosing sequence" (transfer_inject.rs line ~58).
//!
//! ## Why this is a worthwhile pin despite being defensive
//!
//! This is endgame "documentation validation": the cycle-220 P3.2
//! hazard note is a documented boundary, and this file turns it into
//! an executable characterization. If a FUTURE pass ever does
//! construct the at-risk shape (e.g. a hoist that lifts a consumer
//! out of a loop while leaving its Wait behind), the
//! `at_risk_shape_emits_scope_gap_error` pin proves the TASK-0364 guard
//! converts it into a fail-loud `EmitError::ContractGap` instead of a
//! silent miscompile, and `safe_in_loop_wait_and_consumer_emits_ok`
//! proves the guard does NOT over-fire on the well-scoped shape. The
//! guard lives in `collect::check_let_at_wait_scope_safety`, called
//! once at the `event_walker::render_worker_events` entry. A code
//! comment at `wait.rs` (the `let {name} = {rhs};` emit site) and
//! `collect.rs` (the loop descent) cross-references TASK-0364.
//!
//! ## Sibling audit (silent-sibling discipline)
//!
//! The `let {name} = {rhs};` let-at-wait emit is reached by the FIVE
//! backends that build a populated let-at-wait `WalkerCtx` from
//! `collect_let_at_wait_data` and call `render_worker_events`
//! (verified by grepping the `collect_let_at_wait_data` / `let_at_wait_data`
//! call sites under `nucleus/backends/*/src/`, cycle 222 architect P3.1):
//! pthreads-sync, pthreads-async, mp-tcp-event, mp-uds-event, and
//! openmp-rs. (An earlier draft of this doc undercounted to "three"
//! — `feedback-comment-doc-lie-recurring`.) mp-tcp-bufsync bypasses the
//! walker and calls `render_wait_assign` directly
//! (`backend-common/src/tcp_plan/events.rs`), passing
//! `WalkerCtx::empty_let_at_wait_set()` UNCONDITIONALLY — it never
//! classifies any data as let-at-wait, precisely because its Wait emit
//! wraps the assign in a `{ let __buf = ...; <assign> }` block and a
//! `let {name}` would be block-scoped to that wrap (documented at the
//! cycle-220 comment in `events.rs`). So mp-tcp-bufsync is
//! structurally immune to this hazard. All five populated-set callers
//! share the same upstream `transfer_inject` co-location protection, so
//! none emits the broken-scope `let {name}` inside a loop from valid
//! lower/link output today; the TASK-0364 fix-author must audit all
//! five if the protection ever weakens.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::algo::{IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    ArgBinding, DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId, SeqTag,
    WorkerId,
};
use nucleus_compiler::sidecar::{KernelSig, LoopBound, NameSidecar};
use nucleus_compiler::NameTables;

use backend_common::multi_worker_walker::{
    collect_let_at_wait_data, render_worker_events, WalkerCtx,
};
use backend_common::render::EmitError;

/// Build the at-risk fixture: a whole-array data `buf : i32[8]`, a
/// loop var `t : 0..4`, and a kernel `consume` taking one aggregate
/// `i32[8]` param (whole-array consumer).
fn at_risk_tables() -> (NameTables, NameSidecar, DataId, SeqTag, IterVar, KernelId) {
    let data = DataId(7);
    let seq = SeqTag(3);
    let iv = IterVar(1);
    let kernel = KernelId(5);

    let mut names = NameTables::default();
    names.data.insert(data, "buf".to_string());
    names.iter_var.insert(iv, "t".to_string());
    names.kernel.insert(kernel, "consume".to_string());
    names.worker.insert(WorkerId(0), "w0".to_string());
    names.worker.insert(WorkerId(1), "host".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.data_types.insert(
        data,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![8],
        },
    );
    sidecar.loop_bounds.insert(
        iv,
        LoopBound {
            lo: IrExpr::IntLit(0),
            hi: IrExpr::IntLit(4),
        },
    );
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![8],
            }],
            ret: None,
        },
    );
    (names, sidecar, data, seq, iv, kernel)
}

/// The at-risk event sequence: `Loop { body: [Wait(buf)] }` followed
/// at the OUTER scope by a `Fire` whose kernel input reads `buf`.
fn at_risk_events(data: DataId, seq: SeqTag, iv: IterVar, kernel: KernelId) -> Vec<Event> {
    let inner_wait = Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    };
    let loop_ev = Event::loop_over(iv, 0..4, vec![inner_wait]);
    let consumer = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![],
            })],
            output: None,
        },
    };
    vec![loop_ev, consumer]
}

#[test]
fn classifier_includes_in_loop_whole_array_wait_for_at_risk_shape() {
    // PRECONDITION pin: the classifier DOES include `buf` as
    // let-at-wait for this exact event shape (whole-array Wait nested
    // in a Loop body, no slice, not accumulate, not indexed). This is
    // what makes the emit hazard reachable in the first place. The
    // sibling classifier file pins the same descent in isolation
    // (`collect_let_at_wait_data.rs::
    // whole_array_wait_inside_event_loop_body_included`); here we pin
    // it on the FULL at-risk event sequence so the two tests in this
    // file describe one coherent scenario.
    let (_names, sidecar, data, seq, iv, kernel) = at_risk_tables();
    let events = at_risk_events(data, seq, iv, kernel);

    // No pair_tiles entry → whole-array. Empty accumulate + indexed.
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let acc: BTreeSet<DataId> = BTreeSet::new();
    let indexed: BTreeSet<DataId> = BTreeSet::new();

    let classified = collect_let_at_wait_data(&events, &pair_tiles, &sidecar, &acc, &indexed);
    assert!(
        classified.contains(&data),
        "the classifier MUST include `buf` as let-at-wait for the \
         at-risk shape (whole-array Wait nested in Loop body); this is \
         the precondition that makes the emit hazard reachable; got: \
         {classified:?}"
    );
}

#[test]
fn at_risk_shape_emits_scope_gap_error() {
    // TASK-0364 (OPTION B) GUARD PIN: drive `render_worker_events` on
    // the at-risk shape with `buf` classified let-at-wait. The walker's
    // entry chokepoint (`collect::check_let_at_wait_scope_safety`) MUST
    // now reject this with `EmitError::ContractGap` BEFORE any code is
    // emitted — the in-loop Wait of `buf` is consumed by a Fire at the
    // enclosing root scope that no Wait of `buf` lexically dominates.
    let (names, sidecar, data, seq, iv, kernel) = at_risk_tables();
    let events = at_risk_events(data, seq, iv, kernel);

    let mut rendezvous_ids: BTreeMap<(DataId, SeqTag), usize> = BTreeMap::new();
    rendezvous_ids.insert((data, seq), 0usize);
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let mut let_at_wait: BTreeSet<DataId> = BTreeSet::new();
    // NOTE (cycle 222 architect P3.2; re-characterized cycle 222
    // TASK-0364): this inserts `data` DIRECTLY, bypassing the
    // `collect_let_at_wait_data` classifier. OPTION B (the landed fix)
    // is an EMIT-SIDE guard, so it fires on whatever is IN the
    // let_at_wait set regardless of how it got there — forcing the set
    // membership here is exactly the right driver for the guard. The
    // sibling `classifier_includes_in_loop_whole_array_wait_for_at_risk_shape`
    // separately pins that the real classifier ALSO includes `buf` (so
    // the guard is reachable from a real schedule, not just this forced
    // set). OPTION B leaves the classifier UNCHANGED, so that sibling
    // stays green.
    let_at_wait.insert(data);

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "ring",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: &let_at_wait,
    };

    let mut out = String::new();
    let err = render_worker_events(&ctx, WorkerId(1), &events, &mut out, 0, "")
        .expect_err("TASK-0364 guard must reject the at-risk cross-scope shape");

    match &err {
        EmitError::ContractGap(msg) => {
            // The message must name the data and the scope hazard and be
            // greppable to TASK-0364 (project comment/doc-lie discipline:
            // every claim here is checked against the emitted string).
            assert!(
                msg.contains("buf"),
                "ContractGap must name the offending data `buf`; got: {msg}"
            );
            assert!(
                msg.contains("TASK-0364"),
                "ContractGap must reference TASK-0364 for greppability; got: {msg}"
            );
            assert!(
                msg.contains("scope") && msg.contains("Wait"),
                "ContractGap must describe the let-at-wait Wait / enclosing-scope \
                 hazard; got: {msg}"
            );
        }
        other => panic!("expected EmitError::ContractGap, got {other:?}"),
    }

    // The guard fails BEFORE emit, so nothing is written: the
    // previously-characterized broken-scope footprint is now
    // unreachable. (We do not assert `out.is_empty()` because the guard
    // runs first and short-circuits — but it must NOT contain the
    // broken in-loop `let buf` that the consumer would read out of
    // scope.)
    assert!(
        !out.contains("let buf"),
        "guard must short-circuit before emitting the broken-scope `let buf`; \
         got:\n{out}"
    );
}

#[test]
fn safe_in_loop_wait_and_consumer_emits_ok() {
    // TASK-0364 BOUNDARY / over-fire pin (AC#3): the SAFE shape where
    // the Wait of `buf` AND its consuming Fire are BOTH inside the SAME
    // loop body (scope-path `[L0]` each). The in-loop Wait's `let buf`
    // lexically dominates the in-loop consumer, so the guard MUST NOT
    // fire — `render_worker_events` returns Ok and emits the
    // declare-and-assign `let buf = ...;` followed by
    // `kernels::consume(buf);` BOTH inside the `for { }` block.
    let (names, sidecar, data, seq, iv, kernel) = at_risk_tables();

    // Safe event shape: Loop { body: [Wait(buf), Fire(consume buf)] }.
    let inner_wait = Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    };
    let inner_consumer = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![],
            })],
            output: None,
        },
    };
    let events = vec![Event::loop_over(iv, 0..4, vec![inner_wait, inner_consumer])];

    let mut rendezvous_ids: BTreeMap<(DataId, SeqTag), usize> = BTreeMap::new();
    rendezvous_ids.insert((data, seq), 0usize);
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let mut let_at_wait: BTreeSet<DataId> = BTreeSet::new();
    let_at_wait.insert(data);

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "ring",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: &let_at_wait,
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(1), &events, &mut out, 0, "")
        .expect("guard must NOT fire on the well-scoped in-loop Wait+consumer shape");

    // Both the `let buf` declare-and-assign AND the consumer land
    // inside the `for { }` block — well-scoped, compiles.
    let expected = "\
for t in (0_i64)..(4_i64) {
    let buf = ring_0.wait(); // recv `buf` from w0
    kernels::consume(buf);
}
";
    assert_eq!(
        out, expected,
        "well-scoped shape must emit both the in-loop `let buf` and the \
         in-loop consumer inside the `for` block; got:\n{out}"
    );
}

#[test]
fn sibling_loop_wait_and_consumer_fires_guard() {
    // TASK-0364 EDGE pin (cycle-222 architect P3): the per-occurrence
    // scope-path counter (NOT `iter_var`) is load-bearing for the
    // sibling-loop shape. `buf` is Wait-recv'd inside loop A and
    // consumed inside a SEPARATE sibling loop B at the SAME nesting
    // depth. The in-loop `let buf` in A is block-scoped to A's `for`
    // and does NOT dominate the consumer in B — so the emitted Rust
    // would not compile, and the guard MUST fire. The two sibling loops
    // get DISTINCT occurrence indices (`[0]` vs `[1]`), so neither
    // Wait-path is a prefix of the other consumer-path → not dominated
    // → unsafe. (If the path used `iter_var` instead of a fresh
    // occurrence index and the two loops happened to share an iter_var,
    // this case would be silently mis-classified as safe — hence the
    // counter.)
    let (names, sidecar, data, seq, iv, kernel) = at_risk_tables();

    let wait_in_a = Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    };
    let consume_in_b = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![],
            })],
            output: None,
        },
    };
    // Loop A (Wait) and a sibling Loop B (consumer) reusing the SAME
    // iter_var `t` — the occurrence counter still disambiguates them.
    let loop_a = Event::loop_over(iv, 0..4, vec![wait_in_a]);
    let loop_b = Event::loop_over(iv, 0..4, vec![consume_in_b]);
    let events = vec![loop_a, loop_b];

    let mut rendezvous_ids: BTreeMap<(DataId, SeqTag), usize> = BTreeMap::new();
    rendezvous_ids.insert((data, seq), 0usize);
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let mut let_at_wait: BTreeSet<DataId> = BTreeSet::new();
    let_at_wait.insert(data);

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "ring",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: &let_at_wait,
    };

    let mut out = String::new();
    let res = render_worker_events(&ctx, WorkerId(1), &events, &mut out, 0, "");
    match res {
        Err(EmitError::ContractGap(msg)) => {
            assert!(
                msg.contains("buf") && msg.contains("TASK-0364"),
                "sibling-loop guard error must name `buf` + TASK-0364; got: {msg}"
            );
        }
        other => panic!(
            "guard MUST fire on the sibling-loop shape (consumer in a \
             different loop than the Wait); got {other:?}"
        ),
    }
}

#[test]
fn two_waits_root_and_in_loop_with_root_consumer_does_not_fire() {
    // TASK-0364 EDGE pin (cycle-222 architect P3): false-positive
    // guard. `buf` has TWO Waits — one at the ROOT scope (path `[]`)
    // and one inside a loop (path `[L0]`) — and a consumer at the root
    // scope. The root Wait's `let buf` lexically dominates the root
    // consumer (`[]` is a prefix of `[]`), so the shape is SAFE and the
    // guard MUST NOT fire, even though the in-loop Wait alone would not
    // dominate the consumer. This pins that domination is satisfied by
    // ANY one Wait (the `.any()` in the rule), not ALL Waits — guarding
    // against a future tightening to `.all()` that would false-reject.
    let (names, sidecar, data, seq, iv, kernel) = at_risk_tables();

    let root_wait = Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    };
    let in_loop_wait = Event::Wait {
        src: WorkerId(0),
        data,
        tile: IterTile::empty(),
        seq,
    };
    let root_consumer = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![],
            })],
            output: None,
        },
    };
    // Root Wait, then a loop containing a second Wait, then the root
    // consumer — all of `buf`.
    let events = vec![
        root_wait,
        Event::loop_over(iv, 0..4, vec![in_loop_wait]),
        root_consumer,
    ];

    let mut rendezvous_ids: BTreeMap<(DataId, SeqTag), usize> = BTreeMap::new();
    rendezvous_ids.insert((data, seq), 0usize);
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let mut let_at_wait: BTreeSet<DataId> = BTreeSet::new();
    let_at_wait.insert(data);

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "ring",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: &let_at_wait,
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(1), &events, &mut out, 0, "")
        .expect("guard must NOT fire when a root-scope Wait dominates the root consumer");
}
