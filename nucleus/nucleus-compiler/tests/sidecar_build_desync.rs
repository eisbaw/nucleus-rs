//! TASK-0459: `build_sidecar`'s name<->id desync guards must fail LOUD,
//! never silently drop a symbol.
//!
//! `build_sidecar` resolves several name-keyed source-walk results into
//! the canonical id space (`acfg.name_data` / `name_kernels` /
//! `name_iter_vars`). For a link-valid IR every such lookup is total —
//! a miss is a compiler-internal table desync, not user input. The
//! sibling guards `(a)` `data_types`, `(d)` `kernel_sigs`, and
//! `(j)` `data_decl_order` all `unwrap_or_else(panic!)` on a miss.
//!
//! Block `(k)` `cumulative_data` previously `filter_map`ed the miss away
//! SILENTLY. A dropped cumulative symbol would skip the
//! COPY-not-accumulate exclusion — the xN-double-count protection that is
//! value-correctness-load-bearing (see the 16-jacobi cumulative-array
//! discriminator: whole-array *accumulate* was xN-wrong for cumulative
//! cross-iteration state until the discriminator landed). This test pins
//! that `(k)` now fails loud IDENTICALLY in kind to its siblings.
//!
//! Strategy: build a real, link-valid `linked` + `acfg` for 16-jacobi
//! (whose `field` symbol IS classified cumulative), then DELIBERATELY
//! desync the id table by removing `field` from `acfg.name_data` and
//! assert `build_sidecar` panics with the cumulative-desync context —
//! rather than silently producing a sidecar whose `cumulative_data` set
//! is missing `field`'s DataId.

use nucleus_compiler::acfg::{build_acfg, ACFG};
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::link::{self, LinkedIR};
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::sidecar::build_sidecar;

/// Read an example file under `nuc-nucleus/examples/`. Mirrors the
/// `read_example` helper in `tests/petri_to_events.rs` (kept local so the
/// two test files stay independent).
fn read_example(relpath: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let full = repo_root.join("nuc-nucleus").join("examples").join(relpath);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", full.display(), e))
}

/// The same parse -> lower -> link -> build_acfg -> inject pipeline
/// `full_pipeline_with_linked` runs in `tests/petri_to_events.rs`,
/// reproduced locally so this file owns its inputs.
fn full_pipeline(algo_rel: &str, sched_rel: &str) -> (LinkedIR, ACFG) {
    let algo =
        lower_algo(&parse_algo(&read_example(algo_rel)).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(&read_example(sched_rel)).expect("sched parse"))
        .expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    (linked, acfg)
}

const JACOBI_ALGO: &str = "16-jacobi/prog.algo.nuc";
const JACOBI_SCHED: &str = "16-jacobi/schedules/naive.sched.nuc";

/// Positive control: on a consistent (un-desynced) ACFG, 16-jacobi's
/// `field` IS classified cumulative and build_sidecar succeeds — so the
/// desync test below is exercising the genuine cumulative path, not an
/// already-empty set. If `field` ever stops being cumulative, this fires
/// FIRST and tells us the desync test has gone vacuous.
#[test]
fn jacobi_field_is_cumulative_and_carried_in_clean_build() {
    let (linked, acfg) = full_pipeline(JACOBI_ALGO, JACOBI_SCHED);
    let field_id = *acfg
        .name_data
        .get("field")
        .expect("16-jacobi declares `field`");
    let sidecar = build_sidecar(&linked, &acfg).expect("clean 16-jacobi sidecar builds");
    assert!(
        sidecar.cumulative_data.contains(&field_id),
        "16-jacobi's cross-iteration `field` must be carried in the clean \
         sidecar's cumulative_data set; got {:?}",
        sidecar.cumulative_data
    );
}

/// The TASK-0459 pin: a name<->id desync on the cumulative path must
/// PANIC with context (matching the `data_decl_order` / `data_types` /
/// `kernel_sigs` siblings), NOT silently drop `field` from
/// cumulative_data. We construct the desync by removing `field` from
/// `acfg.name_data` while `linked.algo.stmts` still classifies it as
/// cumulative.
#[test]
#[should_panic(expected = "cumulative<->id table desync")]
fn cumulative_data_name_id_desync_panics_loud() {
    let (linked, mut acfg) = full_pipeline(JACOBI_ALGO, JACOBI_SCHED);

    // Sanity: `field` is present before we break the table. (If this
    // ever fails, the example/grammar changed and the desync we build
    // below would be vacuous — fail honestly here rather than emit a
    // green-but-meaningless test.)
    assert!(
        acfg.name_data.remove("field").is_some(),
        "16-jacobi must declare `field` in name_data for this desync to be real"
    );

    // `field` is still cumulative per linked.algo.stmts, but absent from
    // the id table -> internal desync. Must fail loud, not drop.
    let _ = build_sidecar(&linked, &acfg);
}
