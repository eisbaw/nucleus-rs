//! Shared test fixtures for the `backend-common` integration tests
//! (TASK-0358).
//!
//! Before this module, the `(NameTables, NameSidecar)` fixture
//! builders had drifted into FIVE near-identical copies across five
//! test files (four `make_minimal_tables` + one `make_tables`,
//! colliding on the name across two unrelated signatures), and the
//! `tile_{1,2,3}d` / `empty_accumulate_and_indexed` helpers were
//! duplicated byte-for-byte across three more. Any change to
//! [`NameTables`] / [`NameSidecar`] meant updating every copy
//! independently — the exact `feedback-silent-sibling-defect` surface
//! TASK-0237 first flagged at cycle 37.
//!
//! The single construction primitive is [`Tables`], a chainable
//! builder: each test declares precisely the table entries it needs
//! and nothing more, so the per-field construction logic (e.g. the
//! `(i64) -> ()` kernel-sig shape) lives in exactly one place.
//!
//! `#![allow(dead_code)]` is load-bearing: cargo compiles
//! `tests/common/mod.rs` into EVERY integration test crate that
//! declares `mod common;`, and each binary uses only a subset of the
//! helpers. Without the allow, the unused helpers in any given binary
//! would trip the workspace `cargo clippy --all-targets -- -D warnings`
//! gate (`just clippy`).

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::ops::Range;

use nucleus_compiler::algo::{CombineOp, IrExpr, Purity, ResolvedType, ScalarType};
use nucleus_compiler::event::{DataId, IterTile, IterVar, KernelId, WorkerId};
use nucleus_compiler::sidecar::{KernelSig, LoopBound, NameSidecar};
use nucleus_compiler::NameTables;

/// Chainable builder for the minimal `(NameTables, NameSidecar)`
/// fixture pairs the backend-common walker / wait / slice tests drive.
///
/// Each `with_*` method populates exactly one table entry and returns
/// `self`, so a call site reads as the precise list of names/sidecar
/// state it sets up. [`build`](Tables::build) hands back the pair.
#[derive(Default)]
pub struct Tables {
    names: NameTables,
    sidecar: NameSidecar,
}

impl Tables {
    pub fn new() -> Self {
        Self::default()
    }

    /// Data symbol with a resolved `i32[dims]` type: inserts both the
    /// `names.data` name and the `sidecar.data_types` entry. The
    /// dataflow-fixture shape (Family A). Shorthand for
    /// [`with_data_typed`](Tables::with_data_typed) with an `i32`
    /// scalar.
    pub fn with_data(self, data: DataId, name: &str, dims: Vec<usize>) -> Self {
        self.with_data_typed(data, name, ScalarType::I32, dims)
    }

    /// Data symbol with an explicit scalar type — for fixtures that
    /// exercise a non-`i32` element type (e.g. the float-accumulate
    /// `ContractGap` arm). Inserts both `names.data` and
    /// `sidecar.data_types`.
    pub fn with_data_typed(
        mut self,
        data: DataId,
        name: &str,
        scalar: ScalarType,
        dims: Vec<usize>,
    ) -> Self {
        self.names.data.insert(data, name.to_string());
        self.sidecar
            .data_types
            .insert(data, ResolvedType { scalar, dims });
        self
    }

    /// Data symbol NAME only — no `sidecar.data_types` entry. Used by
    /// fixtures whose code-under-test never reads the resolved type
    /// (the reuse-marker walker path), so adding a `data_types` entry
    /// would change the fixture state without cause.
    pub fn with_data_name(mut self, data: DataId, name: &str) -> Self {
        self.names.data.insert(data, name.to_string());
        self
    }

    pub fn with_worker(mut self, worker: WorkerId, name: &str) -> Self {
        self.names.worker.insert(worker, name.to_string());
        self
    }

    /// The `WorkerId(0)="w0"` + `WorkerId(1)="host"` pair the
    /// wait-assign slice tests use for their two-worker host gather.
    pub fn with_default_workers(self) -> Self {
        self.with_worker(WorkerId(0), "w0")
            .with_worker(WorkerId(1), "host")
    }

    pub fn with_iter_var(mut self, iv: IterVar, name: &str) -> Self {
        self.names.iter_var.insert(iv, name.to_string());
        self
    }

    pub fn with_loop_bound(mut self, iv: IterVar, lo: i64, hi: i64) -> Self {
        self.sidecar.loop_bounds.insert(
            iv,
            LoopBound {
                lo: IrExpr::IntLit(lo),
                hi: IrExpr::IntLit(hi),
            },
        );
        self
    }

    /// Kernel NAME plus a scalar `(i64) -> ()` signature. The walker's
    /// `render_fire_args_pub` joins on this sig; the two Family-B
    /// fixtures (reuse-marker, blocked-rebind) both insert exactly this
    /// shape, so its absence trips a different error than the one those
    /// tests target.
    pub fn with_kernel_i64(mut self, kernel: KernelId, name: &str) -> Self {
        self.names.kernel.insert(kernel, name.to_string());
        self.sidecar.kernel_sigs.insert(
            kernel,
            KernelSig {
                params: vec![ResolvedType {
                    scalar: ScalarType::I64,
                    dims: vec![],
                }],
                ret: None,
                purity: Purity::Pure,
                combine: None,
            },
        );
        self
    }

    /// Declare the overlapping-write combine identity for an accumulator
    /// `DataId` (TASK-0343.01.01): populates `sidecar.combine_for_data`.
    /// A fixture exercising the accumulate fan-in MUST set this for every
    /// accumulator data symbol, or `check_accumulator_consistency` /
    /// `render_accumulate_assign` fail loud (the AC#4 soundness reject).
    pub fn with_combine_for_data(mut self, data: DataId, op: CombineOp) -> Self {
        self.sidecar.combine_for_data.insert(data, op);
        self
    }

    pub fn build(self) -> (NameTables, NameSidecar) {
        (self.names, self.sidecar)
    }
}

/// Family-A data fixture, no worker entries: `names.data` +
/// `sidecar.data_types(i32[dims])`.
pub fn make_minimal_tables(
    data: DataId,
    data_name: &str,
    dims: Vec<usize>,
) -> (NameTables, NameSidecar) {
    Tables::new().with_data(data, data_name, dims).build()
}

/// Family-A data fixture plus the default two-worker entries
/// (`WorkerId(0)="w0"`, `WorkerId(1)="host"`) the host-gather slice
/// tests read out of `names`.
pub fn make_minimal_tables_with_workers(
    data: DataId,
    data_name: &str,
    dims: Vec<usize>,
) -> (NameTables, NameSidecar) {
    Tables::new()
        .with_data(data, data_name, dims)
        .with_default_workers()
        .build()
}

/// 1-D iteration tile: one `(IterVar, Range)` axis.
pub fn tile_1d(iv: u64, range: Range<i64>) -> IterTile {
    IterTile::new(vec![(IterVar(iv), range)])
}

/// 2-D iteration tile: outer then inner `(IterVar, Range)` axis.
pub fn tile_2d(iv0: u64, r0: Range<i64>, iv1: u64, r1: Range<i64>) -> IterTile {
    IterTile::new(vec![(IterVar(iv0), r0), (IterVar(iv1), r1)])
}

/// 3-D iteration tile: outer-to-inner `(IterVar, Range)` axes.
pub fn tile_3d(
    iv0: u64,
    r0: Range<i64>,
    iv1: u64,
    r1: Range<i64>,
    iv2: u64,
    r2: Range<i64>,
) -> IterTile {
    IterTile::new(vec![
        (IterVar(iv0), r0),
        (IterVar(iv1), r1),
        (IterVar(iv2), r2),
    ])
}

/// Empty `(accumulate_data, indexed_input)` arg pair for the
/// let-at-wait classifier — the common "no accumulate / no indexed
/// input" case.
pub fn empty_accumulate_and_indexed() -> (BTreeSet<DataId>, BTreeSet<DataId>) {
    (BTreeSet::new(), BTreeSet::new())
}
