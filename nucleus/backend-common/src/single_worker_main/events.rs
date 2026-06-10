//! Single-worker Event-rendering tree shared by every tier-1 backend:
//! `render_events` / `render_events_in` / `render_event`. Originally
//! carved out of `pthreads-sync` `lib.rs` per TASK-0437.01; relocated
//! here in TASK-0455.11 (the last inter-backend arrow) — no logic
//! changed, the emitted bytes are byte-identical.
//!
//! `render_events` and `render_event` are re-exported `pub(crate)` from
//! the parent module (`single_worker_main`) so the `break_loop`
//! sibling keeps resolving them via `super::render_events` /
//! `super::render_event`. `render_events_in` is only used inside this
//! module, so it stays private here.

use std::fmt::Write as _;

use nucleus_compiler::event::{Event, IterVar, ViolationKind};

use crate::check_frame::{emit_count_branch, emit_log_branch, sanitize_loop_var};
use crate::render::EmitError;
use crate::render::{
    data_name, render_const_expr, render_fire_args, render_fire_output_assign, render_loop_bounds,
    render_reuse_buf_decls, render_reuse_marker_comment, render_reuse_per_iter_update, RenderCtx,
};

use super::break_loop;

pub(crate) fn render_events(
    events: &[Event],
    out: &mut String,
    indent: usize,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    render_events_in(events, out, indent, ctx, None)
}

/// `enclosing` is the iter-var of the immediately-enclosing
/// `Event::Loop` (the tile loop, when the child is a strip-mined
/// inner-block loop) — `None` at top level.
fn render_events_in(
    events: &[Event],
    out: &mut String,
    indent: usize,
    ctx: &RenderCtx<'_>,
    enclosing: Option<IterVar>,
) -> Result<(), EmitError> {
    for e in events {
        render_event(e, out, indent, ctx, enclosing)?;
    }
    Ok(())
}

pub(crate) fn render_event(
    event: &Event,
    out: &mut String,
    indent: usize,
    ctx: &RenderCtx<'_>,
    enclosing: Option<IterVar>,
) -> Result<(), EmitError> {
    let pad = "    ".repeat(indent);
    match event {
        Event::Fire {
            kernel, bindings, ..
        } => {
            let callee = ctx.names.kernel.get(kernel).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "kernel id {kernel:?} in a Fire has no name in NameTables"
                ))
            })?;
            let rendered_args = render_fire_args(*kernel, &bindings.inputs, ctx)?;
            match &bindings.output {
                None => {
                    // Effect statement.
                    writeln!(out, "{pad}kernels::{callee}({rendered_args});").ok();
                }
                Some(o) if o.indices.is_empty() => {
                    // Whole-array (or scalar) binding.
                    let name = data_name(o.data, ctx)?;
                    writeln!(
                        out,
                        "{pad}let mut {name} = kernels::{callee}({rendered_args});"
                    )
                    .ok();
                }
                Some(o) => {
                    // Indexed assignment. Pre-init guaranteed the
                    // data exists as a flat Vec<T>. Classify scalar
                    // vs partial sub-array (TASK-0209): a full-rank
                    // LHS writes a single slot; a partial-rank LHS
                    // (e.g. `feat1[n] <-- conv_block_1(input[n])` on
                    // a rank-4 `feat1`) writes a contiguous trailing
                    // sub-array via `copy_from_slice`.
                    let rhs = format!("kernels::{callee}({rendered_args})");
                    let stmt = render_fire_output_assign(o, &rhs, ctx)?;
                    writeln!(out, "{pad}{stmt}").ok();
                }
            }
            Ok(())
        }
        Event::Loop {
            iter_var,
            range,
            body,
            block_tag,
            check_frame,
            break_cond,
        } => {
            let var = ctx.names.iter_var.get(iter_var).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "iter var {iter_var:?} in an Event::Loop has no name in NameTables"
                ))
            })?;

            // Absolute-index rebinding (TASK-0180, root-cause fix).
            //
            // A strip-mined inner-block loop reuses the SOURCE iter
            // var and iterates `0..inner_len` (NOT `LO..HI`), so its
            // loop variable must be expanded to the ABSOLUTE source
            // value at every body use site. Whether — and HOW — to
            // rebind is now read PER-OCCURRENCE from
            // `Event::Loop.block_tag` (set by `block_transform`, the
            // only site that knows `N`/`num_full`/the full-vs-partial
            // split), NOT from a program-global EventList occurrence
            // count. The old `divisible_inner_block_vars` (`counts==1`)
            // heuristic conflated three cases sharing one reused
            // IterVar and silently dropped a loop-var name reused
            // across N evenly-divisible passes (04-prefix-sum/blocked
            // accumulator double-count). The tag is per-occurrence so
            // each of the N passes — and the full vs trailing-partial
            // tile — rebinds independently and correctly.
            //
            //   * full / divisible nest (`is_partial == false`):
            //         abs = LO + tile*N + inner
            //     where `tile` is the enclosing tile-loop variable
            //     (its iteration count is `num_full`).
            //   * trailing partial tile (`is_partial == true`):
            //         abs = LO + num_full*N + inner
            //     its own tile loop is `0..1`, so `tile*N` would be 0
            //     (the wrong base) — the constant `num_full*N` offset
            //     is used instead. This also gives TASK-0173 exactly
            //     its AC#1 (per-tile-nest base offset / partial marker
            //     / N + num_full); non-divisible accumulators are now
            //     rebound correctly too.
            //
            // `LO` (source lower bound) is the same for every reused
            // occurrence and lives in `sidecar.loop_bounds` keyed by
            // the (reused) IterVar — single source of truth, not
            // duplicated into the tag.
            if let Some(tag) = block_tag {
                // A `for..until` early-exit predicate (TASK-0341.02.01.05.04)
                // is a SOURCE-loop fact; a strip-mined inner loop is
                // compiler machinery (`block_transform`) and the projection
                // (`petri_to_events`) only ever sets `break_cond` from the
                // source `Repeat`, which is never tagged. A tagged loop
                // carrying a break predicate is therefore a projection-layer
                // bug — fail loud rather than silently dropping the break
                // (the strip-mined arm below returns early without an emit
                // site for it). Mirrors the check_frame+block_tag guard
                // further down.
                if break_cond.is_some() {
                    return Err(EmitError::ContractGap(format!(
                        "Event::Loop {{ iter_var: {iter_var:?} }} carries BOTH a \
                         break_cond (for..until predicate) and a block_tag — \
                         `petri_to_events` only projects break_cond from the \
                         untagged SOURCE Repeat, so a tagged loop must never \
                         carry one. This is a projection-layer bug \
                         (TASK-0341.02.01.05.04 invariant)."
                    )));
                }
                let lo_src = ctx
                    .sidecar
                    .loop_bounds
                    .get(iter_var)
                    .map(|b| render_const_expr(&b.lo, ctx))
                    .transpose()?
                    .unwrap_or_else(|| "0_i64".to_string());
                let n = tag.block_n;
                // Build `abs` (the rebound absolute iv expression at
                // body sites) AND its `iv=0` counterpart `strip_lo_expr`
                // (the absolute coordinate of the strip-mined loop's
                // first iteration, used by the reuse-prologue) from the
                // SAME structural components. The previous shape used
                // `abs.replace(var, "0_i64")` to derive the prologue lo
                // — that is unsafe whenever `var` is a substring of the
                // sibling `tile_name`: `block_transform` constructs
                // `tile_name = format!("{var}__tile")`, so for iv="x"
                // the `abs.replace("x", "0_i64")` step corrupted the
                // enclosing `x__tile` token into `0_i64__tile` (review
                // P1.1, cycle 103 architect NO-GO). Structural
                // construction is safe regardless of name overlap and
                // keeps the two expressions trivially consistent.
                let (abs, strip_lo_expr) = if tag.is_partial {
                    // Constant base: the partial tile's own tile loop
                    // is `0..1`, so a `tile*N` term is always 0.
                    (
                        format!("({lo_src} + ({}_i64 * {n}_i64) + {var})", tag.num_full),
                        format!("({lo_src} + ({}_i64 * {n}_i64) + 0_i64)", tag.num_full),
                    )
                } else {
                    // Variable base from the enclosing tile loop. A
                    // tagged full nest ALWAYS has an enclosing tile
                    // loop (block_transform emits `tile -> seq ->
                    // inner`); a missing one is a malformed EventList —
                    // fail loud with context (typed error, not panic).
                    let tile_iv = enclosing.ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "strip-mined full-tile inner loop {iter_var:?} (block_tag \
                             is_partial=false) has no enclosing tile loop — \
                             block_transform always wraps it; malformed EventList"
                        ))
                    })?;
                    let tile_name = ctx.names.iter_var.get(&tile_iv).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "tile iter var {tile_iv:?} has no name in NameTables"
                        ))
                    })?;
                    (
                        format!("({lo_src} + ({tile_name} * {n}_i64) + {var})"),
                        format!("({lo_src} + ({tile_name} * {n}_i64) + 0_i64)"),
                    )
                };
                let mut child_subst = ctx.abs_subst.clone();
                child_subst.insert(var.clone(), abs.clone());
                // TASK-0269 strip-mined arm: a strip-mined inner loop
                // CAN carry reuse (`loop x : block=64, reuse;` —
                // 05-stencil/distributed). The buffer decl + prologue
                // lives at the OUTER pad above the for header (so it
                // persists across the inner-loop's iterations). For
                // the prologue's reuse-axis "lo" we use the rebound
                // ABSOLUTE expression at iv=0 (`LO + tile*N + 0`)
                // because the strip-mined loop's lexical range is
                // `0..inner_len`, not `LO..HI`.
                let reuse_groups =
                    render_reuse_buf_decls(out, indent, *iter_var, var, &strip_lo_expr, body, ctx)?;
                let mut child_reuse = ctx.reuse_active.clone();
                for (data_id, gs) in reuse_groups.clone() {
                    child_reuse.insert(data_id, gs);
                }
                let child = RenderCtx {
                    names: ctx.names,
                    sidecar: ctx.sidecar,
                    abs_subst: child_subst,
                    reuse_active: child_reuse,
                };
                // Loop header uses the concrete folded range
                // (`{start}_i64..{end}_i64`) — NOT the source-form
                // bound, which would re-introduce the full range.
                writeln!(
                    out,
                    "{pad}for {var} in ({}_i64)..({}_i64) {{",
                    range.start, range.end
                )
                .ok();
                // TASK-0265 Tier 1: strip-mined inner loop CAN carry
                // reuse (e.g. `loop x : block=64, reuse;`). Emit the
                // marker comment at body entry with the rebound child
                // RenderCtx — matches the non-tagged path below.
                render_reuse_marker_comment(
                    out,
                    indent + 1,
                    *iter_var,
                    var,
                    ctx.sidecar,
                    ctx.names,
                );
                // TASK-0269 per-iter update: the iv expression here
                // is the rebound ABSOLUTE expression (so the source-
                // array index reflects the strip-mined coordinate),
                // not the bare `var`.
                render_reuse_per_iter_update(out, indent + 1, &reuse_groups, &abs, &child)?;
                render_events_in(body, out, indent + 1, &child, Some(*iter_var))?;
                writeln!(out, "{pad}}}").ok();
                return Ok(());
            }

            let (lo_s, hi_s) = render_loop_bounds(*iter_var, range, ctx)?;
            // TASK-0269 cycle 103: real circular-buffer codegen.
            // ORDER: buffer decl + initial-fill prologue MUST live
            // OUTSIDE the for-header (the buffer must persist across
            // iterations), so we emit them BEFORE writing the for
            // line. `render_reuse_buf_decls` walks the body to
            // discover EVERY unique outer-axes pattern per (data,
            // axis) (TASK-0282 multi-outer-coord generalisation),
            // emits one Vec<T> decl + unrolled prologue PER GROUP, and
            // returns the per-DataId Vec<ReuseRewriteGroup> the child
            // RenderCtx threads into the body recursion. Empty when
            // the iv carries no reuse slot — byte-identical no-op for
            // every pre-TASK-0269 schedule.
            let reuse_groups =
                render_reuse_buf_decls(out, indent, *iter_var, var, &lo_s, body, ctx)?;
            writeln!(out, "{pad}for {var} in ({lo_s})..({hi_s}) {{").ok();
            // TASK-0265 Tier 1: regular (non-strip-mined) loop —
            // marker comment at body entry. The substring
            // `reuse_widths_pending` is grep-able for AC#4 of the
            // parent task. NO-OP when the iv carries no reuse.
            let body_indent_for_marker = indent + 1;
            render_reuse_marker_comment(
                out,
                body_indent_for_marker,
                *iter_var,
                var,
                ctx.sidecar,
                ctx.names,
            );
            // TASK-0269 per-iter update: load the most-distant
            // element into the buffer slot before any Fire arg reads
            // it. Iv expression here is the bare var (no abs_subst
            // rebinding on this non-strip-mine path).
            render_reuse_per_iter_update(out, body_indent_for_marker, &reuse_groups, var, ctx)?;
            // Real-time `check loop V : latency_max=T` (TASK-0052.02 /
            // PRD §6.3.5). The projection pass `inject_check_frames`
            // populates `check_frame` ONLY on outer source loops
            // (`block_tag == None`), and the strip-mined / tagged path
            // above returns early — so reaching here with a tagged loop
            // and a `check_frame` would be a projection-layer bug.
            // Defend the invariant rather than silently dropping the
            // assertion.
            if check_frame.is_some() && block_tag.is_some() {
                return Err(EmitError::ContractGap(format!(
                    "Event::Loop {{ iter_var: {iter_var:?} }} carries BOTH a \
                     check_frame and a block_tag — `inject_check_frames` is \
                     contracted to populate check_frame only on outer source \
                     loops (block_tag == None). This is a projection-layer \
                     bug (TASK-0052.02 invariant)."
                )));
            }
            let body_indent = indent + 1;
            let body_pad = "    ".repeat(body_indent);
            // TASK-0269: build the child RenderCtx that carries the
            // newly-active reuse groups into the body recursion. The
            // parent's reuse_active is preserved (so nested reuse
            // loops compose); new groups OVERRIDE on data_id collision
            // (a hypothetical inner-loop reuse on the SAME data; not
            // exercised by 05-stencil/reuse but the BTreeMap semantics
            // are well-defined).
            let mut child_reuse = ctx.reuse_active.clone();
            for (data_id, gs) in reuse_groups {
                child_reuse.insert(data_id, gs);
            }
            let body_ctx = RenderCtx {
                names: ctx.names,
                sidecar: ctx.sidecar,
                abs_subst: ctx.abs_subst.clone(),
                reuse_active: child_reuse,
            };
            if let Some(frame) = check_frame {
                // TASK-0221 (a): CheckFrame.loop_var carries the
                // user-source loop variable name, but `var` (resolved
                // from NameTables) is the authoritative source of the
                // same identifier at emit time. Defensive assert in
                // dev builds catches any future projection that
                // diverges the two; release builds skip the check
                // (no perf or behaviour change on the codegen path).
                debug_assert_eq!(
                    var.as_str(),
                    frame.loop_var.as_str(),
                    "CheckFrame.loop_var diverged from NameTables.iter_var \
                     (projection-layer bug — both should name the same \
                     user-source loop variable; TASK-0221)"
                );
                // Tier-1 clock: std::time::Instant. PRD §6.3.5 names
                // this for backends "where Instant is available";
                // pthreads-sync runs hosted on a real OS so this is
                // free. Determinism: the success-path emitted BYTES
                // are unchanged (`_check_start` is computed and
                // consumed locally, never written to stdout). The
                // panic message on violation is the only behavioural
                // difference, and panic terminates with rustc's
                // standard exit code 101 — the cross-backend
                // differential treats "exit 101 + empty stdout" as a
                // clean assertion signal, not a corrupt-output false
                // positive.
                writeln!(
                    out,
                    "{body_pad}let _check_start = std::time::Instant::now();"
                )
                .ok();
                render_events_in(body, out, body_indent, &body_ctx, Some(*iter_var))?;
                writeln!(
                    out,
                    "{body_pad}let _check_elapsed = _check_start.elapsed().as_nanos();"
                )
                .ok();
                match frame.on_violation {
                    ViolationKind::Panic => {
                        // `as u128` widen of `latency_max_ns: u64` keeps
                        // the comparison total-ordered (Instant::elapsed
                        // returns u128). The panic message embeds:
                        //   1. loop_var name (from the user's directive)
                        //   2. measured ns (runtime value)
                        //   3. threshold ns (compile-time literal)
                        // — AC#3 requires all three.
                        writeln!(
                            out,
                            "{body_pad}if _check_elapsed > {ns}_u128 {{ panic!(\"latency budget violated on `check loop {lv}`: iteration took {{}} ns, max {ns} ns\", _check_elapsed); }}",
                            ns = frame.latency_max_ns,
                            lv = frame.loop_var,
                        )
                        .ok();
                    }
                    ViolationKind::Log => {
                        // TASK-0052.04. eprintln-once per violation;
                        // execution continues. Stderr-only (the
                        // cross-backend differential compares stdout /
                        // output.bin), so this stays determinism-safe
                        // on the success-path bytes. The runtime SHAPE
                        // of when this fires is non-deterministic
                        // (clock-dependent), but that does not perturb
                        // the byte-identical comparison.
                        // TASK-0222: shared template — see emit_log_branch.
                        emit_log_branch(out, &body_pad, &frame.loop_var, frame.latency_max_ns);
                    }
                    ViolationKind::Count => {
                        // TASK-0052.04. Atomic fetch_add per violation.
                        // The summary line is printed by the Drop on
                        // the guard local (`_nuc_check_reporter_<id>`),
                        // emitted at the top of `fn main`; the static
                        // counter (`NUC_CHECK_COUNT_<id>`) lives at file
                        // scope. Both are emitted in `render_main_rs`
                        // from `collect_count_check_frames(events)`,
                        // which performs the SAME walk this codegen
                        // path takes — so the static+guard pair always
                        // exists by the time this fetch_add runs.
                        //
                        // Relaxed ordering is sufficient: single-worker
                        // emit, so there is no cross-thread fence
                        // requirement; the Drop-time `load(Relaxed)`
                        // observes the fetch_adds because they all
                        // happen on the same thread before `main`
                        // returns. (Multi-worker pthreads-sync wires
                        // the same shape with a SHARED static across
                        // worker threads — TASK-0052.05.)
                        // TASK-0222: shared template — see emit_count_branch.
                        let id = sanitize_loop_var(&frame.loop_var);
                        emit_count_branch(out, &body_pad, &id, frame.latency_max_ns);
                    }
                }
            } else {
                render_events_in(body, out, body_indent, &body_ctx, Some(*iter_var))?;
            }
            // `for..until` early-exit break (epic S4/S5). Emitted as the
            // LAST loop-body statement (capture-before-break + cap-hit
            // sentinel); the cohesive logic + rationale live in
            // `break_loop::emit_break_check`. `None` for every plain `for`
            // loop -> nothing emitted (byte-identical regression guard).
            if let Some(cond) = break_cond {
                break_loop::emit_break_check(out, cond, var, &body_pad, &body_ctx)?;
            }
            writeln!(out, "{pad}}}").ok();
            Ok(())
        }
        // A single-worker schedule must not carry cross-worker
        // events. Surfacing rather than silently dropping keeps the
        // fail-loud contract (a lone worker with a Sync/Push/Wait is
        // a projection bug worth seeing).
        Event::Sync { .. } => Err(EmitError::ContractGap(
            "Event::Sync in a single-worker EventList — the straight-line \
             emitter expects no cross-worker synchronisation"
                .to_string(),
        )),
        Event::Push { .. } | Event::Wait { .. } => Err(EmitError::ContractGap(
            "Event::Push/Wait in a single-worker EventList — no cross-worker \
             transfer is possible with one worker"
                .to_string(),
        )),
        // Alloc/Free are not emitted by the current projection for
        // tier-1 examples; a backend that needs explicit
        // allocation lifetime would handle them here. Ignoring an
        // Alloc/Free is faithful: storage is `Vec`-managed in the
        // straight-line emitter (RAII), so an explicit region
        // reservation has no Rust counterpart.
        Event::Alloc { .. } | Event::Free { .. } => Ok(()),
    }
}
