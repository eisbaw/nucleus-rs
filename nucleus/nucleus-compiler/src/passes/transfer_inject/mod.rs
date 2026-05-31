//! Transfer injection — TASK-0018.
//!
//! For every dataflow edge that crosses worker entities, insert a
//! matched pair of [`ACFGNode::Xfer`] placeholders carrying
//! [`XferRole::Push`] (on the producer's prior position) and
//! [`XferRole::Wait`] (immediately before the consumer). Both
//! endpoints share a fresh, unique [`SeqTag`] and the
//! [`TransferPolicy`] resolved from `LinkedIR.sched.transfers`.
//!
//! See PRD §6.3.4 (transfer directives), §8.3 (Push/Wait/seq), and
//! TASK-0018 for the contract this implements.
//!
//! Error convention (decision-0003): a cross-worker `Wait` that
//! escapes the ACFG with no producing `Operation` is a cross-pass
//! invariant violation `build_acfg` guarantees cannot occur for valid
//! IR, so this pass `panic!`s with context (the invariant side); it
//! never returns a user-facing error.
//!
//! ## Inputs and outputs
//!
//! Pure function:
//!
//! ```text
//! inject_transfers : (&LinkedIR, ACFG) -> ACFG
//! ```
//!
//! `&LinkedIR` is needed because:
//!
//! - `linked.data_producers` / `linked.data_consumers` tell us where
//!   each data symbol lives, so we know which dataflow edges cross
//!   workers.
//! - `linked.sched.transfers` tells us the policy declared in the
//!   schedule. Per the task spec, capability validation (e.g. async
//!   on a sync-only backend) is deferred — we just thread the policy
//!   through onto each [`XferPlaceholder`].
//!
//! The `ACFG` is consumed and a new ACFG is returned; the name-table
//! maps are forwarded unchanged.
//!
//! ## Algorithm
//!
//! Walk the tree recursively. Maintain a per-data "last known producer
//! worker" map; the link pass tells us each data symbol has at most
//! one producer kernel (single-assignment, PRD §6.2.1), so the
//! producer worker entity for `D` is uniquely identified by the
//! schedule's placement of that kernel.
//!
//! For every [`ACFGNode::Operation`] node O on consumer worker entity
//! `W_c` that reads data `D`:
//!
//! 1. Look up `D`'s producer worker entity `W_p` via
//!    `linked.data_producers`. If absent (e.g. top-level
//!    `load_image` result whose producer entity is the loader's
//!    placement — also recorded in `data_producers`) we skip.
//! 2. If `W_p == W_c`, nothing to inject (intra-worker transfer).
//! 3. Otherwise, allocate a fresh `SeqTag`, build a Push placeholder
//!    on the (src, dst, data, tile, policy) tuple, and a matching
//!    Wait. Insert the Wait immediately before O in O's enclosing
//!    sequence; insert the Push immediately after the producer's
//!    Operation. The producer Operation is located by a parallel
//!    walk that remembers, per data symbol, where it was last
//!    written.
//!
//! Worker entities in the link pass are `BTreeSet<String>` (a kernel
//! can be placed on multiple workers — `place k on {w0,w1,w2,w3}`).
//! The ACFG carries `BTreeSet<WorkerId>` on each Operation. Both sides
//! collapse the same way — we use the BTreeSet equality directly
//! when deciding "same entity?".
//!
//! For the `src` and `dst` fields on [`XferPlaceholder`] we need *a*
//! `WorkerId`, not a set. TASK-0117 fan-out (now landed) emits one
//! XferPlaceholder pair per (src-worker, dst-worker) member of the
//! cartesian product of the producer and consumer entities, skipping
//! same-worker pairs. Each pair carries a fresh `SeqTag` and a tile
//! that is the enclosing iteration tile with any axis named in the
//! ACFG's `partition_worker_ranges` sidecar (TASK-0212) rewritten to
//! the compute worker's slice. The {host} -> {single worker} shape
//! (M1/M2 single-distributed cases) lowers to exactly one pair — no
//! behavioural delta versus the pre-TASK-0117 canonical-collapse path
//! for those cases.
//!
//! ## Honest limitations (recorded for follow-up)
//!
//! - **N-to-M fan-out tile-rewrite convention** (the
//!   `rewrite_partition_tiles` per-Xfer rule, in the `partition`
//!   submodule). When
//!   constructing per-pair tiles for a cross-worker `Xfer`, the
//!   compute worker is `src` if `src` has a `partition_ranges` entry
//!   (the N-to-1 gather direction, e.g. an output), otherwise `dst` if
//!   `dst` has one (the 1-to-N broadcast direction, e.g. an input),
//!   otherwise neither (the 1-to-1 host↔single-worker shape — no
//!   partition involvement, tile left unchanged). The in-tree
//!   schedules exercise only the 1-to-N and N-to-1 shapes, never a
//!   true N-to-M coordinate mapping where both sides are partitioned
//!   on the same axis with non-aligned slices. A coordinate-mapping
//!   policy for true N-to-M (both partitioned, slices need explicit
//!   mapping) is a follow-up not blocked by any current example.
//!   IMPORTANT: this paragraph describes the per-pair tile-rewrite
//!   for the CROSS-WORKER case where the pair was emitted (the
//!   same-set short-circuit did NOT fire). The same-set elision case is
//!   a separate path described in the next bullet.
//!
//! - **Same-worker-set producer/consumer with the consumer reading
//!   outside its own produce-tile** (TASK-0324 cycle-144).
//!   `build_waits_for_op` short-circuits with `continue; no transfer`
//!   when `producer_workers == consumer_workers` on a `BTreeSet`-
//!   equality test (grep-witness anchor: `if producer_workers ==
//!   &consumer_workers` inside `build_waits_for_op`). The elision is
//!   CORRECTNESS-SAFE
//!   when the consumer's read indices stay within the slice the local
//!   worker produced (e.g. 13-cnn-inference/batch_parallel:
//!   `feat1[n] <-- conv_block_1(input[n])` with `loop n :
//!   partition=workers` — reader iv `n` IS the partition iv, so each
//!   worker reads only the slice it wrote). It is a SILENT MISCOMPILE
//!   when the consumer's read indices reach OUTSIDE the local
//!   producer's slice (e.g. 06-separable-filter/distributed2:
//!   `out[vy][vx] <-- vblur_acc(vy, vx, tmp[vm][vx])` with `loop vy :
//!   partition=rows` — reader iv `vm` is the inner pass-2 sweep, NOT
//!   the partition iv `vy`, so each consumer reads ALL rows of `tmp`
//!   while owning only its `hy` row-band from pass 1).
//!
//!   AC#2 of TASK-0324 (cycle 144) added a front-loaded diagnose-first
//!   guard (`check_no_silent_elision_risk`) that walks every Operation
//!   BEFORE the emission recursion. The correctness-safe shape (every
//!   consumer read index is a bare `Ident(X)` where X is the partition
//!   iv on the consumer's enclosing scope) was allowed through; the
//!   unsafe shape rejected with a typed
//!   `TransferInjectError::SameSetSilentElisionRisk`.
//!
//!   AC#3 of TASK-0324 (cycle 147) lifts the same-set rejection. The
//!   validator now classifies but does NOT reject the same-set unsafe
//!   shape; `build_waits_for_op`'s same-set short-circuit (grep-
//!   witness: `if producer_workers == &consumer_workers`) classifies
//!   with the SAME predicate (`same_set_elision_unsafe_reason`) and
//!   falls through to the cartesian-product fan-out below when
//!   unsafe — emitting N*(N-1) cross-worker pairs. Each producer w_i
//!   pushes its partition slice (compute_worker = src per
//!   `rewrite_partition_tiles`'s N-to-1 gather rule); each consumer
//!   w_j receives (N-1) cross-pushes covering the other producers'
//!   slices; w_j's own slice is filled by local production. The
//!   receiver-side `render_wait_assign`'s `WaitSlice::Rows` path
//!   composes the row-bands into the full data. The
//!   06-separable-filter/distributed2 schedule is the in-tree
//!   reproducer this lift unblocks.
//!
//!   The partial-overlap case (`producer_workers !=
//!   &consumer_workers` but non-empty intersection — the per-element
//!   `if src == dst` skip in the cartesian fan-out, grep-witness:
//!   `if src == dst` inside `build_waits_for_op`) is NOT in cycle
//!   147's AC#3 scope. The validator still rejects unsafe shapes
//!   there. No in-tree schedule exercises it; a future cycle that
//!   adds one extends AC#3 to that path too.
//!
//!   TASK-0325 (cycle 145) generalised the guard from a set-equality
//!   test on the worker sets to a non-empty-intersection test. The
//!   per-element `if src == dst { continue; }` inside the cartesian-
//!   product fan-out (grep-witness anchor: `if src == dst` inside
//!   `build_waits_for_op`) is the structurally identical sibling of
//!   the `producer_workers == &consumer_workers` whole-set short-
//!   circuit (grep-witness anchor: `if producer_workers ==
//!   &consumer_workers` inside `build_waits_for_op`): it elides one
//!   transfer per same-worker pair in the intersection of
//!   `producer_workers` and `consumer_workers`. Under partial overlap
//!   (e.g. producer={w0..w3}, consumer={w0..w3, w4}) the same per-
//!   axis safety conditions apply to each self-pair, so the validator
//!   covers both elision sites with one check. No in-tree schedule
//!   exercises partial overlap today — the partial-overlap arm is
//!   dormant-but-defended.
//!
//!   TASK-0328 (cycle 154) removed an over-lenient "no partition iv
//!   active on consumer scope → safe elision" short-circuit (formerly
//!   clause (1)) from BOTH the validator and the emit-site mirror in
//!   `build_waits_for_op`. The reasoning "every worker owns the full
//!   data" conflated the consumer's enclosing scope with what each
//!   worker has in memory: a partition-sliced producer leaves each
//!   worker with only its band, regardless of whether the consumer's
//!   enclosing scope has a partition iv. With clause (1) at the emit
//!   site, the cartesian-product fan-out NEVER fired for the
//!   consumer-at-top-level case — silent miscompile. Removal makes
//!   the per-axis discriminator (`same_set_elision_unsafe_reason`)
//!   the single load-bearing classifier for both sites. Test pins:
//!   `task0328_ac2_positive_partition_producer_topfile_consumer`
//!   (consumer at top-level reading constant indices → emit must
//!   produce 12 cross-worker pairs); `task0328_ac2_negative_no_
//!   partition_anywhere` (no partition anywhere → elision stays
//!   correct, 0 cross-worker pairs).
//!
//!   What this pass does NOT yet emit (deferred to AC#3): the
//!   actual cross-worker `tmp` transfers for the unsafe shape — each
//!   producer w_i pushes its slice to every consumer w_j, each
//!   consumer assembles. Until AC#3 lands, the unsafe shape is a
//!   typed error rather than a wrong-output silent miscompile.
//!
//! - **Per-point granularity outside `block=`.** When the consumer
//!   is inside a non-blocked `for`, we still fire one Push/Wait per
//!   *iteration* (per Operation visit). The producer for an
//!   `outer <-- load_input()` placed on `host` therefore gets *one*
//!   Push for every consuming iteration that crosses worker, even
//!   though a real backend would aggregate into one bulk send. The
//!   IterTile we record is the consumer Operation's enclosing-loop
//!   tile. General tile-coalescing across non-`block=` loops remains
//!   a follow-up.
//!
//! - **Per-tile hoist for `block=N`.** When the consumer sits inside
//!   a `block`-inner intra-tile Repeat (marked by
//!   [`ACFG::inner_block_iter_vars`] from
//!   [`crate::passes::block_transform`]), any Wait whose data was
//!   NOT produced in the same intra-tile sequence is hoisted up
//!   through every enclosing `block`-inner Repeat to the nearest
//!   per-tile body sequence and emitted there. The matching Push is
//!   spliced after the producer Operation at *the level the
//!   producer lives at* — same scoping rule the pass has used since
//!   M1. The hoisted Wait's `IterTile` is rewritten to the
//!   destination sequence's enclosing tile so the per-tile semantic
//!   (PRD §6.3.3, "transfers happen per tile") shows up on the
//!   placeholder. TASK-0143.
//!
//!   The hoist is loop-invariance-by-structure, not by index
//!   analysis: any data that crosses a worker boundary and isn't
//!   produced inside the intra-tile body is treated as invariant
//!   w.r.t. the intra-tile iteration. As of TASK-0150 the ACFG
//!   layer *does* now carry per-firing index expressions
//!   (`DataflowEdge::data_in_access` / `data_out_access`, see
//!   `acfg.rs`) — the data needed for a precise per-tile / halo-band
//!   check is present. This pass deliberately does **not** consume
//!   it yet: the precise check only changes behaviour once data is
//!   *partitioned* across workers (TASK-0117 distributed
//!   placement), and synthesising halo strips on top of the
//!   structural hoist is its own substantial pass. So TASK-0150
//!   plumbs the data and stops; the consumer is TASK-0158 (filed),
//!   coupled to TASK-0117. Until then the structural hoist remains
//!   the behaviour — conservatively safe (it can only over-transfer
//!   a full tile where a halo strip would suffice, never
//!   under-synchronise).
//!
//! - **Block-entangled non-block transfers — per-Wait classification
//!   (TASK-0267).** Pass A / Pass B used to treat any Repeat whose
//!   subtree contained a `block`-inner loop as opaque (the original
//!   TASK-0151 over-approximation). The conservative gate predates
//!   TASK-0263's halo-aware tile rewrite; once tiles carry the
//!   partition slice (`rewrite_partition_tiles`) + halo widths
//!   (`extend_xfer_tiles_for_halo`), the per-Wait stay-vs-bubble logic
//!   already in the recursive arms is sufficient. The rule is now
//!   per-Wait: a Wait whose `data` is produced INSIDE the enclosing
//!   Repeat's subtree stays inside (per-iteration rendezvous); a Wait
//!   whose `data` is produced OUTSIDE bubbles up and gets a
//!   whole-symbol Push spliced after the producer, regardless of
//!   whether some sibling block-inner loop coexists. This unblocks
//!   the M5 stencil/distributed shape (host-loaded `img_in`,
//!   partition=rows + inner `block=N`): the row-band slice carried in
//!   `tile` is the right amount to transfer, the inner block tile
//!   does not change what `img_in` data the worker needs, so hoisting
//!   the Wait past the block-governed outer Repeat is correct. Pinned
//!   by `block_nested_in_plain_loop_pairs_the_invariant_wait` and
//!   `mixed_block_and_nonblock_program_pairs_the_nonblock_transfer`.
//!   A future per-block-tile slice (the original TASK-0149/TASK-0158
//!   territory) — where a Repeat iteration genuinely needs a
//!   different sub-region per tile — is not exercised today by any
//!   shipped schedule; when it lands, `extend_xfer_tiles_for_halo`
//!   would need a per-tile counterpart or the Wait would need to
//!   refuse the hoist by inspecting `dataflow_edge.data_in_access`.
//!
//! - **Idempotence by structural skip.** Re-running the pass detects
//!   that a Wait already precedes the consumer Operation (and a Push
//!   already follows the producer Operation) by checking sibling
//!   `Xfer` nodes carrying matching structural keys. The pass does
//!   NOT re-derive `seq` to be the same — the original placeholder
//!   is left in place. Three dedup sites exist; two use the full
//!   `(src, dst, data, tile)` key, one omits `tile`. The divergence
//!   is intentional and is governed by **dedup-set composition**,
//!   NOT by whether the tile was rewritten before the check:
//!
//!     - `inject_in_sequence` (Wait dedup, hoisted-drain site): keys
//!       on full `(role, src, dst, data, tile)`. Delegated to
//!       `is_duplicate_xfer_in_epoch(&out, &w)` so the dedup-set is
//!       the current Sequence's `out` scanned from the end backward,
//!       stopping at the first `ACFGNode::Sync`. A barrier marks a
//!       fresh rendezvous epoch where a hoisted-drain Wait is
//!       legitimate (different consumer phase, different buffer
//!       place); the candidate's role is always Wait, which the
//!       helper checks. (TASK-0335.01 cycle 159 widened this from a
//!       whole-`out` `(src, dst, data, tile)` scan — the pre-cycle
//!       form would silently suppress a legitimate cross-epoch
//!       hoist-drain.)
//!     - `inject_in_sequence` (Wait dedup, per-Op emission site):
//!       keys on full `(role, src, dst, data, tile)`. The dedup-set
//!       is the current Sequence's `out: Vec<ACFGNode>` scanned
//!       from the end backward, stopping at the first
//!       `ACFGNode::Sync` — sync_inject runs BEFORE transfer_inject
//!       and barriers mark fresh rendezvous epochs where duplicate
//!       Waits are legitimate (different consumer phase, different
//!       buffer place). Delegated to the same helper
//!       `is_duplicate_xfer_in_epoch(&out, &w)` (single source of
//!       truth for "duplicate within an epoch"; see grep witness
//!       below).
//!
//!       Cycle-158 (TASK-0335) widened this scope from
//!       `is_duplicate_xfer(out.last(), …)` to the epoch-scoped
//!       scan. The narrower form only fired when the candidate
//!       duplicated the immediately-preceding element;
//!       multi-consumer-Op shapes (e.g. 03-reduction/distributed's
//!       two host-side `combine` Operations both reading
//!       `partials`) put an Operation between the two Wait-bursts,
//!       breaking the immediately-preceding check. The result was
//!       N duplicate Waits → `splice_pushes_global` emitted N
//!       duplicate Pushes → producer-side splice-at-same-index
//!       ordering inverted → mp-tcp-bufsync wire FIFO seq mismatch
//!       at runtime.
//!     - `splice_pushes_for_waits` (Push dedup): keys on full
//!       `(src, dst, data, tile)`. The dedup-set is the single
//!       `out[insert_at]` slot immediately following the producer.
//!       The tile distinguishes Pushes for distinct partition
//!       sub-regions targeting the same consumer.
//!     - `hoist_invariant_waits` (Wait dedup, `place_or_bubble`):
//!       delegated to `is_duplicate_xfer_in_epoch(out, &w)` since
//!       TASK-0335.02 cycle 159. The helper's 5-tuple key includes
//!       `tile`, which is redundant-by-construction here (every
//!       candidate AND every member of `out` is rewritten to
//!       `IterTile::new(enclosing_tile.to_vec())` on the immediately
//!       preceding line — see place_or_bubble closure body), so the
//!       redundant tile comparison costs one extra equality per
//!       element but keeps a single source of truth for "duplicate
//!       within an epoch". The pre-cycle-159 form was a whole-`out`
//!       3-tuple `(src, dst, data)` scan WITHOUT Sync-stop —
//!       strictly more aggressive than cycle-158's inline site, and
//!       would have silently suppressed a legitimate
//!       far-side-of-Sync hoist target (LATENT in-tree at cycle
//!       159; widened defensively).
//!
//!   Grep witness for the LITERAL-XferRole dedup sites
//!   (function-name anchors are the stable index; line numbers are
//!   an as-of-edit stamp, re-run the grep if they have drifted):
//!   `grep -nE 'existing\.role == XferRole::|x\.role == XferRole::' transfer_inject.rs`
//!   yields exactly 7 matches as of cycle 159. The ONE DEDUP-CHECK
//!   match (its `matches!` / `if`-chain continues with
//!   `&& src == … && dst == … && data == …`) is:
//!     - in `splice_pushes_for_waits`: the `XferRole::Push` if-chain
//!       (full 4-tuple including `tile`).
//!
//!   The three OTHER dedup sites — `inject_in_sequence`'s per-Op
//!   Wait emission (cycle 158), `inject_in_sequence`'s hoisted-drain
//!   (cycle 159, TASK-0335.01), and `hoist_invariant_waits`'s
//!   `place_or_bubble` (cycle 159, TASK-0335.02) — are all delegated
//!   to `is_duplicate_xfer_in_epoch(out, cand)` whose own role check
//!   uses `existing.role == cand.role` (no literal `XferRole::`), so
//!   they do NOT appear in the literal-pattern grep witness.
//!
//!   The other grep matches are role-scans for unrelated purposes
//!   (e.g. counting Waits, filtering Push nodes during splice) and
//!   are NOT dedup checks. The only literal-`XferRole::` dedup-check
//!   is `splice_pushes_for_waits`'s Push if-chain (now in the
//!   `sequence` submodule after the TASK-0340.13 split). The three
//!   cycle-159 Wait dedup sites (`inject_in_sequence`'s per-Op emit
//!   and hoisted drain, and `hoist_invariant_waits`'s `place_or_bubble`)
//!   delegate to one of the two `is_duplicate_xfer_in_epoch{,_at_slot}`
//!   helpers (now in the `dedup` submodule) and so do NOT appear in the
//!   literal-XferRole grep witness. (Per-line stamps were removed in the
//!   TASK-0340.13 split — they indexed the former single-file layout.)
//!
//!   Tests cover the cross-site invariant: see
//!   `idempotent_on_synthetic_two_worker_case` in
//!   `tests/transfer_inject.rs`, and `hoisting_is_idempotent` /
//!   `mixed_block_nonblock_tree_is_structurally_idempotent` /
//!   `whole_symbol_finalisation_is_structurally_idempotent` in
//!   `tests/transfer_inject_hoist.rs`.
//!
//! - **Conflict detection happens upstream at sched-lower.** A schedule
//!   writing `transfer D : sync, async;` is rejected by `lower_transfer`
//!   in `crate::sched::lower` as `SchedLowerErrorKind::ConflictingTransferMode`
//!   (TASK-0193 / TASK-0119) BEFORE this pass runs. So the
//!   `policy_from_directive` helper here is reached only with at most
//!   one mode flag set — the last-option-wins shape is a legacy of when
//!   the conflict check lived only in this pass; the upstream reject
//!   makes it dead-code defensive. Pinned by
//!   `negative_mutually_exclusive_transfer_sync_async` in
//!   `nucleus-compiler/tests/sched_lower.rs`.
//!
//! - **Capability check is the driver's job, not this pass's.** The
//!   backend isn't chosen at this point, so this pass carries the
//!   schedule's stated policy onto every placeholder unchanged. The
//!   driver's capability gate (`check_schedule_compat`, TASK-0019,
//!   Done) rejects `async`/`buffer>1`/`notify=event` against backends
//!   whose `capabilities.toml` doesn't list them, once the backend is
//!   chosen (before codegen).

use std::collections::{BTreeMap, BTreeSet};

use std::num::NonZeroU64;

use crate::acfg::{
    ACFGNode, DataAccess, NotifyMode, Operation, TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
use crate::algo::ir::IrExpr;
use crate::event::{DataId, IterTile, IterVar, KernelId, SeqTag, WorkerId};
use crate::link::{LinkedIR, WorkerEntity};
// TASK-0373: shared data-dependence predicate (single source of truth
// with halo_inference) — a dim whose index is a gather (`x[col[k]]`) is
// recorded OPAQUE so the conservative whole-array broadcast serves it.
use crate::passes::common::expr_contains_dataref_or_call;
use crate::sched::{ResolvedLoopOption, ResolvedTransferDirective, ResolvedTransferOption};

// --------------------------------------------------------------------
// Error surface
// --------------------------------------------------------------------

/// Typed error from [`inject_transfers`]. Mirrors the structural-error
/// convention used by [`crate::passes::halo_inference`] and
/// [`crate::passes::reuse_inference`] (precedent: those passes already
/// return `Result<ACFG, _>`).
///
/// Variants here name patterns transfer-injection currently CANNOT
/// lower; the carried message must be diagnosis-quality (name the
/// offending DataId, the failure shape, and the forward-link).
///
/// Marked `#[non_exhaustive]` so future variants (e.g. once
/// TASK-0324 AC#3 lifts the SameSetSilentElisionRisk variant or
/// adds a sibling for the partially-overlapping-worker-set case,
/// or new typed errors land for other transfer-inject limitations)
/// can be added without breaking downstream `match` exhaustiveness.
/// Reviewer P2.2 cycle-144 fold-back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransferInjectError {
    /// TASK-0324 cycle-144 + TASK-0325 cycle-145 fail-loud guard.
    /// Producer and consumer worker sets share at least one common
    /// worker (`BTreeSet` intersection non-empty — covers both the
    /// `producer_workers == &consumer_workers` short-circuit inside
    /// `build_waits_for_op` AND the per-element `if src == dst { continue; }`
    /// skip inside the cartesian-product fan-out in the same function)
    /// AND the consumer's read indices reach OUTSIDE the local
    /// producer's partition slice — i.e. the elision the existing
    /// `continue; no transfer` would have performed is a silent
    /// miscompile.
    ///
    /// The discriminator's safe escape valve (where the elision IS
    /// correct) is when every read index for this data is a bare
    /// `IrExpr::Ident(X)` whose `X` is the partition iv on the
    /// consumer's enclosing scope — i.e. the consumer iterates over
    /// exactly the same partition the producer wrote (the 13-cnn-
    /// inference/batch_parallel `feat1[n]` with `loop n :
    /// partition=workers` shape).
    ///
    /// The cross-worker codegen extension that lifts this rejection is
    /// AC#3 of TASK-0324 (N-to-N broadcast-of-gather).
    SameSetSilentElisionRisk {
        /// The data symbol whose elision was rejected.
        data: DataId,
        /// Diagnosis-quality message (DataId + reason + TASK-0324
        /// forward-link).
        message: String,
    },
    /// TASK-0366 cycle-214 fold-back (cycle-213 architect P3). A
    /// CUMULATIVE data symbol (a `NameSidecar::cumulative_data` member)
    /// has an `Xfer` for which [`cumulative_band_bounds`] returned
    /// `None` (no partitioned iv covers any data dim on this `src` →
    /// `saw_band == false`) WHILE A PARTITION IS ACTIVE
    /// (`partition_ranges` is non-empty).
    ///
    /// This is a CONSERVATIVE tripwire, not a precise per-array
    /// diagnosis: the guard proves only that *some* partition is active
    /// *somewhere in this pass invocation* AND no band covers *this*
    /// cumulative array — it does NOT itself prove the array is
    /// replicated across the partition workers. For every shipped
    /// schedule today the only way to reach it WOULD be the
    /// replicated-across-workers shape (where the host gather / w2w
    /// exchange of N identical full slices re-introduces the xN
    /// double-count `rewrite_cumulative_band_tiles` exists to remove),
    /// so failing loud here is strictly safer than silently keeping the
    /// whole-array tile. A future program could in principle reach it
    /// with a single-worker cumulative array sitting alongside an
    /// unrelated partitioned array; that case is over-rejected on
    /// purpose (it forces the write-band derivation to be extended
    /// rather than shipping a possibly-xN-wrong tile).
    ///
    /// This is distinct from the UNPARTITIONED cumulative case
    /// (`partition_ranges` EMPTY, e.g. 11-game-of-life/pipelined: a
    /// single compute worker owns all of `grid`, transferred whole over
    /// the async double-buffer channel) — there the whole-array tile is
    /// CORRECT and the pass keeps it silently, never reaching this
    /// variant. The guard fires only on the partitioned-replicated
    /// shape, which is provably dead for every shipped schedule today
    /// (16-jacobi `field` always derives a 3-dim iv map with a
    /// partitioned y-band on every compute-worker `src`). Reaching this
    /// variant means a NEW partitioned-cumulative shape has hit the
    /// gap — fail at compile time rather than emit xN-wrong output.
    CumulativeWholeArrayFallback {
        /// The cumulative data symbol whose band tile could not be
        /// derived.
        data: DataId,
        /// The transfer's `src` (the band-owning compute worker for a
        /// cumulative gather / w2w-send).
        src: WorkerId,
        /// Diagnosis-quality message (DataId + src + xN-risk reason +
        /// TASK-0366 forward-link).
        message: String,
    },
}

impl std::fmt::Display for TransferInjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferInjectError::SameSetSilentElisionRisk { message, .. } => {
                write!(f, "transfer_inject silent-elision risk: {message}")
            }
            TransferInjectError::CumulativeWholeArrayFallback { message, .. } => {
                write!(
                    f,
                    "transfer_inject cumulative whole-array fallback: {message}"
                )
            }
        }
    }
}

impl std::error::Error for TransferInjectError {}

// --------------------------------------------------------------------
// Context and mutable state
// --------------------------------------------------------------------

/// Immutable inputs threaded through the walk.
struct InjectCtx<'a> {
    /// Producer worker set for each data symbol.
    producers_by_data: &'a BTreeMap<DataId, BTreeSet<WorkerId>>,
    /// Transfer policy for each data symbol that the schedule named.
    policies_by_data: &'a BTreeMap<DataId, TransferPolicy>,
    /// Iter-var IDs that the block-transform pass marked as inner
    /// (intra-tile) loops. Push/Wait pairs emitted inside one of
    /// these get hoisted to the enclosing per-tile body — TASK-0143.
    inner_block_iter_vars: &'a BTreeSet<IterVar>,
    /// Iter-vars partitioned across worker sets. Used by
    /// `build_waits_for_op`'s same-set short-circuit (TASK-0324 AC#3)
    /// to decide whether the existing `continue; no transfer` elision
    /// is safe for this (op, data) pair — same predicate the AC#2
    /// validator uses.
    partition_iter_vars: &'a BTreeSet<IterVar>,
    /// Name -> IterVar map (link-pass output). Threaded for the
    /// same AC#3 predicate.
    name_iter_vars: &'a BTreeMap<String, IterVar>,
    /// Per-data producer-side write access pattern (single-assignment
    /// invariant, PRD §6.2.1; first lexical occurrence wins for
    /// accumulator self-writes). Threaded for the AC#3 predicate.
    producer_writes: &'a BTreeMap<DataId, DataAccess>,
    /// Producer-statement RANK per data symbol (TASK-0389): the
    /// depth-first walk position of each symbol's producing Operation,
    /// i.e. the order `splice_pushes_global` sends the host's Pushes.
    /// `build_waits_for_op` sorts each worker's per-channel Wait
    /// sequence by this rank so the receiver reads in the host's send
    /// order on a strict-FIFO channel, for ANY declaration order.
    producer_rank: &'a BTreeMap<DataId, usize>,
}

/// Mutable state threaded through the walk.
struct State {
    /// Next SeqTag to hand out.
    next_seq: u64,
    /// For every data symbol that has been written so far in the walk,
    /// the worker set that wrote it. Updated as we encounter
    /// `Operation`s that produce data. Used to confirm the consumer's
    /// view of where data came from (a sanity tie-back against
    /// `linked.data_producers`; the two should agree).
    last_writer: BTreeMap<DataId, BTreeSet<WorkerId>>,
}

impl State {
    fn fresh_seq(&mut self) -> SeqTag {
        let s = SeqTag(self.next_seq);
        self.next_seq += 1;
        s
    }
}

/// Per-tile hoist sink: collected Wait placeholders that the inner
/// block-loop body would have emitted, plus a record of producer
/// indices observed at the parent (per-tile) sequence so the matching
/// Push can be spliced after the right Operation.
///
/// One sink instance is created by `inject_in_sequence` whenever it
/// is about to recurse into a `block`-inner `Repeat` child; the sink
/// is then drained back into the parent sequence's `out` vector and
/// the parent's local-producer index map, around the inner Repeat
/// node.
#[derive(Default)]
struct HoistSink {
    /// Waits generated inside the inner block-loop, to be placed in
    /// the parent (per-tile body) sequence *before* the inner
    /// `Repeat` node. Idempotence dedup happens at drain time.
    waits: Vec<XferPlaceholder>,
}

// --------------------------------------------------------------------
// Inspection helpers (used by tests)
// --------------------------------------------------------------------

impl ACFGNode {
    /// Count [`ACFGNode::Xfer`] nodes anywhere in this subtree.
    /// Sister to `count_operations` / `count_syncs`.
    pub fn count_xfers(&self) -> usize {
        match self {
            ACFGNode::Xfer(_) => 1,
            ACFGNode::Repeat { body, .. } => body.count_xfers(),
            ACFGNode::Sequence(children) => children.iter().map(ACFGNode::count_xfers).sum(),
            ACFGNode::Operation(_) | ACFGNode::Sync(_) => 0,
        }
    }

    /// Count [`ACFGNode::Xfer`] nodes of a specific role.
    pub fn count_xfers_role(&self, role: XferRole) -> usize {
        match self {
            ACFGNode::Xfer(x) => {
                if x.role == role {
                    1
                } else {
                    0
                }
            }
            ACFGNode::Repeat { body, .. } => body.count_xfers_role(role),
            ACFGNode::Sequence(children) => children.iter().map(|c| c.count_xfers_role(role)).sum(),
            ACFGNode::Operation(_) | ACFGNode::Sync(_) => 0,
        }
    }

    /// Collect every [`XferPlaceholder`] in source order.
    pub fn collect_xfers(&self) -> Vec<XferPlaceholder> {
        let mut out = Vec::new();
        self.collect_xfers_into(&mut out);
        out
    }

    fn collect_xfers_into(&self, out: &mut Vec<XferPlaceholder>) {
        match self {
            ACFGNode::Xfer(x) => out.push(x.clone()),
            ACFGNode::Repeat { body, .. } => body.collect_xfers_into(out),
            ACFGNode::Sequence(children) => {
                for c in children {
                    c.collect_xfers_into(out);
                }
            }
            ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        }
    }
}

impl ACFG {
    /// Total Xfer count across the whole ACFG.
    pub fn xfer_count(&self) -> usize {
        self.root.count_xfers()
    }

    /// Total Push count across the whole ACFG.
    pub fn push_count(&self) -> usize {
        self.root.count_xfers_role(XferRole::Push)
    }

    /// Total Wait count across the whole ACFG.
    pub fn wait_count(&self) -> usize {
        self.root.count_xfers_role(XferRole::Wait)
    }
}

// --------------------------------------------------------------------
// Submodule decomposition (TASK-0340.13). The pass was a single 6389-
// LoC file; it is carved into cohesive submodules along the seams named
// in the module docs above. Shared walk-context types (`InjectCtx`,
// `State`, `HoistSink`) stay in this parent module so every submodule
// (a descendant) can reach their private fields without `pub(super)`
// markings; only cross-module *free functions* are `pub(super)`. The
// FIFO host-Push/worker-Wait ordering cluster is kept together in
// `ordering` (TASK-0389.01 forward-carry: splitting it would scatter the
// single invariant `host Push textual order == producer_rank order ==
// worker Wait order`).
// --------------------------------------------------------------------
mod dedup;
mod elision;
mod halo_strip;
mod inject;
mod ordering;
mod partition;
mod sequence;
mod tiles;

pub use inject::inject_transfers;

use dedup::*;
use elision::*;
use halo_strip::*;
use inject::*;
use ordering::*;
use partition::*;
use sequence::*;
use tiles::*;

#[cfg(test)]
mod tests_ordering;
#[cfg(test)]
mod tests_tiles;
