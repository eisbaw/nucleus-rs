//! Compile-time registry of every backend's capability matrix, plus the
//! "which OTHER backends would accept this schedule?" fix-hint helper
//! (TASK-0455.06.04).
//!
//! # Why this lives in the driver, not in `nucleus-compiler`
//!
//! `nucleus-compiler::check_schedule_compat` only ever sees the ONE
//! backend the user chose — it has no notion of "the other backends".
//! Listing the backends that WOULD accept a rejected schedule needs ALL
//! ten capability matrices in hand. The single source of truth for those
//! matrices is each backend crate's committed `capabilities.toml`; we
//! pull them in verbatim at build time with [`include_str!`] rather than
//! hand-duplicating the matrices in Rust (the same single-source pattern
//! `mp_tcp_common::WIRE_RUNTIME_SRC` uses for the wire runtime).
//!
//! The driver is the right home because it is the crate that already
//! enumerates all ten backend names (see `dispatch::dispatch_backend`)
//! and owns the `nucleus build --backend <name>` selection. Keeping the
//! registry here (a crate the `check-include-str-coverage` gate does NOT
//! scan) also avoids that gate looking for a `mod`/`include!` pairing for
//! a `.toml` include — the gate's contract is about `.rs` `include_str!`s
//! that need compile coverage, which does not apply to a parsed-at-
//! runtime TOML blob.
//!
//! # Determinism
//!
//! [`BACKEND_CAPS`] is a fixed-order array (the same order
//! `dispatch::dispatch_backend` registers the backends in, tier-1 →
//! tier-3 → tier-2). [`accepting_backends`] preserves that order, so the
//! emitted "backends that accept this schedule" hint is a deterministic
//! pure function of the schedule — the project pins diagnostic strings
//! and runs a determinism gate, so a hash-order or filesystem-order
//! dependence here would be a defect.

use nucleus_compiler::sched::SchedIR;
use nucleus_compiler::{check_schedule_compat, parse_capabilities};

/// `(backend name, its committed capabilities.toml text)` for every
/// backend the driver can dispatch to. The TOML is pulled in at build
/// time straight from each backend crate's source file, so this registry
/// cannot drift from the matrices the backends actually ship — the file
/// IS the single source of truth.
///
/// Order: identical to `dispatch::dispatch_backend`'s match arms
/// (tier-1, then tier-3 embedded, then tier-2 MPI). Load-bearing for the
/// determinism of [`accepting_backends`]'s output.
pub(crate) const BACKEND_CAPS: &[(&str, &str)] = &[
    (
        "pthreads-sync",
        include_str!("../../backends/pthreads-sync/capabilities.toml"),
    ),
    (
        "mp-tcp-bufsync",
        include_str!("../../backends/mp-tcp-bufsync/capabilities.toml"),
    ),
    (
        "pthreads-async",
        include_str!("../../backends/pthreads-async/capabilities.toml"),
    ),
    (
        "mp-tcp-event",
        include_str!("../../backends/mp-tcp-event/capabilities.toml"),
    ),
    (
        "openmp-rs",
        include_str!("../../backends/openmp-rs/capabilities.toml"),
    ),
    (
        "mp-tcp-poll",
        include_str!("../../backends/mp-tcp-poll/capabilities.toml"),
    ),
    (
        "mp-uds-event",
        include_str!("../../backends/mp-uds-event/capabilities.toml"),
    ),
    (
        "embedded-pattern",
        include_str!("../../backends/embedded-pattern/capabilities.toml"),
    ),
    (
        "mpi-blocking",
        include_str!("../../backends/mpi-blocking/capabilities.toml"),
    ),
    (
        "mpi-nonblocking",
        include_str!("../../backends/mpi-nonblocking/capabilities.toml"),
    ),
];

/// The backends — other than `chosen` — whose capability matrix would
/// accept `sched`, in [`BACKEND_CAPS`] declaration order.
///
/// `chosen` is excluded because the caller only reaches here AFTER the
/// chosen backend rejected the schedule; listing it back would be noise.
/// A backend whose `capabilities.toml` somehow fails to parse is silently
/// skipped (it cannot be a recommendable target if its own matrix is
/// malformed) — but every committed file parses, and the
/// `cap_registry_tests::all_committed_matrices_parse` test pins that, so
/// a future malformed edit fails loudly in CI rather than silently
/// shrinking this list at runtime.
pub(crate) fn accepting_backends(chosen: &str, sched: &SchedIR) -> Vec<&'static str> {
    BACKEND_CAPS
        .iter()
        .filter(|(name, _)| *name != chosen)
        .filter_map(|(name, toml)| {
            let caps = parse_capabilities(toml).ok()?;
            check_schedule_compat(&caps, sched).ok().map(|()| *name)
        })
        .collect()
}

/// Format the actionable "which backends accept this?" fix hint appended
/// to a capability-rejection error. Returns the help line WITHOUT a
/// leading newline (the caller controls separation). Deterministic:
/// fixed registry order; "none" handled explicitly.
pub(crate) fn accepting_backends_hint(chosen: &str, sched: &SchedIR) -> String {
    let accepting = accepting_backends(chosen, sched);
    if accepting.is_empty() {
        "help: no available backend accepts this schedule as written".to_string()
    } else {
        format!(
            "help: backends that accept this schedule: {}",
            accepting.join(", ")
        )
    }
}

#[cfg(test)]
mod cap_registry_tests {
    use super::*;
    use nucleus_compiler::algo::{lower_algo, parse_algo};
    use nucleus_compiler::link;
    use nucleus_compiler::sched::{lower_sched, parse_sched};

    /// Every committed `capabilities.toml` in the registry must parse +
    /// validate. If one stops parsing, `accepting_backends` would
    /// silently drop it from the hint — pin it loudly here (AC#4 anti-rot
    /// + the malformed-skip caveat in `accepting_backends`).
    #[test]
    fn all_committed_matrices_parse() {
        for (name, toml) in BACKEND_CAPS {
            assert!(
                parse_capabilities(toml).is_ok(),
                "registry capabilities.toml for `{name}` failed to parse"
            );
        }
        // All ten backends present (guards against a copy/paste drop).
        assert_eq!(BACKEND_CAPS.len(), 10, "expected 10 backends in registry");
    }

    /// Build a linked `SchedIR` from algorithm + schedule source. The
    /// helper mirrors the driver's own lower→link sequence so the test
    /// exercises the SAME `SchedIR` shape `accepting_backends` sees in
    /// production.
    fn linked_sched(algo_src: &str, sched_src: &str) -> SchedIR {
        let algo_ast = parse_algo(algo_src).expect("algo parses");
        let algo = lower_algo(&algo_ast).expect("algo lowers");
        let sched_ast = parse_sched(sched_src).expect("sched parses");
        let sched = lower_sched(&sched_ast).expect("sched lowers");
        link(algo, sched).expect("links").sched
    }

    /// An `async` transfer is rejected by sync-only backends
    /// (pthreads-sync) but accepted by the async backends. The hint must
    /// name the async-capable backends, in registry order, and must NOT
    /// name the chosen sync backend. (AC#1 + AC#4.)
    #[test]
    fn async_schedule_lists_async_capable_backends() {
        let algo = "\
const N : usize = 4;
data seeds  : i32[N];
data stream : i32[N];
data result : i32[N];
kernel produce   : (i32) -> i32 pure;
kernel transform : (i32) -> i32 pure;
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> ()    effectful;
seeds <-- load_input();
for n : 0 .. N {
    stream[n] <-- produce(seeds[n]);
    result[n] <-- transform(stream[n]);
}
save_output(result);
";
        let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place produce     on w0;
    place transform   on w1;
    transfer seeds  : sync;
    transfer stream : async;
    transfer result : sync;
}
";
        let s = linked_sched(algo, sched);
        // pthreads-sync rejects async; verify it is NOT in the list.
        let accepting = accepting_backends("pthreads-sync", &s);
        assert!(
            !accepting.contains(&"pthreads-sync"),
            "chosen backend must be excluded; got {accepting:?}"
        );
        // The async-capable tier-1 backends accept it.
        for want in ["pthreads-async", "mp-tcp-event", "mp-uds-event"] {
            assert!(
                accepting.contains(&want),
                "async-capable `{want}` must accept the async schedule; got {accepting:?}"
            );
        }
        // Deterministic registry order: the list is a subsequence of the
        // registry order (every adjacent pair respects the declared
        // order).
        let order: Vec<usize> = accepting
            .iter()
            .map(|n| BACKEND_CAPS.iter().position(|(b, _)| b == n).unwrap())
            .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "accepting list must be in registry order; got {accepting:?}"
        );
        // Rendered hint shape.
        let hint = accepting_backends_hint("pthreads-sync", &s);
        assert!(
            hint.starts_with("help: backends that accept this schedule: "),
            "got: {hint}"
        );
    }

    /// A schedule every backend can satisfy still yields a non-empty
    /// list when the chosen backend is one that accepts it (the others
    /// also accept) — guards against a degenerate "always empty" bug.
    #[test]
    fn trivial_schedule_lists_other_backends() {
        let algo = "\
kernel k : () -> () effectful;
k();
";
        let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place k on host;
}
";
        let s = linked_sched(algo, sched);
        let hint = accepting_backends_hint("pthreads-sync", &s);
        assert!(
            hint.starts_with("help: backends that accept this schedule: "),
            "a trivial schedule should be accepted by other backends; got: {hint}"
        );
    }
}
