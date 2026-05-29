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
//! ## What this file establishes (OUTCOME-MATRIX branch d)
//!
//! TASK-0356 AC#2 asked for either (a) an `EmitError` contract-gap or
//! (b) a correct outer-scope `let mut` fallback. This file
//! characterizes the ACTUAL behaviour, which is NEITHER:
//!
//! - `render_worker_events` returns `Ok` (no `EmitError`) — the
//!   walker has no scope-tracking that would detect the cross-scope
//!   use. Pinned by `at_risk_shape_emits_broken_scope_no_emit_error`.
//! - The emit is the BROKEN scope: `let <name>` inside the `for`
//!   block, consumer reading `<name>` after it. Pinned, with the
//!   exact emitted string, by the same test. This broken Rust was
//!   independently confirmed non-compiling (rustc `E0425: cannot find
//!   value` … `in this scope`) during cycle-222 implementation — see
//!   the commit message for the standalone reproducer.
//!
//! The protection against this in shipped code is therefore NOT the
//! emit and NOT an `EmitError` — it is an UPSTREAM CO-LOCATION
//! INVARIANT enforced by the transfer-injection pass: every cross-
//! worker `Wait` is placed into the SAME sequence (same scope) as its
//! consuming `Operation`, immediately before it. A consumer at the
//! outer scope therefore gets its Wait at the outer scope, never in a
//! nested loop body. The at-risk shape this file hand-builds is NOT
//! producible by `inject_transfers`.
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
//! `at_risk_shape_emits_broken_scope_no_emit_error` pin documents
//! exactly what breaks and where the real fix belongs
//! (TASK-0364, filed cycle 222): make the let-at-wait classifier
//! scope-aware (exclude an in-loop Wait whose data is consumed at an
//! outer scope) OR emit a typed `EmitError` for the cross-scope use.
//! A code comment at `wait.rs` (the `let {name} = {rhs};` emit site)
//! and `collect.rs` (the loop descent) cross-references TASK-0364.
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

use backend_common::multi_worker_walker::{collect_let_at_wait_data, render_worker_events, WalkerCtx};

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
fn at_risk_events(
    data: DataId,
    seq: SeqTag,
    iv: IterVar,
    kernel: KernelId,
) -> Vec<Event> {
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
fn at_risk_shape_emits_broken_scope_no_emit_error() {
    // CHARACTERIZATION (OUTCOME-MATRIX branch d): drive
    // `render_worker_events` on the at-risk shape with `buf`
    // classified let-at-wait. Pin the ACTUAL emit.
    let (names, sidecar, data, seq, iv, kernel) = at_risk_tables();
    let events = at_risk_events(data, seq, iv, kernel);

    let mut rendezvous_ids: BTreeMap<(DataId, SeqTag), usize> = BTreeMap::new();
    rendezvous_ids.insert((data, seq), 0usize);
    let pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    let mut let_at_wait: BTreeSet<DataId> = BTreeSet::new();
    // NOTE (cycle 222 architect P3.2): this inserts `data` DIRECTLY,
    // bypassing the `collect_let_at_wait_data` classifier. Consequence
    // for the TASK-0364 fix-author: a classifier-side fix (option A —
    // exclude an in-loop Wait with an outer consumer from the set) will
    // NOT break THIS test (it forces the set membership), but WILL break
    // the sibling `classifier_includes_in_loop_whole_array_wait_for_at_risk_shape`
    // (which drives the real classifier). An emit-side fix (option B —
    // a typed EmitError) WILL break this test (the `.expect()` below
    // fails). So the two tests together cover both fix options; neither
    // alone guards both. Re-characterize both when TASK-0364 lands.
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
    // FACT 1: the walker returns Ok — there is NO EmitError for this
    // shape. AC#2(a) is NOT met (the walker has no scope-tracking).
    render_worker_events(&ctx, WorkerId(1), &events, &mut out, 0, "")
        .expect("walker emits without error for the at-risk shape (no EmitError guard)");

    // FACT 2: the `let buf = ...` declaration lands INSIDE the `for`
    // block (declare-and-assign at the Wait's scope, per the
    // let-at-wait emit). AC#2(b) is NOT met (no outer-scope `let mut`
    // fallback — the pre-init drop omitted it).
    assert!(
        out.contains("for t in (0_i64)..(4_i64) {\n    let buf = ring_0.wait();"),
        "let-at-wait emit must declare `buf` INSIDE the `for` block \
         (declare-and-assign at the Wait scope); this is the broken-\
         scope footprint; got:\n{out}"
    );

    // FACT 3: the outer consumer reads bare `buf` AFTER the loop
    // closes — out of scope relative to the in-loop `let buf`. The
    // emitted Rust does not compile (E0425, confirmed by a standalone
    // rustc reproducer during cycle-222 implementation; see commit
    // message). Pin the exact broken footprint.
    assert!(
        out.contains("}\nkernels::consume(buf);"),
        "the outer consumer must read bare `buf` AFTER the `for` block \
         closes — the use-before/out-of-scope footprint; got:\n{out}"
    );

    // ABSENCE pins: neither AC#2(a) nor AC#2(b) machinery exists. No
    // outer-scope `let mut buf` fallback was emitted (would be the
    // AC#2(b) fix), and the consumer is NOT inside the loop (which
    // would make it well-scoped but is not what the walker produces).
    assert!(
        !out.contains("let mut buf"),
        "no outer-scope `let mut buf` fallback is emitted (AC#2(b) is \
         NOT met — the per-backend pre-init drop omits it for \
         let-at-wait data); got:\n{out}"
    );

    // Whole-program footprint, pinned exactly (single source of truth
    // for what branch-d actually produces). If a FUTURE cycle lands
    // the TASK-0364 scope-aware fix, this exact-string pin will fail
    // loudly and force the fix-author to re-characterize here.
    let expected = "\
for t in (0_i64)..(4_i64) {
    let buf = ring_0.wait(); // recv `buf` from w0
}
kernels::consume(buf);
";
    assert_eq!(
        out, expected,
        "branch-d emit footprint drift: the at-risk shape no longer \
         produces the characterized broken-scope string. If this is \
         the TASK-0364 scope-aware fix landing, re-characterize this \
         test (and flip it to assert the EmitError / outer-scope \
         fallback per whichever option TASK-0364 chose)."
    );
}
