//! TASK-0422.01 (cycle-243): PRD §8.3 invariant (2) — Push/Wait events
//! form matched pairs — holds on the **post-mediation** per-worker
//! EventList for the mp-tcp-{bufsync,event,poll} and mp-uds-event
//! backends, across the entire example corpus.
//!
//! ## Why this test exists (and why it lives in the driver crate)
//!
//! TASK-0428 (tests/petri_to_events.rs::task0428_inv2_holds_for_entire_
//! example_corpus) proved inv(2) on the **backend-agnostic** pre-
//! `acfg_to_events` chain (build_acfg -> block_transforms ->
//! partition_{workers,rows,blocks2d} -> halo -> reuse -> inject_syncs ->
//! inject_transfers). That chain is what the pthreads-{sync,async} and
//! openmp-rs backends consume directly.
//!
//! BUT the four message-passing backends listed above run TWO MORE ACFG
//! passes AFTER `inject_transfers`, re-routing Push/Wait through an
//! elected host (`apply_host_mediation_inject` then, for event/uds only,
//! `apply_host_data_relay_inject`). inv(2) over THAT post-mediation
//! EventList was the live hard-blocker for TASK-0422 (wiring
//! `validate_event_lists` as a production gate). This test discharges it.
//!
//! It lives in `driver/tests/` deliberately: the driver is the single
//! source of truth for the per-backend pass ordering + host election
//! (src/main.rs ~464-553), and is the only crate that depends on BOTH
//! `nucleus-compiler` (the passes) AND `backend-common` (the host-
//! election helper). `nucleus-compiler/tests/` cannot host it without a
//! dev-dependency cycle (backend-common depends on nucleus-compiler).
//!
//! ## Faithfulness to the driver (anti-skew discipline)
//!
//! Host election uses the SHARED `backend_common::elect_host_from_name_
//! workers` helper — the exact call the driver makes — NOT a hand-
//! replicated rule (memory `feedback-driver-must-mirror-backend-election-
//! exactly`: replicating the election rule in a second site is the
//! latent-skew anti-pattern this project already retired by lifting the
//! rule to a shared helper).
//!
//! The per-backend pass set is mirrored EXACTLY, asymmetry included:
//!   - bufsync, poll        : mediation ONLY (no data-relay)
//!   - mp-tcp-event, mp-uds-event : mediation THEN data-relay
//!
//! ## Honest scope / limits
//!
//! - The corpus discovery + `schedule for "..."` algo resolution is a
//!   verbatim copy of the TASK-0428 sweep (several schedules pair with a
//!   sibling `prog.<variant>.algo.nuc`, not the default).
//! - `apply_host_data_relay_inject` only rewrites Push/Wait pairs whose
//!   BOTH endpoints are non-host; for many schedules it is a no-op. That
//!   is expected — this test asserts inv(2) holds on whatever the pass
//!   produces, not that the pass changed anything.
//! - This validates the COMPILE-time EventList contract only; runtime
//!   behaviour is covered by the e2e bit-identical differential.

use std::collections::BTreeSet;

use backend_common::elect_host_from_name_workers;
use nucleus_compiler::acfg::build_acfg;
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::link;
use nucleus_compiler::passes::petri_to_events::acfg_to_events;
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::{
    apply_block_transforms, apply_halo_inference_partition_aware, apply_host_data_relay_inject,
    apply_host_mediation_inject, apply_partition_blocks2d, apply_partition_rows,
    apply_partition_workers, apply_reuse_inference, validate_event_lists, WorkerId, ACFG,
};

/// The four message-passing backends that run host_mediation_inject
/// (and, for the two `*-event` ones, host_data_relay_inject) AFTER
/// `inject_transfers`. The `bool` is `applies_data_relay`, mirroring the
/// driver's `backend == "mp-tcp-event" || backend == "mp-uds-event"`
/// gate (src/main.rs:531). bufsync/poll = mediation ONLY.
const MEDIATED_BACKENDS: &[(&str, bool)] = &[
    ("mp-tcp-bufsync", false),
    ("mp-tcp-poll", false),
    ("mp-tcp-event", true),
    ("mp-uds-event", true),
];

/// Re-derive `used` (WorkerIds with a non-empty projected EventList) and
/// elect host via the SHARED helper — the exact sequence the driver runs
/// before each mediation pass (src/main.rs:484-493 and :537-546).
fn elect_host(acfg: &ACFG) -> Option<WorkerId> {
    let preview = acfg_to_events(acfg);
    let used: BTreeSet<WorkerId> = preview
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    elect_host_from_name_workers(&acfg.name_workers, &used)
}

/// Apply the post-`inject_transfers` mediation passes for `backend`,
/// mirroring driver/src/main.rs ~464-553 EXACTLY:
///   1. {all four}: elect -> apply_host_mediation_inject (None => pass through)
///   2. {*-event only}: re-elect on post-mediation ACFG ->
///      apply_host_data_relay_inject (None => pass through)
fn apply_mediation(backend_applies_data_relay: bool, acfg: ACFG) -> ACFG {
    // Step 1 — host_mediation_inject (all four mediated backends).
    let acfg = match elect_host(&acfg) {
        Some(h) => apply_host_mediation_inject(acfg, h),
        None => acfg, // degenerate ACFG (every per-worker list empty); pass through.
    };
    // Step 2 — host_data_relay_inject (mp-tcp-event / mp-uds-event ONLY).
    if backend_applies_data_relay {
        // Re-project on the POST-mediation ACFG before re-electing —
        // mediation may have added Sync events to host's list; the
        // driver re-projects here too (src/main.rs:537).
        match elect_host(&acfg) {
            Some(h) => apply_host_data_relay_inject(acfg, h),
            None => acfg,
        }
    } else {
        acfg
    }
}

#[test]
fn task0422_01_inv2_holds_post_mediation_for_mp_backends() {
    // CARGO_MANIFEST_DIR = <repo>/nucleus/driver. The example corpus
    // lives at <repo>/nuc-nucleus/examples (the cargo workspace root is
    // <repo>/nucleus, NOT the git repo root), so climb TWO levels:
    // driver -> nucleus -> <repo>. (The TASK-0428 sweep climbs two from
    // nucleus-compiler/tests for the same reason — both crates are
    // direct children of <repo>/nucleus.)
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let examples = repo_root.join("nuc-nucleus").join("examples");
    assert!(
        examples.is_dir(),
        "example corpus not found at {} — did the layout move?",
        examples.display()
    );

    // Discover (algo, sched) pairs deterministically (verbatim from the
    // TASK-0428 sweep so the corpus set stays identical).
    let mut exdirs: Vec<_> = std::fs::read_dir(&examples)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    exdirs.sort();

    let mut violations: Vec<String> = Vec::new();
    let mut ok = 0usize;
    let mut errd = 0usize;

    for d in exdirs {
        let algo_path = d.join("prog.algo.nuc");
        if !algo_path.exists() {
            continue;
        }
        let schdir = d.join("schedules");
        if !schdir.is_dir() {
            continue;
        }
        let mut scheds: Vec<_> = std::fs::read_dir(&schdir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "nuc").unwrap_or(false))
            .collect();
        scheds.sort();

        for sp in scheds {
            let sched_label = format!(
                "{}/{}",
                d.file_name().unwrap().to_string_lossy(),
                sp.file_name().unwrap().to_string_lossy()
            );
            let sched_src = std::fs::read_to_string(&sp).unwrap();
            // Resolve the algorithm from the `schedule for "..."`
            // directive (relative to the schedule dir), mirroring the
            // real harness — several schedules pair with a sibling
            // prog.<variant>.algo.nuc, NOT the default prog.algo.nuc.
            let resolved_algo = sched_src
                .lines()
                .find_map(|l| {
                    let l = l.trim();
                    let i = l.find("schedule for \"")?;
                    let rest = &l[i + "schedule for \"".len()..];
                    let j = rest.find('"')?;
                    Some(schdir.join(&rest[..j]))
                })
                .unwrap_or_else(|| algo_path.clone());
            let algo_src = std::fs::read_to_string(&resolved_algo)
                .unwrap_or_else(|_| std::fs::read_to_string(&algo_path).unwrap());

            // Build the backend-AGNOSTIC ACFG once (everything up to and
            // including inject_transfers). ANY error here = an EARLIER
            // pass rejected this (algo, sched) pair, so there is no
            // EventList to mediate/validate — counted as `errd`, NOT an
            // inv(2) violation. The TASK-0428 sweep already asserts the
            // current corpus has zero such cases; we re-assert below so a
            // future silently-failing schedule cannot hide here.
            let base_acfg = (|| -> Result<ACFG, String> {
                let algo = lower_algo(&parse_algo(&algo_src).map_err(|e| format!("{e:?}"))?)
                    .map_err(|e| format!("{e:?}"))?;
                let sched = lower_sched(&parse_sched(&sched_src).map_err(|e| format!("{e:?}"))?)
                    .map_err(|e| format!("{e:?}"))?;
                let linked = link::link(algo, sched).map_err(|e| format!("{e:?}"))?;
                let acfg = build_acfg(&linked).map_err(|e| format!("{e:?}"))?;
                let acfg = apply_block_transforms(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = apply_partition_workers(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = apply_partition_rows(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg =
                    apply_partition_blocks2d(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let (acfg, _adv) = apply_halo_inference_partition_aware(&linked, acfg)
                    .map_err(|e| format!("{e:?}"))?;
                let acfg = apply_reuse_inference(&linked, acfg).map_err(|e| format!("{e:?}"))?;
                let acfg = inject_syncs(acfg).map_err(|e| format!("{e:?}"))?;
                inject_transfers(&linked, acfg).map_err(|e| format!("{e:?}"))
            })();

            let base_acfg = match base_acfg {
                Ok(a) => a,
                Err(e) => {
                    errd += 1;
                    eprintln!("[TASK-0422.01 pipeline ERR] {sched_label}: {e}");
                    continue;
                }
            };

            // Per mp-* backend: apply that backend's post-injection
            // mediation passes, project, and validate inv(2) (full
            // surface). Each cell is independent (clone the base ACFG).
            for &(backend, applies_data_relay) in MEDIATED_BACKENDS {
                let mediated = apply_mediation(applies_data_relay, base_acfg.clone());
                let events = acfg_to_events(&mediated);
                match validate_event_lists(&events) {
                    Ok(()) => ok += 1,
                    Err(errs) => {
                        violations.push(format!("{backend} :: {sched_label}: {errs:?}"))
                    }
                }
            }
        }
    }

    eprintln!(
        "[TASK-0422.01] validated {ok} post-mediation (backend, schedule) cells \
         ({} mp-* backends), {errd} pipeline errors, {} inv(2) violations",
        MEDIATED_BACKENDS.len(),
        violations.len()
    );

    // Hard regression pin: every (mp-* backend, shipping schedule) cell's
    // POST-mediation EventList satisfies inv(2). A violation is a REAL
    // finding (mediation-pass bug or genuine inv(2)-reshape need) — this
    // assertion is deliberately NOT relaxed (TASK-0422.01 no-relax rule).
    assert!(
        violations.is_empty(),
        "TASK-0422.01: PRD §8.3 inv(2) (matched Push/Wait pairs) violated on the \
         POST-mediation EventList for {} (backend, schedule) cell(s):\n{}",
        violations.len(),
        violations.join("\n")
    );
    // No (algo, sched) pair silently failed to lower (would mask the
    // question for that schedule). Matches the TASK-0428 errd==0 pin.
    assert_eq!(
        errd, 0,
        "TASK-0422.01: {errd} schedule(s) failed to lower through the pre-mediation \
         chain; the post-mediation inv(2) sweep cannot validate them — resolve the \
         algo/schedule pairing or file the regression before relying on this pin"
    );
    // 55+ schedules × 4 mediated backends = 220+ validated cells. The
    // 55 lower bound mirrors the TASK-0428 corpus floor; ×4 because each
    // schedule is mediated and validated once per mp-* backend.
    assert!(
        ok >= 55 * MEDIATED_BACKENDS.len(),
        "TASK-0422.01: expected >= {} post-mediation cells validated \
         (>=55 schedules × {} mp-* backends), got {ok}; did a schedule directory \
         move or a `schedule for` path stop resolving?",
        55 * MEDIATED_BACKENDS.len(),
        MEDIATED_BACKENDS.len()
    );
}

/// Build the backend-agnostic ACFG (through `inject_transfers`) for one
/// explicit (algo, sched) pair under the example corpus, panicking on any
/// pipeline error. Used by the non-vacuity pin below.
fn build_base_acfg(algo_rel: &str, sched_rel: &str) -> ACFG {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let examples = repo_root.join("nuc-nucleus").join("examples");
    let algo_src = std::fs::read_to_string(examples.join(algo_rel)).expect("read algo");
    let sched_src = std::fs::read_to_string(examples.join(sched_rel)).expect("read sched");
    let algo = lower_algo(&parse_algo(&algo_src).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(&sched_src).expect("sched parse")).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block_transforms");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");
    let acfg = apply_partition_rows(&linked, acfg).expect("partition_rows");
    let acfg = apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d");
    let (acfg, _adv) =
        apply_halo_inference_partition_aware(&linked, acfg).expect("halo");
    let acfg = apply_reuse_inference(&linked, acfg).expect("reuse");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    inject_transfers(&linked, acfg).expect("inject_transfers")
}

/// NON-VACUITY pin. The broad sweep above would still pass if the
/// mediation passes were no-ops everywhere (it would then be equivalent
/// to the pre-mediation TASK-0428 sweep, proving nothing NEW). This test
/// proves the post-mediation projection is GENUINELY DIFFERENT from the
/// pre-mediation one for at least one shipping schedule, so the sweep's
/// inv(2) guarantee is over a distinct artefact.
///
/// 09-producer-consumer/pipelined is the documented case (driver
/// src/main.rs:513-520): a per-iteration worker-to-worker Push inside
/// `for n in 0..16` that `host_data_relay_inject` re-routes through host.
/// Under mp-tcp-event (mediation + data-relay) the projected EventList
/// MUST differ from the pre-mediation projection.
#[test]
fn task0422_01_mediation_is_non_vacuous_for_pipelined() {
    let base = build_base_acfg(
        "09-producer-consumer/prog.algo.nuc",
        "09-producer-consumer/schedules/pipelined.sched.nuc",
    );
    let pre = acfg_to_events(&base);
    // mp-tcp-event applies mediation THEN data-relay.
    let mediated = apply_mediation(true, base.clone());
    let post = acfg_to_events(&mediated);
    assert_ne!(
        pre, post,
        "TASK-0422.01 non-vacuity: 09-producer-consumer/pipelined under mp-tcp-event \
         (mediation + data-relay) produced an IDENTICAL EventList to the pre-mediation \
         projection — either the mediation passes silently no-op'd (then the broad sweep \
         proves nothing beyond TASK-0428) or this schedule stopped exercising the \
         documented w2w-Push-in-Repeat relay path. Investigate before trusting the sweep."
    );
    // And the post-mediation result is still inv(2)-clean (redundant with
    // the sweep, but pins the specific reshaped artefact by name).
    assert_eq!(
        validate_event_lists(&post),
        Ok(()),
        "TASK-0422.01: post-mediation EventList for 09-producer-consumer/pipelined \
         (mp-tcp-event) must satisfy PRD §8.3 inv(2)"
    );
}
