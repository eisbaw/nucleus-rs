//! Kernel-as-Rust-function contract verification (TASK-0012).
//!
//! PRD §6.2.2: a kernel declared in `*.algo.nuc` like
//!
//! ```text
//! kernel blur3 : (f32, f32, ..., f32) -> f32  pure;
//! ```
//!
//! is a *contract* the user's adjacent `kernels.rs` must satisfy.
//! Specifically there must exist a `pub fn blur3(a: f32, ..., i: f32)
//! -> f32` whose name, arity, parameter types, and return type match
//! the declaration. This module verifies that contract.
//!
//! ## Strategy
//!
//! Two phases, both run unconditionally and both contribute to the
//! returned error vector:
//!
//! 1. **Compile-check.** Invoke `rustc --emit=metadata --crate-type=rlib
//!    --edition 2021 kernels.rs -o <tmp>` directly, NOT via a generated
//!    Cargo project. Reasons for not using Cargo:
//!
//!    - No `Cargo.toml` to template, no `target/` directory churn,
//!      no lockfile to maintain, no dependency resolver to wait for.
//!    - A v2 `kernels.rs` is, by PRD §6.2.2, a sibling file with no
//!      external dependencies — `std`, `core`, intrinsics, `unsafe`
//!      are all available via plain rustc. If a real example later
//!      needs an external crate, the strategy can be revisited.
//!    - Faster: ~tens of ms for a small file vs ~seconds for a full
//!      `cargo check` first run.
//!
//!    If the compile fails for reasons unrelated to the contract
//!    (syntax error, undefined macro, missing intrinsic), we emit
//!    [`ContractError::RustCheckFailed`] with the captured stderr and
//!    *still* attempt the signature parse below — partial diagnostics
//!    are more useful than nothing.
//!
//! 2. **Signature parse.** Read `kernels.rs` as text, parse it with
//!    `syn` into a [`syn::File`], scan top-level [`syn::ItemFn`]s,
//!    and match each declared kernel against a function by name. The
//!    parse phase is independent of (1): even if rustc rejects the
//!    file, syn can often still produce a token tree we can inspect.
//!
//!    For each declared kernel we report at most ONE error: the
//!    first matching variant in this order: `KernelNotFound`,
//!    `MissingPub`, `ArityMismatch`, `TypeMismatch`. This avoids
//!    blast-radius noise (one mis-typed kernel does not produce
//!    six errors).
//!
//! ## Type matching: scalar-only at v2
//!
//! Nuc scalars (`f32`, `u64`, `bool`, ...) map 1:1 to Rust path types
//! by name. We compare the Rust parameter type's terminal path
//! segment to the Nuc scalar's spelling. This handles `f32`, `::core::primitive::f32`,
//! and `std::primitive::f32` uniformly.
//!
//! Aggregate Nuc types (e.g. `f32[H][W]`) are NOT matched against
//! their Rust counterparts (`Box<[[f32; W]; H]>`, `&[[f32; W]; H]`,
//! flat slices, etc.) — the surface for this is large and depends on
//! choices the codegen pass hasn't made yet. If a declared kernel has
//! a non-scalar parameter or return, we emit a [`ContractError::TypeMismatch`]
//! with `expected = "<aggregate type, not yet supported>"` and a clear
//! reason. This is loud failure rather than silent acceptance.
//!
//! ## What this module does NOT check
//!
//! - **Purity.** The PRD §6.2.2 explicitly notes that Rust's type
//!   system can't prove `pure` vs `effectful`. A pure-declared kernel
//!   whose body opens a socket compiles fine. Purity is documentation
//!   for downstream passes (transfer/reorder licences) and a contract
//!   the user upholds — not a static check.
//! - **Generic parameters, lifetimes, `where` clauses, attributes.**
//!   v2 kernels are concretely typed; generics on a kernel signature
//!   are out of scope. Detected at parse time as `TypeMismatch` if a
//!   declared scalar slot meets a generic parameter.
//! - **Multiple `kernels.rs` files / module-nested kernels.** All
//!   kernels must be top-level `pub fn` items in the single sibling
//!   `kernels.rs`. Anything else is invisible to this pass.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::algo::{AlgoIR, ResolvedKernel, ResolvedType, ScalarType};

// --------------------------------------------------------------------
// Public types
// --------------------------------------------------------------------

/// A single contract violation. Each variant names a distinct failure
/// mode; the link-step pattern (collect all and return as `Vec`) is
/// reused for parity with [`crate::LinkError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// A kernel declared in the algorithm has no matching `fn` item
    /// in `kernels.rs`.
    KernelNotFound { kernel: String },

    /// The `fn` exists but is not declared `pub`.
    MissingPub { kernel: String },

    /// The `fn` exists and is `pub` but the number of parameters does
    /// not match the declaration.
    ArityMismatch {
        kernel: String,
        expected_arity: usize,
        actual_arity: usize,
    },

    /// A specific parameter (or return) type does not match. `position`
    /// is `Some(i)` for the i-th (0-based) parameter, or `None` for
    /// the return type.
    TypeMismatch {
        kernel: String,
        position: Option<usize>,
        expected: String,
        actual: String,
    },

    /// `rustc` rejected `kernels.rs` for a reason unrelated to the
    /// declared contract (syntax error, missing item, etc.). The full
    /// stderr is preserved so the user can act on it.
    RustCheckFailed { stderr: String },

    /// `kernels.rs` could not be read (file missing, permission
    /// denied, ...). Distinct from `RustCheckFailed` because no
    /// compiler ever ran.
    KernelsFileUnreadable { path: PathBuf, io_error: String },

    /// `kernels.rs` was read but could not be parsed by `syn`. This
    /// usually means a syntax error rustc would also catch; we keep
    /// a separate variant so the test surface can distinguish "rustc
    /// said no" from "syn said no".
    KernelsFileUnparseable { path: PathBuf, parse_error: String },
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractError::KernelNotFound { kernel } => write!(
                f,
                "kernel `{kernel}` is declared in the algorithm but no matching `fn {kernel}` exists in kernels.rs"
            ),
            ContractError::MissingPub { kernel } => write!(
                f,
                "`fn {kernel}` in kernels.rs must be declared `pub`"
            ),
            ContractError::ArityMismatch {
                kernel,
                expected_arity,
                actual_arity,
            } => write!(
                f,
                "kernel `{kernel}`: declaration has {expected_arity} parameter(s), Rust function has {actual_arity}"
            ),
            ContractError::TypeMismatch {
                kernel,
                position,
                expected,
                actual,
            } => match position {
                Some(i) => write!(
                    f,
                    "kernel `{kernel}` parameter #{i}: declared as `{expected}`, Rust function has `{actual}`"
                ),
                None => write!(
                    f,
                    "kernel `{kernel}` return type: declared as `{expected}`, Rust function has `{actual}`"
                ),
            },
            ContractError::RustCheckFailed { stderr } => {
                write!(f, "rustc rejected kernels.rs:\n{stderr}")
            }
            ContractError::KernelsFileUnreadable { path, io_error } => write!(
                f,
                "could not read kernels.rs at {}: {}",
                path.display(),
                io_error
            ),
            ContractError::KernelsFileUnparseable { path, parse_error } => write!(
                f,
                "could not parse kernels.rs at {}: {}",
                path.display(),
                parse_error
            ),
        }
    }
}

impl std::error::Error for ContractError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Verify every kernel declaration in `algo` against the Rust file at
/// `kernels_rs_path`.
///
/// Returns `Ok(())` if every declared kernel has a matching
/// `pub fn` with compatible scalar signature AND `rustc` accepts the
/// file. Otherwise returns every contract violation found in one
/// pass — no fail-fast (parity with [`crate::link`](crate::link())).
///
/// Errors are deterministically ordered: file-level errors
/// (unreadable, unparseable, rustc rejection) appear first in the
/// order they were detected, followed by per-kernel errors sorted by
/// the kernel's declared name (which is the [`AlgoIR::kernels`]
/// `BTreeMap` iteration order, already deterministic).
pub fn check_kernels_contract(
    algo: &AlgoIR,
    kernels_rs_path: &Path,
) -> Result<(), Vec<ContractError>> {
    let mut errors: Vec<ContractError> = Vec::new();

    // Phase 0: read the file. Without it we can't do (1) or (2).
    let source = match std::fs::read_to_string(kernels_rs_path) {
        Ok(s) => s,
        Err(e) => {
            errors.push(ContractError::KernelsFileUnreadable {
                path: kernels_rs_path.to_path_buf(),
                io_error: e.to_string(),
            });
            return Err(errors);
        }
    };

    // Phase 1: invoke rustc on the source. Capture stderr for the
    // RustCheckFailed variant; success is silent.
    if let Err(stderr) = rustc_check(kernels_rs_path) {
        errors.push(ContractError::RustCheckFailed { stderr });
        // Fall through. Even when rustc rejects the file, syn can
        // usually still produce a partial parse; reporting "all
        // kernels missing" because the file has a typo in one place
        // is unhelpful. We trust syn's robustness here.
    }

    // Phase 2: parse with syn and match signatures.
    let file: syn::File = match syn::parse_file(&source) {
        Ok(f) => f,
        Err(e) => {
            errors.push(ContractError::KernelsFileUnparseable {
                path: kernels_rs_path.to_path_buf(),
                parse_error: e.to_string(),
            });
            // No AST means no signature checks. Return what we have.
            return Err(errors);
        }
    };

    // Collect top-level `fn` items by name, regardless of `pub`. We
    // record visibility so MissingPub can be reported precisely.
    let rust_fns = collect_top_level_fns(&file);

    // Walk declared kernels in BTreeMap order (deterministic).
    for kernel in algo.kernels.values() {
        if let Some(err) = check_one_kernel(kernel, &rust_fns) {
            errors.push(err);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// --------------------------------------------------------------------
// Phase 1: rustc compile-check
// --------------------------------------------------------------------

/// Derive a valid Rust crate-name identifier from a kernels-file path
/// (TASK-0363). rustc's `--crate-name` (and the default derived from
/// the file stem) must be a valid identifier: only `[A-Za-z0-9_]`, and
/// not starting with a digit. We map every other character of the file
/// STEM to `_`, then prefix `_` if the result is empty or starts with a
/// digit. So `kernels.embedded.rs` → stem `kernels.embedded` →
/// `kernels_embedded`; a stem like `3d` → `_3d`. The resulting name is
/// internal to the throwaway metadata build, so collisions across two
/// different stems (e.g. `a.b` and `a-b` both → `a_b`) are harmless —
/// each rustc invocation compiles exactly one file.
fn sanitise_crate_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kernels");
    let mut name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        name.insert(0, '_');
    }
    name
}

/// Invoke `rustc --crate-name <sanitised> --emit=metadata
/// --crate-type=rlib --edition 2021` on the file. Output goes to a temp
/// path that we discard.
///
/// Returns `Err(stderr)` on non-zero exit, `Ok(())` on success. The
/// stderr includes warnings too; we deliberately do not filter for
/// "is this an error or warning" because rustc's exit code already
/// carries that signal.
fn rustc_check(path: &Path) -> Result<(), String> {
    // Per-invocation unique output filename so parallel callers
    // (e.g. cargo's threaded test harness running several fixtures)
    // don't fight over the same `.rmeta`. PID + nanos is enough
    // entropy for our use; if it ever collides, rustc emits an EEXIST
    // and we surface that verbatim via stderr.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out_path = std::env::temp_dir().join(format!(
        "nucleus_contract_check_{}_{}.rmeta",
        std::process::id(),
        nanos
    ));

    // TASK-0363: pass an explicit sanitised `--crate-name`. Without it
    // rustc derives the crate name from the file STEM, so a `--kernels`
    // file with a dot in its stem (e.g. `kernels.embedded.rs`, used by
    // the M11 ex14 sync sibling — TASK-0049.06) was rejected with
    // "invalid character `.` in crate name" — a spurious
    // RustCheckFailed that defeated the phase-1 compile-check for any
    // dotted-stem kernels file. `sanitise_crate_name` maps the stem to
    // a valid Rust identifier so rustc accepts it.
    let crate_name = sanitise_crate_name(path);
    let output = Command::new("rustc")
        .arg("--crate-name")
        .arg(&crate_name)
        .arg("--emit=metadata")
        .arg("--crate-type=rlib")
        .arg("--edition=2021")
        // Suppress warnings; rustc's exit code already carries the
        // pass/fail signal and kernels.rs is not expected to have
        // `#![...]` attributes that would trigger warnings.
        .arg("-A")
        .arg("warnings")
        .arg("-o")
        .arg(&out_path)
        .arg(path)
        .output()
        .map_err(|e| format!("failed to spawn rustc: {e}"))?;

    // Best-effort cleanup. We don't fail the check if removal fails;
    // the temp directory is OS-managed.
    let _ = std::fs::remove_file(&out_path);

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        Err(stderr)
    }
}

// --------------------------------------------------------------------
// Phase 2: signature parse + match
// --------------------------------------------------------------------

/// A top-level `fn` summary suitable for matching. We extract exactly
/// what we need from a [`syn::ItemFn`] so the matching logic doesn't
/// have to know about `syn` types.
struct RustFn {
    name: String,
    is_pub: bool,
    /// Parameter types as rendered token strings. Each entry is the
    /// type of one positional parameter; `self` is dropped (kernels
    /// are free functions per PRD §6.2.2).
    param_types: Vec<String>,
    /// Return type as a rendered token string; `None` for unit
    /// (`-> ()` or no `-> T`).
    return_type: Option<String>,
}

fn collect_top_level_fns(file: &syn::File) -> Vec<RustFn> {
    let mut out = Vec::new();
    for item in &file.items {
        if let syn::Item::Fn(item_fn) = item {
            out.push(summarise_fn(item_fn));
        }
    }
    out
}

fn summarise_fn(item_fn: &syn::ItemFn) -> RustFn {
    let name = item_fn.sig.ident.to_string();
    let is_pub = matches!(item_fn.vis, syn::Visibility::Public(_));

    let mut param_types = Vec::new();
    for input in &item_fn.sig.inputs {
        match input {
            // `self`, `&self`, `&mut self` — kernels don't have
            // receivers, but be defensive and skip.
            syn::FnArg::Receiver(_) => {}
            syn::FnArg::Typed(pat_ty) => {
                param_types.push(render_type(&pat_ty.ty));
            }
        }
    }

    let return_type = match &item_fn.sig.output {
        syn::ReturnType::Default => None,
        syn::ReturnType::Type(_, ty) => {
            let rendered = render_type(ty);
            if rendered == "()" {
                None
            } else {
                Some(rendered)
            }
        }
    };

    RustFn {
        name,
        is_pub,
        param_types,
        return_type,
    }
}

/// Render a `syn::Type` to a normalised string suitable for matching.
///
/// We use the token stream's `Display` impl, then collapse whitespace
/// so `& mut [f32 ; N]` and `&mut [f32; N]` compare equal. This is
/// purely a normalisation step for scalar matching — for aggregates
/// we don't try to match at all, but the string still appears in
/// `TypeMismatch::actual` for the user to read.
fn render_type(ty: &syn::Type) -> String {
    use quote::ToTokens;
    let tokens = ty.to_token_stream().to_string();
    // Collapse runs of whitespace into single spaces.
    let mut s = String::new();
    let mut prev_space = false;
    for ch in tokens.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                s.push(' ');
                prev_space = true;
            }
        } else {
            s.push(ch);
            prev_space = false;
        }
    }
    // For scalars this is already terminal; for aggregates we leave
    // it as-is and let the matcher report it verbatim.
    s.trim().to_string()
}

/// Check one declared kernel against the table of Rust functions.
/// Returns the first applicable error (priority order documented at
/// the module level) or `None` if the contract holds.
fn check_one_kernel(declared: &ResolvedKernel, rust_fns: &[RustFn]) -> Option<ContractError> {
    let Some(found) = rust_fns.iter().find(|f| f.name == declared.name) else {
        return Some(ContractError::KernelNotFound {
            kernel: declared.name.clone(),
        });
    };

    if !found.is_pub {
        return Some(ContractError::MissingPub {
            kernel: declared.name.clone(),
        });
    }

    let expected_arity = declared.params.len();
    let actual_arity = found.param_types.len();
    if expected_arity != actual_arity {
        return Some(ContractError::ArityMismatch {
            kernel: declared.name.clone(),
            expected_arity,
            actual_arity,
        });
    }

    // Per-parameter type check. Scalar-only; aggregate declared types
    // emit a TypeMismatch describing the limitation.
    for (i, (decl_ty, rust_ty)) in declared
        .params
        .iter()
        .zip(found.param_types.iter())
        .enumerate()
    {
        if let Some(err) = compare_scalar(decl_ty, rust_ty, &declared.name, Some(i)) {
            return Some(err);
        }
    }

    // Return-type check. Both sides must agree on unit vs typed; for
    // typed, both must be the same scalar.
    match (&declared.ret, &found.return_type) {
        (None, None) => {}
        (Some(decl_ty), Some(rust_ty)) => {
            if let Some(err) = compare_scalar(decl_ty, rust_ty, &declared.name, None) {
                return Some(err);
            }
        }
        (None, Some(rust_ty)) => {
            return Some(ContractError::TypeMismatch {
                kernel: declared.name.clone(),
                position: None,
                expected: "()".to_string(),
                actual: rust_ty.clone(),
            });
        }
        (Some(decl_ty), None) => {
            return Some(ContractError::TypeMismatch {
                kernel: declared.name.clone(),
                position: None,
                expected: nuc_type_display(decl_ty),
                actual: "()".to_string(),
            });
        }
    }

    None
}

/// Compare one declared Nuc type against one Rust type string.
///
/// Scalar-only: if the declared type has any dimensions (an
/// aggregate), we emit a TypeMismatch with a clear "not yet
/// supported" message rather than silently passing. If the declared
/// type is scalar, we match by terminal segment of the Rust type's
/// path; this accepts `f32`, `core::primitive::f32`,
/// `std::primitive::f32`, etc.
fn compare_scalar(
    declared: &ResolvedType,
    rust_ty: &str,
    kernel: &str,
    position: Option<usize>,
) -> Option<ContractError> {
    if !declared.is_scalar() {
        return Some(ContractError::TypeMismatch {
            kernel: kernel.to_string(),
            position,
            expected: nuc_type_display(declared),
            actual: format!(
                "{rust_ty} (aggregate type matching is not yet implemented; see TASK-0012 follow-ups)"
            ),
        });
    }

    let expected_scalar = scalar_str(&declared.scalar);
    let rust_terminal = terminal_segment(rust_ty);
    if rust_terminal == expected_scalar {
        None
    } else {
        Some(ContractError::TypeMismatch {
            kernel: kernel.to_string(),
            position,
            expected: expected_scalar.to_string(),
            actual: rust_ty.to_string(),
        })
    }
}

/// Return the terminal path segment of a rendered Rust type. For
/// `f32` this is `"f32"`; for `core::primitive::f32` this is
/// `"f32"`. For aggregates (`&[f32]`, `Box<[f32; 16]>`) the
/// "terminal segment" concept doesn't really apply — we return the
/// last alphanumeric run after the final `::` and let the scalar
/// equality test fail naturally.
fn terminal_segment(rust_ty: &str) -> &str {
    let trimmed = rust_ty.trim();
    match trimmed.rsplit("::").next() {
        Some(last) => last.trim(),
        None => trimmed,
    }
}

/// Map an algorithm-side scalar to its Rust spelling. The Nuc
/// grammar's scalar names already match Rust's, by design (PRD
/// §6.2.2 point #1: "Rust's type checker validates kernel bodies").
fn scalar_str(s: &ScalarType) -> &'static str {
    match s {
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

/// Render a Nuc [`ResolvedType`] for human display in error
/// messages: `f32` for scalars, `f32[H][W]` style for aggregates
/// (with concrete numeric dimensions, since they've been resolved by
/// the lowering pass).
fn nuc_type_display(t: &ResolvedType) -> String {
    let mut s = scalar_str(&t.scalar).to_string();
    for d in &t.dims {
        s.push('[');
        s.push_str(&d.to_string());
        s.push(']');
    }
    s
}
