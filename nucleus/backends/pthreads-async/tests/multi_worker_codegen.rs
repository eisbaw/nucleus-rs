//! Codegen-string pins for the pthreads-async multi-worker runtime
//! substrate (TASK-0228 Wave A, cycle 18).
//!
//! These tests pin the emit-string shape of the Wave A pure-function
//! helpers — `emit_ring_struct_decl` and `emit_ring_instance_decl` —
//! BEFORE the Wave B integration (the per-worker `thread::spawn`
//! dispatch) consumes them. The split is deliberate: Wave A's
//! correctness is settle-able against unit tests; Wave B's
//! correctness needs an integration test (build + run a real
//! pipelined fixture). Splitting Wave A lets a drift in the
//! substrate shape (renamed `Ring` field, removed `cap` arg,
//! different condvar pair) surface against a focused test, not
//! buried inside a Wave B integration commit.
//!
//! The assertions intentionally use substring containment + tuple
//! presence rather than full-string match: they pin SHAPE
//! invariants (the right fields exist; the right Mutex/Condvar
//! types are used; the cap is stored as `usize`; push/wait have the
//! documented blocking semantics) without over-pinning the exact
//! comment text. A future cycle that re-flows the docstring should
//! NOT break these tests; a cycle that changes the struct shape
//! SHOULD.

use pthreads_async::{emit_ring_instance_decl, emit_ring_struct_decl};

#[test]
fn ring_struct_decl_pins_documented_shape() {
    let mut out = String::new();
    emit_ring_struct_decl(&mut out);

    // Struct shape: four fields, no generic capacity (cap is `usize`
    // stored in the instance — per design, one struct definition
    // serves rings of every size).
    assert!(
        out.contains("struct Ring<T> {"),
        "Ring struct definition must use generic <T>:\n{out}"
    );
    assert!(
        out.contains("mu: std::sync::Mutex<std::collections::VecDeque<T>>"),
        "Ring.mu must be std::sync::Mutex<std::collections::VecDeque<T>> \
         (any other shape changes the producer/consumer contract):\n{out}"
    );
    assert!(
        out.contains("cap: usize"),
        "Ring.cap must be `usize` (the per-instance capacity slot — \
         baked into the instance, not the type):\n{out}"
    );
    assert!(
        out.contains("not_empty: std::sync::Condvar"),
        "Ring.not_empty must be std::sync::Condvar (consumer-side wake):\n{out}"
    );
    assert!(
        out.contains("not_full: std::sync::Condvar"),
        "Ring.not_full must be std::sync::Condvar (producer-side wake):\n{out}"
    );

    // Constructor: cap is the only argument; ring starts EMPTY (no
    // pre-fill regardless of pipeline depth D — see TASK-0042.01
    // post-TASK-0213 contract).
    assert!(
        out.contains("fn new(cap: usize) -> Self"),
        "Ring::new must take ONLY `cap: usize` — pre-fill arguments \
         would violate the ring-EMPTY-at-start contract:\n{out}"
    );
    assert!(
        out.contains("VecDeque::with_capacity(cap)"),
        "VecDeque should be pre-allocated to `cap` (no resize cost \
         under steady state) but starts EMPTY (no fill):\n{out}"
    );
    assert!(
        !out.contains("push_back") || out.contains("g.push_back(v)"),
        "Push uses VecDeque::push_back (FIFO order):\n{out}"
    );

    // Push semantics: lock, while-block on not_full, push_back, notify.
    assert!(
        out.contains("fn push(&self, v: T)"),
        "push must take owned `v: T` (transfer of ownership):\n{out}"
    );
    assert!(
        out.contains("while g.len() == self.cap"),
        "push must block on a `while` (spurious-wakeup-safe), not `if`, \
         while the ring is at capacity:\n{out}"
    );
    assert!(
        out.contains("self.not_full.wait(g)"),
        "push blocks on not_full when full:\n{out}"
    );
    assert!(
        out.contains("self.not_empty.notify_one()"),
        "push notifies not_empty when it succeeds:\n{out}"
    );

    // Wait semantics: lock, while-block on not_empty, pop_front, notify.
    assert!(
        out.contains("fn wait(&self) -> T"),
        "wait must return owned `T` (transfer of ownership):\n{out}"
    );
    assert!(
        out.contains("while g.is_empty()"),
        "wait must block on a `while` (spurious-wakeup-safe), not `if`, \
         while the ring is empty:\n{out}"
    );
    assert!(
        out.contains("self.not_empty.wait(g)"),
        "wait blocks on not_empty when empty:\n{out}"
    );
    assert!(
        out.contains("g.pop_front()"),
        "wait uses VecDeque::pop_front (FIFO order — consumer sees \
         producer-side push order):\n{out}"
    );
    assert!(
        out.contains("self.not_full.notify_one()"),
        "wait notifies not_full when it succeeds:\n{out}"
    );
}

#[test]
fn ring_instance_decl_pins_arc_ring_shape() {
    let mut out = String::new();
    emit_ring_instance_decl(&mut out, "ring_d1_s2", "Vec<f32>", 4);

    // The instance is an Arc so the producer + consumer threads each
    // hold their own handle; Ring::new(4) sets the capacity to the
    // schedule's `transfer DATA : buffer=4` directive.
    let expected = "    let ring_d1_s2: std::sync::Arc<Ring<Vec<f32>>> = std::sync::Arc::new(Ring::new(4));\n";
    assert_eq!(
        out, expected,
        "ring instance decl must match the documented shape exactly:\n\
         got: {out:?}\nwanted: {expected:?}"
    );
}

#[test]
fn ring_instance_decl_handles_scalar_element_type() {
    let mut out = String::new();
    emit_ring_instance_decl(&mut out, "ring_d3_s5", "f32", 1);
    // Scalar transfer (e.g. `transfer x : async, buffer=1`, x: f32) —
    // smallest legal cap is 1 (`max_buffer=64` per capabilities.toml is
    // upper bound; lower bound is 1 by the link-step `PipelineExceedsBuffer`
    // check which rejects D > N).
    assert_eq!(
        out,
        "    let ring_d3_s5: std::sync::Arc<Ring<f32>> = std::sync::Arc::new(Ring::new(1));\n"
    );
}

#[test]
fn ring_struct_decl_does_not_pre_fill_with_d() {
    // CRITICAL post-TASK-0213 contract: D (pipeline depth) does NOT
    // appear in the runtime code. The ring is sized by `cap = N`
    // (buffer=N from the transfer directive); D appears only in the
    // IR's analysis encoding (acfg_to_petri's TtoP-arc elision).
    let mut out = String::new();
    emit_ring_struct_decl(&mut out);

    // `pipeline_depth` / `D` / `pre_fill` / `initial_marking` must
    // NOT appear in the runtime code — any of them would be a defect
    // (the post-TASK-0213 ring-EMPTY contract).
    let forbidden = ["pipeline_depth", "pre_fill", "prefill", "initial_marking"];
    for word in forbidden {
        assert!(
            !out.contains(word),
            "Ring struct emission contains `{word}` — that would be a \
             runtime pre-fill defect (the ring MUST start empty; D is \
             analysis-only). Output:\n{out}"
        );
    }

    // The struct-level docstring should mention the EMPTY-at-start
    // contract for future readers; if a future cycle removes the
    // docstring, this test catches it (but tolerates rewording).
    assert!(
        out.contains("EMPTY") || out.contains("empty"),
        "Ring docstring should mention the empty-at-start contract \
         (post-TASK-0213); a future reader needs that signal:\n{out}"
    );
}
