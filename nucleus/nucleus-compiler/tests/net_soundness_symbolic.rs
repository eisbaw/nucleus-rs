//! Tests for the symbolic (no-expansion) soundness analysis
//! (TASK-0453.04, rigour epic P4).
//!
//! Three things are pinned here:
//!
//! 1. **Subclass membership (AC#1).** A buffer-free program (single
//!    worker / no cross-worker transfer — e.g. the matmul triple loop)
//!    is classified [`SymbolicSoundness::ProvenSound`]; a distributed
//!    program that communicates is classified
//!    [`SymbolicSoundness::NeedsExpansion`].
//!
//! 2. **Equivalence with the expanded gate (AC#2).** On every corpus
//!    cell the *combined* verdict (use the symbolic answer when
//!    `ProvenSound`, else fall back to the expanded gate — exactly what
//!    the driver does) equals the expanded gate's verdict. This is the
//!    equivalence pin. It also ties the symbolic classification to the
//!    structural buffer-free property exactly: `ProvenSound` iff the
//!    expanded net has zero buffer places.
//!
//! 3. **Soundness preservation / no false `ProvenSound`.** The analysis
//!    never returns `ProvenSound` for an ACFG that carries a transfer
//!    (the only thing that can make a net unsound), even when the
//!    transfer is nested inside a loop body. And it is iteration-count
//!    *independent*: the rolled ACFG it inspects has the same size
//!    whether a loop runs 4 or 4000 times, which is what lifts the
//!    expanded gate's linear-in-firings cost.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::acfg::{ACFGNode, DataflowDag, Operation, SyncPlaceholder, ACFG};
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{IterVar, KernelId, WorkerId};
use nucleus_compiler::link;
use nucleus_compiler::passes::acfg_to_petri::acfg_to_net;
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::{analyze_net_soundness_symbolic, check_net_sound, SymbolicSoundness};

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

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

/// Build the injected ACFG for an example/schedule pair using the same
/// minimal pipeline the boundedness/deadlock integration tests use
/// (`build_acfg` → `inject_syncs` → `inject_transfers`).
fn pipeline_to_acfg(algo_rel: &str, sched_rel: &str) -> ACFG {
    let algo = lower_algo(&parse_algo(&read_example(algo_rel)).expect("algo parse"))
        .expect("algo lower");
    let sched = lower_sched(&parse_sched(&read_example(sched_rel)).expect("sched parse"))
        .expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    inject_transfers(&linked, acfg).expect("inject_transfers")
}

/// Lower an in-memory matmul-shaped algorithm of dimension `n` to an
/// ACFG under a single-worker (host-only) schedule. Used to demonstrate
/// that the symbolic analysis input — the rolled ACFG — is independent
/// of the iteration count, unlike the expanded net.
fn matmul_acfg(n: usize) -> ACFG {
    let algo_src = format!(
        r#"const N : usize = {n};
data a : i32[N][N];
data b : i32[N][N];
data c : i32[N][N];
kernel madd   : (i32, i32, i32) -> i32 pure;
kernel load_a : ()         -> i32[N][N] effectful;
kernel load_b : ()         -> i32[N][N] effectful;
kernel save_c : (i32[N][N]) -> ()       effectful;
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
    workers = { host };
    place load_a on host;
    place load_b on host;
    place save_c on host;
    place madd   on host;
}
"#;
    let algo = lower_algo(&parse_algo(&algo_src).expect("algo parse")).expect("algo lower");
    let sched = lower_sched(&parse_sched(sched_src).expect("sched parse")).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    inject_transfers(&linked, acfg).expect("inject_transfers")
}

/// Count the buffer places (`buf_*`) in an ACFG's expanded net. This is
/// the structural witness of "the net is/ is not buffer-free".
fn buffer_place_count(acfg: &ACFG) -> usize {
    acfg_to_net(acfg)
        .places
        .iter()
        .filter(|p| p.name.starts_with("buf_"))
        .count()
}

fn is_proven_sound(acfg: &ACFG) -> bool {
    matches!(
        analyze_net_soundness_symbolic(acfg),
        SymbolicSoundness::ProvenSound
    )
}

/// Total ACFG-tree node count (the size of the analysis input). Used to
/// show iteration-count independence.
fn acfg_node_count(node: &ACFGNode) -> usize {
    1 + match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        ACFGNode::Sequence(children) => children.iter().map(acfg_node_count).sum(),
        ACFGNode::Repeat { body, .. } => acfg_node_count(body),
    }
}

// The corpus cells exercised by the equivalence pin: a mix of
// single-worker (buffer-free) and distributed (buffered) schedules.
const CORPUS: &[(&str, &str)] = &[
    ("07-matmul/prog.algo.nuc", "07-matmul/schedules/naive.sched.nuc"),
    ("01-elementwise-add/prog.algo.nuc", "01-elementwise-add/schedules/naive.sched.nuc"),
    ("03-reduction/prog.algo.nuc", "03-reduction/schedules/naive.sched.nuc"),
    ("05-stencil/prog.algo.nuc", "05-stencil/schedules/naive.sched.nuc"),
    ("03-reduction/prog.algo.nuc", "03-reduction/schedules/distributed.sched.nuc"),
    ("05-stencil/prog.algo.nuc", "05-stencil/schedules/distributed.sched.nuc"),
    ("07-matmul/prog.algo.nuc", "07-matmul/schedules/distributed.sched.nuc"),
    ("06-separable-filter/prog.algo.nuc", "06-separable-filter/schedules/distributed.sched.nuc"),
];

// --------------------------------------------------------------------
// AC#1 — subclass membership
// --------------------------------------------------------------------

#[test]
fn single_worker_matmul_is_proven_sound_without_expansion() {
    // The cited limitation's headline example: the N=16 triple loop
    // expands to ~8 199 net nodes, but symbolically it is proven sound
    // from the rolled ACFG.
    let acfg = pipeline_to_acfg(
        "07-matmul/prog.algo.nuc",
        "07-matmul/schedules/naive.sched.nuc",
    );
    assert_eq!(buffer_place_count(&acfg), 0, "naive matmul must be buffer-free");
    assert!(
        is_proven_sound(&acfg),
        "single-worker matmul must be classified ProvenSound"
    );
}

#[test]
fn distributed_program_needs_expansion() {
    let acfg = pipeline_to_acfg(
        "07-matmul/prog.algo.nuc",
        "07-matmul/schedules/distributed.sched.nuc",
    );
    assert!(
        buffer_place_count(&acfg) > 0,
        "distributed matmul must carry buffer places"
    );
    assert!(
        !is_proven_sound(&acfg),
        "a program with cross-worker transfers must be NeedsExpansion (fall back)"
    );
}

// --------------------------------------------------------------------
// AC#2 — equivalence with the expanded gate, over the corpus
// --------------------------------------------------------------------

#[test]
fn combined_verdict_matches_expanded_gate_on_corpus() {
    let mut saw_proven = false;
    let mut saw_fallback = false;

    for (algo, sched) in CORPUS {
        let acfg = pipeline_to_acfg(algo, sched);
        let net = acfg_to_net(&acfg);
        let expanded_ok = check_net_sound(&net).is_ok();
        let bufs = net.places.iter().filter(|p| p.name.starts_with("buf_")).count();
        let sym = analyze_net_soundness_symbolic(&acfg);

        // The symbolic classification is EXACTLY the buffer-free
        // structural property — no over/under-reach.
        let proven = matches!(sym, SymbolicSoundness::ProvenSound);
        assert_eq!(
            proven,
            bufs == 0,
            "{sched}: ProvenSound iff buffer-free (bufs={bufs})"
        );

        // The combined gate the driver runs: ProvenSound short-circuits
        // to "sound", otherwise the expanded gate decides.
        let combined_ok = match sym {
            SymbolicSoundness::ProvenSound => {
                saw_proven = true;
                // Soundness-equivalence: a ProvenSound net MUST also be
                // accepted by the expanded gate (the module theorem).
                assert!(
                    expanded_ok,
                    "{sched}: ProvenSound but expanded gate rejected — theorem violated"
                );
                true
            }
            SymbolicSoundness::NeedsExpansion(_) => {
                saw_fallback = true;
                expanded_ok
            }
        };

        assert_eq!(
            combined_ok, expanded_ok,
            "{sched}: combined verdict must equal expanded-gate verdict"
        );
    }

    // The corpus must exercise BOTH arms or the pin is vacuous.
    assert!(saw_proven, "corpus must include at least one ProvenSound (buffer-free) cell");
    assert!(saw_fallback, "corpus must include at least one NeedsExpansion (buffered) cell");
}

// --------------------------------------------------------------------
// AC#3 / soundness preservation — no false ProvenSound
// --------------------------------------------------------------------

#[test]
fn transfer_nested_in_loop_is_never_proven_sound() {
    // The detection must descend loop bodies: an Xfer inside a Repeat
    // still makes the net buffered. (Every distributed schedule with a
    // loop-carried transfer hits this; we assert the structural fact
    // directly on a representative one.)
    let acfg = pipeline_to_acfg(
        "09-producer-consumer/prog.algo.nuc",
        "09-producer-consumer/schedules/pipelined.sched.nuc",
    );
    // Sanity: the transfers really are nested under at least one Repeat.
    fn xfer_under_repeat(node: &ACFGNode, under_loop: bool) -> bool {
        match node {
            ACFGNode::Xfer(_) => under_loop,
            ACFGNode::Operation(_) | ACFGNode::Sync(_) => false,
            ACFGNode::Sequence(cs) => cs.iter().any(|c| xfer_under_repeat(c, under_loop)),
            ACFGNode::Repeat { body, .. } => xfer_under_repeat(body, true),
        }
    }
    assert!(
        xfer_under_repeat(&acfg.root, false),
        "expected a transfer nested inside a loop in this fixture"
    );
    assert!(
        !is_proven_sound(&acfg),
        "a transfer nested inside a loop must still force NeedsExpansion"
    );
}

#[test]
fn symbolic_input_is_iteration_count_independent() {
    // The whole point: the rolled ACFG the symbolic analysis inspects is
    // the SAME size regardless of the iteration count, whereas the
    // expanded net grows ~2 nodes per firing. Pin both: equal ACFG node
    // counts across a 100x dimension change, and a >100x net-node ratio.
    let small = matmul_acfg(4);
    let large = matmul_acfg(40);

    assert!(is_proven_sound(&small) && is_proven_sound(&large));

    let small_acfg_nodes = acfg_node_count(&small.root);
    let large_acfg_nodes = acfg_node_count(&large.root);
    assert_eq!(
        small_acfg_nodes, large_acfg_nodes,
        "the rolled ACFG (symbolic analysis input) must be iteration-count independent"
    );

    // For contrast: the expanded net the old gate would build DOES grow.
    let small_net = acfg_to_net(&small).places.len() + acfg_to_net(&small).transitions.len();
    let large_net = acfg_to_net(&large).places.len() + acfg_to_net(&large).transitions.len();
    assert!(
        large_net > small_net * 100,
        "expanded net must grow with firings (small={small_net}, large={large_net})"
    );
}

// --------------------------------------------------------------------
// Theorem coverage on synthetic nets the corpus does not reach
// --------------------------------------------------------------------
//
// Every buffer-free corpus cell happens to be single-worker, so the
// module theorem's "multi-worker with no transfers" branch (module doc
// "Honest scope") and the degenerate edge cases are not exercised by the
// example-driven tests above. These synthetic ACFGs pin those branches:
// each must be both ProvenSound AND accepted by the expanded gate (a
// false ProvenSound here would be a theorem violation). Promoted from the
// cycle-3 architect review's adversarial probe.

fn synthetic_acfg(root: ACFGNode, workers: &[(&str, u64)]) -> ACFG {
    let mut name_workers = BTreeMap::new();
    for (n, id) in workers {
        name_workers.insert(n.to_string(), WorkerId(*id));
    }
    ACFG {
        root,
        name_kernels: BTreeMap::new(),
        name_data: BTreeMap::new(),
        name_workers,
        name_iter_vars: BTreeMap::new(),
        inner_block_iter_vars: BTreeSet::new(),
        partition_worker_ranges: BTreeMap::new(),
        pipeline_depth_for_seq: BTreeMap::new(),
        halo_widths: BTreeMap::new(),
        reuse_widths: BTreeMap::new(),
        partition_pairs: BTreeMap::new(),
        grid_shape_for_outer_iv: BTreeMap::new(),
    }
}

fn op_on(workers: &[u64]) -> ACFGNode {
    ACFGNode::Operation(Operation {
        kernel: KernelId(0),
        workers: workers.iter().map(|w| WorkerId(*w)).collect(),
        dataflow: DataflowDag::default(),
    })
}

fn sync_on(workers: &[u64]) -> ACFGNode {
    ACFGNode::Sync(SyncPlaceholder {
        participants: workers.iter().map(|w| WorkerId(*w)).collect(),
        ..Default::default()
    })
}

#[test]
fn multiworker_buffer_free_satisfies_theorem() {
    // Two workers, independent ops + a shared barrier + a multi-worker op
    // + a loop body with another barrier. No Xfer ⇒ buffer-free.
    let root = ACFGNode::Sequence(vec![
        op_on(&[0]),
        op_on(&[1]),
        sync_on(&[0, 1]),
        op_on(&[0, 1]),
        op_on(&[1]),
        op_on(&[0]),
        ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..5,
            body: Box::new(ACFGNode::Sequence(vec![
                op_on(&[0]),
                op_on(&[1]),
                sync_on(&[0, 1]),
            ])),
            block_tag: None,
            break_cond: None,
        },
    ]);
    let acfg = synthetic_acfg(root, &[("w0", 0), ("w1", 1)]);

    let net = acfg_to_net(&acfg);
    assert_eq!(
        net.places.iter().filter(|p| p.name.starts_with("buf_")).count(),
        0,
        "constructed net must be buffer-free"
    );
    assert!(is_proven_sound(&acfg), "multi-worker buffer-free must be ProvenSound");
    assert!(
        check_net_sound(&net).is_ok(),
        "THEOREM VIOLATED: ProvenSound but expanded gate rejects a multi-worker buffer-free net"
    );
}

#[test]
fn buffer_free_edge_cases_satisfy_theorem() {
    // Empty loop, an op on the empty worker set, and a single-participant
    // sync — all degenerate but buffer-free.
    let root = ACFGNode::Sequence(vec![
        ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..0,
            body: Box::new(op_on(&[0])),
            block_tag: None,
            break_cond: None,
        },
        op_on(&[]),
        sync_on(&[0]),
    ]);
    let acfg = synthetic_acfg(root, &[("w0", 0)]);
    let net = acfg_to_net(&acfg);
    assert!(is_proven_sound(&acfg));
    assert!(
        check_net_sound(&net).is_ok(),
        "THEOREM VIOLATED on empty-loop / zero-worker edge case"
    );
}

#[test]
fn determinism() {
    let acfg = pipeline_to_acfg(
        "07-matmul/prog.algo.nuc",
        "07-matmul/schedules/naive.sched.nuc",
    );
    let a = analyze_net_soundness_symbolic(&acfg);
    let b = analyze_net_soundness_symbolic(&acfg);
    assert_eq!(a, b);
}
