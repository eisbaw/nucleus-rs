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

use nucleus_compiler::algo::{IrBinOp, IrExpr, ResolvedType, ScalarType};
use nucleus_compiler::event::{ArgBinding, DataId, DataSlice, Event, IterVar, KernelId};
use nucleus_compiler::passes::reuse_inference::ReuseSlot;
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

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
    /// Active reuse-buffer rewrite groups, keyed by `DataId`.
    ///
    /// Populated by [`render_reuse_buf_decls`] at the entry of an
    /// `Event::Loop` body when `sidecar.reuse_widths.get(iter_var)` is
    /// non-empty AND a matching DataRef shape was discovered in the
    /// body (TASK-0269 Stage 2 codegen). Empty for every loop without
    /// reuse-active slots — preserves byte-identicality on every
    /// pre-TASK-0269 schedule (the map is consulted only by
    /// `render_fire_arg` on an `ArgBinding::Data` with non-empty
    /// indices).
    ///
    /// Multi-axis reuse on the same DataId (a separable filter at
    /// different (data, axis) pairs) is supported in shape but the
    /// 05-stencil first landing exercises one (data, axis=1) group.
    pub reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
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
            reuse_active: BTreeMap::new(),
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
    /// See [`RenderCtx::reuse_active`]. Empty for every multi-worker
    /// program in TASK-0269's scope: the multi-worker walker landing
    /// for reuse codegen is forward-carried to TASK-0270, and until
    /// that lands no multi-worker code path populates this map. The
    /// field carries the same shape as `RenderCtx::reuse_active` so
    /// the cross-context `inner()` conversion is a literal copy.
    pub reuse_active: BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
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
            reuse_active: BTreeMap::new(),
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
            reuse_active: self.reuse_active.clone(),
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
            reuse_active: self.reuse_active.clone(),
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

/// Try the reuse-buffer rewrite at a Fire-arg DataRef site.
///
/// Returns `Some(rendered_rust)` iff:
/// 1. `ctx.reuse_active` carries a group for `s.data`, and
/// 2. one of the groups at this `data` has `axis < s.indices.len()`
///    and matches the OUTER axes (every non-reuse axis exactly
///    `IrExpr::eq` to the canonical pattern), and
/// 3. the reuse-axis index decodes as `iv + b` via
///    [`try_reuse_axis_offset`].
///
/// On match the rewrite is
/// `buf[((iv_rendered + b - min_offset).rem_euclid(L)) as usize]`.
/// `iv_rendered` is the iv name (or its `abs_subst`-rebound
/// expression) — the source-code rendering of the iv, NOT a value.
fn try_rewrite_reuse_arg(s: &DataSlice, ctx: &RenderCtx<'_>) -> Option<String> {
    let groups = ctx.reuse_active.get(&s.data)?;
    for g in groups {
        let ax_idx = g.axis as usize;
        if ax_idx >= s.indices.len() {
            continue;
        }
        // Outer-axes match check. The DataSlice's indices with the
        // reuse axis at `ax_idx` skipped must equal `g.outer_axes`
        // verbatim (PartialEq on IrExpr ignores no fields — it's a
        // structural equality on the AST).
        if s.indices.len() != g.outer_axes.len() + 1 {
            continue;
        }
        let outer_match = s
            .indices
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != ax_idx)
            .map(|(_, e)| e)
            .zip(g.outer_axes.iter())
            .all(|(a, b)| a == b);
        if !outer_match {
            continue;
        }
        let b = try_reuse_axis_offset(&s.indices[ax_idx], &g.iv_name)?;
        // Render the iv via render_int_expr on a synthetic
        // `IrExpr::Ident(iv_name)` so any active `abs_subst`
        // substitution (strip-mined inner loop) reaches the rewrite
        // too. Unwrap is safe: render_int_expr on an Ident is
        // infallible (only DataRef/Call branches return Err).
        let iv_expr = IrExpr::Ident(g.iv_name.clone());
        let iv_rendered = render_int_expr(&iv_expr, ctx).ok()?;
        let length = g.slot.length;
        let min_offset = g.slot.min_offset;
        return Some(format!(
            "{buf}[(((({iv}) + ({b}_i64) - ({min_offset}_i64)).rem_euclid({length}_i64)) as usize)]",
            buf = g.buf_ident,
            iv = iv_rendered,
        ));
    }
    None
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
                // TASK-0269 Stage 2 codegen: if a reuse group is
                // active on this `(data, axis)` AND the outer axes
                // match the canonical pattern AND the reuse-axis
                // index decodes as `iv + b` for the recorded iv,
                // rewrite the read into a buffer slot lookup. The
                // restrictive cut keeps the first-cut landing narrow
                // (only 1 of 3 outer-coord variations in 05-stencil/
                // reuse hits the rewrite); a more general rewrite
                // (one buffer per outer-coord variation) is a
                // follow-up.
                if let Some(rewrite) = try_rewrite_reuse_arg(s, ctx) {
                    return Ok(rewrite);
                }
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
    debug_assert!(
        !s.indices.is_empty(),
        "classify_data_slice requires indices"
    );
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

pub fn render_flat_index_pub(s: &DataSlice, ctx: &RenderCtxPub<'_>) -> Result<String, EmitError> {
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

pub fn render_const_expr_pub(e: &IrExpr, ctx: &RenderCtxPub<'_>) -> Result<String, EmitError> {
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

// --------------------------------------------------------------------
// Reuse-widths marker emit — TASK-0265 Tier 1 Stage 2 wiring
// --------------------------------------------------------------------

/// Emit a Rust comment line at an `Event::Loop` body's entry naming
/// every (data, axis, ReuseSlot) the sidecar carries for this iv —
/// the FIRST consumer of `NameSidecar::reuse_widths` (Stage 1 ⇒
/// Stage 2 handoff, TASK-0265).
///
/// # Stage 2 status (Tier 1 wiring; cycle 87)
///
/// This is the LOOKUP scaffolding step. The walker reads
/// `sidecar.reuse_widths.get(iter_var)`, iterates `(DataId, axis,
/// ReuseSlot)` triples in deterministic order (`BTreeMap` keys are
/// `u64`-newtype / `u64`), and writes ONE comment line per slot
/// naming `data=<symbol> axis=<n> length=<L> min_offset=<M>`. The
/// `reuse_widths_pending` marker substring is grep-able by the e2e
/// test crate to assert the consumer ran (AC#4 of TASK-0265 — the
/// "emitted code contains 'circular' / 'delay_line' / similar
/// marker" half).
///
/// Tier 2 / Tier 3 of TASK-0265 (per-backend circular-buffer emit)
/// is forward-carried — the actual delay-line `Vec<T>` declaration,
/// initial-fill prologue, per-iteration rotate, and `grid[iv + b]`
/// → `buf[(iv + b - min_offset) % length]` index rewrite live on
/// each backend's `Plan::emit` and are filed as TASK-0269 (pthreads-sync,
/// .01) + TASK-0270 (multi_worker_walker, .02 — covers pthreads-async +
/// mp-tcp-bufsync + mp-tcp-event). Driver promotion strict /
/// partition-policy-aware: TASK-0271 (.04). IvScopeError unification
/// with halo_inference: TASK-0272 (.05).
///
/// # Determinism
///
/// `BTreeMap` iteration on every level. The comment lines emit in
/// (DataId, axis) order — identical inputs produce identical outputs.
/// Empty-input path is a true no-op: when `reuse_widths.get(iter_var)`
/// is `None` (no reuse on this iv) NOTHING is written, preserving
/// byte-identicality with the pre-TASK-0265 emit for every shipped
/// schedule that does not carry `reuse`.
///
/// # Determinism in name lookup
///
/// Data symbol name comes from the `NameTables` reverse map keyed by
/// `DataId`. A missing entry falls back to `d<id>` (defensive — the
/// invariant is that `name_data` covers every DataId in the
/// sidecar, and an absent name in a non-empty reuse map would be a
/// projection-layer bug; we emit the id-form so the marker still
/// fires and downstream tests can see it without an emit hard-fail).
pub fn render_reuse_marker_comment(
    out: &mut String,
    indent: usize,
    iter_var: IterVar,
    iter_var_name: &str,
    sidecar: &NameSidecar,
    names: &NameTables,
) {
    use std::fmt::Write as _;
    let Some(per_data) = sidecar.reuse_widths.get(&iter_var) else {
        return;
    };
    if per_data.is_empty() {
        return;
    }
    let pad = "    ".repeat(indent);
    for (data_id, per_axis) in per_data {
        let data_name = names
            .data
            .get(data_id)
            .cloned()
            .unwrap_or_else(|| format!("d{}", data_id.0));
        for (axis, slot) in per_axis {
            // Marker substring `reuse_widths_pending` is load-bearing
            // for AC#4 of TASK-0265 — the e2e marker-detection test
            // greps for it. Do NOT rename without updating the test.
            //
            // TASK-0269 (cycle 103): the marker comment now precedes
            // the real circular-buffer codegen on pthreads-sync's
            // single-worker path. The substring is preserved as a
            // regression canary above the buffer decl — Tier 2 + Tier 3
            // landings can subsume it (TASK-0270 multi-worker remains
            // marker-only).
            let _ = writeln!(
                out,
                "{pad}// reuse_widths_pending: iv={iter_var_name} data={data_name} axis={axis} length={length} min_offset={min_offset} (Stage 2 active; circular-buffer codegen below on pthreads-sync — TASK-0269)",
                length = slot.length,
                min_offset = slot.min_offset,
            );
        }
    }
}

// --------------------------------------------------------------------
// Reuse circular-buffer codegen — TASK-0269 Stage 2 Tier 2
// (pthreads-sync single-worker path)
// --------------------------------------------------------------------

/// One reuse rewrite group: the circular buffer that backs every
/// matching DataRef read of a single `(DataId, axis)` slot whose
/// OUTER axes match the canonical `outer_axes` pattern verbatim.
///
/// Populated by [`render_reuse_buf_decls`] when a body's first
/// matching DataRef is discovered; consumed by [`render_fire_arg`] at
/// every Fire-arg read site to rewrite a matching DataRef into a
/// `buf[(iv + b - min_offset).rem_euclid(L) as usize]` lookup.
///
/// The first-cut narrowing (TASK-0269): only DataRefs whose OUTER
/// axes match `outer_axes` exactly (PartialEq on `IrExpr`) are
/// rewritten. In 05-stencil/reuse this means the 3 reads at
/// `img_in[y][x-1..=x+1]` get rewritten; the 6 reads at `img_in[y-1]`
/// and `img_in[y+1]` stay verbatim. A more general rewrite (one
/// buffer per outer-coord variation) is filed as a follow-up.
#[derive(Debug, Clone)]
pub struct ReuseRewriteGroup {
    /// The reuse-axis index inside the DataRef (`axis` key from the
    /// sidecar's `reuse_widths` triple-nested map).
    pub axis: u64,
    /// The slot shape inferred by Stage 1 (`length`, `min_offset`).
    pub slot: ReuseSlot,
    /// The canonical OUTER-axis index expressions (all axes except
    /// `axis`) — a matching DataRef must have these EXACT outer-axis
    /// expressions to be rewritten. Stored in source-order, so axis
    /// indexing is preserved (axis 0 first, then axis 1, etc., with
    /// the reuse axis at position `axis` omitted).
    pub outer_axes: Vec<IrExpr>,
    /// Emitted Rust identifier for the per-(data, axis) buffer. Form:
    /// `__reuse_buf_<data_name>_a<axis>`.
    pub buf_ident: String,
    /// The iter-var name (e.g. `"x"`) — used by `render_fire_arg` to
    /// detect the affine `iv + b` shape on the reuse-axis index.
    pub iv_name: String,
}

/// Try to decompose an affine `iv + b` index. Returns `Some(b)` iff
/// the expression is one of the shapes
/// [`affine_decompose`](nucleus_compiler::passes::common::affine_decompose)
/// accepts with coefficient 1 (the only coefficient Stage 1 records;
/// any other has already been rejected by `apply_reuse_inference`).
/// Pure-integer expressions (no iv mention) return `None` — they're
/// out-of-pattern for a reuse-axis index.
///
/// This is a tiny inlined re-impl of the iv+constant shapes —
/// [`nucleus_compiler::passes::common::affine_decompose`] is
/// `pub(crate)` so codegen cannot call it directly. The shapes we
/// recognise here MUST stay a subset of Stage 1's; mismatch would
/// mean we'd rewrite reads that Stage 1 didn't count (or vice versa).
fn try_reuse_axis_offset(e: &IrExpr, iv_name: &str) -> Option<i64> {
    match e {
        IrExpr::Ident(n) if n == iv_name => Some(0),
        IrExpr::BinOp(IrBinOp::Add, lhs, rhs) => match (lhs.as_ref(), rhs.as_ref()) {
            (IrExpr::Ident(n), IrExpr::IntLit(v)) if n == iv_name => Some(*v),
            (IrExpr::IntLit(v), IrExpr::Ident(n)) if n == iv_name => Some(*v),
            _ => None,
        },
        IrExpr::BinOp(IrBinOp::Sub, lhs, rhs) => match (lhs.as_ref(), rhs.as_ref()) {
            (IrExpr::Ident(n), IrExpr::IntLit(v)) if n == iv_name => Some(-*v),
            _ => None,
        },
        _ => None,
    }
}

/// Walk an `Event` tree shallowly looking for the FIRST matching
/// DataRef of each `(data_id, axis)` reuse slot. "Matching" means:
/// the `ArgBinding::Data`'s `DataSlice` has enough axes, the
/// reuse-axis index decodes via [`try_reuse_axis_offset`], and the
/// outer axes are kept as-is (one canonical variation is picked from
/// the first matching read).
///
/// The walk descends through nested `Event::Loop` bodies and
/// `Event::Fire` arg bindings, including `ArgBinding::Nested`'s
/// `Vec<ArgBinding>`. It stops at the first matching read PER
/// `(data_id, axis)` pair so the canonical outer-axes pattern is
/// stable.
///
/// Returns `BTreeMap<DataId, Vec<ReuseRewriteGroup>>` ordered by
/// `DataId` then by axis (determinism via `BTreeMap` + sorted axis
/// iteration).
fn discover_reuse_groups(
    body: &[Event],
    iv_name: &str,
    per_data: &BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>,
    names: &NameTables,
) -> BTreeMap<DataId, Vec<ReuseRewriteGroup>> {
    let mut out: BTreeMap<DataId, Vec<ReuseRewriteGroup>> = BTreeMap::new();
    for (data_id, per_axis) in per_data {
        // Collect already-found axes so the walk can skip past once
        // every axis on this data has a canonical pattern.
        let mut found: BTreeMap<u64, ReuseRewriteGroup> = BTreeMap::new();
        for ev in body {
            walk_event_for_reuse(ev, *data_id, iv_name, per_axis, &mut found, names);
            if found.len() == per_axis.len() {
                break;
            }
        }
        let groups: Vec<ReuseRewriteGroup> = found.into_values().collect();
        if !groups.is_empty() {
            out.insert(*data_id, groups);
        }
    }
    out
}

fn walk_event_for_reuse(
    ev: &Event,
    data_id: DataId,
    iv_name: &str,
    per_axis: &BTreeMap<u64, ReuseSlot>,
    found: &mut BTreeMap<u64, ReuseRewriteGroup>,
    names: &NameTables,
) {
    match ev {
        Event::Fire { bindings, .. } => {
            for arg in &bindings.inputs {
                walk_arg_for_reuse(arg, data_id, iv_name, per_axis, found, names);
            }
        }
        Event::Loop { body, .. } => {
            for child in body {
                walk_event_for_reuse(child, data_id, iv_name, per_axis, found, names);
            }
        }
        // Sync / Push / Wait / Alloc / Free carry no Fire-arg DataRefs.
        _ => {}
    }
}

fn walk_arg_for_reuse(
    arg: &ArgBinding,
    data_id: DataId,
    iv_name: &str,
    per_axis: &BTreeMap<u64, ReuseSlot>,
    found: &mut BTreeMap<u64, ReuseRewriteGroup>,
    names: &NameTables,
) {
    match arg {
        ArgBinding::Data(s) => {
            if s.data != data_id || s.indices.is_empty() {
                return;
            }
            for (axis, slot) in per_axis {
                if found.contains_key(axis) {
                    continue;
                }
                let ax_idx = *axis as usize;
                if ax_idx >= s.indices.len() {
                    continue;
                }
                // The reuse-axis index must decode as `iv + b`. If not,
                // this DataRef is out-of-pattern (a non-iv index on the
                // reuse axis); skip it. Stage 1 would have rejected the
                // body if NO DataRef matched, so we're guaranteed at
                // least one match per axis (else `per_axis` wouldn't
                // contain `axis`).
                if try_reuse_axis_offset(&s.indices[ax_idx], iv_name).is_none() {
                    continue;
                }
                // Build the OUTER axes (all axes except the reuse one),
                // source-order preserved.
                let outer_axes: Vec<IrExpr> = s
                    .indices
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| if i == ax_idx { None } else { Some(e.clone()) })
                    .collect();
                let data_name = names
                    .data
                    .get(&data_id)
                    .cloned()
                    .unwrap_or_else(|| format!("d{}", data_id.0));
                let buf_ident = format!("__reuse_buf_{}_a{}", data_name, axis);
                found.insert(
                    *axis,
                    ReuseRewriteGroup {
                        axis: *axis,
                        slot: *slot,
                        outer_axes,
                        buf_ident,
                        iv_name: iv_name.to_string(),
                    },
                );
            }
        }
        ArgBinding::Nested { args, .. } => {
            for inner in args {
                walk_arg_for_reuse(inner, data_id, iv_name, per_axis, found, names);
            }
        }
        ArgBinding::Scalar(_) => {}
    }
}

/// Emit Vec<T> circular-buffer declarations + initial-fill prologue
/// for every reuse group active on `iter_var` in `body`. Returns the
/// `reuse_active` map the caller seeds into the child `RenderCtx` it
/// recurses into the body with.
///
/// The prologue unrolls the fills for offsets `b in min_offset ..
/// min_offset+length-1` (i.e. every offset EXCEPT the most-distant
/// `max_offset = min_offset + length - 1`). The per-iter update for
/// `max_offset` is the responsibility of [`render_reuse_per_iter_update`]
/// at body entry.
///
/// `lo_expr_rs` is the Rust expression for the loop's `lo` bound
/// (e.g. `"1_i64"` for `for x : 1..W-1`). It substitutes the iv in
/// the source-axis index at prologue time — the reuse-axis position
/// of the source array reads becomes `lo + b` for each prologue
/// offset.
///
/// `body` is the loop's body — walked once to discover the canonical
/// outer-axes pattern per `(data_id, axis)`.
///
/// Empty path is a true no-op (returns empty map, writes nothing).
/// Byte-identicality with the pre-TASK-0269 emit holds for every
/// schedule without an active reuse slot.
pub fn render_reuse_buf_decls(
    out: &mut String,
    indent: usize,
    iter_var: IterVar,
    iter_var_name: &str,
    lo_expr_rs: &str,
    body: &[Event],
    ctx: &RenderCtx<'_>,
) -> Result<BTreeMap<DataId, Vec<ReuseRewriteGroup>>, EmitError> {
    use std::fmt::Write as _;
    let Some(per_data) = ctx.sidecar.reuse_widths.get(&iter_var) else {
        return Ok(BTreeMap::new());
    };
    if per_data.is_empty() {
        return Ok(BTreeMap::new());
    }
    let groups = discover_reuse_groups(body, iter_var_name, per_data, ctx.names);
    let pad = "    ".repeat(indent);
    for (data_id, gs) in &groups {
        let data_name = data_name(*data_id, ctx)?;
        let ty = ctx.sidecar.data_type(*data_id).ok_or_else(|| {
            EmitError::ContractGap(format!(
                "reuse buffer for data `{data_name}` ({:?}) has no \
                 ResolvedType in the NameSidecar (TASK-0269)",
                data_id
            ))
        })?;
        let scalar_ty = rust_scalar_type(&ty.scalar);
        let zero = rust_scalar_zero(&ty.scalar);
        for g in gs {
            // 1. Buffer decl.
            let _ = writeln!(
                out,
                "{pad}let mut {buf}: Vec<{scalar_ty}> = vec![{zero}; {length}usize];",
                buf = g.buf_ident,
                length = g.slot.length,
            );
            // 2. Prologue: fill every offset EXCEPT the most-distant
            //    one (which is filled per-iter inside the body). The
            //    `lo + b` source-axis index substitutes the iv for the
            //    prologue's evaluation.
            //
            //    The source array's flat index is computed by the same
            //    `render_flat_index`-style sum the body uses; we build
            //    a synthetic `DataSlice` with the prologue's
            //    iv-substituted index and pass it through
            //    `render_flat_index`.
            let max_offset = g.slot.min_offset + (g.slot.length as i64) - 1;
            for b in g.slot.min_offset..max_offset {
                let prologue_slice = prologue_slice_for_offset(g, *data_id, b, lo_expr_rs);
                let src_flat = render_flat_index(&prologue_slice, ctx)?;
                // The buffer slot index MUST match the body's read
                // formula `buf[(iv + b - min_offset).rem_euclid(L)]`
                // evaluated at iv == lo (the first body iteration).
                // Hence: `buf[((lo + b) - min_offset).rem_euclid(L)]`.
                // We emit this as a runtime expression to avoid having
                // to const-fold `lo` at codegen time (it may carry
                // `H-1`-style symbolic-const subtrees from
                // `render_const_expr`).
                let length = g.slot.length;
                let _ = writeln!(
                    out,
                    "{pad}{buf}[(((({lo_expr_rs}) + ({b}_i64) - ({min_offset}_i64)).rem_euclid({length}_i64)) as usize)] = {data_name}[{src_flat}];",
                    buf = g.buf_ident,
                    min_offset = g.slot.min_offset,
                );
            }
        }
    }
    Ok(groups)
}

/// Emit the per-iteration most-distant-element load for every active
/// reuse group. Called at body entry, AFTER the loop header and
/// BEFORE recursing into the body, so the slot is current when any
/// Fire arg reads it.
///
/// `iv_expr_rs` is the Rust expression for the iv (typically just the
/// iv variable name, or the rebound absolute expression under a
/// strip-mined inner loop).
pub fn render_reuse_per_iter_update(
    out: &mut String,
    indent: usize,
    groups: &BTreeMap<DataId, Vec<ReuseRewriteGroup>>,
    iv_expr_rs: &str,
    ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    use std::fmt::Write as _;
    if groups.is_empty() {
        return Ok(());
    }
    let pad = "    ".repeat(indent);
    for (data_id, gs) in groups {
        let data_name = data_name(*data_id, ctx)?;
        for g in gs {
            let max_offset = g.slot.min_offset + (g.slot.length as i64) - 1;
            // Source flat index uses the LIVE iv expression
            // (`iv + max_offset`) on the reuse axis.
            let live_slice = live_slice_for_offset(g, *data_id, max_offset, iv_expr_rs);
            let src_flat = render_flat_index(&live_slice, ctx)?;
            // The buffer slot rotates with the iv. We could fold the
            // `as i64` + `rem_euclid` away if iv is known non-negative,
            // but keeping the rem_euclid form makes the rewrite
            // uniform between the prologue (where the slot index is a
            // const u64 literal) and the per-iter update + rewrite
            // sites (where the slot index depends on the live iv).
            let length = g.slot.length;
            let _ = writeln!(
                out,
                "{pad}{buf}[((({iv_expr_rs}) + ({max_offset}_i64) - ({min_offset}_i64)).rem_euclid({length}_i64)) as usize] = {data_name}[{src_flat}];",
                buf = g.buf_ident,
                min_offset = g.slot.min_offset,
            );
        }
    }
    Ok(())
}

/// Build the synthetic DataSlice the prologue uses to fetch one slot
/// from the source array. Reuse axis is replaced with the literal
/// `(<lo_expr_rs>) + (b)`; outer axes are the canonical
/// [`ReuseRewriteGroup::outer_axes`] pattern verbatim.
///
/// `render_flat_index` consumes the result — its render path treats
/// our synthetic IrExpr nodes the same as natural ones (no
/// observational difference).
fn prologue_slice_for_offset(
    group: &ReuseRewriteGroup,
    data_id: DataId,
    b: i64,
    lo_expr_rs: &str,
) -> DataSlice {
    // Build the prologue's reuse-axis index expression as
    // `Ident("(lo) + (b)")` — render_int_expr emits Ident verbatim
    // (the `abs_subst` table is empty for this synthetic path), so
    // the printed text is exactly the Rust expression we want. This
    // is the same precedent `abs_subst`'s rebound expressions use:
    // pre-rendered Rust strings smuggled through an Ident node.
    let mut indices: Vec<IrExpr> = Vec::with_capacity(group.outer_axes.len() + 1);
    let mut outer_iter = group.outer_axes.iter();
    let ax_idx = group.axis as usize;
    // Splice the reuse-axis index back at its original position.
    for i in 0..(group.outer_axes.len() + 1) {
        if i == ax_idx {
            indices.push(IrExpr::Ident(format!("({lo_expr_rs}) + ({b}_i64)")));
        } else {
            indices.push(
                outer_iter
                    .next()
                    .expect("outer_axes length matches")
                    .clone(),
            );
        }
    }
    DataSlice {
        data: data_id,
        indices,
    }
}

/// Same as [`prologue_slice_for_offset`] but for the live per-iter
/// update — the reuse-axis index becomes
/// `(<iv_expr_rs>) + (offset)`.
fn live_slice_for_offset(
    group: &ReuseRewriteGroup,
    data_id: DataId,
    offset: i64,
    iv_expr_rs: &str,
) -> DataSlice {
    let ax_idx = group.axis as usize;
    let mut indices: Vec<IrExpr> = Vec::with_capacity(group.outer_axes.len() + 1);
    let mut outer_iter = group.outer_axes.iter();
    for i in 0..(group.outer_axes.len() + 1) {
        if i == ax_idx {
            indices.push(IrExpr::Ident(format!("({iv_expr_rs}) + ({offset}_i64)")));
        } else {
            indices.push(
                outer_iter
                    .next()
                    .expect("outer_axes length matches")
                    .clone(),
            );
        }
    }
    DataSlice {
        data: data_id,
        indices,
    }
}
