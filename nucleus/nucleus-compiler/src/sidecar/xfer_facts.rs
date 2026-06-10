//! The unified per-`SeqTag` transfer-facts value carried on
//! [`NameSidecar::xfer_facts`](super::NameSidecar::xfer_facts)
//! (TASK-0455.08).
//!
//! Split out of `sidecar.rs` to keep that file under the 1000-LoC
//! mega-file fence (`just check-mega-files`), the same TASK-0383 /
//! TASK-0343.01.01 precedent that carved out `collectors` and
//! `cumulative_tests`. [`XferFacts`] is re-exported from `super`
//! (`pub use xfer_facts::XferFacts;`) so it is reachable as
//! `nucleus_compiler::sidecar::XferFacts` exactly as before.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::acfg::NotifyMode;
use crate::sched::TransportMode;

/// The unified per-`SeqTag` transfer facts a backend needs to lower a
/// matched Push/Wait pair (TASK-0455.08). One value per
/// [`NameSidecar::xfer_facts`](super::NameSidecar::xfer_facts) entry; the
/// key is the [`SeqTag`](crate::event::SeqTag) that
/// `Event::Push { seq, .. }` and `Event::Wait { seq, .. }` carry, so a
/// codegen consumer joins the event's `seq` here with no name
/// round-trip.
///
/// ## Why one struct instead of parallel maps
///
/// Each field used to ride its OWN `BTreeMap<SeqTag, _>` on
/// [`NameSidecar`](super::NameSidecar) (`transfer_buffer_for_seq`,
/// TASK-0233; `transfer_transport_for_seq`, TASK-0438.02) with its own
/// collector, serde-default, and forward-clone — and `notify` had no
/// sidecar surface at all despite being a [`crate::acfg::TransferPolicy`]
/// field. N independent maps keyed by the same join key is exactly the
/// copy-mirror divergence hazard the
/// [`KernelSig`](super::KernelSig) comment records (the purity
/// omission). Folding them into one value populated by ONE collector
/// (`collect_xfer_facts`) makes a missing/stale field a compile error at
/// the struct, not a silent skew between maps.
///
/// ## Sources
///
/// `buffer` / `transport` / `notify` are copied verbatim from the
/// matched `XferPlaceholder.policy` ([`crate::acfg::TransferPolicy`]).
/// `pipeline_depth` is the ONE field NOT from `policy`: it mirrors
/// `ACFG::pipeline_depth_for_seq` (see [`Self::pipeline_depth`]).
///
/// ## Wire-shape seam (TASK-0455.07 / TASK-0453.22)
///
/// The per-edge *wire shape* (precise gather/scatter element layout for
/// a non-whole-array transfer) is the next per-seq fact slated to land
/// here. It is deliberately NOT a field yet: no producer computes it and
/// no backend reads it, so adding a serde-bearing `Option<XferWire>` now
/// would be a dead field that every serde generator must still produce.
/// When TASK-0455.07/TASK-0453.22 lands the producer, add the field
/// here (additive, `serde(default)`) — this struct is its intended home,
/// keeping the backend-facing per-seq surface a single unified value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct XferFacts {
    /// In-flight transfer capacity — the `transfer DATA : buffer=N`
    /// value (TASK-0233). Pthreads-async multi-worker codegen
    /// (TASK-0228 Wave B) and the `event_plan`/`tcp_plan` substrates
    /// size the per-(DataId, SeqTag) channel/`Arc<Ring<T>>` to this.
    /// Sync transfers carry the `TransferPolicy::default` value `1`;
    /// async transfers carry the schedule's chosen `buffer=N`. Always
    /// `>= 1` (the grammar/lowering reject `buffer=0`).
    pub buffer: u64,

    /// Backend transport-path hint — the `transfer DATA : mode=pio|dma`
    /// hint (TASK-0438.02). The embedded-pattern backend joins this with
    /// `Event::Push`/`Event::Wait`'s `seq` to choose between the PIO
    /// byte-loop (`shim.link_push`/`link_recv`) and a structurally
    /// distinct DMA-shaped descriptor-arm + completion-spin emit. A seq
    /// whose fact is absent (no `xfer_facts` entry) or [`TransportMode::Pio`]
    /// renders the unchanged PIO path — so any schedule without a `mode=`
    /// directive is byte-identical to pre-TASK-0438.02 (load-bearing for
    /// the 02-split-add / 14-hearing-aid byte-exact gate).
    pub transport: TransportMode,

    /// Notification mode — the `transfer DATA : notify=event|poll`
    /// directive ([`crate::acfg::NotifyMode`]), threaded per-seq end-to-end
    /// for the first time by TASK-0455.08 (it previously had NO sidecar
    /// surface). [`NotifyMode::Default`] means the schedule stated no
    /// preference and the backend picks. The per-seq consumer that
    /// honours an explicit `event`/`poll` choice is TASK-0455.02; until
    /// then backends MAY read this fact but the shipped backends treat it
    /// as advisory (the default-pick path is unchanged), so carrying it is
    /// observationally inert pre-TASK-0455.02.
    pub notify: NotifyMode,

    /// Pipeline pre-fill depth — `Some(D)` iff this seq's Push/Wait pair
    /// was created inside a `loop V : pipeline=D` body (TASK-0134),
    /// `None` otherwise.
    ///
    /// ## Why this is a MIRROR, not the source of truth
    ///
    /// Unlike `buffer`/`transport`/`notify` (which are
    /// [`crate::acfg::TransferPolicy`] fields), pipeline depth lives on
    /// `ACFG::pipeline_depth_for_seq` and is consumed ONLY by the
    /// middle-end: `acfg_to_petri::buffer_place_for` pre-seeds each
    /// buffer place's initial marking with `D` tokens (producer-runs-
    /// ahead semantics, PRD §8.2), and `host_data_relay_inject` mirrors
    /// it onto fresh relay-hop seqs. NO backend reads it — pthreads-async
    /// even asserts `pipeline_depth`/`pre_fill`/`initial_marking` never
    /// appear in emitted code
    /// (`pthreads-async/tests/multi_worker_codegen.rs`). The ACFG stays
    /// the single source so the Petri/initial-marking layer is not
    /// duplicated; `build_sidecar` copies it here purely so `XferFacts`
    /// is the COMPLETE backend-facing per-seq surface (a future backend
    /// that wants the depth reads it from one place). The mirror is
    /// derived in `build_sidecar`, never mutated independently.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pipeline_depth: Option<std::num::NonZeroU64>,
}

impl Default for XferFacts {
    /// The all-defaults transfer facts: a sync, single-buffered, PIO,
    /// no-notify-preference, non-pipelined edge — exactly what a
    /// `transfer DATA;` directive with no options produces. Mirrors
    /// [`crate::acfg::TransferPolicy::default`] for the policy-derived
    /// fields and `None` (no pipeline loop) for the mirror.
    fn default() -> Self {
        XferFacts {
            buffer: 1,
            transport: TransportMode::Pio,
            notify: NotifyMode::Default,
            pipeline_depth: None,
        }
    }
}
