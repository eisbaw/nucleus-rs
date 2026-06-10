//! The SHARED single-worker straight-line `main.rs` emitter.
//!
//! This is the renderer every hosted backend's single-worker
//! ("naive schedule") arm uses — directly, or via
//! [`render_single_worker_main_with_signature`] for the SPMD shape
//! (grep the call sites for the current consumer census; hard
//! consumer lists in these docs rotted repeatedly. embedded-pattern
//! is the one backend with its own separate no_std renderer). The
//! byte-identicality of the emitted single-worker `main.rs` across every
//! backend is exactly what the cross-backend differential gate relies on
//! — and the reason this code has ONE home, not a copy per backend.
//!
//! ## Provenance
//!
//! This was the last inter-backend arrow (TASK-0455.11): the code lived
//! in the `pthreads-sync` crate — historically the first backend — and
//! the other backends imported it via `pthreads_sync::*` paths, which
//! made one backend look like a hidden library hub. The function
//! consumes only `backend-common` + `nucleus-compiler` types, so the
//! move into `backend-common` is purely a re-homing: the emitted bytes
//! are byte-identical (verified by the emit-oracle A/B diff in the
//! landing commit). The sibling `events` / `break_loop` submodules came
//! with it (they are the Event-rendering tree + the `for..until`
//! early-exit machinery this renderer calls).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use nucleus_compiler::event::{DataId, Event};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use crate::check_frame::{
    collect_count_check_frames, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static,
};
use crate::render::EmitError;
use crate::render::{render_array_init_for_combine, RenderCtx};

mod break_loop;
mod events;

// `render_events` / `render_event` are re-exported here `pub(crate)` so
// the `break_loop` sibling keeps resolving `super::render_events` /
// `super::render_event` with zero change after the carve from
// pthreads-sync (where they were `crate::render_events` /
// `crate::render_event`).
pub(crate) use events::{render_event, render_events};

/// Render a single worker's straight-line `main.rs` — the SAME renderer
/// every backend's single-process arm uses. A multi-process backend
/// whose schedule has 0/1 used workers emits exactly one process;
/// reusing this keeps that process's code byte-identical to
/// pthreads-sync's, so the differential on single-worker examples is
/// real, not coincidental.
///
/// Emits the bare `mod kernels;` form — siblings-in-the-same-directory.
/// A backend whose binary is NOT a sibling of `kernels.rs` (e.g.
/// `mp-tcp-bufsync` emits under `src/bin/`) must instead call
/// [`render_single_worker_main_with_kernels_attr`] with an explicit
/// `#[path = "..."]` attribute (TASK-0177 — replaces the prior
/// `replacen("mod kernels;", …)` post-hoc string-mangling).
pub fn render_single_worker_main(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    render_single_worker_main_with_kernels_attr(events, names, sidecar, "")
}

/// Variant of [`render_single_worker_main`] that injects an explicit
/// attribute block immediately before `mod kernels;`. The caller owns
/// the full text — including any trailing newline — so the shape of
/// the attribute (`#[path = "…"]`, `#[allow(…)]`, etc.) is the
/// caller's contract, not this renderer's. An empty `kernels_mod_attr`
/// is byte-identical to [`render_single_worker_main`] (TASK-0177).
pub fn render_single_worker_main_with_kernels_attr(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_mod_attr: &str,
) -> Result<String, EmitError> {
    render_main_rs(events, names, sidecar, kernels_mod_attr, "fn main()")
}

/// Variant of [`render_single_worker_main_with_kernels_attr`] that also
/// lets the caller choose the emitted compute entry's
/// function-declaration line (e.g. `"pub fn nuc_compute()"`) instead of
/// the default `"fn main()"`. Used by the mpi-blocking SPMD backend
/// (TASK-0045): the rendered `fn main` is module-private, so to call the
/// compute body from a real `fn main` that wraps it in MPI_Init/Finalize
/// the backend emits it as a `pub fn` in a `compute` module. Passing
/// `"fn main()"` is byte-identical to
/// [`render_single_worker_main_with_kernels_attr`]. `fn_main_signature`
/// MUST be a zero-arg, unit-returning signature (the body uses no params
/// and returns `()`); the caller owns its exact text.
pub fn render_single_worker_main_with_signature(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_mod_attr: &str,
    fn_main_signature: &str,
) -> Result<String, EmitError> {
    render_main_rs(events, names, sidecar, kernels_mod_attr, fn_main_signature)
}

/// Render `main.rs` from a single worker's `EventList`.
///
/// Strategy (mirrors the old AlgoIR walk one-for-one so the output
/// is byte-identical, but reads only the EventList + sidecar):
///
/// - **Pre-init**: every data symbol written *only* via an indexed
///   `Fire` output (`D[i] <-- k(...)`) and never as a whole-array
///   output gets `let mut D = vec![<zero>; product(dims)];` up front,
///   sorted by name. Size + element type come from the sidecar.
/// - **Fire**: reconstruct the call from `FireBinding` + name
///   tables (exactly `eventlist_alone_reconstructs_stencil_kernel_call`).
///   Whole-array output → `let mut D = kernels::k(args);`; indexed
///   output → `D[idx] = kernels::k(args);`; no output → effect call.
/// - **Loop**: `for {var} in ({lo})..({hi})`; the source-form bound
///   is from `sidecar.loop_bounds[iter_var]` + `sidecar.consts`. A
///   loop var with no `loop_bounds` entry is a synthesised tile loop
///   → the concrete `Event::Loop.range` rendered `{n}_i64`.
/// - **Sync**: a single-worker schedule's post-injection ACFG has no
///   Sync/Push/Wait — but we handle them defensively (a lone worker
///   should not have cross-worker events; surface a contract gap if
///   it does rather than silently drop).
fn render_main_rs(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_mod_attr: &str,
    // The function-declaration line for the emitted compute entry. The
    // default-shaped callers pass `"fn main()"` (byte-identical to the
    // historical output). The mpi-blocking SPMD backend passes a `pub
    // fn` so it can call the compute body from a real `fn main` that
    // wraps it in MPI_Init/Finalize (TASK-0045) — the rendered `fn main`
    // would otherwise be module-private and unreachable.
    fn_main_signature: &str,
) -> Result<String, EmitError> {
    let mut out = String::new();
    // Backend-agnostic header (TASK-0231): this renderer is the SHARED
    // single-worker main.rs emitter — pthreads-sync calls it directly,
    // pthreads-async delegates to it, mp-tcp-bufsync calls it for the
    // 0/1-used-worker case. The header must not name any single backend
    // or the provenance lies for the delegators. The byte-identicality
    // invariant across backends on the single-worker path is what the
    // cross-backend differential gate relies on; using a neutral header
    // preserves it.
    writeln!(out, "//! Generated by the nucleus pre-compiler.").ok();
    writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
    writeln!(out).ok();
    writeln!(out, "// The user's kernel bodies live in kernels.rs.").ok();
    // `kernels_mod_attr` (TASK-0177): typed module-attribute injection
    // for backends that need a `#[path = "..."]` redirect (mp-tcp-bufsync
    // emits the binary under `src/bin/`, so the bare `mod kernels;` form
    // does not resolve the sibling `src/kernels.rs`). The caller owns
    // the trailing newline; an empty string emits the bare form unchanged
    // (byte-identical to the pre-TASK-0177 output for pthreads-sync /
    // pthreads-async). Replaces the prior `replacen("mod kernels;", …)`
    // string-mangling in mp-tcp-bufsync::wrap_single_worker.
    write!(out, "{}", kernels_mod_attr).ok();
    writeln!(out, "mod kernels;").ok();
    writeln!(out).ok();

    // Real-time `check loop V : on_violation=count` (TASK-0052.04).
    // For every `Count` check loop in the program, emit ONE file-scope
    // `AtomicU64` static + ONE Drop guard local in `fn main` (below).
    // The Drop guard struct itself is emitted once per file (iff any
    // Count loop exists). The guard's Drop prints a stderr summary at
    // run end. Stdout is untouched -> the cross-backend differential
    // remains stable.
    //
    // Why Drop-guard over atexit/thread-local: see TASK-0052.04 notes.
    // `static` (not `static mut`) + `AtomicU64` is the std-idiomatic
    // way; no `unsafe`, no init, lock-free fetch_add. The guard struct
    // is named `NucCheckCountReporter` (file-private).
    let count_frames = collect_count_check_frames(events);
    if !count_frames.is_empty() {
        emit_count_reporter_struct(&mut out);
        for cf in &count_frames {
            // TASK-0222: shared template — see emit_count_static.
            emit_count_static(&mut out, &cf.ident);
        }
        writeln!(out).ok();
    }

    writeln!(out, "#[allow(unused_mut, dead_code, unused_variables)]").ok();
    writeln!(out, "{fn_main_signature} {{").ok();

    // Per-Count-loop Drop guard local. Variable name is unique per
    // sanitized loop_var and is `_nuc_check_reporter_<ident>`; the
    // underscore prefix suppresses the unused-binding warning while
    // keeping the binding alive until `fn main` returns. The Drop on
    // the guard fires there, printing the final tally to stderr.
    for cf in &count_frames {
        // TASK-0222: shared template — see emit_count_guard_local.
        emit_count_guard_local(&mut out, &cf.ident, &cf.loop_var, cf.latency_max_ns);
    }
    if !count_frames.is_empty() {
        writeln!(out).ok();
    }

    // Pre-initialise every data symbol assigned via an indexed Fire
    // output and never whole-array. Sorted by name (BTreeSet) so the
    // output is deterministic — same order the old
    // `collect_pre_init_data` produced.
    let pre_init = collect_pre_init_data(events, names, sidecar)?;
    for (name, did) in &pre_init {
        let rs_init = render_array_init(*did, sidecar, name)?;
        writeln!(out, "    let mut {name} = {rs_init};").ok();
    }
    if !pre_init.is_empty() {
        writeln!(out).ok();
    }

    // Absolute-index rebinding is now decided PER-OCCURRENCE from
    // each `Event::Loop.block_tag` (TASK-0180), not from a
    // program-global occurrence count — so there is no longer a
    // pre-walk to classify inner-block vars here.
    //
    // `RenderCtx` lives in `backend_common::render` as of TASK-0244;
    // its `abs_subst` map starts empty and is grown per-occurrence in
    // `render_event` when a strip-mined `block_tag` arrives.
    let ctx = RenderCtx::new(names, sidecar);

    // `for..until` runtime break-generation final-read + cap-hit
    // observability (TASK-0341.02.01.05.02 / .05.03). When the EventList
    // carries an early-exit loop the top-level emit differs (break-gen
    // sentinel decl + cap-hit resolution + structural final-read
    // rewrite); when it does not, the events render unchanged. The whole
    // decision + emit is the cohesive break machinery, so it lives in
    // `break_loop` (the touched-file mega-file discipline, cycle-262
    // architect P2-1) rather than inline here.
    break_loop::render_top_level_events(events, &mut out, &ctx, sidecar)?;

    writeln!(out, "}}").ok();
    Ok(out)
}

// `divisible_inner_block_vars` (the program-global `counts==1`
// occurrence heuristic) was DELETED in TASK-0180. It conflated a
// divisible single-nest, a non-divisible full+partial sibling pair,
// and a loop-var name reused across N evenly-divisible passes — all
// of which share ONE reused IterVar — so it silently skipped
// absolute-index rebinding for the reused-name case (the
// 04-prefix-sum/blocked accumulator double-count). Rebinding is now
// decided per-occurrence from `Event::Loop.block_tag`; see the
// `Event::Loop` arm of `render_event`.

/// Find every data symbol written *only* via an indexed `Fire`
/// output (`D[i] <-- k(...)`) — never a whole-array output. Those
/// need an up-front allocation. Returns `(name, DataId)` pairs in
/// lexicographic name order (deterministic; matches the old
/// `collect_pre_init_data` BTreeSet ordering).
fn collect_pre_init_data(
    events: &[Event],
    names: &NameTables,
    _sidecar: &NameSidecar,
) -> Result<Vec<(String, DataId)>, EmitError> {
    let mut whole_array: BTreeSet<DataId> = BTreeSet::new();
    let mut indexed: BTreeSet<DataId> = BTreeSet::new();
    walk_fire_outputs(events, &mut whole_array, &mut indexed);

    // Order by NAME lexicographically (the old code keyed a
    // BTreeSet<String>). Build (name, did) and sort by name.
    let mut out: Vec<(String, DataId)> = Vec::new();
    for did in &indexed {
        if whole_array.contains(did) {
            continue;
        }
        let name = names.data.get(did).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "data id {did:?} written by an indexed Fire has no name in NameTables"
            ))
        })?;
        out.push((name.clone(), *did));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Walk every `Fire` output in `events` (recursing into `Loop` bodies),
/// partitioning the written `DataId`s into `whole_array` (a whole-array
/// output, indices empty) and `indexed` (an indexed output `D[i]`). The
/// single-worker pre-init (`collect_pre_init_data`) and the `for..until`
/// break-write set (`break_loop::collect_break_loop_info`) both consume
/// this.
pub(crate) fn walk_fire_outputs(
    events: &[Event],
    whole_array: &mut BTreeSet<DataId>,
    indexed: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    if o.indices.is_empty() {
                        whole_array.insert(o.data);
                    } else {
                        indexed.insert(o.data);
                    }
                }
            }
            Event::Loop { body, .. } => walk_fire_outputs(body, whole_array, indexed),
            // Sync / Push / Wait / Alloc / Free carry no Fire output.
            _ => {}
        }
    }
}

/// `vec![<identity>; product(dims)]` for an array, or the scalar
/// identity literal for a scalar — sized + typed ENTIRELY from the
/// sidecar (no AlgoIR). Local wrapper that adds a named-DataId
/// contract-gap error around
/// `backend_common::render::render_array_init_for_combine` (TASK-0244
/// single source of truth for the string shape). The identity is the
/// data symbol's combine identity if it owns an accumulator fan-in
/// (`combine_for_data`), else zero (TASK-0343.01.02). Even in the
/// single-worker path a `combine=min|max|and` accumulator must
/// pre-init to its identity to stay bit-identical to the multi-worker
/// per-partition partials.
fn render_array_init(did: DataId, sidecar: &NameSidecar, name: &str) -> Result<String, EmitError> {
    let ty = sidecar.data_type(did).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "data `{name}` ({did:?}) has no ResolvedType in the NameSidecar \
             (build_sidecar should carry every ACFG data symbol)"
        ))
    })?;
    Ok(render_array_init_for_combine(
        ty,
        sidecar.combine_for_data.get(&did).copied(),
    ))
}
