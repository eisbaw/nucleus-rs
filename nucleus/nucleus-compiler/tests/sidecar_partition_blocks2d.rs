//! Tests for `NameSidecar::partition_pairs` +
//! `NameSidecar::grid_shape_for_outer_iv` (TASK-0264 cycle 113, AC#1+2).
//!
//! Pins:
//!
//! 1. After `apply_partition_blocks2d` on a 2D nest with a
//!    `partition=blocks2d` directive, the ACFG sidecar records the
//!    (outer, inner) IterVar pairing AND the `(rows, cols)` grid shape
//!    derived from `decompose_grid(num_workers)`. The codegen contract
//!    surface (`NameSidecar`) mirrors both verbatim via `build_sidecar`.
//!
//! 2. `apply_partition_blocks2d` on a schedule WITHOUT a blocks2d
//!    directive leaves both maps empty — additive-only contract: no
//!    behavioural drift for the 4 tier-1 backends today, which never
//!    consume the new fields.
//!
//! 3. The serde round-trip preserves both new fields verbatim AND an
//!    older wire payload (synthesised by dropping the fields)
//!    deserialises with both defaulting to empty.
//!
//! Mirrors `sidecar_halo.rs` (TASK-0260) + `sidecar_reuse.rs` (TASK-0261)
//! exactly — the load-bearing precedent for additive sidecar fields.
//!
//! The pipeline below intentionally MIRRORS what the driver runs
//! (block_transforms → partition_workers → partition_rows →
//! partition_blocks2d → halo_inference → sync_inject → transfer_inject)
//! so the round-trip exercises the same shape codegen sees.
//!
//! TASK-0289 will land the FIRST consumer (halo-strip Push/Wait
//! synthesis between N/S/E/W neighbours in the 2D worker grid); these
//! tests are the contract pin so a future regression that drops the
//! new fields fires loud here rather than silently breaking that
//! downstream pass.

use std::fs;
use std::path::PathBuf;

use nucleus_compiler::{
    algo::{lower_algo, parse_algo},
    apply_block_transforms, apply_halo_inference_partition_aware, apply_partition_blocks2d,
    apply_partition_rows, apply_partition_workers, build_acfg, build_sidecar, inject_syncs,
    inject_transfers, link,
    sched::{lower_sched, parse_sched},
};

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("two ancestors above compiler crate")
}

/// Run the full lower-link-inject pipeline for a given example/schedule.
/// Mirrors `sidecar_halo.rs::lower_partition_aware` (TASK-0309 cycle
/// 128 — the driver-aligned variant that calls
/// `fn apply_halo_inference_partition_aware`) so the sidecar shape is
/// the driver's shape (partition-aware-B).
fn lower(
    ex_rel: &str,
    sched_rel: &str,
) -> (nucleus_compiler::link::LinkedIR, nucleus_compiler::ACFG) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(ex_rel);
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).expect("read algo");
    let sched_src = fs::read_to_string(ex.join(sched_rel)).expect("read sched");

    let algo_ir = lower_algo(&parse_algo(&algo_src).expect("parse_algo")).expect("lower_algo");
    let sched_ir =
        lower_sched(&parse_sched(&sched_src).expect("parse_sched")).expect("lower_sched");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block_transforms");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");
    let acfg = apply_partition_rows(&linked, acfg).expect("partition_rows");
    let acfg = apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d");
    let (acfg, _) = apply_halo_inference_partition_aware(&linked, acfg).expect("halo_inference");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    (linked, acfg)
}

#[test]
fn shipped_examples_without_blocks2d_leave_maps_empty() {
    // No shipped schedule today (cycle 113) carries `partition=blocks2d`.
    // Every example in the e2e matrix exercises only partition=workers
    // (e.g. 05-stencil/distributed) or partition=rows (synthetic tests
    // only) or no partition. So under every shipped schedule the new
    // sidecar maps remain empty after `apply_partition_blocks2d` — the
    // additive-only contract that keeps every cell byte-identical
    // under codegen.
    for (ex, sched) in &[
        ("01-elementwise-add", "schedules/naive.sched.nuc"),
        ("05-stencil", "schedules/naive.sched.nuc"),
        ("05-stencil", "schedules/distributed.sched.nuc"),
        ("05-stencil", "schedules/reuse.sched.nuc"),
    ] {
        let (linked, acfg) = lower(ex, sched);
        assert!(
            acfg.partition_pairs.is_empty(),
            "{ex}/{sched}: expected empty partition_pairs (no blocks2d directive); got {:?}",
            acfg.partition_pairs
        );
        assert!(
            acfg.grid_shape_for_outer_iv.is_empty(),
            "{ex}/{sched}: expected empty grid_shape_for_outer_iv; got {:?}",
            acfg.grid_shape_for_outer_iv
        );
        let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
        assert_eq!(
            sidecar.partition_pairs, acfg.partition_pairs,
            "{ex}/{sched}: NameSidecar.partition_pairs must mirror ACFG"
        );
        assert_eq!(
            sidecar.grid_shape_for_outer_iv, acfg.grid_shape_for_outer_iv,
            "{ex}/{sched}: NameSidecar.grid_shape_for_outer_iv must mirror ACFG"
        );
    }
}

#[cfg(feature = "serde")]
#[test]
fn partition_pairs_and_grid_shape_serde_roundtrip() {
    // The hand-built integration tests in tests/partition_blocks2d.rs
    // already pin the WRITE side (apply_partition_blocks2d records
    // pair + grid shape). This test pins the WIRE shape: both new
    // fields survive serde JSON round-trip byte-for-byte.
    //
    // Synthesised payload to ensure non-empty maps (no shipped
    // schedule exercises blocks2d today; the round-trip would be
    // observationally inert on a real example. Use a hand-built
    // NameSidecar with known entries).
    use nucleus_compiler::event::IterVar;
    use std::collections::BTreeMap;

    let mut sidecar = nucleus_compiler::NameSidecar::default();
    let mut pairs: BTreeMap<IterVar, IterVar> = BTreeMap::new();
    pairs.insert(IterVar(7), IterVar(8));
    pairs.insert(IterVar(11), IterVar(12));
    sidecar.partition_pairs = pairs.clone();

    let mut grid: BTreeMap<IterVar, (u32, u32)> = BTreeMap::new();
    grid.insert(IterVar(7), (2, 3));
    grid.insert(IterVar(11), (4, 4));
    sidecar.grid_shape_for_outer_iv = grid.clone();

    let json = serde_json::to_string(&sidecar).expect("serialise NameSidecar");
    let back: nucleus_compiler::NameSidecar =
        serde_json::from_str(&json).expect("deserialise NameSidecar");

    assert_eq!(
        back.partition_pairs, pairs,
        "partition_pairs must survive serde JSON round-trip"
    );
    assert_eq!(
        back.grid_shape_for_outer_iv, grid,
        "grid_shape_for_outer_iv must survive serde JSON round-trip"
    );
}

#[cfg(feature = "serde")]
#[test]
fn partition_pairs_serde_default_on_missing_field() {
    // An "old" wire payload that omits both new fields must deserialise
    // to empty maps (additive-only contract, mirroring TASK-0233/
    // TASK-0455.08's xfer_facts + TASK-0260's halo_widths + TASK-0261's
    // reuse_widths).
    let (linked, acfg) = lower("01-elementwise-add", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let value: serde_json::Value =
        serde_json::to_value(&sidecar).expect("serialise NameSidecar to Value");
    let mut obj = value
        .as_object()
        .expect("NameSidecar serialises to JSON object")
        .clone();
    obj.remove("partition_pairs");
    obj.remove("grid_shape_for_outer_iv");
    let pruned = serde_json::Value::Object(obj);
    let stripped_json = serde_json::to_string(&pruned).expect("re-serialise");
    let back: nucleus_compiler::NameSidecar = serde_json::from_str(&stripped_json)
        .expect("payload without partition_pairs / grid_shape_for_outer_iv must deserialise");
    assert!(
        back.partition_pairs.is_empty(),
        "missing partition_pairs field must default to empty map"
    );
    assert!(
        back.grid_shape_for_outer_iv.is_empty(),
        "missing grid_shape_for_outer_iv field must default to empty map"
    );
}
