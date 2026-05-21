//! Pure-function emitters for the pthreads-async multi-worker
//! ring-buffer runtime substrate. PRD §7.1, TASK-0042.01, TASK-0228.
//!
//! # Status (TASK-0228 Wave A, cycle 18, 2026-05-22)
//!
//! This module ships the pure-function emit helpers for the file-scope
//! `Ring<T>` struct + impl, and a per-instance `Arc<Ring<T>>`
//! declaration helper. **Not yet integrated** into [`super::emit`] —
//! the multi-worker arm still ContractGaps. Wave B (next cycle) wires
//! these helpers into a multi-worker `Plan` (mirroring
//! `pthreads_sync::multi_worker::Plan`) that emits per-worker
//! `thread::spawn` bodies with `ring_<id>.push(...)` /
//! `ring_<id>.wait()` dispatch from `Event::Push` / `Event::Wait`.
//!
//! Splitting Wave A off makes the runtime substrate
//! **independently testable** — the `Ring<T>` struct shape and the
//! per-instance declaration shape are pinned by unit tests in
//! `tests/multi_worker_codegen.rs` BEFORE the integration that
//! consumes them lands, so a structural defect in the substrate
//! surfaces against a focused test, not buried inside a larger
//! integration commit.
//!
//! # Design — forward-carried from TASK-0042.01 + TASK-0228
//!
//! - **Container**: `std::sync::Mutex<std::collections::VecDeque<T>>` +
//!   two `std::sync::Condvar`s (one signals "not empty"; one signals
//!   "not full"). Capacity is stored in the instance (`cap: usize`),
//!   NOT in the type — one `Ring<T>` struct definition serves every
//!   instance regardless of `(DataId, SeqTag)` size.
//! - **Boundedness**: producer blocks on `not_full` while
//!   `ring.len() == cap`; consumer blocks on `not_empty` while
//!   `ring.is_empty()`. Each side's release notifies its peer's
//!   condvar. `notify_one` (not `notify_all`) because each peer
//!   guards a single waiter under the schedule's analysis-net
//!   structure — `notify_all` would correctly converge but at extra
//!   scheduler cost.
//! - **Pre-fill (NONE)**: per the post-TASK-0213 corrected contract,
//!   the ring STARTS EMPTY. `D` (pipeline depth, IR's analysis-only
//!   invariant) is NOT used at runtime — the ring is sized to
//!   `buffer=N` (where `N >= D` is guaranteed by the link-step
//!   `PipelineExceedsBuffer` check) and `D` appears nowhere in the
//!   emitted code. Pre-filling with `D` tokens at thread spawn
//!   WOULD BE A DEFECT — the producer's first `N` real pushes would
//!   then deposit on top of an already-D-full ring and overflow at
//!   runtime (the IR's analysis-net algebra is consistent only
//!   because `acfg_to_petri` elides the first `D` Push TtoP arcs as
//!   "head-start credit"; the runtime has no such elision).
//! - **Mutex poisoning**: `.lock().unwrap()` mirrors the
//!   pthreads-sync `Slot<T>` precedent (multi_worker.rs:279). If a
//!   producer thread panics while holding the mutex, every consumer
//!   panics on `.lock().unwrap()` — the desired propagation under
//!   `panic = "abort"` (the program SIGABRTs as a unit; no
//!   thread is left dangling on a poisoned guard).

use std::fmt::Write as _;

/// Emit the file-scope `Ring<T>` struct + impl. Idempotent at the
/// caller layer (the caller is responsible for emitting this exactly
/// once per generated file; emitting twice would compile but is a
/// codegen defect — pin it via callsite invariants, not by guarding
/// here).
///
/// The struct + impl shape is held STABLE by unit tests in
/// `tests/multi_worker_codegen.rs` — any change to the emit string
/// (a renamed field, a different condvar pair, a removed `cap`
/// argument) trips those tests, which is the desired behaviour
/// (drift detection at the codegen boundary, not in production).
pub fn emit_ring_struct_decl(out: &mut String) {
    writeln!(out, "/// Bounded ring buffer used as a per-(DataId, SeqTag)").ok();
    writeln!(out, "/// channel between producer and consumer threads.").ok();
    writeln!(out, "/// Capacity is stored in the instance so a single").ok();
    writeln!(out, "/// `Ring<T>` definition serves rings of every size.").ok();
    writeln!(out, "/// Starts EMPTY (no pre-fill). Producer `push(v)`").ok();
    writeln!(out, "/// blocks while `ring.len() == cap`; consumer `wait()`").ok();
    writeln!(out, "/// blocks while empty. See pthreads-async's TASK-0228").ok();
    writeln!(out, "/// notes for the post-TASK-0213 ring-EMPTY contract.").ok();
    writeln!(out, "struct Ring<T> {{").ok();
    writeln!(out, "    mu: std::sync::Mutex<std::collections::VecDeque<T>>,").ok();
    writeln!(out, "    cap: usize,").ok();
    writeln!(out, "    not_empty: std::sync::Condvar,").ok();
    writeln!(out, "    not_full: std::sync::Condvar,").ok();
    writeln!(out, "}}").ok();
    writeln!(out, "impl<T> Ring<T> {{").ok();
    writeln!(out, "    fn new(cap: usize) -> Self {{").ok();
    writeln!(out, "        Ring {{").ok();
    writeln!(out, "            mu: std::sync::Mutex::new(std::collections::VecDeque::with_capacity(cap)),").ok();
    writeln!(out, "            cap,").ok();
    writeln!(out, "            not_empty: std::sync::Condvar::new(),").ok();
    writeln!(out, "            not_full: std::sync::Condvar::new(),").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "    fn push(&self, v: T) {{").ok();
    writeln!(out, "        let mut g = self.mu.lock().unwrap();").ok();
    writeln!(out, "        while g.len() == self.cap {{").ok();
    writeln!(out, "            g = self.not_full.wait(g).unwrap();").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        g.push_back(v);").ok();
    writeln!(out, "        self.not_empty.notify_one();").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "    fn wait(&self) -> T {{").ok();
    writeln!(out, "        let mut g = self.mu.lock().unwrap();").ok();
    writeln!(out, "        while g.is_empty() {{").ok();
    writeln!(out, "            g = self.not_empty.wait(g).unwrap();").ok();
    writeln!(out, "        }}").ok();
    writeln!(out, "        let v = g.pop_front().unwrap();").ok();
    writeln!(out, "        self.not_full.notify_one();").ok();
    writeln!(out, "        v").ok();
    writeln!(out, "    }}").ok();
    writeln!(out, "}}").ok();
}

/// Emit a per-instance `Arc<Ring<T>>` declaration with the documented
/// capacity. The instance lives in the host thread's setup phase
/// (mirrors `pthreads_sync::multi_worker::Plan`'s slot_<id> per-slot
/// `Arc<Slot<T>>` declaration at multi_worker.rs:392), and gets
/// cloned into each `thread::spawn` closure for the producer +
/// consumer workers.
///
/// `var_name` is the FULL variable name the caller has chosen (e.g.
/// `ring_d1_s2` for `(DataId(1), SeqTag(2))`); this function does NOT
/// invent it — the caller owns the (DataId, SeqTag) -> identifier
/// mapping (the same mapping `pthreads_sync::multi_worker` uses for
/// `slot_<id>`).
///
/// `element_type` is the renderable Rust type for the channel
/// element (e.g. `Vec<f32>` for an array transfer, `f32` for a
/// scalar). The caller derives this from `NameSidecar::data_types`.
///
/// `cap` is the `transfer DATA : buffer=N` value (the link step
/// guarantees `cap >= D` for any `pipeline=D` schedule that reaches
/// codegen — so the runtime ring is sized to hold every concurrent
/// in-flight token without producer-side blocking under steady state).
pub fn emit_ring_instance_decl(
    out: &mut String,
    var_name: &str,
    element_type: &str,
    cap: u64,
) {
    writeln!(
        out,
        "    let {var_name}: std::sync::Arc<Ring<{element_type}>> = std::sync::Arc::new(Ring::new({cap}));",
    )
    .ok();
}
