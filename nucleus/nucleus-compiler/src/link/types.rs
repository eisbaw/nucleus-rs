//! Public types produced by the link step.
//!
//! `WorkerEntity` is the comparable worker-set the link pass reasons
//! about; `LinkedIR` is the output of a successful link (the two
//! source IRs plus resolved placement / producer / consumer maps).
//! Both are consumed by every downstream compiler pass (ACFG build,
//! transfer_inject, partition_*, halo_inference, sidecar, ...).

use std::collections::{BTreeMap, BTreeSet};

use crate::algo::AlgoIR;
use crate::sched::{ResolvedPlaceTarget, ResolvedPlacement, SchedIR};

/// A worker placement collapsed into a comparable set.
///
/// `One("host")` and `Many({w0, w1})` are both represented as a
/// `BTreeSet<String>`; the size and contents distinguish them. Using a
/// `BTreeSet` means equality is order-independent (so
/// `{w0, w1} == {w1, w0}`) and the type implements `Ord` so it can
/// key error-message maps deterministically.
///
/// Conceptually this is the "worker entity" the link step reasons
/// about. The PRD's distributed-placement note in §6.3.2 says the
/// compiler eventually partitions the iter space across the named
/// workers; that partitioning runs in the downstream `partition_*`
/// passes (TASK-0117 + TASK-0258 + TASK-0259, all Done) — the link
/// step itself treats the whole set as one identity for
/// transfer-existence purposes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkerEntity(pub BTreeSet<String>);

impl WorkerEntity {
    pub(super) fn from_target(t: &ResolvedPlaceTarget) -> Self {
        match t {
            ResolvedPlaceTarget::One(w) => {
                let mut s = BTreeSet::new();
                s.insert(w.clone());
                WorkerEntity(s)
            }
            ResolvedPlaceTarget::Many(ws) => WorkerEntity(ws.iter().cloned().collect()),
        }
    }

    /// Stable, human-readable rendering for error messages and
    /// diagnostic maps. `{host}` for singletons, `{w0,w1,w2}` for
    /// sets; deterministic order (BTreeSet iterates sorted).
    pub fn display(&self) -> String {
        let names: Vec<&str> = self.0.iter().map(|s| s.as_str()).collect();
        format!("{{{}}}", names.join(","))
    }
}

/// The result of linking an [`AlgoIR`] and a [`SchedIR`].
///
/// The two source IRs are kept verbatim. Cross-references resolved
/// during linking are exposed as separate maps; downstream passes
/// (ACFG, Petri-net construction) read them rather than re-deriving
/// them from the source IRs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedIR {
    pub algo: AlgoIR,
    pub sched: SchedIR,
    /// For each kernel declared in the algorithm, the placement
    /// directive from the schedule. Keyed by kernel name.
    ///
    /// Invariant after a successful link: this map's keyset equals
    /// `algo.kernels` keyset. (Linking fails if a kernel is unplaced
    /// or if the schedule places a kernel that doesn't exist.)
    pub placements: BTreeMap<String, ResolvedPlacement>,
    /// For each kernel, the resolved worker entity it runs on.
    /// Convenience derivation of [`Self::placements`] — derived once
    /// here so downstream passes don't re-walk targets.
    pub kernel_workers: BTreeMap<String, WorkerEntity>,
    /// For each data symbol that has at least one producer kernel,
    /// the worker entity that produces it. A producer is the kernel
    /// on the RHS of `D <-- Call(...)` in the algorithm.
    ///
    /// Symbols with no producer (e.g. read-only inputs, identity-
    /// copy targets) are omitted. Symbols with multiple producers
    /// across statements: the AlgoIR lowering pass enforces single-
    /// assignment per scope (PRD §6.2.1), so this is unique per
    /// scope, but a `For`-loop body assigning `D[n]` per iteration
    /// is treated as a single producer placement (the body's kernel
    /// placement). We currently record the LAST observed producer
    /// placement — pre-condition: AlgoIR's single-assignment check
    /// has already rejected genuinely-duplicate producers.
    pub data_producers: BTreeMap<String, WorkerEntity>,
    /// For each data symbol read by some kernel, the set of worker
    /// entities that consume it. Multiple distinct consumer entities
    /// are normal (a hub data feeding several workers).
    pub data_consumers: BTreeMap<String, BTreeSet<WorkerEntity>>,
}
