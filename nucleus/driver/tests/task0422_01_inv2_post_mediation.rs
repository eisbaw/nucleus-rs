//! TASK-0422.01 (cycle-243): PRD §8.3 invariant (2) — Push/Wait events
//! form matched pairs — holds on the **post-mediation** per-worker
//! EventList for every host-mediated backend (currently the
//! mp-tcp-{bufsync,event,poll} and mp-uds-event backends, but DERIVED
//! from the `star_topology_host_mediation` capability flag — see below,
//! TASK-0465), across the entire example corpus.
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
//! source of truth for the per-backend pass ordering + host election (the
//! capability-gated `apply_host_mediation_inject` / `apply_host_data_relay_
//! inject` chain in `main`), and is the only crate that depends on BOTH
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
//! Likewise, the SET of backends this sweep mediates is NOT a hand-
//! maintained name list. It is DERIVED at test time by loading every
//! in-tree `backends/<b>/capabilities.toml` via the production
//! `load_capabilities` loader and filtering on
//! `caps.star_topology_host_mediation` — the exact flag the driver reads
//! to decide whether to run `apply_host_mediation_inject`. The second
//! pass (`apply_host_data_relay_inject`) is gated by `caps.host_data_relay`
//! off the same loaded caps. So a new backend that declares
//! `star_topology_host_mediation = true` in its `capabilities.toml` is
//! AUTOMATICALLY covered by this sweep, with no edit here — closing the
//! silent-sibling gap a hard-coded `MEDIATED_BACKENDS` name list invited
//! (TASK-0465; the last surviving copy of those lists).
//!
//! The per-backend pass set is therefore mirrored EXACTLY, asymmetry
//! included, straight off the capability flags:
//!   - `host_data_relay = false` (e.g. bufsync, poll) : mediation ONLY
//!   - `host_data_relay = true`  (e.g. the *-event backends) : mediation
//!     THEN data-relay
//!
//! ### Why reading the same toml as production is still a sound oracle
//!
//! This sweep reads the SAME `capabilities.toml` the driver reads, so it
//! cannot independently catch a wrong FLAG VALUE (a backend mis-declaring
//! its topology would mis-select passes in both production and here
//! identically). That is fine and intentional: the FLAG-value oracle is
//! `task0455_09_capability_pass_selection.rs`, which pins the
//! capability-driven selection against the frozen historical name lists.
//! THIS test's oracle is orthogonal — it asserts that whatever set the
//! flags select, the post-mediation `validate_event_lists` actually RAN
//! per mediated backend AND held (inv(2)), plus the non-vacuity floor
//! below proves the mediation passes genuinely reshape the EventList for
//! at least one corpus cell. So it still catches a production REGRESSION
//! in the mediation passes themselves (a pass that breaks Push/Wait
//! pairing, or stops reshaping), independent of the flag values.
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
use std::path::{Path, PathBuf};

use backend_common::elect_host_from_name_workers;
use nucleus_compiler::passes::petri_to_events::acfg_to_events;
// The backend-agnostic pre-mediation pass chain (build_acfg through
// inject_transfers) is now the SHARED helper (TASK-0422.01.01); the
// individual passes are no longer named here. The post-mediation passes
// and the projection/validation surface stay local.
use nucleus_compiler::test_support::build_pre_mediation_acfg;
use nucleus_compiler::{
    apply_host_data_relay_inject, apply_host_mediation_inject, load_capabilities,
    validate_event_lists, Capabilities, WorkerId, ACFG,
};

/// Every in-tree backend, by directory name. Kept as an explicit list
/// (not a directory scan) so an accidentally-deleted `capabilities.toml`
/// surfaces as a load failure on a KNOWN backend rather than a silently-
/// shorter sweep — the same rationale as `ALL_BACKENDS` in the sibling
/// `task0455_09_capability_pass_selection.rs`. The MEDIATED subset is
/// DERIVED from these by `mediated_backends` below (filtering the loaded
/// `star_topology_host_mediation` flag); this list is only the universe
/// to scan, NOT the oracle for which backends mediate.
const ALL_BACKENDS: &[&str] = &[
    "pthreads-sync",
    "pthreads-async",
    "openmp-rs",
    "mpi-blocking",
    "mpi-nonblocking",
    "embedded-pattern",
    "mp-tcp-bufsync",
    "mp-tcp-poll",
    "mp-tcp-event",
    "mp-uds-event",
];

/// Resolve `<repo>/nucleus/backends/<backend>/capabilities.toml`.
/// `CARGO_MANIFEST_DIR` = `<repo>/nucleus/driver`; the backends live one
/// level up under `nucleus/backends` (the cargo workspace root is
/// `<repo>/nucleus`). Same resolution the sibling capability test uses.
fn caps_path(backend: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .expect("driver dir has a parent (the cargo workspace root)")
        .join("backends")
        .join(backend)
        .join("capabilities.toml")
}

/// Pure derivation of the MEDIATED subset from a set of loaded
/// `Capabilities`: a backend is mediated iff
/// `caps.star_topology_host_mediation` is set (the exact flag the driver
/// reads to run `apply_host_mediation_inject`), and the carried `bool` is
/// `caps.host_data_relay` (the flag the driver reads to additionally run
/// `apply_host_data_relay_inject`). Returns `(name, applies_data_relay)`
/// pairs sorted by name for deterministic iteration.
///
/// Factored out of the on-disk scan so the auto-coverage property — a NEW
/// backend declaring `star_topology_host_mediation = true` is included —
/// can be proved over a SYNTHETIC `Capabilities` (see
/// `derived_set_auto_covers_a_new_mediated_backend`) without touching the
/// real backend tree.
fn mediated_from_caps(all: &[Capabilities]) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = all
        .iter()
        .filter(|c| c.star_topology_host_mediation)
        .map(|c| (c.name.clone(), c.host_data_relay))
        .collect();
    out.sort();
    out
}

/// Load every `ALL_BACKENDS` capability matrix off disk via the
/// production `load_capabilities`, then derive the mediated subset. Panics
/// (fail-loud) if any backend's `capabilities.toml` is missing or invalid
/// — a vanished/garbled cap file must not silently shrink the sweep.
fn mediated_backends() -> Vec<(String, bool)> {
    let all: Vec<Capabilities> = ALL_BACKENDS
        .iter()
        .map(|&b| {
            let path = caps_path(b);
            let caps = load_capabilities(&path).unwrap_or_else(|e| {
                panic!(
                    "TASK-0465: capabilities.toml for backend `{b}` at {} failed to load: {e} \
                     — the mediated-backend oracle cannot be derived",
                    path.display()
                )
            });
            // A copy-paste slip in the `name` field would mediate under
            // the wrong identity; pin it to the directory name.
            assert_eq!(
                caps.name, b,
                "TASK-0465: capabilities.toml `name` is `{}`, expected `{b}`",
                caps.name
            );
            caps
        })
        .collect();
    mediated_from_caps(&all)
}

/// Re-derive `used` (WorkerIds with a non-empty projected EventList) and
/// elect host via the SHARED helper — the exact sequence the driver runs
/// before each mediation pass: project with `acfg_to_events`, take the
/// non-empty-list workers as `used`, then call
/// `elect_host_from_name_workers`. Mirrors the driver's
/// `caps.star_topology_host_mediation` arm.
fn elect_host(acfg: &ACFG) -> Option<WorkerId> {
    let preview = acfg_to_events(acfg);
    let used: BTreeSet<WorkerId> = preview
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    elect_host_from_name_workers(&acfg.name_workers, &used)
}

/// Step 1 of mediation — `apply_host_mediation_inject` (every backend
/// whose `caps.star_topology_host_mediation` is set). Elect host via the
/// shared helper, then apply (None => degenerate ACFG, pass through).
/// Mirrors the driver's `caps.star_topology_host_mediation` arm
/// (`apply_host_mediation_inject` in `main`).
fn apply_mediation_only(acfg: ACFG) -> ACFG {
    match elect_host(&acfg) {
        Some(h) => apply_host_mediation_inject(acfg, h),
        None => acfg, // degenerate ACFG (every per-worker list empty); pass through.
    }
}

/// Step 2 of mediation — `apply_host_data_relay_inject` (only backends
/// whose `caps.host_data_relay` is set; `validate` guarantees that
/// implies `star_topology_host_mediation`, so step 1 always ran first).
/// MUST be called on a POST-mediation ACFG: the driver re-projects +
/// re-elects on the post-mediation ACFG before this pass, because step 1
/// may have added Sync events to host's list. Mirrors that re-election
/// exactly. (In the driver, step 1 and step 2 share the SAME elected host
/// `h`; re-electing here is equivalent because mediation only ADDS events
/// to the already-elected host — see the driver's projection-collapse
/// rationale in the `caps.star_topology_host_mediation` arm.)
fn apply_data_relay(acfg: ACFG) -> ACFG {
    match elect_host(&acfg) {
        Some(h) => apply_host_data_relay_inject(acfg, h),
        None => acfg,
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

    // The mediated backends are DERIVED from the on-disk capability
    // matrices (filter `star_topology_host_mediation`), NOT a hard-coded
    // name list — so a new mediated backend is auto-covered (TASK-0465).
    let mediated = mediated_backends();
    // Non-vacuity floor: the corpus has at least one star-topology backend
    // declared. A zero-length set would make the entire per-backend inner
    // loop a no-op and silently pass without validating anything.
    assert!(
        !mediated.is_empty(),
        "TASK-0465: no backend declares `star_topology_host_mediation = true` in its \
         capabilities.toml — the post-mediation sweep would validate ZERO cells. Either \
         every mediated backend was removed (unlikely) or the capability load/derivation \
         broke; investigate before trusting a green run."
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
    // Corpus-wide non-vacuity counters (architect P2/P3, cycle-243): how
    // many cells each mediation pass GENUINELY reshapes (post-pass
    // projection differs from the input projection). Without these, the
    // whole sweep would still pass if the passes no-op'd everywhere —
    // making it equivalent to the pre-mediation TASK-0428 sweep and
    // proving nothing NEW. `mediation_reshaped` isolates
    // host_mediation_inject (the only pass bufsync/poll run);
    // `relay_reshaped` isolates host_data_relay_inject (event/uds only),
    // measured as the delta ON TOP OF mediation so it cannot be
    // attributed to step 1.
    let mut mediation_reshaped = 0usize;
    let mut relay_reshaped = 0usize;

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
            // The pre-mediation pass chain is the shared
            // `build_pre_mediation_acfg` helper (TASK-0422.01.01) — the
            // SAME chain the TASK-0428 sweep and the production driver
            // run. Consolidating it removes the silent-divergence risk
            // (a pass inserted before inject_transfers would otherwise
            // have to be mirrored across three near-verbatim copies).
            let base_acfg = build_pre_mediation_acfg(&algo_src, &sched_src);

            let base_acfg = match base_acfg {
                Ok(a) => a,
                Err(e) => {
                    errd += 1;
                    eprintln!("[TASK-0422.01 pipeline ERR] {sched_label}: {e}");
                    continue;
                }
            };

            // Pre-mediation projection (the TASK-0428 artefact) — the
            // baseline the non-vacuity counters measure reshape against.
            let pre_events = acfg_to_events(&base_acfg);

            // Per mediated backend (derived from `star_topology_host_
            // mediation`): apply that backend's post-injection mediation
            // passes STAGE BY STAGE (so each pass's reshape is attributed
            // independently), project, and validate inv(2) (full surface).
            // Each cell is independent (clone base ACFG).
            for (backend, applies_data_relay) in &mediated {
                let applies_data_relay = *applies_data_relay;
                // Step 1 — host_mediation_inject (every mediated backend).
                let med_only = apply_mediation_only(base_acfg.clone());
                let med_only_events = acfg_to_events(&med_only);
                if med_only_events != pre_events {
                    mediation_reshaped += 1;
                }
                // Step 2 — host_data_relay_inject (event/uds only). Its
                // reshape is the delta ON TOP OF mediation, so it cannot
                // be mis-attributed to step 1.
                let events = if applies_data_relay {
                    let full = apply_data_relay(med_only);
                    let full_events = acfg_to_events(&full);
                    if full_events != med_only_events {
                        relay_reshaped += 1;
                    }
                    full_events
                } else {
                    med_only_events
                };
                match validate_event_lists(&events) {
                    Ok(()) => ok += 1,
                    Err(errs) => violations.push(format!("{backend} :: {sched_label}: {errs:?}")),
                }
            }
        }
    }

    eprintln!(
        "[TASK-0422.01] validated {ok} post-mediation (backend, schedule) cells \
         ({} mediated backends), {errd} pipeline errors, {} inv(2) violations; \
         non-vacuity: mediation reshaped {mediation_reshaped} cell(s), \
         data-relay reshaped {relay_reshaped} cell(s)",
        mediated.len(),
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
    // 55+ schedules × N mediated backends validated cells. The 55 lower
    // bound mirrors the TASK-0428 corpus floor; ×N because each schedule
    // is mediated and validated once per mediated backend (N derived from
    // the capability flags, not hard-coded — TASK-0465).
    assert!(
        ok >= 55 * mediated.len(),
        "TASK-0422.01: expected >= {} post-mediation cells validated \
         (>=55 schedules × {} mediated backends), got {ok}; did a schedule directory \
         move or a `schedule for` path stop resolving?",
        55 * mediated.len(),
        mediated.len()
    );
    // NON-VACUITY (architect P3, cycle-243): host_mediation_inject — the
    // ONLY mediation pass bufsync/poll run — must reshape the EventList
    // for at least one corpus cell. If it reshapes zero, the bufsync/poll
    // arms of this sweep are byte-identical to the pre-mediation TASK-0428
    // sweep and prove nothing NEW for those two backends; that would
    // itself be a finding to investigate, not a green pass.
    assert!(
        mediation_reshaped > 0,
        "TASK-0422.01 non-vacuity: host_mediation_inject changed the projected \
         EventList for 0 corpus cells — the bufsync/poll arms of this sweep would be \
         equivalent to TASK-0428 (pre-mediation), so the post-mediation inv(2) claim \
         for those backends is vacuous. Investigate (did the pass stop reshaping, or \
         did the corpus lose every host-excluding-barrier schedule?)."
    );
    // NON-VACUITY (architect P2, cycle-243): host_data_relay_inject must
    // reshape the EventList for at least one cell MEASURED AS THE DELTA ON
    // TOP OF mediation (full_events != med_only_events) — so the reshape
    // is attributed to data-relay specifically, not conflated with step 1.
    assert!(
        relay_reshaped > 0,
        "TASK-0422.01 non-vacuity: host_data_relay_inject changed nothing beyond \
         host_mediation_inject for any corpus cell — the event/uds arms add no distinct \
         artefact over the mediation-only projection, so their post-mediation inv(2) \
         claim is not exercising the data-relay rewrite. Investigate (did the w2w-Push-\
         in-Repeat relay path stop triggering across the corpus?)."
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
    // Shared pre-mediation chain (TASK-0422.01.01). This caller's
    // panic-on-error contract is preserved by `.expect`ing the Result
    // (the chain MUST lower for the explicit, known-good pair below).
    build_pre_mediation_acfg(&algo_src, &sched_src).expect("pre-mediation pipeline")
}

/// NON-VACUITY pin. The broad sweep above would still pass if the
/// mediation passes were no-ops everywhere (it would then be equivalent
/// to the pre-mediation TASK-0428 sweep, proving nothing NEW). This test
/// proves the post-mediation projection is GENUINELY DIFFERENT from the
/// pre-mediation one for at least one shipping schedule, so the sweep's
/// inv(2) guarantee is over a distinct artefact.
///
/// 09-producer-consumer/pipelined is the documented case (see the driver's
/// `caps.host_data_relay` / `apply_host_data_relay_inject` comment block):
/// a per-iteration worker-to-worker Push inside `for n in 0..16` that
/// `host_data_relay_inject` re-routes through host.
///
/// The reshape is asserted as the delta of data-relay ON TOP OF mediation
/// (post-mediation-only vs post-mediation+relay) — NOT vs the pre-
/// mediation projection. That isolation matters (architect P2, cycle-243):
/// a `pre != full` assertion would still pass if data-relay silently
/// no-op'd while host_mediation_inject alone reshaped, which is exactly
/// the no-op risk this test guards. Comparing the mediation-only stage to
/// the full stage attributes the change to data-relay specifically.
#[test]
fn task0422_01_mediation_is_non_vacuous_for_pipelined() {
    let base = build_base_acfg(
        "09-producer-consumer/prog.algo.nuc",
        "09-producer-consumer/schedules/pipelined.sched.nuc",
    );
    // mp-tcp-event applies mediation THEN data-relay. Stage them so the
    // data-relay delta is isolated from the mediation delta.
    let med_only = apply_mediation_only(base.clone());
    let med_only_events = acfg_to_events(&med_only);
    let full = apply_data_relay(med_only);
    let full_events = acfg_to_events(&full);
    assert_ne!(
        med_only_events, full_events,
        "TASK-0422.01 non-vacuity: 09-producer-consumer/pipelined — host_data_relay_inject \
         produced an EventList IDENTICAL to the mediation-only projection, i.e. it added \
         nothing of its own. Either data-relay silently no-op'd (then the event/uds sweep \
         proves nothing the bufsync/poll sweep doesn't) or this schedule stopped \
         exercising the documented w2w-Push-in-Repeat relay path. Investigate."
    );
    // And the post-mediation result is still inv(2)-clean (redundant with
    // the sweep, but pins the specific reshaped artefact by name).
    assert_eq!(
        validate_event_lists(&full_events),
        Ok(()),
        "TASK-0422.01: post-mediation EventList for 09-producer-consumer/pipelined \
         (mp-tcp-event) must satisfy PRD §8.3 inv(2)"
    );
}

/// AC#3 floor (TASK-0465), independent of the sweep: the on-disk derived
/// mediated set is non-empty AND every entry's carried `applies_data_relay`
/// equals that backend's `host_data_relay` flag re-read from disk. This
/// re-derives the truth a second way (re-loading each named entry's caps)
/// so a bug in `mediated_from_caps` that dropped/duplicated entries or
/// mis-paired the relay bool is caught here, not only as a corpus-floor
/// miss in the big sweep.
#[test]
fn derived_mediated_set_is_nonempty_and_matches_toml_truth() {
    let mediated = mediated_backends();
    assert!(
        !mediated.is_empty(),
        "TASK-0465: derived mediated-backend set is empty — no in-tree backend declares \
         `star_topology_host_mediation = true`, or the capability load/derivation broke"
    );
    for (name, applies_data_relay) in &mediated {
        let caps =
            load_capabilities(&caps_path(name)).expect("a derived-mediated backend must re-load");
        assert!(
            caps.star_topology_host_mediation,
            "TASK-0465: `{name}` is in the mediated set but its capabilities.toml says \
             star_topology_host_mediation = false — derivation is wrong"
        );
        assert_eq!(
            *applies_data_relay, caps.host_data_relay,
            "TASK-0465: `{name}` carried applies_data_relay={applies_data_relay} but its \
             capabilities.toml host_data_relay={} — the relay-pass gate would diverge from \
             production",
            caps.host_data_relay
        );
    }
}

/// AC#1 (TASK-0465): a NEW backend that flips `star_topology_host_mediation
/// = true` in its `capabilities.toml` is AUTOMATICALLY covered by the
/// derived set, with NO edit to this test. Proven two complementary ways:
///
///  1. Through the PRODUCTION loader on a temp `capabilities.toml` written
///     under `CARGO_TARGET_TMPDIR` — exactly the `load_capabilities` path
///     the driver and `mediated_backends` use. This proves the toml schema
///     accepts the flag and the loader surfaces it.
///  2. Through the pure `mediated_from_caps` derivation over a synthetic
///     in-memory `Capabilities` set including the fictional backend — this
///     proves the FILTER includes it (and excludes a non-mediated peer).
///
/// Together: a future mediated backend needs only its capabilities.toml; it
/// is not silently skipped the way a hard-coded MEDIATED_BACKENDS const
/// (deleted in TASK-0465) would have skipped it.
#[test]
fn derived_set_auto_covers_a_new_mediated_backend() {
    // (1) Loader path: write a fictional mediated backend's toml and load
    // it through the SAME loader production uses.
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("task0465_new_backend-{}.toml", std::process::id()));
    std::fs::write(
        &tmp,
        r#"schema_version = 1
name = "fictional-star-backend"
tier = 1
transport = "tcp"
notify = ["barrier", "blocking"]
supports_async = false
supports_buffer = false
max_buffer = 1
worker_classes = ["default"]
memory_regions = ["heap"]
star_topology_host_mediation = true
host_data_relay = true
reorderable_push = true
"#,
    )
    .expect("write temp capabilities.toml");
    let loaded = load_capabilities(&tmp).expect("temp capabilities.toml must load + validate");
    assert!(
        loaded.star_topology_host_mediation && loaded.host_data_relay,
        "loader must surface the mediation flags from the temp toml"
    );
    let _ = std::fs::remove_file(&tmp);

    // (2) Pure-filter path: the loaded fictional backend, mixed with a
    // non-mediated peer, is included by `mediated_from_caps` (and the peer
    // is excluded), carrying its host_data_relay bit.
    let peer = Capabilities {
        schema_version: 1,
        name: "non-mediated-peer".to_string(),
        tier: 1,
        transport: nucleus_compiler::Transport::SharedMemory,
        notify: vec![nucleus_compiler::CapNotifyMode::Barrier],
        supports_async: false,
        supports_buffer: false,
        max_buffer: 1,
        worker_classes: vec!["default".to_string()],
        memory_regions: vec!["heap".to_string()],
        star_topology_host_mediation: false,
        host_data_relay: false,
        reorderable_push: false,
    };
    let derived = mediated_from_caps(&[peer, loaded]);
    assert_eq!(
        derived,
        vec![("fictional-star-backend".to_string(), true)],
        "TASK-0465: a new backend with star_topology_host_mediation=true must be \
         auto-included (carrying host_data_relay), and a non-mediated peer excluded — \
         this is the auto-coverage property that retires the deleted name list"
    );
}

/// Wave-4 review P2.4: same completeness pin as the sibling
/// `task0455_09_capability_pass_selection.rs` — `ALL_BACKENDS` must
/// equal the `backends/` directory scan, so a NEW backend directory
/// cannot be silently unswept while a deleted toml still fails loud on
/// a known name.
#[test]
fn all_backends_list_matches_backends_directory() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../backends");
    let mut scanned: Vec<String> = std::fs::read_dir(&dir)
        .expect("read nucleus/backends")
        .filter_map(|e| {
            let e = e.ok()?;
            let p = e.path();
            (p.is_dir() && p.join("capabilities.toml").is_file())
                .then(|| e.file_name().to_string_lossy().into_owned())
        })
        .collect();
    scanned.sort();
    let mut listed: Vec<String> = ALL_BACKENDS.iter().map(|s| s.to_string()).collect();
    listed.sort();
    assert_eq!(
        listed, scanned,
        "ALL_BACKENDS is out of sync with nucleus/backends/ — update the list"
    );
}
