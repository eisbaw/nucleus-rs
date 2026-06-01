//! TASK-0329.01.02 (slice 2) — host-mediated data-relay injection.
//!
//! For every `Xfer` pair (Push/Wait at `XferRole::Push` / `XferRole::Wait`,
//! sharing `seq`) whose `src` and `dst` are BOTH non-host, route the
//! transfer through host. The single logical hop is rewritten in-place
//! into FOUR sibling `Xfer` placeholders inside the same parent
//! `Sequence`, with two fresh `SeqTag`s drawn from a monotonic
//! allocator initialised at `max_existing_seq + 1`:
//!
//! ```text
//! before:  ... producer_op, Push(src, dst, data, seq),
//!              ... [other ops] ..., Wait(src, dst, data, seq), consumer_op ...
//!
//! after:   ... producer_op,
//!              Push(src,  host, data, seq_a),     // producer-side push to host
//!              Wait(src,  host, data, seq_a),     // host-side wait from producer
//!              ... [other ops] ...,
//!              Push(host, dst,  data, seq_b),     // host-side push to consumer
//!              Wait(host, dst,  data, seq_b),     // consumer-side wait from host
//!              consumer_op ...
//! ```
//!
//! The two intermediate hops (`Wait(host, src=src)` and
//! `Push(host, dst=dst)`) project onto host's per-worker event list
//! at the SAME structural depth as the original pair lived — including
//! INSIDE `Repeat` bodies (PRD §8.3 + the rule in `petri_to_events`'s
//! per-worker `Event::Loop` projection loop — `if body_events.is_empty()
//! { continue; }` — that empty body events suppress the host's `Loop`
//! projection). Because host now contributes a
//! non-empty body to such `Repeat`s, host's projection naturally
//! carries a per-iteration `Event::Loop` whose body contains the
//! `Wait`/`Push` pair — which the existing
//! `backend-common/multi_worker_walker::render_worker_events` walker
//! already lowers to a typed `Chan<T>` `.wait()` / `.push(...)` call
//! on host. No backend-side rendering changes are required.
//!
//! ## Backend scope (driver-conditional)
//!
//! Applied by the driver for `mp-tcp-event` (cycle 163) and
//! `mp-uds-event` (cycle 197 widening for TASK-0044.03.01). Both
//! per-seq-demux event backends share the same synchronous
//! host-relay shape + the same `collect_w2w_pushes` TASK-0330
//! defensive guard that rejects in-Repeat-body w2w Push; without
//! this pass running, 09-producer-consumer/pipelined ×
//! mp-uds-event would hit the same ContractGap mp-tcp-event did
//! before cycle 163. pthreads-sync / pthreads-async / openmp-rs
//! have native shared-memory channels (`Slot<T>` rendezvous via
//! `Mutex<Option<T>>` + `Condvar`) for any (worker, worker) pair
//! and do NOT benefit from host mediation; applying this pass on
//! those backends would *add* unnecessary serialisation through
//! host. mp-tcp-bufsync is excluded per the AC#5 paired-lift FIFO
//! audit (see "Bufsync audit" below); mp-tcp-poll likewise
//! (per-pair FIFO stream — same exclusion rationale as bufsync,
//! per memory `project-mp-tcp-event-vs-bufsync-safety-profile`).
//!
//! ## Variant choice (AC#1 — Option B2 rationale)
//!
//! Cycle-163 picked B2 (ACFG mutation: route Xfer through host) over
//! B1 (new `ACFGNode::Relay` variant) deliberately:
//!
//! - **Match-exhaustiveness audit (B2 vacuous; B1 expensive).** Many
//!   passes match on `ACFGNode` discriminants (grep
//!   `rg -n 'ACFGNode::(Operation|Repeat|Sequence|Sync|Xfer)' nucleus/`
//!   for current sites). B1 would have required adding a `Relay` arm
//!   at every one — mostly mechanical, but each one is a
//!   structural-correctness decision (does this analysis pass treat
//!   Relay like a Sync, like an Xfer, or specially?). B2 only adds and
//!   removes `Xfer` siblings in `Sequence`s; every existing walker
//!   keeps working unchanged. (Cycle-163b architect P2.2 fold-back:
//!   the originally-cited "~17" count was loose; the structural
//!   reasoning stands regardless of the exact number — what matters is
//!   that B2's incremental edit cost is zero arms, B1's is "every
//!   ACFGNode match arm repo-wide".)
//!
//! - **Projection-side handling (B2 free; B1 requires a new arm).**
//!   `petri_to_events::walk` already handles `ACFGNode::Xfer` by
//!   projecting onto src for `Push` and dst for `Wait`. B2's new host
//!   endpoints project to host naturally. B1 would have required
//!   teaching the projector to emit something on host for `Relay`.
//!
//! - **Codegen-side handling (B2 free; B1 requires a new emit path).**
//!   The walker (`render_worker_events`) emits `Chan<T>::wait()` for
//!   `Event::Wait` and `Chan<T>::push(...)` for `Event::Push`. After
//!   B2, host's body has Wait+Push events that the walker emits as
//!   typed Chan calls, with `collect_pre_init_sets` and
//!   `collect_worker_rendezvous` already lifting host's locals + chan
//!   IDs. B1 would have required a parallel emit path for `Relay`.
//!
//! ## Seq allocator (deterministic; no shared state)
//!
//! The pass pre-walks the ACFG to find `max_existing_seq` (the largest
//! `SeqTag.0` across every `ACFGNode::Xfer`). It then allocates fresh
//! seqs monotonically as `max_existing_seq + 1, max_existing_seq + 2,
//! ...` in a deterministic traversal order (depth-first, sibling order
//! within `Sequence`s). No `&mut SeqAllocator` is threaded through the
//! ACFG type; the pass owns the counter for the duration of its walk.
//!
//! This is sufficient because:
//!
//! 1. Every existing seq is `<= max_existing_seq` (by definition).
//! 2. Fresh seqs are strictly `> max_existing_seq` and monotonically
//!    increasing, so no collision with existing OR among fresh.
//! 3. The traversal is deterministic (depth-first sibling order), so
//!    the same input ACFG produces byte-identical fresh-seq
//!    assignment across runs.
//!
//! ## `pipeline_depth_for_seq` mirroring
//!
//! Any rewritten pair's original `seq` may carry a
//! `pipeline_depth_for_seq` entry (set by `transfer_inject`'s
//! annotator for `loop V : pipeline=D` body Xfers — TASK-0233). Both
//! fresh seqs (the producer→host hop and the host→consumer hop)
//! inherit the original's depth verbatim.
//!
//! **What this mirroring actually controls (cycle-163b QA P3.1
//! correction):** the load-bearing consumer of `pipeline_depth_for_seq`
//! is `passes::acfg_to_petri::buffer_place_for` (its initial-marking
//! block reads `self.acfg.pipeline_depth_for_seq.get(&x.seq)`), which
//! pre-seeds each buffer place with `D` initial tokens to model
//! producer-runs-ahead semantics (TASK-0134, PRD §8.2). Without
//! mirroring, the fresh hops' buffer places start with 0 tokens
//! instead of `D`, so the petri-net under-models the steady-state
//! pipeline fill; downstream runtime semantics would diverge from the
//! original pair's pipeline depth (the failure mode is a model-vs-
//! runtime mismatch / pipeline-stall surface, NOT a compile-time
//! `chan_caps` ContractGap — `chan_caps` is built by
//! `sidecar.rs:collect_transfer_buffers` walking each `XferPlaceholder`
//! directly, so the fresh hops are covered automatically by virtue of
//! existing in the ACFG, independent of this mirroring).
//!
//! ## Honest scope (AC#4 — 09 in cycle 163; 13 in cycle 165)
//!
//! Cycle 163 promoted `09-producer-consumer/pipelined × mp-tcp-event`
//! (2 workers × 1 transfer/iter — structurally the smallest case the
//! pass needs to handle). **Cycle 165 (TASK-0329.01.02.01)** promoted
//! `13-cnn-inference/pipeline_parallel × mp-tcp-event` (4 workers ×
//! three inter-stage transfers per pipelined-batch iter — `input`
//! between host and `w_stage1`, `feat1` between `w_stage1` and
//! `w_stage2`, `feat2` between `w_stage2` and `w_stage3`) WITHOUT
//! structural change to the pass — all three inter-stage Push/Wait
//! pairs are SAME-Sequence in-Repeat-body, so the existing
//! `rewrite_sequence_children` pair-matching predicate fires on every
//! non-host pair (the host↔w_stage1 `input` pair is left alone because
//! it has host as endpoint). The cycle-163b residual `(R-singleton)`
//! enumeration warned that `transfer_inject::hoist_invariant_waits`
//! might split a pair across scopes; empirically that did not surface
//! for 13. Bit-identical against `13-cnn-inference/reference.bin` sha256
//! `d893337208d7b46923581ecdea8e326e07e8c7e1204a13d867807d6795f7b861`
//! across 3 non-flake e2e samples (cycle 165).
//!
//! ## Cycle-163 empirical scope-limit refinement
//!
//! The first cycle-163 draft rewrote EVERY non-host-pair Push/Wait —
//! top-level and in-Repeat-body alike. The e2e gate caught it:
//! `05-stencil/distributed-2d × mp-tcp-event` (slice-1's promotion)
//! regressed `FAIL/diff` because its top-level halo Push/Wait pairs
//! (post-bar_0) got an extra host hop. The cycle-149 flat host-relay
//! ALREADY handles top-level w2w transfers via `relay_one(seq,
//! dst_peer, cap)` (bytes-verbatim, tile-aware via
//! `render_wait_assign`); adding host mediation at the ACFG layer
//! for those pairs introduces a SECOND copy through host's local
//! buffer, which breaks the tile-paste semantics (host's local
//! `img_in` is a fresh zero-init `Vec` that does not carry the
//! source worker's slice when host forwards "the whole array").
//!
//! Fix (this draft): scope the pass to **in-Repeat-body** pairs
//! only — the exact TASK-0330 surface this slice is supposed to
//! lift. The `inside_repeat` flag is threaded through the walker;
//! top-level Sequences are walked for recursion but their direct
//! Xfer children are NOT rewritten. The fix is byte-identity-
//! preserving for every currently-passing mp-tcp-event cell.
//!
//! Forward-carry to slice 3 (Option A threaded host-relay) and slice
//! 4 (Option E full w↔w mesh): the parent design language ("Option B
//! interleaved per-Push host-relay") didn't pre-scope to in-Repeat;
//! the structural rule "top-level pairs ARE already handled by
//! cycle-149; only in-Repeat is the new surface" is empirically
//! discovered and should be inherited by any future ACFG-layer
//! lifts. (Memory `feedback-orchestrator-narrative-also-wrong`
//! applies here too — the design narrative was structurally
//! incomplete; the e2e gate found the gap.)
//!
//! ## Idempotence
//!
//! The pass is idempotent on the *result* but NOT on the *path*:
//! applying it twice would rewrite the host-mediated hops (whose
//! Pushes now have `src = host` or `dst = host`) into yet more hops.
//! That violates the "non-host-only" pairing predicate (`src != host
//! && dst != host`), so the second application is a structural no-op
//! (the predicate filters all candidates). See
//! `apply_host_data_relay_inject_idempotent` test below.
//!
//! ## Bufsync audit (AC#5)
//!
//! mp-tcp-bufsync has the same TASK-0330 defensive guard but a
//! different runtime safety profile (per-pair FIFO single stream;
//! `wire::read_msg_expect` panics on seq mismatch — memory
//! `project-mp-tcp-event-vs-bufsync-safety-profile`). The driver does
//! NOT apply this pass on bufsync because:
//!
//! - mp-tcp-bufsync's 09/13 cells are capability-gated on
//!   async/buffer/event so behavioral verification is impossible
//!   (capability-skip happens BEFORE codegen).
//! - The pass's structural shape — adding host as endpoint on a
//!   per-pair FIFO stream — is per-pair-FIFO-safe in principle (each
//!   (src, host) pair and each (host, dst) pair is independent and
//!   monotonic in its own seqs), but with no runtime verification
//!   path available there is no defensible gain in applying the pass
//!   on bufsync today.
//!
//! Driver-side conditionality is intentional and forward-linked: a
//! future cycle that lifts bufsync's capability gate should
//! re-evaluate whether to enable this pass on bufsync. The pass
//! itself is backend-agnostic — only the driver wiring is conditional.
//!
//! ## Composition with slice 1 (AC#6)
//!
//! This pass operates at the **ACFG layer (pre-projection)**, BEFORE
//! `acfg_to_events`. Slice 1's `apply_safe_push_reorder` operates at
//! the **event-list layer (post-projection)**, AFTER
//! `acfg_to_events`. They are at different layers and therefore
//! commute trivially: this pass synthesises host-mediated hops at
//! the ACFG layer; slice 1's reorder pass acts on the
//! already-projected per-worker event lists and reorders within
//! top-level boundaries, never touching events inside `Event::Loop`
//! bodies — which is precisely where this pass's synthesised host
//! Wait/Push events live (per-iteration, inside `Event::Loop`).
//!
//! ## Cross-reference
//!
//! - Parent: TASK-0329.01 cycle-161 + cycle-161b design.
//! - Sibling slice 1: TASK-0329.01.01 (Option D, event-list-layer
//!   reorder pass).
//! - Cycle-160 sibling: `host_mediation_inject` (CTRL-arm host
//!   mediation; this pass is the DATA-arm analogue, scope-wise).
//! - Cycle-153 defensive guard:
//!   `mp-tcp-event/src/multi_worker.rs:collect_w2w_pushes`
//!   (TASK-0330; stays in place as fail-loud safety net). The precise
//!   residual classes this pass does NOT cover (cycle-163b architect
//!   P2.4 fold-back, enumerated rather than hand-waved):
//!   - **(R-bare)** A bare `Xfer` outside any `Sequence` parent
//!     (no sibling slot to land the 4 routed nodes into) — see
//!     `rewrite_at` early-return on the non-Sequence case.
//!   - **(R-singleton)** A `Push` (or `Wait`) without its matching
//!     sibling endpoint in the SAME `Sequence` — e.g. when
//!     `transfer_inject::hoist_invariant_waits` has hoisted one
//!     endpoint into a different scope — see
//!     `rewrite_sequence_children`'s pair-finding skip.
//!   - **(R-toplevel)** Top-level (depth-0) non-host pairs are not
//!     rewritten by design (cycle-149 flat host-relay handles them);
//!     the TASK-0330 guard does not fire here because it's scoped to
//!     `inside_loop = true`, but the residual-class enumeration would
//!     be incomplete without naming it.
//!     The 13-arm follow-up (TASK-0329.01.02.01) is most likely to
//!     surface **(R-singleton)** when inter-stage transfers get
//!     hoist-split by `transfer_inject`.
//! - Memory `feedback-driver-must-mirror-backend-election-exactly` —
//!   host election in the driver uses the same rule as Plan::build.
//! - Memory `project-mp-tcp-event-vs-bufsync-safety-profile` —
//!   load-bearing for the AC#5 bufsync audit.

use crate::acfg::{ACFGNode, XferPlaceholder, XferRole, ACFG};
use crate::event::{SeqTag, WorkerId};

/// Apply host-mediated data-relay injection to the given ACFG.
///
/// Rewrites every Push/Wait pair `(src, dst)` with `src != host &&
/// dst != host` into a four-hop chain through `host`. The original
/// pair's tile, data, and transfer policy are cloned onto every new
/// hop; fresh seqs are allocated monotonically from
/// `max_existing_seq + 1`.
///
/// The original `pipeline_depth_for_seq` entry (if any) for each
/// rewritten seq is mirrored onto both fresh seqs.
///
/// Returns the modified ACFG. Idempotent: a second application is a
/// structural no-op (the pair predicate filters all candidates after
/// the first rewrite).
///
/// Callers (the driver) must invoke this only for backends that
/// benefit from host-mediated DATA topology. Currently mp-tcp-event
/// (cycle 163) and mp-uds-event (cycle 197 widening for
/// TASK-0044.03.01) — see module-level "Backend scope" + "Bufsync
/// audit".
pub fn apply_host_data_relay_inject(mut acfg: ACFG, host: WorkerId) -> ACFG {
    // Phase 1: discover max_existing_seq across every Xfer in the
    // tree. Cheap (single recursive walk) and deterministic.
    let mut max_seq: u64 = 0;
    collect_max_seq(&acfg.root, &mut max_seq);

    // Phase 2: rewrite every non-host-pair Xfer pair INSIDE a Repeat
    // body, allocating fresh seqs from max_seq + 1.
    //
    // SCOPE LIMIT (cycle-163 empirical refinement after 05-stencil/
    // distributed-2d × mp-tcp-event regression — e2e gate found it):
    // top-level (depth-0) non-host pairs are LEFT ALONE. The cycle-149
    // flat host-relay (`Plan::render_relay_phase`) already handles
    // top-level w2w transfers via `relay_one(seq, dst_peer, cap)` —
    // bytes-verbatim, tile-aware via `render_wait_assign`. Adding
    // host mediation at the ACFG layer for these pairs introduces a
    // SECOND copy through host's local buffer, which breaks the
    // tile-paste semantics (`render_wait_assign` 2D-row paste copies
    // from a temp into the recipient's region with halo-strip
    // offsets; with B2's per-hop allocation, host's local `img_in`
    // is a fresh zero-init Vec that does NOT carry the source
    // worker's slice, so when host pushes "the whole array" onward,
    // it pushes zeros into the slice the consumer pastes from).
    //
    // The TASK-0330 surface is SPECIFICALLY the in-Repeat-body case;
    // scoping the pass to Repeat bodies matches the surface and
    // preserves the existing cycle-149 lift for top-level pairs.
    //
    // This means slice-2's scope is narrower than the parent task's
    // wording suggested: we only synthesise host relay where the
    // existing flat-relay mechanism cannot represent the per-iter
    // semantics — i.e. inside Loop bodies, which is where TASK-0330
    // actually fires.
    let mut counter: u64 = max_seq + 1;
    let mut seq_map: Vec<(SeqTag, SeqTag, SeqTag)> = Vec::new();
    rewrite_at(&mut acfg.root, host, false, &mut counter, &mut seq_map);

    // Phase 3: mirror pipeline_depth_for_seq onto fresh seqs.
    //
    // The load-bearing consumer is `passes::acfg_to_petri::
    // buffer_place_for` (its initial-marking block reads
    // `self.acfg.pipeline_depth_for_seq.get(&x.seq)`), which uses the
    // depth as the petri-net buffer place's `initial_marking` to model
    // producer-runs-ahead semantics (TASK-0134). Without this mirror,
    // the fresh hops' buffer places start with 0 tokens instead of D,
    // diverging the petri model from the pair's intended pipeline
    // depth.
    //
    // NB: chan_caps is built by `sidecar.rs:collect_transfer_buffers`
    // walking every XferPlaceholder directly, so the fresh hops are
    // included automatically by virtue of existing in the ACFG —
    // mirroring pipeline_depth_for_seq is NOT what keeps chan_caps
    // populated. (Cycle-163b QA P3.1 mechanism correction.)
    for (orig, to_host, from_host) in &seq_map {
        if let Some(depth) = acfg.pipeline_depth_for_seq.get(orig).copied() {
            acfg.pipeline_depth_for_seq.insert(*to_host, depth);
            acfg.pipeline_depth_for_seq.insert(*from_host, depth);
        }
    }

    acfg
}

/// Phase 1: walk the ACFG and update `out` to the max `SeqTag.0`
/// seen across every `ACFGNode::Xfer`. `out` is initialised by the
/// caller (typically to 0).
fn collect_max_seq(node: &ACFGNode, out: &mut u64) {
    match node {
        ACFGNode::Xfer(x) => {
            if x.seq.0 > *out {
                *out = x.seq.0;
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_max_seq(c, out);
            }
        }
        ACFGNode::Repeat { body, .. } => collect_max_seq(body, out),
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
    }
}

/// Phase 2: in-place rewrite. Walks every `Sequence` and (where
/// `inside_repeat == true`) rewrites non-host-pair Xfer pairs.
/// Records `(orig_seq, seq_to_host, seq_from_host)` for each rewrite
/// so the caller can mirror sidecar-dependent maps.
///
/// `inside_repeat` is set true the moment we descend into a
/// `Repeat::body` and stays true for nested Sequences and nested
/// Repeats. Top-level (depth-0) Xfer pairs are LEFT ALONE — see the
/// "SCOPE LIMIT" comment in [`apply_host_data_relay_inject`] for the
/// 05-stencil/distributed-2d regression that motivated this scoping.
fn rewrite_at(
    node: &mut ACFGNode,
    host: WorkerId,
    inside_repeat: bool,
    counter: &mut u64,
    seq_map: &mut Vec<(SeqTag, SeqTag, SeqTag)>,
) {
    match node {
        ACFGNode::Sequence(children) => {
            if inside_repeat {
                // Rewrite this sequence's children first (in place),
                // then recurse into each child for nested Sequences /
                // Repeat bodies. Order matters: we rewrite at THIS
                // depth before recursing so the recursion doesn't see
                // the newly-synthesised hops (they're either non-paired
                // (already-host endpoint) or they collapse cleanly
                // under the pair predicate).
                rewrite_sequence_children(children, host, counter, seq_map);
            }
            for c in children.iter_mut() {
                rewrite_at(c, host, inside_repeat, counter, seq_map);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            rewrite_at(body, host, true, counter, seq_map);
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {
            // Bare Xfer outside any Sequence cannot be rewritten (a
            // rewrite produces 4 nodes, needs a sibling slot to land
            // in). In practice every Xfer from `transfer_inject` is
            // produced INSIDE a Sequence; a bare Xfer is a
            // pathological shape this pass intentionally does not
            // handle. If `transfer_inject`'s contract weakens, this
            // would surface as a TASK-0330 guard hit downstream.
        }
    }
}

/// Rewrite the direct children of a `Sequence` in place. For each
/// non-host-pair Push/Wait pair (same `(data, seq)`), replace the
/// Push with two new sibling Xfers (Push to host + Wait on host),
/// and replace the Wait with two new sibling Xfers (Push from host +
/// Wait on dst). The relative ordering of Push and Wait in the
/// original sequence is preserved.
///
/// Single linear scan; if multiple pairs share a Sequence each is
/// rewritten independently. Determinism: BTreeMap-backed pair index +
/// sequential scan of `children`.
fn rewrite_sequence_children(
    children: &mut Vec<ACFGNode>,
    host: WorkerId,
    counter: &mut u64,
    seq_map: &mut Vec<(SeqTag, SeqTag, SeqTag)>,
) {
    use std::collections::BTreeMap;

    // First pass: index every non-host-pair Push and Wait by their
    // shared key. `(data, seq)` is the cross-pair join key per the
    // transfer_inject contract (XferPlaceholder.seq is unique per
    // Push/Wait pair).
    //
    // The predicate is on the role-pair *combined*: we rewrite ONLY
    // when BOTH endpoints are non-host. A Push from worker_a to host
    // (host-receives) is left alone — that's a normal host-mediated
    // hop already. A Push from host to worker_a is also left alone.
    let mut push_idx: BTreeMap<(u64, u64), usize> = BTreeMap::new();
    let mut wait_idx: BTreeMap<(u64, u64), usize> = BTreeMap::new();
    for (i, c) in children.iter().enumerate() {
        if let ACFGNode::Xfer(x) = c {
            if x.src == host || x.dst == host {
                continue;
            }
            let key = (x.data.0, x.seq.0);
            match x.role {
                XferRole::Push => {
                    push_idx.insert(key, i);
                }
                XferRole::Wait => {
                    wait_idx.insert(key, i);
                }
            }
        }
    }

    // Build the rewrite plan: for each key that has BOTH a Push and
    // Wait in this Sequence, allocate fresh seqs and record the
    // replacement quads. Singletons (Push without matching Wait in
    // the same Sequence, or vice versa) are LEFT ALONE — they may be
    // paired in an outer/inner Sequence or hoisted out of a Repeat
    // (the transfer_inject hoist_invariant_waits arm can do this);
    // those shapes are not the cycle-163 target.
    struct Plan {
        push_at: usize,
        wait_at: usize,
        repl_push: (XferPlaceholder, XferPlaceholder),
        repl_wait: (XferPlaceholder, XferPlaceholder),
    }
    let mut plans: Vec<Plan> = Vec::new();
    for (key, push_at) in &push_idx {
        let Some(&wait_at) = wait_idx.get(key) else {
            continue;
        };
        // Both Push and Wait exist for this key; extract the
        // (src, dst, data, tile, policy) from the existing Xfer.
        let push_x = match &children[*push_at] {
            ACFGNode::Xfer(x) => x.clone(),
            _ => unreachable!("indexed as Push but not Xfer; index corruption"),
        };
        let wait_x = match &children[wait_at] {
            ACFGNode::Xfer(x) => x.clone(),
            _ => unreachable!("indexed as Wait but not Xfer; index corruption"),
        };
        // Defensive: src and dst must match between Push and Wait
        // (they are two views of one pair).
        debug_assert_eq!(
            push_x.src, wait_x.src,
            "B2 pair predicate hit but Push.src != Wait.src — \
             transfer_inject contract violation upstream"
        );
        debug_assert_eq!(push_x.dst, wait_x.dst, "B2 pair: Push.dst != Wait.dst");
        debug_assert_eq!(push_x.data, wait_x.data, "B2 pair: data mismatch");

        // Allocate two fresh seqs: one for src→host, one for
        // host→dst. Monotonic from the pre-walked max.
        let seq_to_host = SeqTag(*counter);
        *counter += 1;
        let seq_from_host = SeqTag(*counter);
        *counter += 1;

        // Build the four replacement Xfers.
        //
        // Replacement for the original Push slot: keep at src's
        // position the Push toward host, immediately followed by
        // host's Wait from src. (Wait can sit at any depth that
        // projects onto host — sibling placement under the original
        // Push is the simplest structural fit.)
        let push_to_host = XferPlaceholder {
            role: XferRole::Push,
            src: push_x.src,
            dst: host,
            data: push_x.data,
            tile: push_x.tile.clone(),
            seq: seq_to_host,
            policy: push_x.policy,
        };
        let wait_on_host_from_src = XferPlaceholder {
            role: XferRole::Wait,
            src: push_x.src,
            dst: host,
            data: push_x.data,
            tile: push_x.tile.clone(),
            seq: seq_to_host,
            policy: push_x.policy,
        };

        // Replacement for the original Wait slot: host pushes to
        // dst, then dst waits from host. Wait stays at the
        // consumer-side position.
        let push_from_host_to_dst = XferPlaceholder {
            role: XferRole::Push,
            src: host,
            dst: wait_x.dst,
            data: wait_x.data,
            tile: wait_x.tile.clone(),
            seq: seq_from_host,
            policy: wait_x.policy,
        };
        let wait_on_dst_from_host = XferPlaceholder {
            role: XferRole::Wait,
            src: host,
            dst: wait_x.dst,
            data: wait_x.data,
            tile: wait_x.tile.clone(),
            seq: seq_from_host,
            policy: wait_x.policy,
        };

        plans.push(Plan {
            push_at: *push_at,
            wait_at,
            repl_push: (push_to_host, wait_on_host_from_src),
            repl_wait: (push_from_host_to_dst, wait_on_dst_from_host),
        });
        seq_map.push((push_x.seq, seq_to_host, seq_from_host));
    }

    if plans.is_empty() {
        return;
    }

    // Apply rewrites in REVERSE positional order so earlier index
    // mutations don't invalidate later ones. Each plan replaces ONE
    // index with two siblings, shifting everything to the right by
    // +1. Doing this in reverse order keeps every plan's recorded
    // indices stable. (We also have two indices per plan — Push and
    // Wait — but they're in distinct positions; we sort all
    // (position, payload) pairs together and apply in reverse.)
    struct Replacement {
        at: usize,
        with: (XferPlaceholder, XferPlaceholder),
    }
    let mut replacements: Vec<Replacement> = Vec::with_capacity(plans.len() * 2);
    for p in plans {
        replacements.push(Replacement {
            at: p.push_at,
            with: p.repl_push,
        });
        replacements.push(Replacement {
            at: p.wait_at,
            with: p.repl_wait,
        });
    }
    replacements.sort_by_key(|r| std::cmp::Reverse(r.at));
    for r in replacements {
        // Replace children[r.at] with two siblings (first, second).
        children[r.at] = ACFGNode::Xfer(r.with.0);
        children.insert(r.at + 1, ACFGNode::Xfer(r.with.1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acfg::{ACFGNode, NotifyMode, TransferPolicy, XferPlaceholder, XferRole};
    use crate::event::{DataId, IterTile, IterVar, SeqTag, SyncTag, WorkerId};
    use std::collections::BTreeMap;

    fn host() -> WorkerId {
        WorkerId(0)
    }
    fn w1() -> WorkerId {
        WorkerId(1)
    }
    fn w2() -> WorkerId {
        WorkerId(2)
    }
    fn d(id: u64) -> DataId {
        DataId(id)
    }

    fn empty_acfg(root: ACFGNode) -> ACFG {
        ACFG {
            root,
            name_kernels: Default::default(),
            name_data: Default::default(),
            name_workers: Default::default(),
            name_iter_vars: Default::default(),
            inner_block_iter_vars: Default::default(),
            partition_worker_ranges: Default::default(),
            pipeline_depth_for_seq: Default::default(),
            halo_widths: Default::default(),
            reuse_widths: Default::default(),
            partition_pairs: Default::default(),
            grid_shape_for_outer_iv: Default::default(),
        }
    }

    fn xfer(role: XferRole, src: WorkerId, dst: WorkerId, data: DataId, seq: u64) -> ACFGNode {
        ACFGNode::Xfer(XferPlaceholder {
            role,
            src,
            dst,
            data,
            tile: IterTile::new(vec![]),
            seq: SeqTag(seq),
            policy: TransferPolicy::default(),
        })
    }

    fn pair_xfers(src: WorkerId, dst: WorkerId, data: DataId, seq: u64) -> Vec<ACFGNode> {
        vec![
            xfer(XferRole::Push, src, dst, data, seq),
            xfer(XferRole::Wait, src, dst, data, seq),
        ]
    }

    #[test]
    fn host_only_pairs_unchanged() {
        // Push from host to w1 — already host-mediated; pass leaves alone.
        let root = ACFGNode::Sequence(pair_xfers(host(), w1(), d(0), 1));
        let acfg = empty_acfg(root.clone());
        let out = apply_host_data_relay_inject(acfg, host());
        assert_eq!(out.root, root, "host endpoint pair must be unchanged");
    }

    #[test]
    fn non_host_pair_at_top_level_is_left_alone() {
        // SCOPE LIMIT (cycle-163 empirical refinement after
        // 05-stencil/distributed-2d × mp-tcp-event e2e regression):
        // top-level non-host pairs are NOT rewritten — the cycle-149
        // flat host-relay handles them. Only in-Repeat-body pairs are
        // the TASK-0330 surface this pass lifts.
        let root = ACFGNode::Sequence(pair_xfers(w1(), w2(), d(7), 3));
        let acfg = empty_acfg(root.clone());
        let out = apply_host_data_relay_inject(acfg, host());
        assert_eq!(
            out.root, root,
            "top-level non-host pair must be unchanged (scope limit: \
             only in-Repeat-body pairs get rewritten by this pass)"
        );
    }

    #[test]
    fn non_host_pair_inside_repeat_body_is_rewritten() {
        // Push/Wait pair inside a Repeat body: this is the TASK-0330
        // surface. Expect 4 hops via host.
        let body = ACFGNode::Sequence(pair_xfers(w1(), w2(), d(7), 3));
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(body),
            block_tag: None,
        };
        let acfg = empty_acfg(root);
        let out = apply_host_data_relay_inject(acfg, host());
        // The Sequence inside the Repeat body must have 4 hops now.
        let body_kids = match &out.root {
            ACFGNode::Repeat { body, .. } => match body.as_ref() {
                ACFGNode::Sequence(k) => k.clone(),
                _ => panic!("body must be Sequence"),
            },
            _ => panic!("root must be Repeat"),
        };
        assert_eq!(body_kids.len(), 4, "Push+Wait → 4 hops inside Repeat body");
        // Verify endpoints and roles in order.
        let expectations: [(XferRole, WorkerId, WorkerId); 4] = [
            (XferRole::Push, w1(), host()),
            (XferRole::Wait, w1(), host()),
            (XferRole::Push, host(), w2()),
            (XferRole::Wait, host(), w2()),
        ];
        for (i, (role, src, dst)) in expectations.iter().enumerate() {
            match &body_kids[i] {
                ACFGNode::Xfer(x) => {
                    assert_eq!(x.role, *role, "kid[{i}].role");
                    assert_eq!(x.src, *src, "kid[{i}].src");
                    assert_eq!(x.dst, *dst, "kid[{i}].dst");
                    assert_eq!(x.data, d(7), "kid[{i}].data");
                }
                _ => panic!("kid[{i}] must be Xfer"),
            }
        }
        // Fresh seqs are monotonic from max_existing_seq + 1 = 4.
        let seqs: Vec<u64> = body_kids
            .iter()
            .filter_map(|n| match n {
                ACFGNode::Xfer(x) => Some(x.seq.0),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![4, 4, 5, 5], "fresh seqs from max+1, paired");
    }

    #[test]
    fn rewritten_pair_inside_repeat_body_projects_host_loop() {
        // The load-bearing 09 shape: Push/Wait pair inside a Repeat
        // body. After the pass, host's projection MUST emit a
        // non-empty body for that Repeat (else petri_to_events
        // suppresses host's Loop entirely — the "empty body skip"
        // rule).
        use crate::event::Event;
        use crate::passes::petri_to_events::acfg_to_events;

        let body = ACFGNode::Sequence(pair_xfers(w1(), w2(), d(7), 3));
        let mut name_workers = BTreeMap::new();
        // Register host + workers in name_workers so the projector
        // includes them.
        name_workers.insert("host".to_string(), host());
        name_workers.insert("w1".to_string(), w1());
        name_workers.insert("w2".to_string(), w2());
        let mut acfg = empty_acfg(ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(body),
            block_tag: None,
        });
        acfg.name_workers = name_workers;

        let out = apply_host_data_relay_inject(acfg, host());
        let per_worker = acfg_to_events(&out);
        // host MUST have an Event::Loop with non-empty body.
        let host_evs = per_worker.get(&host()).expect("host events");
        assert_eq!(
            host_evs.len(),
            1,
            "host has exactly one top-level event (the Loop)"
        );
        let host_body = match &host_evs[0] {
            Event::Loop { body, .. } => body,
            other => panic!("expected Event::Loop at host[0], got {other:?}"),
        };
        // Body should contain Wait+Push (from w1 to host then host to w2).
        assert_eq!(host_body.len(), 2, "host's loop body = Wait + Push");
        match &host_body[0] {
            Event::Wait { src, .. } => assert_eq!(*src, w1(), "first body event waits from w1"),
            other => panic!("expected Wait at host_body[0], got {other:?}"),
        }
        match &host_body[1] {
            Event::Push { dst, .. } => assert_eq!(*dst, w2(), "second body event pushes to w2"),
            other => panic!("expected Push at host_body[1], got {other:?}"),
        }
    }

    #[test]
    fn apply_host_data_relay_inject_idempotent() {
        // Second application is a structural no-op: all fresh hops
        // have host as endpoint, so they fail the non-host-pair
        // predicate and aren't rewritten further. (Wrap in Repeat
        // because the scope limit means top-level isn't rewritten.)
        let body = ACFGNode::Sequence(pair_xfers(w1(), w2(), d(7), 3));
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..4,
            body: Box::new(body),
            block_tag: None,
        };
        let once = apply_host_data_relay_inject(empty_acfg(root), host());
        let twice = apply_host_data_relay_inject(once.clone(), host());
        assert_eq!(once.root, twice.root, "second pass must be a no-op");
    }

    #[test]
    fn singleton_push_or_wait_left_alone() {
        // A Push without a matching Wait in the same Sequence is
        // NOT rewritten (the pair must be in the same scope). Wrap
        // in Repeat to put it in scope of the pass.
        let body = ACFGNode::Sequence(vec![xfer(XferRole::Push, w1(), w2(), d(7), 3)]);
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..4,
            body: Box::new(body),
            block_tag: None,
        };
        let acfg = empty_acfg(root.clone());
        let out = apply_host_data_relay_inject(acfg, host());
        assert_eq!(out.root, root, "singleton Push must be unchanged");
    }

    #[test]
    fn pipeline_depth_mirrored_to_fresh_seqs() {
        // Original seq=3 has pipeline_depth=4. After rewrite, both
        // fresh seqs (4, 5) MUST carry depth=4 in
        // pipeline_depth_for_seq, so build_sidecar's
        // transfer_buffer_for_seq map has entries for host's new
        // Chan instances. Wrap pair in Repeat (scope limit).
        let body = ACFGNode::Sequence(pair_xfers(w1(), w2(), d(7), 3));
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(body),
            block_tag: None,
        };
        let mut acfg = empty_acfg(root);
        acfg.pipeline_depth_for_seq.insert(
            SeqTag(3),
            std::num::NonZeroU64::new(4).expect("4 is non-zero"),
        );
        let out = apply_host_data_relay_inject(acfg, host());
        let depth = out.pipeline_depth_for_seq.get(&SeqTag(4)).copied();
        assert_eq!(
            depth.map(|d| d.get()),
            Some(4),
            "fresh seq_to_host inherits pipeline depth"
        );
        let depth = out.pipeline_depth_for_seq.get(&SeqTag(5)).copied();
        assert_eq!(
            depth.map(|d| d.get()),
            Some(4),
            "fresh seq_from_host inherits pipeline depth"
        );
        // Original seq still carries its entry too (we don't remove
        // it; downstream passes ignore orphan SeqTags that have no
        // emitted Xfer).
        let depth = out.pipeline_depth_for_seq.get(&SeqTag(3)).copied();
        assert_eq!(
            depth.map(|d| d.get()),
            Some(4),
            "original seq depth entry preserved (orphan but harmless)"
        );
    }

    #[test]
    fn multiple_pairs_same_sequence_each_rewritten_independently() {
        // Two non-host pairs in one Sequence (inside Repeat per scope
        // limit); each must get its own fresh seq pair.
        let mut body_children = pair_xfers(w1(), w2(), d(7), 3);
        body_children.extend(pair_xfers(w2(), w1(), d(8), 5));
        let body = ACFGNode::Sequence(body_children);
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..4,
            body: Box::new(body),
            block_tag: None,
        };
        let acfg = empty_acfg(root);
        let out = apply_host_data_relay_inject(acfg, host());
        let kids = match &out.root {
            ACFGNode::Repeat { body, .. } => match body.as_ref() {
                ACFGNode::Sequence(k) => k.clone(),
                _ => panic!("body Sequence"),
            },
            _ => panic!("root Repeat"),
        };
        assert_eq!(kids.len(), 8, "two pairs × 4 hops each");
        // Verify each pair has a unique fresh seq pair.
        let seqs: Vec<u64> = kids
            .iter()
            .filter_map(|n| match n {
                ACFGNode::Xfer(x) => Some(x.seq.0),
                _ => None,
            })
            .collect();
        // Fresh seqs start at max+1 = 6, monotonic across both pairs.
        let unique: std::collections::BTreeSet<u64> = seqs.iter().copied().collect();
        assert_eq!(unique.len(), 4, "4 fresh seqs across 2 rewritten pairs");
        for s in &unique {
            assert!(*s >= 6, "fresh seq {s} must be > max original seq 5");
        }
    }

    #[test]
    fn sync_and_operation_nodes_unaffected() {
        // A Sequence with a Sync and a non-host pair, wrapped in
        // Repeat (scope limit) — only the pair is rewritten; Sync is
        // untouched.
        let mut parts = std::collections::BTreeSet::new();
        parts.insert(host());
        parts.insert(w1());
        let sync_node = ACFGNode::Sync(crate::acfg::SyncPlaceholder {
            participants: parts,
            sync: SyncTag(11),
        });
        let mut body_children = vec![sync_node.clone()];
        body_children.extend(pair_xfers(w1(), w2(), d(7), 3));
        let body = ACFGNode::Sequence(body_children);
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..4,
            body: Box::new(body),
            block_tag: None,
        };
        let acfg = empty_acfg(root);
        let out = apply_host_data_relay_inject(acfg, host());
        let kids = match &out.root {
            ACFGNode::Repeat { body, .. } => match body.as_ref() {
                ACFGNode::Sequence(k) => k.clone(),
                _ => panic!("body Sequence"),
            },
            _ => panic!("root Repeat"),
        };
        // Expected: [Sync, Push(w1→host), Wait(w1→host), Push(host→w2), Wait(host→w2)]
        assert_eq!(kids.len(), 5, "1 Sync + 4 hops");
        assert_eq!(&kids[0], &sync_node, "Sync untouched");
    }

    #[test]
    fn deep_repeat_nesting_is_walked() {
        // Pair inside Repeat inside Repeat — both levels walked.
        let inner_body = ACFGNode::Sequence(pair_xfers(w1(), w2(), d(7), 3));
        let inner = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..4,
            body: Box::new(inner_body),
            block_tag: None,
        };
        let outer = ACFGNode::Repeat {
            iter_var: IterVar(1),
            range: 0..2,
            body: Box::new(inner),
            block_tag: None,
        };
        let acfg = empty_acfg(outer);
        let out = apply_host_data_relay_inject(acfg, host());
        // Innermost Sequence should be rewritten.
        let outer_body = match &out.root {
            ACFGNode::Repeat { body, .. } => &**body,
            _ => panic!("outer Repeat"),
        };
        let inner_body = match outer_body {
            ACFGNode::Repeat { body, .. } => &**body,
            _ => panic!("inner Repeat"),
        };
        let kids = match inner_body {
            ACFGNode::Sequence(k) => k,
            _ => panic!("Sequence"),
        };
        assert_eq!(kids.len(), 4, "nested pair rewritten to 4 hops");
    }

    #[test]
    fn mixed_top_level_and_in_repeat_pairs_only_in_repeat_rewritten() {
        // 05-stencil/distributed-2d shape canary (cycle-163 e2e
        // regression pin): the algorithm has top-level halo
        // Push/Wait pairs (post-bar_0) AND no in-loop w2w pairs.
        // The pass MUST NOT rewrite the top-level halo pairs (the
        // cycle-149 flat host-relay handles them); it MUST rewrite
        // any in-Repeat-body pair.
        let top_level_pair = pair_xfers(w1(), w2(), d(0), 1);
        let in_repeat_pair = ACFGNode::Sequence(pair_xfers(w1(), w2(), d(1), 3));
        let repeat = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(in_repeat_pair),
            block_tag: None,
        };
        let mut root_kids: Vec<ACFGNode> = top_level_pair.clone();
        root_kids.push(repeat);
        let root = ACFGNode::Sequence(root_kids);
        let acfg = empty_acfg(root);
        let out = apply_host_data_relay_inject(acfg, host());
        let kids = match &out.root {
            ACFGNode::Sequence(k) => k,
            _ => panic!("Sequence at root"),
        };
        // Top-level pair (kids[0], kids[1]) is UNCHANGED.
        assert_eq!(kids.len(), 3, "[top_push, top_wait, Repeat]");
        match (&kids[0], &kids[1]) {
            (ACFGNode::Xfer(x0), ACFGNode::Xfer(x1)) => {
                assert_eq!(x0.role, XferRole::Push, "top-level Push intact");
                assert_eq!(x0.src, w1());
                assert_eq!(x0.dst, w2());
                assert_eq!(x0.seq, SeqTag(1), "top-level Push seq unchanged");
                assert_eq!(x1.role, XferRole::Wait);
                assert_eq!(x1.seq, SeqTag(1), "top-level Wait seq unchanged");
            }
            _ => panic!("top-level pair must remain as 2 Xfer nodes"),
        }
        // Repeat body's pair is REWRITTEN to 4 hops.
        let body_kids = match &kids[2] {
            ACFGNode::Repeat { body, .. } => match body.as_ref() {
                ACFGNode::Sequence(k) => k.clone(),
                _ => panic!("body Sequence"),
            },
            _ => panic!("kids[2] Repeat"),
        };
        assert_eq!(body_kids.len(), 4, "in-Repeat pair rewritten to 4 hops");
    }

    #[test]
    fn task_0329_01_02_01_13_cnn_inference_pipeline_parallel_shape() {
        // TASK-0329.01.02.01 cycle-165 regression pin for the
        // 13-cnn-inference/pipeline_parallel × mp-tcp-event promotion.
        //
        // Shape: 4 workers (host + w_stage1 + w_stage2 + w_stage3), 3
        // distinct-data inter-stage transfer pairs in the SAME Sequence
        // inside a Repeat body (the per-batch pipelined loop):
        //
        //   - feat1: w_stage1 -> w_stage2  (data id 1, seq 10)
        //   - feat2: w_stage2 -> w_stage3  (data id 2, seq 11)
        //   - output: w_stage3 -> host    (data id 3, seq 12)  — LEFT ALONE (host endpoint)
        //   - input: host -> w_stage1     (data id 0, seq 9)   — LEFT ALONE (host endpoint)
        //
        // The two NON-HOST inter-stage pairs (feat1, feat2) must each
        // be rewritten to 4 hops via host. The two host-endpoint pairs
        // (input, output) must be left UNCHANGED. This is the load-
        // bearing shape the cycle-163b `(R-singleton)` enumeration
        // warned MIGHT surface (if transfer_inject::hoist_invariant_waits
        // had split a pair across scopes) — empirical cycle-165
        // verification confirmed it did NOT for 13, so the existing
        // same-Sequence pair-matching predicate is sufficient.
        //
        // If a future cycle changes hoist_invariant_waits to split one
        // of these pairs across scopes, this test will still pass
        // (it tests the in-Sequence shape directly, not the hoist
        // behaviour) but the 13 e2e cell will regress at the
        // TASK-0330 guard, signalling that an (R-singleton) extension
        // is now required. The pin's role is to lock in the
        // 4-worker 3-pair shape the cycle-165 promotion relied on,
        // not to detect future hoist-split — that surface is the e2e
        // gate's job.
        let w_stage1 = WorkerId(1);
        let w_stage2 = WorkerId(2);
        let w_stage3 = WorkerId(3);
        let input_data = d(0);
        let feat1_data = d(1);
        let feat2_data = d(2);
        let output_data = d(3);
        let mut body_children: Vec<ACFGNode> = Vec::new();
        // Host -> w_stage1 (input) — host endpoint, left alone.
        body_children.extend(pair_xfers(host(), w_stage1, input_data, 9));
        // w_stage1 -> w_stage2 (feat1) — non-host pair, REWRITTEN.
        body_children.extend(pair_xfers(w_stage1, w_stage2, feat1_data, 10));
        // w_stage2 -> w_stage3 (feat2) — non-host pair, REWRITTEN.
        body_children.extend(pair_xfers(w_stage2, w_stage3, feat2_data, 11));
        // w_stage3 -> host (output) — host endpoint, left alone.
        body_children.extend(pair_xfers(w_stage3, host(), output_data, 12));
        let body = ACFGNode::Sequence(body_children);
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..16,
            body: Box::new(body),
            block_tag: None,
        };
        let acfg = empty_acfg(root);
        let out = apply_host_data_relay_inject(acfg, host());
        let kids = match &out.root {
            ACFGNode::Repeat { body, .. } => match body.as_ref() {
                ACFGNode::Sequence(k) => k.clone(),
                _ => panic!("body Sequence"),
            },
            _ => panic!("root Repeat"),
        };
        // Expected children layout in order:
        //   [0] Push(host, w_stage1, input)         — left alone
        //   [1] Wait(host, w_stage1, input)         — left alone
        //   [2..6] feat1 rewritten to 4 hops via host
        //   [6..10] feat2 rewritten to 4 hops via host
        //   [10] Push(w_stage3, host, output)       — left alone
        //   [11] Wait(w_stage3, host, output)       — left alone
        // Total: 2 + 4 + 4 + 2 = 12.
        assert_eq!(
            kids.len(),
            12,
            "expected 2 host-endpoint pairs + 2×(4 hops) for the non-host pairs"
        );
        // Verify the input pair (children[0..2]) is unchanged.
        match (&kids[0], &kids[1]) {
            (ACFGNode::Xfer(x0), ACFGNode::Xfer(x1)) => {
                assert_eq!(x0.src, host(), "input Push src = host");
                assert_eq!(x0.dst, w_stage1, "input Push dst = w_stage1");
                assert_eq!(x0.data, input_data);
                assert_eq!(x0.seq, SeqTag(9), "input Push seq unchanged");
                assert_eq!(x1.src, host());
                assert_eq!(x1.dst, w_stage1);
                assert_eq!(x1.seq, SeqTag(9), "input Wait seq unchanged");
            }
            _ => panic!("input pair (kids[0], kids[1]) must remain as 2 Xfer nodes"),
        }
        // Verify the output pair (children[10..12]) is unchanged.
        match (&kids[10], &kids[11]) {
            (ACFGNode::Xfer(x0), ACFGNode::Xfer(x1)) => {
                assert_eq!(x0.src, w_stage3, "output Push src = w_stage3");
                assert_eq!(x0.dst, host(), "output Push dst = host");
                assert_eq!(x0.data, output_data);
                assert_eq!(x0.seq, SeqTag(12), "output Push seq unchanged");
                assert_eq!(x1.src, w_stage3);
                assert_eq!(x1.dst, host());
                assert_eq!(x1.seq, SeqTag(12), "output Wait seq unchanged");
            }
            _ => panic!("output pair (kids[10], kids[11]) must remain as 2 Xfer nodes"),
        }
        // Verify feat1's 4 hops (kids[2..6]): w_stage1 -> host -> w_stage2.
        let feat1_expected: [(XferRole, WorkerId, WorkerId); 4] = [
            (XferRole::Push, w_stage1, host()),
            (XferRole::Wait, w_stage1, host()),
            (XferRole::Push, host(), w_stage2),
            (XferRole::Wait, host(), w_stage2),
        ];
        for (i, (role, src, dst)) in feat1_expected.iter().enumerate() {
            match &kids[2 + i] {
                ACFGNode::Xfer(x) => {
                    assert_eq!(x.role, *role, "feat1 kid[{i}].role");
                    assert_eq!(x.src, *src, "feat1 kid[{i}].src");
                    assert_eq!(x.dst, *dst, "feat1 kid[{i}].dst");
                    assert_eq!(x.data, feat1_data, "feat1 kid[{i}].data");
                }
                _ => panic!("feat1 kid[{i}] must be Xfer"),
            }
        }
        // Verify feat2's 4 hops (kids[6..10]): w_stage2 -> host -> w_stage3.
        let feat2_expected: [(XferRole, WorkerId, WorkerId); 4] = [
            (XferRole::Push, w_stage2, host()),
            (XferRole::Wait, w_stage2, host()),
            (XferRole::Push, host(), w_stage3),
            (XferRole::Wait, host(), w_stage3),
        ];
        for (i, (role, src, dst)) in feat2_expected.iter().enumerate() {
            match &kids[6 + i] {
                ACFGNode::Xfer(x) => {
                    assert_eq!(x.role, *role, "feat2 kid[{i}].role");
                    assert_eq!(x.src, *src, "feat2 kid[{i}].src");
                    assert_eq!(x.dst, *dst, "feat2 kid[{i}].dst");
                    assert_eq!(x.data, feat2_data, "feat2 kid[{i}].data");
                }
                _ => panic!("feat2 kid[{i}] must be Xfer"),
            }
        }
        // Each rewritten pair must have its own fresh-seq pair, and
        // all four fresh seqs must be > max original seq (12).
        let fresh_seqs: std::collections::BTreeSet<u64> = (2..6)
            .chain(6..10)
            .filter_map(|i| match &kids[i] {
                ACFGNode::Xfer(x) => Some(x.seq.0),
                _ => None,
            })
            .collect();
        assert_eq!(
            fresh_seqs.len(),
            4,
            "exactly 4 distinct fresh seqs across 2 rewritten pairs"
        );
        for s in &fresh_seqs {
            assert!(
                *s > 12,
                "fresh seq {s} must be > max original seq 12 (per monotonic allocator)"
            );
        }
    }

    #[test]
    fn empty_acfg_passes_through() {
        let acfg = empty_acfg(ACFGNode::Sequence(vec![]));
        let out = apply_host_data_relay_inject(acfg.clone(), host());
        assert_eq!(out.root, acfg.root);
    }

    #[test]
    fn pair_with_custom_policy_preserves_policy() {
        // The original pair's TransferPolicy (sync/async, buffer N,
        // notify mode) must be cloned onto every new hop so the
        // sidecar resolves correctly. Wrap in Repeat (scope limit).
        let policy = TransferPolicy {
            synchronous: false,
            buffer: 4,
            notify: NotifyMode::Event,
        };
        let mut p = pair_xfers(w1(), w2(), d(7), 3);
        for c in p.iter_mut() {
            if let ACFGNode::Xfer(x) = c {
                x.policy = policy;
            }
        }
        let body = ACFGNode::Sequence(p);
        let root = ACFGNode::Repeat {
            iter_var: IterVar(0),
            range: 0..4,
            body: Box::new(body),
            block_tag: None,
        };
        let acfg = empty_acfg(root);
        let out = apply_host_data_relay_inject(acfg, host());
        let kids = match &out.root {
            ACFGNode::Repeat { body, .. } => match body.as_ref() {
                ACFGNode::Sequence(k) => k.clone(),
                _ => panic!("body Sequence"),
            },
            _ => panic!("root Repeat"),
        };
        for k in kids {
            match k {
                ACFGNode::Xfer(x) => {
                    assert!(!x.policy.synchronous, "policy.synchronous cloned");
                    assert_eq!(x.policy.buffer, 4, "policy.buffer cloned");
                    assert_eq!(x.policy.notify, NotifyMode::Event, "policy.notify cloned");
                }
                _ => panic!("expected Xfer"),
            }
        }
    }
}
