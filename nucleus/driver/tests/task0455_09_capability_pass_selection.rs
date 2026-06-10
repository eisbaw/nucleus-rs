//! TASK-0455.09 — capability-driven pass selection equivalence.
//!
//! The driver used to decide WHICH host-mediation compiler passes run by
//! string-matching the backend NAME in three hard-coded lists
//! (`driver/src/main.rs`):
//!
//!   - host-mediation (`apply_host_mediation_inject`):
//!     `mp-tcp-bufsync | mp-tcp-event | mp-tcp-poll | mp-uds-event`
//!   - host-data-relay (`apply_host_data_relay_inject`):
//!     `mp-tcp-event | mp-uds-event`
//!   - safe-push-reorder (`apply_safe_push_reorder`):
//!     `mp-tcp-event | mp-uds-event`
//!
//! TASK-0455.09 deleted those lists and now reads the selection off the
//! backend's `capabilities.toml` (the `star_topology_host_mediation` /
//! `host_data_relay` / `reorderable_push` flags). The RISK is that the
//! capability-driven selection silently diverges from the old name-list
//! selection for some backend — feeding the wrong pass set into codegen.
//!
//! This test is the load-bearing PROOF that it does NOT diverge. It is
//! **exhaustive over all 10 in-tree backends**: for each, it loads the
//! REAL on-disk `capabilities.toml` (the exact file the driver loads) and
//! asserts the three-flag tuple equals the tuple the OLD name lists would
//! have produced. The old lists are reproduced here verbatim as the
//! oracle; the new selection is the loaded capabilities. A mismatch is a
//! hard failure naming the backend and the offending flag.
//!
//! It lives in `driver/tests/` because the driver is the single source of
//! truth for the per-backend pass ordering, and is the crate that owns
//! both the deleted name lists and the capability load.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use backend_common::elect_host_from_name_workers;
use nucleus_compiler::capabilities::{
    load_capabilities, CapError, Capabilities, NotifyMode, Transport,
};
use nucleus_compiler::passes::petri_to_events::acfg_to_events;
use nucleus_compiler::test_support::build_pre_mediation_acfg;
use nucleus_compiler::{apply_host_mediation_inject, WorkerId, ACFG};

/// The 10 in-tree backends. Kept as an explicit list (not a directory
/// scan) so an accidentally-deleted `capabilities.toml` shows up as a
/// load failure on a KNOWN backend, not as a silently-shorter sweep.
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

/// The three pass-selection booleans, in the order the driver applies the
/// passes: (host-mediation, host-data-relay, safe-push-reorder).
type Selection = (bool, bool, bool);

/// OLD selection — the three hard-coded backend-NAME lists that lived in
/// `driver/src/main.rs` before TASK-0455.09, reproduced VERBATIM as the
/// equivalence oracle. If the driver's old lists are ever needed again
/// for archaeology, this is the canonical record of what they selected.
fn old_name_list_selection(backend: &str) -> Selection {
    // Host-mediation gate (cycles 160 / 195 / 197): bufsync, then
    // mp-tcp-poll, then mp-uds-event were appended over time.
    let host_mediation = matches!(
        backend,
        "mp-tcp-bufsync" | "mp-tcp-event" | "mp-tcp-poll" | "mp-uds-event"
    );
    // Host-data-relay gate (cycle 163 / 197): the two *-event backends.
    let host_data_relay = matches!(backend, "mp-tcp-event" | "mp-uds-event");
    // Safe-push-reorder gate (cycle 162 / 197): the two *-event backends.
    let safe_push_reorder = matches!(backend, "mp-tcp-event" | "mp-uds-event");
    (host_mediation, host_data_relay, safe_push_reorder)
}

/// NEW selection — read off the loaded `Capabilities` exactly as the
/// driver does after TASK-0455.09.
fn capability_driven_selection(caps: &Capabilities) -> Selection {
    (
        caps.star_topology_host_mediation,
        caps.host_data_relay,
        caps.reorderable_push,
    )
}

/// Resolve `<repo>/nucleus/backends/<backend>/capabilities.toml`.
/// `CARGO_MANIFEST_DIR` = `<repo>/nucleus/driver`; the backends live one
/// level up under `nucleus/backends` (the cargo workspace root is
/// `<repo>/nucleus`).
fn caps_path(backend: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .expect("driver dir has a parent (the cargo workspace root)")
        .join("backends")
        .join(backend)
        .join("capabilities.toml")
}

/// EXHAUSTIVE equivalence: for every in-tree backend, the
/// capability-driven pass selection equals the old name-list selection.
/// This is the AC#2/AC#4 load-bearing proof — it is what lets the three
/// name lists be deleted without an e2e regression.
#[test]
fn capability_selection_matches_old_name_lists_for_all_backends() {
    let mut mismatches: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for &backend in ALL_BACKENDS {
        let path = caps_path(backend);
        let caps = match load_capabilities(&path) {
            Ok(c) => c,
            Err(e) => {
                mismatches.push(format!(
                    "{backend}: capabilities.toml at {} failed to load: {e}",
                    path.display()
                ));
                continue;
            }
        };
        // The loaded `name` field must match the directory name — a
        // copy-paste slip would otherwise mediate the wrong backend.
        assert_eq!(
            caps.name, backend,
            "{backend}: capabilities.toml `name` field is `{}`, expected `{backend}`",
            caps.name
        );

        let old = old_name_list_selection(backend);
        let new = capability_driven_selection(&caps);
        if old != new {
            mismatches.push(format!(
                "{backend}: capability-driven selection {new:?} != old name-list \
                 selection {old:?} \
                 (order: host_mediation, host_data_relay, safe_push_reorder)"
            ));
        }
        checked += 1;
    }

    assert!(
        mismatches.is_empty(),
        "TASK-0455.09: capability-driven pass selection diverges from the deleted \
         name-list selection for {} backend(s):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    // Exhaustiveness pin: every named backend was actually checked. If a
    // capabilities.toml went missing, the load-failure arm above recorded
    // it as a mismatch (so we'd already have failed) — this guards the
    // weaker case where the list itself was edited down.
    assert_eq!(
        checked,
        ALL_BACKENDS.len(),
        "TASK-0455.09: expected to check all {} backends, checked {checked}",
        ALL_BACKENDS.len()
    );
}

/// Spot-pin the three representative selection shapes by name so a future
/// reader sees the intended tuples without re-deriving them from the
/// matrix. (Redundant with the exhaustive sweep, deliberately.)
#[test]
fn representative_backend_selection_shapes() {
    // Native-barrier / direct-w2w backend: no mediation passes at all.
    assert_eq!(
        capability_driven_selection(&load_capabilities(&caps_path("pthreads-sync")).unwrap()),
        (false, false, false),
        "pthreads-sync must select NO host-mediation passes"
    );
    // Strict-FIFO star backend: mediation ONLY.
    assert_eq!(
        capability_driven_selection(&load_capabilities(&caps_path("mp-tcp-bufsync")).unwrap()),
        (true, false, false),
        "mp-tcp-bufsync must select host-mediation only (no relay, no reorder)"
    );
    // Per-seq-demux event star backend: all three.
    assert_eq!(
        capability_driven_selection(&load_capabilities(&caps_path("mp-tcp-event")).unwrap()),
        (true, true, true),
        "mp-tcp-event must select all three host-mediation passes"
    );
}

// --------------------------------------------------------------------
// Negative arm: a logically impossible flag set is rejected LOUDLY by the
// validator (TASK-0455.09 AC#4). We construct `Capabilities` in-memory and
// call the public `validate()` — the EXACT validation `load_capabilities`
// runs after parsing — so the test is hermetic (no TOML round-trip, no
// filesystem, no extra dev-dependency).
// --------------------------------------------------------------------

/// A valid baseline `Capabilities` (mp-tcp-bufsync-shaped) with the three
/// topology flags substituted in. Used to construct both the positive and
/// the contradictory cases below.
fn caps_with_flags(star: bool, relay: bool, reorder: bool) -> Capabilities {
    Capabilities {
        schema_version: 1,
        name: "synthetic-test".to_string(),
        tier: 1,
        transport: Transport::Tcp,
        notify: vec![NotifyMode::Barrier, NotifyMode::Blocking],
        supports_async: false,
        supports_buffer: false,
        max_buffer: 1,
        worker_classes: vec!["default".to_string()],
        memory_regions: vec!["heap".to_string()],
        star_topology_host_mediation: star,
        host_data_relay: relay,
        reorderable_push: reorder,
    }
}

#[test]
fn data_relay_without_mediation_is_rejected() {
    // host_data_relay = true but star_topology_host_mediation = false:
    // a transfer cannot be relayed through a non-mediating host.
    let err = caps_with_flags(false, true, false)
        .validate()
        .expect_err("contradictory flags (relay without mediation) must be rejected");
    match err {
        CapError::InconsistentTopologyFlags { detail } => {
            assert!(
                detail.contains("host_data_relay")
                    && detail.contains("star_topology_host_mediation"),
                "error detail should name both offending flags, got: {detail}"
            );
        }
        other => panic!("expected InconsistentTopologyFlags, got {other:?}"),
    }
}

#[test]
fn reorderable_push_without_mediation_is_rejected() {
    // reorderable_push = true but star_topology_host_mediation = false:
    // the safe-push reorder only applies on the host-relay path.
    let err = caps_with_flags(false, false, true)
        .validate()
        .expect_err("contradictory flags (reorder without mediation) must be rejected");
    match err {
        CapError::InconsistentTopologyFlags { detail } => {
            assert!(
                detail.contains("reorderable_push")
                    && detail.contains("star_topology_host_mediation"),
                "error detail should name both offending flags, got: {detail}"
            );
        }
        other => panic!("expected InconsistentTopologyFlags, got {other:?}"),
    }
}

#[test]
fn consistent_flag_sets_validate() {
    // All four legal combinations along the implication chain validate:
    //   (false,false,false)  native-barrier / direct-w2w
    //   (true,false,false)   strict-FIFO star (bufsync/poll)
    //   (true,true,false)    relay without reorder (legal, even if no
    //                        shipped backend uses it today)
    //   (true,true,true)     per-seq-demux event star
    for (star, relay, reorder) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, true, true),
    ] {
        assert!(
            caps_with_flags(star, relay, reorder).validate().is_ok(),
            "consistent flag set ({star},{relay},{reorder}) must validate"
        );
    }
}

// --------------------------------------------------------------------
// Projection-collapse correctness (TASK-0455.09 AC#3).
//
// The OLD driver re-projected the ACFG and re-elected host TWICE: once
// before host-mediation, once before host-data-relay (on the
// POST-mediation ACFG). TASK-0455.09 collapses those to ONE projection +
// one election, reusing the SAME host `h` for both passes. That is only
// behaviour-preserving if host-mediation cannot change which host gets
// elected. This sweep PROVES it over the shipping corpus: for every
// (algo, sched) pair, the host elected from the pre-mediation projection
// equals the host elected from the post-mediation projection. A single
// counterexample would mean the collapse silently mediates the data-relay
// against the wrong host — exactly the cross-backend skew the standing
// `feedback-driver-must-mirror-backend-election-exactly` rule guards.
// --------------------------------------------------------------------

/// Elect host from `acfg`'s projection via the shared helper — the exact
/// sequence the driver runs before each mediation pass.
fn elect_host(acfg: &ACFG) -> Option<WorkerId> {
    let preview = acfg_to_events(acfg);
    let used: BTreeSet<WorkerId> = preview
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    elect_host_from_name_workers(&acfg.name_workers, &used)
}

#[test]
fn host_election_is_invariant_under_mediation_so_one_projection_suffices() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = Path::new(manifest_dir)
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

    let mut exdirs: Vec<_> = std::fs::read_dir(&examples)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    exdirs.sort();

    let mut checked = 0usize;
    let mut mediated_at_least_one_host = 0usize;
    let mut divergences: Vec<String> = Vec::new();

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
            let label = format!(
                "{}/{}",
                d.file_name().unwrap().to_string_lossy(),
                sp.file_name().unwrap().to_string_lossy()
            );
            let sched_src = std::fs::read_to_string(&sp).unwrap();
            // Resolve the algo from `schedule for "..."` (some schedules
            // pair with a sibling prog.<variant>.algo.nuc).
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

            let base = match build_pre_mediation_acfg(&algo_src, &sched_src) {
                Ok(a) => a,
                // An earlier pass rejected this pair: no ACFG to mediate.
                // Skip (the inv(2) sweep already pins errd==0 on the corpus).
                Err(_) => continue,
            };

            // Pre-mediation election = the host the NEW driver reuses.
            let pre_host = elect_host(&base);
            let Some(h) = pre_host else {
                // Degenerate ACFG (every list empty) — both old and new
                // pass through unchanged. Nothing to compare.
                continue;
            };
            // Apply mediation, then re-elect (the OLD driver's data-relay
            // host). Must equal `h`.
            let mediated = apply_host_mediation_inject(base, h);
            let post_host = elect_host(&mediated);
            if post_host != Some(h) {
                divergences.push(format!(
                    "{label}: pre-mediation host {h:?} != post-mediation host {post_host:?}"
                ));
            }
            mediated_at_least_one_host += 1;
            checked += 1;
        }
    }

    assert!(
        divergences.is_empty(),
        "TASK-0455.09 projection-collapse: host election DIVERGED after host-mediation \
         for {} (algo, sched) pair(s) — reusing the pre-mediation host for the data-relay \
         pass would mediate against the WRONG host:\n{}",
        divergences.len(),
        divergences.join("\n")
    );
    // Non-vacuity: the sweep actually elected a host for a meaningful
    // number of cells (not all degenerate / all rejected).
    assert!(
        checked >= 20 && mediated_at_least_one_host >= 20,
        "TASK-0455.09 projection-collapse sweep checked only {checked} cells \
         (expected >= 20) — did the corpus shrink or every pair start failing to lower?"
    );
}
