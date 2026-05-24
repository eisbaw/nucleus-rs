//! Codegen-string pins for the pthreads-async multi-worker runtime
//! substrate (TASK-0228 Wave A, cycle 18).
//!
//! **File-naming note (cycle-18 review-gate D.4 finding)**: this file
//! is `multi_worker_codegen.rs` but Wave A covers ONLY the `Ring<T>`
//! runtime substrate, not yet the actual multi-worker dispatch
//! codegen. Wave B (the per-worker `thread::spawn` Plan structure
//! mirroring `pthreads_sync::multi_worker::Plan`, plus push/wait
//! dispatch pairs emitting `ring_<id>.push(...)` / `let v =
//! ring_<id>.wait();`) will land in THIS same file. The aspirational
//! filename is intentional — keeps Wave A + Wave B tests co-located.
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

use pthreads_async::{emit, emit_ring_instance_decl, emit_ring_struct_decl};

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
    let expected =
        "    let ring_d1_s2: std::sync::Arc<Ring<Vec<f32>>> = std::sync::Arc::new(Ring::new(4));\n";
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

#[test]
fn ring_struct_decl_negative_checks_pin_design_decisions() {
    // Cycle-18 review-gate C.2 finding: additional negative checks
    // on documented design decisions that the positive-side tests do
    // not lock down.
    let mut out = String::new();
    emit_ring_struct_decl(&mut out);

    // notify_one (not notify_all) is documented as correct given SeqTag
    // uniqueness (each (DataId, SeqTag) ring has exactly ONE producer
    // + ONE consumer; the per-fan-out-pair sizing of TASK-0216 means a
    // fan-out splits into N separate rings, not one shared ring with N
    // consumers). A future cycle "fixing" perceived starvation by
    // switching to notify_all would still be runtime-correct but
    // scheduler-wasteful — and indicates a misunderstanding of the
    // analysis-net structure. Catch it.
    assert!(
        !out.contains("notify_all"),
        "Ring push/wait must use notify_one (per-(DataId,SeqTag) ring is \
         SPSC under TASK-0216 fan-out sizing — notify_all is unnecessary \
         and indicates the implementer misunderstood the structure):\n{out}"
    );

    // Spurious-wakeup safety: both blocks must use `while`, not `if`.
    // The positive-side test asserts `while g.len() == self.cap` and
    // `while g.is_empty()` appear; this negative-side test catches an
    // adjacent `if g.len() == self.cap` (which would be a silent
    // spurious-wakeup defect).
    assert!(
        !out.contains("if g.len() == self.cap"),
        "Ring push must use `while`, not `if`, for spurious-wakeup safety \
         (Condvar wakes are documented as possibly-spurious; `if` would \
         drop a value to an over-cap ring):\n{out}"
    );
    assert!(
        !out.contains("if g.is_empty()"),
        "Ring wait must use `while`, not `if`, for spurious-wakeup safety:\n{out}"
    );

    // Defensive: the runtime must never silently fall back to a
    // 0-capacity ring (deadlock both push and wait). The upstream
    // ZeroBufferOption + PipelineExceedsBuffer gates prevent cap=0
    // from ever reaching codegen — verify the emit string doesn't
    // contain a sneaky `cap = 0` fallback that would defeat them.
    assert!(
        !out.contains("with_capacity(0)"),
        "Ring must never pre-allocate VecDeque::with_capacity(0) — the \
         schedule's buffer=N is `N >= 1` (sched/ir.rs:ZeroBufferOption) \
         and `N >= D` (link.rs:PipelineExceedsBuffer); a 0-cap fallback \
         would defeat both gates:\n{out}"
    );
}

#[test]
fn ring_struct_decl_has_exactly_four_fields() {
    // Cycle-18 review-gate C.1 finding: the four positive-side field
    // checks tolerate ADDITION of fields — a future cycle adding e.g.
    // `n_in_flight: AtomicUsize` for instrumentation would pass every
    // other test but would change the struct shape without intent
    // visible in the diff.
    //
    // Pin the field count explicitly. If a 5th field is needed for a
    // legitimate reason, this test must be updated in the same diff
    // that adds the field — making the design decision explicit.
    let mut out = String::new();
    emit_ring_struct_decl(&mut out);

    // Count lines inside the `struct Ring<T> { ... }` block. Use
    // simple state-machine: enter on the struct line, exit on the
    // closing `}`.
    let mut field_count = 0;
    let mut inside = false;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed == "struct Ring<T> {" {
            inside = true;
            continue;
        }
        if inside && trimmed == "}" {
            break;
        }
        if inside && !trimmed.is_empty() && !trimmed.starts_with("//") {
            // A field line — type-annotated, comma-terminated.
            field_count += 1;
        }
    }
    assert_eq!(
        field_count, 4,
        "Ring<T> must have EXACTLY 4 fields (mu, cap, not_empty, not_full); \
         adding a 5th is a design change that should update this test in \
         lockstep. Counted {field_count} fields in:\n{out}"
    );
}

// --------------------------------------------------------------------
// Wave B-2 (cycle 26, TASK-0228) — emit-string pins for the
// MULTI-WORKER emit path (Plan::emit + render_main_rs_multi). The
// substrate tests above cover the helpers; these tests pin the
// integrated emit through `pthreads_async::emit` on a real fixture
// (02-split-add/split — 2 workers, sync transfers, no partition).
//
// File-scope shape (cycle 26 baseline):
// - One Ring<T> struct definition (the helper is called once).
// - Three Arc<Ring<T>> instances sized cap=1 (02-split's 3 transfers).
// - Three Barriers (inject_syncs produces 3 cross-worker barriers).
// - One spawned non-host worker (w0).
// - One ring_<id>.push(...) and one ring_<id>.wait() at least.
// --------------------------------------------------------------------

fn repo_root() -> std::path::PathBuf {
    let here = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above pthreads-async crate")
        .to_path_buf()
}

/// Emit 02-split-add/split through `pthreads_async::emit` and return
/// the main.rs body. Each caller passes its OWN `scratch_name` (the test
/// function name) so the scratch dirs do NOT collide under cargo's
/// parallel test runner — a single shared scratch was the cycle-26
/// race-flake source (`remove_dir_all` of test A racing the emit-then-
/// read of test B). TASK-0241.
fn emit_02_split_main_rs(scratch_name: &str) -> String {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("02 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("02 split sched");
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );
    let scratch = root.join(format!(
        "nucleus/target/pthreads-async-test-scratch/wave_b2_codegen_pins_{scratch_name}"
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    let result = emit(
        &r.per_worker,
        &r.names,
        &r.sidecar,
        &ex.join("kernels.rs"),
        &scratch,
    )
    .expect("Wave B-2 emit must succeed for 02-split-add/split");
    std::fs::read_to_string(&result.main_rs).expect("read main.rs")
}

#[test]
fn wave_b2_multi_emit_contains_ring_struct_exactly_once() {
    // The file-scope Ring<T> definition must appear EXACTLY once
    // (the helper is documented as "idempotent at the caller layer";
    // emitting twice would compile but is a codegen defect — a future
    // refactor that accidentally calls the helper inside the per-pair
    // loop would multiply the struct, this test catches it).
    let main_rs = emit_02_split_main_rs("contains_ring_struct_exactly_once");
    let count = main_rs.matches("struct Ring<T> {").count();
    assert_eq!(
        count, 1,
        "Ring<T> struct definition must appear exactly ONCE in the \
         emitted main.rs; got {count}. main.rs:\n{main_rs}"
    );
}

#[test]
fn wave_b2_multi_emit_ring_alloc_uses_cap_from_sidecar() {
    // 02-split-add/split has 3 cross-worker transfers, all default
    // buffer=1 (sync). The emit must allocate 3 rings each sized 1.
    let main_rs = emit_02_split_main_rs("ring_alloc_uses_cap_from_sidecar");
    for rid in 0..3 {
        let needle = format!(
            "let ring_{rid}: std::sync::Arc<Ring<Vec<i32>>> = \
             std::sync::Arc::new(Ring::new(1));"
        );
        assert!(
            main_rs.contains(&needle),
            "ring_{rid} allocation must be cap=1 (the 02-split-add/split \
             transfers have default buffer=1). Expected:\n  {needle}\n\
             in main.rs:\n{main_rs}"
        );
    }
    // And no 4th ring (regression catch if the Plan over-allocates).
    assert!(
        !main_rs.contains("let ring_3:"),
        "02-split-add/split must produce EXACTLY 3 rings (ring_0,1,2); \
         a 4th would mean the Plan ring_id counter drifted:\n{main_rs}"
    );
}

#[test]
fn wave_b2_multi_emit_push_wait_pair_uses_ring_prefix() {
    // Pin the Push/Wait emit-string shape: ring_<id>.push(name.clone());
    // and `name = ring_<id>.wait();`. The send/recv comment names the
    // peer worker — drift-detection on the format string.
    let main_rs = emit_02_split_main_rs("push_wait_pair_uses_ring_prefix");
    // Producer side (host pushes input arrays to w0; w0 pushes result
    // back to host). At minimum one push from host and one from w0.
    assert!(
        main_rs.contains(".push(a.clone()); // send `a` to "),
        "host-side push of `a` must use ring_<id>.push(a.clone()) with \
         the documented send-comment shape:\n{main_rs}"
    );
    assert!(
        main_rs.contains(".push(c.clone()); // send `c` to host"),
        "w0-side push of `c` must use ring_<id>.push(c.clone()) targeting \
         host:\n{main_rs}"
    );
    // Consumer side: w0 waits on a, b; host waits on c.
    assert!(
        main_rs.contains("w0_ring_0.wait(); // recv `a` from "),
        "w0-side wait on `a` must use w0_ring_0.wait() with recv-comment \
         shape:\n{main_rs}"
    );
    assert!(
        main_rs.contains("c = ring_2.wait(); // recv `c` from w0"),
        "host-side wait on `c` must use bare `ring_2.wait()` (host has \
         no closure prefix):\n{main_rs}"
    );
}

#[test]
fn wave_b2_multi_emit_barriers_match_sync_tags() {
    // 02-split-add/split has 3 distinct cross-worker barriers; each
    // emits one `let bar_<tag>: Arc<Barrier> = Arc::new(Barrier::new(N));`
    // with N = participant count. Both workers ({host,w0}) participate
    // in every barrier here, so every Barrier::new(2).
    let main_rs = emit_02_split_main_rs("barriers_match_sync_tags");
    let bar_count = main_rs
        .matches(": Arc<Barrier> = Arc::new(Barrier::new(2));")
        .count();
    assert!(
        bar_count >= 3,
        "02-split-add/split must allocate ≥3 2-party Barriers (one per \
         distinct SyncTag); got {bar_count}. main.rs:\n{main_rs}"
    );
}

#[test]
fn wave_b2_multi_emit_spawns_non_host_workers_with_arc_clones() {
    // For every non-host worker, the emit must:
    // - clone every ring the worker touches into `<wname>_ring_<id>`,
    // - clone every barrier the worker participates in into `<wname>_bar_<tag>`,
    // - spawn the closure with `let <wname>_handle = thread::spawn(move || {`,
    // - join with `<wname>_handle.join().expect("worker thread panicked");`.
    let main_rs = emit_02_split_main_rs("spawns_non_host_workers_with_arc_clones");
    assert!(
        main_rs.contains("let w0_ring_0 = Arc::clone(&ring_0);"),
        "w0 must Arc-clone the rings it touches into closure-locals:\n{main_rs}"
    );
    assert!(
        main_rs.contains("let w0_bar_0 = Arc::clone(&bar_0);"),
        "w0 must Arc-clone every barrier it participates in:\n{main_rs}"
    );
    assert!(
        main_rs.contains("let w0_handle = thread::spawn(move || {"),
        "w0 must be spawned as `let w0_handle = thread::spawn(move || {{`:\n{main_rs}"
    );
    assert!(
        main_rs.contains("w0_handle.join().expect(\"worker thread panicked\");"),
        "host must join w0 with the documented panic-propagating .expect:\n{main_rs}"
    );
}

#[test]
fn wave_b2_multi_emit_compiles() {
    // The strongest pin: not "does the emit string contain X" but
    // "does cargo accept the emitted project". A future cycle that
    // introduces a syntax error in render_worker_events (e.g. unbalanced
    // braces in a Loop body, a missing semicolon after a Fire call)
    // would silently pass the substring tests above but FAIL here.
    //
    // Uses a DEDICATED scratch dir (not the shared `wave_b2_codegen_pins`
    // path the substring pins use) so a parallel test that wipes the
    // shared dir doesn't race the cargo invocation here. Slow-ish
    // (~5s for a clean build, ~0.5s incremental).
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("02 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("02 split sched");
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );
    let scratch = root.join("nucleus/target/pthreads-async-test-scratch/wave_b2_compile_check");
    let _ = std::fs::remove_dir_all(&scratch);
    let _ = emit(
        &r.per_worker,
        &r.names,
        &r.sidecar,
        &ex.join("kernels.rs"),
        &scratch,
    )
    .expect("emit must succeed before the cargo compile check");

    let status = std::process::Command::new("cargo")
        .args(["build", "--quiet", "--offline"])
        .current_dir(&scratch)
        .status();
    let Ok(status) = status else {
        // Cargo invocation failed entirely (e.g. cargo not on PATH).
        // Skip rather than spuriously fail — CI runs tests under
        // `nix develop -c just test` so cargo IS on PATH there.
        return;
    };
    assert!(
        status.success(),
        "Wave B-2 emitted main.rs must compile under `cargo build`. \
         A compile failure here means the render_worker_events walk \
         produced invalid Rust (unbalanced braces, missing semicolons, \
         type mismatch). Scratch project: {}",
        scratch.display(),
    );
}
