//! Unit tests for the pthreads-sync backend `emit(...)` function.
//!
//! These exercise the renderer in isolation: parse + lower + link
//! example 01, build the ACFG, run sync + transfer injection, then
//! call `emit(...)` into a tempdir. We assert:
//!
//! - Every expected file appears.
//! - Cargo.toml is a parseable standalone manifest.
//! - main.rs mentions the expected kernel names (a smoke test that
//!   the dataflow walk actually emitted calls).
//!
//! The actual bit-identical end-to-end test (compile + run + diff
//! reference.bin) lives at
//! `nucleus/nucleus-compiler/tests/e2e_example_01.rs` to keep this
//! crate's test load light.
//!
//! Multi-worker codegen (TASK-0122) is now in place. A synthetic
//! two-worker pingpong test lives in `tests/multi_worker.rs`. The
//! load-bearing end-to-end test (example 02 split.sched.nuc against
//! `reference.bin`) lives at
//! `nucleus/nucleus-compiler/tests/e2e_example_02.rs`.
//!
//! At this layer (the unit-test surface of the backend) we keep:
//! - The single-worker happy-path tests below (example 01 × naive).
//! - A small synthetic test that proves a two-worker ACFG produces
//!   compilable Rust without a runtime check (the runtime is in the
//!   multi_worker.rs harness).
//! - A test that rejects an unsupported distributed placement
//!   (`place k on {w0,w1,...}`).

use std::fs;
use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use nucleus_compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};
use pthreads_sync::{emit, EmitError, NameTables};

/// Build the EventList + sidecar + reverse name tables from a
/// post-pass ACFG + LinkedIR — exactly what the driver does. Tests
/// that previously called `emit(&acfg, &linked, ...)` now go through
/// this so they exercise the real TASK-0124 contract path.
fn contract_inputs(
    linked: &nucleus_compiler::link::LinkedIR,
    acfg: &nucleus_compiler::ACFG,
) -> (
    BTreeMap<nucleus_compiler::WorkerId, Vec<nucleus_compiler::event::Event>>,
    NameTables,
    nucleus_compiler::NameSidecar,
) {
    let per_worker = acfg_to_events(acfg);
    let sidecar = build_sidecar(linked, acfg).expect("build_sidecar");
    // TASK-0238 (cycle 25): 5-field NameTables literal collapsed to
    // the centralized constructor.
    let names = NameTables::from_acfg(acfg);
    (per_worker, names, sidecar)
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR -> nucleus/backends/pthreads-sync
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above pthreads-sync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    // TASK-0426.01: per-call-unique scratch dir via the shared helper
    // (created once, never removed) — kills the remove/create race class.
    let target = repo_root().join("nucleus/target/pthreads-sync-test-scratch");
    test_common::unique_scratch_dir(&target, name)
}

/// Load + link example 01 with the naive schedule. Shared helper.
fn link_example_01_naive() -> (
    nucleus_compiler::link::LinkedIR,
    nucleus_compiler::ACFG,
    PathBuf,
) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = fs::read_to_string(ex.join("schedules/naive.sched.nuc")).unwrap();

    let algo_ast = parse_algo(&algo_src).unwrap();
    let sched_ast = parse_sched(&sched_src).unwrap();
    let algo_ir = lower_algo(&algo_ast).unwrap();
    let sched_ir = lower_sched(&sched_ast).unwrap();
    let linked = link(algo_ir, sched_ir).unwrap();
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    (linked, acfg, ex.join("kernels.rs"))
}

/// Load + link an arbitrary example/schedule through the FULL driver
/// pipeline (incl. `apply_block_transforms`) so a blocked schedule
/// gets the tile rewrite. Returns `(linked, post-pass acfg, kernels
/// path)`.
fn link_example(
    ex_rel: &str,
    sched_rel: &str,
) -> (
    nucleus_compiler::link::LinkedIR,
    nucleus_compiler::ACFG,
    PathBuf,
) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(ex_rel);
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = fs::read_to_string(ex.join(sched_rel)).unwrap();
    let algo_ir = lower_algo(&parse_algo(&algo_src).unwrap()).unwrap();
    let sched_ir = lower_sched(&parse_sched(&sched_src).unwrap()).unwrap();
    let linked = link(algo_ir, sched_ir).unwrap();
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = nucleus_compiler::apply_block_transforms(&linked, acfg).unwrap();
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    (linked, acfg, ex.join("kernels.rs"))
}

/// Finding (7) reconciliation (TASK-0124): the sidecar sufficiency
/// tests in `nucleus-compiler/tests/petri_to_events.rs` HAND-MIRROR
/// the backend's bound/type/zero spelling (they cannot call the real
/// backend — `nucleus-compiler` must not depend on `pthreads-sync`).
/// Now
/// that the backend actually consumes the sidecar, pin the EXACT
/// load-bearing strings via the REAL `emit()` codegen here, at the
/// backend layer, so any spelling drift fails loudly against real
/// output rather than only against a mirror that drifts with it.
#[test]
fn golden_real_codegen_strings_pin_sidecar_consumption() {
    // 05-stencil naive: the symbolic loop bound rendered from
    // sidecar.loop_bounds + sidecar.consts, and the sidecar-sized
    // pre-init. Exactly the strings petri_to_events.rs's
    // `sidecar_renders_stencil_symbolic_loop_bound_in_source_form`
    // / `sidecar_alone_sizes_preinit_*` assert via mirrors.
    let (linked, acfg, kernels) = link_example("05-stencil", "schedules/naive.sched.nuc");
    let out = scratch_dir("golden_05_naive");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let main_rs =
        fs::read_to_string(emit(&pw, &names, &sc, &kernels, &out).unwrap().main_rs).unwrap();
    assert!(
        main_rs.contains("for y in (1_i64)..((16_i64 - 1_i64)) {"),
        "05-stencil symbolic bound from sidecar drifted:\n{main_rs}"
    );
    assert!(
        main_rs.contains("for x in (1_i64)..((16_i64 - 1_i64)) {"),
        "05-stencil inner symbolic bound drifted:\n{main_rs}"
    );
    assert!(
        main_rs.contains("let mut img_out = vec![0; 256];"),
        "05-stencil sidecar-sized pre-init drifted:\n{main_rs}"
    );

    // 07-matmul blocked: the absolute-index rebinding the backend
    // MUST do because block_transform defers it (TASK-0124). Pin the
    // exact rebinding spelling so it cannot silently regress.
    let (linked, acfg, kernels) = link_example("07-matmul", "schedules/blocked.sched.nuc");
    let out = scratch_dir("golden_07_blocked");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let main_rs =
        fs::read_to_string(emit(&pw, &names, &sc, &kernels, &out).unwrap().main_rs).unwrap();
    assert!(
        main_rs.contains("for i__tile in (0_i64)..(2_i64) {")
            && main_rs.contains("for i in (0_i64)..(8_i64) {"),
        "07 blocked tile/inner loop headers drifted:\n{main_rs}"
    );
    assert!(
        main_rs.contains("(0_i64 + (i__tile * 8_i64) + i)")
            && main_rs.contains("(0_i64 + (j__tile * 8_i64) + j)"),
        "07 blocked absolute-index rebinding drifted (accumulator would \
         double-count without it):\n{main_rs}"
    );
}

#[test]
fn emit_writes_all_files() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("emit_writes_all_files");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let result = emit(&pw, &names, &sc, &kernels, &out).expect("emit succeeded");

    // All four artefacts present.
    assert!(result.cargo_toml.exists(), "Cargo.toml not written");
    assert!(result.main_rs.exists(), "main.rs not written");
    assert!(result.kernels_rs.exists(), "kernels.rs not copied");
    assert!(result.run_sh.exists(), "run.sh not written");

    // run.sh on unix is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&result.run_sh).unwrap().permissions().mode();
        // 0o111 is "any execute bit set". Don't pin the exact 0o755
        // because umask could legitimately strip the group/other.
        assert!(
            mode & 0o111 != 0,
            "run.sh is not executable (mode={mode:o})"
        );
    }
}

#[test]
fn main_rs_calls_every_kernel() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("main_rs_calls_every_kernel");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let result = emit(&pw, &names, &sc, &kernels, &out).unwrap();
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();

    // Smoke test: every kernel from example 01 appears as a call.
    for kernel in &["load_input", "load_input_b", "add", "save_output"] {
        let needle = format!("kernels::{kernel}");
        assert!(
            main_rs.contains(&needle),
            "main.rs is missing `{needle}`:\n---\n{main_rs}\n---"
        );
    }

    // The for-loop bound for example 01 is N=256.
    assert!(
        main_rs.contains("256_i64"),
        "main.rs is missing the N=256 loop bound:\n---\n{main_rs}\n---"
    );
}

#[test]
fn kernels_rs_is_copied_verbatim() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("kernels_rs_is_copied_verbatim");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let result = emit(&pw, &names, &sc, &kernels, &out).unwrap();
    let src = fs::read_to_string(&kernels).unwrap();
    let dst = fs::read_to_string(&result.kernels_rs).unwrap();
    assert_eq!(src, dst, "kernels.rs was not copied byte-for-byte");
}

/// TASK-0209 AC#4: a Fire whose output binding has *fewer* indices
/// than the data's declared rank must lower to a contiguous sub-array
/// `copy_from_slice` write — NOT a single-scalar `name[idx] = ...`
/// assignment. Symmetrically, a kernel arg with rank-mismatched
/// (partial-prefix) indices must lower to a `[start..start+sub_len]
/// .to_vec()` sub-slice — NOT a single-scalar `name[idx]` access.
///
/// The reproducer is example 13's `feat1[n] <-- conv_block_1(input[n])`
/// where `input`/`feat1` are rank-4 and `n` is rank-1. The
/// pre-TASK-0209 backend hard-rejected this with
/// `EmitError::UnsupportedFeature("rank/shape mismatch with index
/// list")`; the post-TASK-0209 backend emits the sub-slice form.
///
/// We drive the FULL pipeline (parse + lower + link + acfg + inject)
/// against the real example 13 algorithm so the test exercises the
/// real lowering path — not a synthetic Fire that might encode the
/// shape differently from what AlgoIR produces. A scratch
/// `kernels.rs` with the `(Vec<i32>) -> Vec<i32>` aggregate signature
/// matches the kernel-param convention picked under TASK-0103 AND
/// matches the i32 element type of example 13's `prog.algo.nuc` post
/// TASK-0053 cycle-2; the contract pass reports the aggregate gap
/// (TASK-0012) and the driver proceeds, exactly as it does on the
/// command line.
#[test]
fn partial_index_lowers_to_sub_slice() {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/13-cnn-inference");
    let scratch = scratch_dir("partial_index_lowers_to_sub_slice");

    // Synthesise a stub kernels.rs for example 13 (the real one is
    // TASK-0053's scope; we only need a path whose signatures are
    // contract-compatible enough for the driver path to proceed).
    // Element type is `i32` to match the example's `prog.algo.nuc`
    // after TASK-0053 cycle-2 landed (CNN uses integer arithmetic
    // per PRD §13 "Leaning toward integer-only for v2"; see
    // examples/13-cnn-inference/README.md).
    let kernels_path = scratch.join("kernels.rs");
    fs::write(
        &kernels_path,
        r#"
const B: usize = 16;
const H: usize = 28;
const W: usize = 28;
const C0: usize = 1;
const C1: usize = 8;
const C2: usize = 16;
const N_CLASSES: usize = 10;
pub fn load_input() -> Vec<i32> { vec![0; B * C0 * H * W] }
pub fn save_output(_data: Vec<i32>) {}
pub fn conv_block_1(_x: Vec<i32>) -> Vec<i32> { vec![0; C1 * (H / 2) * (W / 2)] }
pub fn conv_block_2(_x: Vec<i32>) -> Vec<i32> { vec![0; C2 * (H / 4) * (W / 4)] }
pub fn classifier(_x: Vec<i32>) -> Vec<i32> { vec![0; N_CLASSES] }
"#,
    )
    .expect("write stub kernels.rs");

    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = fs::read_to_string(ex.join("schedules/naive.sched.nuc")).unwrap();
    let algo_ir = lower_algo(&parse_algo(&algo_src).unwrap()).unwrap();
    let sched_ir = lower_sched(&parse_sched(&sched_src).unwrap()).unwrap();
    let linked = link(algo_ir, sched_ir).unwrap();
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    let out_dir = scratch.join("emit");
    fs::create_dir_all(&out_dir).unwrap();
    let (pw, names, sc) = contract_inputs(&linked, &acfg);
    let result = emit(&pw, &names, &sc, &kernels_path, &out_dir).expect("emit succeeded");
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();

    // ---- Argument-side: `input[n]` (rank-1 index on rank-4 data) ----
    // Pre-init confirmed: `input` is rank-4 i32[16][1][28][28] = 12544
    // slots, sub-array per outer index = 1*28*28 = 784 slots.
    assert!(
        main_rs.contains("input[((n) * 784) as usize..((n) * 784) as usize + 784usize].to_vec()"),
        "TASK-0209 AC#1: partial-index argument did NOT lower to a sub-slice\
         `.to_vec()`; pre-TASK-0209 emission `input[(n) as usize]` (single \
         i32) would have been emitted, breaking cargo build (E0308 expected \
         Vec<i32>, found i32). Generated main.rs:\n{main_rs}"
    );

    // ---- Output-side: `feat1[n]` (rank-1 LHS on rank-4 data) ----
    // feat1 is i32[16][8][14][14] = 25088 slots, sub-array per outer
    // index = 8*14*14 = 1568. Cycle-2 review-gate finding-2: the
    // emission now binds the RHS to `let _rhs = ...;`, asserts
    // `_rhs.len() == sub_len` for fail-loud-with-context on kernel
    // length mismatches, then copies into the destination sub-range.
    assert!(
        main_rs.contains("let _rhs = kernels::conv_block_1(")
            && main_rs.contains(
                "assert_eq!(_rhs.len(), 1568usize, \
                 \"kernel result for `feat1` slot returned {} elements, \
                 declared shape requires {}\""
            )
            && main_rs.contains(
                "feat1[((n) * 1568) as usize..((n) * 1568) as usize + 1568usize]\
                 .copy_from_slice(&_rhs);"
            ),
        "TASK-0209 AC#2 (cycle-2): partial-index Fire output did NOT lower to \
         the bound-RHS + length-assert + sub-range `.copy_from_slice(&_rhs)` \
         emission. Pre-TASK-0209 cycle-1 would have emitted scalar \
         `feat1[(n) as usize] = kernels::conv_block_1(...)` breaking cargo \
         build (E0308 expected i32, found Vec<i32>); pre-cycle-2 would have \
         emitted the bare `.copy_from_slice(&kernels::conv_block_1(...))` \
         form which panics with std's terse message on kernel-impl length \
         mismatch. Generated main.rs:\n{main_rs}"
    );

    // ---- Negative: no single-scalar slot access for the rank-4
    //      partial cases. If a regression reintroduces the scalar
    //      `name[idx] = kernels::callee(...)` form for a partial LHS,
    //      this substring would appear and the test fails LOUD. We
    //      anchor on the ACTUAL pre-TASK-0209 string — the 1D fast
    //      path emitted `(i0) as usize` with no stride factor, so the
    //      pre-cycle-1 spelling was `feat1[(n) as usize] = ...`, NOT
    //      `feat1[((n) * 1568) as usize] = ...` (cycle-2 review-gate
    //      finding-3 corrected this anchor).
    assert!(
        !main_rs.contains("feat1[(n) as usize] = kernels::conv_block_1"),
        "TASK-0209 regression: scalar-slot assignment to a partial-rank LHS \
         (`feat1[(n) as usize] = ...`) reintroduced. Generated main.rs:\n{main_rs}"
    );

    // ---- AC#3: the emitted crate actually cargo-builds. The driver
    //      emit() above wrote it; verify with cargo check (not full
    //      build — the test only needs to prove the type-checker
    //      accepts the Vec<i32>/Vec<i32> contract).
    //
    // We isolate the target dir under `scratch` to avoid colliding
    // with the workspace target. CARGO_TARGET_DIR points the build
    // outputs at the scratch tree.
    let target = scratch.join("nuc-target");
    let status = std::process::Command::new(env!("CARGO"))
        .arg("check")
        .arg("--manifest-path")
        .arg(result.cargo_toml)
        .env("CARGO_TARGET_DIR", &target)
        .status()
        .expect("spawn cargo check");
    assert!(
        status.success(),
        "TASK-0209 AC#3: emitted crate did NOT cargo-check (the original \
         E0308 reproducer). Inspect {} for the generated main.rs.",
        out_dir.display()
    );
}

// ---- TASK-0404: bite tests for the three I/O EmitError variants ----
//
// `EmitError::{KernelsReadFailed, OutputCreateFailed, WriteFailed}` were
// the only EmitError variants with ZERO test coverage (the cycle-236
// audit: `UnsupportedFeature`/`ContractGap`/`AccumulatorShapeMismatch`
// are covered, and `LowerErrorKind`/`SchedLowerErrorKind`/`LinkErrorKind`
// are fully bite-tested). They are thin `fs::*().map_err(EmitError::X)`
// wrappers reachable on a real filesystem failure — proving they fire a
// TYPED error rather than a panic is the panic-not-diagnostic discharge
// (PRD §10) for the emit I/O seam. The same three sites recur verbatim
// in all ten backends; a grep audit (cycle-236) confirmed every one uses
// `.map_err(EmitError::…)` (none `unwrap`/`expect`/`?`-on-raw-io), so
// pthreads-sync — the canonical backend — proves the variants at
// VARIANT granularity for the family. Each failure mode is induced
// deterministically and portably (no permission-bit reliance, which is
// unreliable as root / in CI): a missing read path, a path component
// that is a regular file, and a write target that is a directory.
//
// `EmitError` derives only `Debug` (its `io::Error` payload is not
// `PartialEq`), so each test matches on the variant pattern rather than
// `assert_eq!` on the whole error.

/// `KernelsReadFailed`: the very first I/O op in `emit` is
/// `fs::read_to_string(kernels_rs_path)`; a path that does not exist
/// fails it before any rendering or output creation.
#[test]
fn kernels_read_failed_on_missing_kernels_rs() {
    let (linked, acfg, _kernels) = link_example_01_naive();
    let out = scratch_dir("emiterr_kernels_read_failed");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);

    let missing = out.join("does-not-exist-kernels.rs");
    assert!(!missing.exists(), "precondition: the kernels path must not exist");

    let err = emit(&pw, &names, &sc, &missing, &out)
        .expect_err("a missing kernels.rs must fail with a typed error, not panic");
    match err {
        EmitError::KernelsReadFailed { path, source } => {
            assert_eq!(path, missing, "KernelsReadFailed must name the missing path");
            assert_eq!(
                source.kind(),
                std::io::ErrorKind::NotFound,
                "the wrapped io::Error must be NotFound for a missing file"
            );
        }
        other => panic!("expected KernelsReadFailed, got {other:?}"),
    }
}

/// `OutputCreateFailed`: `emit` reads kernels (ok) then
/// `fs::create_dir_all(out_dir/src)`. Rooting `out_dir` under a regular
/// FILE makes that fail (a path component is not a directory).
#[test]
fn output_create_failed_when_a_path_component_is_a_file() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let scratch = scratch_dir("emiterr_output_create_failed");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);

    // A regular file used as a directory component: `create_dir_all`
    // of `out_dir/src` = `blocker/nested-out/src` must fail because
    // `blocker` is a file, not a directory.
    let blocker = scratch.join("blocker");
    fs::write(&blocker, b"i am a regular file, not a directory").unwrap();
    let out = blocker.join("nested-out");

    let err = emit(&pw, &names, &sc, &kernels, &out)
        .expect_err("out_dir under a regular file must fail with a typed error");
    match err {
        EmitError::OutputCreateFailed { path, .. } => {
            assert!(
                path.starts_with(&out),
                "OutputCreateFailed must name a path under out_dir; got {path:?}"
            );
        }
        other => panic!("expected OutputCreateFailed, got {other:?}"),
    }
}

/// `WriteFailed`: `emit` reads kernels (ok), creates `out_dir/src` (ok),
/// then `write_file(out_dir/Cargo.toml)` — the first generated write.
/// Pre-creating `out_dir/Cargo.toml` as a DIRECTORY makes `fs::write`
/// fail (the target path is a directory).
#[test]
fn write_failed_when_target_path_is_a_directory() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("emiterr_write_failed");
    let (pw, names, sc) = contract_inputs(&linked, &acfg);

    let cargo_toml_as_dir = out.join("Cargo.toml");
    fs::create_dir_all(&cargo_toml_as_dir)
        .expect("pre-create the Cargo.toml path as a directory");

    let err = emit(&pw, &names, &sc, &kernels, &out)
        .expect_err("writing onto a directory must fail with a typed error");
    match err {
        EmitError::WriteFailed { path, .. } => {
            assert_eq!(
                path, cargo_toml_as_dir,
                "WriteFailed must name the Cargo.toml path it could not write"
            );
        }
        other => panic!("expected WriteFailed, got {other:?}"),
    }
}

// REMOVED (TASK-0124): `distributed_placement_is_rejected`.
//
// That test hand-built a `LinkedIR` with a `place dist on {w0,w1}`
// distributed placement and asserted that the *backend*
// (`multi_worker::validate_placements`) rejected it. TASK-0124 moved
// the backend off `&ACFG`/`&LinkedIR` entirely (AC#2): the backend
// now consumes only the per-worker EventList + NameSidecar + name
// tables and has no `placements`/`kernel_workers` to validate — that
// is no longer a backend responsibility. Distributed-placement
// rejection now lives upstream of the projection (the capability
// check + the deliberate e2e SKIP of every `distributed` cell,
// tracked by TASK-0117). Re-introducing this assertion at the
// backend layer would require putting `LinkedIR` back into the
// `emit()` signature, directly contradicting AC#2. Keeping a
// synthetic ACFG/LinkedIR test here would therefore test a
// responsibility the backend no longer has. The two-worker codegen
// path itself stays covered end-to-end by `tests/multi_worker.rs`
// (synthetic pingpong) and `nucleus-compiler/tests/e2e_example_02.rs`
// (example 02 `split.sched.nuc`).
