//! Pin the `reuse_widths_pending` marker emit on the
//! [`backend_common::multi_worker_walker`] code path (TASK-0273).
//!
//! ## Why this file exists
//!
//! TASK-0265 cycle 87 wired `render_reuse_marker_comment` at TWO call
//! sites:
//!   1. `nucleus/backends/pthreads-sync/src/lib.rs::render_event`
//!      (single-worker emit).
//!   2. `nucleus/backend-common/src/multi_worker_walker.rs` (the
//!      shared multi-worker emit — fed by pthreads-async,
//!      mp-tcp-bufsync, mp-tcp-event).
//!
//! The existing grep test
//! `nucleus/nucleus-compiler/tests/e2e_example_05.rs
//!  ::reuse_marker_present_on_reuse_schedule_absent_on_naive`
//! only exercises site (1) — it builds against the single-host
//! `reuse.sched.nuc` schedule. The only shipped multi-worker reuse
//! schedule (`05-stencil/distributed.sched.nuc`) is `[[skip]]`ped
//! across every backend pending TASK-0267 (host-Push synthesis drop) +
//! TASK-0268 (sync_inject barrier deadlock), so site (2) has zero
//! e2e coverage today.
//!
//! A regression that drops the marker emit from
//! `multi_worker_walker.rs` alone would silently pass `just e2e` +
//! the existing single-worker test. This file closes that gap with
//! Option B per TASK-0273 — a standalone synthetic fixture that
//! hand-builds a `WalkerCtx` + an `Event::Loop` and calls
//! `render_worker_events` directly. Decouples coverage from
//! TASK-0267/0268 closure.
//!
//! ## Test surface
//!
//! 1. `multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated`
//!    — presence + per-slot payload (iv, data, axis, length,
//!    min_offset) on the NON-strip-mine call site inside
//!    `render_worker_events_inner` (`block_tag: None` arm).
//!    Catches both "marker dropped entirely" and "marker fires but
//!    with the wrong / empty payload" regressions. Grep witness:
//!    `grep -n "render_reuse_marker_comment(" multi_worker_walker.rs`
//!    returns exactly two production sites; this test covers the
//!    `block_tag: None` arm.
//! 2. `multi_worker_walker_skips_reuse_marker_when_reuse_widths_empty`
//!    — symmetric absence on the same call site. Catches an over-
//!    eager unconditional emit.
//! 3. `multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated_under_block_tag`
//!    (TASK-0278) — same presence + payload shape on the STRIP-MINE
//!    call site inside `render_worker_events_inner` (`block_tag:
//!    Some` arm) with an enclosing tile loop. Closes the cycle-98
//!    honest-limits
//!    gap; the shipped `05-stencil/distributed.sched.nuc` carries
//!    `loop x : block=64, vectorize=8, reuse;` exercising exactly
//!    this arm live, but that cell is `[[skip]]`ped pending
//!    TASK-0267/0268.
//!
//! Together these pin both halves of the contract on BOTH walker
//! call sites: marker fires iff `sidecar.reuse_widths[iter_var]` is
//! non-empty, regardless of strip-mine / non-strip-mine arm.
//!
//! ## Forward-carry note (TASK-0270 landed cycle 104)
//!
//! Real circular-buffer codegen on the multi_worker_walker is now LIVE
//! (TASK-0270 cycle 104 — sibling of TASK-0269 pthreads-sync cycle 103).
//! The marker substring `reuse_widths_pending` is PRESERVED as the
//! first-layer regression canary above the buffer decls (its
//! parenthetical text updated to no longer be a doc-lie on the multi-
//! worker path). The new `__reuse_buf_<data>_a<axis>` + `rem_euclid(L_i64)`
//! substrings are the second-layer codegen canary — pinned by the
//! `..._emits_real_buffer_codegen` tests below for BOTH the regular arm
//! AND the strip-mine arm.
//!
//! Same lockstep instruction lives in
//! `nucleus/backends/pthreads-sync/tests/reuse_marker.rs`.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{IrBinOp, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    ArgBinding, BlockTag, DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId,
    SeqTag, WorkerId,
};
use nucleus_compiler::passes::reuse_inference::ReuseSlot;
use nucleus_compiler::sidecar::{KernelSig, LoopBound, NameSidecar};
use nucleus_compiler::NameTables;

use backend_common::multi_worker_walker::{render_worker_events, WalkerCtx};

type RendezvousIds = BTreeMap<(DataId, SeqTag), usize>;
type PairTiles = BTreeMap<(DataId, SeqTag), IterTile>;

/// Empty cross-worker maps — these tests exercise the non-strip-mine
/// `Event::Loop` body-entry call site only, no Push/Wait rendezvous.
fn empty_walker_maps() -> (RendezvousIds, PairTiles) {
    (BTreeMap::new(), BTreeMap::new())
}

/// Build a minimal `(NameTables, NameSidecar)` pair populated for one
/// source loop variable `iv`, one data symbol `d`, and one kernel
/// `k` whose signature accepts one `i64` scalar — enough that the
/// walker's `for {var} in (lo)..(hi) { ... }` header renders and a
/// `Fire` body (when present) would also render without ContractGap.
///
/// Returns the (names, sidecar) pair. Caller mutates `sidecar.
/// reuse_widths` to drive the presence / absence test arms.
fn make_minimal_tables(
    iv: IterVar,
    iv_name: &str,
    data: DataId,
    data_name: &str,
    kernel: KernelId,
    kernel_name: &str,
) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.iter_var.insert(iv, iv_name.to_string());
    names.data.insert(data, data_name.to_string());
    names.kernel.insert(kernel, kernel_name.to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.loop_bounds.insert(
        iv,
        LoopBound {
            lo: IrExpr::IntLit(0),
            hi: IrExpr::IntLit(16),
        },
    );
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I64,
                dims: vec![],
            }],
            ret: None,
        },
    );
    (names, sidecar)
}

#[test]
fn multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated() {
    // Fixture: a single non-strip-mine Event::Loop over `x : 0..16`
    // with one populated reuse slot — iv=x, data=img_in, axis=1,
    // ReuseSlot{length=3, min_offset=-1}. Mirrors what apply_reuse_
    // inference recovers for 05-stencil's distributed schedule
    // (img_in[y][x-1..=x+1] reads ⇒ length=3, min_offset=-1).
    //
    // Empty body is sufficient: the walker emits the marker comment
    // at body-entry BEFORE recursing into `body` (multi_worker_
    // walker.rs lines 464..485). A non-empty body would only add
    // ceremony unrelated to the marker contract.
    let iv = IterVar(11);
    let data = DataId(42);
    let kernel = KernelId(7);
    let (names, mut sidecar) = make_minimal_tables(iv, "x", data, "img_in", kernel, "k");

    // Populate reuse_widths[x][img_in][axis=1] = ReuseSlot{l=3, o=-1}.
    // Map shape per sidecar.rs:298 — `BTreeMap<IterVar, BTreeMap<
    // DataId, BTreeMap<u64 /* axis */, ReuseSlot>>>`. axis is u64.
    let mut per_axis: BTreeMap<u64, ReuseSlot> = BTreeMap::new();
    per_axis.insert(
        1,
        ReuseSlot {
            length: 3,
            min_offset: -1,
        },
    );
    let mut per_data: BTreeMap<DataId, BTreeMap<u64, ReuseSlot>> = BTreeMap::new();
    per_data.insert(data, per_axis);
    sidecar.reuse_widths.insert(iv, per_data);

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();

    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..16,
        body: vec![],
        block_tag: None,
        check_frame: None,
    };

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[loop_ev], &mut out, 0, "")
        .expect("synthetic loop emit must succeed");

    // PRESENCE: the load-bearing marker substring fires at least once.
    // Mirrors the single-worker e2e test's >=1 assertion shape.
    let count = out.matches("reuse_widths_pending").count();
    assert!(
        count >= 1,
        "TASK-0273: multi_worker_walker's render_reuse_marker_comment \
         call site MUST emit the `reuse_widths_pending` marker when \
         sidecar.reuse_widths[iv] is non-empty; got {count} occurrences.\n\
         If this dropped, the multi-worker arm of the marker contract \
         regressed (single-worker e2e test would still pass — that's \
         the gap this file exists to cover).\n\
         Full emit:\n{out}",
    );

    // PAYLOAD: the marker carries the iv name, data name, axis,
    // length, and min_offset. Catches a regression that fires the
    // marker but loses the per-slot payload (e.g. a refactor that
    // emits a bare `// reuse_widths_pending` with no fields).
    assert!(out.contains("iv=x"), "marker must name iv=x; got:\n{out}",);
    assert!(
        out.contains("data=img_in"),
        "marker must name data=img_in; got:\n{out}",
    );
    assert!(
        out.contains("axis=1"),
        "marker must name axis=1; got:\n{out}",
    );
    assert!(
        out.contains("length=3"),
        "marker must name length=3; got:\n{out}",
    );
    assert!(
        out.contains("min_offset=-1"),
        "marker must name min_offset=-1; got:\n{out}",
    );
}

#[test]
fn multi_worker_walker_skips_reuse_marker_when_reuse_widths_empty() {
    // Same fixture shape, but sidecar.reuse_widths left empty.
    // Defensive against an over-eager unconditional emit (e.g. a
    // refactor that always writes a `// reuse_widths_pending: (none)`
    // line regardless of whether the lookup found anything).
    let iv = IterVar(11);
    let data = DataId(42);
    let kernel = KernelId(7);
    let (names, sidecar) = make_minimal_tables(iv, "x", data, "img_in", kernel, "k");
    // sidecar.reuse_widths stays at its Default::default() empty value.

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();

    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..16,
        body: vec![],
        block_tag: None,
        check_frame: None,
    };

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[loop_ev], &mut out, 0, "")
        .expect("synthetic loop emit must succeed (empty-reuse arm)");

    // ABSENCE: zero occurrences. The empty-reuse path is the
    // shipped-byte-identical contract — every pre-cycle-87 schedule
    // (none of which carry `reuse`) must render with no marker
    // tokens whatsoever. See render.rs:848 — the function returns
    // early on `sidecar.reuse_widths.get(&iter_var) == None`.
    let count = out.matches("reuse_widths_pending").count();
    assert_eq!(
        count, 0,
        "TASK-0273: with empty sidecar.reuse_widths the multi_worker_\
         walker MUST emit zero `reuse_widths_pending` markers \
         (byte-identicality contract for non-reuse schedules); got \
         {count} occurrences.\n\
         Full emit:\n{out}",
    );
}

#[test]
fn multi_worker_walker_emits_reuse_marker_when_reuse_widths_populated_under_block_tag() {
    // TASK-0278: extend TASK-0273 coverage to the strip-mine call
    // site inside `render_worker_events_inner`'s
    // `if let Some(tag) = block_tag` branch. The cycle-98 pins above
    // only exercise the non-strip-mine arm of the same function with
    // `block_tag: None`. The shipped `05-stencil/distributed.sched.nuc`
    // carries `loop x : block=64, vectorize=8, reuse;` — exactly the
    // strip-mine-WITH-reuse shape the line-404 wiring was added for;
    // that cell is `[[skip]]`ped pending TASK-0267 + TASK-0268, so
    // this synthetic fixture is the ONLY coverage of the line-404
    // call site today.
    //
    // Fixture: outer untagged `tile : 0..4` enclosing an inner
    // strip-mined `x : 0..4` carrying BlockTag {N=4, num_full=4,
    // is_partial=false} (full nest, divides cleanly: src 0..16 /
    // block 4 = 4 tiles). Reuse populated on the INNER iv (`x`) —
    // matching the production shape where reuse rides on the
    // strip-mined inner variable.
    //
    // Mirrors the construction pattern in
    // `multi_worker_blocked_rebind.rs::rebinds_full_nest_in_loop_
    // header_and_fire_body` for BlockTag + tile_loop scaffolding.
    let src_iv = IterVar(11); // the strip-mined inner var ("x")
    let tile_iv = IterVar(20); // enclosing tile loop var
    let data = DataId(42);
    let kernel = KernelId(7);
    let (mut names, mut sidecar) = make_minimal_tables(src_iv, "x", data, "img_in", kernel, "k");

    // Add the tile_iv to NameTables (make_minimal_tables only seeded
    // the inner src_iv). `render_block_tag_loop_header` resolves the
    // enclosing tile-loop var via `names.iter_var.get(&enclosing)` so
    // a missing entry would surface as EmitError::ContractGap.
    names.iter_var.insert(tile_iv, "tile".to_string());

    // Populate reuse_widths on the INNER iv (src_iv) — the strip-mine
    // arm at line 404 calls `render_reuse_marker_comment` with the
    // inner `*iter_var`, not the enclosing tile.
    let mut per_axis: BTreeMap<u64, ReuseSlot> = BTreeMap::new();
    per_axis.insert(
        1,
        ReuseSlot {
            length: 3,
            min_offset: -1,
        },
    );
    let mut per_data: BTreeMap<DataId, BTreeMap<u64, ReuseSlot>> = BTreeMap::new();
    per_data.insert(data, per_axis);
    sidecar.reuse_widths.insert(src_iv, per_data);

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();

    // Inner strip-mined loop: 0..4, tagged full-nest, empty body
    // (marker emits at body entry BEFORE recursion). The src_iv has
    // the user-source name "x" via NameTables; the production strip-
    // mine projection reuses the source loop's IterVar id directly
    // (same name + same id), whereas this synthetic fixture only
    // models the rendering invariant (distinct ids, distinct names).
    // The marker assertion `iv=x` confirms the path that matters.
    let inner_loop = Event::Loop {
        iter_var: src_iv,
        range: 0..4,
        body: vec![],
        block_tag: Some(BlockTag {
            block_n: 4,
            num_full: 4,
            is_partial: false,
        }),
        check_frame: None,
    };

    // Enclosing tile loop: 0..4, untagged. Provides the `enclosing`
    // iv that the strip-mine rebinding consults; without it, the
    // line-404 path would fail with ContractGap("strip-mine inner
    // loop has no enclosing tile").
    let tile_loop = Event::Loop {
        iter_var: tile_iv,
        range: 0..4,
        body: vec![inner_loop],
        block_tag: None,
        check_frame: None,
    };

    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[tile_loop], &mut out, 0, "")
        .expect("synthetic strip-mine loop emit must succeed");

    // PRESENCE: the marker fires at least once on the inner-arm path.
    let count = out.matches("reuse_widths_pending").count();
    assert!(
        count >= 1,
        "TASK-0278: multi_worker_walker's strip-mine arm \
         (`render_worker_events_inner`, `block_tag: Some` branch) \
         MUST emit the `reuse_widths_pending` marker when \
         sidecar.reuse_widths[iv] \
         is non-empty and the inner loop carries block_tag=Some. Got \
         {count} occurrences.\n\
         If this dropped, the strip-mine arm of the marker contract \
         regressed (the non-strip-mine test above would still pass — \
         that's the gap THIS test exists to cover).\n\
         Full emit:\n{out}",
    );

    // PAYLOAD: same per-slot discrimination shape as the non-strip-mine
    // arm test above — iv name + data name + axis + length +
    // min_offset. A refactor that fires the marker on the strip-mine
    // path but loses the payload (e.g. forgets to thread `ctx.sidecar`
    // through render_block_tag_loop_header's child context) would
    // surface here.
    assert!(out.contains("iv=x"), "marker must name iv=x; got:\n{out}");
    assert!(
        out.contains("data=img_in"),
        "marker must name data=img_in; got:\n{out}",
    );
    assert!(
        out.contains("axis=1"),
        "marker must name axis=1; got:\n{out}",
    );
    assert!(
        out.contains("length=3"),
        "marker must name length=3; got:\n{out}",
    );
    assert!(
        out.contains("min_offset=-1"),
        "marker must name min_offset=-1; got:\n{out}",
    );
}

/// TASK-0270 cycle 104: regular (non-strip-mine) arm of the multi-
/// worker walker emits REAL circular-buffer codegen (sibling of
/// TASK-0269 cycle-103 hardening on pthreads-sync). The marker-only
/// tests above use empty bodies, so `render_reuse_buf_decls` is a no-op
/// (no DataRef to canonicalise). This test fills that gap by carrying
/// a Fire body with a reuse-axis DataRef so the discovery walker
/// succeeds and the buffer decl + rem_euclid wrap fire.
///
/// Catches a regression that drops the `render_reuse_buf_decls_pub` /
/// `render_reuse_per_iter_update_pub` calls from the regular arm of
/// `render_worker_events_inner` (the marker comment would still emit;
/// the buffer decl + index rewrite would not).
#[test]
fn multi_worker_walker_regular_arm_emits_real_buffer_codegen() {
    // Fixture mirrors `pthreads_sync_strip_mine_arm_emits_real_buffer_codegen`
    // but for the non-strip-mine arm:
    //   - single Event::Loop `x : 1..15` (no block_tag, no enclosing tile)
    //   - body Fire reads img_in[y][x + (-1)] — a reuse-axis DataRef the
    //     discovery walker can canonicalise.
    //   - reuse populated on x_iv with ReuseSlot{length=3, min_offset=-1}.
    let iv = IterVar(11);
    let data = DataId(42);
    let kernel = KernelId(7);

    let mut names = NameTables::default();
    names.iter_var.insert(iv, "x".to_string());
    names.data.insert(data, "img_in".to_string());
    names.kernel.insert(kernel, "k".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.loop_bounds.insert(
        iv,
        LoopBound {
            lo: IrExpr::IntLit(1),
            hi: IrExpr::IntLit(15),
        },
    );
    // The reuse buffer decl needs `data_type(img_in)`. 2D for the
    // reuse-axis-1 split (axis 0 outer, axis 1 inner = reuse).
    sidecar.data_types.insert(
        data,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16, 16],
        },
    );
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: None,
        },
    );

    // Populate reuse on iv: axis=1, length=3, min=-1.
    let mut per_axis: BTreeMap<u64, ReuseSlot> = BTreeMap::new();
    per_axis.insert(
        1,
        ReuseSlot {
            length: 3,
            min_offset: -1,
        },
    );
    let mut per_data: BTreeMap<DataId, BTreeMap<u64, ReuseSlot>> = BTreeMap::new();
    per_data.insert(data, per_axis);
    sidecar.reuse_widths.insert(iv, per_data);

    // Body Fire: img_in[y][x-1]. The outer axis is the free ident `y`
    // (a constant outer-coord for the inner-x walk); inner index is
    // `x + (-1)` (= the reuse-axis offset b=-1).
    let inner_idx = IrExpr::BinOp(
        IrBinOp::Add,
        Box::new(IrExpr::Ident("x".to_string())),
        Box::new(IrExpr::IntLit(-1)),
    );
    let outer_idx = IrExpr::Ident("y".to_string());
    let fire = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![outer_idx, inner_idx],
            })],
            output: None,
        },
    };

    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 1..15,
        body: vec![fire],
        block_tag: None,
        check_frame: None,
    };

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();
    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[loop_ev], &mut out, 0, "")
        .expect("synthetic regular-arm emit must succeed");

    // CODEGEN PRESENCE: buffer decl fires. Buffer name carries `_g0`
    // suffix uniformly post-TASK-0282 (single outer-axes pattern in
    // this synthetic fixture — the `[y]` tuple — so only g0 is
    // emitted).
    assert!(
        out.contains("let mut __reuse_buf_img_in_a1_g0: Vec<i32>"),
        "TASK-0270: regular arm MUST emit the buffer decl when the iv \
         carries reuse + body has a reuse-axis DataRef; got:\n{out}",
    );
    // Buffer length = ReuseSlot.length = 3.
    assert!(
        out.contains("vec![0; 3usize]"),
        "TASK-0270: regular arm MUST initialise the reuse buffer with \
         `vec![0; 3usize]` (length=3); got:\n{out}",
    );
    // Circular-wrap on the slot index.
    assert!(
        out.contains("rem_euclid(3_i64)"),
        "TASK-0270: regular arm MUST emit `rem_euclid(3_i64)` on slot \
         indexing; got:\n{out}",
    );
}

/// TASK-0270 cycle 104: strip-mine arm of the multi-worker walker
/// emits REAL circular-buffer codegen AND uses the structural
/// `strip_lo_expr` from `compute_block_tag_abs_exprs` (NOT a textual
/// `abs.replace(var, "0_i64")` step). Sibling of
/// `pthreads_sync_strip_mine_arm_emits_real_buffer_codegen` in
/// `nucleus/backends/pthreads-sync/tests/reuse_marker.rs`.
///
/// The substring overlap regression (architect P1.1 cycle 103) is the
/// MOST LOAD-BEARING assertion: the cycle-103 first landing constructed
/// the prologue's lo expression by textually replacing the inner-loop
/// var name with `"0_i64"` inside the `abs` string. When
/// `block_transform` emits the canonical tile name
/// `format!("{var}__tile")`, the inner-var name is a substring of the
/// tile-var name, so the textual replace corrupts `x__tile` into
/// `0_i64__tile`. This test pins the structural fix on the multi-worker
/// walker too.
#[test]
fn multi_worker_walker_strip_mine_arm_emits_real_buffer_codegen() {
    let src_iv = IterVar(11); // strip-mined inner var "x"
    let tile_iv = IterVar(20); // enclosing tile var "x__tile" (substring overlap)
    let data = DataId(42);
    let kernel = KernelId(7);

    let mut names = NameTables::default();
    names.iter_var.insert(src_iv, "x".to_string());
    // CRITICAL: tile name CONTAINS `x` as substring. This is the exact
    // shape `block_transform` produces, the substring overlap that
    // broke the cycle-103 first landing's textual `abs.replace(var,
    // "0_i64")` step. Mirror the pthreads-sync regression test exactly.
    names.iter_var.insert(tile_iv, "x__tile".to_string());
    names.data.insert(data, "img_in".to_string());
    names.kernel.insert(kernel, "k".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.loop_bounds.insert(
        src_iv,
        LoopBound {
            lo: IrExpr::IntLit(1),
            hi: IrExpr::IntLit(15),
        },
    );
    sidecar.data_types.insert(
        data,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16, 16],
        },
    );
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: None,
        },
    );

    let mut per_axis: BTreeMap<u64, ReuseSlot> = BTreeMap::new();
    per_axis.insert(
        1,
        ReuseSlot {
            length: 3,
            min_offset: -1,
        },
    );
    let mut per_data: BTreeMap<DataId, BTreeMap<u64, ReuseSlot>> = BTreeMap::new();
    per_data.insert(data, per_axis);
    sidecar.reuse_widths.insert(src_iv, per_data);

    // Body Fire: img_in[y][x-1].
    let inner_idx = IrExpr::BinOp(
        IrBinOp::Add,
        Box::new(IrExpr::Ident("x".to_string())),
        Box::new(IrExpr::IntLit(-1)),
    );
    let outer_idx = IrExpr::Ident("y".to_string());
    let fire = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![outer_idx, inner_idx],
            })],
            output: None,
        },
    };

    // Inner strip-mined loop: 0..4, tagged full-nest.
    let inner_loop = Event::Loop {
        iter_var: src_iv,
        range: 0..4,
        body: vec![fire],
        block_tag: Some(BlockTag {
            block_n: 4,
            num_full: 4,
            is_partial: false,
        }),
        check_frame: None,
    };

    // Enclosing tile loop: 0..4, untagged. Required so the strip-mine
    // path resolves the enclosing iv.
    let tile_loop = Event::Loop {
        iter_var: tile_iv,
        range: 0..4,
        body: vec![inner_loop],
        block_tag: None,
        check_frame: None,
    };

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();
    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[tile_loop], &mut out, 0, "")
        .expect("synthetic strip-mine arm emit must succeed");

    // CODEGEN PRESENCE: buffer decl + rem_euclid fire on the strip-
    // mine path too. Buffer name carries `_g0` suffix uniformly
    // post-TASK-0282 (single outer-axes pattern in this synthetic
    // fixture — the `[y]` tuple — so only g0 is emitted).
    assert!(
        out.contains("let mut __reuse_buf_img_in_a1_g0: Vec<i32>"),
        "TASK-0270: strip-mine arm MUST emit the buffer decl; got:\n{out}",
    );
    assert!(
        out.contains("rem_euclid(3_i64)"),
        "TASK-0270: strip-mine arm MUST emit `rem_euclid(3_i64)`; got:\n{out}",
    );

    // P1.1 NAME-OVERLAP REGRESSION (architect cycle 103 finding,
    // mirrored here): the prologue lo expression must contain
    // `x__tile` (the enclosing tile name) INTACT and must NOT contain
    // `0_i64__tile` (the artefact of the cycle-103-first-landing
    // textual replace). Both expressions (`abs` AND `strip_lo_expr`)
    // are built structurally from the same components via
    // `compute_block_tag_abs_exprs`, so this regression cannot
    // re-emerge.
    assert!(
        out.contains("x__tile"),
        "TASK-0270 P1.1 regression: the enclosing tile name `x__tile` \
         MUST appear intact in the rebound `abs` and the prologue \
         `strip_lo_expr`. If only `0_i64__tile` or `__tile` appears, \
         the textual-replace defect re-emerged on the multi-worker \
         walker. Got:\n{out}",
    );
    assert!(
        !out.contains("0_i64__tile"),
        "TASK-0270 P1.1 regression: the corrupted token `0_i64__tile` \
         (the artefact of `abs.replace(var, \"0_i64\")` when var is a \
         substring of the tile name) MUST NOT appear. Use structural \
         `strip_lo_expr` construction via `compute_block_tag_abs_exprs`. \
         Got:\n{out}",
    );
}

/// TASK-0286 (P2.1 hardening of TASK-0282): two Fire events whose
/// outer-axes are SEMANTICALLY equal but STRUCTURALLY distinct
/// (`Ident("y")` vs `Add(Ident("y"), IntLit(0))`) MUST dedupe to ONE
/// reuse group, not two.
///
/// Pre-TASK-0286 dedupe was `IrExpr::PartialEq` on the raw outer-axes
/// clones, so the `_g0` (outer=[y]) and `_g1` (outer=[y+0]) buffers
/// would both materialise. The AC#4 `<= 3` verbatim-read grep in
/// `nucleus-compiler/tests/e2e_example_05.rs` does NOT bite this:
/// over-emission grows buffer count but NOT verbatim-read count.
///
/// Today every shipped fixture emits already-canonical outer axes, so
/// canonicalisation is a no-op on every e2e cell. This test is the
/// regression pin: a future upstream affine pass that emits
/// `Add(_, IntLit(0))` in an outer-axis position would silently bloat
/// the buffer count without this canonicalisation, and bite here.
#[test]
fn multi_worker_walker_dedupes_canonical_outer_axes_add_zero() {
    use nucleus_compiler::sidecar::ConstValue;

    let iv = IterVar(11); // reuse axis: x
    let outer_iv = IterVar(20); // y — bound to provide `y` as in-scope ident
    let data = DataId(42);
    let kernel = KernelId(7);

    let mut names = NameTables::default();
    names.iter_var.insert(iv, "x".to_string());
    names.iter_var.insert(outer_iv, "y".to_string());
    names.data.insert(data, "img_in".to_string());
    names.kernel.insert(kernel, "k".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.loop_bounds.insert(
        iv,
        LoopBound {
            lo: IrExpr::IntLit(0),
            hi: IrExpr::IntLit(16),
        },
    );
    sidecar.loop_bounds.insert(
        outer_iv,
        LoopBound {
            lo: IrExpr::IntLit(0),
            hi: IrExpr::IntLit(16),
        },
    );
    sidecar.data_types.insert(
        data,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16, 16],
        },
    );
    sidecar.kernel_sigs.insert(
        kernel,
        KernelSig {
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: None,
        },
    );
    // ReuseSlot covers x-1..x+1 (length 3, min_offset -1). Two Fire
    // events below pick offsets -1 and 0, both inside the slot.
    let mut per_axis: BTreeMap<u64, ReuseSlot> = BTreeMap::new();
    per_axis.insert(
        1,
        ReuseSlot {
            length: 3,
            min_offset: -1,
        },
    );
    let mut per_data: BTreeMap<DataId, BTreeMap<u64, ReuseSlot>> = BTreeMap::new();
    per_data.insert(data, per_axis);
    sidecar.reuse_widths.insert(iv, per_data);

    // Avoid the consts-table edge case: no consts needed for the
    // canonical-form fold; affine_decompose runs against an empty
    // consts table for Ident(iv) and Ident(iv)+IntLit constant shapes.
    let _: ConstValue; // sanity import-keeper

    // Fire 1: outer-axes = [Ident("y")]. Reuse-axis index x - 1.
    let fire1 = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![
                    IrExpr::Ident("y".to_string()),
                    IrExpr::BinOp(
                        IrBinOp::Sub,
                        Box::new(IrExpr::Ident("x".to_string())),
                        Box::new(IrExpr::IntLit(1)),
                    ),
                ],
            })],
            output: None,
        },
    };
    // Fire 2: outer-axes = [Add(Ident("y"), IntLit(0))] — semantically
    // equal to Fire 1's outer, but structurally distinct pre-canonical.
    // Reuse-axis index x (offset 0).
    let fire2 = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![
                    IrExpr::BinOp(
                        IrBinOp::Add,
                        Box::new(IrExpr::Ident("y".to_string())),
                        Box::new(IrExpr::IntLit(0)),
                    ),
                    IrExpr::Ident("x".to_string()),
                ],
            })],
            output: None,
        },
    };

    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..16,
        body: vec![fire1, fire2],
        block_tag: None,
        check_frame: None,
    };

    let (rendezvous_ids, pair_tiles) = empty_walker_maps();
    let ctx = WalkerCtx {
        names: &names,
        sidecar: &sidecar,
        rendezvous_prefix: "chan",
        rendezvous_ids: &rendezvous_ids,
        pair_tiles: &pair_tiles,
        accumulate_waits: WalkerCtx::empty_accumulate_set(),
        let_at_wait_data: WalkerCtx::empty_let_at_wait_set(),
    };

    let mut out = String::new();
    render_worker_events(&ctx, WorkerId(0), &[loop_ev], &mut out, 0, "")
        .expect("synthetic canonicalisation-fixture emit must succeed");

    // POSITIVE: the `_g0` buffer fires for the (canonicalised) outer
    // axes [y].
    assert!(
        out.contains("let mut __reuse_buf_img_in_a1_g0: Vec<i32>"),
        "TASK-0286: the first (canonical) outer-axes group MUST emit \
         the `_g0` buffer; got:\n{out}",
    );
    // NEGATIVE: NO `_g1` (or higher) buffer — the second Fire's
    // `Add(y, IntLit(0))` outer-axes canonicalises to the same `y`
    // tuple as Fire 1, so the dedupe coalesces them into ONE group.
    // Pre-TASK-0286 the raw `IrExpr::PartialEq` would compare
    // `Ident(y)` vs `Add(Ident(y), IntLit(0))` as unequal and emit two
    // groups — this assertion would FAIL.
    assert!(
        !out.contains("__reuse_buf_img_in_a1_g1"),
        "TASK-0286 regression bit: outer-axes `Add(Ident(y), IntLit(0))` \
         must canonicalise to `Ident(y)` before the dedupe key is \
         taken. The emit MUST NOT contain `__reuse_buf_img_in_a1_g1` — \
         that identifier appearing here means the canonicalisation \
         silently regressed and a redundant buffer was emitted. \
         Got:\n{out}",
    );
}
