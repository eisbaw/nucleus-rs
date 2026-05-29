//! `EmitError` — the codegen-time error type, re-exported by every
//! tier-1 backend. Split from `render.rs` (TASK-0244 carries the
//! historical move-from-pthreads-sync; this is a mechanical no-
//! behaviour split for file-size hygiene).

use std::io;
use std::path::PathBuf;

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
    /// The structural overlapping-write-accumulator detector
    /// ([`crate::multi_worker_walker::collect_accumulate_waits`]) fired
    /// for a data symbol — `N>=2` whole-array `Wait`s — but the
    /// algorithm-IR shows the symbol is NOT an algorithm-level
    /// accumulator (its `Dataflow` LHS name does not appear among the
    /// RHS data references). Emitting element-wise `wrapping_add`
    /// combine here would be a SILENT miscompile (sum-combining values
    /// that are not accumulator partials), so the driver-level
    /// cross-check
    /// ([`crate::multi_worker_walker::check_accumulator_consistency`],
    /// TASK-0343.03) rejects loudly BEFORE any backend codegen.
    ///
    /// For every shipped schedule the structural pattern and the
    /// algorithm-level accumulator shape coincide (08-histogram/
    /// distributed), so this never fires on the e2e matrix; it guards
    /// against an exotic future schedule that emits multiple whole-array
    /// pushes for non-accumulator semantics.
    AccumulatorShapeMismatch(String),
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
            EmitError::AccumulatorShapeMismatch(msg) => {
                write!(f, "accumulator shape mismatch: {msg}")
            }
        }
    }
}

impl std::error::Error for EmitError {}
