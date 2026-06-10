//! Halo-inference unit tests (TASK-0460 content-preserving split).
//!
//! Carved from the single `tests` module that lived inline in
//! `halo_inference.rs`. This root module holds the shared fixture
//! helpers; the three test groups live in sibling submodules:
//!
//! - [`stencil`]            — direct affine + full-pipeline stencil pins.
//! - [`partition_aware`]    — TASK-0341.02.02.01 (B') fatality pins.
//! - [`gather_scatter`]     — TASK-0373/0384 gather/scatter + whitebox.
//!
//! The `pub(super) use` re-exports below make the parent-module symbols
//! (the public entry points + the carved `pub(super)` whitebox helpers)
//! reachable from the group submodules via `use super::*`.

pub(super) use super::*;
pub(super) use super::partition_policy::scatter_target_replicates_whole_array;
pub(super) use super::walker::{collect_from_stmts, WalkCtx};
pub(super) use crate::algo::{
    AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, Purity, ResolvedConst, ResolvedData,
    ResolvedKernel, ResolvedType, ScalarType,
};
pub(super) use crate::link::link;
pub(super) use crate::sched::{
    ResolvedLoopDirective, ResolvedLoopOption, ResolvedPlaceTarget, ResolvedPlacement,
    ResolvedWorker, SchedIR,
};

mod gather_scatter;
mod partition_aware;
mod stencil;

// ---- Helpers ----

pub(super) fn t_scalar(ty: ScalarType) -> ResolvedType {
    ResolvedType {
        scalar: ty,
        dims: vec![],
    }
}
pub(super) fn t_arr(ty: ScalarType, dims: Vec<usize>) -> ResolvedType {
    ResolvedType { scalar: ty, dims }
}

/// Build a tiny LinkedIR for halo-inference tests. The shape is:
/// - one data symbol `grid` of given dims and one out symbol `out`
/// - one kernel `K` (pure, params/ret types are irrelevant to halo)
/// - placement: K on a single worker `w0`
/// - the body statements are provided by the caller.
pub(super) fn build_linked(stmts: Vec<IrStmt>, grid_dims: Vec<usize>) -> LinkedIR {
    let mut data = BTreeMap::new();
    data.insert(
        "grid".to_string(),
        ResolvedData {
            name: "grid".to_string(),
            ty: t_arr(ScalarType::I32, grid_dims.clone()),
        },
    );
    data.insert(
        "out".to_string(),
        ResolvedData {
            name: "out".to_string(),
            ty: t_arr(ScalarType::I32, grid_dims),
        },
    );
    let mut kernels = BTreeMap::new();
    kernels.insert(
        "K".to_string(),
        ResolvedKernel {
            name: "K".to_string(),
            params: vec![t_scalar(ScalarType::I32)],
            ret: Some(t_scalar(ScalarType::I32)),
            purity: Purity::Pure,
            combine: None,
            name_span: None,
        },
    );
    let algo = AlgoIR {
        consts: BTreeMap::new(),
        data,
        kernels,
        stmts,
        // Decl order is inert for halo inference (this fixture builds
        // `data` directly, not from source); empty (TASK-0049.10.06).
        data_decl_order: Vec::new(),
    };

    // Minimal SchedIR: one placement of K on a single worker.
    let mut places: BTreeMap<String, ResolvedPlacement> = BTreeMap::new();
    places.insert(
        "K".to_string(),
        ResolvedPlacement {
            kernel: "K".to_string(),
            target: ResolvedPlaceTarget::One("w0".to_string()),
            kernel_span: None,
        },
    );
    let mut workers: BTreeMap<String, ResolvedWorker> = BTreeMap::new();
    workers.insert(
        "w0".to_string(),
        ResolvedWorker {
            name: "w0".to_string(),
            class: crate::sched::DEFAULT_WORKER_CLASS.to_string(),
        },
    );
    let sched = SchedIR {
        algo_path: String::new(),
        worker_classes: BTreeMap::new(),
        memory_regions: BTreeMap::new(),
        workers,
        places,
        place_data: BTreeMap::new(),
        loops: BTreeMap::new(),
        transfers: BTreeMap::new(),
        checks: BTreeMap::new(),
    };

    link(algo, sched).expect("link must succeed for halo test fixtures")
}

pub(super) fn ir_int(v: i64) -> IrExpr {
    IrExpr::IntLit(v)
}
pub(super) fn ir_id(s: &str) -> IrExpr {
    IrExpr::Ident(s.to_string())
}
pub(super) fn ir_add(l: IrExpr, r: IrExpr) -> IrExpr {
    IrExpr::BinOp(IrBinOp::Add, Box::new(l), Box::new(r))
}
pub(super) fn ir_sub(l: IrExpr, r: IrExpr) -> IrExpr {
    IrExpr::BinOp(IrBinOp::Sub, Box::new(l), Box::new(r))
}
pub(super) fn ir_mul(l: IrExpr, r: IrExpr) -> IrExpr {
    IrExpr::BinOp(IrBinOp::Mul, Box::new(l), Box::new(r))
}
pub(super) fn ir_call(callee: &str, args: Vec<IrExpr>) -> IrExpr {
    IrExpr::Call {
        callee: callee.to_string(),
        args,
    }
}
pub(super) fn data_ref(name: &str, indices: Vec<IrExpr>) -> IrExpr {
    IrExpr::DataRef(IndexedRef {
        name: name.to_string(),
        indices,
    })
}
pub(super) fn lhs(name: &str, indices: Vec<IrExpr>) -> IndexedRef {
    IndexedRef {
        name: name.to_string(),
        indices,
    }
}

/// Construct a tiny ResolvedLoopDirective adding a
/// `partition=workers` option to the named iv. The exact
/// PartitionKind is irrelevant to `iv_is_partitioned` (it just
/// checks for any `Partition(_)`), but `Workers` is the lowest-
/// dependency variant for synthetic fixtures.
pub(super) fn loop_partition_workers(iv: &str) -> ResolvedLoopDirective {
    ResolvedLoopDirective {
        var: iv.to_string(),
        options: vec![ResolvedLoopOption::Partition(
            crate::sched::PartitionKind::Workers,
        )],
        var_span: None,
    }
}
