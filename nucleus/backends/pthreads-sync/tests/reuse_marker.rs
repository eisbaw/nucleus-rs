//! Pin the `reuse_widths_pending` marker emit on the pthreads-sync
//! single-worker code path (TASK-0279).
//!
//! ## Why this file exists
//!
//! TASK-0265 cycle 87 wired `render_reuse_marker_comment` at FOUR
//! production call sites total:
//!   1. `backend-common/src/multi_worker_walker.rs:404` (strip-mine
//!      arm, multi-worker — covered by TASK-0278 cycle 99).
//!   2. `backend-common/src/multi_worker_walker.rs:478` (non-strip-
//!      mine arm, multi-worker — covered by TASK-0273 cycle 98).
//!   3. `nucleus/backends/pthreads-sync/src/lib.rs:653` (strip-mine
//!      arm, single-worker — covered by THIS file, TASK-0279).
//!   4. `nucleus/backends/pthreads-sync/src/lib.rs:675` (non-strip-
//!      mine arm, single-worker — covered by the existing e2e grep
//!      test `e2e_example_05.rs::reuse_marker_present_on_reuse_
//!      schedule_absent_on_naive`, which builds the shipped
//!      `05-stencil/reuse.sched.nuc` carrying `loop x : reuse;` with
//!      NO `block=`, routing through site 4).
//!
//! Site 3 (the strip-mine arm at line 653) was THE LAST UNCOVERED
//! production marker call site. The shipped `05-stencil/distributed.
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

use nucleus_compiler::algo::{IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{BlockTag, DataId, Event, IterVar, KernelId};
use nucleus_compiler::passes::reuse_inference::ReuseSlot;
use nucleus_compiler::sidecar::{KernelSig, LoopBound, NameSidecar};

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
