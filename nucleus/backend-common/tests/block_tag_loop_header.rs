//! Direct unit tests for [`backend_common::multi_worker_walker::
//! render_block_tag_loop_header`] — the shared helper extracted in
//! TASK-0253 that both the multi-worker walker (pthreads-sync /
//! pthreads-async) and mp-tcp-bufsync's parallel arm now delegate to.
//!
//! The companion `multi_worker_blocked_rebind.rs` tests exercise the
//! helper TRANSITIVELY through `render_worker_events`; this file
//! pins the helper's surface CONTRACT directly so a future refactor
//! that changes either caller cannot silently shift what the helper
//! is contracted to emit. Specifically:
//!
//! 1. `header_full_nest_emits_concrete_range_and_extends_abs_subst` —
//!    the helper writes the strip-mined loop header (concrete folded
//!    range) into `out` AND returns a child `RenderCtxPub` whose
//!    `abs_subst` carries the rebound `(LO + tile*N + inner)`
//!    expression. The body recursion is the caller's job — the helper
//!    must not emit anything but the header + open brace.
//! 2. `header_partial_nest_uses_constant_base_in_abs_subst` —
//!    `is_partial == true` rebinds with `LO + num_full*N + inner` and
//!    does NOT consult the enclosing tile var.
//! 3. `missing_enclosing_tile_returns_typed_contract_gap` — full nest
//!    with no enclosing tile is a `EmitError::ContractGap` (typed
//!    error, never a panic; mirrors the single-worker path).
//! 4. `missing_loop_bounds_falls_back_to_zero_lo` — synthesised tile
//!    loops have no `sidecar.loop_bounds` entry; the helper falls
//!    back to `0_i64` as LO and the rebinding still emits a well-
//!    formed expression.

use nucleus_compiler::algo::IrExpr;
use nucleus_compiler::event::{BlockTag, IterVar};
use nucleus_compiler::sidecar::{LoopBound, NameSidecar};
use nucleus_compiler::NameTables;

use backend_common::multi_worker_walker::render_block_tag_loop_header;
use backend_common::render::{EmitError, RenderCtxPub};

/// Minimal `(NameTables, NameSidecar)` populated with one source iter
/// var (with `LO..HI` bounds) and one enclosing tile iter var (no
/// bounds entry, mirroring how `block_transform` emits the tile).
fn make_tables(
    src_iv: IterVar,
    src_name: &str,
    tile_iv: IterVar,
    tile_name: &str,
    lo: i64,
    hi: i64,
) -> (NameTables, NameSidecar) {
    let mut names = NameTables::default();
    names.iter_var.insert(src_iv, src_name.to_string());
    names.iter_var.insert(tile_iv, tile_name.to_string());

    let mut sidecar = NameSidecar::default();
    sidecar.loop_bounds.insert(
        src_iv,
        LoopBound {
            lo: IrExpr::IntLit(lo),
            hi: IrExpr::IntLit(hi),
        },
    );
    (names, sidecar)
}

#[test]
fn header_full_nest_emits_concrete_range_and_extends_abs_subst() {
    // SHAPE: enclosing tile var = "tile" (no loop_bounds entry, used
    // only by name in the rebinding); inner strip-mined var = "inner"
    // with LO=5, HI=17 in loop_bounds and a full-nest BlockTag
    // {N=4, num_full=3, is_partial=false}. Inner range 0..4.
    let src_iv = IterVar(1);
    let tile_iv = IterVar(2);
    let (names, sidecar) = make_tables(src_iv, "inner", tile_iv, "tile", 5, 17);
    let parent = RenderCtxPub::new(&names, &sidecar);
    let tag = BlockTag {
        block_n: 4,
        num_full: 3,
        is_partial: false,
    };
    let range = 0..4;

    let mut out = String::new();
    let child = render_block_tag_loop_header(
        &mut out,
        0,
        src_iv,
        &range,
        &tag,
        Some(tile_iv),
        &parent,
    )
    .expect("full-nest header emit must succeed");

    // Header line: concrete folded range, exact byte spelling. This is
    // the load-bearing format string — the cross-backend bit-identical
    // differential rests on it.
    assert_eq!(out, "for inner in (0_i64)..(4_i64) {\n");

    // The child's abs_subst MUST carry "inner" -> "(5_i64 + (tile *
    // 4_i64) + inner)" so that downstream Fire arg / index / inner-
    // bound renders see the rebound expression (NOT the bare ident).
    let rebound = child
        .abs_subst
        .get("inner")
        .expect("child abs_subst must contain rebound for `inner`");
    assert_eq!(rebound, "(5_i64 + (tile * 4_i64) + inner)");
}

#[test]
fn header_partial_nest_uses_constant_base_in_abs_subst() {
    // SHAPE: trailing partial tile, `is_partial == true`. The partial's
    // own tile loop is `0..1` so `tile*N` would always be 0; constant
    // base `LO + num_full*N + inner` is used instead. The tile var
    // name is NOT consulted (block_transform still wraps the partial
    // in its own `0..1` tile loop for structural fidelity, but the
    // partial branch reads only the tag's `num_full`).
    let src_iv = IterVar(1);
    let tile_iv = IterVar(2);
    let (names, sidecar) = make_tables(src_iv, "inner", tile_iv, "p_tile", 5, 19);
    let parent = RenderCtxPub::new(&names, &sidecar);
    let tag = BlockTag {
        block_n: 4,
        num_full: 3,
        is_partial: true,
    };
    let range = 0..2;

    let mut out = String::new();
    // `enclosing` is None here on purpose: the partial branch does NOT
    // consult `enclosing`, so the helper must succeed without it. This
    // also proves the typed-error path (test #3) is only taken on the
    // full-nest branch.
    let child = render_block_tag_loop_header(
        &mut out,
        1,
        src_iv,
        &range,
        &tag,
        None,
        &parent,
    )
    .expect("partial-nest header emit must succeed");

    // Indent 1 = 4 leading spaces.
    assert_eq!(out, "    for inner in (0_i64)..(2_i64) {\n");

    // Constant base: LO=5, num_full=3, N=4 -> `(5_i64 + (3_i64 * 4_i64)
    // + inner)`. Crucially the partial branch does NOT mention the
    // tile var "p_tile".
    let rebound = child
        .abs_subst
        .get("inner")
        .expect("child abs_subst must contain rebound for `inner`");
    assert_eq!(rebound, "(5_i64 + (3_i64 * 4_i64) + inner)");
    assert!(
        !rebound.contains("p_tile"),
        "partial branch must NOT reference the enclosing tile var; got `{rebound}`"
    );
}

#[test]
fn missing_enclosing_tile_returns_typed_contract_gap() {
    // A full-nest BlockTag (`is_partial == false`) with `enclosing ==
    // None` is a malformed EventList: block_transform always wraps a
    // full inner in its tile. The helper must fail loud with a typed
    // ContractGap (mirrors the single-worker pthreads-sync path), never
    // panic.
    let src_iv = IterVar(1);
    let tile_iv = IterVar(2);
    let (names, sidecar) = make_tables(src_iv, "inner", tile_iv, "tile", 5, 17);
    let parent = RenderCtxPub::new(&names, &sidecar);
    let tag = BlockTag {
        block_n: 4,
        num_full: 3,
        is_partial: false,
    };
    let range = 0..4;

    let mut out = String::new();
    let result = render_block_tag_loop_header(
        &mut out,
        0,
        src_iv,
        &range,
        &tag,
        None,
        &parent,
    );
    // `RenderCtxPub` does not implement Debug (its `names`/`sidecar`
    // borrows are opaque), so `.expect_err` would fail to compile —
    // pattern-match directly.
    let err = match result {
        Ok(_) => panic!("full nest with no enclosing tile must fail loud, got Ok"),
        Err(e) => e,
    };
    let msg = match &err {
        EmitError::ContractGap(s) => s.clone(),
        other => panic!("expected ContractGap, got {other:?}"),
    };
    assert!(
        msg.contains("no enclosing tile loop") && msg.contains("block_tag"),
        "expected ContractGap mentioning the missing enclosing tile; got: {msg}"
    );
    // The header must NOT be written on the error path (the caller has
    // nothing to recurse into; emitting a half-loop would corrupt the
    // generated source).
    assert!(
        out.is_empty(),
        "no header bytes should be emitted on the error path; got `{out}`"
    );
}

#[test]
fn missing_loop_bounds_falls_back_to_zero_lo() {
    // A synthesised tile loop has no `sidecar.loop_bounds` entry; the
    // helper falls back to `0_i64` as LO so the rebinding still emits
    // a well-formed expression. Same fallback the original walker /
    // mp-tcp-bufsync arms used.
    let src_iv = IterVar(1);
    let tile_iv = IterVar(2);
    let mut names = NameTables::default();
    names.iter_var.insert(src_iv, "inner".to_string());
    names.iter_var.insert(tile_iv, "tile".to_string());
    // No `sidecar.loop_bounds` entry for src_iv.
    let sidecar = NameSidecar::default();
    let parent = RenderCtxPub::new(&names, &sidecar);
    let tag = BlockTag {
        block_n: 4,
        num_full: 3,
        is_partial: false,
    };
    let range = 0..4;

    let mut out = String::new();
    let child = render_block_tag_loop_header(
        &mut out,
        0,
        src_iv,
        &range,
        &tag,
        Some(tile_iv),
        &parent,
    )
    .expect("no-loop-bounds fallback emit must succeed");

    assert_eq!(out, "for inner in (0_i64)..(4_i64) {\n");
    let rebound = child
        .abs_subst
        .get("inner")
        .expect("child abs_subst must contain rebound for `inner`");
    assert_eq!(rebound, "(0_i64 + (tile * 4_i64) + inner)");
}
