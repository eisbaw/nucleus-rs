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
    pub fn add_arc(&mut self, kind: ArcKind, place: PlaceId, transition: TransitionId, weight: u32) {
        assert!(weight > 0, "arc weight must be > 0 (zero weight is always a bug)");
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

    /// Fire transition `t` against `current_marking`. On success the
    /// new marking is committed and returned (cloned). On failure
    /// nothing is mutated — the failure modes are all checked before
    /// any token moves.
    pub fn fire(&mut self, t: TransitionId) -> Result<Marking, FireError> {
        if (t.0 as usize) >= self.transitions.len() {
            return Err(FireError::UnknownTransition(t));
        }

        // 1. Check enabled: every PtoT arc's source has enough tokens.
        //    Multiple arcs from the same place sum their weights.
        let mut needs: BTreeMap<PlaceId, u32> = BTreeMap::new();
        for a in self.arcs.iter().filter(|a| a.transition == t && a.kind == ArcKind::PtoT) {
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
        for a in self.arcs.iter().filter(|a| a.transition == t && a.kind == ArcKind::TtoP) {
            *produces.entry(a.place).or_insert(0) = produces
                .get(&a.place)
                .copied()
                .unwrap_or(0)
                .checked_add(a.weight)
                .expect("arc-weight sum overflowed u32");
        }
        // Union of touched places.
        let mut touched: Vec<PlaceId> =
            needs.keys().chain(produces.keys()).copied().collect();
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

        Ok(self.current_marking.clone())
    }

    /// Return every transition that would succeed if fired against
    /// `marking`. Useful for tests and for the linearisation pass's
    /// validation step ("is the order I picked a legal interleaving?").
    /// Does *not* mutate the net.
    pub fn enabled_transitions(&self, marking: &Marking) -> Vec<TransitionId> {
        let mut out = Vec::new();
        for t in &self.transitions {
            let mut needs: BTreeMap<PlaceId, u32> = BTreeMap::new();
            for a in self.arcs.iter().filter(|a| a.transition == t.id && a.kind == ArcKind::PtoT) {
                *needs.entry(a.place).or_insert(0) += a.weight;
            }
            let enabled = needs.iter().all(|(p, n)| marking.get(*p) >= *n);
            // Capacity check too: a transition that would overflow an
            // output place is not "enabled" from a usability standpoint.
            let mut would_overflow = false;
            if enabled {
                let mut produces: BTreeMap<PlaceId, u32> = BTreeMap::new();
                for a in self.arcs.iter().filter(|a| a.transition == t.id && a.kind == ArcKind::TtoP) {
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
    /// Format matches the inspection-tool convention from PRD §8.5:
    /// per-worker colouring is *not* applied here (that's a backend
    /// concern); this is the raw structural rendering.
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
            writeln!(
                s,
                "  t{} [shape=box, label=\"{}{}\"];",
                t.id.0, t.name, w
            )
            .unwrap();
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
}
