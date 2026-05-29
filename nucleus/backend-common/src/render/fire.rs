//! Name resolution, Fire output/argument rendering, DataSlice
//! classification (scalar vs sub-array), flat-index rendering, and
//! the `write_file` filesystem helper. Split from `render.rs` for
//! file-size hygiene; no behaviour change.
//!
//! All of these helpers consume a [`RenderCtx`] (private) or
//! [`RenderCtxPub`] (multi-worker shim). The classification and
//! flat-index work is grouped here with the Fire-rendering code
//! because every callsite that needs flat-index also needs DataSlice
//! classification (and vice versa).

use nucleus_compiler::algo::ResolvedType;
use nucleus_compiler::event::{ArgBinding, DataId, DataSlice, KernelId};

use super::ctx::{RenderCtx, RenderCtxPub};
use super::error::EmitError;
use super::expr::render_int_expr;
use super::reuse::{sidecar_consts_to_resolved, try_reuse_axis_offset};
use super::types::rust_scalar_type;

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

/// How a contiguous PREFIX sub-array Fire argument is materialised at
/// the call site (TASK-0049.06). A partial-rank DataRef (e.g.
/// `mic_in[frame]` where `mic_in : i32[N_FRAMES][16]`) is a contiguous
/// `[start .. start + sub_len]` window of the flat backing store; the
/// two backend families pass it to the kernel differently:
///
/// - [`SubArrayForm::Vec`] — `<slice>.to_vec()`, an owned `Vec<T>`.
///   This is the TIER-1 std convention (kernel params are `Vec<T>` per
///   TASK-0103). It is the default and keeps the cross-backend
///   bit-identical differential (PRD §10.1) on the existing matrix.
/// - [`SubArrayForm::FixedArray`] — `<slice>.try_into().unwrap()`, an
///   owned fixed `[T; sub_len]`. This is the no_std/embedded
///   convention: kernel params are `[T; N]` (alloc-free), and
///   `<[T]>::try_into::<[T; N]>` is `core`-available (no `Vec`, no
///   `alloc`). The target array length is inferred from the kernel
///   signature param type at the call site. Used ONLY by the
///   embedded-pattern backend's `no_std` lowering — it MUST NOT change
///   the tier-1 emission, so it is reached only through
///   [`render_fire_args_nostd`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubArrayForm {
    /// `<slice>.to_vec()` — owned `Vec<T>` (tier-1 std default).
    Vec,
    /// `<slice>.try_into().unwrap()` — owned `[T; N]` (no_std/embedded).
    FixedArray,
}

/// Render a kernel call's argument list from its [`FireBinding`]
/// inputs. `Data` → indexed/whole-array read; `Scalar` → integer
/// expression with a param-type cast decided via the SIDECAR's
/// kernel signature (TASK-0169, AlgoIR-free); `Nested` → rejected
/// (tier-1 backends do not lower a nested call in argument position).
///
/// Sub-array (partial-rank) arguments materialise as `.to_vec()`
/// (tier-1 std). The embedded `no_std` backend uses
/// [`render_fire_args_nostd`] instead, which renders fixed arrays.
pub fn render_fire_args(
    kernel: KernelId,
    inputs: &[ArgBinding],
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    render_fire_args_with(kernel, inputs, ctx, SubArrayForm::Vec)
}

/// no_std variant of [`render_fire_args`]: contiguous-prefix sub-array
/// arguments materialise as fixed `[T; N]` arrays (`.try_into().unwrap()`)
/// rather than `Vec<T>` (`.to_vec()`), so the call typechecks against a
/// `no_std` kernel signature with `[T; N]` params and the generated lib
/// needs no allocator (TASK-0049.06; the array-typed pure-kernel
/// lowering gap the M11 real-example-14 cross-compile surfaced). Scalar
/// (full-rank) and whole-array arguments render IDENTICALLY to the
/// tier-1 path — only the sub-array materialisation differs.
pub fn render_fire_args_nostd(
    kernel: KernelId,
    inputs: &[ArgBinding],
    ctx: &RenderCtx<'_>,
) -> Result<String, EmitError> {
    render_fire_args_with(kernel, inputs, ctx, SubArrayForm::FixedArray)
}

fn render_fire_args_with(
    kernel: KernelId,
    inputs: &[ArgBinding],
    ctx: &RenderCtx<'_>,
    sub_array_form: SubArrayForm,
) -> Result<String, EmitError> {
    // Per-param types come from the sidecar's kernel signature, NOT
    // `algo.kernels` (AC#2). Absent only if the contract regressed.
    let sig = ctx.sidecar.kernel_sig(kernel);
    let mut parts = Vec::with_capacity(inputs.len());
    for (i, arg) in inputs.iter().enumerate() {
        let param_ty = sig.and_then(|s| s.params.get(i));
        parts.push(render_fire_arg(arg, param_ty, ctx, sub_array_form)?);
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
    use nucleus_compiler::algo::IrExpr;
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
        // TASK-0283: convert sidecar.consts (BTreeMap<String, ConstValue>)
        // to BTreeMap<String, ResolvedConst> the shared affine
        // decomposition consumes. Per-call materialisation is cheap
        // (consts table is small, bounded by program declarations).
        let consts_resolved = sidecar_consts_to_resolved(ctx.sidecar);
        let b = try_reuse_axis_offset(&s.indices[ax_idx], &g.iv_name, &consts_resolved)?;
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
    sub_array_form: SubArrayForm,
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
                // TASK-0269 + TASK-0282 reuse codegen: if a reuse
                // group is active on this `(data, axis)` AND the
                // DataRef's outer axes match one of the discovered
                // groups verbatim AND the reuse-axis index decodes as
                // `iv + b` for the recorded iv, rewrite the read into
                // a buffer slot lookup. TASK-0282 generalised the
                // rewrite from "first matching outer-axes pattern
                // only" (cycle 103) to "every unique outer-axes
                // pattern gets its own buffer" (cycle 110) — in
                // 05-stencil/reuse every one of the 9 `img_in[...]`
                // reads in the blur3 call now rewrites (3 buffers
                // covering y-1, y, y+1 rows).
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
                        // The literal `{sub_len}usize` makes the upper
                        // bound a `usize` so the `start..start+sub_len`
                        // range typechecks (`start` is already `as
                        // usize` from `classify_data_slice`).
                        match sub_array_form {
                            // Tier-1 std: owned `Vec<T>` matches
                            // `rust_type_of` for an aggregate kernel
                            // param (`Vec<T>`); the call moves it,
                            // consistent with the whole-array case
                            // above. PRD §6.2.1 single-assignment
                            // permits the move semantics. Byte-identical
                            // to the pre-TASK-0049.06 emission.
                            SubArrayForm::Vec => Ok(format!(
                                "{name}[{start}..{start} + {sub_len}usize].to_vec()"
                            )),
                            // no_std/embedded: owned `[T; sub_len]` via
                            // `<[T]>::try_into` (core, alloc-free). The
                            // target length is inferred from the kernel
                            // signature's `[T; N]` param at the call
                            // site; `.unwrap()` is correct because the
                            // slice length (`sub_len`) provably matches
                            // the declared shape (both derive from the
                            // sidecar `dims`). TASK-0049.06.
                            SubArrayForm::FixedArray => Ok(format!(
                                "{name}[{start}..{start} + {sub_len}usize].try_into().unwrap()"
                            )),
                        }
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

// --------------------------------------------------------------------
// Filesystem helper
// --------------------------------------------------------------------

// Helper used by callers (pthreads-sync's single-worker emit) that
// need to write a fail-loud emit error to disk.
pub fn write_file(path: &std::path::Path, contents: &str) -> Result<(), EmitError> {
    std::fs::write(path, contents).map_err(|e| EmitError::WriteFailed {
        path: path.to_path_buf(),
        source: e,
    })
}
