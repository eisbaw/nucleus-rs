//! pthreads-sync backend. PRD §7.1, TASK-0020.
//!
//! Tier-1 CPU backend that takes a post-injection ACFG plus the
//! [`LinkedIR`] and emits a standalone Cargo project containing
//! runnable Rust. M1 scope: single-worker ("naive schedule") only.
//! Multi-worker codegen (`std::thread::spawn` + `std::sync::Condvar`)
//! is structurally accommodated by the [`emit`] API and the file
//! layout but not exercised end-to-end yet — TASK-0020 follow-ups
//! cover it.
//!
//! ## Why the backend consumes [`LinkedIR`] in addition to the ACFG
//!
//! The ACFG (TASK-0016 + sync/transfer injection) is the correct
//! single source of truth for execution order and worker placement.
//! But it deliberately drops a few things the codegen needs:
//!
//! - Per-call argument shape (which kernel argument is a scalar
//!   `a[i]` vs a whole-array `Vec<T>`). The ACFG's `DataflowEdge`
//!   keeps only `data_in: Vec<DataId>` — no index expressions, no
//!   nesting. PRD §6.2.2 kernels are real Rust functions whose
//!   arguments are typed at the call site, so generating valid Rust
//!   requires walking the original call expressions.
//! - Loop iteration variable names as strings (we have `IterVar(u64)`
//!   in the ACFG and the name<->id map on `ACFG`, but it is easier to
//!   walk the source `IrStmt` tree directly and emit Rust `for i ...`
//!   directly).
//!
//! We therefore use the ACFG for invariants (every kernel has a
//! worker, transfer/sync placeholders are in place) but walk the
//! `LinkedIR::algo` IR statements for code emission. When example 2
//! (split add) lands the codegen will switch to walking the per-worker
//! projection of the ACFG; at M1 with a single worker, the two are
//! identical.
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
//! `kernels.rs` is *copied* (not `include!`-ed) into the generated
//! project. Trade-off:
//! - Copy: cheap reproducibility; the generated project is fully
//!   self-contained and can be moved out of the repo without breaking.
//!   The cost is that two files now reflect the same source-of-truth;
//!   if the user edits the original after codegen, the generated copy
//!   is stale.
//! - `include!`: lives at the original path; one source of truth, but
//!   the generated project leaks a file dependency.
//!
//! Reproducibility wins. The expected workflow is "run codegen, then
//! build". Editing kernels.rs is followed by re-running codegen.
//!
//! ## Error handling
//!
//! Failures bubble up as [`EmitError`] variants with the offending
//! path / reason attached. No silent fallbacks. The generated Rust
//! itself uses the panic semantics of the user's kernel bodies
//! (e.g. example 01's `save_output` panics on I/O failure).
//!
//! ## Honest limitations at M1
//!
//! - **Single-worker only.** A schedule with more than one `place`-d
//!   worker is rejected at emit time. Multi-worker codegen
//!   (thread spawn + condvar) is filed as a follow-up.
//! - **Aggregate I/O kernels.** The Rust signature is
//!   `() -> Vec<T>` / `(Vec<T>) -> ()`. Codegen recognises this by
//!   looking at the algorithm-declared signature being aggregate
//!   (`T[N]` shape) — and emits whole-array binding/move calls
//!   accordingly.
//! - **No const propagation into generated code.** Loop bounds are
//!   the ACFG's `Range<i64>` literals. If the algorithm wrote
//!   `for i : 0 .. N` the value of `N` is baked into the bound,
//!   which is acceptable but loses the symbolic name.
//! - **No error recovery in generated code.** A panic in any kernel
//!   aborts the whole binary.
//! - **No input.bin format negotiation.** The kernels handle I/O via
//!   their own `NUC_INPUT_PATH`/`NUC_OUTPUT_PATH` env-var contract.
//!   The generated `run.sh` sets those variables and assumes the
//!   format matches what the user's kernels.rs reads.
//! - **No identity-copy support** (`d <-- e` with a bare DataRef RHS).
//!   The link / ACFG passes already note this hole; the backend
//!   inherits it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use compiler::algo::{
    AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, Purity, ResolvedKernel, ResolvedType, ScalarType,
};
use compiler::link::LinkedIR;
use compiler::sched::ResolvedPlaceTarget;
use compiler::ACFG;

// --------------------------------------------------------------------
// Public surface
// --------------------------------------------------------------------

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
    /// Failed to read the user's `kernels.rs` (path bad, permissions,
    /// nonexistent, ...).
    KernelsReadFailed { path: PathBuf, source: io::Error },
    /// Failed to create `out_dir` or any sub-directory.
    OutputCreateFailed { path: PathBuf, source: io::Error },
    /// Failed to write a generated file.
    WriteFailed { path: PathBuf, source: io::Error },
    /// The post-injection ACFG asks for something this backend (M1)
    /// cannot yet emit. Carries a human description of what.
    UnsupportedFeature(String),
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::KernelsReadFailed { path, source } => {
                write!(
                    f,
                    "failed to read kernels.rs at {}: {}",
                    path.display(),
                    source
                )
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
                write!(f, "pthreads-sync (M1): unsupported feature: {msg}")
            }
        }
    }
}

impl std::error::Error for EmitError {}

/// Emit a runnable Cargo project for the given algorithm + schedule
/// (already linked, ACFG already built with sync + transfer injection
/// applied).
///
/// `kernels_rs_path` is the path to the user's adjacent `kernels.rs`;
/// the backend copies it verbatim into the generated project.
///
/// Returns paths to every emitted file on success. On failure, the
/// state of `out_dir` is left as-is (no rollback at M1 — the user
/// re-runs codegen).
pub fn emit(
    acfg: &ACFG,
    linked: &LinkedIR,
    kernels_rs_path: &Path,
    out_dir: &Path,
) -> Result<EmitResult, EmitError> {
    // ---- M1: enforce single-worker. ----
    //
    // The ACFG's name_workers covers every worker the schedule
    // declared (including any unused entries). We instead count the
    // unique workers that actually own at least one Operation: a
    // schedule may declare workers it doesn't use. If more than one
    // distinct worker is referenced by an Operation, bail.
    let used_workers = collect_used_workers(acfg);
    if used_workers.len() > 1 {
        return Err(EmitError::UnsupportedFeature(format!(
            "multi-worker codegen not implemented at M1 (schedule uses {} workers; expected 1)",
            used_workers.len()
        )));
    }

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
    let cargo_toml_src = render_cargo_toml();
    write_file(&cargo_toml, &cargo_toml_src)?;

    // ---- Copy kernels.rs verbatim ----
    write_file(&kernels_rs, &kernels_src)?;

    // ---- Render main.rs ----
    let main_rs_src = render_main_rs(&linked.algo)?;
    write_file(&main_rs, &main_rs_src)?;

    // ---- Render run.sh ----
    let run_sh_src = render_run_sh();
    write_file(&run_sh, &run_sh_src)?;
    // Best-effort: mark run.sh executable. Failure here is non-fatal
    // — the user can `bash run.sh` instead, and the e2e test does so.
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
// Worker analysis
// --------------------------------------------------------------------

/// Collect the union of all worker IDs that any `Operation` in the
/// ACFG runs on. A schedule may declare workers that no kernel uses;
/// those don't count.
fn collect_used_workers(acfg: &ACFG) -> BTreeSet<compiler::WorkerId> {
    let mut s: BTreeSet<compiler::WorkerId> = BTreeSet::new();
    walk_workers(&acfg.root, &mut s);
    s
}

fn walk_workers(node: &compiler::ACFGNode, out: &mut BTreeSet<compiler::WorkerId>) {
    use compiler::ACFGNode;
    match node {
        ACFGNode::Operation(op) => {
            for w in &op.workers {
                out.insert(*w);
            }
        }
        ACFGNode::Repeat { body, .. } => walk_workers(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                walk_workers(c, out);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

// --------------------------------------------------------------------
// File renderers
// --------------------------------------------------------------------

fn render_cargo_toml() -> String {
    // Standalone, no parent workspace, no third-party deps. Panic
    // abort matches the reference implementation for tier-1
    // determinism.
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
    // The script lives in the project root next to Cargo.toml. It
    // builds the binary in release mode (panic=abort, no debug info
    // for size) and runs it pointing NUC_INPUT_PATH and
    // NUC_OUTPUT_PATH at the conventional sibling files.
    //
    // `set -euo pipefail` aborts on any error; matches the
    // fail-loud rule from PRD §10 (a botched run shouldn't return
    // success).
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
// Algorithm -> main.rs codegen
// --------------------------------------------------------------------

fn render_main_rs(algo: &AlgoIR) -> Result<String, EmitError> {
    // Emit a self-contained `main.rs` that walks the algorithm's
    // statements in source order. Single-worker host, no transfers,
    // no syncs (the post-injection ACFG of a naive schedule has no
    // Sync/Xfer nodes), so the lowering is mechanical.
    //
    // Strategy:
    // - For each data declaration we make a binding when it is first
    //   assigned. There are two cases:
    //   1. Whole-array assignment `D <-- aggregate_call()` — the call
    //      returns a `Vec<T>`; we `let mut D = kernels::call();`.
    //   2. Indexed assignment `D[i] <-- scalar_call(...)` — D must
    //      already exist (the algorithm requires this; single-
    //      assignment plus indexed-LHS implies a prior whole-array
    //      `<--` or a default initialisation). At M1 we initialise
    //      indexed-LHS data ahead-of-time with `vec![T::default(); N]`
    //      sized by the algorithm-declared shape.
    // - For effect statements `kernel(args)`: emit a call, passing
    //   aggregate-typed args by move (which is OK because no
    //   subsequent statement reads them — single-assignment
    //   guarantees no aliasing). If a real example needs to read an
    //   aggregate after passing it to an effect, the codegen tightens.
    // - For-loops emit `for i in lo..hi`. Loop bounds are i64 in the
    //   IR; we cast the iter-var to usize when used as an array
    //   index.

    let mut out = String::new();
    writeln!(
        out,
        "//! Generated by the pthreads-sync backend (TASK-0020, M1)."
    )
    .ok();
    writeln!(out, "//! Do not edit; rerun `nucleus build` to regenerate.").ok();
    writeln!(out).ok();
    writeln!(out, "// The user's kernel bodies live in kernels.rs.").ok();
    writeln!(out, "mod kernels;").ok();
    writeln!(out).ok();
    // unused_mut: the codegen attaches `let mut` to every dataflow
    // binding for safety (an indexed-LHS assignment, an effect-pass
    // by move, etc. may follow). Some bindings end up never mutated;
    // suppressing the warning is cleaner than threading mutation
    // analysis through the renderer at M1.
    //
    // dead_code: kernels.rs may declare helper items that this
    // particular algorithm/schedule never calls (e.g. example 01's
    // `read_i32_le_slice` is hit indirectly through `load_input`).
    // The user's source is the source of truth; we don't prune.
    writeln!(out, "#[allow(unused_mut, dead_code, unused_variables)]").ok();
    writeln!(out, "fn main() {{").ok();

    // Pre-initialise every `data` symbol that the algorithm will
    // assign via an indexed LHS (`D[i] <-- ...`). Whole-array binding
    // (D <-- aggregate_call()) takes care of itself at the statement.
    //
    // We need this BEFORE walking the statements so that loop bodies
    // (which use `D[i]` for some D never assigned whole-array) see a
    // valid binding.
    let pre_init = collect_pre_init_data(algo);
    for (name, ty) in &pre_init {
        let rs_init = render_array_init(ty);
        writeln!(out, "    let mut {name} = {rs_init};").ok();
    }
    if !pre_init.is_empty() {
        writeln!(out).ok();
    }

    let ctx = RenderCtx { algo };
    render_stmts(&algo.stmts, &mut out, 1, &ctx)?;

    writeln!(out, "}}").ok();
    Ok(out)
}

struct RenderCtx<'a> {
    algo: &'a AlgoIR,
}

/// Find every `data` symbol that is assigned *only* via indexed
/// dataflow (`D[i] <-- ...`) — never whole-array `D <-- aggregate()`.
/// Those need an up-front allocation so the index assignment has
/// somewhere to land.
///
/// Returns name -> resolved-shape pairs in lexicographic order so
/// generated output is deterministic.
fn collect_pre_init_data(algo: &AlgoIR) -> Vec<(String, ResolvedType)> {
    let mut whole_array: BTreeSet<String> = BTreeSet::new();
    let mut indexed: BTreeSet<String> = BTreeSet::new();
    walk_assign_kinds(&algo.stmts, &mut whole_array, &mut indexed);

    let mut out: Vec<(String, ResolvedType)> = Vec::new();
    for name in indexed.iter() {
        if whole_array.contains(name) {
            continue;
        }
        if let Some(d) = algo.data.get(name) {
            out.push((name.clone(), d.ty.clone()));
        }
    }
    out
}

fn walk_assign_kinds(
    stmts: &[IrStmt],
    whole_array: &mut BTreeSet<String>,
    indexed: &mut BTreeSet<String>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, .. } => {
                if lhs.indices.is_empty() {
                    whole_array.insert(lhs.name.clone());
                } else {
                    indexed.insert(lhs.name.clone());
                }
            }
            IrStmt::Effect { .. } => {}
            IrStmt::For { body, .. } => walk_assign_kinds(body, whole_array, indexed),
        }
    }
}

/// Render `vec![T::default(); N1 * N2 * ...]` for an array type, or a
/// scalar zero for a scalar type. M1: only 1D arrays are exercised by
/// example 01. Higher rank gets flattened to a 1D Vec, which keeps
/// the codegen simple and matches the `Vec<T>` convention in
/// example 01's kernels.rs. If a future example wants `[[T; W]; H]`,
/// the codegen needs to learn the multi-dim form (filed as a
/// follow-up).
fn render_array_init(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        // Scalar `data` is uncommon (effect outputs only) but we
        // support it. Defaults are zero for numeric types and false
        // for bool. The rust_scalar_zero helper returns a literal.
        rust_scalar_zero(&ty.scalar).to_string()
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        format!("vec![{zero}; {total}]")
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
// Statement and expression rendering
// --------------------------------------------------------------------

fn render_stmts(
    stmts: &[IrStmt],
    out: &mut String,
    indent: usize,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    for s in stmts {
        render_stmt(s, out, indent, ctx)?;
    }
    Ok(())
}

fn render_stmt(
    stmt: &IrStmt,
    out: &mut String,
    indent: usize,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    let pad = "    ".repeat(indent);
    match stmt {
        IrStmt::Dataflow { lhs, rhs } => {
            render_dataflow(lhs, rhs, out, &pad, ctx)?;
        }
        IrStmt::Effect { callee, args } => {
            // Effect statement: a kernel call discarded for its side
            // effects. Render args with the same logic as a Call RHS;
            // semicolon-terminate.
            let rendered_args = render_call_args(callee, args, ctx)?;
            writeln!(out, "{pad}kernels::{callee}({rendered_args});").ok();
        }
        IrStmt::For { var, lo, hi, body } => {
            let lo_s = render_const_expr(lo, ctx)?;
            let hi_s = render_const_expr(hi, ctx)?;
            // Iteration variables are i64 to match IterTile's range
            // element type. The body uses usize-casts where indexing.
            writeln!(out, "{pad}for {var} in ({lo_s})..({hi_s}) {{").ok();
            render_stmts(body, out, indent + 1, ctx)?;
            writeln!(out, "{pad}}}").ok();
        }
    }
    Ok(())
}

fn render_dataflow(
    lhs: &IndexedRef,
    rhs: &IrExpr,
    out: &mut String,
    pad: &str,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    match rhs {
        IrExpr::Call { callee, args } => {
            let rendered_args = render_call_args(callee, args, ctx)?;
            if lhs.indices.is_empty() {
                // Whole-array (or scalar) binding. New `let mut`.
                // Reusing `let mut` is robust against later effect
                // statements that need to read the binding.
                writeln!(
                    out,
                    "{pad}let mut {} = kernels::{callee}({rendered_args});",
                    lhs.name
                )
                .ok();
            } else {
                // Indexed assignment. The pre-init pass has ensured
                // the data exists as `Vec<T>` (1D flat layout).
                let idx = render_flat_index(lhs, ctx)?;
                writeln!(
                    out,
                    "{pad}{}[{idx}] = kernels::{callee}({rendered_args});",
                    lhs.name
                )
                .ok();
            }
            Ok(())
        }
        // Identity-copy (`d <-- e` with bare data ref) and other
        // shapes: link / ACFG already note this hole.
        other => Err(EmitError::UnsupportedFeature(format!(
            "dataflow RHS shape not supported at M1: {other:?}"
        ))),
    }
}

/// Render the argument list of a kernel call. Each argument has one
/// of these forms:
///
/// - `D[i]` (scalar element read) -> `D[(i) as usize]`.
/// - `D` bare (aggregate read) -> `D` if the kernel parameter is
///   aggregate; passed by move per single-assignment.
/// - Arithmetic on iter vars / consts: render directly (the result
///   will be used as a scalar argument or as a flat index inside a
///   nested DataRef).
fn render_call_args(
    callee: &str,
    args: &[IrExpr],
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    let kernel = ctx.algo.kernels.get(callee).ok_or_else(|| {
        EmitError::UnsupportedFeature(format!(
            "kernel `{callee}` not in AlgoIR (link should have caught)"
        ))
    })?;
    let mut parts = Vec::with_capacity(args.len());
    for (i, arg) in args.iter().enumerate() {
        let param_ty = kernel.params.get(i);
        parts.push(render_call_arg(arg, param_ty, ctx)?);
    }
    Ok(parts.join(", "))
}

fn render_call_arg(
    arg: &IrExpr,
    param_ty: Option<&ResolvedType>,
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    match arg {
        IrExpr::DataRef(r) => {
            if r.indices.is_empty() {
                // Whole-array argument. If the param is scalar, this
                // is a bug; we render the bare name and let rustc
                // catch the type mismatch loudly.
                Ok(r.name.clone())
            } else {
                // Element read. Flatten indices for a Vec<T> layout.
                let idx = render_flat_index(r, ctx)?;
                Ok(format!("{}[{idx}]", r.name))
            }
        }
        // For scalar value arguments, the IR may carry a scalar arith
        // expression (an iter-var-derived literal). Render it as-is
        // with type-coercion when feeding usize-typed parameters.
        IrExpr::IntLit(_) | IrExpr::Ident(_) | IrExpr::Neg(_) | IrExpr::BinOp(_, _, _) => {
            let rendered = render_int_expr(arg)?;
            // If the kernel param is scalar and not `usize`, the
            // value comes from iter vars (which we type as i64) — we
            // need a cast.
            if let Some(pty) = param_ty {
                if pty.is_scalar() {
                    return Ok(format!("({rendered}) as {}", rust_scalar_type(&pty.scalar)));
                }
            }
            Ok(rendered)
        }
        IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "nested kernel call inside an argument expression".to_string(),
        )),
    }
}

/// Render a flat (row-major) index expression for a 1D Vec<T>
/// representation of an N-dimensional shape. For 1D shapes (which is
/// all M1 exercises) the result is just the single index expression,
/// cast to `usize`. For higher-rank, returns
/// `(i0 * D1 * D2 + i1 * D2 + i2) as usize` etc. Each index is rendered
/// as the underlying integer expression; the cast is applied to the
/// whole sum.
///
/// We fall back gracefully on data that has no resolved shape: the
/// index is rendered as-is with a `as usize` cast.
fn render_flat_index(r: &IndexedRef, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    if r.indices.is_empty() {
        return Err(EmitError::UnsupportedFeature(
            "render_flat_index called on a non-indexed reference".to_string(),
        ));
    }
    if r.indices.len() == 1 {
        let i0 = render_int_expr(&r.indices[0])?;
        return Ok(format!("({i0}) as usize"));
    }
    let shape = ctx.algo.data.get(&r.name).map(|d| d.ty.dims.clone());
    let dims = match shape {
        Some(d) if d.len() == r.indices.len() => d,
        _ => {
            return Err(EmitError::UnsupportedFeature(format!(
                "data `{}` rank/shape mismatch with index list",
                r.name
            )));
        }
    };
    // Row-major: i0 * D1*D2*..*Dn + i1 * D2*..*Dn + ... + i_{n-1}.
    let mut terms: Vec<String> = Vec::with_capacity(r.indices.len());
    for (k, idx_expr) in r.indices.iter().enumerate() {
        let stride: usize = dims[k + 1..].iter().copied().product();
        let rendered = render_int_expr(idx_expr)?;
        if stride == 1 {
            terms.push(format!("({rendered})"));
        } else {
            terms.push(format!("({rendered}) * {stride}"));
        }
    }
    Ok(format!("({}) as usize", terms.join(" + ")))
}

/// Render an integer-valued IR expression (no DataRef, no Call) as
/// Rust. Identifiers are emitted verbatim (they refer to either
/// const declarations — names mirrored as Rust `let` bindings in
/// generated code, but at M1 we don't actually emit bindings for
/// consts, instead we resolve the const value through
/// [`AlgoIR::consts`] before reaching codegen) or iteration variables
/// (which exist in the Rust scope by the same name).
///
/// At M1 we resolve consts here too: if the identifier is a declared
/// const, substitute its value as a literal. This avoids needing to
/// emit a `const NAME: ... = ...;` for every algorithm-level const.
fn render_int_expr(e: &IrExpr) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}")),
        IrExpr::Ident(n) => Ok(n.clone()),
        IrExpr::Neg(inner) => Ok(format!("-({})", render_int_expr(inner)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_int_expr(l)?;
            let rs = render_int_expr(r)?;
            let op_s = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
            };
            Ok(format!("({ls} {op_s} {rs})"))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside an integer index expression".to_string(),
        )),
    }
}

/// Render a *constant* IR expression (loop bounds, etc.) as Rust.
/// Resolves const identifiers to their literal values so the
/// generated code does not depend on Nuc-side consts being mirrored
/// into Rust.
fn render_const_expr(e: &IrExpr, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}_i64")),
        IrExpr::Ident(n) => {
            if let Some(c) = ctx.algo.consts.get(n) {
                Ok(format!("{}_i64", c.value))
            } else {
                // Could be an iteration variable of an outer loop;
                // render as-is and rely on Rust to type-check.
                Ok(n.clone())
            }
        }
        IrExpr::Neg(inner) => Ok(format!("-({})", render_const_expr(inner, ctx)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_const_expr(l, ctx)?;
            let rs = render_const_expr(r, ctx)?;
            let op_s = match op {
                IrBinOp::Add => "+",
                IrBinOp::Sub => "-",
                IrBinOp::Mul => "*",
                IrBinOp::Div => "/",
                IrBinOp::Mod => "%",
            };
            Ok(format!("({ls} {op_s} {rs})"))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside a const expression (loop bound)".to_string(),
        )),
    }
}

/// Rust spelling of a Nuc `ScalarType`. Used for argument casts when
/// passing iter-var-derived integers into kernel parameters.
fn rust_scalar_type(t: &ScalarType) -> &'static str {
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
// File-write helper
// --------------------------------------------------------------------

fn write_file(path: &Path, contents: &str) -> Result<(), EmitError> {
    fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}

// --------------------------------------------------------------------
// Unused-import suppression
// --------------------------------------------------------------------
//
// `Purity` and `ResolvedKernel` are pulled in from compiler::algo so a
// later expansion of the codegen (e.g. validating that effect-typed
// statements call effectful kernels) can use them without re-editing
// imports. Currently unused. Touch them so unused-import lint stays
// silent. Cheap "intent preserving" use; remove these lines when a
// real consumer lands.
const _: fn(&Purity, &ResolvedKernel, &ResolvedPlaceTarget) = |_p, _k, _t| {};
