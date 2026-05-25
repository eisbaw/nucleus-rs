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
// and `mp_tcp_common::wire_runtime`'s `HEADER_LEN` change together.
// HEADER_LEN drift between THIS file and the wire codec is pinned by
// the `runtime_src_compile_check::header_len_matches_wire_runtime`
// test below (architect-review F1 of TASK-0042.05). Framing
// mis-parse on the first cross-worker push is then prevented at the
// host crate's `cargo test` gate, not deferred to a generated-project
// e2e cell.
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
    /// Deadlock-watchdog timeout passed to `poll(timeout)`. If poll
    /// returns with zero readiness events, the reactor panics with a
    /// deadlock diagnostic. Default 30 s; configurable via
    /// `NUC_REACTOR_DEADLOCK_TIMEOUT_S` (env, read once at `new()`).
    /// Set to 0 to disable the watchdog entirely (the poll will then
    /// block indefinitely; useful for kernels whose single iteration
    /// genuinely exceeds 30 s and where a true deadlock is detected
    /// by an outer harness). Architect-review F2 of TASK-0042.05.
    deadlock_timeout: Option<Duration>,
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
        // Read NUC_REACTOR_DEADLOCK_TIMEOUT_S once at construction
        // (a missing / malformed env var falls back to default 30 s;
        // 0 disables the watchdog — None -> blocking poll). Routed
        // through `parse_deadlock_timeout` so the parse path is unit-
        // testable without touching process env.
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
    /// Uses `self.deadlock_timeout` (read from
    /// `NUC_REACTOR_DEADLOCK_TIMEOUT_S` at `Reactor::new`, default 30
    /// s; 0 disables the watchdog). Steady-state the loopback is
    /// always ready when we have outstanding work — a timeout is
    /// genuinely deadlock evidence (or a crashed peer), but a kernel
    /// that legitimately holds the producer for > timeout seconds
    /// without pushing intermediate frames would also trip it. Bump
    /// the env var for those cases; set it to 0 if the deadlock
    /// surface is owned by an outer harness.
    fn pump_once(&mut self) {
        // Always swap to a fresh events buffer to avoid lingering
        // unprocessed events; `Events` clear themselves before poll.
        self.poll
            .poll(&mut self.events, self.deadlock_timeout)
            .unwrap_or_else(|e| panic!("mp-tcp-event: poll() failed: {e}"));
        if self.events.is_empty() {
            if self.deadlock_timeout.is_none() {
                // Watchdog disabled (NUC_REACTOR_DEADLOCK_TIMEOUT_S=0).
                // A blocking poll returning zero events is impossible
                // unless the kernel/mio API misbehaved — treat as a
                // hard internal error rather than silently looping.
                panic!(
                    "mp-tcp-event: blocking poll() returned zero events; \
                     mio API contract violated."
                );
            }
            let secs = self.deadlock_timeout.unwrap().as_secs();
            // Liveness watchdog: zero readiness on a loopback schedule
            // within `secs` is a deadlock (or the peer crashed). Fail
            // loud naming the env knob so an operator can extend it.
            panic!(
                "mp-tcp-event: reactor poll timed out — no readiness on \
                 any peer within {secs}s. Either the schedule is dead-locked, \
                 a peer worker exited unexpectedly, or a Fire-side kernel \
                 legitimately exceeds {secs}s without intermediate Push. \
                 Bump NUC_REACTOR_DEADLOCK_TIMEOUT_S to extend (0 = \
                 disable; default 30)."
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

    /// TASK-0327 (cycle 149): host-relay primitive for
    /// worker-to-worker `Push`/`Wait` on the star topology. Drains one
    /// frame from `inbound[seq]` (received from the src worker via
    /// host's `data_<src>` reactor socket) and re-enqueues it to
    /// `outbound[(seq, dst_peer)]` (forwarded to the dst worker via
    /// host's `data_<dst>` socket). The wire-level seq tag is preserved
    /// verbatim so the dst worker's `chan_<rid>.wait()` (which demuxes
    /// on `inbound[seq]`) sees the same frame the src worker pushed.
    /// `dst_peer` is the dst worker's index in HOST's reactor (its
    /// position in the `non_host_workers` Vec).
    ///
    /// `cap` is the per-pair outbound bound (= `chan_caps[(data, seq)]`
    /// from `Plan`); back-pressure on the dst socket's outbound queue
    /// applies as in any other `push`. Bypasses the typed `Chan<T>`
    /// encode/decode — the payload is bytes-verbatim, the wire codec
    /// is symmetric for w2w.
    ///
    /// Used ONLY by the host's `main()` relay phase, inline-emitted by
    /// `multi_worker::Plan::render_relay_phase`. Non-host workers
    /// never call this — they `push` via their own `chan_<rid>`
    /// (peer_idx = 0 = host) and `wait` via `chan_<rid>.wait()` (reads
    /// `inbound[seq]` on their reactor's `data_host` socket, which
    /// receives whatever host has forwarded).
    ///
    /// ## Safety invariant (cycle-149 architect P2.4 fold-back)
    ///
    /// Unlike mp-tcp-bufsync's cycle-148 relay (which uses
    /// `wire::read_msg_expect(data_<src>, seq)` and panics on a seq
    /// mismatch at the wire layer), this primitive's `wait(seq)` is
    /// per-seq-demuxed and will succeed as long as ANY frame for
    /// `seq` has arrived in `inbound[seq]` — even, in principle, one
    /// produced by a peer other than the intended src. The reason
    /// that is SAFE today is the wire-protocol-v0 invariant: `seq`
    /// is globally unique per Push/Wait pair (see
    /// `mp_tcp_common::WIRE_RUNTIME_SRC` `HEADER_LEN`/seq-tag
    /// contract + the cycle-149 [`Plan::build`] uniqueness check via
    /// `chan_ids`). A future schedule that allowed two distinct Push
    /// events to share a `seq` would surface here as a silent
    /// wrong-payload race rather than the fail-loud seq-mismatch
    /// bufsync gets — file a typed defensive check at `Plan::build`
    /// if that contract ever weakens.
    pub fn relay_one(&mut self, seq: u64, dst_peer: usize, cap: usize) {
        let payload = self.wait(seq);
        self.push(seq, dst_peer, payload, cap);
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

// =====================================================================
// Host-side compile-check tests (architect-review F1 of TASK-0042.05).
// =====================================================================
//
// These only build under the host crate's `#[cfg(test)] mod
// runtime_src;` declaration in `lib.rs`. The generated project's copy
// of this file (`src/runtime.rs` in the emitted Cargo project) is NOT
// compiled with `cfg(test)`, so these tests do not bleed into emitted
// code — they exist purely to give `cargo test --workspace` real
// coverage of the reactor source, instead of the previous deferred-
// to-runtime-of-a-generated-project story.

// Test-only accessors. Gated behind `cfg(test)` so the emitted
// `src/runtime.rs` in a generated project does NOT export these — they
// are host-test-only and would otherwise widen the reactor's public
// surface unnecessarily. Declared BEFORE `mod tests` per
// clippy::items-after-test-module.
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

    /// HEADER_LEN drift between THIS file's reactor and
    /// `mp_tcp_common::wire_runtime`'s codec is the load-bearing
    /// invariant the file-top comment promises. The wire-runtime
    /// constant is private (file-local), so we re-derive it by
    /// scanning the public `WIRE_RUNTIME_SRC` string for the
    /// authoritative declaration. The pair MUST stay in lockstep; if
    /// the wire protocol bumps versions, both sites change together.
    #[test]
    fn header_len_matches_wire_runtime() {
        let needle = "const HEADER_LEN: usize = ";
        let src = mp_tcp_common::WIRE_RUNTIME_SRC;
        let i = src
            .find(needle)
            .expect("wire_runtime.rs lost its `const HEADER_LEN: usize = ` declaration");
        let rest = &src[i + needle.len()..];
        let end = rest
            .find(';')
            .expect("wire_runtime.rs HEADER_LEN declaration missing terminator");
        let wire_header_len: usize = rest[..end]
            .trim()
            .parse()
            .expect("wire_runtime.rs HEADER_LEN value did not parse as usize");
        assert_eq!(
            wire_header_len, HEADER_LEN,
            "wire-codec HEADER_LEN (={wire_header_len}) and reactor HEADER_LEN (={HEADER_LEN}) \
             drifted. Bump both together; see wire-protocol-v0.md."
        );
    }

    /// `Reactor::new` builds with zero peers and exposes an empty
    /// inbound + outbound map. Compile-checks the public construction
    /// path under host test (the file's main use-site is the emitted
    /// `src/runtime.rs` of a generated project, which won't be
    /// exercised at host test time).
    #[test]
    fn reactor_new_empty_compiles_and_runs() {
        let r = super::Reactor::new(Vec::new());
        // Nothing to poll; we don't call `pump_once` because that has
        // a deadlock watchdog and zero peers would trip it. The mere
        // fact that `Reactor::new(vec![])` compiles + executes
        // exercises mio's `Poll::new` + `Events::with_capacity` paths
        // without a generated project in the loop.
        assert!(r.peers_is_empty());
        assert!(r.inbound_is_empty());
        assert!(r.outbound_is_empty());
    }

    /// `NUC_REACTOR_DEADLOCK_TIMEOUT_S` parse path semantics
    /// (architect-review F2 of TASK-0042.05). Tests the helper
    /// directly so we don't have to mutate process env — `set_var`
    /// is thread-unsafe and being marked `unsafe fn` in Rust 2024.
    #[test]
    fn deadlock_timeout_parse_paths() {
        use std::time::Duration;
        // Default: env unset -> 30 s watchdog.
        assert_eq!(
            super::parse_deadlock_timeout(None),
            Some(Duration::from_secs(30))
        );
        // 0 -> disabled (blocking poll).
        assert_eq!(super::parse_deadlock_timeout(Some("0")), None);
        // Valid positive integer -> Some(secs).
        assert_eq!(
            super::parse_deadlock_timeout(Some("120")),
            Some(Duration::from_secs(120))
        );
        // Malformed -> silent fallback to default 30 s (don't crash
        // the worker on env-knob abuse).
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
