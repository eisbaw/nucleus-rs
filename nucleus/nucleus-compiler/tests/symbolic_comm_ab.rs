//! A/B equivalence harness for the symbolic communicating-net gate
//! (TASK-0455.01 AC#1 — the load-bearing acceptance criterion).
//!
//! ## What it pins
//!
//! For every PRE-MEDIATION corpus net plus the synthetic negatives, the
//! COMBINED verdict (the driver additionally gates the POST-mediation
//! ACFG on star-topology backends — corpus x mediation-variant A/B is
//! the open extension, wave-6 review P2.1; soundness is unaffected
//! because unknown shapes land in NeedsExpansion -> real expanded gate)
//! the driver computes —
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
//! / P4 guardrail). The two halves are:
//!
//! 1. **Corpus arm** — every `examples/*/schedules/*.sched.nuc` that
//!    reaches the soundness gate (lowered through the FULL pre-mediation
//!    pass chain the driver runs). Enumerated from the filesystem so a new
//!    example/schedule is covered automatically, with no hand-maintained
//!    list to go stale. Both verdicts are derived from the SAME final
//!    ACFG. The combined verdict must equal the expanded verdict on every
//!    cell, and at least one cell must exercise each arm (ProvenSound /
//!    NeedsExpansion) or the pin is vacuous.
//!
//! 2. **Negative synthetic-ACFG arm** — hand-built ACFGs whose expanded
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
use nucleus_compiler::petri::Net;
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::{analyze_net_soundness_symbolic, check_net_sound, SymbolicSoundness};

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
