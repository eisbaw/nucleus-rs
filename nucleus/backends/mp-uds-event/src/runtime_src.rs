//! mp-uds-event runtime: mio reactor + per-(seq, peer) outbound ring
//! buffer + per-seq inbound queue. This file is the SINGLE SOURCE of
//! the runtime — `mp-uds-event`'s lib.rs `include_str!`s it and emits
//! it verbatim as `src/runtime.rs` in every generated multi-process
//! project (TASK-0044.03.01 cycle 197). Structural twin of
//! mp-tcp-event's `runtime_src.rs` — the only diff is the transport
//! layer (mio's `UnixStream` instead of `TcpStream`).
//!
//! # Contract (identical to mp-tcp-event's reactor at v0)
//!
//! - One [`Reactor`] per worker process, owning every DATA-channel
//!   peer socket plus the per-(seq, peer) outbound queues and the
//!   per-seq inbound queues.
//! - Boundedness: `Reactor::push(seq, peer, frame, cap)` blocks
//!   (driving the reactor) while the (seq, peer) outbound queue is
//!   `>= cap`. Receiver-side `Reactor::wait(seq)` blocks while the
//!   inbound queue for `seq` is empty.
//! - Demultiplexing: incoming frames are routed by the wire-level
//!   seq tag (`HEADER_LEN = 16`, layout `[len u64 LE][seq u64
//!   LE][payload]` — IDENTICAL to mp_tcp_common's TCP layout at v0).
//! - Barriers: NOT handled here. Barriers stay on the synchronous
//!   CTRL channel (`std::os::unix::net::UnixStream`), using
//!   `wire::barrier_cross`. CTRL framing is unchanged from the
//!   inlined UDS `wire.rs`. The reactor only touches the DATA
//!   channels.
//! - Fail-loud: every I/O error path `panic!`s naming the failing
//!   peer + the operation. No silent skip / retry beyond the
//!   `WouldBlock` non-blocking-readiness signal.

#![allow(dead_code, unused_imports)]

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Duration;

use mio::net::UnixStream as MioUds;
use mio::{Events, Interest, Poll, Token};

// HEADER_LEN drift between THIS file and the sibling `wire.rs` (the
// inlined UDS wire runtime) is pinned by `header_len_matches_wire_runtime`
// below. The wire byte layout is transport-agnostic at v0; if a future
// wire protocol bumps versions, this constant + sibling wire.rs's
// const + mp_tcp_common's TCP wire MUST bump together.
const HEADER_LEN: usize = 16;

/// Parse the `NUC_REACTOR_DEADLOCK_TIMEOUT_S` env value into a
/// `pump_once` poll timeout. Pulled into a free function so the parse
/// path is unit-testable without process-env mutation (set_var is
/// thread-unsafe; see Rust 2024 edition note). Semantics:
/// - `None` (env unset): default 30 s.
/// - `Some("0")`: watchdog disabled (None -> blocking poll).
/// - `Some(n)` where `n` parses as u64 > 0: that many seconds.
/// - `Some(garbage)`: default 30 s (silent fallback; env-knob abuse
///   should not crash the worker).
fn parse_deadlock_timeout(raw: Option<&str>) -> Option<Duration> {
    match raw {
        Some(s) => match s.parse::<u64>() {
            Ok(0) => None,
            Ok(n) => Some(Duration::from_secs(n)),
            Err(_) => Some(Duration::from_secs(30)),
        },
        None => Some(Duration::from_secs(30)),
    }
}

// ---------------------------------------------------------------------
// Reactor: the per-process mio Poll wrapper.
// ---------------------------------------------------------------------

/// Read-side framing state per peer socket.
#[derive(Debug)]
enum ReadPhase {
    /// Reading the 16-byte header.
    Header,
    /// Reading `len` payload bytes; we already parsed (len, seq) from
    /// the header.
    Payload { len: usize, seq: u64 },
}

/// One DATA-channel peer socket plus its per-peer framing state.
struct PeerSock {
    /// The mio-managed non-blocking UDS socket.
    sock: MioUds,
    /// Mio token for poll-readiness events. Index into `peers` <<1.
    token: Token,
    /// Interest currently registered with the poll. Toggled between
    /// READABLE and READABLE|WRITABLE depending on outbound backlog.
    interest: Interest,
    /// Read framing state.
    read_phase: ReadPhase,
    /// Buffer holding the bytes-so-far of the in-progress read frame.
    /// Sized to either HEADER_LEN (Header phase) or `len` (Payload).
    read_buf: Vec<u8>,
    /// Number of bytes already read into `read_buf`.
    read_pos: usize,
    /// Bytes pending to write for the head-of-queue outbound frame.
    /// Empty when the head queue is fully sent OR the queue is empty.
    pending_write: Vec<u8>,
    /// Number of bytes already written from `pending_write`.
    pending_pos: usize,
    /// Friendly name used in panic messages (e.g. "w0", "host").
    name: String,
}

impl PeerSock {
    fn new(sock: MioUds, token: Token, name: String) -> Self {
        PeerSock {
            sock,
            token,
            interest: Interest::READABLE,
            read_phase: ReadPhase::Header,
            read_buf: vec![0u8; HEADER_LEN],
            read_pos: 0,
            pending_write: Vec::new(),
            pending_pos: 0,
            name,
        }
    }
}

/// One per worker process. Owns the mio Poll + every DATA socket +
/// per-(seq, peer) outbound queues + per-seq inbound queues.
///
/// The user-level `Chan<T>` borrows the Reactor mutably via
/// `Rc<RefCell<_>>`; one Reactor instance is shared across every
/// Chan<T> in the worker's `main()`.
pub struct Reactor {
    poll: Poll,
    events: Events,
    /// One per DATA peer socket. Vec because we need stable indexing
    /// from outside (the peer_idx encoded in tokens).
    peers: Vec<PeerSock>,
    /// Inbound queues — incoming frames demuxed by seq.
    inbound: BTreeMap<u64, VecDeque<Vec<u8>>>,
    /// Outbound queues — per (seq, peer_idx). The head-of-queue frame
    /// for each peer is serialised into `peers[peer_idx].pending_write`
    /// when the previous head fully sent.
    outbound: BTreeMap<(u64, usize), VecDeque<Vec<u8>>>,
    /// Deadlock-watchdog timeout passed to `poll(timeout)`. Default
    /// 30 s; configurable via `NUC_REACTOR_DEADLOCK_TIMEOUT_S`. Set
    /// to 0 to disable the watchdog entirely.
    deadlock_timeout: Option<Duration>,
}

impl Reactor {
    /// Build a Reactor with `peers.len()` peer sockets pre-registered.
    /// `peers` is consumed; each socket is set non-blocking and
    /// registered for READABLE.
    pub fn new(peers: Vec<(MioUds, String)>) -> Self {
        let poll =
            Poll::new().unwrap_or_else(|e| panic!("mp-uds-event: mio::Poll::new failed: {e}"));
        let events = Events::with_capacity(256);
        let mut wrapped = Vec::with_capacity(peers.len());
        for (idx, (mut sock, name)) in peers.into_iter().enumerate() {
            let token = Token(idx * 2);
            poll.registry()
                .register(&mut sock, token, Interest::READABLE)
                .unwrap_or_else(|e| {
                    panic!("mp-uds-event: poll.register peer `{name}` (token {idx}) failed: {e}")
                });
            wrapped.push(PeerSock::new(sock, token, name));
        }
        let deadlock_timeout = parse_deadlock_timeout(
            std::env::var("NUC_REACTOR_DEADLOCK_TIMEOUT_S")
                .ok()
                .as_deref(),
        );
        Reactor {
            poll,
            events,
            peers: wrapped,
            inbound: BTreeMap::new(),
            outbound: BTreeMap::new(),
            deadlock_timeout,
        }
    }

    /// Enqueue a frame for sending on `(seq, peer)`. Blocks (driving
    /// the reactor) while the outbound queue is `>= cap`. After
    /// enqueueing, OPPORTUNISTICALLY drains as much as the kernel will
    /// accept without blocking.
    pub fn push(&mut self, seq: u64, peer: usize, payload: Vec<u8>, cap: usize) {
        // 1. Back-pressure: pump until the queue has room.
        while self
            .outbound
            .get(&(seq, peer))
            .map(|q| q.len())
            .unwrap_or(0)
            >= cap
        {
            self.pump_once();
        }
        // 2. Enqueue.
        let q = self.outbound.entry((seq, peer)).or_default();
        q.push_back(payload);
        // 3. If the peer's pending_write is empty, serialise the new
        //    head-of-queue (across ALL seqs for this peer, in seq order).
        if self.peers[peer].pending_write.is_empty() {
            self.try_load_next_for_peer(peer);
        }
        // 4. Opportunistic non-blocking write.
        self.drain_writes(peer);
        // 5. Toggle WRITABLE interest if anything is still pending.
        self.refresh_interest(peer);
    }

    /// Block until an inbound frame for `seq` is available; return
    /// its payload. Drives the reactor while waiting.
    pub fn wait(&mut self, seq: u64) -> Vec<u8> {
        loop {
            if let Some(q) = self.inbound.get_mut(&seq) {
                if let Some(payload) = q.pop_front() {
                    return payload;
                }
            }
            self.pump_once();
        }
    }

    /// One reactor turn: poll for readiness then drain socket events.
    ///
    /// Uses `self.deadlock_timeout`. Steady-state UDS loopback is
    /// always ready when we have outstanding work — a timeout is
    /// genuinely deadlock evidence (or a crashed peer).
    fn pump_once(&mut self) {
        self.poll
            .poll(&mut self.events, self.deadlock_timeout)
            .unwrap_or_else(|e| panic!("mp-uds-event: poll() failed: {e}"));
        if self.events.is_empty() {
            if self.deadlock_timeout.is_none() {
                panic!(
                    "mp-uds-event: blocking poll() returned zero events; \
                     mio API contract violated."
                );
            }
            let secs = self.deadlock_timeout.unwrap().as_secs();
            panic!(
                "mp-uds-event: reactor poll timed out — no readiness on \
                 any peer within {secs}s. Either the schedule is dead-locked, \
                 a peer worker exited unexpectedly, or a Fire-side kernel \
                 legitimately exceeds {secs}s without intermediate Push. \
                 Bump NUC_REACTOR_DEADLOCK_TIMEOUT_S to extend (0 = \
                 disable; default 30)."
            );
        }
        let snap: Vec<(Token, bool, bool)> = self
            .events
            .iter()
            .map(|e| (e.token(), e.is_readable(), e.is_writable()))
            .collect();
        for (tok, readable, writable) in snap {
            let peer = tok.0 / 2;
            if peer >= self.peers.len() {
                continue;
            }
            if readable {
                self.drain_reads(peer);
            }
            if writable {
                self.drain_writes(peer);
            }
            self.refresh_interest(peer);
        }
    }

    /// Read as much as the socket has, parsing whole frames into the
    /// inbound queues. Returns when the socket would block.
    fn drain_reads(&mut self, peer: usize) {
        loop {
            let read_result = {
                let p = &mut self.peers[peer];
                let pos = p.read_pos;
                let cap = p.read_buf.len();
                if pos >= cap {
                    p.read_pos = 0;
                    continue;
                }
                let slice = &mut p.read_buf[pos..cap];
                p.sock.read(slice)
            };
            match read_result {
                Ok(0) => {
                    let p = &self.peers[peer];
                    if p.read_pos > 0 {
                        panic!(
                            "mp-uds-event: peer `{}` closed mid-frame (read_phase={:?}, \
                             read_pos={}/{})",
                            p.name,
                            p.read_phase,
                            p.read_pos,
                            p.read_buf.len()
                        );
                    }
                    return;
                }
                Ok(n) => {
                    let (completed_payload, phase_swap): (Option<(u64, Vec<u8>)>, _) = {
                        let p = &mut self.peers[peer];
                        p.read_pos += n;
                        if p.read_pos < p.read_buf.len() {
                            (None, false)
                        } else {
                            match p.read_phase {
                                ReadPhase::Header => {
                                    let len =
                                        u64::from_le_bytes(p.read_buf[0..8].try_into().unwrap())
                                            as usize;
                                    let seq =
                                        u64::from_le_bytes(p.read_buf[8..16].try_into().unwrap());
                                    if len == 0 {
                                        panic!(
                                            "mp-uds-event: peer `{}` sent zero-payload \
                                             DATA frame (seq={seq}); barriers must use \
                                             the CTRL channel via wire::barrier_cross",
                                            p.name
                                        );
                                    }
                                    p.read_phase = ReadPhase::Payload { len, seq };
                                    p.read_buf = vec![0u8; len];
                                    p.read_pos = 0;
                                    (None, false)
                                }
                                ReadPhase::Payload { len: _, seq } => {
                                    let payload = std::mem::take(&mut p.read_buf);
                                    (Some((seq, payload)), true)
                                }
                            }
                        }
                    };
                    if let Some((seq, payload)) = completed_payload {
                        self.inbound.entry(seq).or_default().push_back(payload);
                    }
                    if phase_swap {
                        let p = &mut self.peers[peer];
                        p.read_phase = ReadPhase::Header;
                        p.read_buf = vec![0u8; HEADER_LEN];
                        p.read_pos = 0;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    return;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(e) => {
                    let name = &self.peers[peer].name;
                    panic!("mp-uds-event: read from peer `{name}` failed: {e}");
                }
            }
        }
    }

    /// Drain `pending_write` then load the next queued frame; loop
    /// until the socket would block or every outbound queue for this
    /// peer is empty.
    fn drain_writes(&mut self, peer: usize) {
        loop {
            let p = &mut self.peers[peer];
            if p.pending_write.is_empty() {
                if !self.try_load_next_for_peer(peer) {
                    return;
                }
                continue;
            }
            let p = &mut self.peers[peer];
            let pos = p.pending_pos;
            let slice = &p.pending_write[pos..];
            match p.sock.write(slice) {
                Ok(0) => {
                    panic!(
                        "mp-uds-event: write to peer `{}` returned 0 bytes \
                         (kernel refused)",
                        p.name
                    );
                }
                Ok(n) => {
                    p.pending_pos += n;
                    if p.pending_pos == p.pending_write.len() {
                        p.pending_write.clear();
                        p.pending_pos = 0;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    return;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(e) => {
                    panic!("mp-uds-event: write to peer `{}` failed: {e}", p.name);
                }
            }
        }
    }

    /// Find the lowest-seq outbound queue with a head frame for
    /// `peer`, pop it, serialise to `peers[peer].pending_write`.
    fn try_load_next_for_peer(&mut self, peer: usize) -> bool {
        let mut chosen: Option<u64> = None;
        for (&(seq, p), q) in self.outbound.iter() {
            if p == peer && !q.is_empty() {
                chosen = Some(seq);
                break;
            }
        }
        let Some(seq) = chosen else {
            return false;
        };
        let payload = self
            .outbound
            .get_mut(&(seq, peer))
            .and_then(|q| q.pop_front())
            .expect("non-empty queue head went missing");
        let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
        frame.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        frame.extend_from_slice(&seq.to_le_bytes());
        frame.extend_from_slice(&payload);
        let p = &mut self.peers[peer];
        p.pending_write = frame;
        p.pending_pos = 0;
        true
    }

    /// Update the poll-registered interest for `peer`.
    fn refresh_interest(&mut self, peer: usize) {
        let has_out = !self.peers[peer].pending_write.is_empty()
            || self
                .outbound
                .iter()
                .any(|(&(_, p), q)| p == peer && !q.is_empty());
        let want = if has_out {
            Interest::READABLE | Interest::WRITABLE
        } else {
            Interest::READABLE
        };
        let p = &mut self.peers[peer];
        if p.interest != want {
            self.poll
                .registry()
                .reregister(&mut p.sock, p.token, want)
                .unwrap_or_else(|e| {
                    panic!(
                        "mp-uds-event: reregister peer `{}` (token {:?}) failed: {e}",
                        p.name, p.token
                    )
                });
            p.interest = want;
        }
    }

    /// Host-relay primitive for worker-to-worker `Push`/`Wait` on the
    /// UDS-star topology. Drains one frame from `inbound[seq]` and
    /// re-enqueues it to `outbound[(seq, dst_peer)]`. Wire-level seq
    /// tag is preserved verbatim so dst's `chan_<rid>.wait()` sees
    /// the same frame src pushed. `dst_peer` is the dst worker's
    /// index in HOST's reactor.
    ///
    /// Used ONLY by the host's `main()` relay phase, inline-emitted
    /// by `multi_worker::Plan::render_relay_phase`. Same per-seq-demux
    /// safety profile as mp-tcp-event (memory
    /// `project-mp-tcp-event-vs-bufsync-safety-profile`): a contract-
    /// violating peer that produces a seq the schedule did not
    /// allocate would surface as a silent wrong-payload race rather
    /// than the fail-loud seq-mismatch a per-pair-FIFO backend gets.
    pub fn relay_one(&mut self, seq: u64, dst_peer: usize, cap: usize) {
        let payload = self.wait(seq);
        self.push(seq, dst_peer, payload, cap);
    }

    /// After the schedule's final Push, the worker should drain any
    /// outbound bytes still in flight before exiting.
    pub fn flush_outbound(&mut self) {
        loop {
            let any_pending = self.peers.iter().any(|p| !p.pending_write.is_empty())
                || self.outbound.values().any(|q| !q.is_empty());
            if !any_pending {
                return;
            }
            self.pump_once();
        }
    }
}

// ---------------------------------------------------------------------
// Chan<T>: typed user-level channel handle.
// ---------------------------------------------------------------------

pub struct Chan<T> {
    reactor: Rc<RefCell<Reactor>>,
    seq: u64,
    peer: usize,
    cap: usize,
    encode: fn(&T) -> Vec<u8>,
    decode: fn(&[u8]) -> T,
    _marker: PhantomData<T>,
}

impl<T> Chan<T> {
    pub fn new(
        reactor: Rc<RefCell<Reactor>>,
        seq: u64,
        peer: usize,
        cap: usize,
        encode: fn(&T) -> Vec<u8>,
        decode: fn(&[u8]) -> T,
    ) -> Self {
        Chan {
            reactor,
            seq,
            peer,
            cap,
            encode,
            decode,
            _marker: PhantomData,
        }
    }

    pub fn push(&self, v: T) {
        let buf = (self.encode)(&v);
        self.reactor
            .borrow_mut()
            .push(self.seq, self.peer, buf, self.cap);
    }

    pub fn wait(&self) -> T {
        let buf = self.reactor.borrow_mut().wait(self.seq);
        (self.decode)(&buf)
    }
}

// =====================================================================
// Host-side compile-check tests (cycle-197 F1-style guard).
// =====================================================================

#[cfg(test)]
impl Reactor {
    fn peers_is_empty(&self) -> bool {
        self.peers.is_empty()
    }
    fn inbound_is_empty(&self) -> bool {
        self.inbound.is_empty()
    }
    fn outbound_is_empty(&self) -> bool {
        self.outbound.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::HEADER_LEN;

    /// HEADER_LEN drift between THIS file's UDS reactor and the
    /// sibling UDS `wire.rs` runtime is the load-bearing invariant
    /// the file-top comment promises. The wire-runtime const is
    /// public (re-exported via mp-uds-event's lib for the
    /// `wire_runtime::tests` module).
    #[test]
    fn header_len_matches_wire_runtime() {
        // Re-derive the sibling const by string-scan (mirrors
        // mp-tcp-event's host-test pattern; the wire crate keeps
        // `HEADER_LEN` public-shaped but we re-derive to defend
        // against accidental shadowing). `crate::` resolves to the
        // mp-uds-event lib because runtime_src.rs is included as
        // `#[cfg(test)] mod runtime_src;` inside lib.rs.
        let src = crate::WIRE_RUNTIME_SRC;
        let needle = "pub const HEADER_LEN: usize = ";
        let i = src
            .find(needle)
            .expect("UDS wire_runtime.rs lost its `pub const HEADER_LEN: usize = ` declaration");
        let rest = &src[i + needle.len()..];
        let end = rest
            .find(';')
            .expect("UDS wire_runtime.rs HEADER_LEN declaration missing terminator");
        let wire_header_len: usize = rest[..end]
            .trim()
            .parse()
            .expect("UDS wire_runtime.rs HEADER_LEN value did not parse as usize");
        assert_eq!(
            wire_header_len, HEADER_LEN,
            "UDS wire HEADER_LEN (={wire_header_len}) and UDS reactor HEADER_LEN \
             (={HEADER_LEN}) drifted. Bump both together; see wire-protocol-v0.md."
        );
    }

    /// `Reactor::new` builds with zero peers and exposes an empty
    /// inbound + outbound map.
    #[test]
    fn reactor_new_empty_compiles_and_runs() {
        let r = super::Reactor::new(Vec::new());
        assert!(r.peers_is_empty());
        assert!(r.inbound_is_empty());
        assert!(r.outbound_is_empty());
    }

    /// Parse-path semantics of NUC_REACTOR_DEADLOCK_TIMEOUT_S.
    #[test]
    fn deadlock_timeout_parse_paths() {
        use std::time::Duration;
        assert_eq!(
            super::parse_deadlock_timeout(None),
            Some(Duration::from_secs(30))
        );
        assert_eq!(super::parse_deadlock_timeout(Some("0")), None);
        assert_eq!(
            super::parse_deadlock_timeout(Some("120")),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            super::parse_deadlock_timeout(Some("not-a-number")),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            super::parse_deadlock_timeout(Some("")),
            Some(Duration::from_secs(30))
        );
    }
}
