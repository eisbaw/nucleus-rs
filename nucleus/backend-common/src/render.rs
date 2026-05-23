//! Shared expression / index / kernel-call / loop-bound / type
//! renderers (TASK-0244; moved here from `pthreads-sync` where they
//! first lived as `pub(crate)` shims + `pub` wrappers, accumulating
//! via TASK-0124 / TASK-0156 / TASK-0169 / TASK-0180 / TASK-0209 /
//! TASK-0222).
//!
//! # Single source of truth
//!
//! Every tier-1 backend (pthreads-sync straight-line + multi-worker,
//! pthreads-async single + multi-worker, mp-tcp-bufsync host +
//! worker) routes its expression / index / call / bound / type
//! rendering through the functions here. The cross-backend
//! bit-identical differential (PRD §10.1) holds because there is
//! exactly ONE implementation — no second copy can drift.
//!
//! # Private vs Pub
//!
//! - The full-context `RenderCtx` carries the `abs_subst` map used by
//!   strip-mined absolute-index rebinding (TASK-0180). Pthreads-sync's
//!   single-worker renderer (`render_main_rs` in `pthreads-sync/lib.rs`)
//!   constructs it directly to drive per-occurrence rebinding from
//!   `Event::Loop.block_tag`.
//!
//! - The thin `RenderCtxPub` is for multi-worker / cross-backend
//!   callers (pthreads-sync multi-worker, pthreads-async multi-worker,
//!   mp-tcp-bufsync host + worker). It carries the SAME `abs_subst`
//!   map as `RenderCtx` so per-occurrence strip-mine rebinding works
//!   on the shared multi-worker walker too (TASK-0181). The map is
//!   empty for every non-blocked program — which is every tier-1
//!   multi-worker schedule today — so non-blocked codegen is byte-
//!   identical to the pre-TASK-0181 emission. The `_pub` variants
//!   stay thin pass-throughs.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use compiler::algo::{IrBinOp, IrExpr, ResolvedType, ScalarType};
use compiler::event::{ArgBinding, DataId, DataSlice, IterVar, KernelId};
use compiler::sidecar::NameSidecar;
use compiler::NameTables;

// --------------------------------------------------------------------
// EmitError — the codegen-time error type
// --------------------------------------------------------------------

/// Errors that can stop a codegen run. Moved here (from pthreads-sync,
/// TASK-0244) because every backend re-exports this type and every
/// rendering function uses it; keeping the canonical definition next
/// to the shared renderers eliminates the `pthreads_sync::EmitError`
/// re-export arrow that historically made pthreads-async and
/// mp-tcp-bufsync depend on pthreads-sync.
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
            // TASK-0230: the per-backend prefix is owned by the driver
            // dispatch site (`driver/src/main.rs:406/426/448`), which
            // wraps every Display with "<backend> codegen error:". The
            // inner backend-name literal was a cosmetic lie when
            // surfaced from a re-exporting backend. Dropped so every
            // backend's user-visible error text reads cleanly.
            EmitError::UnsupportedFeature(msg) => {
                write!(f, "unsupported feature: {msg}")
            }
            EmitError::ContractGap(msg) => {
                write!(f, "EventList/sidecar contract gap: {msg}")
            }
        }
    }
}

impl std::error::Error for EmitError {}

// --------------------------------------------------------------------
// RenderCtx (full) + RenderCtxPub (thin)
// --------------------------------------------------------------------

/// The full rendering context. Carries the `abs_subst` map used by
/// strip-mined absolute-index rebinding (TASK-0180). Single-worker
/// callers (pthreads-sync's `render_main_rs`) construct this directly
/// because they walk `Event::Loop.block_tag` and populate the map
/// per-occurrence.
///
/// The fields are `pub` so backend code outside this crate can
/// construct an instance directly — the rebinding logic lives in the
/// backend's per-event walker, not here.
pub struct RenderCtx<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// Active absolute-index substitutions: an inner-block loop
    /// variable name -> the `(LO + tile*N + inner)` Rust expression
    /// it must expand to at every *body* use site. Empty for every
    /// non-blocked program, so non-blocked codegen is byte-identical
    /// to the pre-TASK-0124 backend (the map is consulted only by
    /// `render_int_expr`/`render_const_expr` on an `Ident`).
    pub abs_subst: BTreeMap<String, String>,
}

impl<'a> RenderCtx<'a> {
    /// Construct a fresh context with an empty `abs_subst` map. The
    /// caller fills `abs_subst` per-occurrence at strip-mine inner
    /// loops.
    pub fn new(names: &'a NameTables, sidecar: &'a NameSidecar) -> Self {
        RenderCtx {
            names,
            sidecar,
            abs_subst: BTreeMap::new(),
        }
    }
}

/// Thin context for multi-worker / cross-backend callers. Carries
/// the SAME `abs_subst` map as the private [`RenderCtx`] so the
/// per-occurrence absolute-index rebinding (TASK-0180) reaches the
/// `_pub` render helpers too. Pre-TASK-0181 this struct held only
/// `(names, sidecar)` because the multi-worker walker hard-rejected
/// any `Event::Loop.block_tag.is_some()` (the TASK-0181 fail-loud
/// guard) — so the map was guaranteed empty. TASK-0181 replaces that
/// guard with the actual rebinding logic on the shared
/// [`multi_worker_walker`](crate::multi_worker_walker), which means
/// the `_pub` helpers MUST consult `abs_subst` or the substitution
/// would silently stop at the loop header and never reach Fire arg /
/// const-expr / output-assign sites (the exact accumulator
/// double-count failure mode TASK-0180 closed for the single-worker
/// path).
///
/// Default-constructed empty via [`Self::new`]; the shared
/// `render_block_tag_loop_header` helper
/// ([`crate::multi_worker_walker::render_block_tag_loop_header`])
/// extends a child copy per strip-mined inner-loop occurrence via
/// [`Self::with_abs_subst`], and every multi-worker walker
/// (pthreads-sync, pthreads-async, mp-tcp-bufsync) consumes the
/// returned child (TASK-0253 — was duplicated per-backend pre-TASK-
/// 0253 / cycle 73).
pub struct RenderCtxPub<'a> {
    pub names: &'a NameTables,
    pub sidecar: &'a NameSidecar,
    /// See [`RenderCtx::abs_subst`]. Empty for every non-blocked
    /// multi-worker program (which is every tier-1 schedule today, so
    /// the existing 88/70/0/18 e2e matrix renders byte-identically).
    pub abs_subst: BTreeMap<String, String>,
}

impl<'a> RenderCtxPub<'a> {
    /// Fresh context, empty `abs_subst`. Existing call sites that
    /// pre-date TASK-0181 keep working — they were already passing an
    /// implicit empty map.
    pub fn new(names: &'a NameTables, sidecar: &'a NameSidecar) -> Self {
        RenderCtxPub {
            names,
            sidecar,
            abs_subst: BTreeMap::new(),
        }
    }

    /// Build a child context sharing `(names, sidecar)` with this one
    /// but carrying the supplied `abs_subst`. Used by the shared
    /// multi-worker walker to introduce a per-occurrence strip-mine
    /// rebinding inside one `Event::Loop` body without mutating the
    /// parent context (mirrors the `RenderCtx { abs_subst: child, .. }`
    /// pattern in the single-worker path).
    pub fn with_abs_subst(&self, abs_subst: BTreeMap<String, String>) -> RenderCtxPub<'a> {
        RenderCtxPub {
            names: self.names,
            sidecar: self.sidecar,
            abs_subst,
        }
    }

    /// Internal lowering to the private `RenderCtx` the underlying
    /// helpers consume. Clones the `abs_subst` map (cheap — the map
    /// holds at most one entry per active strip-mine nesting depth,
    /// which is bounded by source loop nesting).
    fn inner(&self) -> RenderCtx<'_> {
        RenderCtx {
            names: self.names,
            sidecar: self.sidecar,
            abs_subst: self.abs_subst.clone(),
        }
    }
}

// --------------------------------------------------------------------
// Name resolution
// --------------------------------------------------------------------

/// Resolve a `DataId` to its source name, failing loud on a gap.
pub fn data_name(did: DataId, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    ctx.names
        .data
        .get(&did)
        .cloned()
        .ok_or_else(|| EmitError::ContractGap(format!("data id {did:?} has no name in NameTables")))
}

// --------------------------------------------------------------------
// Fire output assignment
// --------------------------------------------------------------------

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
pub fn render_fire_output_assign(
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

// --------------------------------------------------------------------
// Fire arguments
// --------------------------------------------------------------------

/// Render a kernel call's argument list from its [`FireBinding`]
/// inputs. `Data` → indexed/whole-array read; `Scalar` → integer
/// expression with a param-type cast decided via the SIDECAR's
/// kernel signature (TASK-0169, AlgoIR-free); `Nested` → rejected
/// (tier-1 backends do not lower a nested call in argument position).
pub fn render_fire_args(
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
            let rendered = render_int_expr(e, ctx)?;
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

// --------------------------------------------------------------------
// DataSlice classification (TASK-0209)
// --------------------------------------------------------------------

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
        let i0 = render_int_expr(&s.indices[0], ctx)?;
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
        let rendered = render_int_expr(idx_expr, ctx)?;
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

// --------------------------------------------------------------------
// Flat-index rendering (legacy entry used by Push/Wait code paths)
// --------------------------------------------------------------------

/// Render a flat (row-major) index for a 1D `Vec<T>` of an
/// N-dimensional shape. 1D → `(i0) as usize`. Higher rank → strides
/// from the sidecar's `dims` (NOT `algo.data`): `(i0*D1*D2 + i1*D2 +
/// i2) as usize`. Mirrors the old `render_flat_index` exactly.
pub fn render_flat_index(s: &DataSlice, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    if s.indices.is_empty() {
        return Err(EmitError::UnsupportedFeature(
            "render_flat_index called on a non-indexed reference".to_string(),
        ));
    }
    if s.indices.len() == 1 {
        let i0 = render_int_expr(&s.indices[0], ctx)?;
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
        let rendered = render_int_expr(idx_expr, ctx)?;
        if stride == 1 {
            terms.push(format!("({rendered})"));
        } else {
            terms.push(format!("({rendered}) * {stride}"));
        }
    }
    Ok(format!("({}) as usize", terms.join(" + ")))
}

// --------------------------------------------------------------------
// Integer-expression rendering
// --------------------------------------------------------------------

/// Render an integer-valued index/scalar expression as Rust.
///
/// Identifier resolution priority (highest first):
///   1. `ctx.abs_subst` — an active strip-mined absolute-index
///      rebinding (`inner_var` → `(LO + tile*N + inner)`). The map
///      is empty for every non-blocked program.
///   2. `ctx.sidecar.consts` — a declared `const` in the source
///      algorithm (e.g. `const N : usize = 32;` referenced as `N`
///      inside an `IndexExpr` such as `grid[t][(i+N-1) % N]`).
///      Grammar §1 line 91-93 explicitly allows consts inside an
///      IndexExpr; PRD §6.2.1 forbids iter-var shadowing a const, so
///      the two namespaces never collide.
///   3. Bare ident — an iteration variable in scope. Emitted as the
///      verbatim Rust identifier; Rust's type-checker is responsible
///      for confirming it is in scope (mirrors the prior backend).
///
/// The const-resolution path (step 2) is the fix for the IndexExpr-
/// const bug discovered while landing example 11-game-of-life
/// (cycle 35 / TASK-0042.04). Prior to that, an IndexExpr like
/// `(t + ITERS) % (ITERS + 1)` rendered as `((t + ITERS) % (ITERS +
/// 1))` — emitting bare `ITERS` as a Rust identifier, which is not
/// in scope in the generated host source (the codegen does not
/// declare a Rust `const ITERS` mirroring the Nuc const). Loop
/// BOUNDS already resolve consts via `render_const_expr` (step 2 in
/// that function), so `for t : 0 .. ITERS+1` worked; the asymmetry
/// was the bug. Examples 01..09/13 only used loop variables in
/// `IndexExpr`, so the bug was inert in the existing matrix.
pub fn render_int_expr(e: &IrExpr, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
    match e {
        IrExpr::IntLit(v) => Ok(format!("{v}")),
        IrExpr::Ident(n) => {
            if let Some(repl) = ctx.abs_subst.get(n) {
                Ok(repl.clone())
            } else if let Some(c) = ctx.sidecar.consts.get(n) {
                Ok(format!("{}", c.value))
            } else {
                Ok(n.clone())
            }
        }
        IrExpr::Neg(inner) => Ok(format!("-({})", render_int_expr(inner, ctx)?)),
        IrExpr::BinOp(op, l, r) => {
            let ls = render_int_expr(l, ctx)?;
            let rs = render_int_expr(r, ctx)?;
            Ok(format!("({ls} {} {rs})", bin_op_str(op)))
        }
        IrExpr::DataRef(_) | IrExpr::Call { .. } => Err(EmitError::UnsupportedFeature(
            "data-ref / call inside an integer index expression".to_string(),
        )),
    }
}

// --------------------------------------------------------------------
// Loop bounds
// --------------------------------------------------------------------

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
pub fn render_loop_bounds(
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
pub fn render_const_expr(e: &IrExpr, ctx: &RenderCtx<'_>) -> Result<String, EmitError> {
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

// --------------------------------------------------------------------
// Type rendering
// --------------------------------------------------------------------

/// Rust spelling of a Nuc `ScalarType`. Internal default; the public
/// re-export `rust_scalar_type_pub` keeps the name stable for external
/// callers.
pub fn rust_scalar_type(t: &ScalarType) -> &'static str {
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

/// Public spelling of the Rust scalar type. Identity wrapper kept
/// for source-level compatibility — `rust_scalar_type` is already
/// `pub` in backend-common, but callers historically imported the
/// `_pub` variant from pthreads-sync.
pub fn rust_scalar_type_pub(t: &ScalarType) -> &'static str {
    rust_scalar_type(t)
}

/// The Rust literal for "zero" of a scalar type.
pub fn rust_scalar_zero(t: &ScalarType) -> &'static str {
    match t {
        ScalarType::Usize | ScalarType::Isize => "0",
        ScalarType::U8 | ScalarType::U16 | ScalarType::U32 | ScalarType::U64 => "0",
        ScalarType::I8 | ScalarType::I16 | ScalarType::I32 | ScalarType::I64 => "0",
        ScalarType::F32 | ScalarType::F64 => "0.0",
        ScalarType::Bool => "false",
    }
}

/// Rust surface type for a `ResolvedType`: scalars natural, arrays
/// flatten to `Vec<T>`. Shared so slot/buffer typing cannot drift
/// between backends.
pub fn rust_type_of(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_type(&ty.scalar).to_string()
    } else {
        format!("Vec<{}>", rust_scalar_type(&ty.scalar))
    }
}

/// `vec![<zero>; product(dims)]` (array) or the scalar zero literal,
/// sized + typed entirely from the sidecar `ResolvedType`. Shared so
/// the per-backend pre-init allocation cannot drift.
pub fn render_array_init_for(ty: &ResolvedType) -> String {
    if ty.is_scalar() {
        rust_scalar_zero(&ty.scalar).to_string()
    } else {
        let total: usize = ty.dims.iter().copied().product();
        let zero = rust_scalar_zero(&ty.scalar);
        format!("vec![{zero}; {total}]")
    }
}

// --------------------------------------------------------------------
// `_pub` wrappers — thin shims for multi-worker callers
// --------------------------------------------------------------------

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
/// (TASK-0209). `mp-tcp-bufsync` and the pthreads-sync/async multi-
/// worker paths call through this so all indexed-assignment sites use
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

// Helper used by callers (pthreads-sync's single-worker emit) that
// need to write a fail-loud emit error to disk.
pub fn write_file(path: &std::path::Path, contents: &str) -> Result<(), EmitError> {
    std::fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
