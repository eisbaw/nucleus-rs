//! Behaviour-pin for the zero-init invariant on the time-stepped
//! state arrays of 16-jacobi and 11-game-of-life (TASK-0348, filed as
//! TASK-0341.02 cycle-206 architect P3.3).
//!
//! ## The invariant
//!
//! Both examples rest on an UNSTATED codegen invariant: a data symbol
//! that is never explicitly assigned by a Dataflow stmt before its
//! first read is pre-initialised to all-zero by the backend's
//! `vec![0; N]` allocation. Two consumers depend on it:
//!
//! 1. **The modular-wrap seed read.** 16-jacobi's kernel reads
//!    `field[(t + ITERS) % (ITERS + 1)][...]` — at `t == 0` this
//!    indexes `field[ITERS]` (the as-yet-unwritten top slice), whose
//!    cells must be 0 (the kernel ignores them at `t == 0` and returns
//!    the seed, but the READ still happens and must not be UB / garbage).
//!    11-game-of-life has the identical `grid[(t + ITERS) % (ITERS + 1)]`
//!    shape.
//! 2. **Dirichlet zero-boundary.** 16-jacobi's boundary cells
//!    (`y in {0, H-1}` / `x in {0, W-1}`) are never written and must
//!    stay 0 — the prog.algo.nuc documents this as
//!    "Dirichlet zero-boundary condition by construction".
//!
//! ## Why a unit-layer pin (precedent: TASK-0303 / TASK-0304)
//!
//! The e2e diff (`just e2e`) WOULD catch a zero-init regression
//! (garbage boundary cells → wrong `reference.bin` compare), but only
//! end-to-end and with a coarse "bytes differ" diagnostic. This pin
//! bites at the unit-test layer with a precise message: if a future
//! cycle swaps `vec![0; N]` for `Vec::with_capacity(N)` + push (or any
//! non-zero-fill allocation), the emitted `main.rs` no longer carries
//! the `vec![0; ...]` line and this test fails pointing straight at the
//! zero-init contract.
//!
//! The expected `vec![0; N]` sizes are derived from the example dims:
//!
//!   - 16-jacobi: `field: i32[ITERS+1][H][W]` = `vec![0; 320]` (5*8*8); `result: i32[H][W]` = `vec![0; 64]`.
//!   - 11-game-of-life: `grid: i32[ITERS+1][N]` = `vec![0; 288]` (9*32); `result: i32[N]` = `vec![0; 32]`.
//!
//! If a benign dims change breaks the *size* assertions (but the
//! `vec![0;` zero-fill prefix still holds), update the expected size;
//! if the `vec![0;` prefix itself is gone, that is the REAL regression
//! this pin guards.
//!
//! Subprocess pattern mirrors `nucleus/driver/tests/cli_reuse_strict.rs`
//! (runs the cargo-prebuilt `nucleus` binary via CARGO_BIN_EXE; unit
//! profile, no `cargo run` recursion, no cargo-build of the emitted
//! project).

use std::path::PathBuf;
use std::process::Command;

/// Walk up from `CARGO_MANIFEST_DIR` until we find the repo root.
/// Same idiom as `cli_reuse_strict.rs` / `emit_pn.rs`.
fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if p.join("nucleus").join("Cargo.toml").exists()
            && p.join("nuc-nucleus").join("PRD.md").exists()
        {
            return p;
        }
        if !p.pop() {
            panic!("could not locate repo root from CARGO_MANIFEST_DIR");
        }
    }
}

fn nucleus_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nucleus"))
}

fn example_dir(name: &str) -> PathBuf {
    repo_root().join("nuc-nucleus").join("examples").join(name)
}

fn fresh_outdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nucleus-task0348-{}-{}", tag, std::process::id()));
    if p.exists() {
        let _ = std::fs::remove_dir_all(&p);
    }
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

/// Build `<example>/schedules/naive.sched.nuc` on pthreads-sync and
/// return the emitted `src/main.rs` as a string.
fn build_naive_main_rs(example: &str, tag: &str) -> String {
    let ex = example_dir(example);
    let out = fresh_outdir(tag);

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(ex.join("prog.algo.nuc"))
        .arg("--sched")
        .arg(ex.join("schedules").join("naive.sched.nuc"))
        .arg("--kernels")
        .arg(ex.join("kernels.rs"))
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--out")
        .arg(&out)
        .output()
        .expect("spawn nucleus");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "nucleus build of {example}/naive/pthreads-sync MUST succeed.\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let main_rs = out.join("src").join("main.rs");
    std::fs::read_to_string(&main_rs)
        .unwrap_or_else(|e| panic!("read emitted main.rs at {}: {e}", main_rs.display()))
}

/// Assert `<name>` is allocated via a zero-fill `vec![0; ...]`, and
/// (precision) at exactly `expected_len`.
fn assert_zero_init(main_rs: &str, example: &str, name: &str, expected_len: usize) {
    let zero_prefix = format!("let mut {name} = vec![0;");
    assert!(
        main_rs.contains(&zero_prefix),
        "ZERO-INIT CONTRACT REGRESSION: emitted main.rs for {example} no longer \
         allocates `{name}` via a zero-fill `vec![0; ...]`. This breaks the \
         modular-wrap seed read (`{name}[(t+ITERS)%(ITERS+1)]` at t==0 must read \
         0) and the Dirichlet zero-boundary (unwritten boundary cells must stay \
         0). If you intentionally changed the allocation strategy (e.g. \
         Vec::with_capacity + push), you MUST preserve zero-fill semantics and \
         update this pin.\n--- emitted main.rs ---\n{main_rs}"
    );
    let exact = format!("let mut {name} = vec![0; {expected_len}];");
    assert!(
        main_rs.contains(&exact),
        "size pin: expected `{exact}` in {example}'s emitted main.rs. If a \
         benign dims change moved the length (but the `vec![0;` zero-fill is \
         intact), update `expected_len`; the zero-fill prefix is the \
         load-bearing invariant and is checked separately above.\n\
         --- emitted main.rs ---\n{main_rs}"
    );
}

#[test]
fn jacobi_field_and_result_are_zero_initialised() {
    let main_rs = build_naive_main_rs("16-jacobi", "jacobi");
    // field: i32[ITERS+1][H][W] = [5][8][8] = 320 (the modular-wrap +
    // Dirichlet-boundary carrier).
    assert_zero_init(&main_rs, "16-jacobi", "field", 320);
    // result: i32[H][W] = [8][8] = 64 (the result-extract destination;
    // the extract loop covers the full grid, so result's zero-init is
    // not load-bearing for correctness, but the pin documents it).
    assert_zero_init(&main_rs, "16-jacobi", "result", 64);
}

#[test]
fn game_of_life_grid_and_result_are_zero_initialised() {
    let main_rs = build_naive_main_rs("11-game-of-life", "gol");
    // grid: i32[ITERS+1][N] = [9][32] = 288 (same modular-wrap shape as
    // 16-jacobi's `field`).
    assert_zero_init(&main_rs, "11-game-of-life", "grid", 288);
    // result: i32[N] = [32].
    assert_zero_init(&main_rs, "11-game-of-life", "result", 32);
}
