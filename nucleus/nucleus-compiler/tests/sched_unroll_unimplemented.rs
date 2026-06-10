//! Negative tests for the `unroll=N` accepted-but-unimplemented loud
//! reject (TASK-0458).
//!
//! `unroll=N` parses (parser.rs), is positivity-checked, and lowers to
//! `ResolvedLoopOption::Unroll` — but NO downstream pass consumes it.
//! Silently accepting it would be the exact silent-downgrade pattern the
//! capability matrix forbids elsewhere, so sched-lowering now rejects any
//! `unroll=N` with a typed `UnrollUnimplemented` diagnostic that names the
//! option and cites the deferral (TASK-0293 / PRD §6.3.3).
//!
//! Lives in its own file (not `sched_lower.rs`) per the TASK-0458
//! file-ownership split. When TASK-0293 lands the real unroll consumer,
//! these tests (and the reject they pin) must be removed/replaced.

use nucleus_compiler::sched::{
    lower_sched, parse_sched, SchedIR, SchedLowerErrorKind, SchedLowerErrors,
};

/// Parse + lower in one helper. Panics if parsing fails (these inputs must
/// parse — they exercise lowering, not the parser). Mirrors the
/// `lower_str` helper in `sched_lower.rs`.
fn lower_str(src: &str) -> Result<SchedIR, SchedLowerErrors> {
    let ast = parse_sched(src).expect("source must parse for this lowering test");
    lower_sched(&ast)
}

#[test]
fn negative_bare_unroll_is_rejected_as_unimplemented() {
    // TASK-0458: a bare `unroll=N` (no `block`) lowers to a
    // `ResolvedLoopOption::Unroll` that no pass consumes. Refuse it loudly
    // rather than silently doing nothing.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : unroll=8;
}
";
    let err = lower_str(src)
        .expect_err("bare unroll=N must be rejected as unimplemented")
        .first()
        .clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnrollUnimplemented { var: "n".into() }
    );
}

#[test]
fn negative_unroll_diagnostic_names_option_and_cites_deferral() {
    // AC#1: the diagnostic must NAME the unimplemented option and cite the
    // deferral task so a schedule author knows it is accepted-but-inert,
    // not a typo. This is the load-bearing user-facing contract.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : unroll=4;
}
";
    let err = lower_str(src)
        .expect_err("unroll=N must be rejected")
        .first()
        .clone();
    let msg = format!("{}", err.kind);
    assert!(
        msg.contains("unroll"),
        "diagnostic must name the `unroll` option: {msg}"
    );
    assert!(
        msg.contains("not yet implemented") || msg.contains("unimplemented"),
        "diagnostic must flag the option as unimplemented: {msg}"
    );
    // User-facing diagnostic is tracker-ID-free (TASK-0455.06): no
    // internal TASK-NNNN, but it must give an actionable fix.
    assert!(
        !msg.contains("TASK-0"),
        "user-facing diagnostic must not leak a tracker ID: {msg}"
    );
    assert!(
        msg.contains("Remove the `unroll` option"),
        "diagnostic must give a concrete fix hint: {msg}"
    );
}

#[test]
fn negative_block_divisible_unroll_is_still_rejected() {
    // A `block=N, unroll=M` pair where `M` divides `N` (4 | 8) passes the
    // pre-existing divisibility check — and would, before TASK-0458, have
    // been silently accepted as an inert `ResolvedLoopOption::Unroll`. It
    // must now hit the loud unimplemented reject (no silent acceptance for
    // the divisible case either).
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : block=8, unroll=4;
}
";
    let err = lower_str(src)
        .expect_err("block-divisible unroll must still be rejected as unimplemented")
        .first()
        .clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnrollUnimplemented { var: "n".into() }
    );
}

#[test]
fn negative_block_nondivisible_unroll_keeps_divisibility_diagnostic() {
    // Ordering guard (TASK-0458): the new unimplemented reject is placed
    // AFTER the `UnrollNotDivisibleByBlock` divisibility check, so the more
    // specific bad-combination diagnostic still wins for a non-divisible
    // `block=N, unroll=M` pair (6 % 4 == 2). This pins the ordering so a
    // future refactor cannot silently swap which error surfaces.
    let src = "\
schedule for \"../prog.algo.nuc\" {
    workers = { host };
    loop n : block=6, unroll=4;
}
";
    let err = lower_str(src)
        .expect_err("non-divisible unroll must fail")
        .first()
        .clone();
    assert_eq!(
        err.kind,
        SchedLowerErrorKind::UnrollNotDivisibleByBlock {
            var: "n".into(),
            unroll: 4,
            block: 6,
        }
    );
}
