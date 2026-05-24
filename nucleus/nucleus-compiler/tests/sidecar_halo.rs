//! Tests for `NameSidecar::halo_widths` (TASK-0260, Stage 1).
//!
//! Pins both halves of the invariant:
//!
//! 1. A 3x3 stencil algorithm (example 05) produces non-trivial halo
//!    entries for both axes after `apply_halo_inference`. The pipeline
//!    must populate the ACFG sidecar AND the NameSidecar (the codegen
//!    contract surface) so the Stage 2 consumer (TASK-0263,
//!    transfer_inject extension) has a path to read it.
//!
//! 2. A non-stencil algorithm (example 01 / naive) produces an EMPTY
//!    halo_widths map — Stage 1 records nothing where there's nothing
//!    to record. This pins the additive-only contract: existing
//!    examples remain byte-identical under codegen because no consumer
//!    has wired through yet (the field is observationally inert).
//!
//! 3. The serde round-trip preserves `halo_widths` AND an older
//!    payload (synthesised by dropping the field) deserialises with
//!    `halo_widths` defaulting to an empty map.

use std::fs;
use std::path::PathBuf;

use nucleus_compiler::{
    algo::{lower_algo, parse_algo},
    apply_block_transforms, apply_halo_inference, apply_partition_blocks2d, apply_partition_rows,
    apply_partition_workers, build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
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
/// Mirrors `sidecar_buffer.rs::lower` but includes the three partition
/// passes + `apply_halo_inference` so the sidecar is populated as the
/// driver does it.
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
    let acfg = apply_halo_inference(&linked, acfg).expect("halo_inference");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    (linked, acfg)
}

#[test]
fn stencil_3x3_produces_halo_one_on_both_axes() {
    // Example 05 / 3x3 stencil, naive schedule. The algorithm reads
    // img_in[y±1][x±1] (and centre + edges); halo inference must
    // record (blur3, y) -> 1 AND (blur3, x) -> 1.
    let (linked, acfg) = lower("05-stencil", "schedules/naive.sched.nuc");

    let kid = *acfg.name_kernels.get("blur3").expect("blur3 in ACFG");
    let y_iv = *acfg.name_iter_vars.get("y").expect("y in ACFG");
    let x_iv = *acfg.name_iter_vars.get("x").expect("x in ACFG");

    let y_halo = acfg
        .halo_widths
        .get(&kid)
        .and_then(|m| m.get(&y_iv))
        .copied();
    let x_halo = acfg
        .halo_widths
        .get(&kid)
        .and_then(|m| m.get(&x_iv))
        .copied();
    assert_eq!(
        y_halo,
        Some(1),
        "halo_widths[blur3][y] must be 1 (offsets -1, 0, +1)"
    );
    assert_eq!(
        x_halo,
        Some(1),
        "halo_widths[blur3][x] must be 1 (offsets -1, 0, +1)"
    );

    // The codegen-contract surface (NameSidecar) must mirror the ACFG
    // sidecar verbatim.
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert_eq!(
        sidecar.halo_widths, acfg.halo_widths,
        "NameSidecar.halo_widths must mirror ACFG.halo_widths"
    );
    assert_eq!(
        sidecar
            .halo_widths
            .get(&kid)
            .and_then(|m| m.get(&y_iv))
            .copied(),
        Some(1)
    );
    assert_eq!(
        sidecar
            .halo_widths
            .get(&kid)
            .and_then(|m| m.get(&x_iv))
            .copied(),
        Some(1)
    );
}

#[test]
fn elementwise_add_records_only_zero_halos() {
    // Example 01 (elementwise-add): kernel reads `a[i]` and `b[i]` —
    // both bare-iv reads, halo offset 0. The implementation records
    // an explicit 0-width entry per (kernel, iter-var) the detector
    // inspects (see `halo_inference.rs::no_halo_bare_iv` for the
    // contract rationale). The MAX halo width across all entries
    // must therefore be 0 — equivalent to "no halo needed".
    let (linked, acfg) = lower("01-elementwise-add", "schedules/naive.sched.nuc");
    let max_halo = acfg
        .halo_widths
        .values()
        .flat_map(|m| m.values().copied())
        .max()
        .unwrap_or(0);
    assert_eq!(
        max_halo, 0,
        "elementwise-add reads only `a[i]`/`b[i]` (offset 0); max halo width must be 0; got map {:?}",
        acfg.halo_widths
    );

    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert_eq!(
        sidecar.halo_widths, acfg.halo_widths,
        "NameSidecar.halo_widths must mirror ACFG.halo_widths"
    );
}

#[cfg(feature = "serde")]
#[test]
fn halo_widths_serde_roundtrip() {
    // Round-trip the NameSidecar through serde JSON; the halo_widths
    // map must survive byte-for-byte.
    let (linked, acfg) = lower("05-stencil", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert!(
        !sidecar.halo_widths.is_empty(),
        "fixture must produce non-trivial halo for the round-trip test"
    );
    let json = serde_json::to_string(&sidecar).expect("serialise NameSidecar");
    let back: nucleus_compiler::NameSidecar =
        serde_json::from_str(&json).expect("deserialise NameSidecar");
    assert_eq!(
        back.halo_widths, sidecar.halo_widths,
        "halo_widths must survive serde JSON round-trip"
    );
}

#[cfg(feature = "serde")]
#[test]
fn halo_widths_serde_default_on_missing_field() {
    // An "old" wire payload that omits the `halo_widths` field must
    // deserialise to an empty map (TASK-0260 backward-compat contract,
    // mirroring TASK-0233's transfer_buffer_for_seq).
    //
    // We synthesise the "old" payload by round-tripping a real
    // NameSidecar through JSON and then stripping the `halo_widths`
    // key from the JSON object string. The result still has every
    // OTHER field, so non-serde-default fields like `data_types`
    // remain present and the test isolates the additive-compat claim
    // to the new field alone.
    let (linked, acfg) = lower("01-elementwise-add", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let value: serde_json::Value =
        serde_json::to_value(&sidecar).expect("serialise NameSidecar to Value");
    let mut obj = value
        .as_object()
        .expect("NameSidecar serialises to JSON object")
        .clone();
    obj.remove("halo_widths");
    let pruned = serde_json::Value::Object(obj);
    let stripped_json = serde_json::to_string(&pruned).expect("re-serialise");
    let back: nucleus_compiler::NameSidecar =
        serde_json::from_str(&stripped_json).expect("payload without halo_widths must deserialise");
    assert!(
        back.halo_widths.is_empty(),
        "missing halo_widths field must default to empty map"
    );
}
