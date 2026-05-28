//! mp-tcp-bufsync plan shim.
//!
//! The multi-process emit `Plan` substrate (host election, xfer
//! registry, slice-paste tile derivation, accumulator classification,
//! FIFO-shape hazards, the entire event walk, worker-program assembly,
//! and host-relay codegen) lives ONCE in
//! [`backend_common::tcp_plan`], parameterised over the
//! [`backend_common::tcp_plan::WirePrimitives`] trait (TASK-0044.02.03
//! lift). This file supplies only mp-tcp-bufsync's `WirePrimitives`
//! impl — the BLOCKING wire primitives — plus a `Plan` type alias the
//! crate's `lib.rs` consumes exactly as it did the old in-crate `Plan`.
//!
//! mp-tcp-poll is the sibling consumer; its shim
//! (`nucleus/backends/mp-tcp-poll/src/plan.rs`) supplies the
//! nonblocking-poll variant. The two impls are the COMPLETE,
//! enumerable delta between the backends — everything else is shared.

use backend_common::tcp_plan::WirePrimitives;

use crate::SO_BUF_COMMENT_BUFSYNC;

/// mp-tcp-bufsync wire primitives: blocking `recv` framing. The three
/// `*_call` methods build the `wire::...` expression STRUCTURALLY from
/// their arguments (never by textual replace — sibling identifiers
/// contain the primitive names as substrings; see
/// `feedback-textual-replace-codegen-unsafe`).
pub(crate) struct BufsyncWire;

impl WirePrimitives for BufsyncWire {
    const BACKEND_NAME: &'static str = "mp-tcp-bufsync";
    const FILE_HEADER_BACKEND: &'static str = "mp-tcp-bufsync";
    const FILE_HEADER_TASK: &'static str = "TASK-0036";

    fn read_msg_expect_call(cv: &str, seq: u64) -> String {
        format!("wire::read_msg_expect(&mut {cv}, {seq})")
    }

    fn write_msg_call(cv: &str, seq: u64, enc: &str) -> String {
        format!("wire::write_msg(&mut {cv}, {seq}, &{enc})")
    }

    fn barrier_cross_call(cv: &str, bid: u64) -> String {
        format!("wire::barrier_cross(&mut {cv}, {bid})")
    }

    fn sock_setup_extra(_sock: &str) -> Option<String> {
        // Blocking transport — sockets stay blocking; no extra setup.
        None
    }

    fn so_buf_comment() -> &'static str {
        SO_BUF_COMMENT_BUFSYNC
    }

    fn relay_banner(pad: &str) -> String {
        // TASK-0327 star-topology wording. Emitted verbatim into the
        // generated host binary's relay phase (byte-identical to the
        // pre-lift bufsync emit).
        format!(
            "{pad}// TASK-0327 host-relay phase: forward worker-to-worker Push/Wait\n\
             {pad}// pairs through host's existing (data, ctrl)-pair-per-(host, worker)\n\
             {pad}// star topology. SYNCHRONOUS: read from data_<src>, write to data_<dst>,\n\
             {pad}// one (seq, dst) hop at a time, srcs iterated in sorted-WorkerId order."
        )
    }
}

/// mp-tcp-bufsync's concrete `Plan` — the shared generic substrate with
/// the blocking wire primitives bound in. `lib.rs` uses this exactly as
/// it used the old in-crate `Plan` (`Plan::build`, `.render_worker_program`,
/// `.render_run_sh`, `.used_workers`, `.worker_name`).
pub(crate) type Plan<'a> = backend_common::tcp_plan::Plan<'a, BufsyncWire>;
