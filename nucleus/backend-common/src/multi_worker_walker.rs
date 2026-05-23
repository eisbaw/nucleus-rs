//! Shared multi-worker event-walker for the pthreads-sync and
//! pthreads-async backends (TASK-0239).
//!
//! # Why this module exists
//!
//! Cycle 26 (TASK-0228 Wave B-2) implemented pthreads-async multi-worker
//! emit by COPYING ~400 LoC of pthreads-sync's walker
//! (`render_worker_events`, `render_wait_assign`, `leading_axis_slice`,
//! `collect_pre_init_sets`, `collect_xfer_pairs`, `collect_worker_slots`,
//! `collect_barriers_by_tag`, `LeadingAxis`), substituting `slot_<id>`
//! for `ring_<id>` at the four Push/Wait callsites. The duplication was
//! mechanically maintainable for one cycle but every subsequent edit
//! to the walker would risk silent drift between two backends whose
//! cross-backend bit-identical differential (PRD §10.1) is the headline
//! thesis-falsifiability claim.
//!
//! TASK-0239 (this module) lifts the walker into a single source of
//! truth parameterised by ONE string: the rendezvous variable prefix
//! (`"slot"` for pthreads-sync, `"ring"` for pthreads-async). Everything
//! else (Fire / Loop / Sync / Wait gather, check_frame instrumentation,
//! per-worker partition range override, per-occurrence strip-mine
//! block_tag rebinding (TASK-0181),
//! barrier identity via `SyncTag`, slice-paste leading-axis arithmetic)
//! is shared verbatim across both backends — there is no second axis
//! of variation worth a trait abstraction.
//!
//! # What stays per-backend
//!
//! The shared walker handles only the per-worker EventList walk and the
//! Wait gather. Each backend's `Plan::emit` retains:
//!
//! - The substrate struct decl (`struct Slot<T> { Mutex+Condvar }` vs
//!   the bounded `Ring<T>` from `pthreads_async::ring_buffer::emit_ring_
//!   struct_decl`).
//! - The per-pair instance allocation (`Slot::new()` vs `Ring::new(cap)`,
//!   where cap is sidecar-derived).
//! - The `Plan` struct definition itself (the async variant carries
//!   an extra `ring_caps: BTreeMap<(DataId, SeqTag), u64>` for sizing).
//!
//! That keeps the two backends' real semantic difference (one-shot
//! rendezvous vs bounded buffered channel) visible at the `emit()`
//! entry point.
//!
//! # Design choice: direct parameter, not trait
//!
//! Option (B) from the cycle-31 plan: pass `rendezvous_prefix: &str`
//! through a small `WalkerCtx` struct. Option (A) (a `RendezvousDispatch`
//! trait) was rejected because the variation is a single string; the
//! existing `RenderCtxPub` shared-helper precedent in `lib.rs` is also
//! direct-pass. A trait would introduce dispatch ceremony for no second
//! axis.
//!
//! # SlotId == RingId == usize
//!
//! Confirmed by both backends' type alias (`type SlotId = usize` /
//! `pub(crate) type RingId = usize`). The shared map shape is
//! `BTreeMap<(DataId, SeqTag), usize>` and is reused verbatim.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use compiler::event::{DataId, Event, IterTile, IterVar, SeqTag, SyncTag, ViolationKind, WorkerId};
use compiler::NameTables;
use compiler::sidecar::NameSidecar;

use crate::check_frame::{emit_count_branch, emit_log_branch, sanitize_loop_var};
use crate::render::{
    render_const_expr_pub, render_fire_args_pub, render_fire_output_assign_pub, EmitError,
    RenderCtxPub,
};

/// Stable identifier for one rendezvous channel (slot or ring) keyed
/// by `(DataId, SeqTag)` ordered ascending. Same shape as the
/// per-backend `SlotId` / `RingId` aliases — both are `usize`, so the
/// map is shared.
pub type RendezvousId = usize;

/// Leading-axis slice descriptor for the receiver-side gather
/// (TASK-0117). Lifted from the per-backend duplicates so both
/// backends now route through one definition. The fields stay
/// crate-private — only `render_wait_assign` destructures them, and
/// it lives in this module.
pub struct LeadingAxis {
    lo: usize,
    hi: usize,
    /// Product of the data type's inner dims; per-outer-axis stride
    /// in flat-Vec elements.
    stride: usize,
}

/// Walker-time bundle of every fact the per-worker event walk needs.
///
/// Mirrors the per-backend `Plan` field set, but only the references
/// the walker actually reads — no ownership transfer, no copying. The
/// `rendezvous_prefix` field is THE ONE knob that distinguishes the
/// two backends (`"slot"` vs `"ring"`); every other field is shared
/// verbatim.
pub struct WalkerCtx<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// `"slot"` for pthreads-sync, `"ring"` for pthreads-async. Used
    /// in the four emit-string substitutions (`{prefix}_{id}.push(...)`
    /// and `{prefix}_{id}.wait()`).
    pub rendezvous_prefix: &'a str,
    /// Cross-worker Push/Wait pair -> rendezvous index. Both backends
    /// key by `(DataId, SeqTag)` and assign indices ascending.
    pub rendezvous_ids: &'a BTreeMap<(DataId, SeqTag), RendezvousId>,
    /// Per-pair iteration tile from the originating XferPlaceholder
    /// (TASK-0117). Drives the receiver-side leading-axis slice-paste
    /// in `render_wait_assign`.
    pub pair_tiles: &'a BTreeMap<(DataId, SeqTag), IterTile>,
}

impl WalkerCtx<'_> {
    /// Render context for the shared expression renderers
    /// (`render_fire_args_pub`, `render_const_expr_pub`, etc.).
    fn render_ctx(&self) -> RenderCtxPub<'_> {
        RenderCtxPub::new(self.names, self.sidecar)
    }

    /// Worker name from the reverse NameTables, falling back to
    /// `w<id>` if the table is missing the entry (defensive — should
    /// never happen for an in-Plan WorkerId).
    fn worker_name(&self, w: WorkerId) -> String {
        self.names
            .worker
            .get(&w)
            .cloned()
            .unwrap_or_else(|| format!("w{}", w.0))
    }

    /// Data name lookup; fails LOUD via [`EmitError::ContractGap`]
    /// when a DataId in the event stream has no name in the tables.
    fn data_name(&self, d: DataId) -> Result<String, EmitError> {
        self.names.data.get(&d).cloned().ok_or_else(|| {
            EmitError::ContractGap(format!("data id {d:?} has no name in NameTables"))
        })
    }
}

/// Walk one worker's EventList, emitting Rust statements into `out`.
///
/// This is the SHARED walker — both pthreads-sync's `Plan` and
/// pthreads-async's `Plan` call through it. The substitution surface
/// is exactly `ctx.rendezvous_prefix` (the variable-name prefix on
/// `{prefix}_<id>.push(...)` / `{prefix}_<id>.wait()`).
///
/// # Strip-mine rebinding (TASK-0181)
///
/// A `block_tag.is_some()` `Event::Loop` is per-occurrence absolute-
/// index rebound exactly as the single-worker path does (TASK-0180).
/// The rebinding map is threaded through `RenderCtxPub.abs_subst` so
/// it reaches every Fire arg / index / const-expr render site, not
/// just the loop header (the subtle TASK-0181 review-gate finding:
/// substituting only at the header would leave Fire body uses un-
/// rebound, re-introducing the accumulator double-count TASK-0180
/// closed). No tier-1 multi-worker schedule blocks today, so this
/// path is structurally unreachable from the e2e matrix; the rebinding
/// is exercised by unit tests in `backend-common/tests/`.
///
/// # Per-worker partition override (TASK-0212)
///
/// When the partition pass recorded a per-worker slice for THIS
/// worker on this iter var (`sidecar.partition_worker_ranges`), the
/// loop renders the concrete literal range. Otherwise the source-form
/// symbolic / literal precedence from `sidecar.loop_bounds` applies.
/// The strip-mine path above renders the concrete folded range
/// instead and skips the partition-slice check (the strip-mined inner
/// loop iterates `0..N`, not the partitioned source slice).
///
/// # check_frame defense
///
/// `check_frame.is_some() && block_tag.is_some()` is an
/// `inject_check_frames`-layer invariant violation (frames attach
/// only to outer source loops). The strip-mine path above returns
/// early before reaching this branch, so the defense is structurally
/// unreachable today, but catches a future projection-layer regression.
pub fn render_worker_events(
    ctx: &WalkerCtx<'_>,
    worker: WorkerId,
    events: &[Event],
    out: &mut String,
    indent: usize,
    prefix: &str,
) -> Result<(), EmitError> {
    let render_ctx = ctx.render_ctx();
    render_worker_events_inner(ctx, worker, events, out, indent, prefix, &render_ctx, None)
}

/// `enclosing` is the iter-var of the immediately-enclosing
/// `Event::Loop` (the tile loop, when the child is a strip-mined
/// inner-block loop with `block_tag.is_partial == false`). `None` at
/// top level. Mirrors the single-worker `render_events_in` /
/// `render_event` parameter (TASK-0180).
///
/// Eight params is one over clippy's `too_many_arguments` threshold;
/// the alternative (a single bundle struct) would be a synthetic
/// container with no semantic content — the parameters are the
/// genuine inputs to one stateless event-walk step. Local allow.
#[allow(clippy::too_many_arguments)]
fn render_worker_events_inner(
    ctx: &WalkerCtx<'_>,
    worker: WorkerId,
    events: &[Event],
    out: &mut String,
    indent: usize,
    prefix: &str,
    render_ctx: &RenderCtxPub<'_>,
    enclosing: Option<IterVar>,
) -> Result<(), EmitError> {
    let pad = "    ".repeat(indent);
    let rendezvous_prefix = ctx.rendezvous_prefix;
    for e in events {
        match e {
            Event::Fire {
                kernel, bindings, ..
            } => {
                let callee = ctx.names.kernel.get(kernel).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "kernel id {kernel:?} in a Fire has no name in NameTables"
                    ))
                })?;
                let args = render_fire_args_pub(*kernel, &bindings.inputs, render_ctx)?;
                match &bindings.output {
                    None => {
                        writeln!(out, "{pad}kernels::{callee}({args});").ok();
                    }
                    Some(o) if o.indices.is_empty() => {
                        let name = ctx.data_name(o.data)?;
                        writeln!(
                            out,
                            "{pad}let mut {name} = kernels::{callee}({args});"
                        )
                        .ok();
                    }
                    Some(o) => {
                        // TASK-0209 shared scalar-vs-sub-array
                        // classifier — both backends route through
                        // `render_fire_output_assign_pub` so the Fire-
                        // output sites cannot drift.
                        let rhs = format!("kernels::{callee}({args})");
                        let stmt = render_fire_output_assign_pub(o, &rhs, render_ctx)?;
                        writeln!(out, "{pad}{stmt}").ok();
                    }
                }
            }
            Event::Loop {
                iter_var,
                range,
                body,
                block_tag,
                check_frame,
            } => {
                let var = ctx.names.iter_var.get(iter_var).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "iter var {iter_var:?} in Event::Loop has no name in NameTables"
                    ))
                })?;

                // Per-occurrence absolute-index rebinding (TASK-0181;
                // mirrors TASK-0180 on the single-worker path).
                //
                // A strip-mined inner-block loop reuses the SOURCE iter
                // var and iterates `0..inner_len` (NOT `LO..HI`), so
                // its loop variable must be expanded to the ABSOLUTE
                // source value at every body use site. The tag is set
                // per-occurrence by `block_transform` (the only site
                // that knows N / num_full / full-vs-partial) and
                // threaded through `Event::Loop.block_tag`. The
                // `abs_subst` map lives on `RenderCtxPub` and is
                // consulted by `render_int_expr` / `render_const_expr`
                // — so Fire args, indexed assignments, and inner loop
                // bounds all see the rebound expression, not just the
                // loop header.
                //
                //   * full / divisible nest (`is_partial == false`):
                //         abs = LO + tile*N + inner
                //     where `tile` is the enclosing tile-loop variable
                //     (its iteration count is `num_full`).
                //   * trailing partial tile (`is_partial == true`):
                //         abs = LO + num_full*N + inner
                //     its own tile loop is `0..1`, so `tile*N` would
                //     be 0 (the wrong base) — the constant
                //     `num_full*N` offset is used instead.
                //
                // `LO` lives in `sidecar.loop_bounds` keyed by the
                // (reused) IterVar — single source of truth, not
                // duplicated into the tag.
                if let Some(tag) = block_tag {
                    let lo_src = ctx
                        .sidecar
                        .loop_bounds
                        .get(iter_var)
                        .map(|b| render_const_expr_pub(&b.lo, render_ctx))
                        .transpose()?
                        .unwrap_or_else(|| "0_i64".to_string());
                    let n = tag.block_n;
                    let abs = if tag.is_partial {
                        // Constant base: the partial tile's own tile
                        // loop is `0..1`, so a `tile*N` term is always
                        // 0.
                        format!("({lo_src} + ({}_i64 * {n}_i64) + {var})", tag.num_full)
                    } else {
                        // Variable base from the enclosing tile loop.
                        // A tagged full nest ALWAYS has an enclosing
                        // tile loop (block_transform emits `tile -> seq
                        // -> inner`); missing one is a malformed
                        // EventList — fail loud with context (typed
                        // error, not panic).
                        let tile_iv = enclosing.ok_or_else(|| {
                            EmitError::ContractGap(format!(
                                "strip-mined full-tile inner loop {iter_var:?} \
                                 (block_tag is_partial=false) has no enclosing tile \
                                 loop — block_transform always wraps it; malformed \
                                 EventList"
                            ))
                        })?;
                        let tile_name = ctx.names.iter_var.get(&tile_iv).ok_or_else(|| {
                            EmitError::ContractGap(format!(
                                "tile iter var {tile_iv:?} has no name in NameTables"
                            ))
                        })?;
                        format!("({lo_src} + ({tile_name} * {n}_i64) + {var})")
                    };
                    let mut child_subst = render_ctx.abs_subst.clone();
                    child_subst.insert(var.clone(), abs);
                    let child = render_ctx.with_abs_subst(child_subst);
                    // Loop header uses the concrete folded range
                    // (`{start}_i64..{end}_i64`) — NOT the source-form
                    // bound (would re-introduce the full range) and
                    // NOT the partition slice (the strip-mined inner
                    // loop iterates over the tile, not the worker's
                    // partition slice).
                    writeln!(
                        out,
                        "{pad}for {var} in ({}_i64)..({}_i64) {{",
                        range.start, range.end
                    )
                    .ok();
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        indent + 1,
                        prefix,
                        &child,
                        Some(*iter_var),
                    )?;
                    writeln!(out, "{pad}}}").ok();
                    continue;
                }

                // Defense-in-depth invariant
                // (`inject_check_frames` is contracted to populate
                // check_frame only on outer source loops; block_tag ==
                // None). The strip-mine path above returns early, so
                // reaching here with both set is a projection-layer
                // bug.
                if check_frame.is_some() && block_tag.is_some() {
                    return Err(EmitError::ContractGap(format!(
                        "Event::Loop for iter var `{var}` carries BOTH a check_frame \
                         and a block_tag — `inject_check_frames` is contracted to \
                         populate check_frame only on outer source loops; this is a \
                         projection-layer bug (TASK-0052.05 multi-worker invariant)."
                    )));
                }
                // Per-worker partition override (TASK-0212): if the
                // partition pass recorded a slice for THIS worker on
                // this iter var, render the concrete literal range.
                // The symbolic `loop_bounds` entry names the SOURCE
                // range, not the partitioned slice. A worker not
                // listed in the per-iter-var map (e.g. host, which
                // doesn't participate in partition=workers) falls
                // through to the source-form symbolic / literal
                // precedence exactly as before TASK-0212.
                let partition_slice = ctx
                    .sidecar
                    .partition_worker_ranges
                    .get(iter_var)
                    .and_then(|m| m.get(&worker));
                let (lo, hi) = match partition_slice {
                    Some(r) => (
                        format!("{}_i64", r.start),
                        format!("{}_i64", r.end),
                    ),
                    None => match ctx.sidecar.loop_bounds.get(iter_var) {
                        Some(b) => (
                            render_const_expr_pub(&b.lo, render_ctx)?,
                            render_const_expr_pub(&b.hi, render_ctx)?,
                        ),
                        None => (
                            format!("{}_i64", range.start),
                            format!("{}_i64", range.end),
                        ),
                    },
                };
                writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                let body_indent = indent + 1;
                let body_pad = "    ".repeat(body_indent);
                if let Some(frame) = check_frame {
                    // TASK-0221 defensive — `var` (NameTables) and
                    // `frame.loop_var` (CheckFrame) must name the same
                    // user-source loop variable. Dev-only assert
                    // catches future projection divergence.
                    debug_assert_eq!(
                        var.as_str(),
                        frame.loop_var.as_str(),
                        "CheckFrame.loop_var diverged from NameTables.iter_var \
                         (projection-layer bug; TASK-0221)"
                    );
                    writeln!(
                        out,
                        "{body_pad}let _check_start = std::time::Instant::now();"
                    )
                    .ok();
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        body_indent,
                        prefix,
                        render_ctx,
                        Some(*iter_var),
                    )?;
                    writeln!(
                        out,
                        "{body_pad}let _check_elapsed = _check_start.elapsed().as_nanos();"
                    )
                    .ok();
                    match frame.on_violation {
                        ViolationKind::Panic => {
                            writeln!(
                                out,
                                "{body_pad}if _check_elapsed > {ns}_u128 {{ panic!(\"latency budget violated on `check loop {lv}`: iteration took {{}} ns, max {ns} ns\", _check_elapsed); }}",
                                ns = frame.latency_max_ns,
                                lv = frame.loop_var,
                            )
                            .ok();
                        }
                        ViolationKind::Log => {
                            // TASK-0222: shared template — see emit_log_branch.
                            emit_log_branch(
                                out,
                                &body_pad,
                                &frame.loop_var,
                                frame.latency_max_ns,
                            );
                        }
                        ViolationKind::Count => {
                            // TASK-0222: shared template — see emit_count_branch.
                            let id = sanitize_loop_var(&frame.loop_var);
                            emit_count_branch(out, &body_pad, &id, frame.latency_max_ns);
                        }
                    }
                } else {
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        indent + 1,
                        prefix,
                        render_ctx,
                        Some(*iter_var),
                    )?;
                }
                writeln!(out, "{pad}}}").ok();
            }
            Event::Sync { sync, .. } => {
                // Barrier identity is the contract-carried SyncTag
                // (TASK-0172): every participant of this barrier
                // carries the same tag, so all participants .wait()
                // on the same `bar_<tag>` with no pre-order-index
                // recovery.
                let bid = sync.0;
                writeln!(out, "{pad}{prefix}bar_{bid}.wait();").ok();
            }
            Event::Push { data, dst, seq, .. } => {
                let rid = ctx.rendezvous_ids.get(&(*data, *seq)).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "Push of data {data:?} (seq {seq:?}) has no rendezvous id \
                         (not collected as cross-worker)"
                    ))
                })?;
                let name = ctx.data_name(*data)?;
                let to = ctx.worker_name(*dst);
                writeln!(
                    out,
                    "{pad}{prefix}{rendezvous_prefix}_{rid}.push({name}.clone()); // send `{name}` to {to}",
                )
                .ok();
            }
            Event::Wait {
                data, src, seq, ..
            } => {
                let rid = ctx.rendezvous_ids.get(&(*data, *seq)).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "Wait of data {data:?} (seq {seq:?}) has no rendezvous id \
                         (not collected as cross-worker)"
                    ))
                })?;
                let name = ctx.data_name(*data)?;
                let from = ctx.worker_name(*src);
                // TASK-0117 host-side gather: see render_wait_assign.
                let assign = render_wait_assign(
                    ctx,
                    &name,
                    *data,
                    *seq,
                    &format!("{prefix}{rendezvous_prefix}_{rid}.wait()"),
                )?;
                writeln!(out, "{pad}{assign} // recv `{name}` from {from}",).ok();
            }
            Event::Alloc { .. } | Event::Free { .. } => {
                // RAII Vec storage; no explicit reservation.
            }
        }
    }
    Ok(())
}

/// Render the receiver-side assignment statement for one Wait event.
/// Returns one statement (no trailing newline).
///
/// Two shapes:
/// - **Whole-array assign** (`name = <rhs>;`) — the pre-TASK-0117
///   single-pair behaviour. Selected when the pair's tile is empty
///   (no enclosing iteration nest, e.g. a top-level load_input ⇒ host
///   transfer), OR when the leading axis of the tile covers the
///   data's full leading-axis range (i.e. the producer sent the whole
///   array on this pair).
/// - **Slice-paste** (`{ let _tmp = <rhs>; name[lo..hi]
///   .copy_from_slice(&_tmp[lo..hi]); }`) — TASK-0117 host-side
///   gather. Selected when the tile's outer axis is a strict
///   sub-range of the data's leading axis. The producer pushed its
///   whole local buffer with only its tile-slice populated; the
///   receiver copies that slice into its own whole buffer. The
///   byte/element stride per outer-axis element is the product of
///   the data's inner dims.
pub fn render_wait_assign(
    ctx: &WalkerCtx<'_>,
    name: &str,
    data: DataId,
    seq: SeqTag,
    rhs: &str,
) -> Result<String, EmitError> {
    let slice = match ctx.pair_tiles.get(&(data, seq)) {
        Some(tile) => leading_axis_slice(ctx, data, tile)?,
        None => None,
    };
    match slice {
        None => {
            // Empty tile (or no shape match) — whole-array assign.
            Ok(format!("{name} = {rhs};"))
        }
        Some(LeadingAxis { lo, hi, stride }) => {
            // Slice-paste: receiver-side gather half of TASK-0117.
            let lo_off = lo.saturating_mul(stride);
            let hi_off = hi.saturating_mul(stride);
            Ok(format!(
                "{{ let _tmp = {rhs}; \
                 {name}[{lo_off}usize..{hi_off}usize].copy_from_slice(\
                 &_tmp[{lo_off}usize..{hi_off}usize]); }}"
            ))
        }
    }
}

/// Compute the leading-axis slice for a Wait's tile.
///
/// Returns `Some(LeadingAxis { lo, hi, stride })` when the tile's
/// outer axis is a strict sub-range of the data's leading axis (the
/// slice-paste path), `None` when the tile is empty or the outer
/// axis covers the full source range (the whole-array path).
///
/// Returns `Err` on a shape mismatch — e.g. the tile's leading axis
/// range exceeds the data's leading-dim length; a compiler-pass
/// invariant violation worth failing loud rather than silently
/// emitting an out-of-bounds slice.
///
/// Module-private — both backends consume this only indirectly via
/// `render_wait_assign`.
///
/// # HONEST-PARTIAL ASSUMPTION (TASK-0117 cycle-1 review-gate)
///
/// Assumes `tile.bounds[0].iter_var` maps to the DATA's leading dim
/// (axis 0). The `_iv` is not consulted — only the numerical range
/// is validated. For `partition=workers` schedules whose loop var is
/// the leading-axis index (the in-tree case), this holds. For a
/// hypothetical inner-axis partition, the slice would silently
/// address the wrong axis. Tracked as a honest-limit in TASK-0117.
fn leading_axis_slice(
    ctx: &WalkerCtx<'_>,
    data: DataId,
    tile: &IterTile,
) -> Result<Option<LeadingAxis>, EmitError> {
    // Empty tile -> no per-axis slicing.
    let Some((_iv, range)) = tile.bounds.first() else {
        return Ok(None);
    };
    let ty = ctx.sidecar.data_type(data).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "Wait of data {data:?} has no ResolvedType in NameSidecar"
        ))
    })?;
    // Scalar data: no slice axes — whole-value transfer.
    if ty.dims.is_empty() {
        return Ok(None);
    }
    let leading_dim = ty.dims[0] as i64;
    // Pre-TASK-0117 single-pair: tile covers the full source range
    // of the leading axis (0..B). No slicing.
    if range.start == 0 && range.end == leading_dim {
        return Ok(None);
    }
    if range.start < 0 || range.end > leading_dim || range.start >= range.end {
        return Err(EmitError::ContractGap(format!(
            "Wait of data {data:?}: tile leading-axis range {:?} out of \
             bounds for data dims {:?} (leading-dim {})",
            range, ty.dims, leading_dim
        )));
    }
    let stride: usize = ty.dims[1..].iter().product();
    Ok(Some(LeadingAxis {
        lo: range.start as usize,
        hi: range.end as usize,
        stride,
    }))
}

// --------------------------------------------------------------------
// Event-walk helpers (recurse into Event::Loop bodies)
// --------------------------------------------------------------------

/// Collect every `(DataId, SeqTag)` pair appearing on a Push or Wait
/// event in `events` (descending into Loop bodies). The map's value is
/// the pair's tile, copied from the first event sighting; the same
/// `seq` is carried on both endpoints by the XferPlaceholder
/// construction (TASK-0018) so first-sighting is well-defined.
pub fn collect_xfer_pairs(
    events: &[Event],
    out: &mut BTreeMap<(DataId, SeqTag), IterTile>,
) {
    for e in events {
        match e {
            Event::Push {
                data, seq, tile, ..
            }
            | Event::Wait {
                data, seq, tile, ..
            } => {
                out.entry((*data, *seq))
                    .or_insert_with(|| tile.clone());
            }
            Event::Loop { body, .. } => collect_xfer_pairs(body, out),
            _ => {}
        }
    }
}

/// Per-worker visit of Push/Wait events to collect the worker's
/// rendezvous-id touch set. Descends into `Event::Loop` bodies.
///
/// Replaces the per-backend `collect_worker_slots` /
/// `collect_worker_rings` — both walked identically, only the value
/// type alias differed (`SlotId = RingId = usize`).
pub fn collect_worker_rendezvous(
    events: &[Event],
    ids: &BTreeMap<(DataId, SeqTag), RendezvousId>,
    out: &mut BTreeSet<RendezvousId>,
) {
    for e in events {
        match e {
            Event::Push { data, seq, .. } | Event::Wait { data, seq, .. } => {
                if let Some(s) = ids.get(&(*data, *seq)) {
                    out.insert(*s);
                }
            }
            Event::Loop { body, .. } => collect_worker_rendezvous(body, ids, out),
            _ => {}
        }
    }
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index,
/// no fallibility (every tag is an independent barrier, so there is
/// nothing to validate / reject here any more).
pub fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
where
    F: FnMut(SyncTag, &BTreeSet<WorkerId>),
{
    for e in events {
        match e {
            Event::Sync {
                participants, sync, ..
            } => f(*sync, participants),
            Event::Loop { body, .. } => collect_barriers_by_tag(body, f),
            _ => {}
        }
    }
}

/// Visit every `Event::Wait` / `Event::Fire` output to build the
/// three sets needed for the pre-init computation:
///
/// - `waited`: cross-worker inputs the worker WAITs on (these will
///   be overwritten by the .wait() and need to exist as locals).
/// - `whole`: data the worker writes via a whole-array Fire output
///   (let-bound at the Fire site; no pre-init needed).
/// - `indexed`: data the worker writes via an indexed Fire output
///   (must be pre-initialised so the indexed assign has something to
///   write into).
///
/// A worker's pre-init set is `waited UNION (indexed - whole)`.
pub fn collect_pre_init_sets(
    events: &[Event],
    waited: &mut BTreeSet<DataId>,
    whole: &mut BTreeSet<DataId>,
    indexed: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, .. } => {
                waited.insert(*data);
            }
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    if o.indices.is_empty() {
                        whole.insert(o.data);
                    } else {
                        indexed.insert(o.data);
                    }
                }
            }
            Event::Loop { body, .. } => collect_pre_init_sets(body, waited, whole, indexed),
            _ => {}
        }
    }
}
