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
use nucleus_compiler::event::{DataId, Event, IterVar, ViolationKind, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

// Shared codegen primitives (TASK-0244 cycle 37). The expression /
// index / kernel-call / loop-bound / type renderers, the check_frame
// emit templates, and the per-worker multi-worker event walker all
// live in backend-common — every backend (this one, pthreads-async,
// mp-tcp-bufsync) consumes the SAME implementation, no drift.
use backend_common::check_frame::{
    collect_count_check_frames, emit_count_branch, emit_count_guard_local,
    emit_count_reporter_struct, emit_count_static, emit_log_branch, sanitize_loop_var,
};
use backend_common::project_skeleton::single_binary::{render_cargo_toml, render_run_sh};
use backend_common::render::{
    data_name, render_array_init_for, render_const_expr, render_fire_args,
    render_fire_output_assign, render_loop_bounds, render_reuse_buf_decls,
    render_reuse_marker_comment, render_reuse_per_iter_update, RenderCtx,
};
// Re-export the codegen-time error type so downstream callers (the
// driver, tests, other backends that delegate to this crate's
// single-worker emitter) continue to spell it `pthreads_sync::
// EmitError`. The canonical definition lives in backend-common since
// every backend re-exports it identically.
pub use backend_common::render::EmitError;

mod multi_worker;

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
    render_events(events, &mut out, 1, &ctx)?;

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

fn walk_fire_outputs(
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

/// `vec![<zero>; product(dims)]` for an array, or the scalar zero
/// literal for a scalar — sized + typed ENTIRELY from the sidecar
/// (no AlgoIR). Local wrapper that adds a named-DataId contract-gap
/// error around `backend_common::render::render_array_init_for`
/// (TASK-0244 single source of truth for the string shape).
fn render_array_init(did: DataId, sidecar: &NameSidecar, name: &str) -> Result<String, EmitError> {
    let ty = sidecar.data_type(did).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "data `{name}` ({did:?}) has no ResolvedType in the NameSidecar \
             (build_sidecar should carry every ACFG data symbol)"
        ))
    })?;
    Ok(render_array_init_for(ty))
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

fn render_events(
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

fn render_event(
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
