//! A/B equivalence harness for the symbolic communicating-net gate
//! (TASK-0455.01 AC#1 — the load-bearing acceptance criterion).
//!
//! ## What it pins
//!
//! For every corpus net (PRE-mediation AND each POST-mediation variant a
//! real backend would apply) plus the synthetic negatives, the COMBINED
//! verdict the driver computes —
//!
//! ```text
//!   match analyze_net_soundness_symbolic(acfg) {
//!     ProvenSound       => Ok,                       // skip expanded gate
//!     NeedsExpansion(_) => check_net_sound(net),     // fall back
//!   }
//! ```
//!
//! — must equal the EXPANDED gate's verdict `check_net_sound(net).is_ok()`
//! run unconditionally. That is the soundness-equivalence the whole
//! symbolic fast path rests on: it may change *when* the expanded analysis
//! runs, never *whether* an unsound net is caught (thesis `sec:fw-quant`
//! / P4 guardrail). The arms are:
//!
//! 1. **Corpus arm (pre-mediation)** — every
//!    `examples/*/schedules/*.sched.nuc` that reaches the soundness gate
//!    (lowered through the FULL pre-mediation pass chain the driver runs).
//!    Enumerated from the filesystem so a new example/schedule is covered
//!    automatically, with no hand-maintained list to go stale. Both
//!    verdicts are derived from the SAME final ACFG. The combined verdict
//!    must equal the expanded verdict on every cell, and at least one cell
//!    must exercise each arm (ProvenSound / NeedsExpansion) or the pin is
//!    vacuous.
//!
//! 2. **Corpus arm (post-mediation variants)** — TASK-0455.18, closing
//!    wave-6 review P2.1. The driver ALSO gates the POST-mediation ACFG on
//!    star-topology backends: before the soundness gate it runs
//!    `apply_host_mediation_inject` (every star backend) and, for backends
//!    with no native worker-to-worker DATA channel,
//!    `apply_host_data_relay_inject` (4-hop relay Xfer pairs, incl.
//!    in-Repeat ones). The PRODUCTION gate input for mp-tcp-*/mp-uds-event
//!    is therefore the MEDIATED net, not the pre-mediation one. This arm
//!    A/B-s that net too: for each DISTINCT `(star, relay)` mediation
//!    combination present across the 10 shipping `capabilities.toml`
//!    files (DERIVED from those files at runtime, not a hardcoded list —
//!    the same completeness-pin discipline as `ALL_BACKENDS` <-> the
//!    backends/ dir), it applies the same inject passes against the SAME
//!    elected host the driver uses (host election MIRRORED inline from
//!    `backend_common::host_election` — `nucleus-compiler` cannot depend
//!    on `backend-common`, the arrow runs the other way; the
//!    driver-mirrors-election rule), then asserts symbolic verdict ==
//!    expanded verdict on the post-mediation net. Soundness was never in
//!    doubt (the theorem is shape-generic; an unknown mediated shape lands
//!    in NeedsExpansion -> real expanded gate), but this widens the
//!    empirical falsifier from the pre-mediation input alone.
//!
//! 3. **Negative synthetic-ACFG arm** — hand-built ACFGs whose expanded
//!    net is UNSOUND (over-capacity, stalling, two-consumer/free-choice).
//!    The structural inject-pass guards make a real schedule incapable of
//!    producing one, so these are constructed directly. Each must (a) be
//!    classified `NeedsExpansion` by the symbolic gate (NEVER ProvenSound
//!    — a false ProvenSound here would trade soundness for scaling, the
//!    one thing forbidden) and (b) be rejected by the expanded gate, so
//!    the combined verdict equals the expanded verdict (both reject).
//!
//! The expanded gate runs the `fire_marking` borrowed-net replay core
//! (TASK-0455.10) through `check_net_sound`, exactly as the driver does.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;

use nucleus_compiler::acfg::{
    ACFGNode, DataflowDag, Operation, TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{DataId, IterTile, IterVar, KernelId, SeqTag, WorkerId};
use nucleus_compiler::link;
use nucleus_compiler::passes::acfg_to_petri::acfg_to_net;
use nucleus_compiler::passes::petri_to_events::acfg_to_events;
use nucleus_compiler::petri::Net;
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::{
    analyze_net_soundness_symbolic, apply_host_data_relay_inject, apply_host_mediation_inject,
    check_net_sound, load_capabilities, SymbolicSoundness,
};

// --------------------------------------------------------------------
// Corpus enumeration + lowering
// --------------------------------------------------------------------

fn examples_dir() -> std::path::PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("nuc-nucleus")
        .join("examples")
}

/// Enumerate every (algo, sched) pair on disk: for each `NN-example/`
/// directory with a `prog.algo.nuc`, every `schedules/*.sched.nuc`.
/// Sorted for determinism.
fn enumerate_corpus() -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    let root = examples_dir();
    let mut out = Vec::new();
    let mut example_dirs: Vec<_> = std::fs::read_dir(&root)
        .expect("read examples dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    example_dirs.sort();
    for ex in example_dirs {
        let algo = ex.join("prog.algo.nuc");
        if !algo.exists() {
            continue;
        }
        let sched_dir = ex.join("schedules");
        if !sched_dir.is_dir() {
            continue;
        }
        let mut scheds: Vec<_> = std::fs::read_dir(&sched_dir)
            .expect("read schedules dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "nuc").unwrap_or(false))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with(".sched.nuc"))
                    .unwrap_or(false)
            })
            .collect();
        scheds.sort();
        for s in scheds {
            out.push((algo.clone(), s));
        }
    }
    out
}

/// Lower an (algo, sched) pair through the FULL pre-mediation pass chain
/// (`run_pre_mediation_passes`) the driver runs before the soundness gate.
/// Returns `None` when ANY upstream step (parse / lower / link / a
/// pre-mediation pass) errors — those cells never reach the gate in the
/// driver either, so they are not part of the gate's equivalence domain.
fn lower_to_gate_acfg(algo_path: &std::path::Path, sched_path: &std::path::Path) -> Option<ACFG> {
    let algo_src = std::fs::read_to_string(algo_path).ok()?;
    let sched_src = std::fs::read_to_string(sched_path).ok()?;
    let algo = lower_algo(&parse_algo(&algo_src).ok()?).ok()?;
    let sched = lower_sched(&parse_sched(&sched_src).ok()?).ok()?;
    let linked = link::link(algo, sched).ok()?;
    let (acfg, _advisory) = nucleus_compiler::run_pre_mediation_passes(&linked).ok()?;
    Some(acfg)
}

/// The combined verdict the driver computes (symbolic-first, loud
/// fallback). Returns `(combined_ok, was_proven_sound)`.
fn combined_verdict(acfg: &ACFG, net: &Net) -> (bool, bool) {
    match analyze_net_soundness_symbolic(acfg) {
        SymbolicSoundness::ProvenSound => (true, true),
        SymbolicSoundness::NeedsExpansion(_) => (check_net_sound(net).is_ok(), false),
    }
}

// --------------------------------------------------------------------
// Corpus arm
// --------------------------------------------------------------------

#[test]
fn corpus_combined_verdict_equals_expanded_verdict() {
    let corpus = enumerate_corpus();
    assert!(
        corpus.len() >= 60,
        "corpus enumeration looks wrong (found {} pairs)",
        corpus.len()
    );

    let mut gated = 0usize; // cells that reached the gate
    let mut skipped = 0usize; // cells that errored before the gate
    let mut proven = 0usize; // ProvenSound (symbolic short-circuit)
    let mut fellback = 0usize; // NeedsExpansion (expanded gate decided)
    let mut buffered_proven = 0usize; // ProvenSound AND carries a buffer place
    let mut mismatches: Vec<String> = Vec::new();

    for (algo, sched) in &corpus {
        let label = format!(
            "{}::{}",
            algo.parent().unwrap().file_name().unwrap().to_string_lossy(),
            sched.file_name().unwrap().to_string_lossy()
        );
        let Some(acfg) = lower_to_gate_acfg(algo, sched) else {
            skipped += 1;
            continue;
        };
        gated += 1;

        let net = acfg_to_net(&acfg);
        let expanded_ok = check_net_sound(&net).is_ok();
        let (combined_ok, was_proven) = combined_verdict(&acfg, &net);

        if combined_ok != expanded_ok {
            mismatches.push(format!(
                "{label}: combined={combined_ok} expanded={expanded_ok} (proven={was_proven})"
            ));
            continue;
        }

        let has_buffer = net.places.iter().any(|p| p.name.starts_with("buf_"));
        if was_proven {
            proven += 1;
            // Soundness-equivalence direction: ProvenSound MUST imply the
            // expanded gate accepts. A ProvenSound cell the expanded gate
            // rejected would be a theorem violation (and would have been a
            // mismatch above, but assert it explicitly for a clear message).
            assert!(
                expanded_ok,
                "{label}: ProvenSound but expanded gate REJECTS — soundness traded for scaling"
            );
            if has_buffer {
                buffered_proven += 1;
            }
        } else {
            fellback += 1;
        }
    }

    eprintln!(
        "symbolic-comm A/B corpus totals: enumerated={} gated={} skipped(pre-gate-error)={} \
         proven_sound={} (of which buffered/communicating={}) needs_expansion={}",
        corpus.len(),
        gated,
        skipped,
        proven,
        buffered_proven,
        fellback
    );

    assert!(mismatches.is_empty(), "A/B verdict mismatches:\n  {}", mismatches.join("\n  "));

    // Wave-6 review P3.8: a gated-count floor beside the enumeration
    // floor — a regression that converts gated cells into pre-gate
    // errors would silently shrink the equivalence domain otherwise.
    assert!(
        gated >= 55,
        "A/B equivalence domain shrank: only {gated} cells reached the gate \
         (>= 55 expected); pre-gate errors grew — investigate before trusting \
         the harness"
    );

    // The pin must not be vacuous: both arms exercised, AND the keystone —
    // at least one BUFFERED (communicating) cell proven sound symbolically
    // (the new TASK-0455.01 capability, distinct from buffer-free).
    assert!(proven > 0, "no ProvenSound cell — symbolic fast path never fired");
    assert!(fellback > 0, "no NeedsExpansion cell — fallback path never exercised");
    assert!(
        buffered_proven > 0,
        "no BUFFERED cell proven sound — the communicating-net subclass (the point of \
         TASK-0455.01) was never exercised"
    );
}

// --------------------------------------------------------------------
// Corpus arm — POST-mediation variants (TASK-0455.18, wave-6 review P2.1)
// --------------------------------------------------------------------
//
// The driver runs `apply_host_mediation_inject` / `apply_host_data_relay_inject`
// on the ACFG BEFORE the soundness gate for star-topology backends, so the
// PRODUCTION gate input for mp-tcp-*/mp-uds-event is the MEDIATED net. The
// pre-mediation corpus arm above never A/B-s that net. This arm closes the
// gap: it re-runs each gated corpus cell through every distinct mediation
// combination a real backend applies and asserts symbolic == expanded on
// the post-mediation net too.

/// One `(star_topology_host_mediation, host_data_relay)` mediation
/// combination as declared by a shipping backend. `relay` implies `star`
/// (the validator rejects relay-without-mediation), mirroring the driver's
/// nested `if caps.host_data_relay` inside `if caps.star_topology_host_mediation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MediationVariant {
    star: bool,
    relay: bool,
}

/// Resolve `<repo>/nucleus/backends/`. `CARGO_MANIFEST_DIR` for this test
/// crate is `<repo>/nucleus/nucleus-compiler`; backends live one level up.
fn backends_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("nucleus-compiler dir has a parent (the cargo workspace root)")
        .join("backends")
}

/// Derive the DISTINCT mediation variants present across every shipping
/// `backends/*/capabilities.toml`, loaded with the EXACT `load_capabilities`
/// the driver uses (TASK-0455.09). NOT a hardcoded `{(true,false),(true,true)}`
/// list: deriving from the real files is the same completeness discipline as
/// the `all_backends_list_matches_backends_directory` pin in
/// driver/tests/task0455_09_capability_pass_selection.rs — a new
/// backend declaring a new `(star, relay)` shape is then A/B-d automatically,
/// and a backend whose flags change is picked up without a test edit.
///
/// The identity variant `(false, false)` — every native-barrier / direct-w2w
/// backend — is EXCLUDED from the returned set: it applies no passes, so its
/// gate input IS the pre-mediation ACFG already covered by the corpus arm
/// above. Re-A/B-ing an unmediated ACFG would be a vacuous duplicate.
fn distinct_mediation_variants() -> Vec<MediationVariant> {
    let dir = backends_dir();
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read backends dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("capabilities.toml").is_file())
        .collect();
    entries.sort();
    assert!(
        entries.len() >= 10,
        "expected >= 10 in-tree backends with capabilities.toml, found {} — did the \
         backends/ layout move? (completeness-pin precedent: task0455_09 ALL_BACKENDS)",
        entries.len()
    );

    let mut variants = BTreeSet::new();
    for ent in &entries {
        let path = ent.join("capabilities.toml");
        let caps = load_capabilities(&path)
            .unwrap_or_else(|e| panic!("load capabilities {}: {e}", path.display()));
        // Mirror the driver's nested-flag structure: relay only runs inside
        // the star branch. `validate()` (run by `load_capabilities`) already
        // rejects relay-without-star, so `relay && !star` is unreachable, but
        // normalise defensively so the derived variant is exactly what the
        // driver chain would execute.
        let star = caps.star_topology_host_mediation;
        let relay = star && caps.host_data_relay;
        if star {
            variants.insert(MediationVariant { star, relay });
        }
    }
    variants.into_iter().collect()
}

/// Elect the host EXACTLY as the driver's mediation chain does
/// (`driver/src/main.rs`, the `if caps.star_topology_host_mediation` block):
/// project the ACFG via `acfg_to_events`, collect the workers with non-empty
/// event lists into `used`, then apply the canonical host-election rule.
///
/// The rule is MIRRORED inline from `backend_common::host_election`
/// (`elect_host_from_name_workers`): prefer the worker literally named
/// `"host"` AND present in `used`, else the smallest used `WorkerId`
/// (`used` is a `BTreeSet`, so `.iter().next()` is the smallest), else
/// `None` for a degenerate ACFG. `nucleus-compiler` cannot depend on
/// `backend-common` (the dependency arrow runs the other way), so the
/// canonical helper is unreachable from here; this inline mirror is the
/// SAME two-line rule, with the source pinned. The driver-mirrors-election
/// rule (memory `feedback-driver-must-mirror-backend-election-exactly`)
/// requires the mediation A/B use the same host the driver elects, or the
/// post-mediation net under test would differ from the one the production
/// gate sees. The election-invariant-under-mediation property
/// (`task0455_09::host_election_is_invariant_under_mediation_*`) means one
/// election before the first pass suffices for both passes.
fn elect_host_mirroring_driver(acfg: &ACFG) -> Option<WorkerId> {
    let preview = acfg_to_events(acfg);
    let used: BTreeSet<WorkerId> = preview
        .iter()
        .filter(|(_, evs)| !evs.is_empty())
        .map(|(w, _)| *w)
        .collect();
    // Inline mirror of backend_common::host_election::elect_host_from_name_workers.
    let named_host_in_used = acfg
        .name_workers
        .get("host")
        .copied()
        .filter(|w| used.contains(w));
    named_host_in_used.or_else(|| used.iter().next().copied())
}

/// Apply the mediation passes for `variant` against the driver-elected host,
/// mirroring the driver chain order: `apply_host_mediation_inject` first,
/// then (only when `relay`) `apply_host_data_relay_inject` with the SAME
/// host. Returns the ACFG unchanged for a degenerate ACFG (no elected host)
/// — exactly the driver's `None => acfg` pass-through.
fn apply_mediation(acfg: ACFG, variant: MediationVariant) -> ACFG {
    debug_assert!(variant.star, "identity variant should not reach apply_mediation");
    let Some(h) = elect_host_mirroring_driver(&acfg) else {
        return acfg;
    };
    let acfg = apply_host_mediation_inject(acfg, h);
    if variant.relay {
        apply_host_data_relay_inject(acfg, h)
    } else {
        acfg
    }
}

#[test]
fn post_mediation_corpus_combined_verdict_equals_expanded_verdict() {
    let corpus = enumerate_corpus();
    assert!(
        corpus.len() >= 60,
        "corpus enumeration looks wrong (found {} pairs)",
        corpus.len()
    );

    let variants = distinct_mediation_variants();
    // The shipping tree has exactly two non-identity shapes: host-mediation
    // only (mp-tcp-bufsync, mp-tcp-poll) and host-mediation + host-data-relay
    // (mp-tcp-event, mp-uds-event). Pin the count so a silently-shorter sweep
    // (a capabilities.toml that lost its flag) is loud, while still DERIVING
    // the set from the files rather than hardcoding it.
    assert!(
        !variants.is_empty(),
        "no non-identity mediation variant derived from capabilities.toml files — \
         every star-topology backend lost its flag? (expected >= 1)"
    );
    assert!(
        variants.iter().all(|v| v.star),
        "a non-star variant leaked into the mediation sweep: {variants:?}"
    );

    let mut mediated_cells = 0usize; // (cell, variant) pairs A/B-d post-mediation
    let mut mediated_proven = 0usize; // ProvenSound on a post-mediation net
    let mut mediated_fellback = 0usize; // NeedsExpansion on a post-mediation net
    let mut mediated_changed = 0usize; // variant actually changed the ACFG net
    let mut mismatches: Vec<String> = Vec::new();

    for (algo, sched) in &corpus {
        let Some(base) = lower_to_gate_acfg(algo, sched) else {
            continue; // pre-gate error — never reaches the gate in the driver either
        };
        let base_net_places = acfg_to_net(&base).places.len();

        for &variant in &variants {
            let label = format!(
                "{}::{} [star={} relay={}]",
                algo.parent().unwrap().file_name().unwrap().to_string_lossy(),
                sched.file_name().unwrap().to_string_lossy(),
                variant.star,
                variant.relay
            );
            let mediated = apply_mediation(base.clone(), variant);
            let net = acfg_to_net(&mediated);
            if net.places.len() != base_net_places {
                mediated_changed += 1;
            }

            let expanded_ok = check_net_sound(&net).is_ok();
            let (combined_ok, was_proven) = combined_verdict(&mediated, &net);

            if combined_ok != expanded_ok {
                mismatches.push(format!(
                    "{label}: combined={combined_ok} expanded={expanded_ok} (proven={was_proven})"
                ));
                continue;
            }
            if was_proven {
                mediated_proven += 1;
                // Same soundness-equivalence direction as the pre-mediation
                // arm: ProvenSound MUST imply the expanded gate accepts.
                assert!(
                    expanded_ok,
                    "{label}: post-mediation ProvenSound but expanded gate REJECTS — \
                     soundness traded for scaling"
                );
            } else {
                mediated_fellback += 1;
            }
            mediated_cells += 1;
        }
    }

    eprintln!(
        "symbolic-comm A/B POST-mediation totals: variants={} mediated_cells={} \
         proven_sound={} needs_expansion={} (variant changed the net for {} cells)",
        variants.len(),
        mediated_cells,
        mediated_proven,
        mediated_fellback,
        mediated_changed
    );

    // If a REAL post-mediation verdict divergence ever appears this is a P1
    // finding (the symbolic gate and the expanded gate disagree on a net the
    // production driver actually feeds the gate) — surfaced LOUDLY here, not
    // swallowed. Per the task brief the fix would NOT live in this test.
    assert!(
        mismatches.is_empty(),
        "POST-mediation A/B verdict mismatches (P1 — symbolic != expanded on a net the \
         driver gates in production):\n  {}",
        mismatches.join("\n  ")
    );

    // Post-mediation gated floor, in step with the pre-mediation `gated >= 55`
    // floor (Wave-6 review P3.8 rationale): a regression converting gated cells
    // into pre-gate errors, or a mediation sweep that silently lost a variant,
    // would shrink this domain otherwise. With 2 non-identity variants x ~62
    // gated cells the count is ~124; floor at 100 leaves headroom for benign
    // corpus churn while still catching a halved sweep.
    assert!(
        mediated_cells >= 100,
        "POST-mediation A/B domain shrank: only {mediated_cells} (cell, variant) pairs A/B-d \
         (>= 100 expected) — a variant dropped out of the capabilities-derived set or pre-gate \
         errors grew; investigate before trusting the harness"
    );
    // Non-vacuity: the mediation passes must actually transform SOME cell's
    // net (otherwise we are re-A/B-ing the pre-mediation nets under a fancier
    // name). A host-excluding barrier or a w2w Push/Wait pair triggers it.
    assert!(
        mediated_changed > 0,
        "no corpus cell's net changed under ANY mediation variant — the inject passes were \
         structurally inert across the whole corpus, so this arm adds nothing over the \
         pre-mediation arm; the corpus may have lost its star-topology cells"
    );
    // Both verdict arms must be exercised on post-mediation nets too, or the
    // post-mediation pin is as vacuous as a one-sided pre-mediation pin.
    assert!(
        mediated_proven > 0,
        "no post-mediation cell ProvenSound — symbolic fast path never fired on a mediated net"
    );
    assert!(
        mediated_fellback > 0,
        "no post-mediation cell NeedsExpansion — fallback path never exercised on a mediated net"
    );
}

// --------------------------------------------------------------------
// Negative synthetic-ACFG arm
// --------------------------------------------------------------------

fn policy_buffer(n: u64) -> TransferPolicy {
    TransferPolicy {
        buffer: n,
        ..TransferPolicy::default()
    }
}

fn xfer(role: XferRole, seq: u64, policy: TransferPolicy) -> ACFGNode {
    ACFGNode::Xfer(XferPlaceholder {
        role,
        src: WorkerId(0),
        dst: WorkerId(1),
        data: DataId(0),
        tile: IterTile::empty(),
        seq: SeqTag(seq),
        policy,
    })
}

fn op(workers: &[u64]) -> ACFGNode {
    ACFGNode::Operation(Operation {
        kernel: KernelId(0),
        workers: workers.iter().copied().map(WorkerId).collect(),
        dataflow: DataflowDag::default(),
    })
}

fn synthetic_acfg(root: ACFGNode) -> ACFG {
    synthetic_acfg_with_pipeline(root, BTreeMap::new())
}

fn synthetic_acfg_with_pipeline(
    root: ACFGNode,
    pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64>,
) -> ACFG {
    let mut name_workers = BTreeMap::new();
    name_workers.insert("w0".to_string(), WorkerId(0));
    name_workers.insert("w1".to_string(), WorkerId(1));
    let mut name_data = BTreeMap::new();
    name_data.insert("d".to_string(), DataId(0));
    ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data,
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars: BTreeSet::new(),
        partition_worker_ranges: BTreeMap::new(),
        pipeline_depth_for_seq,
        halo_widths: BTreeMap::new(),
        reuse_widths: BTreeMap::new(),
        partition_pairs: BTreeMap::new(),
        grid_shape_for_outer_iv: BTreeMap::new(),
    }
}

/// Assert the negative-net equivalence: symbolic NeedsExpansion (never
/// ProvenSound), expanded REJECTS, combined == expanded (both reject).
fn assert_negative_equivalence(acfg: &ACFG, what: &str) {
    let net = acfg_to_net(acfg);
    let sym = analyze_net_soundness_symbolic(acfg);
    assert!(
        matches!(sym, SymbolicSoundness::NeedsExpansion(_)),
        "{what}: an UNSOUND net must NOT be ProvenSound (would trade soundness for scaling)"
    );
    let expanded_ok = check_net_sound(&net).is_ok();
    assert!(!expanded_ok, "{what}: expanded gate must REJECT this unsound net");
    let (combined_ok, _) = combined_verdict(acfg, &net);
    assert_eq!(
        combined_ok, expanded_ok,
        "{what}: combined verdict must equal expanded verdict (both reject)"
    );
}

#[test]
fn negative_over_capacity_premark_exceeds_buffer() {
    // GENUINE boundedness violation: a buffer place whose pipeline pre-mark
    // P=2 exceeds its capacity C=1, so it starts already over capacity →
    // the expanded gate's replay rejects (boundedness). (The link step
    // rejects P>C for real schedules; we build the ACFG directly to
    // exercise the gate.) Symbolic: pipeline_p != 0 ⇒ NeedsExpansion.
    //
    // NB the simpler "two sequential Pushes into a cap-1 buffer" is NOT a
    // genuine overflow — the marking-aware firing order pulls the Wait
    // forward to drain between the pushes, so peak stays at 1 (the
    // expanded gate correctly ACCEPTS it). That case is covered as a
    // fall-back-equivalence cell below, not as a rejection.
    let mut pdepth = BTreeMap::new();
    pdepth.insert(SeqTag(0), NonZeroU64::new(2).unwrap());
    let root = ACFGNode::Sequence(vec![
        op(&[0]),
        xfer(XferRole::Push, 0, policy_buffer(1)),
        xfer(XferRole::Wait, 0, policy_buffer(1)),
        op(&[1]),
    ]);
    let acfg = synthetic_acfg_with_pipeline(root, pdepth);
    assert_negative_equivalence(&acfg, "over-capacity (pipeline pre-mark P=2 > cap=1)");
}

#[test]
fn nonunit_push_count_falls_back_and_combined_equals_expanded() {
    // Two sequential Pushes into a cap-1 buffer with a single Wait. This is
    // actually SOUND (the marking-aware firing order drains the buffer
    // between the pushes — peak occupancy 1 ≤ cap 1), but it is OUTSIDE the
    // single-shot subclass (push_count == 2). The symbolic gate MUST fall
    // back (never optimistically prove) and the combined verdict must equal
    // the expanded verdict (both accept).
    let root = ACFGNode::Sequence(vec![
        op(&[0]),
        xfer(XferRole::Push, 0, policy_buffer(1)),
        xfer(XferRole::Push, 0, policy_buffer(1)),
        xfer(XferRole::Wait, 0, policy_buffer(1)),
        op(&[1]),
    ]);
    let acfg = synthetic_acfg(root);
    let net = acfg_to_net(&acfg);
    assert!(
        matches!(
            analyze_net_soundness_symbolic(&acfg),
            SymbolicSoundness::NeedsExpansion(_)
        ),
        "non-unit push count must fall back, never be optimistically proven"
    );
    let expanded_ok = check_net_sound(&net).is_ok();
    let (combined_ok, was_proven) = combined_verdict(&acfg, &net);
    assert!(!was_proven, "must have reached the expanded gate via fallback");
    assert_eq!(combined_ok, expanded_ok, "combined == expanded on the fallback path");
}

#[test]
fn negative_stalling_wait_with_no_push() {
    // A Wait for a seq with NO matching Push: the buffer place is never
    // deposited, so the Wait stalls → deadlock reject. Symbolic:
    // push_count == 0 (≠ 1) ⇒ NeedsExpansion.
    let root = ACFGNode::Sequence(vec![
        op(&[0]),
        xfer(XferRole::Wait, 0, policy_buffer(1)),
        op(&[1]),
    ]);
    let acfg = synthetic_acfg(root);
    assert_negative_equivalence(&acfg, "stalling (wait with no push)");
}

#[test]
fn negative_two_waits_one_push_is_unsound() {
    // One Push, TWO Waits on the same seq (cap=1). The single deposited
    // token can satisfy only one Wait; the second stalls → deadlock
    // reject (and the shared buffer place has two consumer transitions —
    // the free-choice shape). Symbolic: wait_count == 2 ⇒ NeedsExpansion.
    let root = ACFGNode::Sequence(vec![
        op(&[0]),
        xfer(XferRole::Push, 0, policy_buffer(1)),
        xfer(XferRole::Wait, 0, policy_buffer(1)),
        xfer(XferRole::Wait, 0, policy_buffer(1)),
        op(&[1]),
    ]);
    let acfg = synthetic_acfg(root);
    assert_negative_equivalence(&acfg, "two-waits / free-choice (one push, two waits)");
}

#[test]
fn negative_loop_nested_pair_falls_back_even_though_sound() {
    // A loop-nested single Push/Wait pair (buffer=1, P=0). This one is
    // actually SOUND (the expanded gate accepts), but it is OUTSIDE the
    // single-shot subclass, so the symbolic gate MUST fall back loudly
    // (NeedsExpansion) rather than guess. Pins the conservative boundary:
    // combined == expanded == accept, via the FALLBACK path.
    let body = ACFGNode::Sequence(vec![
        op(&[0]),
        xfer(XferRole::Push, 0, policy_buffer(1)),
        xfer(XferRole::Wait, 0, policy_buffer(1)),
        op(&[1]),
    ]);
    let root = ACFGNode::Repeat {
        iter_var: IterVar(0),
        range: 0..4,
        body: Box::new(body),
        block_tag: None,
        break_cond: None,
    };
    let acfg = synthetic_acfg(root);
    let net = acfg_to_net(&acfg);

    // Outside the subclass ⇒ NeedsExpansion (conservative, not optimistic).
    assert!(
        matches!(
            analyze_net_soundness_symbolic(&acfg),
            SymbolicSoundness::NeedsExpansion(_)
        ),
        "loop-nested pair must fall back, not be optimistically proven"
    );
    // But it IS sound, so the expanded gate accepts and the combined
    // verdict (via fallback) matches.
    let expanded_ok = check_net_sound(&net).is_ok();
    assert!(expanded_ok, "loop-nested single pair is sound; expanded gate accepts");
    let (combined_ok, was_proven) = combined_verdict(&acfg, &net);
    assert!(!was_proven, "must have reached the expanded gate via fallback");
    assert_eq!(combined_ok, expanded_ok, "combined == expanded on the fallback path");
}

// --------------------------------------------------------------------
// Scaling perf-pin (AC#2): the keystone — distributed matmul is proven
// sound at N=16 AND N=512 from an iteration-count-INDEPENDENT input,
// so the gate's cost is flat where the expanded net would be ~N^3 (and
// OOM at N=512). Run 3x via #[test] re-invocation determinism inside.
// --------------------------------------------------------------------

/// Lower the distributed matmul (4-worker row-band partition) at
/// dimension `n` through the full pipeline.
fn matmul_distributed_acfg(n: usize) -> ACFG {
    let algo_src = format!(
        r#"const N : usize = {n};
data a : i32[N][N];
data b : i32[N][N];
data c : i32[N][N];
kernel madd   : (i32, i32, i32) -> i32 pure;
kernel load_a : ()          -> i32[N][N] effectful;
kernel load_b : ()          -> i32[N][N] effectful;
kernel save_c : (i32[N][N]) -> ()        effectful;
a <-- load_a();
b <-- load_b();
for i : 0 .. N {{
for j : 0 .. N {{
for k : 0 .. N {{
    c[i][j] <-- madd(c[i][j], a[i][k], b[k][j]);
}}}}}}
save_c(c);
"#
    );
    let sched_src = r#"schedule for "../prog.algo.nuc" {
    workers = { host, w0, w1, w2, w3 };
    place load_a on host;
    place load_b on host;
    place save_c on host;
    place madd   on { w0, w1, w2, w3 };
    loop i : partition=workers;
    transfer a : sync;
    transfer b : sync;
    transfer c : sync;
}
"#;
    let algo = lower_algo(&parse_algo(&algo_src).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(sched_src).expect("sched parse")).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    nucleus_compiler::run_pre_mediation_passes(&linked)
        .expect("run_pre_mediation_passes")
        .0
}

/// Total ACFG-tree node count — the size of the SYMBOLIC analysis input.
fn acfg_node_count(node: &ACFGNode) -> usize {
    1 + match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        ACFGNode::Sequence(cs) => cs.iter().map(acfg_node_count).sum(),
        ACFGNode::Repeat { body, .. } => acfg_node_count(body),
    }
}

/// Does the ROLLED ACFG carry an `Xfer` node (a cross-worker transfer,
/// hence a buffer place once expanded)? Checked structurally from the
/// rolled tree — WITHOUT building the expanded net (which at N=512 would be
/// ~125 GB, the very OOM this pin proves the gate avoids).
fn acfg_has_xfer(node: &ACFGNode) -> bool {
    match node {
        ACFGNode::Xfer(_) => true,
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => false,
        ACFGNode::Sequence(cs) => cs.iter().any(acfg_has_xfer),
        ACFGNode::Repeat { body, .. } => acfg_has_xfer(body),
    }
}

#[test]
fn distributed_matmul_proven_sound_and_iteration_count_independent_at_n512() {
    // The keystone the task lifts: the production gate previously forced
    // the expanded single-order replay for ANY cross-worker transfer; that
    // net is ~N^3 nodes for distributed matmul and OOMs (~25 GB measured)
    // at N=512. The symbolic communicating gate proves it sound from the
    // ROLLED ACFG, whose size is independent of N — so the gate's cost is
    // FLAT. Pin: ProvenSound at both N, an identical ACFG node count, and —
    // crucially — WITHOUT EVER CALLING `acfg_to_net` at N=512 (doing so
    // would rebuild the ~125 GB expanded net and OOM, defeating the very
    // thing under test). At N=16 the expanded net is tiny (~20 K nodes), so
    // we cross-check the buffer-place fact there; at N=512 the buffer-place
    // existence is checked symbolically (the ACFG carries `Xfer` nodes).
    let small = matmul_distributed_acfg(16);
    let large = matmul_distributed_acfg(512);

    // N=16: cheap to expand — cross-check that it really communicates
    // (carries buffer places) and is ProvenSound.
    let small_net = acfg_to_net(&small);
    assert!(
        small_net.places.iter().any(|p| p.name.starts_with("buf_")),
        "N=16: distributed matmul must carry buffer places (it communicates)"
    );
    assert!(
        matches!(
            analyze_net_soundness_symbolic(&small),
            SymbolicSoundness::ProvenSound
        ),
        "N=16: single-shot distributed matmul must be ProvenSound symbolically"
    );

    // N=512: the ACFG carries transfers (checked symbolically — NO expansion)
    // and the gate proves it sound from the rolled ACFG alone.
    assert!(
        acfg_has_xfer(&large.root),
        "N=512: distributed matmul ACFG must carry Xfer nodes (it communicates)"
    );
    assert!(
        matches!(
            analyze_net_soundness_symbolic(&large),
            SymbolicSoundness::ProvenSound
        ),
        "N=512: single-shot distributed matmul must be ProvenSound symbolically \
         WITHOUT expanding the net (the keystone)"
    );

    // The SYMBOLIC analysis input (the rolled ACFG) is byte-for-byte the
    // SAME SIZE at N=16 and N=512 — the property that makes the gate cost
    // flat. (The expanded net the OLD gate built would grow ~32768x in
    // firings between these two N — never materialised here.)
    let small_nodes = acfg_node_count(&small.root);
    let large_nodes = acfg_node_count(&large.root);
    assert_eq!(
        small_nodes, large_nodes,
        "the rolled ACFG (symbolic analysis input) must be iteration-count independent: \
         N=16 -> {small_nodes} nodes, N=512 -> {large_nodes} nodes"
    );
}
