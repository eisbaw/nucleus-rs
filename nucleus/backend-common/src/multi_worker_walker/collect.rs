//! Event-walk helpers (recurse into Event::Loop bodies). Lifted from
//! per-backend duplication into single sources of truth (TASK-0239 /
//! TASK-0300). Consumed by every tier-1 backend's `multi_worker::Plan::build`
//! (and by backend-common's unit tests).

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::algo::{collect_dataref_names, AlgoIR, IrStmt};
use nucleus_compiler::event::{ArgBinding, DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::NameTables;

use super::ctx::RendezvousId;
use crate::render::EmitError;

/// Collect every `(DataId, SeqTag)` pair appearing on a Push or Wait
/// event in `events` (descending into Loop bodies). The map's value is
/// the pair's tile, copied from the first event sighting; the same
/// `seq` is carried on both endpoints by the XferPlaceholder
/// construction (TASK-0018) so first-sighting is well-defined.
pub fn collect_xfer_pairs(events: &[Event], out: &mut BTreeMap<(DataId, SeqTag), IterTile>) {
    for e in events {
        match e {
            Event::Push {
                data, seq, tile, ..
            }
            | Event::Wait {
                data, seq, tile, ..
            } => {
                out.entry((*data, *seq)).or_insert_with(|| tile.clone());
            }
            Event::Loop { body, .. } => collect_xfer_pairs(body, out),
            _ => {}
        }
    }
}

/// Build a `(DataId, SeqTag) -> IterTile` map by folding
/// [`collect_xfer_pairs`] across every worker's projected events.
///
/// Single source of truth for the construction shape that all four
/// tier-1 backends (pthreads-sync, pthreads-async, mp-tcp-bufsync,
/// mp-tcp-event) had been duplicating inline (TASK-0300, cycle 130
/// hardening from TASK-0296 cycle-116 architect P1.2).
///
/// # Contract
///
/// First-sighting on a given `(DataId, SeqTag)` wins; later sightings
/// are dropped. Under valid input, both endpoints carry the same
/// `IterTile` by the XferPlaceholder construction (TASK-0018), so the
/// dropped sightings agree with the kept one and the choice is
/// observationally a no-op. "First" means *first in the input
/// iterator's order* — the helper has no opinion about what that order
/// is; it inherits it from the caller.
///
/// # Current-caller convention (informational, not part of the contract)
///
/// All four tier-1 backends pass `per_worker.values()` where
/// `per_worker: BTreeMap<WorkerId, Vec<Event>>`. `BTreeMap::values()`
/// iterates in key-ascending order, so for those callers "first
/// sighting" = the lowest-`WorkerId` worker whose event list names
/// that `(DataId, SeqTag)`. This is what the cycle-130 pin test
/// `first_sighting_wins_on_conflicting_tiles` relies on. A different
/// caller (e.g. `Vec<&[Event]>::iter().copied()` from the cycle-131
/// `vec_of_slices_input_compiles_and_collects` test) sees
/// insertion-order, not WorkerId-ascending — both are valid uses; the
/// helper does not assume the BTreeMap shape.
///
/// The output is keyed only on `(DataId, SeqTag)`, so input iteration
/// order cannot leak into the output's KEY ordering — only into which
/// tile wins on a conflict.
pub fn collect_pair_tiles<'a, I, T>(events_per_worker: I) -> BTreeMap<(DataId, SeqTag), IterTile>
where
    I: IntoIterator<Item = &'a T>,
    T: AsRef<[Event]> + 'a + ?Sized,
{
    let mut out: BTreeMap<(DataId, SeqTag), IterTile> = BTreeMap::new();
    for evs in events_per_worker {
        collect_xfer_pairs(evs.as_ref(), &mut out);
    }
    out
}

/// Per-worker visit of Push/Wait events to collect the worker's
/// rendezvous-id touch set. Descends into `Event::Loop` bodies.
///
/// Replaces the per-backend `collect_worker_slots` /
/// `collect_worker_rings` — both walked identically, only the value
/// type alias differed (`SlotId = RingId = usize`).
pub fn collect_worker_rendezvous(
    events: &[Event],
    ids: &BTreeMap<(DataId, SeqTag), RendezvousId>,
    out: &mut BTreeSet<RendezvousId>,
) {
    for e in events {
        match e {
            Event::Push { data, seq, .. } | Event::Wait { data, seq, .. } => {
                if let Some(s) = ids.get(&(*data, *seq)) {
                    out.insert(*s);
                }
            }
            Event::Loop { body, .. } => collect_worker_rendezvous(body, ids, out),
            _ => {}
        }
    }
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index,
/// no fallibility (every tag is an independent barrier, so there is
/// nothing to validate / reject here any more).
pub fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
where
    F: FnMut(SyncTag, &BTreeSet<WorkerId>),
{
    for e in events {
        match e {
            Event::Sync {
                participants, sync, ..
            } => f(*sync, participants),
            Event::Loop { body, .. } => collect_barriers_by_tag(body, f),
            _ => {}
        }
    }
}

/// Detect the **overlapping-write accumulator fan-in** pattern at the
/// codegen layer (TASK-0343, cycle 189).
///
/// Returns the set of `(DataId, SeqTag)` pairs for which the
/// receiver-side `Event::Wait` MUST emit element-wise accumulate
/// (sum identity) instead of the default whole-array overwrite assign.
///
/// # The pattern this detects
///
/// 08-histogram/distributed: 4 compute workers each compute a local
/// partial histogram over their input partition and Push the FULL
/// 16-element output array to the host. The host then has 4 Waits on
/// the same data symbol, all carrying whole-array tiles. Pre-cycle-189
/// `render_wait_assign` emitted `histogram = slot_N.wait();` for each
/// (last-write-wins); the cycle-189 fix replaces each with an
/// element-wise `wrapping_add` accumulate into the host's
/// zero-initialised destination.
///
/// Contrast 03-reduction's `partials[w]` shape (the DISJOINT-write
/// accumulator, NOT this pattern): each worker pushes ONE slot of
/// `partials`, the Wait tiles carry per-worker slice ranges (`(w,
/// w..w+1)`), `wait_slice` returns `Some(WaitSlice::Flat)`, and the
/// existing slice-paste arm emits the bit-correct gather. This
/// helper does NOT classify those Waits as accumulate (the predicate
/// requires whole-array tiles).
///
/// # The predicate
///
/// For each `DataId` that has N>=2 Waits in `events` where every Wait's
/// tile (from `pair_tiles`) is whole-array — i.e. either has no bounds
/// at all, OR the data is scalar (zero dims), OR every bound's range
/// covers the corresponding data dim's full source range — all of the
/// `(DataId, SeqTag)` pairs for those Waits enter the output set.
///
/// "Whole-array" here matches the same condition `wait_slice` (sibling
/// `wait.rs`) uses to return `None` (the whole-array assign arm), so
/// pre-cycle-189 emit identity holds for any Wait this helper does NOT
/// classify as accumulate: every Wait that would have gone through the
/// `name = rhs;` arm AND is not part of an N>=2 fan-in group still
/// emits exactly that pre-cycle-189 string.
///
/// # Scope LIMITS (filed as TASK-0343 follow-ups)
///
/// - **Sum identity only**: the consumer (`render_wait_assign`) emits
///   `wrapping_add` for integer scalar types and returns
///   `EmitError::ContractGap` for floats / bool (sum identity is not
///   defined; floats also collide with PRD §10.1 bit-identity).
/// - **No algorithm-level cross-check**: detection is purely structural
///   (N>=2 whole-array Waits). An exotic schedule that emits multiple
///   whole-array Pushes for non-accumulator semantics would mis-combine.
///   For every shipped schedule today (08-histogram/distributed is the
///   load-bearing case), the structural pattern is equivalent to the
///   algorithm-level accumulator pattern (LHS appears in RHS).
/// - **No iterative (Repeat-body) accumulator**: Waits inside a
///   `Event::Loop` body carry per-iter tiles (the iter-var range is one
///   of the consulted dims), so they do NOT match the whole-array
///   predicate and the helper does not classify them. Multi-pass
///   time-step accumulator patterns are a separate follow-up.
///
/// Descends into `Event::Loop` bodies on the recursion side ONLY to
/// gather Waits — the whole-array-tile predicate naturally excludes
/// in-loop Waits (their tiles carry the enclosing iter-var range).
pub fn collect_accumulate_waits(
    events: &[Event],
    sidecar: &NameSidecar,
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
) -> BTreeSet<(DataId, SeqTag)> {
    // Group Waits by data: data -> Vec<seq>.
    let mut waits_per_data: BTreeMap<DataId, Vec<SeqTag>> = BTreeMap::new();
    walk_waits(events, &mut waits_per_data);

    let mut out: BTreeSet<(DataId, SeqTag)> = BTreeSet::new();
    for (data, seqs) in waits_per_data {
        if seqs.len() < 2 {
            continue;
        }
        let all_whole = seqs.iter().all(|seq| {
            pair_tiles
                .get(&(data, *seq))
                // Unified classifier (TASK-0355 cycle 225): route through
                // `is_whole_array_recv` so both this accumulator-detection
                // call site AND the let-at-wait classifier at
                // `collect_let_at_wait_inner` (collect.rs:392) consult the
                // same guard chain in `wait_slice` (rank, OOB, sidecar
                // lookup). `Err` arms swallowed to `false` — matches the
                // sibling site's `.unwrap_or(false)` convention and the
                // pre-cycle-225 `is_whole_array_tile` silent-false on
                // axis-beyond-dims (now an explicit `Err` from wait_slice
                // that is treated as not-whole-array here).
                //
                // Conservative-default rationale (cycle-189 architect P3.2,
                // preserved across the cycle-225 unification): if a Wait's
                // (data, seq) is missing from `pair_tiles`, we have NO
                // evidence the tile is whole-array; do NOT classify as
                // accumulate. The branch is structurally unreachable for
                // shipped schedules — `collect_pair_tiles` (sibling, this
                // module) is contracted to record every (Push|Wait) pair
                // (TASK-0018 XferPlaceholder construction). If a future
                // projection-layer regression breaks that contract,
                // falling back to pre-cycle-189 `name = rhs;` overwrite
                // emit is a forward-compatible loss (the user still sees
                // wrong output, but the cycle-189 fix does not silently
                // change behaviour here on a different missing-tile
                // surface).
                .map(|tile| super::wait::is_whole_array_recv(sidecar, data, tile).unwrap_or(false))
                .unwrap_or(false)
        });
        if all_whole {
            for seq in seqs {
                out.insert((data, seq));
            }
        }
    }
    out
}

/// Per-data Wait-seq accumulator. Descends into Loop bodies (in-loop
/// Waits are still gathered; the whole-array predicate naturally
/// filters them out at classification time).
fn walk_waits(events: &[Event], out: &mut BTreeMap<DataId, Vec<SeqTag>>) {
    for e in events {
        match e {
            Event::Wait { data, seq, .. } => {
                out.entry(*data).or_default().push(*seq);
            }
            Event::Loop { body, .. } => walk_waits(body, out),
            _ => {}
        }
    }
}

/// Algorithm-level cross-check for the structural overlapping-write
/// accumulator detector (TASK-0343.03; hardens the cycle-189 structural
/// landing TASK-0343).
///
/// [`collect_accumulate_waits`] detects the accumulator-fan-in pattern
/// PURELY STRUCTURALLY: per worker, `N>=2` whole-array `Wait`s on one
/// data symbol ⇒ classify all as element-wise (`wrapping_add`) combine.
/// For every shipped schedule today (08-histogram/distributed is the
/// load-bearing case) that structural pattern is EQUIVALENT to the
/// algorithm-level accumulator shape: the data's `Dataflow` LHS name
/// also appears among its RHS data references, e.g.
/// `histogram[b] <-- bin_inc(histogram[b], input[i], b)`.
///
/// But the structural detector has no algorithm-level evidence: an
/// exotic schedule that emits multiple whole-array Pushes for a
/// NON-accumulator data symbol would be silently mis-combined as an
/// element-wise sum — a silent miscompile. This cross-check closes that
/// gap. It consults the algorithm-IR and FAILS LOUD
/// ([`EmitError::AccumulatorShapeMismatch`]) when the structural
/// accumulate pattern fires on a data symbol whose algorithm-level shape
/// is NOT an accumulator.
///
/// # What "algorithm-level accumulator" means here
///
/// A data symbol is an accumulator iff it is the LHS of some
/// `IrStmt::Dataflow { lhs, rhs }` where `lhs.name` also appears among
/// the RHS's referenced data symbols (the LHS-appears-in-RHS shape). The
/// RHS data references are collected with the canonical
/// [`nucleus_compiler::algo::collect_dataref_names`] walker (the SAME
/// walker `link::pipeline` uses), so there is no second, drifting copy
/// of the IrExpr walk. `IrStmt::For` bodies are descended; `Effect`
/// statements never write a data symbol so are irrelevant to the
/// LHS-in-RHS test.
///
/// # Reuse of the structural detector (NO duplicated logic)
///
/// For each worker this calls the EXISTING [`collect_pair_tiles`] +
/// [`collect_accumulate_waits`] verbatim — the very functions the
/// backends consume — so the check and the codegen agree by
/// construction. It does NOT reimplement the whole-array predicate.
///
/// # Scope LIMIT (filed: TASK-0343.03.02)
///
/// The accumulator test is exactly LHS-appears-in-RHS. It does NOT yet
/// recognise a transitive accumulator (e.g. `acc <-- f(tmp)` where
/// `tmp <-- g(acc)` earlier in the same scope). No shipped schedule
/// needs this; a transitive-accumulator generalisation is filed as
/// TASK-0343.03.02. Because the conservative direction of this check is
/// to REJECT (it only adds a reject path), a transitive accumulator that
/// the LHS-in-RHS test misses would be a FALSE REJECT, not a silent
/// miscompile — so the failure mode stays fail-loud, never silent.
///
/// # Inputs
///
/// - `algo_ir`: the lowered algorithm (the driver's `linked.algo`).
/// - `per_worker`: the projected per-worker event lists (after
///   check-frame / host-mediation injection — the SAME map the backend
///   `emit()` receives).
/// - `sidecar`: needed by [`collect_accumulate_waits`] →
///   `is_whole_array_recv` to read data dims.
/// - `data_names`: `DataId -> name` (the driver's `NameTables::data`),
///   the bridge between the codegen `DataId` space and the algorithm-IR
///   `String`-name space. A `DataId` missing here is itself a contract
///   regression and is reported as [`EmitError::ContractGap`] (NEVER
///   silently skipped — CLAUDE.md no-workarounds).
pub fn check_accumulator_consistency(
    algo_ir: &AlgoIR,
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    sidecar: &NameSidecar,
    data_names: &BTreeMap<DataId, String>,
) -> Result<(), EmitError> {
    // 1. Algorithm-level accumulator name set: LHS-appears-in-RHS.
    let mut accumulator_names: BTreeSet<String> = BTreeSet::new();
    collect_accumulator_names(&algo_ir.stmts, &mut accumulator_names);

    // 2. Per worker, reuse the EXACT structural detector the backends
    //    use, then cross-check each detected DataId against the
    //    algorithm-level accumulator name set.
    for events in per_worker.values() {
        let pair_tiles = collect_pair_tiles([events.as_slice()]);
        let detected = collect_accumulate_waits(events, sidecar, &pair_tiles);
        for (data, _seq) in &detected {
            let name = data_names.get(data).ok_or_else(|| {
                EmitError::ContractGap(format!(
                    "accumulator cross-check: DataId {data:?} structurally classified as an \
                     overlapping-write accumulator has no entry in the DataId->name table \
                     (NameTables::data); cannot cross-check its algorithm-level shape"
                ))
            })?;
            if !accumulator_names.contains(name) {
                return Err(EmitError::AccumulatorShapeMismatch(format!(
                    "data symbol `{name}` is structurally classified as an overlapping-write \
                     accumulator (>=2 whole-array Waits on it ⇒ element-wise sum combine at the \
                     host), but the algorithm-IR shows it is NOT an accumulator: no `<--` \
                     statement writing `{name}` reads `{name}` on its own RHS \
                     (LHS-appears-in-RHS). Element-wise summing here would be a silent \
                     miscompile. This is a defensive tightening (TASK-0343.03) over the \
                     structural-only cycle-189 detector; for every shipped schedule the two \
                     shapes coincide, so this rejection means an exotic schedule has emitted \
                     multiple whole-array pushes for non-accumulator semantics"
                )));
            }
        }
    }
    Ok(())
}

/// Walk `stmts` (descending into `IrStmt::For` bodies) and insert into
/// `out` every data symbol that is an algorithm-level accumulator: the
/// LHS of an `IrStmt::Dataflow` whose own name also appears among the
/// RHS data references (LHS-appears-in-RHS). Reuses the canonical
/// [`collect_dataref_names`] walker for the RHS read set.
fn collect_accumulator_names(stmts: &[IrStmt], out: &mut BTreeSet<String>) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                let mut rhs_refs: BTreeSet<String> = BTreeSet::new();
                collect_dataref_names(rhs, &mut rhs_refs);
                if rhs_refs.contains(&lhs.name) {
                    out.insert(lhs.name.clone());
                }
            }
            IrStmt::For { body, .. } => collect_accumulator_names(body, out),
            // `Effect` statements never write a data symbol (no LHS), so
            // they cannot introduce an accumulator.
            IrStmt::Effect { .. } => {}
        }
    }
}

/// Visit every `Event::Wait` / `Event::Fire` output to build the
/// three sets needed for the pre-init computation:
///
/// - `waited`: cross-worker inputs the worker WAITs on (these will
///   be overwritten by the .wait() and need to exist as locals).
/// - `whole`: data the worker writes via a whole-array Fire output
///   (let-bound at the Fire site; no pre-init needed).
/// - `indexed`: data the worker writes via an indexed Fire output
///   (must be pre-initialised so the indexed assign has something to
///   write into).
///
/// A worker's pre-init set is `waited UNION (indexed - whole)`.
pub fn collect_pre_init_sets(
    events: &[Event],
    waited: &mut BTreeSet<DataId>,
    whole: &mut BTreeSet<DataId>,
    indexed: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, .. } => {
                waited.insert(*data);
            }
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    if o.indices.is_empty() {
                        whole.insert(o.data);
                    } else {
                        indexed.insert(o.data);
                    }
                }
            }
            Event::Loop { body, .. } => collect_pre_init_sets(body, waited, whole, indexed),
            _ => {}
        }
    }
}

/// TASK-0349 cycle 220: classify which DataIds have a provably-dead
/// pre-init `let mut <name>: Vec<..> = vec![0; N];` because every
/// `Event::Wait` on them is a whole-array recv (and they are not in
/// the accumulate-fan-in set nor the indexed-Fire-write set, both of
/// which need the zero-init to be live).
///
/// For DataIds in the returned set, the per-backend pre-init pass
/// omits the `let mut` line and the walker's `render_wait_assign`
/// emits `let <name> = <rhs>;` at the recv site (declare-and-assign in
/// one statement). The result is a Vec<T> coming into scope at the
/// first .wait() call, no dead zero-init, no `unused_assignments`
/// warning on cargo build of the emitted project.
///
/// Conservative on shape-errors: if `wait_slice` returns Err for any
/// of the data's Waits, the data is kept OUT of the returned set (the
/// pre-init stays — emit-time render_wait_assign will surface the
/// same error to the caller).
///
/// Inputs:
/// - `events`: the worker's projected event list (descends into Loop
///   bodies).
/// - `pair_tiles`: `(DataId, SeqTag) -> IterTile` map; missing entry
///   means no tile (whole-array transfer).
/// - `sidecar`: needed by `is_whole_array_recv` to read data dims.
/// - `accumulate_data`: DataIds classified as accumulate-fan-in
///   anywhere in this worker (passed as a flat-by-data slice of the
///   per-(worker, data, seq) accumulate set; the caller filters on
///   `WorkerId` upstream).
/// - `indexed`: DataIds the worker writes via indexed Fire output
///   (computed by `collect_pre_init_sets`).
pub fn collect_let_at_wait_data(
    events: &[Event],
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
    sidecar: &NameSidecar,
    accumulate_data: &BTreeSet<DataId>,
    indexed: &BTreeSet<DataId>,
) -> BTreeSet<DataId> {
    let mut waited: BTreeSet<DataId> = BTreeSet::new();
    let mut not_all_whole: BTreeSet<DataId> = BTreeSet::new();
    collect_let_at_wait_inner(events, pair_tiles, sidecar, &mut waited, &mut not_all_whole);
    let mut out: BTreeSet<DataId> = BTreeSet::new();
    for d in &waited {
        if not_all_whole.contains(d) {
            continue;
        }
        if accumulate_data.contains(d) {
            continue;
        }
        if indexed.contains(d) {
            continue;
        }
        out.insert(*d);
    }
    out
}

fn collect_let_at_wait_inner(
    events: &[Event],
    pair_tiles: &BTreeMap<(DataId, SeqTag), IterTile>,
    sidecar: &NameSidecar,
    waited: &mut BTreeSet<DataId>,
    not_all_whole: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, seq, .. } => {
                waited.insert(*data);
                let is_whole = match pair_tiles.get(&(*data, *seq)) {
                    None => true,
                    Some(tile) => {
                        super::wait::is_whole_array_recv(sidecar, *data, tile).unwrap_or(false)
                    }
                };
                if !is_whole {
                    not_all_whole.insert(*data);
                }
            }
            Event::Loop { body, .. } => {
                // Descend into the loop body: a whole-array Wait
                // buried inside a loop body IS classified let-at-wait.
                // This classification is UNCHANGED by TASK-0364.
                //
                // SCOPE HAZARD (TASK-0356 cycle 222, characterized;
                // guard LANDED as TASK-0364, OPTION B): classifying an
                // in-loop Wait as let-at-wait means the sibling `wait::
                // render_wait_assign` emits its `let {name} = {rhs};`
                // INSIDE the emitted `for { }` block. A consumer of
                // that data at the ENCLOSING outer scope would read it
                // out of scope. NOT producible today —
                // `transfer_inject` co-locates each Wait with its
                // consumer (see the TASK-0356 note at the `wait.rs`
                // emit site and `tests/wait_let_at_wait_loop_scope.rs`).
                // Rather than make the classifier scope-aware (option
                // A, NOT taken), the sibling
                // `check_let_at_wait_scope_safety` (below) fails LOUD
                // with `EmitError::ContractGap` at render entry if this
                // at-risk shape ever arises — so the classifier keeps
                // descending unconditionally here and the guard is the
                // single chokepoint that rejects the bad scope.
                collect_let_at_wait_inner(body, pair_tiles, sidecar, waited, not_all_whole);
            }
            _ => {}
        }
    }
}

/// TASK-0364 cycle 222 follow-up: fail-loud scope-safety guard for the
/// let-at-wait EMIT hazard characterized by TASK-0356 (OUTCOME-MATRIX
/// branch d).
///
/// # The hazard this guards
///
/// For a DataId in `let_at_wait`, the per-backend pre-init pass OMITS
/// the outer `let mut <name>: Vec<..> = vec![0; N];`, so the sibling
/// [`super::wait::render_wait_assign`] emits a declare-and-assign `let
/// <name> = <rhs>;` AT THE WAIT'S lexical scope. When that Wait sits
/// inside an `Event::Loop` body, the `let` lands inside the emitted
/// `for { }` block. If a CONSUMER of the same data (a `Fire` whose
/// kernel-arg reads it, or a `Push` of it) sits at the ENCLOSING outer
/// scope, the consumer references `<name>` after the loop closes — out
/// of scope — and the generated Rust does not compile (rustc E0425,
/// confirmed empirically cycle 222).
///
/// This shape is NOT producible today: `transfer_inject`
/// (`inject_in_sequence`) co-locates every cross-worker `Wait` in the
/// SAME sequence (same scope) as its consuming Operation. So this is a
/// DEFENSIVE guard for a FUTURE pass (e.g. a hoist that lifts a
/// consumer out of a loop while leaving its Wait behind) that would
/// otherwise ship a latent miscompile. OPTION B (typed `EmitError`,
/// fail loud) was chosen over OPTION A (a scope-aware classifier
/// exclusion): the shape is non-producible, so failing loud with a
/// `ContractGap` carries near-zero regression risk and matches the
/// project's panic-not-diagnostic response to contract gaps, whereas a
/// silent classifier transform would change a code path no shipped
/// schedule exercises.
///
/// # Detection rule
///
/// Walk `events` maintaining a lexical scope-path: a stack of
/// per-occurrence loop identities. A FRESH pre-order occurrence index
/// is pushed when entering an `Event::Loop` body and popped on exit
/// (the occurrence index — NOT the `iter_var` — gives an unambiguous
/// identity even when `iter_var`s repeat across sibling/nested loops).
/// The root scope is the empty path `[]`.
///
/// For each `D` in `let_at_wait`, collect the scope-path of every
/// `Wait` of `D` and the scope-path of every CONSUMER of `D`. A
/// consumer is an `Event::Fire` whose `bindings.inputs` reads `D`
/// (`ArgBinding::Data(DataSlice { data: D, .. })`, recursing into
/// `ArgBinding::Nested` args), OR an `Event::Push { data: D, .. }`. A
/// `Fire` OUTPUT write is NOT a consumer (it produces, not reads).
///
/// `D` is **scope-unsafe** iff there exists a consumer of `D` at
/// scope-path `c` such that NO `Wait` of `D` has a scope-path that is a
/// (non-strict) prefix of `c`. Prefix-domination means the Wait's `let`
/// lexically dominates the consumer; the empty root path is a prefix of
/// every path. This fires on the at-risk shape (in-loop Wait path
/// `[L0]`, consumer path `[]` root → not dominated → unsafe) and does
/// NOT fire on the shipped-safe shape (in-loop Wait + in-loop consumer,
/// both `[L0]` → dominated), nor on root-Wait+root-consumer, nor on
/// root-Wait+nested-consumer.
///
/// # Conservatism / what it deliberately does NOT do
///
/// The rule is structural (lexical scope only): it does not reason
/// about whether a consumer is actually reachable, nor about value
/// liveness. It treats a `Fire` output as a non-consumer even though a
/// later in-place read-modify-write would also need scope; that is out
/// of scope here because the let-at-wait classifier already excludes
/// indexed-Fire-written and accumulate-fan-in data (the only
/// read-modify shapes), so a `let_at_wait` datum is never both
/// classified and Fire-output-written.
///
/// # No-op on the empty set
///
/// Returns `Ok(())` immediately when `let_at_wait` is empty, so
/// mp-tcp-bufsync (which always passes
/// [`super::WalkerCtx::empty_let_at_wait_set`] and also bypasses the
/// walker entirely) is unaffected even if it were ever routed through
/// this guard.
pub fn check_let_at_wait_scope_safety(
    events: &[Event],
    let_at_wait: &BTreeSet<DataId>,
    names: &NameTables,
) -> Result<(), EmitError> {
    if let_at_wait.is_empty() {
        return Ok(());
    }

    // Per-data: scope-paths of Waits, and scope-paths of consumers.
    let mut wait_paths: BTreeMap<DataId, Vec<Vec<usize>>> = BTreeMap::new();
    let mut consumer_paths: BTreeMap<DataId, Vec<Vec<usize>>> = BTreeMap::new();

    // `next_occurrence` is a single monotonically-increasing counter so
    // every Loop body entered in pre-order gets a distinct identity,
    // even sibling loops that reuse the same iter_var.
    let mut next_occurrence: usize = 0;
    let mut path: Vec<usize> = Vec::new();
    collect_scope_paths(
        events,
        let_at_wait,
        &mut path,
        &mut next_occurrence,
        &mut wait_paths,
        &mut consumer_paths,
    );

    for d in let_at_wait {
        let waits = match wait_paths.get(d) {
            Some(w) => w,
            // A let_at_wait datum with no Wait sighted here cannot have
            // a scope hazard from THIS event list (it is classified
            // let-at-wait on some other worker / event list). Nothing
            // to check.
            None => continue,
        };
        let Some(consumers) = consumer_paths.get(d) else {
            continue;
        };
        for c in consumers {
            let dominated = waits.iter().any(|w| is_prefix(w, c));
            if !dominated {
                let name = names
                    .data
                    .get(d)
                    .cloned()
                    .unwrap_or_else(|| format!("{d:?}"));
                return Err(EmitError::ContractGap(format!(
                    "let-at-wait scope hazard (TASK-0364): data `{name}` ({d:?}) \
                     has a whole-array Wait nested in a loop (let-at-wait \
                     declare-and-assign emits `let {name} = ...;` inside the \
                     `for {{ }}` block), but a consumer of `{name}` sits at an \
                     ENCLOSING scope no Wait lexically dominates — the emitted \
                     `let {name}` would be out of scope at the consumer \
                     (rustc E0425). This shape is not producible by \
                     `transfer_inject` today; if a future pass (e.g. a hoist) \
                     constructs it, make the consumer's scope dominated by a \
                     Wait or retain the outer-scope `let mut {name}` pre-init."
                )));
            }
        }
    }
    Ok(())
}

/// Pre-order walk for [`check_let_at_wait_scope_safety`]. Records, per
/// DataId in `let_at_wait`, the current scope-`path` at every Wait of
/// the data and at every consumer of the data. `next_occurrence` is the
/// shared occurrence counter that stamps each entered Loop body with a
/// fresh identity (pushed on entry, popped on exit).
fn collect_scope_paths(
    events: &[Event],
    let_at_wait: &BTreeSet<DataId>,
    path: &mut Vec<usize>,
    next_occurrence: &mut usize,
    wait_paths: &mut BTreeMap<DataId, Vec<Vec<usize>>>,
    consumer_paths: &mut BTreeMap<DataId, Vec<Vec<usize>>>,
) {
    for e in events {
        match e {
            Event::Wait { data, .. } => {
                if let_at_wait.contains(data) {
                    wait_paths.entry(*data).or_default().push(path.clone());
                }
            }
            Event::Push { data, .. } => {
                if let_at_wait.contains(data) {
                    consumer_paths.entry(*data).or_default().push(path.clone());
                }
            }
            Event::Fire { bindings, .. } => {
                // A consumer is a READ of the data among the kernel
                // inputs (Fire OUTPUT writes are NOT consumers). Recurse
                // into Nested args so a data read inside a nested call
                // in argument position is counted at this Fire's scope.
                let mut reads: BTreeSet<DataId> = BTreeSet::new();
                for arg in &bindings.inputs {
                    collect_arg_data_reads(arg, &mut reads);
                }
                for d in reads {
                    if let_at_wait.contains(&d) {
                        consumer_paths.entry(d).or_default().push(path.clone());
                    }
                }
            }
            Event::Loop { body, .. } => {
                let occ = *next_occurrence;
                *next_occurrence += 1;
                path.push(occ);
                collect_scope_paths(
                    body,
                    let_at_wait,
                    path,
                    next_occurrence,
                    wait_paths,
                    consumer_paths,
                );
                path.pop();
            }
            _ => {}
        }
    }
}

/// Collect every DataId read by an `ArgBinding`, recursing into
/// `ArgBinding::Nested` argument lists. `ArgBinding::Scalar` reads no
/// data.
fn collect_arg_data_reads(arg: &ArgBinding, out: &mut BTreeSet<DataId>) {
    match arg {
        ArgBinding::Data(slice) => {
            out.insert(slice.data);
        }
        ArgBinding::Nested { args, .. } => {
            for a in args {
                collect_arg_data_reads(a, out);
            }
        }
        ArgBinding::Scalar(_) => {}
    }
}

/// `true` iff `prefix` is a (non-strict) prefix of `full` — i.e. the
/// scope at `prefix` lexically dominates (encloses, or equals) the
/// scope at `full`. The empty path (root scope) is a prefix of every
/// path.
fn is_prefix(prefix: &[usize], full: &[usize]) -> bool {
    prefix.len() <= full.len() && full[..prefix.len()] == *prefix
}
