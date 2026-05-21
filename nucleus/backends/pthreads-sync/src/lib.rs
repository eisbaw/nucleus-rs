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
//! - **TASK-0156** put the per-firing value binding ([`FireBinding`])
//!   on [`Event::Fire`] — a backend reconstructs the exact kernel
//!   call (callee, indexed args, output slice) from the event alone.
//! - **TASK-0159** made the projection structure-preserving:
//!   [`Event::Loop`] mirrors `ACFGNode::Repeat` (iter var + range +
//!   nested body) instead of unrolling it, so a rolled `for` is
//!   re-emittable.
//! - **TASK-0160/0169** added the [`NameSidecar`]: per-`DataId`
//!   [`ResolvedType`] (pre-init sizing + slot typing + scalar casts),
//!   const values, the *unevaluated* symbolic loop bounds (so
//!   `for y : 1 .. H-1` re-renders as `(1_i64)..((16_i64 - 1_i64))`
//!   verbatim, not the folded `1..15`), and per-`KernelId`
//!   signatures (scalar-arg cast decisions without `algo.kernels`).
//!
//! With `(EventList, name tables, NameSidecar)` the codegen path is
//! **fully AlgoIR-/LinkedIR-free** (AC#2): this module imports
//! neither `compiler::algo` nor `compiler::link` for code emission.
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
use std::io;
use std::path::{Path, PathBuf};

// AlgoIR-free: the ONLY `compiler::algo` import is the inert
// expression grammar the EventList itself carries (index / scalar
// arg / const-bound expressions). No `AlgoIR`, no `LinkedIR`, no
// `ResolvedKernel`, no statement walk. ResolvedType/ScalarType come
// exclusively from the NameSidecar.
use compiler::algo::{IrBinOp, IrExpr, ResolvedType, ScalarType};
use compiler::event::{
    ArgBinding, DataId, DataSlice, Event, IterVar, KernelId, ViolationKind, WorkerId,
};
use compiler::sidecar::NameSidecar;

mod multi_worker;

// --------------------------------------------------------------------
// Public surface
// --------------------------------------------------------------------

/// Reverse name tables travelling alongside the per-worker
/// `EventList` + the [`NameSidecar`]. Each map is the inverse of the
/// corresponding `ACFG::name_*` table (`name -> id` inverted to
/// `id -> name`). The backend joins these against the opaque ids the
/// `Event`s / sidecar carry — exactly the join the proven
/// reconstruction tests in `compiler/tests/petri_to_events.rs`
/// perform. The driver builds these from the post-pass ACFG; the
/// backend never sees the ACFG itself.
#[derive(Debug, Clone, Default)]
pub struct NameTables {
    /// `DataId -> data symbol name` (inverse of `acfg.name_data`).
    pub data: BTreeMap<DataId, String>,
    /// `KernelId -> kernel name` (inverse of `acfg.name_kernels`).
    pub kernel: BTreeMap<KernelId, String>,
    /// `WorkerId -> worker name` (inverse of `acfg.name_workers`).
    pub worker: BTreeMap<WorkerId, String>,
    /// `IterVar -> loop-variable name` (inverse of
    /// `acfg.name_iter_vars`). A `block_transform`-synthesised tile
    /// iter-var has an entry here too (it has a generated name) but
    /// NO `NameSidecar::loop_bounds` entry — that absence is how the
    /// backend tells "synthesised tile loop, use concrete range" from
    /// "source loop, render symbolic bound".
    pub iter_var: BTreeMap<IterVar, String>,
    /// The set of *inner intra-tile* loop iter-vars produced by
    /// `block_transform` (verbatim `acfg.inner_block_iter_vars`).
    ///
    /// `block_transform` rewrites `for VAR : LO..HI  block=N` into
    /// `for VAR__tile : 0..ceil((HI-LO)/N) { for VAR : 0..N { body } }`
    /// and **reuses VAR's original [`IterVar`] on the inner loop**.
    /// Its module docs are explicit (line ~83): the inner loop
    /// iterates `0..N`, NOT `LO..LO+N`, so **codegen** that wants the
    /// absolute iteration value "must compute `LO + tile*N + inner`"
    /// — block_transform deliberately defers that index rebinding to
    /// the backend.
    ///
    /// The pre-TASK-0124 AlgoIR-walking backend never did this: it
    /// walked the *source* `IrStmt` (which has no block transform at
    /// all) and emitted the untiled loop, which is runtime-correct
    /// only because it never tiled. The EventList faithfully carries
    /// the tiled structure, so the EventList-only backend MUST do
    /// the absolute-index rebinding block_transform defers, or an
    /// accumulator kernel (07-matmul `madd`) double-counts.
    ///
    /// LIMITATION (filed as TASK-0173): the rebinding is applied only
    /// for the **evenly-divisible** case (one tile nest per inner
    /// var — e.g. 07-matmul `block=8`, N=16). The **non-divisible /
    /// trailing-partial-tile** case (05-stencil `block=4`, range
    /// length 14) decomposes into TWO sibling nests whose correct
    /// absolute formulas differ (`LO + tile*N + inner` for the full
    /// nest vs the constant base `LO + num_full*N + inner` for the
    /// partial nest) — and the EventList/ACFG does NOT carry
    /// `num_full` / a "this is the partial tile" marker, so a correct
    /// general rebinding is a real contract extension, not a clean
    /// backend-local change. For the non-divisible inner var the
    /// backend keeps the source-form bound (current behaviour);
    /// 05-stencil/blocked stays runtime-correct because `blur3` is
    /// idempotent (re-writing `img_out[y][x]` with the same value).
    /// See TASK-0173 for the proper fix.
    pub inner_block_iter_vars: BTreeSet<IterVar>,
}

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

/// Errors that can stop a codegen run.
#[derive(Debug)]
pub enum EmitError {
    /// Failed to read the user's `kernels.rs`.
    KernelsReadFailed { path: PathBuf, source: io::Error },
    /// Failed to create `out_dir` or any sub-directory.
    OutputCreateFailed { path: PathBuf, source: io::Error },
    /// Failed to write a generated file.
    WriteFailed { path: PathBuf, source: io::Error },
    /// The EventList asks for something this backend cannot emit
    /// (nested call in argument position, identity-copy shape, …).
    UnsupportedFeature(String),
    /// The `(EventList, NameSidecar, name tables)` contract did not
    /// carry a fact the backend needs to emit valid code (a `DataId`
    /// with no sidecar type, a `KernelId` with no name, …). This is
    /// the fail-loud seam for a contract regression — NEVER paper
    /// over it with a default (CLAUDE.md: no workarounds).
    ContractGap(String),
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::KernelsReadFailed { path, source } => {
                write!(f, "failed to read kernels.rs at {}: {}", path.display(), source)
            }
            EmitError::OutputCreateFailed { path, source } => write!(
                f,
                "failed to create output directory {}: {}",
                path.display(),
                source
            ),
            EmitError::WriteFailed { path, source } => {
                write!(f, "failed to write {}: {}", path.display(), source)
            }
            EmitError::UnsupportedFeature(msg) => {
                write!(f, "pthreads-sync: unsupported feature: {msg}")
            }
            EmitError::ContractGap(msg) => {
                write!(
                    f,
                    "pthreads-sync: EventList/sidecar contract gap: {msg}"
                )
            }
        }
    }
}

impl std::error::Error for EmitError {}

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
    write_file(&cargo_toml, &render_cargo_toml())?;

    // ---- Copy kernels.rs verbatim ----
    write_file(&kernels_rs, &kernels_src)?;

    // ---- Render main.rs ----
    let main_rs_src = if used_workers.len() <= 1 {
        let events = used_workers
            .first()
            .and_then(|w| per_worker.get(w))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        render_main_rs(events, names, sidecar)?
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
// File renderers (unchanged from TASK-0020 — byte-stable)
// --------------------------------------------------------------------

fn render_cargo_toml() -> String {
    String::from(
        "# Generated by the pthreads-sync backend. Do not edit; rerun \
         `nucleus build` to regenerate.\n\
         [package]\n\
         name        = \"nuc-generated\"\n\
         version     = \"0.0.0\"\n\
         edition     = \"2021\"\n\
         publish     = false\n\
         \n\
         [workspace]\n\
         # Empty: this crate is standalone, not part of any parent workspace.\n\
         \n\
         [[bin]]\n\
         name = \"nuc-generated\"\n\
         path = \"src/main.rs\"\n\
         \n\
         [profile.release]\n\
         panic = \"abort\"\n",
    )
}

fn render_run_sh() -> String {
    String::from(
        "#!/usr/bin/env bash\n\
         # Generated by the pthreads-sync backend. Rerun `nucleus build` to regenerate.\n\
         # Usage: bash run.sh INPUT_BIN OUTPUT_BIN\n\
         set -euo pipefail\n\
         \n\
         here=\"$(cd -- \"$(dirname -- \"${BASH_SOURCE[0]}\")\" && pwd)\"\n\
         input_bin=\"${1:-input.bin}\"\n\
         output_bin=\"${2:-output.bin}\"\n\
         \n\
         (cd \"$here\" && cargo build --release --quiet)\n\
         \n\
         NUC_INPUT_PATH=\"$input_bin\" \\\n\
         NUC_OUTPUT_PATH=\"$output_bin\" \\\n\
         \"$here/target/release/nuc-generated\"\n",
    )
}

// --------------------------------------------------------------------
// EventList -> main.rs codegen (single-worker / straight-line)
// --------------------------------------------------------------------

/// Render `main.rs` from a single worker's `EventList`.
///
/// Strategy (mirrors the old AlgoIR walk one-for-one so the output
/// is byte-identical, but reads only the EventList + sidecar):
///
/// - **Pre-init**: every data symbol written *only* via an indexed
///   `Fire` output (`D[i] <-- k(...)`) and never as a whole-array
///   output gets `let mut D = vec![<zero>; product(dims)];` up front,
///   sorted by name. Size + element type come from the sidecar.
/// - **Fire**: reconstruct the call from [`FireBinding`] + name
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
) -> Result<String, EmitError> {
    let mut out = String::new();
    writeln!(out, "//! Generated by the pthreads-sync backend (TASK-0020, M1).").ok();
    writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
    writeln!(out).ok();
    writeln!(out, "// The user's kernel bodies live in kernels.rs.").ok();
    writeln!(out, "mod kernels;").ok();
    writeln!(out).ok();
    writeln!(out, "#[allow(unused_mut, dead_code, unused_variables)]").ok();
    writeln!(out, "fn main() {{").ok();

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
    let ctx = RenderCtx {
        names,
        sidecar,
        abs_subst: BTreeMap::new(),
    };
    render_events(events, &mut out, 1, &ctx)?;

    writeln!(out, "}}").ok();
    Ok(out)
}

struct RenderCtx<'a> {
    names: &'a NameTables,
    sidecar: &'a NameSidecar,
    /// Active absolute-index substitutions: an inner-block loop
    /// variable name -> the `(LO + tile*N + inner)` Rust expression
    /// it must expand to at every *body* use site. Empty for every
    /// non-blocked program, so non-blocked codegen is byte-identical
    /// to the pre-TASK-0124 backend (the map is consulted only by
    /// `render_int_expr`/`render_const_expr` on an `Ident`).
    abs_subst: BTreeMap<String, String>,
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
/// (no AlgoIR). Mirrors the old `render_array_init`.
fn render_array_init(
    did: DataId,
    sidecar: &NameSidecar,
    name: &str,
) -> Result<String, EmitError> {
    let ty = sidecar.data_type(did).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "data `{name}` ({did:?}) has no ResolvedType in the NameSidecar \
             (build_sidecar should carry every ACFG data symbol)"
        ))
    })?;
    if ty.is_scalar() {
        Ok(rust_scalar_zero(&ty.scalar).to_string())
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        Ok(format!("vec![{zero}; {total}]"))
    }
}

/// The Rust literal for "zero" of a scalar type.
fn rust_scalar_zero(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::Usize | ScalarType::Isize => "0",
        ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 => "0",
        ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 => "0",
        ScalarType::F32 | ScalarType::F64 => "0.0",
        ScalarType::Bool => "false",
    }
}

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
                let abs = if tag.is_partial {
                    // Constant base: the partial tile's own tile loop
                    // is `0..1`, so a `tile*N` term is always 0.
                    format!("({lo_src} + ({}_i64 * {n}_i64) + {var})", tag.num_full)
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
                    format!("({lo_src} + ({tile_name} * {n}_i64) + {var})")
                };
                let mut child_subst = ctx.abs_subst.clone();
                child_subst.insert(var.clone(), abs);
                let child = RenderCtx {
                    names: ctx.names,
                    sidecar: ctx.sidecar,
                    abs_subst: child_subst,
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
                render_events_in(body, out, indent + 1, &child, Some(*iter_var))?;
                writeln!(out, "{pad}}}").ok();
                return Ok(());
            }

            let (lo_s, hi_s) = render_loop_bounds(*iter_var, range, ctx)?;
            writeln!(out, "{pad}for {var} in ({lo_s})..({hi_s}) {{").ok();
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
            if let Some(frame) = check_frame {
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
                render_events_in(body, out, body_indent, ctx, Some(*iter_var))?;
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
                    ViolationKind::Log | ViolationKind::Count => {
                        // log/count handlers deferred to TASK-0052.04.
                        // The projection pass currently only materialises
                        // Panic (it is also the codegen default per
                        // PRD §6.3.5), so the user can't currently reach
                        // here — but a future enabling change must NOT
                        // silently no-op the assertion. Fail loud.
                        return Err(EmitError::ContractGap(format!(
                            "on_violation={:?} for `check loop {lv}` is \
                             deferred to TASK-0052.04 — only Panic is \
                             wired in pthreads-sync codegen this cycle. \
                             Refusing to emit without the assertion.",
                            frame.on_violation,
                            lv = frame.loop_var,
                        )));
                    }
                }
            } else {
                render_events_in(body, out, body_indent, ctx, Some(*iter_var))?;
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

/// Resolve a `DataId` to its source name, failing loud on a gap.
fn data_name(did: DataId, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    ctx.names
        .data
        .get(&did)
        .cloned()
        .ok_or_else(|| EmitError::ContractGap(format!("data id {did:?} has no name in NameTables")))
}

/// Render one indexed-assignment statement (an `Event::Fire` with a
/// non-empty `bindings.output.indices`). The caller supplies the RHS
/// EXPRESSION (typically `kernels::<callee>(<args>)`) — no trailing
/// semicolon, no leading indent. We return the full Rust statement
/// ending in `;`. The two shapes (TASK-0209):
///
/// - **Scalar** (full rank): `D[idx] = <rhs>;` — single slot, byte-
///   identical to the pre-TASK-0209 emission for examples 01..07.
/// - **Sub-array** (partial prefix rank): the emitted code binds the
///   RHS to a local, runtime-asserts the length against `sub_len`
///   with a fail-loud-with-context message naming the slot, then
///   `D[start..start+sub_len].copy_from_slice(&_rhs);`. The
///   length-assert exists because the LHS `sub_len` derives from the
///   declared shape (sidecar) while the RHS length depends on the
///   kernel author's implementation. Without it, `copy_from_slice`
///   would panic with std's terse `source slice length (N) does not
///   match destination slice length (M)` message; the assert turns
///   that into a diagnostic naming the slot and expected length
///   (MPED fail-loud-with-context discipline; review-gate finding
///   cycle-2 TASK-0209). Note the assert fires AFTER the RHS is
///   evaluated, so behaviour for valid input is byte-identical when
///   kernels honour their declared return shape.
///
/// Sharing this site between the single-worker `render_main_rs`, the
/// multi-worker pthreads renderer, and the mp-tcp-bufsync renderer
/// keeps the three Fire-output sites byte-identical — no codegen
/// drift between backends, which is what the cross-backend
/// differential (PRD §10.1) ultimately rests on.
fn render_fire_output_assign(
    o: &DataSlice,
    rhs: &str,
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    let name = data_name(o.data, ctx)?;
    match classify_data_slice(o, ctx)? {
        SliceForm::Scalar(idx) => Ok(format!("{name}[{idx}] = {rhs};")),
        SliceForm::SubArray { start, sub_len } => Ok(format!(
            "{{ let _rhs = {rhs}; \
             assert_eq!(_rhs.len(), {sub_len}usize, \
             \"kernel result for `{name}` slot returned {{}} elements, declared shape requires {{}}\", \
             _rhs.len(), {sub_len}usize); \
             {name}[{start}..{start} + {sub_len}usize].copy_from_slice(&_rhs); }}"
        )),
    }
}

/// Render a kernel call's argument list from its [`FireBinding`]
/// inputs. `Data` → indexed/whole-array read; `Scalar` → integer
/// expression with a param-type cast decided via the SIDECAR's
/// kernel signature (TASK-0169, AlgoIR-free); `Nested` → rejected
/// (tier-1 backends do not lower a nested call in argument position).
fn render_fire_args(
    kernel: KernelId,
    inputs: &[ArgBinding],
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    // Per-param types come from the sidecar's kernel signature, NOT
    // `algo.kernels` (AC#2). Absent only if the contract regressed.
    let sig = ctx.sidecar.kernel_sig(kernel);
    let mut parts = Vec::with_capacity(inputs.len());
    for (i, arg) in inputs.iter().enumerate() {
        let param_ty = sig.and_then(|s| s.params.get(i));
        parts.push(render_fire_arg(arg, param_ty, ctx)?);
    }
    Ok(parts.join(", "))
}

fn render_fire_arg(
    arg: &ArgBinding,
    param_ty: Option<&ResolvedType>,
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    match arg {
        ArgBinding::Data(s) => {
            if s.indices.is_empty() {
                // Whole-array argument (passed by move per
                // single-assignment). If the param is scalar this is
                // a program bug; emit the bare name and let rustc
                // catch it loudly (same as the old backend).
                data_name(s.data, ctx)
            } else {
                // Classify scalar vs sub-array based on rank match
                // (TASK-0209). Sidecar `dims` is the single source of
                // truth for the data's declared shape — fewer indices
                // than dims = a contiguous trailing sub-array, NOT a
                // scalar. The old code emitted `name[idx]` (scalar)
                // unconditionally; for a rank-4 `f32[B][C0][H][W]`
                // accessed with one outer index this passed `f32`
                // where the kernel signature expected `Vec<f32>` and
                // cargo build failed E0308 (example 13 reproducer).
                let name = data_name(s.data, ctx)?;
                match classify_data_slice(s, ctx)? {
                    SliceForm::Scalar(idx) => Ok(format!("{name}[{idx}]")),
                    SliceForm::SubArray { start, sub_len } => {
                        // Owned `Vec<T>` matches `rust_type_of` for an
                        // aggregate kernel param (`Vec<T>`); the call
                        // moves it, consistent with the whole-array
                        // case above. PRD §6.2.1 single-assignment
                        // permits the move semantics.
                        //
                        // The literal `{sub_len}usize` makes the upper
                        // bound a `usize` so the `start..start+sub_len`
                        // range typechecks (`start` is already `as
                        // usize` from `classify_data_slice`).
                        Ok(format!(
                            "{name}[{start}..{start} + {sub_len}usize].to_vec()"
                        ))
                    }
                }
            }
        }
        ArgBinding::Scalar(e) => {
            let rendered = render_int_expr(e, &ctx.abs_subst)?;
            // Iter-var-derived scalars are typed i64; a scalar kernel
            // param needs a cast. Param type from the sidecar.
            if let Some(pty) = param_ty {
                if pty.is_scalar() {
                    return Ok(format!("({rendered}) as {}", rust_scalar_type(&pty.scalar)));
                }
            }
            Ok(rendered)
        }
        ArgBinding::Nested { .. } => Err(EmitError::UnsupportedFeature(
            "nested kernel call inside an argument expression".to_string(),
        )),
    }
}

/// The two shapes an indexed [`DataSlice`] lowers to in the flat-Vec
/// layout (TASK-0209). Row-major: a PREFIX of the dims being indexed
/// leaves a CONTIGUOUS trailing region — that's a sub-array. Equal
/// rank between indices and dims is a single scalar slot.
///
/// Non-prefix partial access (e.g. fix the inner dim, leave the outer
/// free) is NOT contiguous in row-major and cannot be produced by the
/// current grammar — `IndexedLValue.indices` (`algo/ast.rs`) is a
/// positional `Vec<SpExpr>` with no skip-marker, and the parser
/// grammar `IDENT ('[' EXPR ']')*` always indexes outer dims first.
/// `classify_data_slice` trusts that grammar floor and does not
/// defensively reject; if a future IR or surface-syntax change adds
/// skip-indexing, the classifier must be extended with a
/// non-contiguous gather emission path (review-gate finding,
/// cycle-2 TASK-0209).
enum SliceForm {
    /// Full-rank access — single slot in the flat `Vec<T>`.
    Scalar(String),
    /// Partial-prefix access — contiguous sub-slice
    /// `[start .. start + sub_len]` of the flat `Vec<T>`.
    SubArray { start: String, sub_len: usize },
}

/// Decide whether an indexed [`DataSlice`] is a scalar (full rank) or
/// a contiguous prefix sub-array (partial rank), and render the
/// flat-Vec coordinates.
///
/// Caller is responsible for `s.indices.is_empty() == false` (a
/// whole-array reference has no index expression to lower — its
/// argument-site rendering is the bare name).
///
/// The classification uses the sidecar `dims` ONLY: it is the single
/// source of truth for declared shape (AlgoIR-free path; AC#2 of
/// TASK-0124 still holds — no `algo.data` lookup).
fn classify_data_slice(s: &DataSlice, ctx: &RenderCtx<'_>) -> Result<SliceForm, EmitError> {
    debug_assert!(!s.indices.is_empty(), "classify_data_slice requires indices");
    let name = data_name(s.data, ctx)?;
    let ty = ctx.sidecar.data_type(s.data).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "data `{name}` ({:?}) used with a {}-D index has no ResolvedType \
             in the NameSidecar",
            s.data,
            s.indices.len()
        ))
    })?;
    let dims = &ty.dims;
    if s.indices.len() > dims.len() {
        // Over-indexed: a real bug upstream of the backend (the
        // contract pass should reject this). Fail LOUD with context.
        return Err(EmitError::UnsupportedFeature(format!(
            "data `{name}` over-indexed (sidecar dims={dims:?}, \
             indices={}); contract pass should have rejected",
            s.indices.len()
        )));
    }
    if dims.is_empty() {
        // Scalar data with at least one index — also a contract bug.
        return Err(EmitError::UnsupportedFeature(format!(
            "scalar data `{name}` indexed with {} expressions",
            s.indices.len()
        )));
    }
    // Special-case `indices.len() == 1`: the pre-TASK-0209
    // `render_flat_index` 1D fast-path emitted `({i0}) as usize` (one
    // paren level, no stride factor since `stride == dims[1..].prod()`).
    // Preserve that exact spelling for examples 01..07 (load-bearing
    // for byte-identical determinism on the existing matrix); the
    // partial-rank-1 case (rank-1 index on rank>=2 data, e.g. example
    // 13's `input[n]`) ALSO uses this scalar `(i0)` form for the start
    // expression, with `sub_len = product(dims[1..])` carrying the
    // trailing extent.
    if s.indices.len() == 1 {
        let i0 = render_int_expr(&s.indices[0], &ctx.abs_subst)?;
        let expr = format!("({i0}) as usize");
        return if dims.len() == 1 {
            Ok(SliceForm::Scalar(expr))
        } else {
            // Partial outer index into rank-N data (N>=2). The flat
            // start is `i0 * product(dims[1..])`; reuse the scalar
            // spelling for the index expression and multiply by the
            // sub_len when emitting the range. To keep the start
            // expression cheap and byte-identical to a hand-multiply,
            // bake the stride into `start` here.
            let sub_len: usize = dims[1..].iter().copied().product();
            let start = if sub_len == 1 {
                expr
            } else {
                format!("(({i0}) * {sub_len}) as usize")
            };
            Ok(SliceForm::SubArray { start, sub_len })
        };
    }
    // Multi-dim case (indices.len() >= 2). Row-major stride for the
    // full or partial PREFIX: index k contributes
    // `(i_k) * (D_{k+1} * .. * D_{n-1})`. For partial rank, sub_len
    // is `product(dims[indices.len()..])`. For full rank, sub_len's
    // product is 1 (empty product) and the sum is the scalar flat
    // index — byte-identical to the pre-TASK-0209 `render_flat_index`
    // multi-dim path.
    let mut terms: Vec<String> = Vec::with_capacity(s.indices.len());
    for (k, idx_expr) in s.indices.iter().enumerate() {
        let stride: usize = dims[k + 1..].iter().copied().product();
        let rendered = render_int_expr(idx_expr, &ctx.abs_subst)?;
        if stride == 1 {
            terms.push(format!("({rendered})"));
        } else {
            terms.push(format!("({rendered}) * {stride}"));
        }
    }
    let expr = format!("({}) as usize", terms.join(" + "));
    if s.indices.len() == dims.len() {
        Ok(SliceForm::Scalar(expr))
    } else {
        let sub_len: usize = dims[s.indices.len()..].iter().copied().product();
        Ok(SliceForm::SubArray {
            start: expr,
            sub_len,
        })
    }
}

/// Render a flat (row-major) index for a 1D `Vec<T>` of an
/// N-dimensional shape. 1D → `(i0) as usize`. Higher rank → strides
/// from the sidecar's `dims` (NOT `algo.data`): `(i0*D1*D2 + i1*D2 +
/// i2) as usize`. Mirrors the old `render_flat_index` exactly.
fn render_flat_index(s: &DataSlice, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    if s.indices.is_empty() {
        return Err(EmitError::UnsupportedFeature(
            "render_flat_index called on a non-indexed reference".to_string(),
        ));
    }
    if s.indices.len() == 1 {
        let i0 = render_int_expr(&s.indices[0], &ctx.abs_subst)?;
        return Ok(format!("({i0}) as usize"));
    }
    let name = data_name(s.data, ctx)?;
    let ty = ctx.sidecar.data_type(s.data).ok_or_else(|| {
        EmitError::ContractGap(format!(
            "data `{name}` ({:?}) used with a {}-D index has no ResolvedType \
             in the NameSidecar",
            s.data,
            s.indices.len()
        ))
    })?;
    let dims = &ty.dims;
    if dims.len() != s.indices.len() {
        return Err(EmitError::UnsupportedFeature(format!(
            "data `{name}` rank/shape mismatch with index list \
             (sidecar dims={dims:?}, indices={})",
            s.indices.len()
        )));
    }
    // Row-major: i0 * D1*D2*..*Dn + i1 * D2*..*Dn + ... + i_{n-1}.
    let mut terms: Vec<String> = Vec::with_capacity(s.indices.len());
    for (k, idx_expr) in s.indices.iter().enumerate() {
        let stride: usize = dims[k + 1..].iter().copied().product();
        let rendered = render_int_expr(idx_expr, &ctx.abs_subst)?;
        if stride == 1 {
            terms.push(format!("({rendered})"));
        } else {
            terms.push(format!("({rendered}) * {stride}"));
        }
    }
    Ok(format!("({}) as usize", terms.join(" + ")))
}

/// Render an integer-valued index/scalar expression as Rust.
/// Identifiers (iter vars) are emitted verbatim UNLESS they are an
/// active absolute-index substitution (a strip-mined inner-block
/// loop var → `(LO + tile*N + inner)`). The map is empty for every
/// non-blocked program, so this is byte-identical to the old
/// `render_int_expr` there.
fn render_int_expr(e: &IrExpr, subst: &BTreeMap<String, String>) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}")),
        IrExpr::Ident(n) => Ok(match subst.get(n) {
            Some(repl) => repl.clone(),
            None => n.clone(),
        }),
        IrExpr::Neg(inner) => Ok(format!("-({})", render_int_expr(inner, subst)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_int_expr(l, subst)?;
            let rs = render_int_expr(r, subst)?;
            Ok(format!("({ls} {} {rs})", bin_op_str(op)))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside an integer index expression".to_string(),
        )),
    }
}

/// The `(lo, hi)` source strings for an `Event::Loop`.
///
/// If the iter var has a `sidecar.loop_bounds` entry it is a SOURCE
/// `for` loop — render the *unevaluated* bound through
/// `render_const_expr` + `sidecar.consts` so `for y : 1 .. H-1`
/// (H=16) becomes `1_i64 .. (16_i64 - 1_i64)`, byte-identical to the
/// old AlgoIR-walking backend.
///
/// If there is NO `loop_bounds` entry the loop is a
/// `block_transform`-synthesised tile loop with no source form — use
/// the concrete `Event::Loop.range`, rendered `{n}_i64` (matching the
/// old backend, which rendered the ACFG `Repeat.range` literals for
/// synthesised loops the same way).
fn render_loop_bounds(
    iter_var: IterVar,
    range: &std::ops::Range<i64>,
    ctx: &RenderCtx<'_>,
) -> Result<(String, String), EmitError> {
    match ctx.sidecar.loop_bounds.get(&iter_var) {
        Some(b) => {
            let lo = render_const_expr(&b.lo, ctx)?;
            let hi = render_const_expr(&b.hi, ctx)?;
            Ok((lo, hi))
        }
        None => {
            // Synthesised tile loop: concrete folded range, same
            // spelling as an integer literal const bound.
            Ok((format!("{}_i64", range.start), format!("{}_i64", range.end)))
        }
    }
}

/// Render a *constant* loop-bound expression, resolving const idents
/// to their value via the SIDECAR's const table (NOT `algo.consts`,
/// AC#2). Mirrors the old `render_const_expr` spelling exactly:
/// `IntLit(v)` → `{v}_i64`, a const ident → `{value}_i64`, an
/// outer-loop iter var → bare name, `BinOp` → `({l} op {r})`.
fn render_const_expr(e: &IrExpr, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}_i64")),
        IrExpr::Ident(n) => {
            if let Some(c) = ctx.sidecar.consts.get(n) {
                Ok(format!("{}_i64", c.value))
            } else if let Some(repl) = ctx.abs_subst.get(n) {
                // A rebound strip-mined outer iter var referenced in
                // an inner loop bound: use its absolute expression.
                // (Empty for every non-blocked program -> the old
                // bare-name behaviour, byte-identical.)
                Ok(repl.clone())
            } else {
                // An outer loop's iter var: render as-is, rely on
                // Rust to type-check (mirrors the old backend).
                Ok(n.clone())
            }
        }
        IrExpr::Neg(inner) => Ok(format!("-({})", render_const_expr(inner, ctx)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_const_expr(l, ctx)?;
            let rs = render_const_expr(r, ctx)?;
            Ok(format!("({ls} {} {rs})", bin_op_str(op)))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside a const expression (loop bound)".to_string(),
        )),
    }
}

fn bin_op_str(op: &IrBinOp) -> &'static str {
    match op {
        IrBinOp::Add => "+",
        IrBinOp::Sub => "-",
        IrBinOp::Mul => "*",
        IrBinOp::Div => "/",
        IrBinOp::Mod => "%",
    }
}

/// Rust spelling of a Nuc `ScalarType`. Shared with multi_worker.
pub(crate) fn rust_scalar_type(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::Usize => "usize",
        ScalarType::Isize => "isize",
        ScalarType::U8 => "u8",
        ScalarType::U16 => "u16",
        ScalarType::U32 => "u32",
        ScalarType::U64 => "u64",
        ScalarType::I8 => "i8",
        ScalarType::I16 => "i16",
        ScalarType::I32 => "i32",
        ScalarType::I64 => "i64",
        ScalarType::F32 => "f32",
        ScalarType::F64 => "f64",
        ScalarType::Bool => "bool",
    }
}

// --------------------------------------------------------------------
// Crate-internal re-exports for the multi_worker module
// --------------------------------------------------------------------
//
// The multi-worker emitter shares the EXACT same expression / call /
// index / bound rendering as the single-worker emitter so the two
// paths cannot drift (the byte-identical invariant must hold for both
// 02-naive single-worker and 02-split multi-worker). Rather than
// duplicate the renderers (the pre-TASK-0124 code had two divergent
// copies), `multi_worker` calls these thin `pub(crate)` shims over
// the single private implementations above.

/// `RenderCtx` re-exported under a crate-visible name for
/// `multi_worker` AND for sibling EventList-consuming backends
/// (`mp-tcp-bufsync`, TASK-0036). Exposing this `pub` (was
/// `pub(crate)`) is deliberate: it is the *single shared
/// implementation* of expression / index / kernel-call / loop-bound
/// rendering. The drift risk TASK-0124 flagged ("two divergent
/// copies of the renderers") is structurally prevented by having the
/// second backend call THESE shims rather than re-mirror them. The
/// single-worker emitter ([`render_main_rs`]) is also `pub` for the
/// same reason: a multi-process backend running a 0/1-worker schedule
/// emits a byte-identical single process.
///
/// These pub shims (`render_fire_args_pub` / `render_flat_index_pub`
/// / `render_const_expr_pub`) render expressions / indices / bounds
/// with an EMPTY `abs_subst`: they are used by `multi_worker` /
/// `mp-tcp-bufsync` only for the worker-loop scaffolding (slot types,
/// barrier-free bound rendering) where no strip-mine rebinding is in
/// scope. The single-worker `render_single_worker_main` path (which
/// BOTH backends use for a 0/1-worker schedule — see
/// `mp-tcp-bufsync::emit`) does the per-occurrence `Event::Loop`
/// rebinding from `block_tag` via the full `RenderCtx`, so a blocked
/// single-host schedule (04/05/06/07) is correct on both backends
/// through that shared path. A blocked *multi*-worker schedule would
/// thread the same tag-driven rebinding (the renderers are shared —
/// one implementation, no drift); none exists in the tier-1 set.
pub struct RenderCtxPub<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
}

impl<'a> RenderCtxPub<'a> {
    pub fn new(names: &'a NameTables, sidecar: &'a NameSidecar) -> Self {
        RenderCtxPub { names, sidecar }
    }

    fn inner(&self) -> RenderCtx<'_> {
        RenderCtx {
            names: self.names,
            sidecar: self.sidecar,
            abs_subst: BTreeMap::new(),
        }
    }
}

pub fn render_fire_args_pub(
    kernel: KernelId,
    inputs: &[ArgBinding],
    ctx: &RenderCtxPub<'_>,
) -> Result<String, EmitError> {
    render_fire_args(kernel, inputs, &ctx.inner())
}

pub fn render_flat_index_pub(
    s: &DataSlice,
    ctx: &RenderCtxPub<'_>,
) -> Result<String, EmitError> {
    render_flat_index(s, &ctx.inner())
}

/// Public shim for the shared Fire-output assignment renderer
/// (TASK-0209). `mp-tcp-bufsync` and the pthreads-sync multi-worker
/// path call through this so all three indexed-assignment sites use
/// ONE implementation — no codegen drift between backends, which the
/// cross-backend bit-identical differential (PRD §10.1) depends on.
pub fn render_fire_output_assign_pub(
    o: &DataSlice,
    rhs: &str,
    ctx: &RenderCtxPub<'_>,
) -> Result<String, EmitError> {
    render_fire_output_assign(o, rhs, &ctx.inner())
}

pub fn render_const_expr_pub(
    e: &IrExpr,
    ctx: &RenderCtxPub<'_>,
) -> Result<String, EmitError> {
    render_const_expr(e, &ctx.inner())
}

/// Render a single worker's straight-line `main.rs` body — the SAME
/// renderer the single-process pthreads-sync path uses. A
/// multi-process backend whose schedule has 0/1 used workers emits
/// exactly one process; reusing this keeps that process's code
/// byte-identical to pthreads-sync's, so the differential on
/// single-worker examples is real, not coincidental. Public for
/// `mp-tcp-bufsync` (TASK-0036).
pub fn render_single_worker_main(
    events: &[Event],
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    render_main_rs(events, names, sidecar)
}

/// `vec![<zero>; product(dims)]` (array) or the scalar zero literal,
/// sized + typed entirely from the sidecar `ResolvedType`. Shared so
/// the mp-tcp pre-init allocation matches pthreads-sync exactly.
pub fn render_array_init_for(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_zero(&ty.scalar).to_string()
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        format!("vec![{zero}; {total}]")
    }
}

/// Rust surface type for a `ResolvedType`: scalars natural, arrays
/// flatten to `Vec<T>`. Shared with mp-tcp so slot/buffer typing
/// cannot drift from pthreads-sync.
pub fn rust_type_of(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_type(&ty.scalar).to_string()
    } else {
        format!("Vec<{}>", rust_scalar_type(&ty.scalar))
    }
}

/// Public spelling of the Rust scalar type (was `pub(crate)`).
pub fn rust_scalar_type_pub(t: &ScalarType) -> &'static str {
    rust_scalar_type(t)
}

// (No `render_int_expr_pub`: `multi_worker` renders args/indices via
// the higher-level `render_fire_args_pub` / `render_flat_index_pub`
// shims, which call the shared `render_int_expr` internally — one
// implementation, no second copy to drift.)

// --------------------------------------------------------------------
// File-write helper
// --------------------------------------------------------------------

fn write_file(path: &Path, contents: &str) -> Result<(), EmitError> {
    fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
