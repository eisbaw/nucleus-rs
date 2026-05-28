//! mp-tcp-poll plan shim.
//!
//! The multi-process emit `Plan` substrate (host election, xfer
//! registry, slice-paste tile derivation, accumulator classification,
//! FIFO-shape hazards, the entire event walk, worker-program assembly,
//! and host-relay codegen) lives ONCE in
//! [`backend_common::tcp_plan`], parameterised over the
//! [`backend_common::tcp_plan::WirePrimitives`] trait (TASK-0044.02.03
//! lift). This file supplies only mp-tcp-poll's `WirePrimitives` impl —
//! the NONBLOCKING-POLL wire primitives plus the `apply_nonblocking`
//! per-socket setup line — and a `Plan` type alias the crate's
//! `lib.rs` consumes exactly as it did the old in-crate `Plan`.
//!
//! mp-tcp-bufsync is the sibling consumer; its shim
//! (`nucleus/backends/mp-tcp-bufsync/src/plan.rs`) supplies the
//! blocking variant. The two impls are the COMPLETE, enumerable delta
//! between the backends — everything else is shared.

use backend_common::tcp_plan::WirePrimitives;

use crate::SO_BUF_COMMENT_POLL;

/// mp-tcp-poll wire primitives: nonblocking poll + `yield_now` framing.
/// The three `*_call` methods build the `wire::..._poll` expression
/// STRUCTURALLY from their arguments (never by textual replace —
/// `read_msg_expect` is a substring of `read_msg_expect_poll`; see
/// `feedback-textual-replace-codegen-unsafe`).
pub(crate) struct PollWire;

impl WirePrimitives for PollWire {
    const BACKEND_NAME: &'static str = "mp-tcp-poll";
    const FILE_HEADER_BACKEND: &'static str = "mp-tcp-poll";
    const FILE_HEADER_TASK: &'static str = "TASK-0044.02.02";

    fn read_msg_expect_call(cv: &str, seq: u64) -> String {
        // POLL variant: nonblocking-read loop with yield_now per cycle
        // and a deadline-bound loud-failure panic on a never-sending
        // peer (AC#7 of TASK-0044.02.02). Contract otherwise identical
        // to read_msg_expect (seq-tag mismatch still panics, payload
        // returned verbatim).
        format!("wire::read_msg_expect_poll(&mut {cv}, {seq})")
    }

    fn write_msg_call(cv: &str, seq: u64, enc: &str) -> String {
        // POLL variant: write_msg_poll handles WouldBlock on the send
        // side (large-payload safety on a nonblocking socket). Same
        // wire bytes as write_msg.
        format!("wire::write_msg_poll(&mut {cv}, {seq}, &{enc})")
    }

    fn barrier_cross_call(cv: &str, bid: u64) -> String {
        format!("wire::barrier_cross_poll(&mut {cv}, {bid})")
    }

    fn sock_setup_extra(sock: &str) -> Option<String> {
        // mp-tcp-poll delta: BOTH directions of each socket go
        // nonblocking so the wire::*_poll helpers see WouldBlock and
        // can yield_now. Idempotent; loud-on-syscall-failure semantics
        // (see apply_nonblocking docstring). Emitted immediately after
        // `wire::apply_sock_buf(&{sock});`.
        Some(format!("wire::apply_nonblocking(&{sock});"))
    }

    fn so_buf_comment() -> &'static str {
        SO_BUF_COMMENT_POLL
    }

    fn relay_banner(pad: &str) -> String {
        // Cycle-195 poll-variant wording. Emitted verbatim into the
        // generated host binary's relay phase (byte-identical to the
        // pre-lift mp-tcp-poll emit).
        format!(
            "{pad}// mp-tcp-poll host-relay phase (sibling of bufsync TASK-0327): forward\n\
             {pad}// worker-to-worker Push/Wait pairs through host's existing star topology.\n\
             {pad}// SYNCHRONOUS with poll-variant wire helpers (nonblocking socket: poll-read +\n\
             {pad}// poll-write); one (seq, dst) hop at a time, srcs iterated in sorted-WorkerId\n\
             {pad}// order. seq cross-check via wire::read_msg_expect_poll preserves the\n\
             {pad}// fail-loud contract on Push/Wait pairing divergence."
        )
    }
}

/// mp-tcp-poll's concrete `Plan` — the shared generic substrate with
/// the nonblocking-poll wire primitives bound in. `lib.rs` uses this
/// exactly as it used the old in-crate `Plan`.
pub(crate) type Plan<'a> = backend_common::tcp_plan::Plan<'a, PollWire>;
