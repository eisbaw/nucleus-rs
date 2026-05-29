//! Shared multi-worker event-walker. Originating consumers were the
//! pthreads-sync and pthreads-async backends (TASK-0239 cycle 31);
//! mp-tcp-event joined as the third consumer at cycle 79 with
//! `rendezvous_prefix = "chan"`. mp-tcp-bufsync is the fourth tier-1
//! backend but bypasses this walker entirely — it calls
//! `render_wait_assign` directly without going through
//! `render_worker_events`.
//!
//! # Why this module exists
//!
//! Cycle 26 (TASK-0228 Wave B-2) implemented pthreads-async multi-worker
//! emit by COPYING ~400 LoC of pthreads-sync's walker
//! (`render_worker_events`, `render_wait_assign`, the `WaitSlice`
//! shape-dispatch (pre-TASK-0294 `leading_axis_slice` / `LeadingAxis`),
//! `collect_pre_init_sets`, `collect_xfer_pairs`, `collect_worker_slots`,
//! `collect_barriers_by_tag`), substituting `slot_<id>`
//! for `ring_<id>` at the four Push/Wait callsites. The duplication was
//! mechanically maintainable for one cycle but every subsequent edit
//! to the walker would risk silent drift between the two then-
//! existing backends whose cross-backend bit-identical differential
//! (PRD §10.1) is the headline thesis-falsifiability claim. (mp-tcp-
//! event later joined as the third prefix-using consumer at cycle 79;
//! mp-tcp-bufsync remains on the direct-`render_wait_assign` bypass.)
//!
//! TASK-0239 (this module) lifts the walker into a single source of
//! truth parameterised by ONE string: the rendezvous variable prefix
//! (`"slot"` for pthreads-sync, `"ring"` for pthreads-async). Everything
//! else (Fire / Loop / Sync / Wait gather, check_frame instrumentation,
//! per-worker partition range override, per-occurrence strip-mine
//! block_tag rebinding (TASK-0181; the header + abs_subst-construction
//! half is the shared [`render_block_tag_loop_header`] helper that
//! mp-tcp-bufsync ALSO consumes — TASK-0253),
//! barrier identity via `SyncTag`, slice-paste 1D/2D-tile arithmetic
//! (TASK-0117 leading-axis path + TASK-0294 row-loop path for 2D
//! `partition=blocks2d` tiles)) is shared verbatim across both
//! backends — there is no second axis of variation worth a trait
//! abstraction.
//!
//! # What stays per-backend
//!
//! The shared walker handles only the per-worker EventList walk and the
//! Wait gather. Each backend's `Plan::emit` retains:
//!
//! - The substrate struct decl (`struct Slot<T> { Mutex+Condvar }` vs
//!   the bounded `Ring<T>` from `pthreads_async::ring_buffer::emit_ring_
//!   struct_decl`).
//! - The per-pair instance allocation (`Slot::new()` vs `Ring::new(cap)`,
//!   where cap is sidecar-derived).
//! - The `Plan` struct definition itself (the async variant carries
//!   an extra `ring_caps: BTreeMap<(DataId, SeqTag), u64>` for sizing).
//!
//! That keeps each prefix-using backend's real semantic difference
//! visible at the `emit()` entry point — pthreads-sync (one-shot
//! `Slot<T>` rendezvous via `Mutex+Condvar`), pthreads-async
//! (bounded `Ring<T>` buffered channel), mp-tcp-event (mio reactor
//! with bounded outbound queues and `seq`-keyed inbound queues).
//! The "What stays per-backend" section above enumerates only the
//! pthreads-sync / pthreads-async axis (the cycle-31 originating
//! pair); mp-tcp-event's substrate (`runtime_src.rs` + per-peer
//! `Chan<T>`) is documented in its own module-doc at
//! `nucleus/backends/mp-tcp-event/src/multi_worker.rs`.
//!
//! # Design choice: direct parameter, not trait
//!
//! Option (B) from the cycle-31 plan: pass `rendezvous_prefix: &str`
//! through a small `WalkerCtx` struct. Option (A) (a `RendezvousDispatch`
//! trait) was rejected because the variation is a single string; the
//! existing `RenderCtxPub` shared-helper precedent in `lib.rs` is also
//! direct-pass. A trait would introduce dispatch ceremony for no second
//! axis.
//!
//! # SlotId == RingId == usize
//!
//! Confirmed by all three prefix-using backends' type aliases:
//! `type SlotId = RendezvousId` (pthreads-sync), `type RingId =
//! RendezvousId` (pthreads-async), `type ChanId = RendezvousId`
//! (mp-tcp-event). The shared map shape is `BTreeMap<(DataId,
//! SeqTag), usize>` and is reused verbatim. mp-tcp-bufsync does
//! not need a rendezvous-id alias because it bypasses
//! `render_worker_events`.

pub mod block_tag;
pub mod collect;
pub mod ctx;
pub mod event_walker;
pub mod wait;

pub use block_tag::{compute_block_tag_abs_exprs, render_block_tag_loop_header};
pub use collect::{
    check_accumulator_consistency, check_let_at_wait_scope_safety, collect_accumulate_waits,
    collect_barriers_by_tag, collect_let_at_wait_data, collect_pair_tiles, collect_pre_init_sets,
    collect_worker_rendezvous, collect_xfer_pairs,
};
pub use ctx::{RendezvousId, WalkerCtx};
pub use event_walker::render_worker_events;
pub use wait::render_wait_assign;
