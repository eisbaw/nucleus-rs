//! Reverse name tables (TASK-0238 moved here from `pthreads-sync`).
//!
//! # Why this module lives in `compiler`
//!
//! `NameTables` is the inverse of `ACFG::name_*`: it maps the opaque
//! id types (`DataId`, `KernelId`, `WorkerId`, `IterVar`) back to the
//! source names. The struct holds zero backend-specific content — it
//! is the join key every codegen consumer needs against the
//! `EventList`'s opaque ids and the `NameSidecar`'s typed data.
//!
//! Historically the struct lived in `pthreads-sync` (path of least
//! resistance during TASK-0124 / the first backend); mp-tcp-bufsync
//! and pthreads-async re-exported it. That created two pain points:
//!
//! 1. **The cross-backend test-helper crate (`test-common`,
//!    TASK-0237) could not depend on pthreads-sync** (would be a
//!    circular dep — pthreads-sync dev-deps test-common). So
//!    `lower_for_test` returned the 5 raw reverse-name-table maps
//!    and each backend test composed its own NameTables in a 5-line
//!    literal block (3 sites today, 4 with Wave B-2).
//!
//! 2. **Adding a new field to NameTables forces synchronous edits
//!    at every composition site.** The driver + 3 backend tests
//!    each carry a 5-field literal; adding a 6th means 4 updates.
//!
//! Moving NameTables into compiler dissolves both. test-common can
//! now return a pre-built NameTables in its result, the driver gets
//! a `NameTables::from_acfg(&acfg)` helper, and a future field-add
//! lands at a single site.
//!
//! Cycle-24 review-gate B.1 + cycle-25 TASK-0238.

use std::collections::{BTreeMap, BTreeSet};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::event::{DataId, IterVar, KernelId, WorkerId};

/// Reverse name tables travelling alongside the per-worker
/// `EventList` + the [`crate::sidecar::NameSidecar`]. Each map is the
/// inverse of the corresponding `ACFG::name_*` table (`name -> id`
/// inverted to `id -> name`). The backend joins these against the
/// opaque ids the `Event`s / sidecar carry — exactly the join the
/// proven reconstruction tests in
/// `compiler/tests/petri_to_events.rs` perform. The driver builds
/// these from the post-pass ACFG; the backend never sees the ACFG
/// itself.
///
/// Pre-TASK-0238 history: this struct lived in `pthreads-sync`. Moved
/// to `compiler` so the cross-backend test-helper crate `test-common`
/// (TASK-0237) can return a pre-built instance instead of 5 raw
/// BTreeMaps + a BTreeSet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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
    /// LIMITATION (TASK-0173): see the pre-TASK-0238 docstring in
    /// `pthreads-sync` git history for the full discussion of the
    /// non-divisible / trailing-partial-tile case. Summary: the
    /// rebinding is correct only for the evenly-divisible case; the
    /// non-divisible case decomposes into two sibling nests whose
    /// correct absolute formulas differ.
    pub inner_block_iter_vars: BTreeSet<IterVar>,
}

impl NameTables {
    /// Build a `NameTables` from a post-pass ACFG by inverting its
    /// `name_*` maps. Centralises the 5-field literal block that
    /// previously appeared at every codegen-consumer site (driver,
    /// each backend test).
    ///
    /// Equivalent to the historical literal:
    ///
    /// ```ignore
    /// NameTables {
    ///     data:    acfg.name_data.iter().map(|(n,i)| (*i, n.clone())).collect(),
    ///     kernel:  acfg.name_kernels.iter().map(|(n,i)| (*i, n.clone())).collect(),
    ///     worker:  acfg.name_workers.iter().map(|(n,i)| (*i, n.clone())).collect(),
    ///     iter_var: acfg.name_iter_vars.iter().map(|(n,i)| (*i, n.clone())).collect(),
    ///     inner_block_iter_vars: acfg.inner_block_iter_vars.clone(),
    /// }
    /// ```
    pub fn from_acfg(acfg: &crate::acfg::ACFG) -> Self {
        NameTables {
            data: acfg
                .name_data
                .iter()
                .map(|(n, i)| (*i, n.clone()))
                .collect(),
            kernel: acfg
                .name_kernels
                .iter()
                .map(|(n, i)| (*i, n.clone()))
                .collect(),
            worker: acfg
                .name_workers
                .iter()
                .map(|(n, i)| (*i, n.clone()))
                .collect(),
            iter_var: acfg
                .name_iter_vars
                .iter()
                .map(|(n, i)| (*i, n.clone()))
                .collect(),
            inner_block_iter_vars: acfg.inner_block_iter_vars.clone(),
        }
    }
}
