//! Pin the `reuse_widths_pending` marker emit on the pthreads-sync
//! single-worker code path (TASK-0279).
//!
//! ## Why this file exists
//!
//! TASK-0265 cycle 87 wired `render_reuse_marker_comment` at FOUR
//! production call sites total. Grep witness:
//! `grep -n render_reuse_marker_comment( <files-listed-below>` returns
//! exactly four production sites (TASK-0319 cycle-146 audit; line
//! citations below are ADVISORY-ONLY and may drift — function/file
//! anchors are the load-bearing index):
//!   1. `backend-common/src/multi_worker_walker.rs` strip-mine arm
//!      inside `render_worker_events_inner` (multi-worker — covered
//!      by TASK-0278 cycle 99).
//!   2. `backend-common/src/multi_worker_walker.rs` non-strip-mine
//!      arm inside `render_worker_events_inner` (multi-worker —
//!      covered by TASK-0273 cycle 98).
//!   3. `nucleus/backends/pthreads-sync/src/lib.rs` strip-mine arm
//!      inside `render_event` (single-worker — covered by THIS file,
//!      TASK-0279).
//!   4. `nucleus/backends/pthreads-sync/src/lib.rs` non-strip-mine
//!      arm inside `render_event` (single-worker — covered by the
//!      existing e2e grep test `e2e_example_05.rs::reuse_marker_
//!      present_on_reuse_schedule_absent_on_naive`, which builds the
//!      shipped `05-stencil/reuse.sched.nuc` carrying `loop x :
//!      reuse;` with NO `block=`, routing through site 4).
//!
//! Site 3 (the strip-mine arm in `render_event`) was THE LAST
//! UNCOVERED production marker call site. The shipped
//! `05-stencil/distributed.
//! sched.nuc` carries `loop x : block=64, vectorize=8, reuse;` —
//! exactly the strip-mine-WITH-reuse shape this arm was wired for —
//! but that cell is `[[skip]]`ped across every backend pending
//! TASK-0267 + TASK-0268. Until then, site 3 has zero e2e coverage.
//!
//! This file closes the gap with the same Option-B synthetic-fixture
//! pattern that closed sites 1 + 2 in `nucleus/backend-common/tests/
//! multi_worker_reuse_marker.rs`. Together with that file and the
//! existing `e2e_example_05.rs` grep test, ALL FOUR marker call sites
//! now have presence pins — the silent-sibling defect family
//! (cf. memory `feedback-silent-sibling-defect`) is end-to-end closed
//! for `render_reuse_marker_comment`.
//!
//! ## Forward-carry warning (next implementer of TASK-0269 / TASK-0270)
//!
//! When real circular-buffer codegen lands (TASK-0269 pthreads-sync
//! single-worker; TASK-0270 multi-worker walker), the
//! `reuse_widths_pending` marker substring will be subsumed by actual
//! code (likely renamed `reuse_buf_decl` or removed in favour of a
//! `let __reuse_buf_<data>: Vec<...>` declaration). **Update this
//! file's expected substrings in lockstep** — do NOT silently delete
//! the marker without replacing the assertion shape with whatever the
//! new contract is. Same instruction lives in
//! `nucleus/backend-common/tests/multi_worker_reuse_marker.rs`.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{IrBinOp, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    ArgBinding, BlockTag, DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId,
};
use nucleus_compiler::passes::reuse_inference::ReuseSlot;
use nucleus_compiler::sidecar::{ConstValue, KernelSig, LoopBound, NameSidecar};

use pthreads_sync::{render_single_worker_main, NameTables};

#[test]
fn pthreads_sync_emits_reuse_marker_when_reuse_widths_populated_under_block_tag() {
    // TASK-0279: pin the strip-mine call site at pthreads-sync/src/
    // lib.rs:653 (inside the `if let Some(tag) = block_tag` branch of
    // `render_event`). The non-strip-mine sibling at line 675 is
    // already covered by `e2e_example_05.rs::reuse_marker_present_on_
    // reuse_schedule_absent_on_naive` (the shipped `reuse.sched.nuc`
    // routes through it — no `block=` on its reuse iv).
    //
    // Fixture mirrors the cycle-99 `multi_worker_reuse_marker.rs`
    // strip-mine pin: outer untagged tile_loop enclosing an inner
    // Event::Loop carrying BlockTag {block_n=4, num_full=4,
    // is_partial=false}. Reuse populated on the INNER iv (the
    // strip-mined one); marker assertion confirms presence + payload.
    let src_iv = IterVar(11); // strip-mined inner var "x"
    let tile_iv = IterVar(20); // enclosing tile loop var "tile"
    let data = DataId(42);
    let kernel = KernelId(7);

    let mut names = NameTables::default();
    names.iter_var.insert(src_iv, "x".to_string());
    names.iter_var.insert(tile_iv, "tile".to_string());
    names.data.insert(data, "img_in".to_string());
    names.kernel.insert(kernel, "k".to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.loop_bounds.insert(
        src_iv,
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

    // Populate reuse_widths on the INNER iv (src_iv) — the strip-mine
    // arm at line 653 calls `render_reuse_marker_comment` with the
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

    // Inner strip-mined loop: 0..4, tagged full-nest, empty body
    // (marker emits at body entry BEFORE recursion into body).
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

    // Enclosing tile loop: 0..4, untagged. Required by the strip-mine
    // path so the rebound child RenderCtx has an enclosing iv to
    // substitute against (`render_event` line 615..639 builds the
    // `(lo_src + tile*N + var)` substitution from the enclosing iv).
    let tile_loop = Event::Loop {
        iter_var: tile_iv,
        range: 0..4,
        body: vec![inner_loop],
        block_tag: None,
        check_frame: None,
    };

    let out = render_single_worker_main(&[tile_loop], &names, &sidecar)
        .expect("synthetic strip-mine main.rs emit must succeed");

    // PRESENCE: the marker fires at least once on the inner-arm path.
    let count = out.matches("reuse_widths_pending").count();
    assert!(
        count >= 1,
        "TASK-0279: pthreads-sync's strip-mine arm \
         (pthreads-sync/src/lib.rs:653) MUST emit the \
         `reuse_widths_pending` marker when sidecar.reuse_widths[iv] \
         is non-empty and the inner loop carries block_tag=Some. Got \
         {count} occurrences.\n\
         If this dropped, the strip-mine arm of the single-worker \
         marker contract regressed (the existing e2e grep test on \
         `reuse.sched.nuc` covers the non-strip-mine sibling line \
         675 — that's the gap THIS test exists to cover).\n\
         Full emit:\n{out}",
    );

    // PAYLOAD: same per-slot discrimination as the multi-worker arm
    // tests (iv name + data name + axis + length + min_offset). A
    // refactor that fires the marker on the strip-mine path but loses
    // the payload (e.g. forgets to thread `ctx.sidecar` through the
    // rebound child RenderCtx) would surface here.
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

/// TASK-0269 cycle 103 review-hardening (architect P1.1 + P1.2): pin
/// the strip-mine arm's REAL circular-buffer codegen, with an iv name
/// that is a substring of the enclosing tile name. The cycle-103 first
/// landing constructed the prologue `lo` expression via
/// `abs.replace(var, "0_i64")` — when `block_transform` produces the
/// canonical `tile_name = format!("{var}__tile")`, the textual replace
/// corrupts the enclosing token into `0_i64__tile`, emitting broken
/// Rust. The architect P1.1 fix rebuilds the lo expression structurally.
///
/// This test would have caught that defect: the cycle-103 first landing
/// (P1.1 site `abs.replace(var.as_str(), "0_i64")`) would emit
/// `(1_i64 + (0_i64__tile * 4_i64) + 0_i64)` in the prologue source
/// index — a string this assertion forbids — while the structural fix
/// emits the correctly-named `(1_i64 + (x__tile * 4_i64) + 0_i64)`.
///
/// The cycle-103 first landing also had no direct codegen test on the
/// strip-mine arm (P1.2 — the marker test above uses an empty body, so
/// `discover_reuse_groups` returns empty and `render_reuse_buf_decls`
/// is a no-op). This test fills that gap by carrying a Fire with a
/// DataRef on the reuse axis so the discovery succeeds.
#[test]
fn pthreads_sync_strip_mine_arm_emits_real_buffer_codegen() {
    // Fixture: outer tile loop `x__tile` 0..4, inner strip-mined loop
    // `x` 0..4 carrying BlockTag{block_n=4, num_full=4, is_partial=false},
    // body Fire reads `img_in[y][x+(-1)]` — a reuse-axis DataRef the
    // discovery walker can canonicalise. (`y` is a free outer-axis ident
    // — the rewrite cut treats it as a constant outer coord, which is
    // fine for this codegen pin.)
    let src_iv = IterVar(11); // strip-mined inner var "x"
    let tile_iv = IterVar(20); // enclosing tile loop var "x__tile"
    let data = DataId(42);
    let kernel = KernelId(7);

    let mut names = NameTables::default();
    names.iter_var.insert(src_iv, "x".to_string());
    // CRITICAL: tile name CONTAINS `x` as a substring. This is the
    // exact shape `block_transform` produces (`format!("{var}__tile")`),
    // and is the substring overlap that broke the cycle-103 first
    // landing's textual `abs.replace(var, "0_i64")` step.
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

    // Populate reuse on the inner iv (src_iv): axis=1, length=3, min=-1.
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

    // Body Fire: img_in[y][x-1]. Outer-axis is the free ident `y`
    // (constant for the inner-x walk); inner index is `x + (-1)` (= the
    // reuse-axis offset b=-1, the smallest of the slot's offsets).
    // One DataRef suffices for `discover_reuse_groups` to pin the
    // canonical outer-axes pattern `[y]`.
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

    let tile_loop = Event::Loop {
        iter_var: tile_iv,
        range: 0..4,
        body: vec![inner_loop],
        block_tag: None,
        check_frame: None,
    };

    let out = render_single_worker_main(&[tile_loop], &names, &sidecar)
        .expect("synthetic strip-mine main.rs emit must succeed");

    // CODEGEN PRESENCE (P1.2 coverage gap closed):
    assert!(
        out.contains("let mut __reuse_buf_img_in_a1_g0: Vec<i32>"),
        "TASK-0269: strip-mine arm MUST emit the buffer decl when the \
         iv carries reuse + body has a reuse-axis DataRef. Buffer name \
         carries `_g0` suffix uniformly post-TASK-0282 (group_idx in \
         source-order). Got:\n{out}",
    );
    assert!(
        out.contains("rem_euclid(3_i64)"),
        "TASK-0269: strip-mine arm MUST emit the circular-buffer slot \
         expression `rem_euclid(3_i64)`; got:\n{out}",
    );

    // P1.1 NAME-OVERLAP REGRESSION: the prologue lo expression must
    // contain `x__tile` (the enclosing tile name) intact AND must NOT
    // contain `0_i64__tile` (the artefact of the cycle-103-first-landing
    // textual replace). The same applies to the per-iter update path
    // — both share the `abs` expression family.
    assert!(
        out.contains("x__tile"),
        "TASK-0269 P1.1 regression: the enclosing tile name `x__tile` \
         MUST appear intact in the rebound abs expression and the \
         prologue lo expression. If only `0_i64__tile` or `__tile` \
         appears, the textual-replace defect re-emerged; got:\n{out}",
    );
    assert!(
        !out.contains("0_i64__tile"),
        "TASK-0269 P1.1 regression: the corrupted token `0_i64__tile` \
         (the artefact of `abs.replace(var, \"0_i64\")` when var is a \
         substring of the tile name) MUST NOT appear. Use structural \
         lo-expr construction instead. Got:\n{out}",
    );
}


/// TASK-0283 cycle 105 (cross-pass agreement): the codegen rewrite
/// site must recognise an `iv + STRIDE` reuse-axis index when
/// `const STRIDE = 1` is declared in the sidecar consts table. Pre-
/// TASK-0283 the codegen had an inlined re-impl of the iv+const
/// shapes that only matched `Ident(iv) + IntLit(v)` — Ident-Ident on
/// the RHS was silently rejected. Stage 1 inference DID recognise
/// this shape (via the broader `affine_decompose`), so a `for V :
/// reuse` body reading `data[iv + STRIDE]` produced: marker fires,
/// buffer declared and filled, but body-read rewrite SKIPS — leaving
/// the raw `data[iv + STRIDE]` access verbatim. Silent codegen
/// mismatch.
///
/// Lifting `try_reuse_axis_offset` onto the shared
/// `nucleus_compiler::affine_decompose` makes this divergence
/// structurally impossible. This test pins the agreement.
#[test]
fn codegen_recognises_const_named_offset_via_affine_decompose() {
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
            lo: IrExpr::IntLit(0),
            hi: IrExpr::IntLit(16),
        },
    );
    // CRITICAL: const STRIDE = 1 in the sidecar.
    sidecar.consts.insert(
        "STRIDE".to_string(),
        ConstValue {
            ty: ScalarType::I64,
            value: 1,
        },
    );
    sidecar.data_types.insert(
        data,
        ResolvedType {
            scalar: ScalarType::I32,
            dims: vec![16],
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

    // Reuse on iv: axis=0, length=2, min_offset=0 → active offsets
    // {0, +1}. The body reads data[iv + STRIDE] (= data[iv + 1])
    // which decodes via affine_decompose + const-folding STRIDE → 1.
    // Stage 1 records offset +1; Stage 2 (post-TASK-0283) must agree.
    let mut per_axis: BTreeMap<u64, ReuseSlot> = BTreeMap::new();
    per_axis.insert(
        0,
        ReuseSlot {
            length: 2,
            min_offset: 0,
        },
    );
    let mut per_data: BTreeMap<DataId, BTreeMap<u64, ReuseSlot>> = BTreeMap::new();
    per_data.insert(data, per_axis);
    sidecar.reuse_widths.insert(iv, per_data);

    // Body Fire: data[iv + STRIDE] (Ident-Ident — the pre-cycle-105
    // narrow matcher would NOT recognise this shape).
    let inner_idx = IrExpr::BinOp(
        IrBinOp::Add,
        Box::new(IrExpr::Ident("x".to_string())),
        Box::new(IrExpr::Ident("STRIDE".to_string())),
    );
    let fire = Event::Fire {
        kernel,
        tile: IterTile::empty(),
        bindings: FireBinding {
            inputs: vec![ArgBinding::Data(DataSlice {
                data,
                indices: vec![inner_idx],
            })],
            output: None,
        },
    };

    let loop_ev = Event::Loop {
        iter_var: iv,
        range: 0..16,
        body: vec![fire],
        block_tag: None,
        check_frame: None,
    };

    let out = render_single_worker_main(&[loop_ev], &names, &sidecar)
        .expect("synthetic single-worker emit must succeed");

    // Buffer must be declared (independent of shape recognition).
    // Buffer name carries `_g0` suffix uniformly post-TASK-0282
    // (group_idx in source-order; this synthetic fixture has a single
    // outer-axes pattern — the empty tuple — so only g0 is emitted).
    assert!(
        out.contains("let mut __reuse_buf_img_in_a0_g0: Vec<i32>"),
        "TASK-0283: expected reuse buffer decl for iv x axis 0 \
         (`_g0` group); got:\n{out}",
    );

    // CRITICAL: the body-read rewrite MUST fire. Pre-TASK-0283 the
    // narrow shape matcher would have left the raw img_in[(x +
    // STRIDE)] read verbatim inside kernels::k(...).
    let kernels_call_start = out
        .find("kernels::k(")
        .expect("emit must contain kernels::k call");
    let kernels_call_end = out[kernels_call_start..]
        .find(");")
        .expect("kernels::k call must close")
        + kernels_call_start;
    let kernels_call_text = &out[kernels_call_start..kernels_call_end];
    assert!(
        kernels_call_text.contains("__reuse_buf_img_in_a0_g0["),
        "TASK-0283: kernels::k call MUST contain the rewritten reuse \
         buffer read (data[iv + STRIDE] should rewrite to \
         __reuse_buf_img_in_a0_g0[...]). Pre-TASK-0283 the narrow shape \
         matcher only handled `Ident(iv) + IntLit(v)`, so `iv + \
         STRIDE` (Ident-Ident) was silently skipped. Got:\n{kernels_call_text}\n\
         Full emit:\n{out}",
    );

    // Symmetric absence: kernels::k MUST NOT contain a raw img_in[...]
    // read on the reuse axis (the rewrite would have silently skipped).
    assert!(
        !kernels_call_text.contains("img_in["),
        "TASK-0283: kernels::k call MUST NOT contain raw img_in[...] \
         reads — they should all be rewritten to __reuse_buf_img_in_a0_g0[...]. \
         Got:\n{kernels_call_text}",
    );
}
