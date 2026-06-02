//! openmp-rs multi-worker emit-string pins (TASK-0044.01.01 cycle 196).
//!
//! Structural-diff oracle: for any in-scope multi-worker schedule, the
//! openmp-rs `main.rs` differs from pthreads-sync's ONLY in the four
//! legal classes the substrate swap introduces.
//!
//! Class 1: `use std::thread;` (pthreads-sync only) — REMOVED in openmp-rs.
//!
//! Class 2: `rayon::scope(|s| {` wrapper around the spawn loop —
//! openmp-rs only; closing `});` on a sibling line.
//!
//! Class 3: `let WNAME_handle = thread::spawn(move || {` becomes
//! `s.spawn(move |_| {` (and closing `});` instead of `});` plus
//! `handles.push(WNAME_handle);`).
//!
//! Class 4: `handle.join().expect("worker thread panicked");` loop —
//! pthreads-sync only; absent in openmp-rs (rayon::scope's implicit
//! join at scope-end).
//!
//! Plus indentation: openmp-rs's host body lives INSIDE the rayon::scope
//! closure (one indent deeper than pthreads-sync's host body), and the
//! spawned-worker bodies are two indents deeper.
//!
//! Everything else (Slot<T> struct, slot allocations, barrier
//! allocations, pre-init declarations, the per-worker walker emit
//! through `backend_common::multi_worker_walker`, kernel calls, loop
//! headers, partition slices, reuse codegen, check-frame
//! instrumentation) MUST be byte-identical after the canonicaliser
//! removes the four legal differences. This is the cross-backend
//! differential invariant at the emit-string layer — it bites earlier
//! than the e2e bit-identical OUTPUT oracle and catches silent-sibling
//! regressions of the cycle-196 substrate swap before they hit
//! `cargo build`.
//!
//! Cycle-195b lesson: pin the load-bearing substrate needles (rayon /
//! NOT thread) BEFORE the canonicaliser runs. Otherwise a regression
//! back to thread::spawn would silently no-op the canonicaliser's
//! "remove rayon::scope wrapper" step and the test would pass while
//! the code is wrong.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above openmp-rs crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    // TASK-0426.01: per-call-unique scratch dir via the shared helper
    // (created once, never removed) — kills the remove/create race class.
    let target = repo_root().join("nucleus/target/openmp-rs-multi-worker-emit-scratch");
    test_common::unique_scratch_dir(&target, name)
}

/// Rewrite an openmp-rs multi-worker main.rs into the
/// pthreads-sync-equivalent form (the four legal differences
/// reverted). Any residual diff vs pthreads-sync's emit then surfaces
/// as a real codegen drift.
///
/// Step-by-step:
///
/// (a) Re-introduce `use std::thread;` after the `use std::sync::...`
///     line (pthreads-sync emits it as the next line).
/// (b) Strip the `    rayon::scope(|s| {` opener line; reduce indent
///     of every line ENCLOSED by the scope by 4 spaces (the rayon
///     wrapper added one indent level). Strip the closing `    });`
///     that matches the rayon::scope opener.
/// (c) Substitute `s.spawn(move |_| {` -> `thread::spawn(move || {`,
///     and rewrite the `let WNAME_var =` clones above EACH spawn
///     from `        let WNAME_var = ...;` to `    let WNAME_var =
///     ...;` (the indent shrink in step (b) handles this
///     uniformly). Then prepend `let WNAME_handle =` to the spawn
///     and append a `handles.push(WNAME_handle);` after the closing
///     `});`. Append the `handle.join()` epilogue at the end.
/// (d) Re-prepend `let mut handles: Vec<_> = Vec::new();` immediately
///     after the barrier-alloc block (the line just before the spawn
///     loop in pthreads-sync's emit).
///
/// Reality check: step (d) is a substantive insert (pthreads-sync
/// declares `let mut handles` implicitly via the writeln!ed
/// `let WNAME_handle = ...` plus `handles.push(WNAME_handle)`; the
/// actual var declaration is the first `let WNAME_handle = ...`).
/// We DON'T need step (d) — pthreads-sync uses `let WNAME_handle =`
/// (introducing a new binding per worker) and a final loop over a
/// `handles: Vec<String>` only in codegen-helper-internal state.
/// The EMIT just writes `let WNAME_handle = thread::spawn(...)` per
/// worker + a per-worker `handle.join()` line at end. So step (c)
/// and step (d) collapse into: each `s.spawn(...)` becomes
/// `let WNAME_handle = thread::spawn(...)`, and the final closing of
/// rayon::scope is replaced with N `handle.join()` lines.
///
/// Implementing this precisely as a string canonicaliser is brittle.
/// We use a slightly different strategy: scrub the rayon-specific
/// LINES out + un-indent every nested line by 4 spaces + scrub the
/// pthreads-sync-specific LINES out of the reference. Compare the
/// scrubbed forms. Less precise than a structural diff but robust to
/// minor formatting drift.
fn rayon_canonicalise(src: &str) -> String {
    let mut out = Vec::new();
    let mut inside_rayon_scope = false;
    for raw_line in src.lines() {
        let trimmed = raw_line.trim_end();
        // Strip rayon::scope wrapper. The opening is `    rayon::scope(|s| {`;
        // the closing is `    });` at indent 1. Both are openmp-rs-only.
        if trimmed == "    rayon::scope(|s| {" {
            inside_rayon_scope = true;
            continue;
        }
        if inside_rayon_scope && trimmed == "    });" {
            inside_rayon_scope = false;
            continue;
        }
        // Within rayon::scope, strip 4 leading spaces (the wrapper's
        // indent contribution). Lines that don't start with 4 spaces
        // stay verbatim (defensive — shouldn't happen, but don't
        // silently mangle).
        let line = if inside_rayon_scope && raw_line.starts_with("    ") {
            &raw_line[4..]
        } else {
            raw_line
        };
        // Substrate-swap line rewrites.
        // `s.spawn(move |_| {` -> a synthetic marker; we don't
        // reconstruct pthreads-sync's `let WNAME_handle =
        // thread::spawn(move || {` exactly (we'd need the worker
        // name); instead we collapse BOTH backends' spawn-opener
        // lines to a stable marker so the comparison still bites
        // structurally.
        let canon = if line.trim_start().starts_with("s.spawn(move |_| {") {
            // Preserve leading indent; replace body with marker.
            let lead: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            format!("{lead}<<SPAWN_OPEN>>")
        } else if line.trim_start().starts_with("let ")
            && line.contains("_handle = thread::spawn(move || {")
        {
            let lead: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            format!("{lead}<<SPAWN_OPEN>>")
        } else {
            line.to_string()
        };
        out.push(canon);
    }
    let mut result = out.join("\n");
    // Strip the pthreads-sync-only `use std::thread;` line.
    result = result.replace("use std::thread;\n", "");
    // Strip the pthreads-sync-only `handle.join().expect("worker thread
    // panicked");` lines (one per non-host worker). We only filter the
    // exact text to avoid catching any legitimate `.join()` calls
    // elsewhere (e.g. a `Vec<String>::join`).
    result = result
        .lines()
        .filter(|l| !l.contains(".join().expect(\"worker thread panicked\")"))
        .collect::<Vec<_>>()
        .join("\n");
    // Strip rayon::scope-only `});` closure-end after a spawn (the
    // matched `});` of `s.spawn(move |_| {`). pthreads-sync emits
    // `});` for the matched `let WNAME_handle = thread::spawn(move
    // || {` opener too — but at a different indent. After the indent
    // collapse in step (b) both should be at indent 1 (`    });`).
    // We keep the closing `});` as a structural marker (`<<SPAWN_CLOSE>>`)
    // to ensure both backends emit the same number of them.
    // Actually skip this — the matched closures `});` line on
    // pthreads-sync side is already preserved verbatim. The
    // rayon-collapsed `});` lines are at the same indent. They
    // should match line-for-line.
    if src.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Pin the load-bearing substrate needles BEFORE the canonicaliser
/// runs (cycle-195b oracle lesson). If openmp-rs regressed to emit
/// `thread::spawn`, the rayon-stripping canonicaliser would silently
/// no-op and the comparison vs pthreads-sync would be a vacuous PASS.
fn assert_openmp_uses_rayon_substrate(label: &str, openmp_src: &str, sync_src: &str) {
    // openmp-rs side: MUST carry rayon::scope + s.spawn.
    assert!(
        openmp_src.contains("rayon::scope(|s| {"),
        "{label}: openmp-rs main.rs MUST carry rayon::scope (cycle-196 substrate)"
    );
    assert!(
        openmp_src.contains("s.spawn(move |_| {"),
        "{label}: openmp-rs main.rs MUST carry s.spawn(move |_| ...) (cycle-196 substrate)"
    );
    // openmp-rs side: MUST NOT carry pthreads-sync substrate.
    assert!(
        !openmp_src.contains("use std::thread"),
        "{label}: openmp-rs MUST NOT import std::thread"
    );
    assert!(
        !openmp_src.contains("thread::spawn(move ||"),
        "{label}: openmp-rs MUST NOT call thread::spawn"
    );
    assert!(
        !openmp_src.contains(".join().expect(\"worker thread panicked\")"),
        "{label}: openmp-rs MUST NOT carry pthreads-sync's join loop"
    );
    // pthreads-sync side: MUST carry its substrate (oracle invariant
    // preservation). A pthreads-sync regression would invalidate the
    // canonicaliser's assumption that the swap is real.
    assert!(
        sync_src.contains("use std::thread;"),
        "{label}: pthreads-sync main.rs MUST carry use std::thread; (oracle precondition)"
    );
    assert!(
        sync_src.contains("thread::spawn(move ||"),
        "{label}: pthreads-sync main.rs MUST carry thread::spawn (oracle precondition)"
    );
    assert!(
        !sync_src.contains("rayon::scope"),
        "{label}: pthreads-sync MUST NOT carry rayon::scope (oracle precondition)"
    );
}

fn assert_openmp_main_equiv_sync(label: &str, openmp_src: &str, sync_src: &str) {
    assert_openmp_uses_rayon_substrate(label, openmp_src, sync_src);

    let openmp_canon = rayon_canonicalise(openmp_src);
    // Apply the same SPAWN_OPEN marker normalisation to pthreads-sync.
    let sync_canon_lines: Vec<String> = sync_src
        .lines()
        .filter(|l| !l.contains("use std::thread;"))
        .map(|line| {
            if line.trim_start().starts_with("let ")
                && line.contains("_handle = thread::spawn(move || {")
            {
                let lead: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                format!("{lead}<<SPAWN_OPEN>>")
            } else if line.contains(".join().expect(\"worker thread panicked\")") {
                // Drop pthreads-sync's join loop (rayon::scope has implicit join).
                return String::new();
            } else {
                line.to_string()
            }
        })
        .collect();
    let mut sync_canon = sync_canon_lines.join("\n");
    if sync_src.ends_with('\n') && !sync_canon.ends_with('\n') {
        sync_canon.push('\n');
    }

    // Drop ALL blank lines from BOTH sides. The join-loop strip on
    // the pthreads-sync side leaves a blank line that pthreads-sync
    // emitted as the join-loop's leading separator; openmp-rs has no
    // such separator. Rather than surgical-strip-just-that-one (which
    // would be fragile to emit-order changes), apply the rule uniformly
    // — collapse runs of ≥2 newlines to exactly 1. Paragraph separators
    // in the host body collapse on BOTH sides equally, so structural
    // comparison still bites; the load-bearing assertion is the
    // substantive code, not the blank-line structure.
    fn collapse_blanks(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut nl_count = 0;
        for c in s.chars() {
            if c == '\n' {
                nl_count += 1;
                if nl_count <= 1 {
                    out.push(c);
                }
            } else {
                nl_count = 0;
                out.push(c);
            }
        }
        out
    }
    let openmp_canon = collapse_blanks(&openmp_canon);
    let sync_canon = collapse_blanks(&sync_canon);

    if openmp_canon != sync_canon {
        let ol: Vec<&str> = openmp_canon.lines().collect();
        let sl: Vec<&str> = sync_canon.lines().collect();
        let mut diff = String::new();
        for (i, (a, b)) in ol.iter().zip(sl.iter()).enumerate() {
            if a != b {
                diff.push_str(&format!(
                    "line {}:\n  openmp(canon): {a:?}\n  sync(canon):   {b:?}\n",
                    i + 1
                ));
                if diff.len() > 4096 {
                    break;
                }
            }
        }
        if ol.len() != sl.len() {
            diff.push_str(&format!("\nlength: openmp={} sync={}", ol.len(), sl.len()));
        }
        panic!(
            "{label}: openmp-rs main.rs (after rayon-canonicalisation) differs from \
             pthreads-sync's. This means the cycle-196 substrate swap missed a \
             call site OR a non-substrate codegen drift slipped into openmp-rs's \
             multi_worker.rs.\n--- divergences:\n{diff}\n\
             --- openmp (canon, {olen} lines, head 4 KB) ---\n{ohead}\n\
             --- sync (canon, {slen} lines, head 4 KB) ---\n{shead}\n",
            olen = ol.len(),
            slen = sl.len(),
            ohead = &openmp_canon[..openmp_canon.len().min(4096)],
            shead = &sync_canon[..sync_canon.len().min(4096)],
        );
    }
}

/// 02-split-add/split — simplest in-tree multi-worker schedule. Two
/// used workers (host + w0); minimal codegen surface; if the
/// substrate swap missed any spawn site, this fails first.
#[test]
fn split_02_openmp_equiv_pthreads_sync() {
    let scratch = scratch_dir("split_02");
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).unwrap();
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );
    let kernels = ex.join("kernels.rs");

    let openmp_out = scratch.join("openmp");
    let sync_out = scratch.join("sync");
    let openmp = openmp_rs::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &openmp_out)
        .expect("openmp-rs emit");
    let sync = pthreads_sync::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &sync_out)
        .expect("pthreads-sync emit");

    let openmp_src = std::fs::read_to_string(&openmp.main_rs).expect("read openmp main.rs");
    let sync_src = std::fs::read_to_string(&sync.main_rs).expect("read sync main.rs");
    assert_openmp_main_equiv_sync("02-split-add/split", &openmp_src, &sync_src);
}

/// 06-separable-filter/distributed2 — host + 4 workers, exercises a
/// wider spawn loop + partition_workers + halo + reuse. Pass-sequence
/// MIRRORS bufsync's `tests/host_relay_emit.rs::emit_06_distributed2`.
#[test]
fn separable_filter_06_distributed2_openmp_equiv_pthreads_sync() {
    use nucleus_compiler::{
        acfg_to_events,
        algo::{lower_algo, parse_algo},
        apply_block_transforms, apply_halo_inference_partition_aware, apply_partition_blocks2d,
        apply_partition_rows, apply_partition_workers, apply_reuse_inference, build_acfg,
        build_sidecar, inject_syncs, inject_transfers, link,
        sched::{lower_sched, parse_sched},
        NameTables,
    };

    let scratch = scratch_dir("separable_filter_06_distributed2");
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/06-separable-filter");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = std::fs::read_to_string(ex.join("schedules/distributed2.sched.nuc")).unwrap();
    let kernels = ex.join("kernels.rs");

    let algo_ir = lower_algo(&parse_algo(&algo_src).expect("parse")).expect("lower");
    let sched_ir = lower_sched(&parse_sched(&sched_src).expect("parse")).expect("lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block transforms");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");
    let acfg = apply_partition_rows(&linked, acfg).expect("partition_rows");
    let acfg = apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d");
    let (acfg, _advisory) =
        apply_halo_inference_partition_aware(&linked, acfg).expect("halo inference");
    let acfg = apply_reuse_inference(&linked, acfg).expect("reuse inference");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);

    let openmp_out = scratch.join("openmp");
    let sync_out = scratch.join("sync");
    let openmp = openmp_rs::emit(&per_worker, &names, &sidecar, &kernels, &openmp_out)
        .expect("openmp-rs emit");
    let sync = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels, &sync_out)
        .expect("pthreads-sync emit");

    let openmp_src = std::fs::read_to_string(&openmp.main_rs).expect("read openmp main.rs");
    let sync_src = std::fs::read_to_string(&sync.main_rs).expect("read sync main.rs");
    assert_openmp_main_equiv_sync("06-separable-filter/distributed2", &openmp_src, &sync_src);
}

/// 15-transpose/distributed-rows — the host-EXCLUDING-barrier coverage
/// arm (TASK-0044.01.03). The two pre-existing fixtures above
/// (02-split-add/split, 06-separable-filter/distributed2) both emit
/// barriers that INCLUDE host: every barrier-participant union there is
/// driven by a host-touching transfer boundary. This fixture closes the
/// hole by pinning a schedule whose `xpose on {w0,w1,w2,w3}` placement
/// produces a genuinely host-EXCLUDING inner barrier
/// (`Arc::new(Barrier::new(4))`, participants `{w0,w1,w2,w3}`), the
/// barrier-participant-count path the two existing fixtures never reach.
///
/// PROVENANCE / PREMISE CORRECTION (TASK-0044.01.03 cycle 231): the
/// filed task named `03-reduction/distributed` as the host-excluding
/// shape. That premise was empirically false — 03-reduction/distributed
/// emits only `Arc::new(Barrier::new(5))` barriers (all four phase
/// boundaries cross host, which owns load_input / combine), so
/// `Arc::new(Barrier::new(4))` never appears and the original AC#2 was
/// unsatisfiable. A scan of every multi-worker schedule found
/// 15-transpose/distributed-rows is the ONLY schedule that is BOTH
/// genuinely host-excluding AND a promoted `[[required]]` e2e cell on
/// openmp-rs (see `nuc-nucleus/e2e-matrix.toml`). Retargeting here
/// fulfils the coverage intent and is strictly stronger: this cell
/// exercises BOTH the host-included `new(5)` shape AND the host-excluded
/// `new(4)` shape in one emit.
///
/// Lowering: `lower_for_test` with `apply_partition_workers = true`.
/// 15-transpose/distributed-rows uses only `partition=workers` (on the
/// inner `j` loop); it has no `block=`, no `partition=rows/blocks2d`, no
/// halo, no reuse, so the passes `lower_for_test` omits are all no-ops
/// for this schedule — verified empirically (the openmp-rs/pthreads-sync
/// emit via this shorter pipeline is byte-identical to the full-driver
/// emit's barrier set).
#[test]
fn transpose_15_distributed_rows_openmp_equiv_pthreads_sync() {
    let scratch = scratch_dir("transpose_15_distributed_rows");
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/15-transpose");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/distributed-rows.sched.nuc")).unwrap();
    // `partition=workers` is the load-bearing toggle: without it the
    // outer `j` loop is not partitioned across w0..w3 and no
    // host-excluding compute barrier is produced. `apply_block_transforms`
    // stays at its default `true` (no-op here — no `block=` directive).
    let opts = test_common::LowerForTestOpts {
        apply_partition_workers: true,
        ..test_common::LowerForTestOpts::default()
    };
    let r = test_common::lower_for_test(&algo_src, &sched_src, &opts);
    let kernels = ex.join("kernels.rs");

    let openmp_out = scratch.join("openmp");
    let sync_out = scratch.join("sync");
    let openmp = openmp_rs::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &openmp_out)
        .expect("openmp-rs emit");
    let sync = pthreads_sync::emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &sync_out)
        .expect("pthreads-sync emit");

    let openmp_src = std::fs::read_to_string(&openmp.main_rs).expect("read openmp main.rs");
    let sync_src = std::fs::read_to_string(&sync.main_rs).expect("read sync main.rs");

    // AC#2 — the host-EXCLUDING barrier shape. The inner `xpose` barrier
    // synchronises only the four compute workers; openmp-rs (like
    // pthreads-sync) emits it directly as `Arc::new(Barrier::new(4))`
    // because openmp-rs is shared-memory and does NOT apply
    // host-mediation. A regression to `new(5)` (off-by-one mediating
    // host into a shared-memory barrier) or `new(3)` would fail HERE,
    // before the byte-equivalence check, and before the slow e2e oracle.
    assert!(
        openmp_src.contains("Arc::new(Barrier::new(4)); // participants: {w0,w1,w2,w3}"),
        "15-transpose/distributed-rows: openmp-rs main.rs MUST carry the \
         host-EXCLUDING inner barrier `Arc::new(Barrier::new(4))` over \
         {{w0,w1,w2,w3}} (NOT new(5)); its absence means the \
         barrier-participant-count path regressed. main.rs:\n{openmp_src}"
    );
    // Negative: the off-by-one host-mediated count must NOT appear for a
    // barrier whose participant comment is the host-excluding set. (A
    // host-INCLUDED new(5) barrier legitimately appears elsewhere in
    // this same emit — that is asserted positively below — so we anchor
    // the anti-needle to the host-excluding participant comment.)
    assert!(
        !openmp_src.contains("Arc::new(Barrier::new(5)); // participants: {w0,w1,w2,w3}"),
        "15-transpose/distributed-rows: a barrier over {{w0,w1,w2,w3}} must \
         have count 4, not 5 — host must NOT be mediated into a \
         shared-memory openmp-rs barrier. main.rs:\n{openmp_src}"
    );
    // This cell ALSO exercises the host-INCLUDED shape (the phase-
    // boundary barriers that cross host on the load/gather transfers),
    // so both barrier-participant-count paths are covered in one fixture.
    assert!(
        openmp_src.contains("Arc::new(Barrier::new(5)); // participants: {host,w0,w1,w2,w3}"),
        "15-transpose/distributed-rows: openmp-rs main.rs MUST also carry \
         a host-INCLUDED `Arc::new(Barrier::new(5))` over \
         {{host,w0,w1,w2,w3}} (the load/gather phase boundaries) — both \
         barrier shapes are exercised by this cell. main.rs:\n{openmp_src}"
    );

    assert_openmp_main_equiv_sync("15-transpose/distributed-rows", &openmp_src, &sync_src);
}

/// Cycle-193 forward-carried const-in-IndexExpr regression pin
/// (mirrors `pthreads-async/tests/skeleton.rs::
/// const_in_indexexpr_pthreads_async_resolves_to_literal_value`).
///
/// Why this pin is independent of the byte-equivalence-vs-pthreads-sync
/// tests: a future regression of `pthreads_sync::render_const_expr_pub`
/// would cause BOTH backends to emit the broken form (since openmp-rs's
/// multi-worker arm consumes the SAME backend_common renderer that
/// flows through that pub shim). The byte-equivalence test would
/// happily PASS while the emitted code fails to build (`ITERS` is not
/// in scope in the emitted main.rs). This independent literal-needle
/// + bare-ident-anti-needle pin BITES.
#[test]
fn const_in_indexexpr_openmp_rs_resolves_to_literal_value() {
    let r = test_common::lower_for_test(
        test_common::CONST_IN_INDEXEXPR_ALGO_SRC,
        test_common::CONST_IN_INDEXEXPR_SCHED_SRC,
        &test_common::LowerForTestOpts::default(),
    );

    let scratch = scratch_dir("const_in_indexexpr_openmp_rs");
    let kernels = scratch.join("kernels.rs");
    std::fs::write(&kernels, "// stub for emit-string test\n").unwrap();

    let result = openmp_rs::emit(
        &r.per_worker,
        &r.names,
        &r.sidecar,
        &kernels,
        &scratch.join("gen"),
    )
    .expect("openmp-rs emit must succeed on const-in-IndexExpr fixture");
    let main_rs = std::fs::read_to_string(&result.main_rs).expect("read main.rs");

    let iters_val = test_common::CONST_IN_INDEXEXPR_ITERS_VALUE;
    let resolved_row = format!("({iters_val}) * 4");
    let bare_ident = test_common::CONST_IN_INDEXEXPR_ITERS_IDENT;

    // (1) The resolved literal `8` appears at the IndexExpr site.
    // pthreads-async uses the same row-stride formula. openmp-rs
    // consumes the same backend_common renderer so the literal must
    // render the same way.
    assert!(
        main_rs.contains(&resolved_row),
        "openmp-rs multi-worker main.rs must contain the resolved `ITERS=8` literal \
         at the IndexExpr row-stride site (`{resolved_row}`); cycle-35 fix not \
         reaching this code path via backend_common::multi_worker_walker. main.rs:\n{main_rs}"
    );

    // (2) The bare const ident `ITERS` does NOT appear anywhere in
    // the emitted main.rs.
    assert!(
        !main_rs.contains(bare_ident),
        "openmp-rs multi-worker main.rs must NOT contain the bare const ident \
         `{bare_ident}` — its presence means `render_int_expr` failed to \
         resolve to the sidecar const value, and the generated main.rs would \
         fail to compile. main.rs:\n{main_rs}"
    );
}
