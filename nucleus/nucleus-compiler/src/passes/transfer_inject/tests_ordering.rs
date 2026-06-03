use super::*;

// ----------------------------------------------------------------
// TASK-0389: FIFO host-Push / worker-Wait ordering robustness. The
// worker's per-channel Wait sequence is sorted to the host's
// per-channel Push order (producer-statement rank) so any
// declaration order is FIFO-correct on the strict-FIFO backends.
// ----------------------------------------------------------------

/// A producing Operation that writes `data_out` (no inputs needed
/// for the rank walk). Mirrors the `Operation` shape `build_acfg`
/// emits for a `<--` statement; `producer_rank_by_data` keys on
/// `output_data(op)`.
fn producer_op(kernel: KernelId, data_out: DataId) -> ACFGNode {
    ACFGNode::Operation(Operation {
        kernel,
        workers: [WorkerId(0)].into_iter().collect(),
        dataflow: crate::acfg::DataflowDag {
            edges: vec![crate::acfg::DataflowEdge {
                data_in: Vec::new(),
                kernel,
                data_out: Some(data_out),
                data_in_access: Vec::new(),
                data_out_access: None,
                args: Vec::new(),
            }],
        },
    })
}

/// `producer_rank_by_data` ranks each data symbol by its producing
/// Operation's depth-first walk position — the SAME order
/// `splice_pushes_global` sends the host's Pushes. For the
/// reversed-declaration gather (`val` then `x` then `col_idx`), the
/// gathered array `x` outranks its index array `col_idx`.
#[test]
fn task0389_producer_rank_follows_producer_statement_order() {
    let val = DataId(0);
    let x = DataId(1);
    let col_idx = DataId(2);
    // Top-level Sequence: load val, load x, load col_idx — the
    // reversed-decl producer order (x BEFORE col_idx).
    let root = ACFGNode::Sequence(vec![
        producer_op(KernelId(10), val),
        producer_op(KernelId(11), x),
        producer_op(KernelId(12), col_idx),
    ]);

    let rank = producer_rank_by_data(&root);

    assert_eq!(rank.get(&val), Some(&0), "val is produced first");
    assert_eq!(
        rank.get(&x),
        Some(&1),
        "x (gathered array) is produced second"
    );
    assert_eq!(
        rank.get(&col_idx),
        Some(&2),
        "col_idx (gather index array) is produced LAST in the reversed-decl \
         program — so the host pushes it last, and the worker must wait it last",
    );
    // Strict order: x outranks col_idx (the whole point of the fix).
    assert!(
        rank[&x] < rank[&col_idx],
        "TASK-0389: in the reversed-decl gather the outer array x is \
         produced (and pushed) before its index array col_idx",
    );
}

/// First-writer wins under the single-assignment invariant: a Repeat
/// body's per-iteration producer ranks at its body position, and the
/// rank is assigned at the FIRST occurrence (BTreeMap or_insert).
#[test]
fn task0389_producer_rank_recurses_into_repeat_body_first_occurrence() {
    let a = DataId(0);
    let y = DataId(1);
    let root = ACFGNode::Sequence(vec![
        producer_op(KernelId(10), a),
        ACFGNode::Repeat {
            iter_var: IterVar(1),
            range: 0..4,
            body: Box::new(ACFGNode::Sequence(vec![producer_op(KernelId(11), y)])),
            block_tag: None,
            break_cond: None,
        },
    ]);

    let rank = producer_rank_by_data(&root);
    assert_eq!(rank.get(&a), Some(&0));
    assert_eq!(
        rank.get(&y),
        Some(&1),
        "a producer inside a Repeat body still gets a rank (the dual of \
         producer_repeat_path walking into Repeat bodies)",
    );
}

/// End-to-end of the fix at the `build_waits_for_op` seam: given
/// Waits emitted in `data_in` index-FIRST traversal order
/// (col_idx-before-x, as `collect_dataref_access_expr` produces),
/// the producer-rank sort REORDERS them so the single host->worker
/// channel reads in producer-statement order (val, x, col_idx) —
/// matching the host's send order. WITHOUT the sort the worker would
/// wait {val, col_idx, x} and `read_msg_expect` would panic.
#[test]
fn task0389_build_waits_sorts_per_channel_to_producer_order() {
    let host = WorkerId(0);
    let w0 = WorkerId(1);
    let val = DataId(0);
    let x = DataId(1);
    let col_idx = DataId(2);

    // Reversed-decl producer order: val, x, col_idx.
    let producer_rank: BTreeMap<DataId, usize> = [(val, 0usize), (x, 1usize), (col_idx, 2usize)]
        .into_iter()
        .collect();

    // The op reads val, col_idx, x — `data_in` index-FIRST traversal
    // for `gather_madd(y[i], val[i][k], x[col_idx[i][k]])` records
    // val, then (index-first) col_idx, then x.
    let edge = crate::acfg::DataflowEdge {
        data_in: vec![val, col_idx, x],
        kernel: KernelId(7),
        data_out: Some(DataId(3)),
        data_in_access: Vec::new(),
        data_out_access: None,
        args: Vec::new(),
    };
    let op = Operation {
        kernel: KernelId(7),
        workers: [w0].into_iter().collect(),
        dataflow: crate::acfg::DataflowDag { edges: vec![edge] },
    };

    // All three inputs are produced on the host (host->w0 channel).
    let producers_by_data: BTreeMap<DataId, BTreeSet<WorkerId>> = [
        (val, [host].into_iter().collect::<BTreeSet<_>>()),
        (x, [host].into_iter().collect()),
        (col_idx, [host].into_iter().collect()),
    ]
    .into_iter()
    .collect();
    let policies_by_data = BTreeMap::new();
    let inner_block_iter_vars = BTreeSet::new();
    let partition_iter_vars = BTreeSet::new();
    let name_iter_vars = BTreeMap::new();
    let producer_writes = BTreeMap::new();
    let ctx = InjectCtx {
        producers_by_data: &producers_by_data,
        policies_by_data: &policies_by_data,
        inner_block_iter_vars: &inner_block_iter_vars,
        partition_iter_vars: &partition_iter_vars,
        name_iter_vars: &name_iter_vars,
        producer_writes: &producer_writes,
        producer_rank: &producer_rank,
    };
    let mut state = State {
        next_seq: 0,
        last_writer: BTreeMap::new(),
    };

    let waits = build_waits_for_op(&op, &ctx, &mut state, &[]);

    // One Wait per input, all on the host->w0 channel, sorted into
    // PRODUCER order val, x, col_idx — NOT the data_in traversal
    // order val, col_idx, x.
    let got: Vec<DataId> = waits
        .iter()
        .filter(|w| w.role == XferRole::Wait && w.src == host && w.dst == w0)
        .map(|w| w.data)
        .collect();
    assert_eq!(
        got,
        vec![val, x, col_idx],
        "TASK-0389: the worker's per-channel Wait order must be sorted to \
         the host's per-channel Push (producer-statement) order val, x, \
         col_idx — NOT the data_in index-first traversal order val, \
         col_idx, x that would trip a read_msg_expect seq/tag mismatch on \
         the strict-FIFO backends.",
    );
}

// ----------------------------------------------------------------
// TASK-0389.01: co-hoisted multi-data-per-channel FIFO ordering.
//
// The residual the raw producer-rank Wait sort could not see: ≥2
// loop-OUTPUT data on ONE (src→dst) channel co-hoisting past the
// SAME enclosing Repeat. The naive `splice_after_repeat` ("insert
// immediately after the Repeat") REVERSED them — the host sent in
// reverse-rank order while the worker waited in rank order → a
// strict-FIFO read_msg_expect seq mismatch. CRUX EMPIRICALLY
// CONFIRMED REAL (this fixture printed Push textual order [b, a] vs
// rank order [a, b] BEFORE the fix). FIX: feed the Pushes in
// producer-rank order + append each after any already-spliced
// Pushes at the same insertion point → textual Push order == rank
// order == worker Wait order. Below pins the FIXED behaviour.
// ----------------------------------------------------------------

/// Build a Wait placeholder for `data` on the `src->dst` channel.
fn wait_ph(src: WorkerId, dst: WorkerId, data: DataId, seq: u64) -> XferPlaceholder {
    XferPlaceholder {
        role: XferRole::Wait,
        src,
        dst,
        data,
        tile: IterTile::new(Vec::new()),
        seq: SeqTag(seq),
        policy: TransferPolicy::default(),
    }
}

/// Collect the textual (top-to-bottom) order of Push `data` on the
/// `src->dst` channel — this is the order the backend renders the
/// host's sends, hence the wire FIFO order read_msg_expect enforces.
fn push_textual_order(node: &ACFGNode, src: WorkerId, dst: WorkerId) -> Vec<DataId> {
    fn walk(node: &ACFGNode, src: WorkerId, dst: WorkerId, out: &mut Vec<DataId>) {
        match node {
            ACFGNode::Xfer(x) if x.role == XferRole::Push && x.src == src && x.dst == dst => {
                out.push(x.data)
            }
            ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
            ACFGNode::Repeat { body, .. } => walk(body, src, dst, out),
            ACFGNode::Sequence(children) => {
                for c in children {
                    walk(c, src, dst, out)
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(node, src, dst, &mut out);
    out
}

#[test]
fn task038901_crux_cohoisted_pushes_textual_order_vs_rank() {
    let w0 = WorkerId(1); // producer worker (loop output lives here)
    let host = WorkerId(0); // consumer (reads both after the loop)
    let a = DataId(0);
    let b = DataId(1);

    // Two loop-output data produced INSIDE one Repeat (iter t), in
    // declaration/rank order a then b. Two top-level Waits (post-
    // hoist) on the SAME w0->host channel consume them after the
    // loop, so BOTH Pushes take the cut-hoist (splice_after_repeat).
    // collect_waits DFS order = [a, b] (top-level sequence order).
    let root = ACFGNode::Sequence(vec![
        ACFGNode::Repeat {
            iter_var: IterVar(9),
            range: 0..4,
            body: Box::new(ACFGNode::Sequence(vec![
                producer_op(KernelId(10), a),
                producer_op(KernelId(11), b),
            ])),
            block_tag: None,
            break_cond: None,
        },
        // Top-level Waits, in rank order a then b.
        ACFGNode::Xfer(wait_ph(w0, host, a, 100)),
        ACFGNode::Xfer(wait_ph(w0, host, b, 101)),
    ]);

    let rank = producer_rank_by_data(&root);
    assert_eq!(rank.get(&a), Some(&0), "a produced first inside the loop");
    assert_eq!(rank.get(&b), Some(&1), "b produced second inside the loop");

    let name_data: BTreeMap<String, DataId> = [("a".to_string(), a), ("b".to_string(), b)]
        .into_iter()
        .collect();
    let spliced = splice_pushes_global(root, &name_data);

    let push_order = push_textual_order(&spliced, w0, host);
    // POST-FIX: the two co-hoisted Pushes land in producer-rank
    // order [a, b] textually — matching the order
    // `build_waits_for_op` sorts the worker's Waits into. Pre-fix
    // this was [b, a] (reversed), the FIFO seq-mismatch shape.
    assert_eq!(
        push_order,
        vec![a, b],
        "TASK-0389.01: co-hoisted Push textual order MUST equal \
         producer-rank order [a, b] (was reversed [b, a] pre-fix). \
         Reverse-rank host send vs rank-order worker Wait is the \
         strict-FIFO read_msg_expect seq-mismatch shape.",
    );
}

/// Three co-hoisted data on one channel — pins that the append-
/// after-existing-Pushes logic generalises beyond a pair (a single
/// 2-element swap could masquerade as correct). Rank order is
/// [a, b, c]; textual Push order must be [a, b, c].
#[test]
fn task038901_three_cohoisted_pushes_preserve_rank_order() {
    let w0 = WorkerId(1);
    let host = WorkerId(0);
    let a = DataId(0);
    let b = DataId(1);
    let c = DataId(2);
    let root = ACFGNode::Sequence(vec![
        ACFGNode::Repeat {
            iter_var: IterVar(9),
            range: 0..4,
            body: Box::new(ACFGNode::Sequence(vec![
                producer_op(KernelId(10), a),
                producer_op(KernelId(11), b),
                producer_op(KernelId(12), c),
            ])),
            block_tag: None,
            break_cond: None,
        },
        ACFGNode::Xfer(wait_ph(w0, host, a, 100)),
        ACFGNode::Xfer(wait_ph(w0, host, b, 101)),
        ACFGNode::Xfer(wait_ph(w0, host, c, 102)),
    ]);
    let name_data: BTreeMap<String, DataId> = [
        ("a".to_string(), a),
        ("b".to_string(), b),
        ("c".to_string(), c),
    ]
    .into_iter()
    .collect();
    let spliced = splice_pushes_global(root, &name_data);
    assert_eq!(
        push_textual_order(&spliced, w0, host),
        vec![a, b, c],
        "TASK-0389.01: 3 co-hoisted Pushes must land in rank order",
    );
}

/// The DFS-collect order is NOT producer-rank order in general: if
/// the Waits appear at top-level in REVERSE rank order, the
/// producer-rank sort in `splice_pushes_global` must still produce
/// rank-order textual Pushes. This is the property that makes the
/// fix robust to Wait declaration/placement order (the analogue of
/// the gather_revdecl shape, but for the co-hoist case).
#[test]
fn task038901_cohoist_robust_to_wait_placement_order() {
    let w0 = WorkerId(1);
    let host = WorkerId(0);
    let a = DataId(0);
    let b = DataId(1);
    // Producers a-then-b (rank [a,b]); Waits placed b-then-a.
    let root = ACFGNode::Sequence(vec![
        ACFGNode::Repeat {
            iter_var: IterVar(9),
            range: 0..4,
            body: Box::new(ACFGNode::Sequence(vec![
                producer_op(KernelId(10), a),
                producer_op(KernelId(11), b),
            ])),
            block_tag: None,
            break_cond: None,
        },
        ACFGNode::Xfer(wait_ph(w0, host, b, 101)),
        ACFGNode::Xfer(wait_ph(w0, host, a, 100)),
    ]);
    let name_data: BTreeMap<String, DataId> = [("a".to_string(), a), ("b".to_string(), b)]
        .into_iter()
        .collect();
    let spliced = splice_pushes_global(root, &name_data);
    assert_eq!(
        push_textual_order(&spliced, w0, host),
        vec![a, b],
        "TASK-0389.01: textual Push order keys on producer rank, \
         NOT on Wait DFS/placement order",
    );
}
