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
//!   `rewrite_partition_tiles` per-Xfer rule, lines 1689-1698). When
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
//!   for the CROSS-WORKER case where the pair was emitted (the line-
//!   2501 short-circuit did NOT fire). The same-set elision case is
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
//!   BEFORE the emission recursion and rejects the unsafe shape with
//!   a typed `TransferInjectError::SameSetSilentElisionRisk`. The
//!   correctness-safe shape (every consumer read index is a bare
//!   `Ident(X)` where X is the partition iv on the consumer's
//!   enclosing scope) is allowed through; everything else (non-Ident
//!   indices, non-partition ivs, empty indices on a partitioned-data
//!   read) is conservatively rejected pending AC#3's cross-worker
//!   codegen extension (the N-to-N broadcast-of-gather shape).
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
//!     - `inject_in_sequence` (Wait dedup): keys on full
//!       `(src, dst, data, tile)`. The dedup-set is the entire
//!       enclosing `out: Vec<ACFGNode>` and may include
//!       previously-emitted Waits that carry tile granularities
//!       from EARLIER iterations (not all rewritten to this
//!       sequence's enclosing tile). The `tile` component is
//!       therefore part of the identity even though the inserting
//!       site rewrites the candidate to the enclosing tile just
//!       before the check.
//!     - `splice_pushes_for_waits` (Push dedup): keys on full
//!       `(src, dst, data, tile)`. The dedup-set is the single
//!       `out[insert_at]` slot immediately following the producer.
//!       The tile distinguishes Pushes for distinct partition
//!       sub-regions targeting the same consumer.
//!     - `hoist_invariant_waits` (Wait dedup, `place_or_bubble`):
//!       keys on `(src, dst, data)` only. The dedup-set is the
//!       local `out: Vec<ACFGNode>` of the current `place_or_bubble`
//!       scope; every candidate is rewritten to
//!       `IterTile::new(enclosing_tile.to_vec())` on the immediately
//!       preceding line, so every member of the dedup-set carries
//!       the SAME tile by construction. Including `tile` in the key
//!       would be redundant.
//!
//!   Grep witness for the three dedup sites (function-name anchors
//!   are the stable index; line numbers are an as-of-edit stamp,
//!   re-run the grep if they have drifted):
//!   `grep -nE 'existing\.role == XferRole::|x\.role == XferRole::' transfer_inject.rs`
//!   yields exactly 9 matches. The three DEDUP-CHECK matches are the
//!   ones whose `matches!` / `if`-chain continues with
//!   `&& src == … && dst == … && data == …`:
//!     - in `inject_in_sequence`: the `XferRole::Wait` match-arm
//!       (full 4-tuple including `tile`).
//!     - in `splice_pushes_for_waits`: the `XferRole::Push` if-chain
//!       (full 4-tuple including `tile`).
//!     - in `hoist_invariant_waits::place_or_bubble`: the
//!       `XferRole::Wait` match-arm (3-tuple, NO `tile`).
//!
//!   The other 6 grep matches are role-scans for unrelated purposes
//!   (e.g. counting Waits, filtering Push nodes during splice) and
//!   are NOT dedup checks. As-of-cycle-138 line stamp: dedup checks
//!   at 944 / 1053 / 1227; role-scans at 996 / 1022 / 1190 / 1334 /
//!   1424 / 1445.
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
//! - **Capability check deferred.** Per the task spec, the backend
//!   isn't chosen at this point. We carry the schedule's stated
//!   policy onto every placeholder; codegen-time TASK-0019+ rejects
//!   `async`/`buffer>1`/`notify=event` against backends whose
//!   `capabilities.toml` doesn't list them.

use std::collections::{BTreeMap, BTreeSet};

use std::num::NonZeroU64;

use crate::acfg::{
    ACFGNode, DataAccess, NotifyMode, Operation, TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
use crate::algo::ir::IrExpr;
use crate::event::{DataId, IterTile, IterVar, KernelId, SeqTag, WorkerId};
use crate::link::{LinkedIR, WorkerEntity};
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
}

impl std::fmt::Display for TransferInjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferInjectError::SameSetSilentElisionRisk { message, .. } => {
                write!(f, "transfer_inject silent-elision risk: {message}")
            }
        }
    }
}

impl std::error::Error for TransferInjectError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Inject [`ACFGNode::Xfer`] placeholders into `acfg` per the rules in
/// the module docs. The original name-table maps on the [`ACFG`] are
/// forwarded unchanged.
///
/// Pure: same `(linked, acfg)` produces the same output across runs
/// (BTreeMap-based iteration order and a deterministic `SeqTag`
/// counter make this so).
///
/// Idempotent: re-running on the output yields the same tree
/// structurally. See `tests/transfer_inject.rs`.
pub fn inject_transfers(linked: &LinkedIR, acfg: ACFG) -> Result<ACFG, TransferInjectError> {
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        // Forward through pre-existing pipeline depths. In the
        // standard driver pipeline this is always empty here (we are
        // the populator), but tests may inject their own values
        // before calling us — preserve them.
        pipeline_depth_for_seq: pre_existing_pipeline_depth,
        // TASK-0263 Stage 2: transfer_inject consumes halo_widths to
        // extend per-tile transfer ranges by the inferred stencil
        // halo widths. The sidecar is read below at the
        // `extend_xfer_tiles_for_halo` step and forwarded verbatim
        // afterwards — it remains the single source of truth (the
        // pass does not rewrite or invalidate halo entries; it only
        // applies them to Xfer tiles).
        halo_widths,
        // TASK-0261: transfer_inject currently DOES NOT read the
        // reuse-widths sidecar — Stage 2 (TASK-0265, backend walker
        // delay-line emit) is the wiring cycle. Forward verbatim.
        reuse_widths,
        // TASK-0264 cycle 113: transfer_inject consumes partition_pairs +
        // grid_shape_for_outer_iv (populated by partition_blocks2d) as
        // of TASK-0289 cycle 114a — see `inject_halo_strip_xfers` at the
        // tail of the finalisation chain below. Forward verbatim into
        // the returned ACFG: the consumer is read-only and never
        // mutates either map.
        partition_pairs,
        grid_shape_for_outer_iv,
    } = acfg;

    // Resolve the link pass's `WorkerEntity` (BTreeSet<String>) to a
    // `BTreeSet<WorkerId>` once, keyed by data symbol name. This is
    // the producer-side worker entity per data symbol.
    let producers_by_data: BTreeMap<DataId, BTreeSet<WorkerId>> = linked
        .data_producers
        .iter()
        .filter_map(|(data_name, entity)| {
            let data_id = name_data.get(data_name).copied()?;
            let workers = entity_to_workerid_set(entity, &name_workers);
            Some((data_id, workers))
        })
        .collect();

    // Resolve the schedule's transfer directives, keyed by DataId.
    // The schedule's `transfers` is keyed by data NAME; we translate
    // once.
    let policies_by_data: BTreeMap<DataId, TransferPolicy> = linked
        .sched
        .transfers
        .iter()
        .filter_map(|(data_name, dir)| {
            let data_id = name_data.get(data_name).copied()?;
            Some((data_id, policy_from_directive(dir)))
        })
        .collect();

    // Resolve `loop VAR : pipeline=D` directives into a per-IterVar
    // map (TASK-0134). The schedule's `loops` is keyed by var NAME;
    // we translate once via `name_iter_vars`. Loops not in
    // `name_iter_vars` (e.g. a `loop x : pipeline=N` referencing a
    // var the algorithm doesn't have) are already a link-step error
    // (LinkErrorKind::UnknownLoop), so we silently skip them here —
    // build_acfg will not have reached us if link rejected.
    let pipeline_depth_for_iter_var: BTreeMap<IterVar, NonZeroU64> = linked
        .sched
        .loops
        .iter()
        .filter_map(|(var_name, dir)| {
            let iv = name_iter_vars.get(var_name).copied()?;
            // PRD §6.3.3: only `pipeline=D` lowers to an initial
            // marking; `block=`, `vectorize=`, `unroll=`, `reuse`,
            // `partition=` all have other effects handled by other
            // passes.
            let depth = dir.options.iter().find_map(|opt| match opt {
                ResolvedLoopOption::Pipeline(d) => NonZeroU64::new(*d),
                _ => None,
            })?;
            Some((iv, depth))
        })
        .collect();

    let ctx = InjectCtx {
        producers_by_data: &producers_by_data,
        policies_by_data: &policies_by_data,
        inner_block_iter_vars: &inner_block_iter_vars,
    };

    // TASK-0324 cycle-144 AC#2 + TASK-0325 cycle-145 generalisation:
    // front-loaded fail-loud guard. Walk every Operation BEFORE the
    // emission recursion and reject the same-worker-pair-elision +
    // consumer-reads-outside-local-slice shape with a typed
    // `TransferInjectError::SameSetSilentElisionRisk`. The unsafe
    // pattern is the silent-miscompile path filed as TASK-0324 cycle
    // 143; pre-validating here keeps the existing recursive emission
    // walk unchanged for the safe shapes (e.g. 13-cnn-inference/
    // batch_parallel reader-iv == partition-iv) while turning the
    // unsafe shape into a typed compiler error rather than wrong
    // output. Companion comments live at both same-worker short-circuits
    // inside `build_waits_for_op` (the `producer_workers ==
    // &consumer_workers` whole-set short-circuit AND the `if src == dst`
    // per-element skip in the cartesian-product fan-out — grep
    // `if producer_workers == &consumer_workers` and `if src == dst`
    // for the witness anchors).
    let partition_iter_vars: BTreeSet<IterVar> =
        partition_worker_ranges.keys().copied().collect();
    check_no_silent_elision_risk(
        &root,
        &producers_by_data,
        &partition_iter_vars,
        &name_iter_vars,
        &name_data,
    )?;

    // Counter state for SeqTag generation. A single monotonic counter
    // across the whole ACFG is simplest and meets the "unique per
    // pair" requirement. PRD §8.3 talks about per-(src,dst,data)
    // monotonic numbering — that ordering is also a strict subset of
    // a single global counter (per-triple subsequences are still
    // strictly increasing). The global counter has the property that
    // every Push/Wait pair in the whole program has a distinct seq,
    // which is the strongest reading of "unique".
    let mut state = State {
        next_seq: 0,
        last_writer: BTreeMap::new(),
    };

    let new_root = inject_in_node(root, &ctx, &mut state);

    // Cross-scope finalisation (TASK-0136 / TASK-0139). The single-
    // pass `inject_in_sequence` above only rendezvouses a Push with a
    // Wait when both live in the *same* sequence. When the consumer is
    // inside a plain `for` loop and the producer is in the enclosing
    // sequence (example 02-split: `load_input` on host at top level,
    // `add` on w0 inside `for i`), the Wait is trapped inside the
    // Repeat body and never gets a Push — the net deadlocks.
    //
    // The correct model is *whole-symbol* transfer: a datum that is
    // loop-invariant w.r.t. a Repeat crosses the worker boundary once,
    // not once per iteration. Pass A hoists such Waits out of the loop
    // body; Pass B places the matching Push (after the producer, or
    // after the producer's enclosing loop when the consumer sits
    // outside it — the dual case, e.g. `c` produced inside `for i` and
    // read by `save_output` after the loop).
    //
    // Scope: PER-WAIT (TASK-0267). Pass A's stay-vs-bubble decision is
    // already keyed on whether the Wait's `data` is produced inside
    // the enclosing Repeat's subtree, so a block-governed nest does
    // not need a separate opacity gate: a host-loaded image (producer
    // outside) bubbles up and gets a whole-symbol Push at the
    // producer's scope, while an in-loop produced datum (producer
    // inside the loop body) stays for per-iteration rendezvous. The
    // per-worker partition slice is added later by
    // `rewrite_partition_tiles` (TASK-0117/TASK-0212) and halo widths
    // by `extend_xfer_tiles_for_halo` (TASK-0263), so the Wait carries
    // the right slice regardless of which Repeat it crossed. A future
    // per-block-tile slice (the original TASK-0149/TASK-0158
    // territory) would need a per-tile counterpart to halo extension;
    // no shipped schedule exercises that shape today.
    let new_root = {
        let (hoisted, escaped_at_root) = hoist_invariant_waits(new_root, &[]);
        // A Wait that bubbled all the way out of the tree had no
        // producing scope anywhere (its data is produced by no
        // Operation in the whole ACFG). For a well-formed ACFG this
        // never happens — a cross-worker Wait is only emitted because
        // the schedule records a producer, which `build_acfg` lowers
        // to an Operation. Dropping it silently here would hand a
        // downstream pass an unpaired Wait to mis-diagnose. Fail loud
        // with context instead (TASK-0152).
        if let Some(w) = escaped_at_root.first() {
            let data_name = name_data
                .iter()
                .find_map(|(n, id)| (*id == w.data).then_some(n.as_str()))
                .unwrap_or("<unknown>");
            panic!(
                "transfer_inject invariant violation: cross-worker Wait for \
                 data `{data_name}` (id {:?}, seq {:?}, {:?} -> {:?}) escaped \
                 the whole ACFG with no producing Operation. A Wait is only \
                 emitted when the schedule records a producer for the symbol, \
                 so this is a malformed ACFG (producer kernel not lowered), \
                 not a partial test input.",
                w.data, w.seq, w.src, w.dst
            );
        }
        let spliced = splice_pushes_global(hoisted, &name_data);
        // TASK-0117 per-worker tile finalisation. The hoist + splice
        // passes above may rewrite a Wait/Push tile to the
        // post-hoist enclosing tile (e.g. a loop-invariant `input`
        // Wait at top level lands with `[]`). For a fan-out pair the
        // tile must reflect the COMPUTE worker's partition slice so
        // the backend host-side gather can slice-paste. We do the
        // rewrite as a final ACFG walk keyed by the pair's
        // src/dst against `partition_worker_ranges`; pairs whose
        // neither endpoint is partitioned (the 1:1 host↔single-worker
        // shape from examples 01..07) survive unchanged because the
        // map has no entry for either side.
        // TASK-0301 / TASK-0302: Build the per-data, per-dim iter-var
        // index map by walking every Operation's DataflowDag. Consulted
        // by `rewrite_partition_tiles_inner` to enforce the
        // *contiguous-prefix* invariant on per-Xfer tile bounds: bounds
        // must be in dim order AND cover only a contiguous prefix of
        // the data's dims, lest `wait_slice` (whose convention is
        // `tile.bounds[i].iter_var ↔ data.dim[i]`) silently mis-map a
        // sparse covering. The 1D AXIS-MAPPING discharge (TASK-0301)
        // covered the "iv not in the data's union" case via the
        // per-symbol filter; TASK-0302 generalises the input to a
        // per-dim map so the 2D `b[k][j]` × `partition=blocks2d(i,j)`
        // shape (where j IS in b's union but only at dim 1, with no
        // partitioned iv covering dim 0) also drops safely to a
        // whole-array broadcast rather than slicing the wrong dim.
        let data_dim_iv_map = collect_data_dim_iv_map(&spliced, &name_iter_vars);
        let partitioned = rewrite_partition_tiles(
            spliced,
            &partition_worker_ranges,
            &data_dim_iv_map,
        );
        // TASK-0263 Stage 2 halo extension. For each XferPlaceholder
        // whose tile axis carries a non-zero halo entry (the data
        // symbol's consumer kernel's halo widths along that
        // iter-var), extend the axis range by halo on both sides.
        // Clamp against the iter-var's algorithm source range
        // expanded by halo — i.e. the data extent the (unpartitioned)
        // loop would have read. No-op when halo_widths is empty
        // (every pre-Stage-2 example).
        let with_halo = extend_xfer_tiles_for_halo(partitioned, &halo_widths);
        // TASK-0289 cycle 114a halo-strip Push/Wait synthesis. For each
        // `(outer_iv, inner_iv)` pair recorded in `partition_pairs`
        // (populated by partition_blocks2d), synthesise cross-worker
        // halo-strip Push/Wait pairs between N/S/E/W neighbours in the
        // 2D worker grid. Corner pairs (NE/NW/SE/SW) are excluded in
        // this first cut per the TASK-0289 brief. Runs AFTER
        // `rewrite_partition_tiles` and `extend_xfer_tiles_for_halo`
        // so the carefully-crafted halo-strip tiles we emit are not
        // clobbered by either pass — both rewrite tiles in-place by
        // walking every `Xfer`. Short-circuits on empty
        // `partition_pairs` (AC#3 additive-only contract: every shipped
        // schedule today has empty pairs and therefore sees no change
        // in injected XferPlaceholders).
        inject_halo_strip_xfers(
            with_halo,
            &halo_widths,
            &partition_pairs,
            &grid_shape_for_outer_iv,
            &partition_worker_ranges,
            &policies_by_data,
            &data_dim_iv_map,
            &mut state,
        )
    };

    // TASK-0134 — populate the pipeline-depth sidecar AFTER all the
    // hoist/splice/rewrite passes have run. Doing it now (rather than
    // at `fresh_seq` time) is load-bearing: `hoist_invariant_waits`
    // moves Waits OUT of the loop body for loop-invariant whole-symbol
    // transfers, AND rewrites their `tile` to the post-hoist
    // enclosing tile. The seq's pipeline depth must reflect WHERE the
    // Push/Wait pair LANDED, not where it was born — otherwise we'd
    // pre-seed a buffer place with `D` tokens when only one
    // Push/Wait fires for the whole loop and the buffer place would
    // overflow at construction.
    //
    // Walk each `Xfer`'s `tile` (post-hoist enclosing iteration tile)
    // against `pipeline_depth_for_iter_var`; the innermost matching
    // entry wins. If neither endpoint of the pair sits inside a
    // pipelined loop, the seq has no entry (-> `initial_marking = 0`,
    // the default). Pairs whose Push and Wait have differing tiles
    // are kept in sync because: (1) the Wait's tile is the
    // authoritative enclosing context (it is the consumer side; the
    // pair fires per-consumer-tile), and (2) for whole-symbol
    // hoisting both sides are moved together so tiles agree post-
    // splice. We aggregate per seq using "any endpoint inside a
    // pipelined loop" as the trigger, but in practice the two
    // endpoints share the same enclosing tile by construction.
    let mut pipeline_depth_for_seq: BTreeMap<SeqTag, NonZeroU64> = pre_existing_pipeline_depth;
    if !pipeline_depth_for_iter_var.is_empty() {
        annotate_pipeline_depth_for_seq(
            &new_root,
            &pipeline_depth_for_iter_var,
            &mut pipeline_depth_for_seq,
        );
    }

    Ok(ACFG {
        root: new_root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths,
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
    })
}

/// Walk the final ACFG (post-hoist, post-splice, post-rewrite-tiles)
/// and populate `out` with (SeqTag, depth) for every `Xfer`
/// placeholder whose enclosing iteration tile contains an iter-var
/// with a `pipeline=D` directive. **Innermost wins.**
///
/// The "innermost wins" semantic rests on [`IterTile::bounds`]'s
/// documented convention (event.rs ~line 227): "Outer-most iteration
/// variable first." Walking `bounds.iter().rev()` therefore visits
/// innermost-to-outermost, and `find_map` stops at the first
/// (= innermost) pipelined iter-var. All construction sites build
/// `bounds` in nest order — the enclosing-tile stack in
/// `inject_in_sequence`, the fan-out path, and (since TASK-0224) the
/// partition-rewrite path, which walks the enclosing `Repeat` stack
/// directly rather than relying on `IterVar` id ordering.
///
/// Why we use the Xfer's `tile` (not the live walk's enclosing-tile
/// stack): after `hoist_invariant_waits` runs, a Wait may live at a
/// shallower nesting than the Repeat it was born in, but its `tile`
/// is rewritten to that shallower nesting (see line ~733 in
/// `inject_in_sequence`). The tile is therefore the authoritative
/// "in which loop does this transfer fire" record.
///
/// Determinism: the walk is depth-first source-order; `BTreeMap`
/// insertion preserves stable iteration. If Push and Wait of the
/// same seq disagree on depth (shouldn't happen in well-formed
/// inputs — `splice_pushes_global` places the Push at the same
/// tile-scope as the Wait), **last-visited wins**: source-order
/// traversal visits Push before Wait inside a Sequence, so in
/// practice the Wait's annotation is what lands. In well-formed
/// inputs they agree, so the distinction is academic; documented
/// here so a future invariant change doesn't surprise the reader.
fn annotate_pipeline_depth_for_seq(
    node: &ACFGNode,
    pipeline_for_iv: &BTreeMap<IterVar, NonZeroU64>,
    out: &mut BTreeMap<SeqTag, NonZeroU64>,
) {
    match node {
        ACFGNode::Xfer(x) => {
            let depth = x
                .tile
                .bounds
                .iter()
                .rev()
                .find_map(|(iv, _)| pipeline_for_iv.get(iv).copied());
            if let Some(d) = depth {
                out.insert(x.seq, d);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Sequence(children) => {
            for c in children {
                annotate_pipeline_depth_for_seq(c, pipeline_for_iv, out);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            annotate_pipeline_depth_for_seq(body, pipeline_for_iv, out);
        }
    }
}

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

// --------------------------------------------------------------------
// Recursive walker
// --------------------------------------------------------------------

/// Inject into one ACFGNode. Returns the (possibly rewritten) node.
fn inject_in_node(node: ACFGNode, ctx: &InjectCtx<'_>, state: &mut State) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            ACFGNode::Sequence(inject_in_sequence(children, ctx, state, &[], None))
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            // Build the enclosing-tile contribution from this Repeat.
            // We re-walk the body with the contribution pushed onto a
            // local stack; the helper `inject_in_node_with_tile` does
            // the bookkeeping.
            let outer_tile = vec![(iter_var, range.clone())];
            let new_body = inject_in_node_with_tile(*body, ctx, state, &outer_tile, None);
            ACFGNode::Repeat {
                iter_var,
                range,
                body: Box::new(new_body),
                // transfer_inject only injects Push/Wait/Xfer into the
                // body; the strip-mine rebinding tag is structural and
                // survives verbatim (TASK-0180).
                block_tag,
            }
        }
        // Leaves: nothing to inject inside.
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => {
            // Record any side-effect on `last_writer` for non-nested
            // top-level ops in the Sequence walker; here we only see
            // leaves *outside* a Sequence (shouldn't really happen
            // from `build_acfg`, but defensive). Update the writer map
            // if it's an Operation.
            if let ACFGNode::Operation(op) = &leaf {
                update_writer(state, op);
            }
            leaf
        }
    }
}

/// Same as [`inject_in_node`] but with an enclosing-tile context
/// passed in. Used inside `Repeat` bodies so that any
/// `XferPlaceholder` created carries the iteration tile of the
/// enclosing loop(s).
///
/// The optional `hoist_sink` is set when this node sits *inside* a
/// `block`-inner intra-tile loop; instead of emitting Waits in-line
/// at the consumer Operation and Pushes after the producer
/// Operation, the walker forwards them to the sink so the
/// enclosing per-tile body sequence can place them around the inner
/// `Repeat`. TASK-0143.
fn inject_in_node_with_tile(
    node: ACFGNode,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
    hoist_sink: Option<&mut HoistSink>,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => ACFGNode::Sequence(inject_in_sequence(
            children,
            ctx,
            state,
            enclosing_tile,
            hoist_sink,
        )),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            let mut nested = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            // Forward whatever sink we received into the body. The
            // decision of WHERE a sink originates and WHERE it
            // drains lives entirely in `inject_in_sequence` (which
            // creates one when it encounters a `block`-inner Repeat
            // child and parent_sink is None). Repeats themselves
            // are transparent: they neither create sinks nor drop
            // them, regardless of whether the current Repeat is
            // block-inner or not. This is what lets a 2D-blocked
            // schedule hoist through the (outer-j, inner-i) layers
            // all the way up to the outer-i tile body. TASK-0143.
            let new_body = inject_in_node_with_tile(*body, ctx, state, &nested, hoist_sink);
            ACFGNode::Repeat {
                iter_var,
                range,
                body: Box::new(new_body),
                // Transparent reconstruction — preserve the strip-mine
                // rebinding tag verbatim (TASK-0180).
                block_tag,
            }
        }
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => {
            if let ACFGNode::Operation(op) = &leaf {
                update_writer(state, op);
            }
            leaf
        }
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

/// Walk a sequence of children, injecting Wait placeholders before
/// each Operation that reads cross-worker data and Push placeholders
/// after each Operation that produces data that someone downstream
/// will read on a different worker.
///
/// Implementation note: we emit *both* Wait (before the consumer) and
/// Push (after the producer) in a single linear scan. For each
/// consumer-side read of `D` whose producer entity differs from the
/// consumer's, we walk back through the children we've already
/// emitted to find the most recent producer of `D`, and splice a
/// Push placeholder immediately after it. That keeps the rewrite
/// local — no second pass needed.
fn inject_in_sequence(
    children: Vec<ACFGNode>,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
    // If non-None, this sequence sits *inside* a `block`-inner
    // intra-tile loop chain: any non-locally-produced Wait we
    // would emit gets diverted into the sink instead of staying
    // here, so the eventual destination sequence (the nearest
    // ancestor whose `parent_sink` was None) places the Wait at
    // per-tile granularity.
    mut parent_sink: Option<&mut HoistSink>,
) -> Vec<ACFGNode> {
    let mut out: Vec<ACFGNode> = Vec::with_capacity(children.len());

    // For each data symbol we've already produced in this sequence,
    // remember the index of the producing Operation in `out`. We use
    // this to splice a Push placeholder immediately after it.
    // BTreeMap for deterministic iteration; only the per-key insert
    // matters semantically.
    let mut local_producer_idx: BTreeMap<DataId, usize> = BTreeMap::new();

    // Buffer of (insert-before-index-in-out, Wait) for Xfers hoisted
    // out of an inner block-loop sibling that THIS sequence is the
    // destination for. Only populated when `parent_sink` is None at
    // the point we encounter a block-inner Repeat child. Drained into
    // `out` at the end of the walk.
    let mut hoisted_waits_to_place: Vec<(usize, XferPlaceholder)> = Vec::new();
    // Local sink (one per block-inner child we are draining for).
    // Held outside the loop so its lifetime spans the loop.
    let mut local_sink: HoistSink = HoistSink::default();
    let parent_sink_is_some = parent_sink.is_some();

    for child in children {
        // Peek at the child's shape before the recursion.
        let child_is_block_inner_repeat = matches!(
            &child,
            ACFGNode::Repeat { iter_var, .. }
                if ctx.inner_block_iter_vars.contains(iter_var)
        );

        let child = if child_is_block_inner_repeat && parent_sink_is_some {
            // We're not the destination; forward hoisted Waits
            // through to our parent's sink. `as_deref_mut` reborrows
            // for the duration of the recursion.
            let sink_ref: &mut HoistSink = parent_sink.as_deref_mut().expect("checked");
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(sink_ref))
        } else if child_is_block_inner_repeat {
            // We are the per-tile destination. Pass our `local_sink`
            // to the recursion; record the slot where the rewritten
            // Repeat will land so we can place hoisted Waits
            // immediately before it; then move the sink's contents
            // into `hoisted_waits_to_place` for the post-walk drain.
            let rewritten =
                inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(&mut local_sink));
            let slot = out.len();
            // Drain local_sink between block-inner siblings.
            for w in std::mem::take(&mut local_sink.waits) {
                hoisted_waits_to_place.push((slot, w));
            }
            rewritten
        } else if parent_sink_is_some {
            // Non-block-inner child inside an inner-block context:
            // propagate parent_sink so Waits emitted further down
            // can still hoist past this scope.
            let sink_ref: &mut HoistSink = parent_sink.as_deref_mut().expect("checked");
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(sink_ref))
        } else {
            // Plain non-block-inner child outside any inner-block
            // context: recurse with no sink. Standard pre-TASK-0143
            // path.
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, None)
        };

        match &child {
            ACFGNode::Operation(op) => {
                // ---- Wait injection (consumer side) ----
                let waits = build_waits_for_op(op, ctx, state, enclosing_tile);
                for w in waits {
                    // Idempotence: if the immediately-preceding
                    // element is already a Wait with matching
                    // (src, dst, data, tile), skip pushing it.
                    if !is_duplicate_xfer(out.last(), &w) {
                        out.push(ACFGNode::Xfer(w));
                    }
                }

                // ---- Update local_producer_idx for any data this op
                //      writes. We record the index AFTER pushing the
                //      Operation so the Push insertion targets the
                //      right slot. ----
                let idx_after_push = out.len();
                update_writer(state, op);
                out.push(child.clone());
                if let Some(data_out) = output_data(op) {
                    local_producer_idx.insert(data_out, idx_after_push);
                }

                // ---- Push injection (producer side) for any data
                //      this op produces whose downstream consumer
                //      will be cross-worker. We can't know future
                //      consumers up-front, so we add Pushes lazily
                //      when a future consumer is seen (see the
                //      Wait-handling branch above): the moment a
                //      Wait is emitted for (src, dst, D, tile), we
                //      also splice a Push at `local_producer_idx[D]
                //      + 1` if not already present. That happens in
                //      the next iteration; see below. ----
            }
            _ => {
                // Non-Operation nodes (Repeat, Sequence, Sync, Xfer):
                // already rewritten by recursion. Push as-is.
                out.push(child);
            }
        }
    }

    // Drain hoisted Waits: insert each (slot, Wait) into `out` so
    // the Wait sits immediately before the inner Repeat. Inserting
    // shifts later indices, so we process by descending slot. For a
    // given slot, multiple Waits keep their relative order — same
    // semantics as the in-line emission path.
    //
    // Before placement, rewrite each hoisted Wait's `IterTile` to
    // match THIS sequence's enclosing tile. The consumer's Wait was
    // built deeper in the nest and carries axes for every loop
    // between the consumer and the original block-inner; once
    // hoisted to the per-tile destination, only the axes enclosing
    // this destination should remain. Replacing the tile wholesale
    // is simpler than tracking which axes were crossed during the
    // hoist forward, and matches the PRD §6.3.3 "transfers happen
    // per tile" semantic: the Wait fires at this sequence's
    // enclosing-tile granularity.
    for (_, w) in hoisted_waits_to_place.iter_mut() {
        w.tile = IterTile::new(enclosing_tile.to_vec());
    }
    if !hoisted_waits_to_place.is_empty() {
        // Stable sort by slot ascending so later (larger-slot)
        // inserts can be applied first without invalidating earlier
        // slots' positions.
        hoisted_waits_to_place.sort_by_key(|(s, _)| *s);
        // Group by slot, then for each group insert in reverse slot
        // order. Within a group, preserve original order by
        // inserting in reverse.
        let mut by_slot: BTreeMap<usize, Vec<XferPlaceholder>> = BTreeMap::new();
        for (s, w) in hoisted_waits_to_place {
            by_slot.entry(s).or_default().push(w);
        }
        // Iterate in *reverse* slot order. Iter rev on BTreeMap.
        let mut shifts: Vec<(usize, Vec<XferPlaceholder>)> = by_slot.into_iter().collect();
        shifts.sort_by_key(|(s, _)| *s);
        for (slot, waits) in shifts.into_iter().rev() {
            // Insert each Wait at `slot`, in reverse so the first
            // listed Wait ends up first in `out`.
            for w in waits.into_iter().rev() {
                // Idempotence: skip if `out` already contains an
                // identical Wait *anywhere*. Re-running the pass on
                // a hoisted ACFG re-derives the same Wait from the
                // consumer Op; without this dedup we would emit a
                // second copy at the same slot. We check against
                // the entire sequence (not just `out[slot]`) because
                // the previously-hoisted Wait may sit at a slot that
                // shifted as siblings were rewritten.
                let already_present = out.iter().any(|n| {
                    matches!(
                        n,
                        ACFGNode::Xfer(existing)
                            if existing.role == XferRole::Wait
                                && existing.src == w.src
                                && existing.dst == w.dst
                                && existing.data == w.data
                                && existing.tile == w.tile
                    )
                });
                if already_present {
                    continue;
                }
                out.insert(slot, ACFGNode::Xfer(w));
            }
        }
        // Recompute `local_producer_idx` -- insertions shifted some
        // recorded indices. Cheap to walk `out` once.
        local_producer_idx.clear();
        for (i, n) in out.iter().enumerate() {
            if let ACFGNode::Operation(op) = n {
                if let Some(data_out) = output_data(op) {
                    local_producer_idx.insert(data_out, i);
                }
            }
        }
    }

    // Second pass: for every Wait we emitted (in-line *or* hoisted)
    // in this sequence, walk back and splice a matching Push
    // immediately after the producer Operation (which is in `out`
    // because the producer's Operation appeared earlier in the same
    // sequence) — *unless* the producer is in a different scope
    // (outer sequence, sibling sequence). In that case the
    // outer-scope walker handles it. We detect "different scope" by
    // checking `local_producer_idx`: if the Wait names a data symbol
    // not in `local_producer_idx`, the producer is not in this
    // sequence and we leave the Push to the caller.
    splice_pushes_for_waits(&mut out, &local_producer_idx);

    // If this sequence is *itself* sitting inside a block-inner
    // Repeat chain (i.e. our caller passed a `parent_sink`), forward
    // any Waits we just placed up to the parent so they land at the
    // eventual destination's per-tile granularity. We only forward
    // Waits whose producer was NOT in this sequence — in-sequence
    // Push/Wait pairs already form a complete intra-tile rendezvous
    // and need not propagate further.
    //
    // Tile rewrite happens at the destination (see the drain block
    // above); here we simply hand the Wait off unchanged.
    if let Some(sink) = parent_sink {
        let mut kept: Vec<ACFGNode> = Vec::with_capacity(out.len());
        for node in out.into_iter() {
            match node {
                ACFGNode::Xfer(x)
                    if x.role == XferRole::Wait && !local_producer_idx.contains_key(&x.data) =>
                {
                    sink.waits.push(x);
                }
                other => kept.push(other),
            }
        }
        out = kept;
    }

    out
}

/// Walk `out` left-to-right; for every Wait placeholder, look up the
/// producer Operation in `local_producer_idx` and splice a Push
/// placeholder right after it (with the same `seq` as the Wait).
///
/// Splicing later items shifts indices, so we collect insertion
/// requests first and then apply them in reverse order to keep the
/// recorded indices valid.
fn splice_pushes_for_waits(out: &mut Vec<ACFGNode>, local_producer_idx: &BTreeMap<DataId, usize>) {
    // (insert_at, placeholder)
    let mut inserts: Vec<(usize, XferPlaceholder)> = Vec::new();

    for n in out.iter() {
        if let ACFGNode::Xfer(x) = n {
            if x.role == XferRole::Wait {
                if let Some(&producer_idx) = local_producer_idx.get(&x.data) {
                    // Push goes immediately after the producer
                    // Operation, i.e. at producer_idx + 1.
                    let insert_at = producer_idx + 1;
                    let push = XferPlaceholder {
                        role: XferRole::Push,
                        src: x.src,
                        dst: x.dst,
                        data: x.data,
                        tile: x.tile.clone(),
                        seq: x.seq,
                        policy: x.policy,
                    };
                    inserts.push((insert_at, push));
                }
            }
        }
    }

    // Apply in reverse index order so earlier insertions don't
    // invalidate the later ones.
    inserts.sort_by_key(|(i, _)| *i);
    for (insert_at, push) in inserts.into_iter().rev() {
        // Idempotence: if the slot immediately after the producer is
        // already a Push with matching (src, dst, data, tile), skip.
        let already = out.get(insert_at).and_then(|n| match n {
            ACFGNode::Xfer(x) => Some(x.clone()),
            _ => None,
        });
        if let Some(existing) = already {
            if existing.role == XferRole::Push
                && existing.src == push.src
                && existing.dst == push.dst
                && existing.data == push.data
                && existing.tile == push.tile
            {
                continue;
            }
        }
        out.insert(insert_at, ACFGNode::Xfer(push));
    }
}

// --------------------------------------------------------------------
// Pass A — hoist loop-invariant Waits out of plain Repeat bodies
// --------------------------------------------------------------------
//
// TASK-0136 / TASK-0139. `inject_in_sequence` emits a Wait immediately
// before the consumer Operation. When the consumer sits inside a
// `for` loop but the data it reads is produced *outside* the loop
// (loop-invariant by structure: the data symbol is not written by any
// Operation inside the loop body), the Wait should fire once before
// the loop, not once per iteration. The Petri lowering unrolls Repeat
// bodies (`acfg_to_petri::walk`), so a per-iteration Wait against a
// single whole-symbol Push deadlocks at iteration 2 (one token, N
// consumers). Hoisting the Wait out of the body fixes this and matches
// what a real backend does (transfer the whole symbol once).
//
// Returns the rewritten node plus the Waits that escaped past the top
// of `node` and must be placed (or further hoisted) by the caller.

/// Collect every data symbol written by some Operation anywhere in the
/// subtree rooted at `node`.
fn produced_data_set(node: &ACFGNode, acc: &mut BTreeSet<DataId>) {
    match node {
        ACFGNode::Operation(op) => {
            if let Some(d) = output_data(op) {
                acc.insert(d);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
        ACFGNode::Repeat { body, .. } => produced_data_set(body, acc),
        ACFGNode::Sequence(children) => {
            for c in children {
                produced_data_set(c, acc);
            }
        }
    }
}

/// Number of Operations in the subtree that write `data`. Used only
/// by a debug-assert guarding the single-assignment invariant
/// (TASK-0153); not on the release hot path.
fn count_producers(node: &ACFGNode, data: DataId) -> usize {
    match node {
        ACFGNode::Operation(op) => usize::from(output_data(op) == Some(data)),
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        ACFGNode::Repeat { body, .. } => count_producers(body, data),
        ACFGNode::Sequence(children) => children.iter().map(|c| count_producers(c, data)).sum(),
    }
}

fn hoist_invariant_waits(
    node: ACFGNode,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
) -> (ACFGNode, Vec<XferPlaceholder>) {
    match node {
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => (leaf, Vec::new()),
        // A bare Wait/Xfer outside a Sequence shouldn't occur from the
        // builder, but if it does we cannot decide invariance without
        // its sibling context — leave it in place.
        leaf @ ACFGNode::Xfer(_) => (leaf, Vec::new()),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            let mut nested: Vec<(IterVar, std::ops::Range<i64>)> = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            let (body2, escaped) = hoist_invariant_waits(*body, &nested);

            let mut produced = BTreeSet::new();
            produced_data_set(&body2, &mut produced);

            // A Wait that escaped the body but whose data IS produced
            // inside this loop is NOT loop-invariant w.r.t. this
            // Repeat: it belongs inside, as a per-iteration rendezvous.
            // Re-inject it at the front of the body. Otherwise it keeps
            // bubbling up.
            let mut stay: Vec<XferPlaceholder> = Vec::new();
            let mut bubble: Vec<XferPlaceholder> = Vec::new();
            for w in escaped {
                if produced.contains(&w.data) {
                    stay.push(w);
                } else {
                    bubble.push(w);
                }
            }

            let body3 = if stay.is_empty() {
                body2
            } else {
                let mut children: Vec<ACFGNode> = stay.into_iter().map(ACFGNode::Xfer).collect();
                match body2 {
                    ACFGNode::Sequence(cs) => children.extend(cs),
                    other => children.push(other),
                }
                ACFGNode::Sequence(children)
            };

            (
                ACFGNode::Repeat {
                    iter_var,
                    range,
                    body: Box::new(body3),
                    // Wait-hoisting only reshuffles the body; the
                    // strip-mine rebinding tag is preserved verbatim
                    // (TASK-0180).
                    block_tag,
                },
                bubble,
            )
        }
        ACFGNode::Sequence(children) => {
            // Recurse non-Wait children; hold direct Wait Xfers aside
            // so we can decide per-Wait whether it stays here (its data
            // is produced in this sequence: an intra-scope rendezvous)
            // or bubbles up (loop-invariant, escape the enclosing
            // Repeat).
            enum Slot {
                Node(ACFGNode),
                Wait(XferPlaceholder),
            }
            let mut slots: Vec<(Slot, Vec<XferPlaceholder>)> = Vec::new();
            for child in children {
                match child {
                    ACFGNode::Xfer(x) if x.role == XferRole::Wait => {
                        slots.push((Slot::Wait(x), Vec::new()));
                    }
                    other => {
                        let (c2, esc) = hoist_invariant_waits(other, enclosing_tile);
                        slots.push((Slot::Node(c2), esc));
                    }
                }
            }

            // Data produced anywhere in this sequence (post-recursion).
            let mut produced = BTreeSet::new();
            for (slot, _) in &slots {
                if let Slot::Node(n) = slot {
                    produced_data_set(n, &mut produced);
                }
            }

            let mut out: Vec<ACFGNode> = Vec::new();
            let mut escaped_up: Vec<XferPlaceholder> = Vec::new();

            let place_or_bubble =
                |w: XferPlaceholder,
                 out: &mut Vec<ACFGNode>,
                 escaped_up: &mut Vec<XferPlaceholder>| {
                    if produced.contains(&w.data) {
                        // Lands here: rewrite tile to this sequence's
                        // enclosing-tile granularity and dedup against
                        // an already-placed equivalent Wait (keeps the
                        // pass idempotent on re-run — the regenerated
                        // Wait carries a fresh seq, but the structural
                        // (src,dst,data) key is stable, so we keep the
                        // first and drop the duplicate).
                        let mut w = w;
                        w.tile = IterTile::new(enclosing_tile.to_vec());
                        let dup = out.iter().any(|n| {
                            matches!(n, ACFGNode::Xfer(x)
                                if x.role == XferRole::Wait
                                    && x.src == w.src
                                    && x.dst == w.dst
                                    && x.data == w.data)
                        });
                        if !dup {
                            out.push(ACFGNode::Xfer(w));
                        }
                    } else {
                        escaped_up.push(w);
                    }
                };

            for (slot, esc) in slots {
                // Escaped Waits from this child (a Repeat) are placed
                // immediately *before* the child.
                for w in esc {
                    place_or_bubble(w, &mut out, &mut escaped_up);
                }
                match slot {
                    Slot::Node(n) => out.push(n),
                    Slot::Wait(x) => {
                        if produced.contains(&x.data) {
                            // Intra-scope rendezvous: producer is a
                            // sibling here. Keep the Wait where it is.
                            out.push(ACFGNode::Xfer(x));
                        } else {
                            // Loop-invariant w.r.t. the Repeat that
                            // encloses this sequence: hoist it out.
                            escaped_up.push(x);
                        }
                    }
                }
            }

            (ACFGNode::Sequence(out), escaped_up)
        }
    }
}

// --------------------------------------------------------------------
// Pass B — global Push finaliser (cross-scope)
// --------------------------------------------------------------------
//
// After Pass A every Wait is positioned. Each Wait still needs a
// matching Push after its producer. `splice_pushes_for_waits` already
// did this for in-sequence pairs during the single-pass walk; this
// pass finalises the cross-scope ones it could not see, idempotently
// (keyed on the unique `seq`, with a structural (src,dst,data) guard).
//
// Whole-symbol placement rule, given producer P of D and the Wait W:
//
//   * If P sits inside one or more Repeats that W is NOT inside, the
//     Push goes immediately after the *outermost* such Repeat (D is a
//     loop output, available once the loop completes — the dual of
//     Pass A's loop-input hoist). Example 02: `c` produced by `add`
//     inside `for i`, read by `save_output` after the loop.
//
//   * Otherwise the Push goes immediately after P itself (same scope,
//     or P and W co-resident in the same loop = per-iteration
//     rendezvous).

/// Outer→inner list of Repeat iter-vars enclosing the (unique,
/// single-assignment) producer Operation of `data`.
fn producer_repeat_path(
    node: &ACFGNode,
    data: DataId,
    path: &mut Vec<IterVar>,
    found: &mut Option<Vec<IterVar>>,
) {
    if found.is_some() {
        return;
    }
    match node {
        ACFGNode::Operation(op) => {
            if output_data(op) == Some(data) {
                *found = Some(path.clone());
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
        ACFGNode::Repeat { iter_var, body, .. } => {
            path.push(*iter_var);
            producer_repeat_path(body, data, path, found);
            path.pop();
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                producer_repeat_path(c, data, path, found);
            }
        }
    }
}

/// Outer→inner list of Repeat iter-vars enclosing the Wait carrying
/// `seq`.
fn wait_repeat_path(
    node: &ACFGNode,
    seq: SeqTag,
    path: &mut Vec<IterVar>,
    found: &mut Option<Vec<IterVar>>,
) {
    if found.is_some() {
        return;
    }
    match node {
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Xfer(x) => {
            if x.role == XferRole::Wait && x.seq == seq {
                *found = Some(path.clone());
            }
        }
        ACFGNode::Repeat { iter_var, body, .. } => {
            path.push(*iter_var);
            wait_repeat_path(body, seq, path, found);
            path.pop();
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                wait_repeat_path(c, seq, path, found);
            }
        }
    }
}

fn subtree_produces(node: &ACFGNode, data: DataId) -> bool {
    let mut s = BTreeSet::new();
    produced_data_set(node, &mut s);
    s.contains(&data)
}

/// Insert `push` immediately after the (unique) producer Operation of
/// `push.data` wherever it directly resides.
fn splice_after_producer(node: ACFGNode, push: &XferPlaceholder) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            let mut out = Vec::with_capacity(children.len() + 1);
            for c in children {
                let is_producer = matches!(&c, ACFGNode::Operation(op)
                    if output_data(op) == Some(push.data));
                let c = splice_after_producer(c, push);
                out.push(c);
                if is_producer {
                    out.push(ACFGNode::Xfer(push.clone()));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_producer(*body, push)),
            block_tag,
        },
        leaf => leaf,
    }
}

/// Insert `push` immediately after the Repeat whose iter-var is
/// `cut_iv` and which (transitively) produces `push.data`.
fn splice_after_repeat(node: ACFGNode, cut_iv: IterVar, push: &XferPlaceholder) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            let mut out = Vec::with_capacity(children.len() + 1);
            for c in children {
                let is_cut = matches!(&c, ACFGNode::Repeat { iter_var, .. }
                    if *iter_var == cut_iv && subtree_produces(&c, push.data));
                if is_cut {
                    out.push(c);
                    out.push(ACFGNode::Xfer(push.clone()));
                } else {
                    out.push(splice_after_repeat(c, cut_iv, push));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(splice_after_repeat(*body, cut_iv, push)),
            block_tag,
        },
        leaf => leaf,
    }
}

fn collect_push_seqs(node: &ACFGNode, seqs: &mut BTreeSet<u64>) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Push => {
            seqs.insert(x.seq.0);
        }
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => collect_push_seqs(body, seqs),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_push_seqs(c, seqs);
            }
        }
    }
}

/// Collect Waits eligible for whole-symbol Push finalisation.
/// TASK-0267: per-Wait classification — every Wait in the tree is
/// eligible; the per-Wait stay-vs-bubble decision in Pass A
/// (`hoist_invariant_waits`) has already placed each Wait at the
/// scope where its `data` becomes producer-relative, so Pass B's
/// producer-path / wait-path cut handles all shapes uniformly.
fn collect_waits(node: &ACFGNode, out: &mut Vec<XferPlaceholder>) {
    match node {
        ACFGNode::Xfer(x) if x.role == XferRole::Wait => out.push(x.clone()),
        ACFGNode::Xfer(_) | ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Repeat { body, .. } => {
            collect_waits(body, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_waits(c, out);
            }
        }
    }
}

fn splice_pushes_global(mut root: ACFGNode, name_data: &BTreeMap<String, DataId>) -> ACFGNode {
    let mut have_seqs: BTreeSet<u64> = BTreeSet::new();
    collect_push_seqs(&root, &mut have_seqs);

    let mut waits: Vec<XferPlaceholder> = Vec::new();
    collect_waits(&root, &mut waits);

    for w in waits {
        // Idempotence keyed on `seq` ALONE — deliberately not on
        // (src,dst,data). `seq` is unique per Push/Wait pair (global
        // monotonic counter, see the SeqTag note in `inject_transfers`),
        // so "a Push with this seq exists" is the exact "this transfer
        // is already paired" predicate.
        //
        // We must NOT also skip on (src,dst,data): single-assignment
        // data can have *several* cross-worker consumers on the same
        // dst worker (e.g. `d` produced on host, read by two distinct
        // Operations on w0). Each consumer gets its own Wait with its
        // own seq and its own seq-keyed buffer place in the Petri
        // lowering; suppressing the second because (host,w0,d) was
        // already seen would leave its buffer place unfilled and
        // deadlock that consumer. Idempotence on re-run is still
        // guaranteed because Pass A collapses the regenerated
        // fresh-seq duplicate Wait against the surviving original
        // (by (src,dst,data) at the destination sequence) BEFORE this
        // pass runs, so only the original seq reaches here and its
        // Push is already in `have_seqs`.
        if have_seqs.contains(&w.seq.0) {
            continue;
        }

        // Locate producer and its loop nesting vs the Wait's.
        let mut pp = None;
        producer_repeat_path(&root, w.data, &mut Vec::new(), &mut pp);
        let producer_path = match pp {
            Some(p) => p,
            // DEFENSE-IN-DEPTH behind the root-boundary check in
            // `inject_transfers` (the `escaped_at_root` panic). That
            // check should catch every producerless cross-worker Wait
            // BEFORE this pass runs: a Wait with no producing scope
            // bubbles out of Pass A's hoist and is rejected there.
            // This branch is therefore *expected* unreachable in
            // practice — but it is NOT proven unreachable: the two
            // guards key off different structures (Pass A's
            // whole-symbol-hoist root residue vs a per-`w.data`
            // producer walk of the post-hoist tree), and a future
            // hoist/block-transform change could let a Wait reach here
            // unpaired. Keep it as a loud `panic!` (not `unreachable!`,
            // which would assert a proof we do not have; not
            // `continue`, which would resurrect the silent-drop bug).
            // TASK-0152.
            //
            // Invariant rationale: a cross-worker Wait only exists
            // because `build_waits_for_op` found this symbol in
            // `producers_by_data` (mirrors `linked.data_producers`);
            // therefore a producing Operation MUST exist somewhere in
            // the ACFG (`build_acfg` places the producing kernel).
            // `None` here means the ACFG is malformed — a Wait whose
            // producer kernel was never lowered to an Operation. Fail
            // loud with full context per the `acfg.rs` fail-fast
            // precedent rather than silently leaving an unpaired Wait
            // for a downstream pass to mis-diagnose as a deadlock.
            None => {
                let data_name = name_data
                    .iter()
                    .find_map(|(n, id)| (*id == w.data).then_some(n.as_str()))
                    .unwrap_or("<unknown>");
                panic!(
                    "transfer_inject invariant violation: cross-worker Wait \
                     for data `{data_name}` (id {:?}, seq {:?}, {:?} -> {:?}) \
                     has no producing Operation in the ACFG. A Wait is only \
                     emitted when the schedule records a producer for the \
                     symbol, so this is a malformed ACFG (producer kernel not \
                     lowered), not a partial test input.",
                    w.data, w.seq, w.src, w.dst
                );
            }
        };

        // Single-assignment invariant (PRD §6.2.1, TASK-0153):
        // `producer_repeat_path` takes the FIRST Operation in walk
        // order that writes `w.data`. That is only well-defined if
        // there is exactly one. v2 data is single-assignment, so two
        // writers would be a front-end/lowering bug that would
        // mis-place the Push silently. Assert it in debug builds; no
        // release-build cost.
        debug_assert_eq!(
            count_producers(&root, w.data),
            1,
            "single-assignment violated: data id {:?} has multiple producing \
             Operations; producer_repeat_path would mis-place the Push",
            w.data
        );

        let mut wp = None;
        wait_repeat_path(&root, w.seq, &mut Vec::new(), &mut wp);
        let wait_path = wp.unwrap_or_default();

        let push = XferPlaceholder {
            role: XferRole::Push,
            src: w.src,
            dst: w.dst,
            data: w.data,
            tile: w.tile.clone(),
            seq: w.seq,
            policy: w.policy,
        };

        // The cut: outermost Repeat enclosing the producer that does
        // NOT also enclose the Wait. If present, the producer is a
        // loop output the consumer reads after the loop -> Push goes
        // after that Repeat. Otherwise Push goes right after producer.
        let cut = producer_path
            .iter()
            .find(|iv| !wait_path.contains(iv))
            .copied();

        root = match cut {
            Some(cut_iv) => splice_after_repeat(root, cut_iv, &push),
            None => splice_after_producer(root, &push),
        };

        have_seqs.insert(w.seq.0);
    }

    root
}

// --------------------------------------------------------------------
// TASK-0117 per-worker tile finalisation
// --------------------------------------------------------------------

/// Walk the ACFG and rewrite every `Xfer`'s `tile` to its
/// fan-out compute worker's partition slice.
///
/// "Compute worker" rule:
/// - If `src` appears in `partition_ranges` for any iter-var:
///   compute_worker = src (the N:1 gather direction, e.g. output).
/// - Else if `dst` appears: compute_worker = dst (the 1:N broadcast
///   direction, e.g. input).
/// - Else: no partition involvement — leave the tile unchanged
///   (the 1:1 host↔single-worker shape from examples 01..07).
///
/// The new tile is built by iterating the partitioned iter-vars in
/// NEST ORDER (outer-to-inner) and appending
/// `(iv, partition_ranges[iv][compute_worker])` for each iv where the
/// compute worker has a partition entry. The nest order itself is
/// derived from the ACFG topology via a single DFS pre-order pass over
/// the `Repeat` nodes — outer Repeats are visited before their
/// children — restricted to iter-vars that have a `partition_ranges`
/// entry. The resulting `bounds` is naturally OUTER-to-INNER, the
/// convention load-bearing for downstream passes
/// (`annotate_pipeline_depth_for_seq` walks `bounds.iter().rev()` for
/// "innermost wins").
///
/// TASK-0224 replaced an earlier per-Xfer `for (iv, ...) in
/// partition_ranges` loop that iterated in BTreeMap key-ascending =
/// IterVar-id ascending order. That coincided with nest order for
/// schedules where outer-loop iter-vars happened to have lower ids
/// (every in-tree schedule today) but is not guaranteed by the
/// `IterTile::bounds` convention. Pinned by
/// `rewrite_partition_tiles_bounds_in_nest_order_not_itervar_id_order`
/// and `rewrite_partition_tiles_three_level_nest_order` (synthetic
/// fixtures with non-monotonic IterVar ids).
///
/// Why we derive nest order from the ACFG rather than from the live
/// "enclosing-Repeat stack" at the Xfer's current position: by the
/// time this pass runs, `hoist_invariant_waits` has already moved
/// loop-invariant whole-symbol Waits OUT of their birth-position
/// Repeats (TASK-0151). For a fan-out broadcast like `host -> {w1..w4}`
/// over a partitioned `for n`, the Wait now sits at top-level, with an
/// empty live-stack — but we still want the per-worker partition slice
/// of `for n` recorded in its tile (the test
/// `transfer_fanout_composes_with_partition_sidecar` pins exactly
/// that). The Repeat node still exists in the ACFG, just at a sibling
/// position; a DFS pre-order over `Repeat::iter_var` captures the
/// program-wide nest order regardless of where any one Xfer happens
/// to have been hoisted to.
///
/// Determinism: DFS pre-order is a pure function of the ACFG; the
/// resulting `partition_axis_order` is the same for the same input.
///
/// We do this AFTER `hoist_invariant_waits` + `splice_pushes_global`
/// rather than at `build_waits_for_op` time because the hoist
/// rewrites the tile to the post-hoist enclosing tile (TASK-0151), so
/// a tile set at construction time would be clobbered for any Wait
/// hoisted out of the partitioned loop body. Setting the tile here
/// is the single sink for the contract.
fn rewrite_partition_tiles(
    node: ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) -> ACFGNode {
    if partition_ranges.is_empty() {
        return node;
    }
    // Derive the program-wide nest order of partitioned iter-vars via
    // a DFS pre-order over `Repeat::iter_var`, filtered to those in
    // `partition_ranges`. Outer Repeats appear before their children,
    // so the resulting Vec is OUTER-to-INNER. The same iter-var
    // appearing twice (e.g. via `block_transform` strip-mining that
    // reuses an IterVar across a full/partial split) is recorded only
    // on its first visit — the partition slice is keyed per iter-var,
    // and a second copy at deeper nest would double the bounds entry
    // for the same logical axis.
    //
    // `partition_axis_order` is the FALL-BACK iteration order used when
    // a data symbol has no observed indexed accesses (TASK-0301
    // additive contract → preserves pre-TASK-0301 behaviour for
    // synthetic test fixtures using `DataflowEdge::new`). For data
    // symbols WITH observed accesses, `data_dim_iv_map` carries the
    // per-dim info needed for the TASK-0302 dim-prefix logic, which
    // emits bounds in *data-dim* order (matching `wait_slice`'s
    // axis-mapping convention).
    let mut partition_axis_order: Vec<IterVar> = Vec::new();
    collect_partitioned_iter_var_nest_order(&node, partition_ranges, &mut partition_axis_order);
    rewrite_partition_tiles_inner(
        node,
        partition_ranges,
        &partition_axis_order,
        data_dim_iv_map,
    )
}

/// DFS pre-order walk of the ACFG recording each `Repeat::iter_var`
/// that has a `partition_ranges` entry, OUTER-to-INNER. Each iter-var
/// is recorded at most once even if a `Repeat` for the same `IterVar`
/// appears multiple times (e.g. strip-mined full/partial split sharing
/// one `IterVar`); first-occurrence wins, which is the outermost
/// occurrence under DFS pre-order.
fn collect_partitioned_iter_var_nest_order(
    node: &ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    out: &mut Vec<IterVar>,
) {
    match node {
        ACFGNode::Repeat { iter_var, body, .. } => {
            if partition_ranges.contains_key(iter_var) && !out.contains(iter_var) {
                out.push(*iter_var);
            }
            collect_partitioned_iter_var_nest_order(body, partition_ranges, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_partitioned_iter_var_nest_order(c, partition_ranges, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Recursive worker for [`rewrite_partition_tiles`].
///
/// `partition_axis_order` is the OUTER-to-INNER nest-order vec of
/// partitioned iter-vars precomputed by the caller. As of TASK-0302 it
/// is consulted only on the *fall-back* path (data symbols with no
/// observed indexed accesses — synthetic fixtures using
/// `DataflowEdge::new`, OR bare-aggregate-only data symbols). The
/// canonical path keys off `data_dim_iv_map` via
/// [`compute_partition_bounds_with_dim_prefix`], which emits bounds in
/// data-dim order (matching `wait_slice`'s
/// `tile.bounds[i] ↔ data.dim[i]` convention).
fn rewrite_partition_tiles_inner(
    node: ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    partition_axis_order: &[IterVar],
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(mut x) => {
            // Pick the compute worker per the rule above.
            let compute_worker = if partition_ranges.values().any(|m| m.contains_key(&x.src)) {
                Some(x.src)
            } else if partition_ranges.values().any(|m| m.contains_key(&x.dst)) {
                Some(x.dst)
            } else {
                None
            };
            if let Some(w) = compute_worker {
                // TASK-0302: prefer the per-dim contiguous-prefix
                // filter (which subsumes TASK-0301's per-symbol-union
                // filter and additionally handles the sparse-coverage
                // case where partition axes do NOT form a prefix of
                // the data's dims). Returns:
                //   - `Some(bounds)` — emit in DATA-DIM order. Empty
                //     vec means "drop to whole-array broadcast"
                //     (sparse coverage OR ambiguous multi-iv-per-dim).
                //   - `None` — no observed indexed accesses on this
                //     data; fall back to the pre-TASK-0301 nest-order
                //     iteration that preserves the synthetic-fixture
                //     contract (DataflowEdge::new constructs accesses
                //     with empty indices).
                //
                // Silent-sibling audit (architect P3.3, cycle 118;
                // updated TASK-0306 cycle 133; cycle-137 doc-lie
                // audit listing-correction; cycle-137 architect P1
                // fold-back adds `build_waits_for_op`, fixes line
                // cites for `extend_xfer_tiles_inner` and
                // `inject_halo_strip_xfers`; TASK-0319 cycle 146
                // migrates absolute line citations to function-name
                // anchors as primary indices — line numbers are
                // ADVISORY-ONLY): every other site that mutates
                // `x.tile` / `w.tile` OR constructs an `IterTile`
                // carrying the enclosing-tile assumption either:
                //
                // - **Builds tile from enclosing-loop stack** — three
                //   production sites, all using
                //   `IterTile::new(enclosing_tile.to_vec())`:
                //     1. `inject_in_sequence` — hoisted-Wait tile
                //        rewrite at sequence boundary, called via
                //        `inject_in_node_with_tile`'s dispatch.
                //     2. `hoist_invariant_waits` — Wait tile rewrite
                //        when an invariant-Wait is hoisted across an
                //        enclosing sequence (separate post-pass, NOT
                //        in the `inject_in_node_with_tile` family).
                //     3. `build_waits_for_op` — INITIAL
                //        `XferPlaceholder` tile on every cross-
                //        worker Wait the cartesian fan-out emits.
                //        This `rewrite_partition_tiles_inner` arm
                //        later overwrites it with `bounds`.
                //   None of the three references `partition_ranges`
                //   or `data_dim_iv_map` in its own scope, so the
                //   axis-mapping assumption cannot leak in via them.
                //   Grep witnesses (must remain consistent):
                //   - `grep -n "IterTile::new(enclosing_tile" `
                //     returns exactly three production sites (one
                //     per function above) PLUS module-doc citations.
                //   - `grep -nE "data_dim_iv_map|partition_ranges"`
                //     restricted to the body of each function above
                //     returns zero hits.
                // - **Extends an already-filtered bounds set** —
                //   `extend_xfer_tiles_for_halo`, with the per-Xfer
                //   mutation inside the worker `extend_xfer_tiles_inner`.
                //   Runs AFTER this pass per the explicit pass-order
                //   call in `inject_transfers`. Iterates the post-
                //   partition `x.tile.bounds` and widens each range
                //   by halo, preserving order.
                // - **Hand-crafts the (outer_iv, inner_iv) pair from
                //   the partition_pairs sidecar** —
                //   `inject_halo_strip_xfers` constructs fresh tiles
                //   at FOUR cardinal-direction `emit_pair` sites
                //   AFTER `order_halo_strip_bounds_by_data_dim`
                //   returns the data-dim-ordered bounds; axis-
                //   correct since cycle 133 by construction. Grep
                //   witness: `grep -n "emit_pair(neighbour"` returns
                //   exactly four sites (N/S/W/E neighbours) and all
                //   live inside `inject_halo_strip_xfers`.
                //
                // A future N-dim partition pass that constructs tile
                // bounds MUST consult `data_dim_iv_map` to avoid
                // re-importing the axis-mapping assumption — using
                // either:
                //   - `compute_partition_bounds_with_dim_prefix` for
                //     REGULAR-XFER TILE REWRITE (the path this match
                //     arm is on), where partitioned bounds replace
                //     the existing source tile under wait_slice's
                //     dim convention; or
                //   - `order_halo_strip_bounds_by_data_dim` for
                //     HALO-STRIP PUSH/WAIT SYNTHESIS (the
                //     `inject_halo_strip_xfers` path), where a
                //     freshly-constructed `(outer_iv, inner_iv)`
                //     tuple must be re-ordered into data-dim order
                //     before emit.
                let bounds = match compute_partition_bounds_with_dim_prefix(
                    x.data,
                    data_dim_iv_map,
                    partition_ranges,
                    w,
                ) {
                    Some(b) => b,
                    None => {
                        // Pre-TASK-0301 fall-back: iterate the
                        // partition_axis_order (nest order) and append
                        // every partitioned axis the worker has an
                        // entry for. Used only when the data symbol
                        // has no observed indexed accesses (synthetic
                        // fixtures + bare-aggregate-only data).
                        //
                        // TASK-0317: emit a `NUC_TRACE`-gated diagnostic
                        // on the fall-back. Mirrors the cycle-134
                        // (TASK-0315) defence on
                        // `order_halo_strip_bounds_by_data_dim`. The
                        // TASK-0301 axis-mapping defence (data-dim-aware
                        // emit) is BYPASSED on this call; trace makes
                        // the bypass observable so a future regression
                        // masked by synthetic-fixture coverage is not
                        // silent. Production callers always observe
                        // accesses; this trace fires only on synthetic
                        // / bare-aggregate-only data paths.
                        // cycle-135 architect P2-1 fold-back: mirror
                        // cycle-134's `entry = absent | Some(empty)`
                        // disambiguator. `compute_partition_bounds_with_
                        // dim_prefix` returns None on exactly two paths
                        // (dim_iv_map.get is None, or per_dim.is_empty())
                        // so the 2-arm match is exhaustive at the trace
                        // site by the helper's contract — parallel to
                        // cycle-134's identical disambiguator shape.
                        crate::nuc_trace!(
                            "transfer_inject::rewrite_partition_tiles_inner: fall-back to \
                             partition_axis_order nest-order emit (data={data:?}, worker={w:?}); \
                             compute_partition_bounds_with_dim_prefix returned None — \
                             data_dim_iv_map entry is {entry} — \
                             TASK-0301 axis-mapping defence BYPASSED for this call \
                             (expected only on synthetic fixtures via DataflowEdge::new or \
                             bare-aggregate-only data symbols)",
                            data = x.data,
                            entry = match data_dim_iv_map.get(&x.data) {
                                None => "absent",
                                Some(_) => "Some(empty)",
                            },
                        );
                        let mut tmp: Vec<(IterVar, std::ops::Range<i64>)> = Vec::new();
                        for iv in partition_axis_order {
                            if let Some(per_worker) = partition_ranges.get(iv) {
                                if let Some(range) = per_worker.get(&w) {
                                    tmp.push((*iv, range.clone()));
                                }
                            }
                        }
                        tmp
                    }
                };
                if !bounds.is_empty() {
                    x.tile = IterTile::new(bounds);
                }
            }
            ACFGNode::Xfer(x)
        }
        ACFGNode::Sequence(children) => ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| {
                    rewrite_partition_tiles_inner(
                        c,
                        partition_ranges,
                        partition_axis_order,
                        data_dim_iv_map,
                    )
                })
                .collect(),
        ),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(rewrite_partition_tiles_inner(
                *body,
                partition_ranges,
                partition_axis_order,
                data_dim_iv_map,
            )),
            block_tag,
        },
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => leaf,
    }
}

// --------------------------------------------------------------------
// TASK-0301 / TASK-0302: data → per-dim iter-vars indexing
// (axis-mapping filter input)
// --------------------------------------------------------------------

/// Build the per-data, per-dim iter-var sets observed across every
/// indexed access on the symbol.
///
/// Returns `BTreeMap<DataId, Vec<BTreeSet<IterVar>>>` where the inner
/// `Vec` is indexed by *data dim position* (the position in
/// [`DataAccess::indices`]) and each set is the union of iter-vars
/// referenced by the index expression at that dim across all observed
/// accesses on the data. The Vec length equals the maximum index-count
/// seen across accesses; ragged accesses (one site indexing fewer dims
/// than another — pathological, not seen in canonical AlgoIR) are
/// tolerated.
///
/// TASK-0302 generalisation of the TASK-0301 per-symbol-union map: the
/// per-dim positions are what
/// [`compute_partition_bounds_with_dim_prefix`] needs to enforce the
/// *contiguous-prefix* invariant that `wait_slice`'s axis-mapping
/// convention rests on. The per-symbol union the older code produced is
/// recoverable as `iter().flatten().copied().collect()` over a data's
/// Vec entry — kept implicit; no caller needs it any more.
///
/// Index-extraction semantics (unchanged from TASK-0301):
/// - `IrExpr::Ident(name)` leaves whose `name` resolves through
///   `name_iter_vars` to an `IterVar` contribute that iv to the
///   per-dim set.
/// - A non-`Ident` leaf (`IntLit`, `Neg`/`BinOp` over only consts) records
///   no iv — those axes are partition-invariant.
/// - Arithmetic over an iv (`y - 1`, `k + halo`) still records the iv
///   via recursion on the `BinOp`/`Neg` arms.
/// - `DataRef` / `Call` in index position is defensive descent (the
///   canonical grammar does not nest these inside index expressions, but
///   recursing keeps a future grammar widening from silently dropping
///   ivs).
///
/// Symbols seen only as bare-aggregate accesses (`save_c(c)` style:
/// `indices.is_empty()`) get an entry with an *empty* `Vec` — the same
/// fall-back trigger
/// [`compute_partition_bounds_with_dim_prefix`] treats as "no observed
/// dim info, pre-TASK-0301 contract applies". Symbols not seen on any
/// access at all get no map entry, same fall-back path.
///
/// Structurally independent of `extend_xfer_tiles_for_halo` (which keys
/// off the `halo_widths` sidecar) — the two are only required to be
/// *consistent*, not coupled.
fn collect_data_dim_iv_map(
    root: &ACFGNode,
    name_iter_vars: &BTreeMap<String, IterVar>,
) -> BTreeMap<DataId, Vec<BTreeSet<IterVar>>> {
    let mut out: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    walk_data_dim_iv_map(root, name_iter_vars, &mut out);
    out
}

fn walk_data_dim_iv_map(
    node: &ACFGNode,
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) {
    match node {
        ACFGNode::Operation(op) => {
            for edge in &op.dataflow.edges {
                for access in &edge.data_in_access {
                    record_access_per_dim(access, name_iter_vars, out);
                }
                if let Some(access) = &edge.data_out_access {
                    record_access_per_dim(access, name_iter_vars, out);
                }
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                walk_data_dim_iv_map(c, name_iter_vars, out);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            walk_data_dim_iv_map(body, name_iter_vars, out);
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

fn record_access_per_dim(
    access: &DataAccess,
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) {
    let entry = out.entry(access.data).or_default();
    if entry.len() < access.indices.len() {
        entry.resize(access.indices.len(), BTreeSet::new());
    }
    for (dim, ix_expr) in access.indices.iter().enumerate() {
        collect_ivs_from_expr(ix_expr, name_iter_vars, &mut entry[dim]);
    }
}

/// TASK-0302 helper: apply the data-dim-prefix filter to derive per-Xfer
/// `IterTile::bounds` from the partition state.
///
/// Returns:
/// - `None` — no observed dim info for this data (no access on the
///   symbol, OR every access is a bare-aggregate `indices.is_empty()`
///   reference, OR a synthetic `DataflowEdge::new` fixture with empty
///   indices). The caller must then fall back to the *pre-TASK-0301*
///   "every partitioned iv applies" behaviour. This contract preserves
///   every shipped pre-TASK-0301 test fixture verbatim.
/// - `Some(bounds)` — applied the per-dim contiguous-prefix logic. The
///   returned Vec is in *data-dim* order. An empty Vec means the data's
///   partition-covered dims do not form a contiguous prefix starting at
///   dim 0 (the 07-matmul `b[k][j]` × `partition=blocks2d(i,j)` case:
///   only dim 1 is partition-covered, dim 0 is not, so no safe slice is
///   possible without an `iv → dim` mapping on `wait_slice`) — the
///   caller drops the tile to whole-array broadcast, the same shape
///   TASK-0301's i-membership filter already used for the
///   `partition=workers(i)` × `b[k][j]` 1D case.
///
/// ### Why dim-order, not partition_axis_order
///
/// `wait_slice`'s axis-mapping convention is `tile.bounds[i].iter_var
/// ↔ data.dim[i]` (row-major / nest-order). Emitting in dim order is
/// what makes that convention hold. The two orders coincide on every
/// shipped M5 cell because every cell's partition axes ARE the data's
/// dim-0 (`workers` on `[y][x]`, `[i][k]`, `[i][j]`) OR dim-0+dim-1
/// nest-order (`blocks2d` on `[y][x]`), so this rewrite is
/// observationally identical for them. The change bites when partition
/// axes are *sparse over the data's dim space* — the 07-matmul
/// `b[k][j]` × `partition=blocks2d(i,j)` shape that motivates this task.
///
/// ### Ambiguity handling
///
/// If a single dim is observed indexed by *multiple* partitioned ivs
/// (e.g. `a[i+j]` where both `i` and `j` are partitioned), the slicing
/// shape is ambiguous (which partition is "the" partition of this dim?).
/// The conservative choice is `Some(empty)` → whole-array broadcast.
/// Today's grammar + every shipped schedule keeps the iv-per-dim
/// cardinality at 1; this branch is defensive.
///
/// `partition_axis_order` is consulted only when `dim_iv_map.get(&data)`
/// is `None` (fall-back path in the caller), so it is intentionally NOT
/// a parameter to this function.
fn compute_partition_bounds_with_dim_prefix(
    data: DataId,
    dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    worker: WorkerId,
) -> Option<Vec<(IterVar, std::ops::Range<i64>)>> {
    let per_dim = dim_iv_map.get(&data)?;
    if per_dim.is_empty() {
        return None;
    }
    // Resolve per-dim coverage: at each dim, find the (unique) partitioned
    // iv that indexes it on this worker.
    let mut per_dim_cover: Vec<Option<(IterVar, std::ops::Range<i64>)>> =
        Vec::with_capacity(per_dim.len());
    for iv_set in per_dim {
        let partitioned: Vec<IterVar> = iv_set
            .iter()
            .copied()
            .filter(|iv| partition_ranges.contains_key(iv))
            .collect();
        match partitioned.len() {
            0 => per_dim_cover.push(None),
            1 => {
                let iv = partitioned[0];
                let range = partition_ranges
                    .get(&iv)
                    .and_then(|m| m.get(&worker))
                    .cloned();
                per_dim_cover.push(range.map(|r| (iv, r)));
            }
            _ => {
                // Ambiguous (multiple partitioned ivs at the same dim).
                // Defensive whole-array broadcast.
                return Some(Vec::new());
            }
        }
    }
    // Walk dims in order, accept contiguous prefix from dim 0; reject
    // sparse coverage (a covered dim AFTER a hole).
    let mut bounds: Vec<(IterVar, std::ops::Range<i64>)> = Vec::new();
    let mut hit_hole = false;
    for slot in per_dim_cover {
        match (slot, hit_hole) {
            (Some(b), false) => bounds.push(b),
            (None, _) => hit_hole = true,
            (Some(_), true) => {
                // Sparse coverage (e.g. b[k][j] with partition on j but
                // not on k): wait_slice's dim-i ↔ bounds[i] convention
                // would silently mis-map. Drop to whole-array.
                return Some(Vec::new());
            }
        }
    }
    Some(bounds)
}

/// TASK-0306: order an `inject_halo_strip_xfers` strip tile by data-dim
/// position (matching `wait_slice`'s `tile.bounds[i] ↔ data.dim[i]`
/// convention).
///
/// `inject_halo_strip_xfers` constructs each strip tile from the
/// partition-pair `(outer_iv, inner_iv)` and the per-axis halo band
/// arithmetic. The pre-cycle-133 emit hard-coded `[(outer_iv, ...),
/// (inner_iv, ...)]` — correct ONLY when the halo-bearing data is
/// indexed `[outer_iv][inner_iv]` (outer-axis-leading) AND both ivs
/// form a contiguous prefix of the data's dims. Every shipped schedule
/// (05/distributed-2d's `img_in[y][x]` × `partition=blocks2d(y, x)`)
/// is in that safe regime; the two open shapes this helper guards
/// against are:
///
/// 1. **Inner-axis-leading partition** — data indexed `[inner_iv]
///    [outer_iv]`. `outer_dim = 1`, `inner_dim = 0`; the emit must
///    flip to `[(inner_iv, ...), (outer_iv, ...)]`.
/// 2. **Non-prefix data layout** — data indexed `[k][inner_iv]`
///    where `k` is not partitioned. `outer_dim = None`; the emit
///    must drop to a whole-array push (empty bounds) rather than
///    silently mis-map dim 0 to the outer iv.
///
/// ### Fall-back: no observed dim info
///
/// Synthetic test fixtures built via `DataflowEdge::new` carry empty
/// `data_in_access` indices, so `walk_data_dim_iv_map` records the
/// data with `Some(vec![])` (an empty per-dim Vec). Production
/// callers always observe accesses with non-empty indices (TASK-0150
/// populates them at `build_acfg` time). We treat both `None` (data
/// not in the map at all) and `Some(empty)` uniformly as "no
/// observed dim info" and return the pre-cycle-133 default ordering
/// so the existing `halo_strip_synth.rs` fixtures (positive_3x3 /
/// positive_2x2 / determinism / placement) stay byte-identical. The
/// `None` branch is defensive — no production caller reaches it.
///
/// ### Ambiguity (both ivs at same dim)
///
/// `a[outer_iv + inner_iv]` is theoretically possible (composite
/// index expression); not seen in canonical AlgoIR. Defensive drop to
/// whole-array (empty bounds), same policy as
/// [`compute_partition_bounds_with_dim_prefix`].
fn order_halo_strip_bounds_by_data_dim(
    data: DataId,
    outer_iv: IterVar,
    outer_range: std::ops::Range<i64>,
    inner_iv: IterVar,
    inner_range: std::ops::Range<i64>,
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) -> Vec<(IterVar, std::ops::Range<i64>)> {
    // Fall-back hot path: synthetic fixtures + the defensive
    // missing-entry case. Build the default order LAZILY (no clones
    // on the canonical-indexed-access branches below).
    //
    // TASK-0315: emit a `NUC_TRACE`-gated diagnostic on the fall-back
    // so a future engineer who unwittingly constructs a synthetic
    // fixture via `DataflowEdge::new` (empty `data_in_access`) gets
    // an observable signal that the cycle-133 axis-mapping defence
    // is BYPASSED on this call. Production callers always observe
    // accesses (`walk_data_dim_iv_map` records non-empty per-dim
    // sets), so this trace fires only for synthetic test paths.
    let per_dim = match data_dim_iv_map.get(&data) {
        Some(p) if !p.is_empty() => p,
        _ => {
            crate::nuc_trace!(
                "transfer_inject::order_halo_strip_bounds_by_data_dim: fall-back to default \
                 order (data={data:?}, outer_iv={outer_iv:?}, inner_iv={inner_iv:?}); \
                 data_dim_iv_map entry is {entry} — axis-mapping defence BYPASSED for this \
                 call (expected only on synthetic fixtures built via DataflowEdge::new)",
                entry = match data_dim_iv_map.get(&data) {
                    None => "absent",
                    Some(_) => "Some(empty)",
                }
            );
            return vec![(outer_iv, outer_range), (inner_iv, inner_range)];
        }
    };
    let outer_dim = per_dim.iter().position(|s| s.contains(&outer_iv));
    let inner_dim = per_dim.iter().position(|s| s.contains(&inner_iv));
    match (outer_dim, inner_dim) {
        (Some(od), Some(id)) if od == id => Vec::new(),
        (Some(od), Some(id)) if od < id => {
            vec![(outer_iv, outer_range), (inner_iv, inner_range)]
        }
        (Some(_), Some(_)) => {
            vec![(inner_iv, inner_range), (outer_iv, outer_range)]
        }
        _ => Vec::new(),
    }
}

/// Recursively collect every `IrExpr::Ident(name)` whose `name` resolves
/// to an `IterVar` via `name_iter_vars`, accumulating into `out`. Const
/// idents (lookup misses) are silently ignored — they are partition-
/// invariant and don't contribute to axis-mapping.
fn collect_ivs_from_expr(
    expr: &IrExpr,
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeSet<IterVar>,
) {
    match expr {
        IrExpr::IntLit(_) => {}
        IrExpr::Ident(name) => {
            if let Some(iv) = name_iter_vars.get(name) {
                out.insert(*iv);
            }
        }
        IrExpr::Neg(inner) => collect_ivs_from_expr(inner, name_iter_vars, out),
        IrExpr::BinOp(_, lhs, rhs) => {
            collect_ivs_from_expr(lhs, name_iter_vars, out);
            collect_ivs_from_expr(rhs, name_iter_vars, out);
        }
        IrExpr::DataRef(ix_ref) => {
            // A nested DataRef in an index position is structurally
            // exotic (the AlgoIR grammar treats DataRef as a kernel
            // argument or RHS, not normally an index inside another
            // index). Descend defensively into its own indices so a
            // future grammar widening doesn't silently miss ivs.
            for ix in &ix_ref.indices {
                collect_ivs_from_expr(ix, name_iter_vars, out);
            }
        }
        IrExpr::Call { args, .. } => {
            // A Call in index position is also non-canonical; descend
            // for the same defensiveness as DataRef.
            for a in args {
                collect_ivs_from_expr(a, name_iter_vars, out);
            }
        }
    }
}

// --------------------------------------------------------------------
// TASK-0263 Stage 2 halo extension
// --------------------------------------------------------------------

/// Extend each [`XferPlaceholder`]'s `tile` axis range by the halo
/// width inferred for `(consumer_kernel, iter_var)` along that axis
/// (TASK-0263 Stage 2).
///
/// ### Why this is the right sink for halo widths
///
/// By the time `rewrite_partition_tiles` has run, every Xfer's tile
/// reflects the COMPUTE worker's partition slice (or the source range
/// when no partition applies). The halo is an additive correction:
/// "the kernel reading the slice ALSO reads `halo` rows of
/// neighbouring data". Applied here, the halo extension composes with
/// any partition policy (rows/blocks2d/workers) without each policy
/// having to know about halo.
///
/// ### Sidecar shape and the (DataId -> consumer KernelIds) join
///
/// `halo_widths` is keyed by `(KernelId, IterVar) -> u64`. An
/// XferPlaceholder carries `(DataId, IterVar)` in its tile. We bridge
/// the two by walking the ACFG once to collect, for each `DataId`, the
/// set of kernel ids that READ it (the `data_in` of every
/// `DataflowEdge`). The halo on a `(DataId, IterVar)` is then the
/// MAX of `halo_widths[kid][iv]` across all consumer kids — the union
/// of every reader's halo requirement. This is the only sound merge:
/// each transfer is one slice, used by every consumer at the
/// destination; the slice must cover the union of reads.
///
/// In practice every in-tree example has at most one consumer kernel
/// per data symbol; the max-across-consumers reduces to the single
/// entry. The variant is documented for correctness when a future
/// algorithm reads the same symbol from two different kernels.
///
/// ### Clamp policy
///
/// Extension is `lo' = lo - halo, hi' = hi + halo`. We clamp against
/// the iter-var's algorithm-source loop range expanded by halo —
/// `[source_lo - halo, source_hi + halo)`. That bounds the legitimate
/// data extent the loop would read in absence of partitioning. For
/// schedules where the per-worker band already abuts the source range
/// (the corner workers), the clamp is a no-op; for non-corner workers
/// it is always a no-op (the extension stays strictly inside the source
/// range expanded by halo).
///
/// Why not clamp against the data symbol's `ResolvedType::dims`: that
/// is the right semantic for a STRICTER clamp (don't read beyond data
/// extent), and is a natural follow-up. For every in-tree example, the
/// algorithm declares data wide enough that the (loop range ± halo)
/// stays within the data extent, so the looser clamp here is correct.
/// Filed as a future tightening on TASK-0263.
///
/// ### Bare-iv halo = 0 contract (forward-carry from TASK-0260)
///
/// `halo_widths` records an explicit `0` entry for every
/// `(kernel, iv)` the detector inspected with a bare-iv index. We
/// treat `0` as "no extension needed" (the natural arithmetic
/// identity). This honours the TASK-0260 Stage 1 lenient contract.
///
/// ### No-op when `halo_widths` is empty
///
/// An empty sidecar — every pre-Stage-2 example — short-circuits at
/// the top: no walk, no allocation, the input is returned unchanged.
fn extend_xfer_tiles_for_halo(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
) -> ACFGNode {
    if halo_widths.is_empty() {
        return node;
    }
    // Build the (DataId -> Set<KernelId>) consumer index by walking
    // the ACFG once. Operations' DataflowEdge::data_in lists every
    // DataId the edge.kernel reads.
    let mut consumers: BTreeMap<DataId, BTreeSet<KernelId>> = BTreeMap::new();
    collect_data_consumers(&node, &mut consumers);

    // Build the (IterVar -> source-loop-range) index by walking the
    // ACFG's Repeat nodes. The first occurrence wins (matches
    // `rewrite_partition_tiles`'s nest-order convention); a future
    // strip-mined inner Repeat that reuses the same IterVar would not
    // overwrite the outer source range.
    let mut source_ranges: BTreeMap<IterVar, std::ops::Range<i64>> = BTreeMap::new();
    collect_iter_var_source_ranges(&node, &mut source_ranges);

    extend_xfer_tiles_inner(node, halo_widths, &consumers, &source_ranges)
}

/// Walk `node` and union every `Operation`'s `data_in` into the
/// `(DataId -> KernelId)` consumer index. Read-only walk.
///
/// The kernel id we attribute to a read is the EDGE's kernel
/// (`edge.kernel`), not the enclosing `Operation.kernel` — they are
/// the same in the M1 single-edge-per-Operation shape, but explicit
/// is safer if a future multi-edge DAG lands.
fn collect_data_consumers(node: &ACFGNode, out: &mut BTreeMap<DataId, BTreeSet<KernelId>>) {
    match node {
        ACFGNode::Operation(op) => {
            for edge in &op.dataflow.edges {
                for d in &edge.data_in {
                    out.entry(*d).or_default().insert(edge.kernel);
                }
            }
        }
        ACFGNode::Repeat { body, .. } => collect_data_consumers(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_data_consumers(c, out);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Walk `node` and record, for each `Repeat::iter_var`, its source
/// `range`. First occurrence wins. Read-only walk.
///
/// "First" is the outermost occurrence under DFS pre-order. A future
/// `block_transform` strip-mine that reuses an IterVar across full +
/// partial split would land its inner-tile Repeats deeper in the tree;
/// taking the outer occurrence keeps the halo clamp anchored on the
/// SOURCE loop range (the algorithm's `for y : 1..H-1`), not on a
/// downstream-synthesised inner-tile range.
fn collect_iter_var_source_ranges(
    node: &ACFGNode,
    out: &mut BTreeMap<IterVar, std::ops::Range<i64>>,
) {
    match node {
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            ..
        } => {
            out.entry(*iter_var).or_insert_with(|| range.clone());
            collect_iter_var_source_ranges(body, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_iter_var_source_ranges(c, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Recursive worker for [`extend_xfer_tiles_for_halo`].
fn extend_xfer_tiles_inner(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    consumers: &BTreeMap<DataId, BTreeSet<KernelId>>,
    source_ranges: &BTreeMap<IterVar, std::ops::Range<i64>>,
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(mut x) => {
            // Look up the consumer kernels for this data symbol. If
            // none recorded (the data is never read in the ACFG —
            // e.g. an output written but not re-consumed), no halo
            // applies. Skip.
            let Some(kernel_ids) = consumers.get(&x.data) else {
                return ACFGNode::Xfer(x);
            };
            let mut new_bounds: Vec<(IterVar, std::ops::Range<i64>)> =
                Vec::with_capacity(x.tile.bounds.len());
            for (iv, range) in &x.tile.bounds {
                // Max halo across consumer kernels for this iv.
                let halo: u64 = kernel_ids
                    .iter()
                    .filter_map(|kid| halo_widths.get(kid))
                    .filter_map(|per_iv| per_iv.get(iv))
                    .copied()
                    .max()
                    .unwrap_or(0);
                if halo == 0 {
                    new_bounds.push((*iv, range.clone()));
                    continue;
                }
                // halo is u64; convert to i64. For all in-tree
                // examples halo widths are small (1 to a few), so the
                // overflow path is academic — we still guard with
                // try_into + saturating fallback to keep the function
                // total.
                let halo_i: i64 = halo.try_into().unwrap_or(i64::MAX);
                // Extend.
                let ext_lo = range.start.saturating_sub(halo_i);
                let ext_hi = range.end.saturating_add(halo_i);
                // Clamp to the iter-var's source loop range expanded by
                // halo. (See module docs for the rationale on this
                // looser clamp vs the data-extent clamp.)
                let (clamp_lo, clamp_hi) = match source_ranges.get(iv) {
                    Some(src) => (
                        src.start.saturating_sub(halo_i),
                        src.end.saturating_add(halo_i),
                    ),
                    // No source range recorded for this iv — happens
                    // if an Xfer carries an iv that has no Repeat in
                    // the ACFG (synthetic test fixtures). Skip the
                    // clamp; the extension is still applied.
                    None => (i64::MIN, i64::MAX),
                };
                let new_lo = ext_lo.max(clamp_lo);
                let new_hi = ext_hi.min(clamp_hi);
                new_bounds.push((*iv, new_lo..new_hi));
            }
            x.tile = IterTile::new(new_bounds);
            ACFGNode::Xfer(x)
        }
        ACFGNode::Sequence(children) => ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| extend_xfer_tiles_inner(c, halo_widths, consumers, source_ranges))
                .collect(),
        ),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(extend_xfer_tiles_inner(
                *body,
                halo_widths,
                consumers,
                source_ranges,
            )),
            block_tag,
        },
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => leaf,
    }
}

// --------------------------------------------------------------------
// TASK-0324 cycle-144 AC#2 — silent-elision-risk validator
// --------------------------------------------------------------------

/// Walk every Operation in `root` and reject the same-worker-pair
/// elision + consumer-reads-outside-local-slice shape with a typed
/// [`TransferInjectError::SameSetSilentElisionRisk`]. Called from
/// [`inject_transfers`] BEFORE the recursive emission walk so the
/// existing same-worker short-circuits fire only after this validator
/// has certified the elision as safe.
///
/// ## Two elision sites covered by one validator
///
/// The validator defends against two structurally identical same-
/// worker elision sites:
///
/// 1. **Whole-set elision** (grep-witness anchor:
///    `if producer_workers == &consumer_workers` inside
///    `build_waits_for_op` — `continue; no transfer`): fires when the
///    producer and consumer worker sets are equal.
///
/// 2. **Per-element fan-out elision** (grep-witness anchor: `if src
///    == dst` inside `build_waits_for_op`'s cartesian-product fan-
///    out — `continue;`): fires for every worker in
///    `producer_workers ∩ consumer_workers`, even when the sets are
///    not equal. Example: producer = {w0..w3}, consumer = {w0..w3,
///    w4} — the four self-pairs `(w_i, w_i)` are skipped per-element
///    while the cross pairs to/from w4 are emitted normally. Each
///    skipped self-pair has the same silent-miscompile risk profile
///    as a whole-set elision restricted to that worker.
///
/// The validator therefore fires when
/// `producer_workers ∩ consumer_workers` is non-empty — generalising
/// the original cycle-144 set-equality check to cover the per-element
/// case (TASK-0325 cycle-145 reviewer P1.1 fold-back from cycle 144).
///
/// ## Discriminator (axis-by-axis against producer's write pattern)
///
/// For every `(op, data_id)` pair where the consumer (`op`) and the
/// producer (`producers_by_data[data_id]`) share at least one common
/// worker, the elision the same-worker short-circuit performs is
/// correct iff for every axis `k` of the data:
///
/// - If the PRODUCER's `data_out_access.indices[k]` is a bare
///   [`IrExpr::Ident`] whose name resolves to an iv in
///   `partition_iter_vars` (i.e. axis `k` is partitioned at the
///   producer — worker `w` writes only its own partition's k-th slot),
///   then the CONSUMER's `data_in_access.indices[k]` MUST be a bare
///   [`IrExpr::Ident`] with the same name (the consumer must read its
///   own worker's k-th slot, not an arbitrary one). Same-name
///   comparison is sufficient because: (a) producer and consumer
///   share the same worker set; (b) when both ops live in the same
///   `Repeat`-named outer scope, the iv name resolves to the same
///   [`IterVar`] in both; (c) when the consumer's enclosing scope
///   uses a different iv name from the producer's (e.g. the
///   06/distributed2 `vm` vs producer's `hy`), the names differ AND
///   the read references a non-matching slice — both wrong reasons to
///   elide.
///
/// - If the producer's write index on axis `k` is NOT a bare Ident
///   matching a partition iv (e.g. a `IntLit`, an arithmetic
///   expression, or a bare Ident referencing a non-partition iv),
///   axis `k` is NOT partition-sliced at the producer and EVERY
///   worker owns the full k-th axis of the data — the consumer's
///   read on this axis can be anything.
///
/// ## Why per-axis against the producer's write pattern, not the
/// consumer's scope
///
/// An earlier formulation rejected any consumer-read iv that wasn't
/// the consumer's enclosing partition iv. That over-rejected the
/// accumulator-self-read shape `tmp[hy][hx] <-- hblur_acc(...,
/// tmp[hy][hx])` (06/distributed): the consumer reads `tmp[hy][hx]`
/// where `hy` IS the partition iv but `hx` is the inner sweep — yet
/// the producer ALSO writes `tmp[hy][hx]`, so worker `w_i` reads
/// EXACTLY what it wrote. The per-axis check sees `P_0 == C_0 ==
/// Ident("hy")` (axis 0 is partitioned and aligned) and `P_1 ==
/// Ident("hx") which is NOT a partition iv` (axis 1 is not
/// partitioned at all) — both safe.
///
/// For 06/distributed2 the same machinery fires correctly:
/// `P_0 == Ident("hy")` is a partition iv, but `C_0 == Ident("vm")` —
/// names differ, so the consumer reads a non-aligned slot on axis 0
/// → reject.
fn check_no_silent_elision_risk(
    root: &ACFGNode,
    producers_by_data: &BTreeMap<DataId, BTreeSet<WorkerId>>,
    partition_iter_vars: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
    name_data: &BTreeMap<String, DataId>,
) -> Result<(), TransferInjectError> {
    // Index producer-side write accesses for each DataId once. Per PRD
    // §6.2.1 single-assignment (enforced upstream by `algo::lower` —
    // see `LowerErrorKind::DoubleAssignment` in
    // `nucleus-compiler/src/algo/ir.rs:256-260`, grep-witness for the
    // single-assignment rule), every data symbol has at most ONE
    // writer Operation, modulo accumulator self-writes (an op whose
    // `data_in` and `data_out` are the same DataId at the same
    // indices, e.g. `c[i][j] <-- madd(c[i][j], ...)` — exercised by
    // 07-matmul's reference shape). Accumulator self-writes carry the
    // SAME `data_out_access` on every iteration by construction, so
    // the lexically-first occurrence is authoritative: the
    // `or_insert_with` below is sound only under (§6.2.1 single-
    // assignment) ∧ (accumulator-self-writes carry the same access on
    // every iteration). Reviewer P2.5 cycle-144 fold-back.
    let mut producer_writes: BTreeMap<DataId, DataAccess> = BTreeMap::new();
    collect_producer_writes(root, &mut producer_writes);

    check_no_silent_elision_risk_inner(
        root,
        &[],
        producers_by_data,
        partition_iter_vars,
        name_iter_vars,
        name_data,
        &producer_writes,
    )
}

fn collect_producer_writes(node: &ACFGNode, out: &mut BTreeMap<DataId, DataAccess>) {
    match node {
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_producer_writes(c, out);
            }
        }
        ACFGNode::Repeat { body, .. } => collect_producer_writes(body, out),
        ACFGNode::Operation(op) => {
            for edge in &op.dataflow.edges {
                if let Some(out_access) = &edge.data_out_access {
                    out.entry(out_access.data).or_insert_with(|| out_access.clone());
                }
            }
        }
        ACFGNode::Xfer(_) | ACFGNode::Sync(_) => {}
    }
}

fn check_no_silent_elision_risk_inner(
    node: &ACFGNode,
    enclosing_ivs: &[IterVar],
    producers_by_data: &BTreeMap<DataId, BTreeSet<WorkerId>>,
    partition_iter_vars: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
    name_data: &BTreeMap<String, DataId>,
    producer_writes: &BTreeMap<DataId, DataAccess>,
) -> Result<(), TransferInjectError> {
    match node {
        ACFGNode::Sequence(children) => {
            for c in children {
                check_no_silent_elision_risk_inner(
                    c,
                    enclosing_ivs,
                    producers_by_data,
                    partition_iter_vars,
                    name_iter_vars,
                    name_data,
                    producer_writes,
                )?;
            }
            Ok(())
        }
        ACFGNode::Repeat { iter_var, body, .. } => {
            let mut nested: Vec<IterVar> = enclosing_ivs.to_vec();
            nested.push(*iter_var);
            check_no_silent_elision_risk_inner(
                body,
                &nested,
                producers_by_data,
                partition_iter_vars,
                name_iter_vars,
                name_data,
                producer_writes,
            )
        }
        ACFGNode::Operation(op) => check_op_no_silent_elision_risk(
            op,
            enclosing_ivs,
            producers_by_data,
            partition_iter_vars,
            name_iter_vars,
            name_data,
            producer_writes,
        ),
        ACFGNode::Xfer(_) | ACFGNode::Sync(_) => Ok(()),
    }
}

fn check_op_no_silent_elision_risk(
    op: &Operation,
    enclosing_ivs: &[IterVar],
    producers_by_data: &BTreeMap<DataId, BTreeSet<WorkerId>>,
    partition_iter_vars: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
    name_data: &BTreeMap<String, DataId>,
    producer_writes: &BTreeMap<DataId, DataAccess>,
) -> Result<(), TransferInjectError> {
    let consumer_workers: BTreeSet<WorkerId> = op.workers.clone();

    // Partition ivs that are ACTUALLY in the consumer's enclosing scope.
    // Used only for the "no-partition active" short-circuit (clause
    // (1) below); the per-axis check (clause (2)) uses the global
    // `partition_iter_vars` against the producer's write pattern.
    let enclosing_partition_ivs: BTreeSet<IterVar> = enclosing_ivs
        .iter()
        .copied()
        .filter(|iv| partition_iter_vars.contains(iv))
        .collect();

    let mut seen: BTreeSet<DataId> = BTreeSet::new();
    for edge in &op.dataflow.edges {
        for (i, &data_id) in edge.data_in.iter().enumerate() {
            if !seen.insert(data_id) {
                continue;
            }
            let producer_workers = match producers_by_data.get(&data_id) {
                Some(p) => p,
                None => continue, // No recorded producer.
            };
            // TASK-0325 cycle-145: both elision sites are checked
            // together. The whole-set short-circuit (grep-witness:
            // `if producer_workers == &consumer_workers` inside
            // `build_waits_for_op`) fires for the whole-set case.
            // The per-element skip (grep-witness: `if src == dst`
            // inside `build_waits_for_op`'s cartesian-product fan-out)
            // fires for EVERY worker in the intersection of
            // producer_workers and consumer_workers — that is, even
            // when the sets are not equal (e.g. producer={w0..w3},
            // consumer={w0..w3, w4}: the four self-pairs (w_i, w_i)
            // are skipped per-element while the cross pairs to/from
            // w4 are emitted normally). Both elisions risk a silent
            // miscompile under the same per-axis safety conditions,
            // so the validator fires when the intersection is non-
            // empty.
            let same_worker_set: BTreeSet<WorkerId> = producer_workers
                .intersection(&consumer_workers)
                .copied()
                .collect();
            if same_worker_set.is_empty() {
                continue; // No same-worker elision happens anywhere.
            }

            // Clause (1): no partition iv active on the consumer's
            // enclosing scope → every worker owns the full data →
            // safe elision.
            if enclosing_partition_ivs.is_empty() {
                continue;
            }

            // Need the producer's write access to drive the per-axis
            // discriminator. If we don't have it (synthetic test
            // fixtures that omit data_out_access, or a producer not
            // captured by `collect_producer_writes`), conservatively
            // continue: the validator is additive over the existing
            // behaviour, and missing producer access means we cannot
            // distinguish safe from unsafe — falling back to the
            // pre-TASK-0324 elision is acceptable for synthetic-only
            // fixtures because those don't exercise the silent-
            // miscompile code path in production.
            let prod_access = match producer_writes.get(&data_id) {
                Some(p) => p,
                None => continue,
            };

            // Clause (2): for each axis k, if the producer's write
            // index P_k is a bare Ident matching a partition iv,
            // require the consumer's read index C_k to be a bare
            // Ident with the SAME name. Iterate over every parallel
            // consumer access for this data; ALL must satisfy.
            let mut first_unsafe_reason: Option<String> = None;

            'outer: for access in edge.data_in_access.iter() {
                if access.data != data_id {
                    continue;
                }

                // If the producer carries indices but the consumer
                // carries none (a whole-array read of partitioned
                // data), reject — the worker does NOT own the whole
                // array.
                if access.indices.is_empty() && !prod_access.indices.is_empty() {
                    // Determine if ANY producer axis is a partition iv;
                    // if all axes are non-partition (so producer wrote
                    // whole-array), the elision is still safe.
                    let any_partitioned_axis = prod_access
                        .indices
                        .iter()
                        .any(|p| ident_iv_in_set(p, partition_iter_vars, name_iter_vars).is_some());
                    if any_partitioned_axis {
                        first_unsafe_reason = Some(format!(
                            "consumer reads data as a whole array (no indices) while \
                             the producer writes with a partition iv on at least one \
                             axis (producer indices: {:?}); each worker owns only \
                             its partition slice",
                            prod_access.indices
                        ));
                        break;
                    } else {
                        continue;
                    }
                }

                // Per-axis check.
                let n_axes = prod_access.indices.len().min(access.indices.len());
                for k in 0..n_axes {
                    let p_iv =
                        ident_iv_in_set(&prod_access.indices[k], partition_iter_vars, name_iter_vars);
                    let Some(p_iv) = p_iv else {
                        // Producer's axis-k index is not a bare Ident
                        // matching a partition iv. The cases are:
                        //   - `IntLit(c)` / non-iv-`Ident` / `Neg(...)`:
                        //     axis k is whole-array at every worker
                        //     (the producer writes the same slot
                        //     regardless of partition) → no
                        //     constraint on consumer's axis-k read.
                        //   - `BinOp(...)` involving a partition iv
                        //     (e.g. `tmp[hy*2][hx]`, `tmp[hy+1][hx]`):
                        //     CONSERVATIVELY-NOT-REJECTED here. The
                        //     access IS partition-sliced semantically
                        //     (the worker writes only its own
                        //     transformed range), but the per-axis
                        //     check skips this axis and treats it as
                        //     unconstrained. No in-tree schedule
                        //     today exercises this shape, so the
                        //     under-conservative path is dormant —
                        //     filed as TASK-0326 (reviewer P1.3
                        //     cycle-144 fold-back).
                        //   - `Call(...)` / `DataRef(...)`: not used
                        //     as a producer index in any algorithm
                        //     today; rejected upstream by `algo::
                        //     lower` before reaching this pass.
                        continue;
                    };
                    // Producer's axis k IS partition-sliced. The
                    // consumer's axis-k read must be a bare Ident with
                    // the same partition iv name.
                    let c_iv =
                        ident_iv_in_set(&access.indices[k], partition_iter_vars, name_iter_vars);
                    if c_iv != Some(p_iv) {
                        first_unsafe_reason = Some(format!(
                            "axis {k} is partition-sliced at the producer (writes \
                             at {:?}, partition iv {:?}); consumer reads at \
                             {:?} which does not match — worker reads a slice \
                             it does not own",
                            prod_access.indices[k], p_iv, access.indices[k]
                        ));
                        break 'outer;
                    }
                }
            }

            if let Some(reason) = first_unsafe_reason {
                let data_name = name_data
                    .iter()
                    .find_map(|(n, id)| (*id == data_id).then_some(n.as_str()))
                    .unwrap_or("<unknown>");
                // TASK-0325 cycle-145: distinguish the whole-set
                // elision shape (grep-witness anchor:
                // `if producer_workers == &consumer_workers`) from
                // the partial-overlap per-element elision shape
                // (grep-witness anchor: `if src == dst` inside
                // `build_waits_for_op`'s cartesian-product fan-out)
                // in the error message. The variant remains
                // SameSetSilentElisionRisk because the underlying
                // defect class (same-worker self-pair elision under
                // an unsafe consumer access pattern) is identical;
                // only the shape that produces it differs.
                let same_worker_kind = if producer_workers == &consumer_workers {
                    format!(
                        "producer worker set and consumer worker set are \
                         identical ({consumer_workers:?})"
                    )
                } else {
                    format!(
                        "producer worker set ({producer_workers:?}) and consumer \
                         worker set ({consumer_workers:?}) overlap on the \
                         same-worker pairs {same_worker_set:?}, which the \
                         cartesian-product fan-out skips per-element"
                    )
                };
                let message = format!(
                    "data `{data_name}` (id {data_id:?}, edge.data_in index {i}): \
                     {same_worker_kind}, AND {reason}. Without a cross-worker \
                     transfer this elides into a silent miscompile — see TASK-0324 \
                     (set-equality elision) and TASK-0325 (per-element fan-out \
                     elision). The diagnose-first guard (AC#2) lands ahead of the \
                     cross-worker codegen extension (AC#3); kernels.rs is \
                     unchanged. Remove the same-set placement, switch to a \
                     different partition iv, or wait for AC#3."
                );
                return Err(TransferInjectError::SameSetSilentElisionRisk {
                    data: data_id,
                    message,
                });
            }
        }
    }
    Ok(())
}

/// If `expr` is `IrExpr::Ident(name)` AND `name_iter_vars[name]` is in
/// `set`, return `Some(IterVar)`. Otherwise `None`. Used by the
/// per-axis discriminator to recognise partition-iv indices.
fn ident_iv_in_set(
    expr: &IrExpr,
    set: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
) -> Option<IterVar> {
    match expr {
        IrExpr::Ident(name) => {
            let iv = name_iter_vars.get(name).copied()?;
            if set.contains(&iv) {
                Some(iv)
            } else {
                None
            }
        }
        _ => None,
    }
}

// --------------------------------------------------------------------
// Wait construction
// --------------------------------------------------------------------

/// For each cross-worker read in `op`, produce a Wait placeholder.
/// Multiple distinct reads of the same data symbol in one op yield
/// ONE Wait — the consumer needs one rendezvous per (op, data, src)
/// triple, not one per read.
fn build_waits_for_op(
    op: &Operation,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
) -> Vec<XferPlaceholder> {
    let consumer_workers: BTreeSet<WorkerId> = op.workers.clone();
    let mut seen: BTreeSet<DataId> = BTreeSet::new();
    let mut out = Vec::new();

    for edge in &op.dataflow.edges {
        for &data_id in &edge.data_in {
            if !seen.insert(data_id) {
                continue;
            }
            let producer_workers = match ctx.producers_by_data.get(&data_id) {
                Some(p) => p,
                None => continue, // No recorded producer (e.g. unread const data).
            };
            if producer_workers == &consumer_workers {
                // Same worker set on both sides — intra-worker dataflow.
                //
                // TASK-0324 cycle-144 AC#2: the SILENT-MISCOMPILE arm
                // of this short-circuit (consumer reads a slice the
                // local producer does NOT own) is rejected up-front by
                // `check_no_silent_elision_risk` (called at the top of
                // `inject_transfers`). Reaching this `continue` means
                // the validator has already certified the elision as
                // safe — either there is no partition iv active on the
                // consumer's enclosing scope (each worker owns the
                // full data) OR every consumer read index for this
                // data is a bare `Ident(X)` where X is the partition
                // iv on the consumer's enclosing scope (e.g. 13-cnn-
                // inference/batch_parallel `feat1[n]` with `loop n :
                // partition=workers` — reader iv == partition iv, each
                // worker reads only the slice it wrote).
                //
                // The "compute worker = dst" fallback paragraph this
                // module's header once claimed for the N-to-M case
                // does NOT exist for the same-set case; the structural
                // code path is `continue; no transfer`. AC#3 of
                // TASK-0324 will replace the AC#2 fail-loud with the
                // actual cross-worker `tmp` codegen (N-to-N broadcast-
                // of-gather) and lift the validator's rejection.
                //
                // TASK-0325 cycle-145: this short-circuit is one of
                // TWO same-worker elision sites in this function. The
                // sibling (`if src == dst` inside the cartesian-
                // product fan-out below) elides per-pair when the
                // sets are NOT equal but share at least one worker.
                // Both sites are defended by the same validator
                // (`check_no_silent_elision_risk` fires on non-empty
                // worker-set intersection).
                continue;
            }
            let policy = ctx
                .policies_by_data
                .get(&data_id)
                .copied()
                .unwrap_or_default();

            // TASK-0117 fan-out: emit one Wait per (src-worker,
            // dst-worker) member of the cartesian product of the
            // producer and consumer worker entities, skipping any
            // same-worker pair. Each pair gets its own fresh `seq` so
            // the projection's per-worker EventLists carry distinct
            // Pushes/Waits, and the backend can allocate per-pair
            // slots without collision.
            //
            // The pair's `tile` is the enclosing iteration tile with
            // any partitioned axis (TASK-0212) rewritten to the
            // *compute* worker's slice. The "compute worker" for a
            // pair is the worker in the multi-worker side: for a
            // (host -> {w0..w3}) pair to w_i, compute = w_i; for a
            // ({w0..w3} -> host) pair from w_i, compute = w_i. When
            // both sides are size 1, the source range survives
            // unchanged (no partition meaning) — this is exactly the
            // pre-TASK-0117 behaviour for {host} -> {single worker}
            // transfers.
            //
            // Determinism: BTreeSet iterates in sorted WorkerId
            // order; the cartesian-product enumeration is therefore
            // a deterministic function of (producer_workers,
            // consumer_workers). Seq tags are assigned monotonically
            // by `state.fresh_seq()` — same input ⇒ same seqs.
            // Emit one Wait per (src, dst) member of the cartesian
            // product. The per-pair `tile` is initialised here from
            // `enclosing_tile` (the construction-site semantic the
            // pre-TASK-0117 single-pair path used); the final
            // per-worker tile is set by `rewrite_partition_tiles`
            // at the end of `inject_transfers`, after the hoist +
            // splice passes have positioned the placeholders. Doing
            // the partition rewrite as a separate sink keeps the
            // hoist invariant (TASK-0151's loop-invariant tile
            // rewrite) untouched.
            for &src in producer_workers.iter() {
                for &dst in consumer_workers.iter() {
                    if src == dst {
                        // TASK-0325 cycle-145: per-element same-
                        // worker elision. Skipping this pair is the
                        // structural sibling of the whole-set short-
                        // circuit above (`if producer_workers ==
                        // &consumer_workers`); the SILENT-MISCOMPILE
                        // arm of this skip (consumer reads a slice
                        // the local producer does NOT own, under
                        // partial worker-set overlap) is rejected
                        // up-front by `check_no_silent_elision_risk`,
                        // which fires when the intersection is non-
                        // empty (so it covers this pair-skip when
                        // the wider sets aren't equal). Reaching
                        // this `continue` means the validator
                        // certified the per-pair elision as safe.
                        continue;
                    }
                    out.push(XferPlaceholder {
                        role: XferRole::Wait,
                        src,
                        dst,
                        data: data_id,
                        tile: IterTile::new(enclosing_tile.to_vec()),
                        seq: state.fresh_seq(),
                        policy,
                    });
                }
            }
        }
    }

    out
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Convert a [`WorkerEntity`] (BTreeSet<String>) to a
/// `BTreeSet<WorkerId>` using the ACFG's name table. Skip names that
/// aren't in the table — that would be a link-pass invariant
/// violation; we don't loudly panic so downstream tests that build
/// synthetic ACFGs can still feed an empty/partial name table.
fn entity_to_workerid_set(
    entity: &WorkerEntity,
    name_workers: &BTreeMap<String, WorkerId>,
) -> BTreeSet<WorkerId> {
    entity
        .0
        .iter()
        .filter_map(|name| name_workers.get(name).copied())
        .collect()
}

/// Convert a [`ResolvedTransferDirective`]'s option list into a
/// [`TransferPolicy`]. Multiple options in one directive are walked
/// in order; later options override earlier ones for the same field.
///
/// The defaults match [`TransferPolicy::default`]: synchronous, buffer
/// 1, notify-default.
fn policy_from_directive(dir: &ResolvedTransferDirective) -> TransferPolicy {
    let mut p = TransferPolicy::default();
    for opt in &dir.options {
        match opt {
            ResolvedTransferOption::Sync => p.synchronous = true,
            ResolvedTransferOption::Async => p.synchronous = false,
            ResolvedTransferOption::Buffer(n) => p.buffer = *n,
            ResolvedTransferOption::Notify(k) => p.notify = NotifyMode::from(*k),
        }
    }
    p
}

/// Return the data symbol this Operation writes, if any.
/// At M1 a [`crate::acfg::DataflowDag`] holds one edge per firing; we
/// take the first edge's `data_out`. If a future multi-edge DAG lands,
/// this helper widens to a `Vec<DataId>` and callers adjust.
fn output_data(op: &Operation) -> Option<DataId> {
    op.dataflow.edges.first().and_then(|e| e.data_out)
}

/// Update the `last_writer` map with `op`'s output, if any.
fn update_writer(state: &mut State, op: &Operation) {
    if let Some(d) = output_data(op) {
        state.last_writer.insert(d, op.workers.clone());
    }
}

/// True iff `prev` is an `Xfer` placeholder whose `(role, src, dst,
/// data, tile)` matches `cand`. Used for idempotence: re-running the
/// pass must not duplicate an already-present placeholder.
fn is_duplicate_xfer(prev: Option<&ACFGNode>, cand: &XferPlaceholder) -> bool {
    match prev {
        Some(ACFGNode::Xfer(existing)) => {
            existing.role == cand.role
                && existing.src == cand.src
                && existing.dst == cand.dst
                && existing.data == cand.data
                && existing.tile == cand.tile
        }
        _ => false,
    }
}

// --------------------------------------------------------------------
// TASK-0289 cycle 114a halo-strip Push/Wait synthesis
// --------------------------------------------------------------------

/// Synthesise cross-worker halo-strip Push/Wait pairs between N/S/E/W
/// neighbour cells in a 2D worker grid established by
/// `partition=blocks2d` (TASK-0289 AC#1).
///
/// ### Why this lives at the tail of `inject_transfers`
///
/// `rewrite_partition_tiles` walks every `Xfer` and unconditionally
/// rewrites the tile from the partition sidecar (compute-worker rule);
/// `extend_xfer_tiles_for_halo` walks every `Xfer` and extends each tile
/// axis by the consumer kernel's halo width. Both rewrite in-place. A
/// synthesised halo-strip transfer carries a precisely-bounded
/// `[lo .. hi)` per axis (the strip-side of the neighbour's band, NOT
/// the worker's own partition slice + halo); if it ran BEFORE those
/// passes, the partition rewrite would replace its tile with the
/// src-worker's full band and the halo extension would re-pad it from
/// the wrong base. Running AFTER lets the strip-tile survive verbatim.
///
/// ### Why we synthesise BOTH Push and Wait pre-paired (instead of
/// only the Wait and letting `splice_pushes_global` produce the Push)
///
/// `splice_pushes_global` finds the matching Push location by walking
/// for the data symbol's PRODUCER Operation in the ACFG — but for a
/// halo strip on `img_in` (host-loaded), the producer is the host's
/// `load_image`, not the neighbour worker. Letting splice infer the
/// Push location would mis-route the Push to the host's scope; the
/// per-worker projection then emits a Push on `host`, not on the
/// neighbour worker. Synthesising both endpoints with a shared
/// `SeqTag` BEFORE splice runs would also be wrong (splice already
/// ran above by this point in the chain). So we run here, post-splice,
/// and create the pair with a `state.fresh_seq()` SeqTag pulled from
/// the shared monotonic counter — guaranteeing no SeqTag collision
/// with any pair already in the tree.
///
/// ### Per-Wait routing
///
/// The per-worker projection (`crate::passes::petri_to_events::emit_xfer`)
/// emits a Push event into `x.src`'s EventList and a Wait event into
/// `x.dst`'s EventList. So placing both Push and Wait nodes in the
/// SAME ACFG `Sequence` is fine: each worker's EventList picks up only
/// the endpoint matching its WorkerId. We insert the pairs as
/// siblings into whatever `Sequence` contains the outer Repeat for
/// `outer_iv`, AFTER the last producing Operation in that Sequence
/// (TASK-0290 cycle 114b — see `prepend_strip_pairs` for the
/// placement rule and fallback).
///
/// ### Strip tile math
///
/// For a worker at grid `(row, col)` with y-band `[y_lo, y_hi)`,
/// x-band `[x_lo, x_hi)`, and per-axis halo `h_y` / `h_x`:
///
/// - **N-strip** (received FROM `(row-1, col)`):
///   y in `[y_lo - h_y, y_lo)`, x in `[x_lo, x_hi)`
/// - **S-strip** (received FROM `(row+1, col)`):
///   y in `[y_hi, y_hi + h_y)`, x in `[x_lo, x_hi)`
/// - **W-strip** (received FROM `(row, col-1)`):
///   y in `[y_lo, y_hi)`, x in `[x_lo - h_x, x_lo)`
/// - **E-strip** (received FROM `(row, col+1)`):
///   y in `[y_lo, y_hi)`, x in `[x_hi, x_hi + h_x)`
///
/// Workers on an edge of the grid skip the off-grid neighbour.
/// Corners (NE/NW/SE/SW) are excluded in this first cut per the
/// TASK-0289 brief — the diagonal halo cell would require a separate
/// (worker, neighbour-diagonal) pair to be sound; filed as a follow-up.
///
/// ### Honest limitations (this cycle)
///
/// - **Placement**: pairs are PREPENDED to the parent Sequence that
///   contains the outer Repeat. For a single-pass stencil (no time
///   loop) this lands them at top-level. For a multi-pass / time-step
///   stencil, a future task should refine placement to "inside the
///   timestep Repeat, before the partitioned outer Repeat" so the
///   halo exchange fires per timestep. The current placement is
///   correct for the synthetic unit test and not yet exercised by
///   any shipped schedule (AC#3 short-circuit holds).
///
/// - **Not idempotent on re-run** when partition_pairs is non-empty.
///   On a re-run of `inject_transfers`, (1)
///   `rewrite_partition_tiles` clobbers strip tiles before this
///   synthesis sees them (its compute-worker rule replaces strip
///   with src's full partition slice) and (2)
///   `splice_pushes_for_waits` (inside `inject_in_sequence`)
///   splices a NEW Push for every halo-strip Wait it sees, because
///   the existing Pushes sit BEFORE the producer load Op (outside
///   its immediate-successor dedupe window). No production driver
///   path re-runs `inject_transfers`; the shipped-schedule
///   idempotence test (`tests/transfer_inject.rs::
///   idempotent_on_synthetic_two_worker_case`) stays green because
///   partition_pairs is empty for every shipped schedule and the
///   AC#3 guard short-circuits this entire function. Filed
///   forward-carried on TASK-0290.
///
/// - **No deduplication against existing halo Pushes**: if a later
///   pass adds halo pairs from a different join, two pairs for the
///   same `(src, dst, data, tile)` could co-exist. Today no such
///   later pass exists; the unit test asserts the exact pair count.
///
/// - **Policy**: each synthesised pair inherits the policy of its
///   data symbol from `policies_by_data` (the schedule's
///   `transfer D : ...` directive), falling back to
///   `TransferPolicy::default()` (sync, buffer=1, notify-default) when
///   the symbol has no directive. Matches the convention used by
///   `build_waits_for_op` for regular Push/Wait pairs.
///
/// ### Determinism
///
/// All iteration is over `BTreeMap` / `BTreeSet` (numeric order). The
/// per-axis emit order is fixed (N, S, W, E). SeqTags come from the
/// shared `state` counter in DFS-deterministic visit order. Same
/// input ⇒ byte-identical output.
#[allow(clippy::too_many_arguments)]
fn inject_halo_strip_xfers(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    partition_pairs: &BTreeMap<IterVar, IterVar>,
    grid_shape_for_outer_iv: &BTreeMap<IterVar, (u32, u32)>,
    partition_worker_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    policies_by_data: &BTreeMap<DataId, TransferPolicy>,
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    state: &mut State,
) -> ACFGNode {
    // AC#3 additive-only short-circuit. Every shipped schedule has an
    // empty `partition_pairs` (verified by the
    // `shipped_examples_without_blocks2d_leave_maps_empty` test in
    // `tests/sidecar_partition_blocks2d.rs`); the early return is the
    // structural guarantee that no existing Xfer count or shape
    // changes today.
    if partition_pairs.is_empty() {
        return node;
    }

    // Re-run dedupe is INTENTIONALLY OMITTED today. Per the comment
    // block in `tests/halo_strip_synth.rs` (search for IDEMPOTENCE /
    // RE-RUN CAVEAT), `inject_transfers` is not idempotent when
    // partition_pairs is non-empty: `rewrite_partition_tiles` clobbers
    // halo-strip tiles on a re-run, and `splice_pushes_for_waits`
    // splices a new Push for every halo-strip Wait it sees outside the
    // immediate-producer-neighbour dedupe window. Adding a 4-tuple
    // dedupe in this function alone would not restore structural
    // idempotence (the tile would still be wrong, and the new pushes
    // from splice_pushes_for_waits would still land). The full fix is
    // forward-carried to TASK-0290.
    //
    // No production code path re-runs `inject_transfers` on its own
    // output (the driver pipeline calls it once), so this is a
    // first-cut concession rather than a behavioural regression for
    // any shipped schedule.

    // Build (DataId -> Set<KernelId>) consumer index — same shape +
    // walk as `extend_xfer_tiles_for_halo`'s `collect_data_consumers`.
    // We need this to translate `halo_widths[kid][iv]` (keyed by
    // kernel id) into "which data symbols cross the halo boundary for
    // this consumer kernel".
    let mut consumers: BTreeMap<DataId, BTreeSet<KernelId>> = BTreeMap::new();
    collect_data_consumers(&node, &mut consumers);

    // For each (outer_iv, inner_iv) pair, build the list of
    // (data, src_worker, dst_worker, tile) halo-strip transfers to
    // synthesise. Group by outer_iv: each group is placed in the
    // parent Sequence containing the outer Repeat for that outer_iv
    // (TASK-0290 cycle 114b: placement is AFTER the last producing
    // Operation, not at the front — see `prepend_strip_pairs`).
    //
    // `BTreeMap<IterVar, Vec<XferPlaceholder>>` so the insertion order
    // matches outer_iv numeric order — deterministic.
    let mut to_insert: BTreeMap<IterVar, Vec<XferPlaceholder>> = BTreeMap::new();

    for (&outer_iv, &inner_iv) in partition_pairs {
        // Look up the grid shape recorded by partition_blocks2d.
        let Some(&(grid_rows, grid_cols)) = grid_shape_for_outer_iv.get(&outer_iv) else {
            // partition_pairs entry without a matching grid_shape entry
            // would be a partition_blocks2d invariant violation. Skip
            // defensively: the AC#3 short-circuit covers the "no pairs"
            // case; this is the belt-and-braces guard for malformed
            // sidecars on the per-pair path.
            continue;
        };
        let grid_cols_usize = grid_cols as usize;
        let grid_rows_usize = grid_rows as usize;

        // Per-worker y-band (from outer_iv) + x-band (from inner_iv).
        let Some(per_worker_y) = partition_worker_ranges.get(&outer_iv) else {
            continue;
        };
        let Some(per_worker_x) = partition_worker_ranges.get(&inner_iv) else {
            continue;
        };
        // Workers in row-major order — matches partition_blocks2d's
        // assignment (`for (i, wid) in body_workers.iter().enumerate()
        // { row = i / cols; col = i % cols }`). BTreeMap key iteration
        // is numeric WorkerId order, equivalent to the BTreeSet
        // iteration partition_blocks2d used on the write side.
        let body_workers: Vec<WorkerId> = per_worker_y.keys().copied().collect();
        if body_workers.len() != grid_rows_usize * grid_cols_usize {
            // Defensive: the row/col inversion only makes sense when
            // body_workers.len() == grid_rows * grid_cols. A mismatch
            // is a partition_blocks2d invariant violation; skip rather
            // than synthesise wrong pairs.
            continue;
        }

        // For each consumer kernel with a halo entry on outer_iv or
        // inner_iv, find every data symbol that kernel consumes. That
        // is the set of data symbols needing a strip transfer.
        //
        // `data_with_halo_y[data] = h_y` is the y-axis halo for the
        // FIRST consumer kernel matching that data (single-consumer
        // case in every in-tree example; documented as a limit for
        // multi-consumer cases — same caveat the per-tile halo
        // extension carries).
        let mut data_with_halo_y: BTreeMap<DataId, u64> = BTreeMap::new();
        let mut data_with_halo_x: BTreeMap<DataId, u64> = BTreeMap::new();
        for (data, kids) in &consumers {
            for kid in kids {
                if let Some(per_iv) = halo_widths.get(kid) {
                    if let Some(&h) = per_iv.get(&outer_iv) {
                        if h > 0 {
                            // Take MAX across consumer kernels — same
                            // merge rule `extend_xfer_tiles_for_halo`
                            // uses. The map's `entry().and_modify().or_insert`
                            // gives "max across kids" naturally.
                            data_with_halo_y
                                .entry(*data)
                                .and_modify(|cur| *cur = (*cur).max(h))
                                .or_insert(h);
                        }
                    }
                    if let Some(&h) = per_iv.get(&inner_iv) {
                        if h > 0 {
                            data_with_halo_x
                                .entry(*data)
                                .and_modify(|cur| *cur = (*cur).max(h))
                                .or_insert(h);
                        }
                    }
                }
            }
        }

        // Union of data symbols touched on either axis. Iteration in
        // BTreeSet numeric order — deterministic.
        let mut all_halo_data: BTreeSet<DataId> = BTreeSet::new();
        all_halo_data.extend(data_with_halo_y.keys().copied());
        all_halo_data.extend(data_with_halo_x.keys().copied());

        // No data symbols with halo on either partitioned axis ⇒
        // nothing to synthesise for this pair. Skip without inserting
        // an empty group (`to_insert.entry(outer_iv).or_default()`
        // would needlessly noise the placement walk).
        if all_halo_data.is_empty() {
            continue;
        }

        let group = to_insert.entry(outer_iv).or_default();

        // For each worker, compute (row, col), enumerate cardinal
        // neighbours, synthesise one Push+Wait pair per (data,
        // direction).
        //
        // Direction emit order is FIXED as (N, S, W, E) — the exact
        // order is a stability contract for the unit test's pair-
        // counting predicate. SeqTags monotonically increase in that
        // visit order, matching the per-worker BTreeMap iteration.
        for (i, &this_w) in body_workers.iter().enumerate() {
            let row = i / grid_cols_usize;
            let col = i % grid_cols_usize;
            let Some(y_band) = per_worker_y.get(&this_w) else {
                continue;
            };
            let Some(x_band) = per_worker_x.get(&this_w) else {
                continue;
            };
            let (y_lo, y_hi) = (y_band.start, y_band.end);
            let (x_lo, x_hi) = (x_band.start, x_band.end);

            // Helper: build the XferPlaceholder pair for a single
            // (src_worker, strip-tile, data) and push both Push and
            // Wait into `group`. No re-run dedupe today — see the
            // re-run-caveat comment near the top of this function.
            let mut emit_pair =
                |src: WorkerId, dst: WorkerId, data: DataId, tile: IterTile, state: &mut State| {
                    let policy = policies_by_data.get(&data).copied().unwrap_or_default();
                    let seq = state.fresh_seq();
                    group.push(XferPlaceholder {
                        role: XferRole::Push,
                        src,
                        dst,
                        data,
                        tile: tile.clone(),
                        seq,
                        policy,
                    });
                    group.push(XferPlaceholder {
                        role: XferRole::Wait,
                        src,
                        dst,
                        data,
                        tile,
                        seq,
                        policy,
                    });
                };

            // ---- N neighbour: row-1, col. ----
            //
            // Per-axis bands: outer (y) extends DOWN by h_y into the
            // neighbour's bottom row; inner (x) stays at this worker's
            // x-band. `order_halo_strip_bounds_by_data_dim` (TASK-0306)
            // orders the tile by data-dim position — preserving the
            // pre-cycle-133 `(outer, inner)` shape for canonical
            // outer-axis-leading layouts and flipping / dropping for
            // inner-axis-leading or non-prefix layouts.
            if row > 0 {
                let neighbour_idx = (row - 1) * grid_cols_usize + col;
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_y) = data_with_halo_y.get(&data) {
                        let h_y_i: i64 = h_y.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            (y_lo - h_y_i)..y_lo,
                            inner_iv,
                            x_lo..x_hi,
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- S neighbour: row+1, col. ----
            if row + 1 < grid_rows_usize {
                let neighbour_idx = (row + 1) * grid_cols_usize + col;
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_y) = data_with_halo_y.get(&data) {
                        let h_y_i: i64 = h_y.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            y_hi..(y_hi + h_y_i),
                            inner_iv,
                            x_lo..x_hi,
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- W neighbour: row, col-1. ----
            if col > 0 {
                let neighbour_idx = row * grid_cols_usize + (col - 1);
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_x) = data_with_halo_x.get(&data) {
                        let h_x_i: i64 = h_x.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            y_lo..y_hi,
                            inner_iv,
                            (x_lo - h_x_i)..x_lo,
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- E neighbour: row, col+1. ----
            if col + 1 < grid_cols_usize {
                let neighbour_idx = row * grid_cols_usize + (col + 1);
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_x) = data_with_halo_x.get(&data) {
                        let h_x_i: i64 = h_x.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            y_lo..y_hi,
                            inner_iv,
                            x_hi..(x_hi + h_x_i),
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }
        }
    }

    if to_insert.is_empty() {
        return node;
    }

    // Walk the ACFG once and place each group's pairs in the parent
    // Sequence containing the outer Repeat (TASK-0290 cycle 114b:
    // placement is AFTER the LAST producing Operation, not at the
    // front of the Sequence; see `prepend_strip_pairs` for the
    // rationale and the fallback rule). `to_insert` is threaded
    // through the walk as a `&mut` arg shared with the recursive
    // helper; entries are drained (removed) by `prepend_strip_pairs`
    // as it places them. After the walk, any outer_iv still in the
    // map is a sidecar/tree mismatch we tolerate silently (a
    // synthetic test or a future composition where the
    // partition_pairs entry survives a tree restructure that loses
    // the matching Repeat). No panic — the synthesis is simply a
    // no-op for that pair.
    let mut to_insert_mut = to_insert;
    prepend_strip_pairs(node, &mut to_insert_mut)
}

/// Walk `node` and insert each group's `Vec<XferPlaceholder>` into the
/// Sequence that contains the Repeat for the matching `outer_iv`.
///
/// **Placement rule** (TASK-0290 cycle 114b — architect P1 from cycle 114a):
/// for each group keyed by `outer_iv`, the synthesised Xfers are
/// inserted into the parent Sequence AFTER the LAST direct-child
/// `Operation` whose `dataflow.edges[*].data_out` is one of the halo
/// data symbols that group carries (the producer Operations on the
/// host side that load the data the halo strips will exchange).
///
/// **Why**: a worker's emitted EventList orders Push/Wait pairs by
/// source-tree order. If the synthesised `Wait` lands BEFORE the
/// host's `load_image` Operation in the root Sequence, the receiving
/// worker schedules its halo-strip Wait against a producer Push that
/// has not yet fired on the host — a real ordering defect, not just
/// aesthetic. Inserting AFTER the producer Operation orders the new
/// Push/Wait pair behind the producer's host-broadcast pairs (which
/// `splice_pushes_for_waits` placed immediately after the producer
/// via the dedupe-window path), which is the data-flow-correct order.
///
/// **Fallback**: if NO direct-child Operation in the parent Sequence
/// produces a halo data symbol for this group, the group is prepended
/// to the front of the Sequence (the pre-cycle-114b behaviour). This
/// keeps synthetic test fixtures without a top-level producer
/// (`tests/halo_strip_synth.rs` — 4 of 5 cases) green by construction:
/// their parent Sequence contains only the outer Repeat, so the
/// "insert at index 0" position is unchanged.
///
/// `to_insert` is consumed: when an outer_iv's group is placed, its
/// entry is removed. After the walk, any entries left in `to_insert`
/// were unmatched — the synthesis is silently dropped for those (a
/// malformed tree-vs-sidecar disagreement we don't loudly panic on,
/// matching the defensive skip in the synthesis loop above).
fn prepend_strip_pairs(
    node: ACFGNode,
    to_insert: &mut BTreeMap<IterVar, Vec<XferPlaceholder>>,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            // Phase 1: bind each matching outer_iv to its placement
            // position in the children vector. Position = (last
            // producer index for the group's halo data symbols) + 1,
            // or 0 if no producer is found (fallback for synthetic
            // fixtures + structurally-redundant single-pass stencils).
            //
            // We use BTreeMap iteration order on `to_insert` to keep
            // the per-outer_iv visit order deterministic (numeric
            // IterVar order). Multiple groups landing at the SAME
            // position concatenate in BTreeMap order.
            let mut groups_at: BTreeMap<usize, Vec<XferPlaceholder>> = BTreeMap::new();
            let matched_ivs: Vec<IterVar> = children
                .iter()
                .filter_map(|c| {
                    if let ACFGNode::Repeat { iter_var, .. } = c {
                        if to_insert.contains_key(iter_var) {
                            return Some(*iter_var);
                        }
                    }
                    None
                })
                .collect();
            for iv in matched_ivs {
                // Pull the group out (we know it exists).
                let group = to_insert.remove(&iv).expect("checked above");
                // Halo data symbols this group carries — used to
                // identify producing Operations in the parent
                // Sequence.
                let halo_data: BTreeSet<DataId> =
                    group.iter().map(|p| p.data).collect();
                // Find the LAST direct-child Operation that writes any
                // symbol in `halo_data`. We walk all edges (not just
                // edges[0]) — a future multi-edge DAG may carry the
                // halo data on a non-first edge, and we want the
                // placement to follow the data, not the convention of
                // edge[0].
                let mut last_producer_idx: Option<usize> = None;
                for (idx, child) in children.iter().enumerate() {
                    if let ACFGNode::Operation(op) = child {
                        let writes_halo = op
                            .dataflow
                            .edges
                            .iter()
                            .any(|e| {
                                e.data_out
                                    .map(|d| halo_data.contains(&d))
                                    .unwrap_or(false)
                            });
                        if writes_halo {
                            last_producer_idx = Some(idx);
                        }
                    }
                }
                let insert_pos = match last_producer_idx {
                    Some(idx) => idx + 1,
                    None => 0,
                };
                groups_at.entry(insert_pos).or_default().extend(group);
            }
            // Phase 2: recurse into each child so any nested
            // partitioned Repeat can drain its own group from
            // `to_insert`. We DO NOT recurse into the children before
            // computing `last_producer_idx` above, because the
            // recursion might restructure them — but the matched_ivs
            // and producer-index computation reads only the
            // top-level shape (Operation/Repeat at this Sequence
            // level), so the pre-recursion scan is safe.
            //
            // Recursion MUST preserve per-child arity (every
            // ACFGNode::{Sequence,Repeat,Operation,Sync,Xfer} arm in
            // `prepend_strip_pairs` returns exactly one node), so the
            // post-recursion `rewritten_children.len()` equals the
            // pre-recursion `children.len()` — keeping the
            // `last_producer_idx` indices stable across phases. If a
            // future variant of this walker ever inserted at a child
            // level the indices would silently shift; the
            // `debug_assert!` below pins the invariant
            // (TASK-0290 cycle 114b architect P1.2 hardening).
            let n_existing = children.len();
            let rewritten_children: Vec<ACFGNode> = children
                .into_iter()
                .map(|c| prepend_strip_pairs(c, to_insert))
                .collect();
            debug_assert_eq!(
                rewritten_children.len(),
                n_existing,
                "prepend_strip_pairs phase-2 recursion must preserve per-child arity \
                 (every ACFGNode arm returns exactly one node); if this fires, a future \
                 variant has begun inserting at a child level and last_producer_idx is \
                 now stale"
            );
            // Phase 3: assemble the output Sequence, splicing each
            // group's Xfers in at its computed `insert_pos`. We walk
            // children in order; before emitting the child at index
            // `k`, we drain any groups whose insert_pos == k. After
            // the last child, drain any groups whose insert_pos ==
            // n_existing (appended at the tail).
            let n_inserted: usize = groups_at.values().map(|v| v.len()).sum();
            let mut out: Vec<ACFGNode> = Vec::with_capacity(n_inserted + n_existing);
            for (k, child) in rewritten_children.into_iter().enumerate() {
                if let Some(group) = groups_at.remove(&k) {
                    for p in group {
                        out.push(ACFGNode::Xfer(p));
                    }
                }
                out.push(child);
            }
            // Tail: groups whose insert_pos == n_existing (i.e. after
            // every child). In practice the only positions that ever
            // get computed are 0..=n_existing-1 (a producer must
            // exist as a child to anchor the placement), but the
            // tail-drain keeps the code total and easy to reason
            // about. Drained in BTreeMap key order.
            for (_, group) in groups_at {
                for p in group {
                    out.push(ACFGNode::Xfer(p));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            // A bare Repeat (not wrapped in a Sequence) whose iter_var
            // matches a group: wrap it. In practice `build_acfg` always
            // wraps top-level statements in a Sequence, so this branch
            // is defensive; recursing into the body lets deeper
            // matches still drain.
            let new_body = Box::new(prepend_strip_pairs(*body, to_insert));
            ACFGNode::Repeat {
                iter_var,
                range,
                body: new_body,
                block_tag,
            }
        }
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => leaf,
    }
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

#[cfg(test)]
mod tests {
    //! Inline unit tests for `order_halo_strip_bounds_by_data_dim`.
    //!
    //! TASK-0315: pin the helper's branch selection directly. The
    //! integration tests in `tests/halo_strip_synth.rs` (task0306_ac3 /
    //! ac4) exercise the helper via `inject_transfers` end-to-end and
    //! observe the EMIT shape; these direct-call tests prove the
    //! helper's branch ordering as a unit, so a future refactor of
    //! `inject_halo_strip_xfers` that bypasses or short-circuits the
    //! helper would still be caught by these unit pins.
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn iv_set(ivs: &[IterVar]) -> BTreeSet<IterVar> {
        ivs.iter().copied().collect()
    }

    /// AC#2 (positive arm): with a populated `data_dim_iv_map` entry,
    /// the helper returns dim-ordered bounds — proving the fall-back
    /// branch was NOT taken. Indices `[outer_iv][inner_iv]` → canonical
    /// outer-leading emit.
    #[test]
    fn task0315_outer_leading_takes_canonical_path_not_fallback() {
        let outer_iv = IterVar(7);
        let inner_iv = IterVar(8);
        let data = DataId(99);
        let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
        map.insert(data, vec![iv_set(&[outer_iv]), iv_set(&[inner_iv])]);

        let got = order_halo_strip_bounds_by_data_dim(
            data, outer_iv, 8..9, inner_iv, 0..8, &map,
        );

        assert_eq!(
            got,
            vec![(outer_iv, 8..9), (inner_iv, 0..8)],
            "TASK-0315 AC#2: outer-leading data layout MUST emit canonical \
             outer-first order via the data-dim consultation branch, not the \
             default-order fall-back (which would happen to coincide here, \
             but the swapped-layout test below proves the canonical branch \
             actually fires).",
        );
    }

    /// AC#2 (cross-check): with a SWAPPED dim layout (inner_iv at dim 0,
    /// outer_iv at dim 1) the helper MUST flip emit order. Default-order
    /// fall-back would return `[(outer_iv, ...), (inner_iv, ...)]` — a
    /// different vector — so this directly distinguishes the canonical
    /// path from the fall-back.
    #[test]
    fn task0315_inner_leading_flips_order_proving_non_fallback() {
        let outer_iv = IterVar(7);
        let inner_iv = IterVar(8);
        let data = DataId(99);
        let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
        // dim 0 = inner_iv, dim 1 = outer_iv (inner-axis-leading layout).
        map.insert(data, vec![iv_set(&[inner_iv]), iv_set(&[outer_iv])]);

        let got = order_halo_strip_bounds_by_data_dim(
            data, outer_iv, 8..9, inner_iv, 0..8, &map,
        );

        assert_eq!(
            got,
            vec![(inner_iv, 0..8), (outer_iv, 8..9)],
            "TASK-0315 AC#2: inner-axis-leading layout MUST flip to \
             inner-first emit. The fall-back branch would have returned \
             [(outer_iv, ...), (inner_iv, ...)] — its return value \
             differs from this expected output, so a passing assertion \
             here is direct evidence the canonical (non-fall-back) \
             branch fired.",
        );
    }

    /// Fall-back branch: `Some(empty)` per-dim Vec ⇒ default order. This
    /// is the path the cycle-133 NUC_TRACE diagnostic now reports. The
    /// behaviour is unchanged from cycle 133 — pinned here so a future
    /// edit to the fall-back cannot silently change its return shape.
    #[test]
    fn task0315_some_empty_falls_back_to_default_order() {
        let outer_iv = IterVar(7);
        let inner_iv = IterVar(8);
        let data = DataId(99);
        let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
        map.insert(data, Vec::new());

        let got = order_halo_strip_bounds_by_data_dim(
            data, outer_iv, 8..9, inner_iv, 0..8, &map,
        );

        assert_eq!(
            got,
            vec![(outer_iv, 8..9), (inner_iv, 0..8)],
            "TASK-0315: Some(empty) takes fall-back; default order is \
             outer-leading.",
        );
    }

    /// Fall-back branch: data missing from the map ⇒ default order (the
    /// `None` arm). Production callers never reach this path; pinned for
    /// the synthetic-fixture safety net.
    #[test]
    fn task0315_missing_data_falls_back_to_default_order() {
        let outer_iv = IterVar(7);
        let inner_iv = IterVar(8);
        let data = DataId(99);
        let map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();

        let got = order_halo_strip_bounds_by_data_dim(
            data, outer_iv, 8..9, inner_iv, 0..8, &map,
        );

        assert_eq!(
            got,
            vec![(outer_iv, 8..9), (inner_iv, 0..8)],
            "TASK-0315: missing entry takes fall-back; default order is \
             outer-leading.",
        );
    }

    // ----------------------------------------------------------------
    // TASK-0317: silent-sibling pins for
    // `compute_partition_bounds_with_dim_prefix`.
    //
    // Same shape as the TASK-0315 inline tests above. The helper is
    // the canonical-path arm of `rewrite_partition_tiles_inner`; the
    // `None` arm now triggers a NUC_TRACE diagnostic in the caller
    // (the fall-back observability addition this task lands).
    //
    // We pin: (a) the data-dim-aware canonical Some-arm, (b) the
    // missing-entry None arm that drives the caller's nest-order
    // fall-back + trace emit, (c) the empty-per-dim None arm
    // (parallel to TASK-0315's Some(empty) twin), and (d) the
    // sparse-coverage whole-array drop.
    // ----------------------------------------------------------------

    #[allow(clippy::type_complexity)]
    fn make_partition_ranges(
        entries: &[(IterVar, &[(WorkerId, std::ops::Range<i64>)])],
    ) -> BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>> {
        let mut out: BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>> = BTreeMap::new();
        for (iv, per_worker) in entries {
            let map: BTreeMap<_, _> =
                per_worker.iter().map(|(w, r)| (*w, r.clone())).collect();
            out.insert(*iv, map);
        }
        out
    }

    /// AC: canonical Some-arm — `data_dim_iv_map` indexed [outer][inner]
    /// with both partitioned ⇒ returns dim-ordered bounds. Caller does
    /// NOT take the fall-back; no trace fires.
    #[test]
    fn task0317_canonical_path_returns_dim_ordered_bounds_no_fallback() {
        let outer_iv = IterVar(7);
        let inner_iv = IterVar(8);
        let data = DataId(99);
        let worker = WorkerId(2);
        let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
        map.insert(data, vec![iv_set(&[outer_iv]), iv_set(&[inner_iv])]);

        let partition_ranges = make_partition_ranges(&[
            (outer_iv, &[(worker, 8..16)]),
            (inner_iv, &[(worker, 0..8)]),
        ]);

        let got = compute_partition_bounds_with_dim_prefix(
            data, &map, &partition_ranges, worker,
        );

        assert_eq!(
            got,
            Some(vec![(outer_iv, 8..16), (inner_iv, 0..8)]),
            "TASK-0317 canonical: both dims covered by partitioned ivs in \
             nest-prefix order ⇒ dim-ordered bounds, no fall-back.",
        );
    }

    /// AC: None arm — `data_dim_iv_map` missing entry for the data
    /// symbol ⇒ caller takes nest-order fall-back + emits NUC_TRACE.
    /// This is the arm the trace is observability-instrumenting.
    #[test]
    fn task0317_missing_entry_returns_none_drives_fallback() {
        let outer_iv = IterVar(7);
        let data = DataId(99);
        let worker = WorkerId(2);
        let map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
        let partition_ranges =
            make_partition_ranges(&[(outer_iv, &[(worker, 8..16)])]);

        let got = compute_partition_bounds_with_dim_prefix(
            data, &map, &partition_ranges, worker,
        );

        assert_eq!(
            got, None,
            "TASK-0317 None-arm: missing data_dim_iv_map entry returns \
             None; rewrite_partition_tiles_inner's fall-back arm fires \
             on this case and emits the NUC_TRACE diagnostic.",
        );
    }

    /// AC: None arm — `data_dim_iv_map` records data with empty per-dim
    /// Vec (synthetic fixtures via DataflowEdge::new) ⇒ returns None
    /// via the `per_dim.is_empty()` early-out at line 1938. Caller
    /// takes the same fall-back as the missing-entry case.
    #[test]
    fn task0317_empty_per_dim_returns_none_drives_fallback() {
        let outer_iv = IterVar(7);
        let data = DataId(99);
        let worker = WorkerId(2);
        let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
        map.insert(data, Vec::new());
        let partition_ranges =
            make_partition_ranges(&[(outer_iv, &[(worker, 8..16)])]);

        let got = compute_partition_bounds_with_dim_prefix(
            data, &map, &partition_ranges, worker,
        );

        assert_eq!(
            got, None,
            "TASK-0317 None-arm (twin): empty per-dim Vec returns None \
             via the explicit is_empty() early-out. Caller fall-back \
             trace fires on this arm too.",
        );
    }

    /// AC: sparse-coverage whole-array drop — partitioned iv covers
    /// dim 1 but not dim 0 (a hole at dim 0 followed by a covered
    /// dim 1 violates the contiguous-prefix invariant). Returns
    /// `Some(Vec::new())` per the safe-drop policy at line 1980.
    #[test]
    fn task0317_sparse_coverage_drops_to_whole_array() {
        let outer_iv = IterVar(7);
        let inner_iv = IterVar(8);
        let unpart_iv = IterVar(42);
        let data = DataId(99);
        let worker = WorkerId(2);
        let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
        // dim 0 = unpartitioned (k), dim 1 = inner_iv (x, partitioned).
        // outer_iv is partitioned but doesn't index this data.
        map.insert(data, vec![iv_set(&[unpart_iv]), iv_set(&[inner_iv])]);
        let partition_ranges = make_partition_ranges(&[
            (outer_iv, &[(worker, 8..16)]),
            (inner_iv, &[(worker, 0..8)]),
        ]);

        let got = compute_partition_bounds_with_dim_prefix(
            data, &map, &partition_ranges, worker,
        );

        assert_eq!(
            got, Some(Vec::new()),
            "TASK-0317 sparse: dim 0 has no partitioned iv covering it \
             (k is unpartitioned), dim 1 does. Sparse coverage triggers \
             the safe whole-array drop per compute_partition_bounds_with_\
             dim_prefix's hole-after-cover policy.",
        );
    }
}
