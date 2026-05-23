//! mp-tcp-event runtime: mio reactor + per-(seq, peer) outbound ring
//! buffer + per-seq inbound queue. This file is the SINGLE SOURCE of
//! the runtime — `mp-tcp-event`'s lib.rs `include_str!`s it and emits
//! it verbatim as `src/runtime.rs` in every generated multi-process
//! project, exactly the way `mp-tcp-common::wire_runtime.rs` carries
//! the wire codec (TASK-0042.05 / Stage 3 of TASK-0042.02).
//!
//! # Contract
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
//!   LE][payload]`) into per-seq queues, NOT per-peer. Two Push
//!   events from different peers with the same seq cannot exist by
//!   construction — `(DataId, SeqTag)` uniquely identifies one
//!   transfer pair, and `seq` is globally unique per Push/Wait pair.
//! - Barriers: NOT handled here. Barriers stay on the synchronous
//!   CTRL channel (`std::net::TcpStream`), using
//!   `wire::barrier_cross`. CTRL framing is unchanged from
//!   mp-tcp-bufsync. The reactor only touches the DATA channels.
//! - Fail-loud: every I/O error path `panic!`s naming the failing
//!   peer + the operation. No silent skip / retry beyond the
//!   `WouldBlock` non-blocking-readiness signal.
//!
//! # Why not split into separate ring + reactor + chan files
//!
//! The three types are intimately coupled (Chan<T> borrows Reactor
//! mutably through Rc<RefCell<_>>; Reactor's queue maps are keyed on
//! the same `seq` Chan<T> carries). Keeping them in one file is the
//! same precedent `wire_runtime.rs` set — one source-of-truth file,
//! emitted verbatim, no inter-file include order to maintain at the
//! emit site.

#![allow(dead_code, unused_imports)]

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Duration;

use mio::net::TcpStream as MioTcp;
use mio::{Events, Interest, Poll, Token};

// We deliberately do NOT `use super::wire;` here. The sibling
// `src/wire.rs` (`mp_tcp_common::WIRE_RUNTIME_SRC`) keeps its
// `HEADER_LEN` private (it's a wire-protocol internal — the public
// surface is the `write_msg` / `read_msg` / `barrier_cross` /
// `enc_*` / `dec_*` functions). The reactor needs the same 16-byte
// header invariant, so we re-state it here as a file-local constant
// with a cross-reference. If the wire protocol ever bumps versions
// (currently v0; see docs/wire-protocol-v0.md), BOTH this constant
// and `mp_tcp_common::wire_runtime`'s `HEADER_LEN` change together
// — a single test in `mp-tcp-common` and a build-time check in this
// crate pin that pair. Drift here would surface immediately as a
// framing mis-parse on the first cross-worker push.
const HEADER_LEN: usize = 16;

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
    /// The mio-managed non-blocking TCP socket.
    sock: MioTcp,
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
    /// Bytes pending to write for the head-of-queue outbound frame
    /// (the frame's serialised `[header || payload]`, not consumed
    /// yet). Empty when the head queue is fully sent OR the queue is
    /// empty.
    pending_write: Vec<u8>,
    /// Number of bytes already written from `pending_write`.
    pending_pos: usize,
    /// Friendly name used in panic messages (e.g. "w0", "host").
    name: String,
}

impl PeerSock {
    fn new(sock: MioTcp, token: Token, name: String) -> Self {
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
    /// when the previous head fully sent; while `pending_write` is
    /// non-empty the queue's head is "in-flight" but not yet ACKed by
    /// the kernel.
    outbound: BTreeMap<(u64, usize), VecDeque<Vec<u8>>>,
}

impl Reactor {
    /// Build a Reactor with `peers.len()` peer sockets pre-registered.
    /// `peers` is consumed; each socket is set non-blocking and
    /// registered for READABLE.
    pub fn new(peers: Vec<(MioTcp, String)>) -> Self {
        let poll =
            Poll::new().unwrap_or_else(|e| panic!("mp-tcp-event: mio::Poll::new failed: {e}"));
        let events = Events::with_capacity(256);
        let mut wrapped = Vec::with_capacity(peers.len());
        for (idx, (mut sock, name)) in peers.into_iter().enumerate() {
            let token = Token(idx * 2);
            poll.registry()
                .register(&mut sock, token, Interest::READABLE)
                .unwrap_or_else(|e| {
                    panic!("mp-tcp-event: poll.register peer `{name}` (token {idx}) failed: {e}")
                });
            wrapped.push(PeerSock::new(sock, token, name));
        }
        Reactor {
            poll,
            events,
            peers: wrapped,
            inbound: BTreeMap::new(),
            outbound: BTreeMap::new(),
        }
    }

    /// Enqueue a frame for sending on `(seq, peer)`. Blocks (driving
    /// the reactor) while the outbound queue is `>= cap` — this is
    /// the bounded-buffer back-pressure point. After enqueueing,
    /// OPPORTUNISTICALLY drains as much as the kernel will accept
    /// without blocking (no `poll()` round-trip; just a direct
    /// non-blocking write). The reason this is here (rather than
    /// waiting for the next reactor trip):
    ///
    /// Between Push events the user code may execute arbitrarily
    /// long stretches without invoking `push` or `wait` (e.g. a
    /// compute loop guarded only by `bar.wait()` barriers, which
    /// drive the CTRL channel — not the DATA reactor). If push
    /// returned immediately leaving frames queued, the consumer side
    /// would be parked on `wait()` polling its READABLE socket
    /// against a peer that is silently holding the data. Draining
    /// post-enqueue makes the steady-state guarantee local:
    /// "every push observed by the producer either reaches the
    /// kernel send buffer, or the kernel refused (WouldBlock) and
    /// the reactor still owns the bytes — fail-loud diagnostics on
    /// next push/wait".
    ///
    /// Does NOT block for ACK; the kernel can buffer the frame
    /// indefinitely. The next push or wait will resume the drain via
    /// `pump_once` if WouldBlock fired here.
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
        //    head-of-queue (across ALL seqs for this peer, in seq
        //    order — BTreeMap is ascending, so we pick the smallest
        //    seq with a non-empty queue for this peer).
        if self.peers[peer].pending_write.is_empty() {
            self.try_load_next_for_peer(peer);
        }
        // 4. Opportunistic non-blocking write. Steady-state on
        //    loopback, this writes the entire frame and clears the
        //    queue; under back-pressure it short-circuits on
        //    WouldBlock and the next pump_once handles the rest.
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
    /// We use a small timeout (rather than indefinite) so a buggy
    /// schedule can't park the process forever — though steady-state
    /// the loopback is always ready when we have outstanding work.
    fn pump_once(&mut self) {
        // Always swap to a fresh events buffer to avoid lingering
        // unprocessed events; `Events` clear themselves before poll.
        self.poll
            .poll(&mut self.events, Some(Duration::from_secs(30)))
            .unwrap_or_else(|e| panic!("mp-tcp-event: poll() failed: {e}"));
        if self.events.is_empty() {
            // Liveness watchdog: 30 s with no readiness on a loopback
            // schedule is a deadlock (or the peer crashed). Fail loud.
            panic!(
                "mp-tcp-event: reactor poll timed out — no readiness on \
                 any peer within 30s. Either the schedule is dead-locked \
                 or a peer worker exited unexpectedly."
            );
        }
        // Snapshot the events into a local Vec so we can mutate
        // `self.peers` / `self.outbound` during processing without
        // aliasing `self.events`.
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
                    // Defensive: should have advanced via promote phase.
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
                            "mp-tcp-event: peer `{}` closed mid-frame (read_phase={:?}, \
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
                    // Update read_pos; if a frame portion completed,
                    // promote the phase (and possibly deliver into the
                    // inbound queues).
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
                                            "mp-tcp-event: peer `{}` sent zero-payload \
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
                    panic!("mp-tcp-event: read from peer `{name}` failed: {e}");
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
                // Try to load another frame from one of this peer's
                // outbound queues (lowest seq first — deterministic).
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
                        "mp-tcp-event: write to peer `{}` returned 0 bytes \
                         (kernel refused)",
                        p.name
                    );
                }
                Ok(n) => {
                    p.pending_pos += n;
                    if p.pending_pos == p.pending_write.len() {
                        // Head fully written; clear and loop to load
                        // the next queued frame.
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
                    panic!("mp-tcp-event: write to peer `{}` failed: {e}", p.name);
                }
            }
        }
    }

    /// Find the lowest-seq outbound queue with a head frame for
    /// `peer`, pop it, serialise to `peers[peer].pending_write`.
    /// Returns true if something was loaded.
    fn try_load_next_for_peer(&mut self, peer: usize) -> bool {
        // Find the smallest seq with a non-empty queue for this peer.
        let mut chosen: Option<u64> = None;
        for (&(seq, p), q) in self.outbound.iter() {
            if p == peer && !q.is_empty() {
                chosen = Some(seq);
                break; // BTreeMap iterates ascending; first match wins.
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
        // Serialise [len LE][seq LE][payload].
        let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
        frame.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        frame.extend_from_slice(&seq.to_le_bytes());
        frame.extend_from_slice(&payload);
        let p = &mut self.peers[peer];
        p.pending_write = frame;
        p.pending_pos = 0;
        true
    }

    /// Update the poll-registered interest for `peer` based on whether
    /// we currently have any outbound work (pending_write OR any
    /// non-empty queue).
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
                        "mp-tcp-event: reregister peer `{}` (token {:?}) failed: {e}",
                        p.name, p.token
                    )
                });
            p.interest = want;
        }
    }

    /// After the schedule's final Push, the worker should drain any
    /// outbound bytes still in flight before exiting; otherwise a
    /// fast-exit worker can drop the last frames the kernel hasn't
    /// shipped yet. Called from each worker's `main()` after the
    /// shared walker output.
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

/// Typed channel handle for one `(DataId, SeqTag)` pair. Each instance
/// carries the encode/decode function pair for its element type and
/// the (seq, peer) coordinates the reactor demuxes on. `push` and
/// `wait` borrow the reactor mutably for the duration of the call
/// (RefCell single-threaded — every worker process has exactly one
/// thread in mp-tcp-event, so no `RefCell::borrow_mut` overlap is
/// possible).
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
