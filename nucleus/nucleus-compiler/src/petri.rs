//! Petri-net IR data structures (PRD §8).
//!
//! Nucleus v2 uses a deterministic, bounded, place/transition Petri net
//! as the central scheduling IR (PRD §8.1). The scheduler is a pure
//! function `(AlgoIR, SchedIR) -> (GlobalNet, {WorkerId -> EventList})`:
//! the per-worker `EventList`s (TASK-0015, `crate::event`) are
//! projections of the `GlobalNet` defined here.
//!
//! The full power of Petri nets is *deliberately* not exposed. PRD
//! §8.4 fences v2 to a small subclass:
//!
//! - **Statically determined firing order.** Firing order is decided
//!   at compile time, not by token availability at runtime. No
//!   free-choice, no confusion. The struct in this module *can*
//!   simulate arbitrary fireable orders (it's a generic place/
//!   transition net, the small library PRD §13 wants), but the
//!   scheduler will only ever drive it along a single linearised
//!   sequence; the simulation surface is here to make analysis
//!   passes (boundedness, deadlock) and tests cheap to write.
//! - **Bounded by construction.** Every production `Place` carries a
//!   `capacity`. A `fire` that would exceed capacity is a hard
//!   `FireError::CapacityExceeded`. The `Option<NonZeroU32>` lets
//!   *analysis* nets temporarily leave a place unbounded; production
//!   v2 nets emitted by the scheduler always set `Some(_)`.
//! - **Acyclic global event DAG.** Cycle detection is *not* implemented
//!   here; it lives one layer up in the lowering pass (TASK-0026)
//!   which has the per-worker order plus the `Push`/`Wait` arcs to
//!   make the DAG check meaningful. This module only stores the net.
//! - **No coloured, stochastic, probabilistic, or hierarchical
//!   extensions.** Tokens are uncoloured `u32` counts. No timing, no
//!   probabilities, no sub-nets.
//!
//! ## Why hand-rolled `Vec<Place>` and not `petgraph`
//!
//! The set of operations we need is tiny — push back, look up by id,
//! iterate. A graph library buys us less than the dependency it adds.
//! The PRD §13 budget of ~500 lines for the whole net library
//! presupposes a hand-rolled `Vec` + `BTreeMap` shape. If that ever
//! changes (e.g. we want incremental subgraph reachability over a
//! petgraph CSR), we revisit.
//!
//! ## Layout
//!
//! - [`PlaceId`] / [`TransitionId`] — opaque `u32` newtypes; assigned
//!   monotonically by the [`Net`] on insertion.
//! - [`Place`] / [`Transition`] / [`Arc`] — the three node/edge types
//!   per PRD §8.2's mapping table.
//! - [`Marking`] — `BTreeMap<PlaceId, u32>` newtype; absent key means
//!   zero tokens.
//! - [`Net`] — the container with `add_*`, `fire`, `reset_to_initial`,
//!   `enabled_transitions`, `serialize_to_dot`.
//! - [`FireError`] — fail-fast errors with the offending IDs in the
//!   payload so diagnostics can point at the source schedule.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::num::NonZeroU32;

use crate::event::WorkerId;

// --------------------------------------------------------------------
// Opaque identifiers
// --------------------------------------------------------------------

/// Opaque identifier for a [`Place`] inside one [`Net`]. Assigned on
/// insertion; valid only inside the net that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlaceId(pub u32);

/// Opaque identifier for a [`Transition`] inside one [`Net`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransitionId(pub u32);

// --------------------------------------------------------------------
// Place, Transition, Arc
// --------------------------------------------------------------------

/// A Petri-net place. Holds zero or more tokens up to its capacity.
///
/// `capacity = None` means "unbounded for analysis"; production v2
/// nets always carry `Some(_)` because schedule-level `buffer=N`
/// directives (PRD §6.3.4) lower to a concrete bound. Leaving the
/// type permissive keeps test fixtures and counter-example nets ergonomic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub id: PlaceId,
    /// Human-readable label, surfaced in error messages and DOT output.
    /// Not required to be unique; uniqueness is the net builder's
    /// problem if it cares.
    pub name: String,
    /// `None` = unbounded (analysis only). `Some(n)` = at most `n`
    /// tokens may be present.
    pub capacity: Option<NonZeroU32>,
    /// Token count present in the initial marking (PRD §8.2:
    /// "pipeline depth / latency-hiding head-start").
    pub initial_marking: u32,
}

/// A Petri-net transition. Carries an optional `worker` for projection:
/// when [`Net`] is partitioned into per-worker [`crate::event::Event`]
/// lists (TASK-0028), the `worker` field decides where the firing
/// projects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub id: TransitionId,
    pub name: String,
    /// Owning worker for projection. `None` for transitions that are
    /// not owned by a worker (e.g. analysis-only auxiliary firings).
    pub worker: Option<WorkerId>,
}

/// Arc direction. v2 has no inhibitor or reset arcs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArcKind {
    /// Place -> Transition. Firing the transition consumes `weight`
    /// tokens from `place`.
    PtoT,
    /// Transition -> Place. Firing the transition deposits `weight`
    /// tokens into `place`.
    TtoP,
}

/// A weighted arc between a [`Place`] and a [`Transition`]. Weight 0
/// is rejected by [`Net::add_arc`] — it would never enable / never
/// produce, and is almost always a bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arc {
    pub kind: ArcKind,
    pub place: PlaceId,
    pub transition: TransitionId,
    pub weight: u32,
}

// --------------------------------------------------------------------
// Marking
// --------------------------------------------------------------------

/// Token counts per place. Absence of a key means zero. We use
/// `BTreeMap` (not `HashMap`) so iteration is deterministic — the
/// scheduler relies on stable orderings for reproducible codegen
/// (PRD §8.6's "reproducible, inspectable" linearisation).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Marking(pub BTreeMap<PlaceId, u32>);

impl Marking {
    /// Empty marking (all places hold zero tokens).
    pub fn new() -> Self {
        Marking(BTreeMap::new())
    }

    /// Tokens in `p`. Zero if `p` has never been touched.
    pub fn get(&self, p: PlaceId) -> u32 {
        self.0.get(&p).copied().unwrap_or(0)
    }

    /// Set `p`'s token count to `n`. `n == 0` removes the entry so
    /// equality stays canonical.
    pub fn set(&mut self, p: PlaceId, n: u32) {
        if n == 0 {
            self.0.remove(&p);
        } else {
            self.0.insert(p, n);
        }
    }
}

// --------------------------------------------------------------------
// FireError
// --------------------------------------------------------------------

/// Reasons a [`Net::fire`] can fail. All carry enough context to point
/// at the offending element in user-facing error messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FireError {
    /// The transition id is not in the net. Always a programming
    /// error (caller built a `TransitionId` not handed out by this
    /// net), never a token-availability issue.
    UnknownTransition(TransitionId),
    /// An input place did not hold enough tokens. `have < need`.
    NotEnabled {
        transition: TransitionId,
        place: PlaceId,
        have: u32,
        need: u32,
    },
    /// An output place would exceed its declared capacity after the
    /// firing. `would_be > capacity`.
    CapacityExceeded {
        transition: TransitionId,
        place: PlaceId,
        would_be: u32,
        capacity: NonZeroU32,
    },
}

impl std::fmt::Display for FireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FireError::UnknownTransition(t) => {
                write!(f, "unknown transition {:?}", t)
            }
            FireError::NotEnabled {
                transition,
                place,
                have,
                need,
            } => write!(
                f,
                "transition {:?} not enabled: place {:?} has {} token(s), needs {}",
                transition, place, have, need
            ),
            FireError::CapacityExceeded {
                transition,
                place,
                would_be,
                capacity,
            } => write!(
                f,
                "transition {:?} firing would overflow place {:?}: would be {}, capacity {}",
                transition, place, would_be, capacity
            ),
        }
    }
}

impl std::error::Error for FireError {}

// --------------------------------------------------------------------
// Net
// --------------------------------------------------------------------

/// Per-transition incident-arc adjacency index (TASK-0377).
///
/// `Net::fire` originally scanned *all* arcs twice per call (once for
/// the `PtoT` inputs, once for the `TtoP` outputs) to find the arcs
/// incident to the transition being fired — O(A) per fire. The three
/// analysis passes ([`crate::passes::boundedness::check_bounded`],
/// [`crate::passes::boundedness::derive_firing_order`],
/// [`crate::passes::deadlock::check_deadlock_free`]) each fire ~T
/// transitions, so the gate was O(T·A) and dominated build time on
/// large multi-worker nets (435 ms / 99% of build on 07-matmul
/// distributed8: T=4149, A=65722).
///
/// This index lets [`Net::fire_in_place`] find a transition's incident
/// arcs in O(deg(t)). Build it ONCE per analysis (or thread one through
/// all three, since it is keyed by [`TransitionId`] which is stable
/// across [`Net::clone`]) via [`Net::build_arc_index`].
///
/// ## Determinism (load-bearing)
///
/// The per-transition arc-index vectors are populated by iterating
/// `net.arcs` in insertion order, so each `in_arcs[t]` / `out_arcs[t]`
/// list preserves arc-insertion order. `fire_in_place` sums arc
/// weights into a `BTreeMap` keyed by place (order-independent), so the
/// firing result is byte-identical to the old all-arcs-scan path
/// regardless — but keeping insertion order means any future
/// order-sensitive reader sees the same sequence the scan produced.
///
/// ## Validity across `Net::clone`
///
/// The index stores `usize` indices into `net.arcs` and is keyed by
/// transition index (== `TransitionId.0`). [`TransitionId`]s equal
/// their position in `net.transitions`, and `Clone` preserves both
/// `transitions` and `arcs` element-for-element, so an index built from
/// the original net is valid against a clone of it. The analysis passes
/// rely on this: they `net.clone()` + `reset_to_initial()` then replay
/// on the clone using an index built from the original.
#[derive(Debug, Clone)]
pub struct ArcIndex {
    /// `in_arcs[t.0]` = indices into `net.arcs` of the `PtoT` arcs
    /// whose `transition == t`, in arc-insertion order.
    in_arcs: Vec<Vec<usize>>,
    /// `out_arcs[t.0]` = indices into `net.arcs` of the `TtoP` arcs
    /// whose `transition == t`, in arc-insertion order.
    out_arcs: Vec<Vec<usize>>,
}

/// The Petri net container. Built incrementally via `add_place`,
/// `add_transition`, `add_arc`. Its `initial_marking` is computed
/// from each `Place::initial_marking` at the first `reset_to_initial`
/// call (or implicitly at `fire` time if never reset; see
/// [`Net::current_marking`]).
///
/// IDs are assigned monotonically and are valid only inside this net.
#[derive(Debug, Clone, Default)]
pub struct Net {
    pub places: Vec<Place>,
    pub transitions: Vec<Transition>,
    pub arcs: Vec<Arc>,
    /// Marking at `reset_to_initial` time. Kept separate from
    /// `current_marking` so a test can fire forward and rewind.
    pub initial_marking: Marking,
    /// Mutated by `fire`. Initialised lazily from each place's
    /// `initial_marking` on first construction.
    pub current_marking: Marking,
}

impl Net {
    /// Empty net.
    pub fn new() -> Self {
        Net::default()
    }

    /// Add a place. Returns the freshly assigned [`PlaceId`]. The
    /// caller may ignore the `id` field on the passed-in `Place`
    /// (we overwrite it).
    pub fn add_place(
        &mut self,
        name: impl Into<String>,
        capacity: Option<NonZeroU32>,
        initial_marking: u32,
    ) -> PlaceId {
        let id = PlaceId(self.places.len() as u32);
        let p = Place {
            id,
            name: name.into(),
            capacity,
            initial_marking,
        };
        if initial_marking > 0 {
            self.initial_marking.set(id, initial_marking);
            self.current_marking.set(id, initial_marking);
        }
        self.places.push(p);
        id
    }

    /// Add a transition. Returns the freshly assigned [`TransitionId`].
    pub fn add_transition(
        &mut self,
        name: impl Into<String>,
        worker: Option<WorkerId>,
    ) -> TransitionId {
        let id = TransitionId(self.transitions.len() as u32);
        self.transitions.push(Transition {
            id,
            name: name.into(),
            worker,
        });
        id
    }

    /// Add a weighted arc. Panics (fail fast) if `weight == 0`, or if
    /// either endpoint id is out of range — both are programming
    /// errors, not runtime conditions.
    pub fn add_arc(
        &mut self,
        kind: ArcKind,
        place: PlaceId,
        transition: TransitionId,
        weight: u32,
    ) {
        assert!(
            weight > 0,
            "arc weight must be > 0 (zero weight is always a bug)"
        );
        assert!(
            (place.0 as usize) < self.places.len(),
            "add_arc: place id {:?} out of range (have {} places)",
            place,
            self.places.len()
        );
        assert!(
            (transition.0 as usize) < self.transitions.len(),
            "add_arc: transition id {:?} out of range (have {} transitions)",
            transition,
            self.transitions.len()
        );
        self.arcs.push(Arc {
            kind,
            place,
            transition,
            weight,
        });
    }

    /// Restore `current_marking` to `initial_marking`. Useful for
    /// tests that walk multiple firing sequences from the same start.
    pub fn reset_to_initial(&mut self) {
        self.current_marking = self.initial_marking.clone();
    }

    /// Build the per-transition incident-arc index (TASK-0377).
    ///
    /// O(A): one pass over `self.arcs` in insertion order, appending
    /// each arc's index to the input- or output-list of its transition.
    /// See [`ArcIndex`] for why insertion order is preserved and why
    /// the index stays valid across [`Net::clone`].
    pub fn build_arc_index(&self) -> ArcIndex {
        let n = self.transitions.len();
        let mut in_arcs: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut out_arcs: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (ai, a) in self.arcs.iter().enumerate() {
            // add_arc asserts the transition id is in range, so this
            // index is always valid for arcs already in the net.
            let ti = a.transition.0 as usize;
            match a.kind {
                ArcKind::PtoT => in_arcs[ti].push(ai),
                ArcKind::TtoP => out_arcs[ti].push(ai),
            }
        }
        ArcIndex { in_arcs, out_arcs }
    }

    /// Fire transition `t` against `current_marking`. On success the
    /// new marking is committed and returned (cloned). On failure
    /// nothing is mutated — the failure modes are all checked before
    /// any token moves.
    ///
    /// This is the convenience entry point for ad-hoc callers and
    /// tests that want the resulting [`Marking`]. It builds a one-shot
    /// [`ArcIndex`] (O(A)) and delegates to [`Net::fire_in_place`],
    /// then clones the committed marking. Hot paths that fire many
    /// transitions (the analysis passes) should build the index ONCE
    /// via [`Net::build_arc_index`] and call `fire_in_place` directly,
    /// avoiding both the per-fire index rebuild and the marking clone
    /// (TASK-0377).
    pub fn fire(&mut self, t: TransitionId) -> Result<Marking, FireError> {
        let index = self.build_arc_index();
        self.fire_in_place(t, &index)?;
        Ok(self.current_marking.clone())
    }

    /// Fire transition `t` against `current_marking`, consulting a
    /// prebuilt [`ArcIndex`] to find `t`'s incident arcs in O(deg(t))
    /// instead of scanning all arcs (TASK-0377). On success the new
    /// marking is committed *in place* (no clone is returned). On
    /// failure nothing is mutated — every failure mode is checked
    /// before any token moves, so the caller may treat
    /// `current_marking` as the pre-firing ("before") state inside an
    /// `Err` arm.
    ///
    /// Behaviour is identical to the old all-arcs-scan `fire`: `needs`
    /// and `produces` are summed into `BTreeMap`s keyed by place
    /// (order-independent), capacity is checked against the
    /// post-firing token count (deltas first, so a self-looping buffer
    /// is checked at its settled count not a transient peak), and the
    /// same `FireError` variants are produced. `index` MUST have been
    /// built from this net (or a clone of it); see [`ArcIndex`].
    pub fn fire_in_place(&mut self, t: TransitionId, index: &ArcIndex) -> Result<(), FireError> {
        if (t.0 as usize) >= self.transitions.len() {
            return Err(FireError::UnknownTransition(t));
        }
        let ti = t.0 as usize;

        // 1. Check enabled: every PtoT arc's source has enough tokens.
        //    Multiple arcs from the same place sum their weights.
        let mut needs: BTreeMap<PlaceId, u32> = BTreeMap::new();
        for &ai in &index.in_arcs[ti] {
            let a = &self.arcs[ai];
            *needs.entry(a.place).or_insert(0) = needs
                .get(&a.place)
                .copied()
                .unwrap_or(0)
                .checked_add(a.weight)
                .expect("arc-weight sum overflowed u32");
        }
        for (place, need) in &needs {
            let have = self.current_marking.get(*place);
            if have < *need {
                return Err(FireError::NotEnabled {
                    transition: t,
                    place: *place,
                    have,
                    need: *need,
                });
            }
        }

        // 2. Check capacity: every TtoP arc's destination, after the
        //    net token delta on that place, stays within capacity.
        //    Compute deltas first so a place that is both consumed
        //    from and produced into (e.g. a buffer that loops on
        //    itself) is checked at its post-firing count, not at a
        //    transient peak.
        let mut produces: BTreeMap<PlaceId, u32> = BTreeMap::new();
        for &ai in &index.out_arcs[ti] {
            let a = &self.arcs[ai];
            *produces.entry(a.place).or_insert(0) = produces
                .get(&a.place)
                .copied()
                .unwrap_or(0)
                .checked_add(a.weight)
                .expect("arc-weight sum overflowed u32");
        }
        // Union of touched places.
        let mut touched: Vec<PlaceId> = needs.keys().chain(produces.keys()).copied().collect();
        touched.sort();
        touched.dedup();
        for place in &touched {
            let have = self.current_marking.get(*place);
            let consumed = needs.get(place).copied().unwrap_or(0);
            let produced = produces.get(place).copied().unwrap_or(0);
            // `have >= consumed` was just enforced above for needs;
            // place is in `touched` because it appears in needs or
            // produces (or both). If it only appears in produces,
            // consumed == 0.
            let would_be = have
                .checked_sub(consumed)
                .and_then(|v| v.checked_add(produced))
                .expect("marking arithmetic overflowed u32");
            if let Some(cap) = self.places[place.0 as usize].capacity {
                if would_be > cap.get() {
                    return Err(FireError::CapacityExceeded {
                        transition: t,
                        place: *place,
                        would_be,
                        capacity: cap,
                    });
                }
            }
        }

        // 3. Commit. We already proved all checked_sub/checked_add hold.
        for place in &touched {
            let have = self.current_marking.get(*place);
            let consumed = needs.get(place).copied().unwrap_or(0);
            let produced = produces.get(place).copied().unwrap_or(0);
            let new = have - consumed + produced;
            self.current_marking.set(*place, new);
        }

        Ok(())
    }

    /// Return every transition that would succeed if fired against
    /// `marking`. Useful for tests and for the linearisation pass's
    /// validation step ("is the order I picked a legal interleaving?").
    /// Does *not* mutate the net.
    ///
    // TASK-0377: not indexed (off gate hot path). This still does the
    // O(T·A) all-arcs scan, but it is never called by the per-build
    // soundness gate (`check_net_sound`) — only by unit tests and the
    // linearisation validation path that runs on tiny nets. Indexing it
    // would be a behaviour-neutral mechanical change; deliberately left
    // un-indexed to keep the diff scoped to the measured hot path.
    pub fn enabled_transitions(&self, marking: &Marking) -> Vec<TransitionId> {
        let mut out = Vec::new();
        for t in &self.transitions {
            let mut needs: BTreeMap<PlaceId, u32> = BTreeMap::new();
            for a in self
                .arcs
                .iter()
                .filter(|a| a.transition == t.id && a.kind == ArcKind::PtoT)
            {
                *needs.entry(a.place).or_insert(0) += a.weight;
            }
            let enabled = needs.iter().all(|(p, n)| marking.get(*p) >= *n);
            // Capacity check too: a transition that would overflow an
            // output place is not "enabled" from a usability standpoint.
            let mut would_overflow = false;
            if enabled {
                let mut produces: BTreeMap<PlaceId, u32> = BTreeMap::new();
                for a in self
                    .arcs
                    .iter()
                    .filter(|a| a.transition == t.id && a.kind == ArcKind::TtoP)
                {
                    *produces.entry(a.place).or_insert(0) += a.weight;
                }
                for (place, prod) in &produces {
                    let have = marking.get(*place);
                    let consumed = needs.get(place).copied().unwrap_or(0);
                    let would_be = have - consumed + prod;
                    if let Some(cap) = self.places[place.0 as usize].capacity {
                        if would_be > cap.get() {
                            would_overflow = true;
                            break;
                        }
                    }
                }
            }
            if enabled && !would_overflow {
                out.push(t.id);
            }
        }
        out
    }

    /// Serialise to Graphviz DOT. Places are circles, transitions are
    /// boxes; arcs are labelled with their weight (omitted when 1).
    /// Initial marking is shown in each place label.
    ///
    /// This is the raw structural rendering. Per-worker colouring is
    /// the job of [`Net::serialize_to_dot_styled`], which the
    /// `nucleus --emit-pn` driver flag uses (PRD §8.5). Kept separate
    /// so internal analyses can dump a plain net without paying for
    /// the subgraph/palette machinery.
    pub fn serialize_to_dot(&self) -> String {
        let mut s = String::new();
        writeln!(s, "digraph petri {{").unwrap();
        writeln!(s, "  rankdir=LR;").unwrap();
        for p in &self.places {
            let cap = match p.capacity {
                Some(c) => format!("/{}", c.get()),
                None => "/inf".to_string(),
            };
            writeln!(
                s,
                "  p{} [shape=circle, label=\"{}\\n{}{}\"];",
                p.id.0, p.name, p.initial_marking, cap
            )
            .unwrap();
        }
        for t in &self.transitions {
            let w = match &t.worker {
                Some(w) => format!("\\nw{}", w.0),
                None => String::new(),
            };
            writeln!(s, "  t{} [shape=box, label=\"{}{}\"];", t.id.0, t.name, w).unwrap();
        }
        for a in &self.arcs {
            let (from, to) = match a.kind {
                ArcKind::PtoT => (format!("p{}", a.place.0), format!("t{}", a.transition.0)),
                ArcKind::TtoP => (format!("t{}", a.transition.0), format!("p{}", a.place.0)),
            };
            let label = if a.weight == 1 {
                String::new()
            } else {
                format!(" [label=\"{}\"]", a.weight)
            };
            writeln!(s, "  {} -> {}{};", from, to, label).unwrap();
        }
        writeln!(s, "}}").unwrap();
        s
    }

    /// Serialise to Graphviz DOT with per-worker colouring and an
    /// optional title (PRD §8.5). Drives `nucleus --emit-pn`.
    ///
    /// Layout:
    ///
    /// - Each worker's transitions are grouped in a Graphviz
    ///   `subgraph cluster_w<id>` and filled with a colour from
    ///   [`WORKER_PALETTE`]. The cluster border carries the same
    ///   colour so a viewer can identify the worker even when the
    ///   transitions are scattered by the layout engine.
    /// - Transitions without an owning worker (analysis-auxiliary
    ///   firings) live at the top level, uncoloured.
    /// - Places are uncoloured — they are shared infrastructure (PRD
    ///   §8.2: "places are data slots, channels, or barriers"; they
    ///   don't belong to a single worker).
    /// - Initial marking is shown in each place label as
    ///   `<initial>/<capacity>`; capacity `inf` for unbounded
    ///   analysis nets.
    /// - Arcs carry a weight label only when `weight > 1` (weight=1
    ///   is the default; labelling every arc would just be noise).
    ///
    /// The optional `title` is rendered as a `graph[label=...]`
    /// attribute. Useful for `<example>/<schedule>` annotation when
    /// dumping many nets side by side.
    ///
    /// Palette is a small fixed list ([`WORKER_PALETTE`]); workers
    /// beyond palette size wrap modulo. PRD §8.5 only requires
    /// "distinct colour per worker" within a single net, and the
    /// largest example (#14, hearing-aid) plans on 6 workers; 8
    /// entries gives headroom without forcing a colour theory choice.
    pub fn serialize_to_dot_styled(&self, title: Option<&str>) -> String {
        let mut s = String::new();
        writeln!(s, "digraph petri {{").unwrap();
        writeln!(s, "  rankdir=LR;").unwrap();
        writeln!(s, "  node [fontname=\"Helvetica\"];").unwrap();
        if let Some(t) = title {
            // DOT escapes: the label uses HTML-like escape for ".
            writeln!(s, "  label=\"{}\";", escape_dot(t)).unwrap();
            writeln!(s, "  labelloc=t;").unwrap();
        }

        // ---- Places: shared infrastructure, uncoloured. -------------
        for p in &self.places {
            let cap = match p.capacity {
                Some(c) => format!("/{}", c.get()),
                None => "/inf".to_string(),
            };
            writeln!(
                s,
                "  p{} [shape=circle, label=\"{}\\n{}{}\"];",
                p.id.0,
                escape_dot(&p.name),
                p.initial_marking,
                cap
            )
            .unwrap();
        }

        // ---- Transitions: grouped per-worker into clusters. ---------
        // Collect a stable, deterministic list of worker ids (BTreeMap
        // ordering by WorkerId.0) plus an "ownerless" bucket.
        let mut by_worker: BTreeMap<WorkerId, Vec<&Transition>> = BTreeMap::new();
        let mut ownerless: Vec<&Transition> = Vec::new();
        for t in &self.transitions {
            match &t.worker {
                Some(w) => by_worker.entry(*w).or_default().push(t),
                None => ownerless.push(t),
            }
        }

        for (w, ts) in &by_worker {
            let color = worker_color(*w);
            writeln!(s, "  subgraph cluster_w{} {{", w.0).unwrap();
            writeln!(s, "    label=\"worker {}\";", w.0).unwrap();
            writeln!(s, "    color=\"{}\";", color).unwrap();
            writeln!(s, "    style=filled;").unwrap();
            writeln!(s, "    fillcolor=\"{}\";", color).unwrap();
            // Transitions inside a cluster get a slightly stronger
            // outline so they read against the cluster fill.
            for t in ts {
                writeln!(
                    s,
                    "    t{} [shape=box, style=filled, fillcolor=\"white\", label=\"{}\\nw{}\"];",
                    t.id.0,
                    escape_dot(&t.name),
                    w.0
                )
                .unwrap();
            }
            writeln!(s, "  }}").unwrap();
        }
        for t in &ownerless {
            writeln!(
                s,
                "  t{} [shape=box, label=\"{}\"];",
                t.id.0,
                escape_dot(&t.name)
            )
            .unwrap();
        }

        // ---- Arcs: same as plain serializer. ------------------------
        for a in &self.arcs {
            let (from, to) = match a.kind {
                ArcKind::PtoT => (format!("p{}", a.place.0), format!("t{}", a.transition.0)),
                ArcKind::TtoP => (format!("t{}", a.transition.0), format!("p{}", a.place.0)),
            };
            let label = if a.weight == 1 {
                String::new()
            } else {
                format!(" [label=\"{}\"]", a.weight)
            };
            writeln!(s, "  {} -> {}{};", from, to, label).unwrap();
        }
        writeln!(s, "}}").unwrap();
        s
    }
}

/// Fixed colour palette for per-worker DOT subgraphs (PRD §8.5).
///
/// Eight entries: enough for the largest planned v2 example (#14,
/// hearing-aid, 6 workers across 3 worker classes) with headroom.
/// Workers beyond the palette wrap modulo — distinct-per-net
/// is only a best-effort guarantee. Chosen as light pastel fills
/// because the transition node labels are black text inside a
/// `fillcolor="white"` inner box; the cluster fill sits behind.
pub const WORKER_PALETTE: &[&str] = &[
    "lightblue",
    "lightgreen",
    "lightyellow",
    "lightpink",
    "lightcoral",
    "lightgrey",
    "lavender",
    "wheat",
];

/// Pick a colour for `worker` from [`WORKER_PALETTE`]. Deterministic;
/// the same worker always lands on the same colour for a given build
/// of nucleus. Wraps modulo palette length.
fn worker_color(worker: WorkerId) -> &'static str {
    WORKER_PALETTE[(worker.0 as usize) % WORKER_PALETTE.len()]
}

/// Minimal DOT string escape: backslash and double-quote. Newlines
/// are left as `\n` since callers already pass `\\n` for in-label
/// line breaks. Other control characters are not expected — kernel
/// and place names come from the user's algorithm/schedule source
/// which is plain ASCII identifiers plus a few separators.
fn escape_dot(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}
