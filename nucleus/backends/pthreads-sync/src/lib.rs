//! pthreads-sync backend. PRD §7.1, TASK-0020, TASK-0124.
//!
//! Tier-1 CPU backend that consumes the **EventList contract**
//! (PRD §7.4 / §8.3) — the per-worker [`Event`] projection of the
//! ACFG plus the [`NameSidecar`] (TASK-0160/0169) and the ACFG
//! name tables — and emits a standalone Cargo project containing
//! runnable Rust. Single-worker ("naive schedule") uses the
//! straight-line emitter; multi-worker uses `std::thread::spawn` +
//! `std::sync::Barrier` + `Slot<T>` channels.
//!
//! ## Why the backend now consumes the EventList, not AlgoIR (TASK-0124)
//!
//! The original M1 backend (TASK-0020) walked `LinkedIR::algo`
//! statements directly because the ACFG stripped index expressions
//! and the per-worker EventList did not yet carry loop structure or
//! value bindings. Three contract extensions closed that gap:
//!
//! - **TASK-0156** put the per-firing value binding (`FireBinding`)
//!   on [`Event::Fire`] — a backend reconstructs the exact kernel
//!   call (callee, indexed args, output slice) from the event alone.
//! - **TASK-0159** made the projection structure-preserving:
//!   [`Event::Loop`] mirrors `ACFGNode::Repeat` (iter var + range +
//!   nested body) instead of unrolling it, so a rolled `for` is
//!   re-emittable.
//! - **TASK-0160/0169** added the [`NameSidecar`]: per-`DataId`
//!   `ResolvedType` (pre-init sizing + slot typing + scalar casts),
//!   const values, the *unevaluated* symbolic loop bounds (so
//!   `for y : 1 .. H-1` re-renders as `(1_i64)..((16_i64 - 1_i64))`
//!   verbatim, not the folded `1..15`), and per-`KernelId`
//!   signatures (scalar-arg cast decisions without `algo.kernels`).
//!
//! With `(EventList, name tables, NameSidecar)` the codegen path is
//! **fully AlgoIR-/LinkedIR-free** (AC#2): this module imports
//! neither `nucleus_compiler::algo` nor `nucleus_compiler::link` for code emission.
//! `IrExpr` is used only as the inert index/scalar-expression grammar
//! the EventList already carries (it is the single source of truth
//! for "what an index is"; no pass evaluates it here).
//!
//! ## Generated artefact layout
//!
//! Under the user-provided `out_dir`:
//!
//! ```text
//! out_dir/
//!   Cargo.toml         -- standalone project, depends on nothing exotic
//!   src/
//!     main.rs          -- runs the algorithm
//!     kernels.rs       -- copy of the user's kernels.rs
//!   run.sh             -- builds + runs the binary with input.bin -> output.bin
//! ```
//!
//! `kernels.rs` is *copied* (not `include!`-ed): the generated
//! project is fully self-contained and movable. Trade-off: two files
//! reflect one source-of-truth; the expected workflow is "run
//! codegen, then build" — editing kernels.rs is followed by
//! re-running codegen.
//!
//! ## Error handling
//!
//! Failures bubble up as [`EmitError`] variants with the offending
//! path / reason attached. No silent fallbacks. The
//! [`EmitError::ContractGap`] variant is the fail-loud seam for "the
//! EventList/sidecar did not carry something the backend needs"
//! (e.g. a `DataId` with no sidecar type) — it must never be papered
//! over with a default.
//!
//! ## Honest limitations
//!
//! - **Single-worker straight-line + multi-worker thread/barrier.**
//!   Distributed placements (`place k on {w0,w1,w2}`) are still
//!   rejected (TASK-0117). Async / `buffer>1` transfers rejected
//!   (sync-only backend).
//! - **Aggregate I/O kernels.** `() -> Vec<T>` / `(Vec<T>) -> ()`
//!   recognised via the sidecar element type; whole-array
//!   binding/move calls emitted accordingly.
//! - **No error recovery in generated code.** A panic in any kernel
//!   aborts the whole binary (`panic = "abort"`).
//! - **No identity-copy support** (`d <-- e` with a bare DataRef
//!   RHS). The contract carries this as a `Fire` with a kernel; a
//!   non-`Call` dataflow shape never reaches a `Fire`, so this hole
//!   is inherited from the front passes, not introduced here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

// AlgoIR-free: this crate is now AlgoIR-FREE — every type used here
// comes through `backend_common::render` (which re-exports from
// `nucleus_compiler::algo` where needed for its OWN typed signatures).
use nucleus_compiler::event::{DataId, Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

// Shared codegen primitives (TASK-0244 cycle 37). The expression /
// index / kernel-call / loop-bound / type renderers, the check_frame
// emit templates, and the per-worker multi-worker event walker all
// live in backend-common — every backend (this one, pthreads-async,
// mp-tcp-bufsync) consumes the SAME implementation, no drift.
use backend_common::check_frame::{
    collect_count_check_frames, emit_count_guard_local, emit_count_reporter_struct,
    emit_count_static,
};
use backend_common::project_skeleton::single_binary::{render_cargo_toml, render_run_sh};
use backend_common::render::{render_array_init_for_combine, RenderCtx};
// Re-export the codegen-time error type so downstream callers (the
// driver, tests, other backends that delegate to this crate's
// single-worker emitter) continue to spell it `pthreads_sync::
// EmitError`. The canonical definition lives in backend-common since
// every backend re-exports it identically.
pub use backend_common::render::EmitError;

mod break_loop;
mod events;
mod multi_worker;

// The single-worker Event-rendering tree (`render_events` /
// `render_events_in` / `render_event`) was carved out to `events.rs`
// (TASK-0437.01) to keep this file under the `just check-mega-files`
// 1000-LoC fence. `render_events` / `render_event` are re-exported
// here `pub(crate)` so the `break_loop` consumer keeps resolving
// `crate::render_events` / `crate::render_event` with zero change.
pub(crate) use events::{render_event, render_events};

// --------------------------------------------------------------------
// Public surface
// --------------------------------------------------------------------

// NameTables moved to `nucleus-compiler` as of TASK-0238 (cycle 25;
// crate previously named `compiler`, renamed in TASK-0084 cycle 76).
// Re-exported here so historic `pthreads_sync::NameTables` paths
// continue to work (mp-tcp-bufsync's `pub use pthreads_sync::NameTables`
// + pthreads-async's same re-export are unchanged; both transitively
// re-export the now-`nucleus_compiler::NameTables` definition).
// Cycle-24 review-gate B.1 found that the struct holds zero
// pthreads-sync-specific content, and its prior home prevented the
// cross-backend test-helper crate `test-common` from depending on
// pthreads-sync (would cycle). Moving to nucleus-compiler dissolves
// both constraints.
pub use nucleus_compiler::NameTables;

/// Paths to the files [`emit`] wrote, returned for callers that want
/// to inspect or invoke them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    /// The generated Cargo project root (== input `out_dir`).
    pub project_dir: PathBuf,
    /// Path to the emitted `Cargo.toml`.
    pub cargo_toml: PathBuf,
    /// Path to the emitted `src/main.rs`.
    pub main_rs: PathBuf,
    /// Path to the emitted `src/kernels.rs`.
    pub kernels_rs: PathBuf,
    /// Path to the emitted `run.sh`.
    pub run_sh: PathBuf,
}

// EmitError moved to backend-common (TASK-0244). The canonical type
// is `backend_common::render::EmitError`; this crate re-exports it at
// the top of this file (`pub use backend_common::render::EmitError`)
// so historic `pthreads_sync::EmitError` paths keep working.

/// Emit a runnable Cargo project from the per-worker EventList.
///
/// AC#1 signature (TASK-0124): the backend consumes the per-worker
/// [`Event`] lists, the [`NameTables`] (reverse `name_*`), and the
/// [`NameSidecar`] — NOT `&ACFG` / `&LinkedIR`. `kernels_rs_path` is
/// copied verbatim into the generated project.
///
/// Single- vs multi-worker is chosen by counting workers whose
/// EventList is non-empty: 0/1 → straight-line emitter, ≥2 →
/// thread/barrier emitter. (`acfg_to_events` seeds every declared
/// worker with an empty list, so an unused declared worker does not
/// trip the multi-worker path — exactly the old `collect_used_workers`
/// semantics, now read off the EventList.)
pub fn emit(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    // ---- Pick a code path: single-worker vs multi-worker. ----
    let used_workers: Vec<WorkerId> = per_worker
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();

    // ---- Read user kernels.rs ----
    let kernels_src =
        fs::read_to_string(kernels_rs_path).map_err(|e| EmitError::KernelsReadFailed {
            path: kernels_rs_path.to_path_buf(),
            source: e,
        })?;

    // ---- Create the output skeleton. ----
    let src_dir = out_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| EmitError::OutputCreateFailed {
        path: src_dir.clone(),
        source: e,
    })?;

    let cargo_toml = out_dir.join("Cargo.toml");
    let main_rs = src_dir.join("main.rs");
    let kernels_rs = src_dir.join("kernels.rs");
    let run_sh = out_dir.join("run.sh");

    // ---- Render Cargo.toml ----
    // `extra_dependencies = None` keeps the emitted Cargo.toml
    // byte-identical to its pre-cycle-196 shape (TASK-0044.01.01
    // added the parameter for openmp-rs's multi-worker `rayon` dep;
    // pthreads-sync has no extra dep).
    write_file(&cargo_toml, &render_cargo_toml(None))?;

    // ---- Copy kernels.rs verbatim ----
    write_file(&kernels_rs, &kernels_src)?;

    // ---- Render main.rs ----
    let main_rs_src = if used_workers.len() <= 1 {
        let events = used_workers
            .first()
            .and_then(|w| per_worker.get(w))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        render_main_rs(events, names, sidecar, "", "fn main()")?
    } else {
        multi_worker::render_main_rs_multi(per_worker, names, sidecar)?
    };
    write_file(&main_rs, &main_rs_src)?;

    // ---- Render run.sh ----
    write_file(&run_sh, &render_run_sh())?;
    // Best-effort: mark run.sh executable. Failure here is non-fatal.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(&run_sh) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(&run_sh, perms);
        }
    }

    Ok(EmitResult {
        project_dir: out_dir.to_path_buf(),
        cargo_toml,
        main_rs,
        kernels_rs,
        run_sh,
    })
}

// --------------------------------------------------------------------
// Project-skeleton renderers (Cargo.toml + run.sh)
// --------------------------------------------------------------------
//
// MOVED in TASK-0246 (cycle 38) to
// `backend_common::project_skeleton::single_binary::{render_cargo_toml,
// render_run_sh}`. The templates are byte-stable inert strings with
// zero pthreads-sync-specific content; lifting them out closes the
// last non-semantic dependency from pthreads-async on pthreads-sync.
// The only inter-backend arrow that survives is
// `render_single_worker_main` (a real semantic delegation: pthreads-
// async's single-worker arm IS pthreads-sync's straight-line main.rs,
// byte-identical by construction; mp-tcp-bufsync's single-process arm
// likewise wraps it). The renderers are imported at the top of this
// file from backend-common and consumed verbatim by `emit()`.

// --------------------------------------------------------------------
// EventList -> main.rs codegen (single-worker / straight-line)
// --------------------------------------------------------------------
//
// The check_frame emit templates (CountCheckLoop, sanitize_loop_var,
// collect_count_check_frames, emit_count_reporter_struct,
// emit_count_static, emit_count_guard_local, emit_log_branch,
// emit_count_branch) lived inline here through TASK-0052.04 and
// TASK-0222. TASK-0244 (cycle 37) moved them into
// `backend_common::check_frame` so every backend imports the SAME
// implementation — no second copy exists to drift.

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
    // pthreads-async delegates to it (lib.rs:298), mp-tcp-bufsync calls
    // it for the 0/1-used-worker case. The header must not name any
    // single backend or the provenance lies for the delegators. The
    // byte-identicality invariant across backends on the single-worker
    // path is what the cross-backend differential gate relies on; using
    // a neutral header preserves it.
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

// `rust_scalar_zero`, `bin_op_str`, `render_int_expr`,
// `render_const_expr`, `render_loop_bounds`, `render_flat_index`,
// `classify_data_slice`, `SliceForm`, `render_fire_arg`,
// `render_fire_args`, `render_fire_output_assign`, `data_name`,
// `rust_scalar_type`, and `RenderCtx` all moved to
// `backend_common::render` (TASK-0244). They are imported at the top
// of this file from there.

// --------------------------------------------------------------------
// Event rendering
// --------------------------------------------------------------------
//
// MOVED in TASK-0437.01 to `events.rs` (this file's single-worker
// Event-rendering tree: `render_events` / `render_events_in` /
// `render_event`). `render_events` / `render_event` are re-exported
// `pub(crate)` near the `mod events;` declaration above so the
// `break_loop` consumer keeps resolving them via `crate::`. The carve
// drops this file under the `just check-mega-files` 1000-LoC fence with
// zero behaviour change.

// `data_name` moved to `backend_common::render` (TASK-0244).

// `render_fire_output_assign`, `render_fire_args`, `render_fire_arg`,
// `SliceForm`, `classify_data_slice`, `render_flat_index`,
// `render_int_expr`, `render_loop_bounds`, `render_const_expr`,
// `bin_op_str`, `rust_scalar_type`, plus the `RenderCtxPub` shim
// surface (`render_fire_args_pub`, `render_flat_index_pub`,
// `render_fire_output_assign_pub`, `render_const_expr_pub`,
// `render_array_init_for`, `rust_type_of`, `rust_scalar_type_pub`)
// all moved to `backend_common::render` in cycle 37 (TASK-0244).
// Backends that imported them via `pthreads_sync::*` now import
// from `backend_common::render::*`. The drift-prevention property
// is unchanged — one implementation, called from every backend.

/// Render a single worker's straight-line `main.rs` body — the SAME
/// renderer the single-process pthreads-sync path uses. A
/// multi-process backend whose schedule has 0/1 used workers emits
/// exactly one process; reusing this keeps that process's code
/// byte-identical to pthreads-sync's, so the differential on
/// single-worker examples is real, not coincidental. Public for
/// `mp-tcp-bufsync` (TASK-0036).
///
/// Emits the bare `mod kernels;` form — siblings-in-the-same-directory.
/// A backend whose binary is NOT a sibling of `kernels.rs` (e.g.
/// `mp-tcp-bufsync` emits under `src/bin/`) must instead call
/// `render_single_worker_main_with_kernels_attr` with an explicit
/// `#[path = "..."]` attribute (TASK-0177 — replaces the prior
/// `replacen("mod kernels;", …)` post-hoc string-mangling).
pub fn render_single_worker_main(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    render_single_worker_main_with_kernels_attr(events, names, sidecar, "")
}

/// Variant of `render_single_worker_main` that injects an explicit
/// attribute block immediately before `mod kernels;`. The caller owns
/// the full text — including any trailing newline — so the shape of
/// the attribute (`#[path = "…"]`, `#[allow(…)]`, etc.) is the
/// caller's contract, not this renderer's. An empty `kernels_mod_attr`
/// is byte-identical to `render_single_worker_main` (TASK-0177).
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

// `render_array_init_for`, `rust_type_of`, `rust_scalar_type_pub`
// moved to `backend_common::render` (TASK-0244). See the consolidated
// move-note comment above `render_single_worker_main`.

// --------------------------------------------------------------------
// File-write helper
// --------------------------------------------------------------------

fn write_file(path: &Path, contents: &str) -> Result<(), EmitError> {
    fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
