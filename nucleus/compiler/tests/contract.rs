//! Integration tests for the kernel-as-Rust-function contract pass
//! (TASK-0012).
//!
//! Test strategy: hand-rolled fixtures under
//! `tests/fixtures/contract/{good, bad-*}` containing both an
//! `algo.nuc` declaring kernels and a `kernels.rs` purporting to
//! implement them. Each negative fixture is constructed to violate
//! exactly one contract clause.
//!
//! Each test parses the algorithm, lowers it to AlgoIR (the contract
//! pass's input), then invokes [`check_kernels_contract`] against the
//! sibling Rust file and asserts the expected outcome.
//!
//! No part of these tests reaches into the wider `nuc-nucleus/examples`
//! tree — TASK-0012 explicitly notes that existing examples are not
//! required to have a `kernels.rs` yet, and the fixtures here are
//! self-contained.

use std::path::PathBuf;

use compiler::algo::{lower_algo, parse_algo};
use compiler::{check_kernels_contract, ContractError};

/// Workspace-relative fixture root.
fn fixture_dir(name: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join("contract")
        .join(name)
}

/// Parse + lower the `algo.nuc` under `fixtures/contract/<name>`.
/// Panics on failure — the contract pass cannot do its job without
/// AlgoIR, and the upstream layers are tested elsewhere.
fn algo_for(name: &str) -> compiler::algo::AlgoIR {
    let path = fixture_dir(name).join("algo.nuc");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let ast = parse_algo(&src).expect("algo fixture must parse");
    lower_algo(&ast).expect("algo fixture must lower")
}

fn kernels_rs_for(name: &str) -> PathBuf {
    fixture_dir(name).join("kernels.rs")
}

// --------------------------------------------------------------------
// Positive: contract holds end-to-end
// --------------------------------------------------------------------

#[test]
fn good_fixture_passes_contract_check() {
    let algo = algo_for("good");
    let kernels = kernels_rs_for("good");
    let result = check_kernels_contract(&algo, &kernels);
    if let Err(errs) = &result {
        panic!("expected contract to hold; got errors: {errs:?}");
    }
}

// --------------------------------------------------------------------
// Negative: one fixture per ContractError variant
// --------------------------------------------------------------------

#[test]
fn bad_kernel_missing_produces_kernel_not_found() {
    let algo = algo_for("bad-kernel-missing");
    let kernels = kernels_rs_for("bad-kernel-missing");
    let errs =
        check_kernels_contract(&algo, &kernels).expect_err("expected at least one ContractError");
    assert!(
        errs.iter()
            .any(|e| matches!(e, ContractError::KernelNotFound { kernel } if kernel == "mul")),
        "expected KernelNotFound(mul); got {errs:?}"
    );
}

#[test]
fn bad_arity_mismatch_produces_arity_mismatch() {
    let algo = algo_for("bad-arity-mismatch");
    let kernels = kernels_rs_for("bad-arity-mismatch");
    let errs =
        check_kernels_contract(&algo, &kernels).expect_err("expected at least one ContractError");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ContractError::ArityMismatch {
                kernel,
                expected_arity: 2,
                actual_arity: 3
            } if kernel == "add"
        )),
        "expected ArityMismatch(add, 2, 3); got {errs:?}"
    );
}

#[test]
fn bad_type_mismatch_produces_type_mismatch_on_param() {
    let algo = algo_for("bad-type-mismatch");
    let kernels = kernels_rs_for("bad-type-mismatch");
    let errs =
        check_kernels_contract(&algo, &kernels).expect_err("expected at least one ContractError");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ContractError::TypeMismatch {
                kernel,
                position: Some(1),
                expected,
                actual,
            } if kernel == "add" && expected == "f32" && actual == "f64"
        )),
        "expected TypeMismatch(add, pos=1, f32 vs f64); got {errs:?}"
    );
}

#[test]
fn bad_return_type_mismatch_produces_type_mismatch_on_return() {
    let algo = algo_for("bad-return-type-mismatch");
    let kernels = kernels_rs_for("bad-return-type-mismatch");
    let errs =
        check_kernels_contract(&algo, &kernels).expect_err("expected at least one ContractError");
    assert!(
        errs.iter().any(|e| matches!(
            e,
            ContractError::TypeMismatch {
                kernel,
                position: None,
                expected,
                actual,
            } if kernel == "norm" && expected == "f32" && actual == "f64"
        )),
        "expected TypeMismatch(norm, return, f32 vs f64); got {errs:?}"
    );
}

#[test]
fn bad_missing_pub_produces_missing_pub() {
    let algo = algo_for("bad-missing-pub");
    let kernels = kernels_rs_for("bad-missing-pub");
    let errs =
        check_kernels_contract(&algo, &kernels).expect_err("expected at least one ContractError");
    assert!(
        errs.iter()
            .any(|e| matches!(e, ContractError::MissingPub { kernel } if kernel == "add")),
        "expected MissingPub(add); got {errs:?}"
    );
}

#[test]
fn bad_rust_check_failed_produces_rust_check_failed() {
    let algo = algo_for("bad-rust-check-failed");
    let kernels = kernels_rs_for("bad-rust-check-failed");
    let errs =
        check_kernels_contract(&algo, &kernels).expect_err("expected at least one ContractError");
    assert!(
        errs.iter()
            .any(|e| matches!(e, ContractError::RustCheckFailed { .. })),
        "expected RustCheckFailed; got {errs:?}"
    );
    // The signature for `add` should still parse OK in this fixture
    // (rustc fails only on the body), so no signature-level error.
    assert!(
        !errs
            .iter()
            .any(|e| matches!(e, ContractError::KernelNotFound { .. })),
        "expected the signature pass to still find `add`; got {errs:?}"
    );
}

// --------------------------------------------------------------------
// Example 01-elementwise-add: documents the current scalar-only
// limitation of the contract pass against a real example's
// kernels.rs (TASK-0013).
//
// The kernels.rs uses `Vec<i32>` for I/O kernels declared in Nuc as
// `i32[N]` (aggregate). The contract pass is scalar-only at this
// point and emits `TypeMismatch` for those aggregate slots with a
// message about aggregates not yet being supported — loud failure,
// not silent acceptance. The scalar `add` kernel passes.
//
// This test pins the *current* observable contract pass behaviour
// for the example. When aggregate matching lands (TASK-0103 picks
// the convention, TASK-0012 follow-ups implement matching), this
// test should flip to assert `Ok(())`. The example file itself
// should not need to change.
// --------------------------------------------------------------------

#[test]
fn example_01_elementwise_add_contract_passes_for_add_and_loud_on_aggregates() {
    // Re-use the parser machinery; don't depend on the fixture dir
    // helper (the example lives outside `tests/fixtures/`).
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let example_dir = repo_root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = std::fs::read_to_string(example_dir.join("prog.algo.nuc"))
        .expect("example 01 prog.algo.nuc must be readable");
    let ast = parse_algo(&algo_src).expect("example 01 must parse");
    let ir = lower_algo(&ast).expect("example 01 must lower");

    let kernels_rs = example_dir.join("kernels.rs");
    let errs = check_kernels_contract(&ir, &kernels_rs)
        .expect_err("aggregate kernels currently produce TypeMismatch");

    // Loud failure mode: every error is a TypeMismatch on an
    // aggregate-typed kernel (load_input, load_input_b, save_output).
    // None of: KernelNotFound, MissingPub, ArityMismatch,
    // RustCheckFailed, KernelsFileUnreadable, KernelsFileUnparseable.
    for e in &errs {
        match e {
            ContractError::TypeMismatch { kernel, .. } => {
                assert!(
                    matches!(
                        kernel.as_str(),
                        "load_input" | "load_input_b" | "save_output"
                    ),
                    "unexpected TypeMismatch on `{kernel}`: scalar `add` must pass; got {errs:?}"
                );
            }
            other => panic!(
                "unexpected ContractError variant on example 01: {other:?}; \
                 only TypeMismatch on the three aggregate I/O kernels is expected. \
                 Did rustc reject kernels.rs? Did the parser fail on the algo?"
            ),
        }
    }

    // Must have caught all three aggregate I/O kernels; otherwise
    // the matcher started silently accepting `Vec<i32>` for some of
    // them — that would be a regression in loudness, not progress.
    let aggregates: std::collections::BTreeSet<&str> = errs
        .iter()
        .filter_map(|e| match e {
            ContractError::TypeMismatch { kernel, .. } => Some(kernel.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        aggregates.contains("load_input"),
        "expected TypeMismatch on load_input; got {errs:?}"
    );
    assert!(
        aggregates.contains("load_input_b"),
        "expected TypeMismatch on load_input_b; got {errs:?}"
    );
    assert!(
        aggregates.contains("save_output"),
        "expected TypeMismatch on save_output; got {errs:?}"
    );

    // Scalar `add` MUST NOT appear in the errors — it has a clean
    // (i32, i32) -> i32 signature on both sides.
    assert!(
        !errs
            .iter()
            .any(|e| matches!(e, ContractError::TypeMismatch { kernel, .. } if kernel == "add")),
        "scalar `add` must pass; got {errs:?}"
    );
}

// --------------------------------------------------------------------
// Example 02-split-add: same contract-pass behaviour as example 01
// (TASK-0021). The two examples share an algorithm shape and a
// kernels.rs shape (Vec<i32> aggregates, scalar `add`). Until
// TASK-0012's aggregate matching lands, the pass is loud on the
// aggregates and clean on `add`.
//
// If example 02's algorithm gains kernels of new shape (it
// shouldn't — its job is the *schedule*-side decomposition, not new
// algorithmic surface), update this test.
// --------------------------------------------------------------------

#[test]
fn example_02_split_add_contract_passes_for_add_and_loud_on_aggregates() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let example_dir = repo_root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(example_dir.join("prog.algo.nuc"))
        .expect("example 02 prog.algo.nuc must be readable");
    let ast = parse_algo(&algo_src).expect("example 02 must parse");
    let ir = lower_algo(&ast).expect("example 02 must lower");

    let kernels_rs = example_dir.join("kernels.rs");
    let errs = check_kernels_contract(&ir, &kernels_rs)
        .expect_err("aggregate kernels currently produce TypeMismatch");

    // Loud failure mode: every error is a TypeMismatch on one of the
    // three aggregate I/O kernels. No KernelNotFound, MissingPub,
    // ArityMismatch, RustCheckFailed, file-IO variants.
    for e in &errs {
        match e {
            ContractError::TypeMismatch { kernel, .. } => {
                assert!(
                    matches!(
                        kernel.as_str(),
                        "load_input" | "load_input_b" | "save_output"
                    ),
                    "unexpected TypeMismatch on `{kernel}`: scalar `add` must pass; got {errs:?}"
                );
            }
            other => panic!(
                "unexpected ContractError variant on example 02: {other:?}; \
                 only TypeMismatch on the three aggregate I/O kernels is expected. \
                 Did rustc reject kernels.rs? Did the parser fail on the algo?"
            ),
        }
    }

    // All three aggregate I/O kernels must have surfaced; if the
    // matcher started silently accepting Vec<i32> for some, that's
    // a regression in loudness, not progress.
    let aggregates: std::collections::BTreeSet<&str> = errs
        .iter()
        .filter_map(|e| match e {
            ContractError::TypeMismatch { kernel, .. } => Some(kernel.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        aggregates.contains("load_input"),
        "expected TypeMismatch on load_input; got {errs:?}"
    );
    assert!(
        aggregates.contains("load_input_b"),
        "expected TypeMismatch on load_input_b; got {errs:?}"
    );
    assert!(
        aggregates.contains("save_output"),
        "expected TypeMismatch on save_output; got {errs:?}"
    );

    // Scalar `add` MUST NOT appear.
    assert!(
        !errs
            .iter()
            .any(|e| matches!(e, ContractError::TypeMismatch { kernel, .. } if kernel == "add")),
        "scalar `add` must pass; got {errs:?}"
    );
}

// --------------------------------------------------------------------
// File-level negative: missing file
// --------------------------------------------------------------------

#[test]
fn missing_kernels_file_produces_unreadable() {
    let algo = algo_for("good");
    let bogus = fixture_dir("good").join("does_not_exist.rs");
    let errs =
        check_kernels_contract(&algo, &bogus).expect_err("expected at least one ContractError");
    assert!(
        errs.iter()
            .any(|e| matches!(e, ContractError::KernelsFileUnreadable { .. })),
        "expected KernelsFileUnreadable; got {errs:?}"
    );
}
