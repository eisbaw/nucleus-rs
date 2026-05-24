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
//!    min_offset). Catches both "marker dropped entirely" and "marker
//!    fires but with the wrong / empty payload" regressions.
//! 2. `multi_worker_walker_skips_reuse_marker_when_reuse_widths_empty`
//!    — symmetric absence. Catches an over-eager unconditional emit.
//!
//! Together these pin both halves of the contract: marker fires iff
//! `sidecar.reuse_widths[iter_var]` is non-empty.
//!
//! ## Forward-carry warning (next implementer of TASK-0269 / TASK-0270)
//!
//! When real circular-buffer codegen lands (TASK-0269 pthreads-sync
//! single-worker; TASK-0270 multi_worker_walker), the
//! `reuse_widths_pending` marker substring will be subsumed by actual
//! code (likely renamed `reuse_buf_decl` or removed in favour of a
//! `let __reuse_buf_<data>: Vec<...>` declaration). **Update this
//! test's expected substrings in lockstep with that emit change** —
//! do NOT silently delete the marker without replacing the assertion
//! shape with whatever the new contract is.

use std::collections::BTreeMap;

use nucleus_compiler::algo::{IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{
    ArgBinding, DataId, Event, FireBinding, IterTile, IterVar, KernelId, SeqTag, WorkerId,
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

// --------------------------------------------------------------------
// Suppress unused-import warnings for symbols referenced only via
// type signatures in the helpers above. The same shape lives in the
// sibling fixture `multi_worker_blocked_rebind.rs`.
// --------------------------------------------------------------------

#[allow(dead_code)]
fn _force_use_argbinding(_a: ArgBinding) {}
#[allow(dead_code)]
fn _force_use_firebinding(_f: FireBinding) {}
