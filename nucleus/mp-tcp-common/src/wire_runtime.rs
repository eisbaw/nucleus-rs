// TCP wire protocol v0 runtime — see docs/wire-protocol-v0.md
// (TASK-0037). This file is the SINGLE SOURCE of the framing code:
//   - `mp-tcp-common`'s lib `include!`s it so the round-trip unit
//     test (AC#4) exercises exactly these bytes;
//   - the mp-tcp-bufsync + mp-tcp-poll backends copy this file
//     verbatim into each generated multi-process project as
//     `src/wire.rs`.
// Keep it dependency-free (std only) and panic-on-protocol-violation
// (fail loud: a framing mismatch is a codegen bug, never recoverable).

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};

/// Header: 8-byte LE length + 8-byte LE seq tag, then `length` bytes.
const HEADER_LEN: usize = 16;

/// Resolve the host-side handshake (accept + first-read) wall-clock
/// deadline in milliseconds. Default 30_000 ms (30 s); override via
/// `NUC_HANDSHAKE_DEADLINE_MS`. Values <= 0 / unparsable fall back to the
/// default (a typo never silently disables the bound).
///
/// # Why this exists (TASK-0461)
///
/// The generated host's `TcpListener::accept()` was an UNBOUNDED blocking
/// call: a non-host worker that never connected (it died, raced on a
/// stale rendezvous port, or wedged before its second connect) left the
/// host blocked in `accept()` FOREVER at 0% CPU with no established
/// socket — the exact 10.5h-night-eater signature. The non-host worker's
/// connect retry was already bounded (6 s, fail-loud), but the host's
/// accept was not. This deadline closes that asymmetry: a host that does
/// not see the expected connection within the budget fails LOUD naming
/// the worker, instead of hanging the whole run.
pub fn handshake_deadline_ms() -> u128 {
    match std::env::var("NUC_HANDSHAKE_DEADLINE_MS")
        .ok()
        .and_then(|s| s.parse::<u128>().ok())
    {
        Some(v) if v > 0 => v,
        _ => 30_000,
    }
}

/// `accept()` one connection from `listener`, bounded by the
/// [`handshake_deadline_ms`] wall-clock deadline. The listener is put in
/// nonblocking mode for the poll loop and restored to blocking before the
/// accepted stream is returned, so the caller's subsequent BLOCKING wire
/// protocol (mp-tcp-bufsync) is unchanged. `role` and `who` name the
/// channel + peer for the loud-failure message.
///
/// On a never-arriving connection this panics with a clear diagnostic
/// rather than blocking forever (TASK-0461 AC#1: bounded, loud failure on
/// the host accept path — the mirror of the worker's bounded
/// `connect_retry`).
pub fn accept_with_deadline(listener: &TcpListener, role: &str, who: &str) -> TcpStream {
    let deadline_ms = handshake_deadline_ms();
    let start = std::time::Instant::now();
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|e| panic!("host: set_nonblocking(true) on {who} listener failed: {e}"));
    let stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                let elapsed = start.elapsed().as_millis();
                if elapsed >= deadline_ms {
                    panic!(
                        "host: accept {role} from {who} timed out after {elapsed} ms \
                         >= NUC_HANDSHAKE_DEADLINE_MS={deadline_ms} ms — the worker never \
                         connected (it died, raced on a stale rendezvous port, or wedged \
                         before connect). Bounded-accept fail-loud (TASK-0461)."
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => panic!("host: accept {role} from {who} failed: {e}"),
        }
    };
    // Restore blocking on the listener (defensive: a later accept on the
    // same listener — e.g. the CTRL accept after DATA — must see a clean
    // mode if it does not itself go through this helper) and ALSO make the
    // accepted stream blocking, since it inherits the listener's
    // nonblocking flag on some platforms.
    listener
        .set_nonblocking(false)
        .unwrap_or_else(|e| panic!("host: restore blocking on {who} listener failed: {e}"));
    stream
        .set_nonblocking(false)
        .unwrap_or_else(|e| panic!("host: set accepted {role} stream from {who} blocking: {e}"));
    stream
}

/// Decide whether the kernel-granted socket buffer is large enough,
/// returning the EXACT clear-error string (naming the OS cap) when it
/// is not. Pure and side-effect-free: no syscalls, no env, no I/O —
/// the entire fail-loud DECISION distilled so it can be unit-tested
/// deterministically without a kernel cap (TASK-0174 AC: prove the
/// "clear error naming net.core.wmem_max/rmem_max" behaviour).
///
/// Linux internally DOUBLES the SO_SNDBUF/SO_RCVBUF request for
/// bookkeeping overhead, so the effective payload capacity is
/// `effective_got / 2`. We require that effective capacity to be
/// `>= want`; otherwise the OS-level cap clamped it below the
/// schedule's per-channel requirement and we must abort rather than
/// proceed under-sized.
///
/// `opt` is the socket option number (SO_SNDBUF=7 / SO_RCVBUF=8),
/// echoed into the message so the failing direction is identifiable.
///
/// Returns `Ok(())` if `effective_got / 2 >= want`, else `Err(msg)`
/// where `msg` is the verbatim panic text `apply_sock_buf` raises.
pub fn check_effective_sock_buf(want: i32, effective_got: i32, opt: i32) -> Result<(), String> {
    if (effective_got / 2) < want {
        return Err(format!(
            "wire: socket buffer too small: requested NUC_SO_BUF={want} \
             but the OS granted only {} effective bytes (opt={opt}); the \
             OS-level cap (net.core.wmem_max / rmem_max) is below the \
             schedule's per-channel requirement. Raise the cap or reduce \
             the transfer size.",
            effective_got / 2
        ));
    }
    Ok(())
}

/// Apply the run.sh-computed socket send/recv buffer size (TASK-0038
/// AC#2: "each binary calls setsockopt"). Size comes from the env var
/// `NUC_SO_BUF` that run.sh exports (derived from the schedule's
/// per-channel buffer requirement). Std has no portable
/// SO_SNDBUF/SO_RCVBUF setter, so on Unix we call libc `setsockopt`
/// directly (libc is always linked; no extra crate — reproducible).
///
/// Best-effort by design: the kernel clamps the request to
/// `net.core.{wmem,rmem}_max`. If the OS cap is *below* the requested
/// size the kernel silently gives less; we then *read it back* and,
/// via [`check_effective_sock_buf`] (the pure decision), if it is
/// smaller than required we FAIL LOUD (panic naming the cap) rather
/// than proceed with an under-sized buffer — exactly the clear-error
/// behaviour TASK-0038 AC#5 asks for. On non-Unix this is a no-op
/// (the supported transport target is Unix loopback).
#[cfg(unix)]
pub fn apply_sock_buf(sock: &TcpStream) {
    use std::os::unix::io::AsRawFd;

    let want: i32 = match std::env::var("NUC_SO_BUF").ok().and_then(|s| s.parse().ok()) {
        Some(v) if v > 0 => v,
        // Not set / unparsable: leave the OS default. run.sh always
        // exports it for a real run; absence means a hand-invocation.
        _ => return,
    };

    // libc constants (Linux x86-64; the supported target). SOL_SOCKET
    // = 1, SO_SNDBUF = 7, SO_RCVBUF = 8.
    const SOL_SOCKET: i32 = 1;
    const SO_SNDBUF: i32 = 7;
    const SO_RCVBUF: i32 = 8;

    extern "C" {
        fn setsockopt(
            fd: i32,
            level: i32,
            optname: i32,
            optval: *const std::ffi::c_void,
            optlen: u32,
        ) -> i32;
        fn getsockopt(
            fd: i32,
            level: i32,
            optname: i32,
            optval: *mut std::ffi::c_void,
            optlen: *mut u32,
        ) -> i32;
    }

    let fd = sock.as_raw_fd();
    for opt in [SO_SNDBUF, SO_RCVBUF] {
        let v = want;
        // SAFETY: fd is a live socket for the lifetime of `sock`; we
        // pass a correctly-sized i32 option value and length.
        let rc = unsafe {
            setsockopt(
                fd,
                SOL_SOCKET,
                opt,
                &v as *const i32 as *const std::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            )
        };
        if rc != 0 {
            panic!(
                "wire: setsockopt(opt={opt}, NUC_SO_BUF={want}) failed: {}",
                std::io::Error::last_os_error()
            );
        }
        // Read back. Linux doubles the value internally (bookkeeping
        // overhead), so the effective payload capacity is `got / 2`.
        // We require that effective capacity to be >= the requested
        // size; if the OS cap (wmem_max/rmem_max) clamped it lower,
        // fail loud with the exact numbers (AC#5).
        let mut got: i32 = 0;
        let mut len = std::mem::size_of::<i32>() as u32;
        let grc = unsafe {
            getsockopt(
                fd,
                SOL_SOCKET,
                opt,
                &mut got as *mut i32 as *mut std::ffi::c_void,
                &mut len as *mut u32,
            )
        };
        if grc == 0 {
            // Pure decision factored out so the fail-loud LOGIC is
            // unit-testable without a kernel cap (TASK-0174). Behaviour
            // is byte-identical: still panics here, with the exact same
            // message the pure function returns on Err.
            if let Err(msg) = check_effective_sock_buf(want, got, opt) {
                panic!("{msg}");
            }
        }
    }
}

#[cfg(not(unix))]
pub fn apply_sock_buf(_sock: &TcpStream) {}

/// Write one framed message: `[len u64 LE][seq u64 LE][payload]`.
/// Fail-loud on any I/O error (the generated program has no recovery
/// path — a broken loopback socket is an abort-worthy bug).
pub fn write_msg(sock: &mut TcpStream, seq: u64, payload: &[u8]) {
    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[8..16].copy_from_slice(&seq.to_le_bytes());
    sock.write_all(&header)
        .unwrap_or_else(|e| panic!("wire: header write failed (seq={seq}): {e}"));
    sock.write_all(payload)
        .unwrap_or_else(|e| panic!("wire: payload write failed (seq={seq}): {e}"));
    sock.flush()
        .unwrap_or_else(|e| panic!("wire: flush failed (seq={seq}): {e}"));
}

/// Read one framed message. Returns `(seq_tag, payload)`. The caller
/// knows the expected `seq` at compile time and asserts it (fail-loud
/// cross-check; see protocol doc).
pub fn read_msg(sock: &mut TcpStream) -> (u64, Vec<u8>) {
    let mut header = [0u8; HEADER_LEN];
    sock.read_exact(&mut header)
        .unwrap_or_else(|e| panic!("wire: header read failed: {e}"));
    let len = u64::from_le_bytes(header[0..8].try_into().unwrap()) as usize;
    let seq = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let mut payload = vec![0u8; len];
    sock.read_exact(&mut payload)
        .unwrap_or_else(|e| panic!("wire: payload read failed (seq={seq}, len={len}): {e}"));
    (seq, payload)
}

/// Read a message and assert its seq tag matches what the schedule
/// said this receive must be. A mismatch means the deterministic
/// event order diverged between the two independently-emitted
/// endpoints — a contract regression, never silently tolerated.
pub fn read_msg_expect(sock: &mut TcpStream, expect_seq: u64) -> Vec<u8> {
    let (seq, payload) = read_msg(sock);
    if seq != expect_seq {
        panic!(
            "wire: seq tag mismatch: receiver expected {expect_seq}, \
             wire delivered {seq} — Push/Wait pairing diverged between \
             the two generated endpoints (protocol v0 contract violation)"
        );
    }
    payload
}

/// A barrier crossing is a zero-payload message whose seq tag is the
/// pre-order barrier id. Two-party barrier over an existing
/// connection: each side sends its token then blocks for the peer's.
/// Order (send-then-recv on both sides) cannot deadlock for a
/// 2-party barrier on a duplex TCP stream — both writes fit in the
/// socket buffer (16 bytes) so neither write blocks before the peer
/// reads.
pub fn barrier_cross(sock: &mut TcpStream, barrier_id: u64) {
    write_msg(sock, barrier_id, &[]);
    let got = read_msg_expect(sock, barrier_id);
    if !got.is_empty() {
        panic!("wire: barrier {barrier_id} carried a non-empty payload");
    }
}

// ---- Nonblocking-poll variants (TASK-0044.02.02; mp-tcp-poll). -----
//
// mp-tcp-poll's PRD §7.1 row 5 wait primitive is *nonblocking poll*,
// not blocking recv. The poll-variant helpers below have the SAME
// contract as `read_msg_expect` / `barrier_cross` but expect the
// caller to have put the socket in nonblocking mode (via
// `apply_nonblocking` immediately after `apply_sock_buf` on connect).
//
// On a `WouldBlock` error the loop yields the thread (`yield_now`),
// NOT a sleep (which would add latency) and NOT a busy-spin (which
// would burn a full core). After `NUC_POLL_DEADLINE_MS` total wait
// time the loop panics LOUD naming the expected seq + the elapsed
// time — a never-sending peer surfaces as a typed error, not a
// silent spin (AC#7 of TASK-0044.02.02; memory
// `project-mp-tcp-event-vs-bufsync-safety-profile` analog).
//
// The deadline is read on every poll-helper invocation from the
// `NUC_POLL_DEADLINE_MS` env var (default 30_000 ms = 30 s; tests can
// override to a small value to exercise the deadline-exceeded path
// deterministically). Per-call re-read is intentional: the
// deadline-exceeded test mutates the env mid-process, and a cached-
// once-per-process value would defeat that override. Cost is a single
// env lookup per Wait/Push — negligible vs the I/O cost.

/// Mark every subsequent read/write on `sock` nonblocking. Idempotent.
/// Best-effort: panics LOUD on syscall failure (broken loopback
/// socket is an abort-worthy bug, same shape as `write_msg`'s I/O
/// error handling).
pub fn apply_nonblocking(sock: &TcpStream) {
    sock.set_nonblocking(true)
        .unwrap_or_else(|e| panic!("wire: set_nonblocking(true) failed: {e}"));
}

/// Poll-friendly write helper for nonblocking sockets. Loops on
/// `WouldBlock` with `yield_now`, bounded by the same
/// `NUC_POLL_DEADLINE_MS` deadline as `read_msg_expect_poll` so a
/// stuck send (full kernel send-buffer + non-draining peer) surfaces
/// as a loud panic instead of an infinite loop. On a freshly nonblocking
/// loopback socket the typical 64+ KiB kernel sendbuf absorbs every
/// in-tree payload in a single write — the yield-loop is here for
/// large-array correctness, not the common case.
fn write_all_poll(sock: &mut TcpStream, buf: &[u8], seq: u64, what: &str) {
    let deadline_ms = poll_deadline_ms();
    let start = std::time::Instant::now();
    let mut written = 0usize;
    while written < buf.len() {
        match sock.write(&buf[written..]) {
            Ok(0) => panic!("wire: {what} write returned 0 (peer closed?) seq={seq}"),
            Ok(n) => {
                written += n;
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                let elapsed = start.elapsed().as_millis();
                if elapsed >= deadline_ms {
                    panic!(
                        "wire: poll deadline exceeded on {what} write (seq={seq}): \
                         elapsed {elapsed} ms >= NUC_POLL_DEADLINE_MS={deadline_ms} ms \
                         after {written}/{} bytes — kernel sendbuf stayed full (peer \
                         not draining?)",
                        buf.len()
                    );
                }
                std::thread::yield_now();
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => panic!("wire: {what} write failed (seq={seq}): {e}"),
        }
    }
}

/// Poll-variant of [`write_msg`]. Same wire bytes; safe to call on a
/// nonblocking socket. Use from mp-tcp-poll codegen. mp-tcp-bufsync
/// continues to call `write_msg` directly (blocking socket).
pub fn write_msg_poll(sock: &mut TcpStream, seq: u64, payload: &[u8]) {
    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[8..16].copy_from_slice(&seq.to_le_bytes());
    write_all_poll(sock, &header, seq, "header");
    write_all_poll(sock, payload, seq, "payload");
    // `flush` is a no-op on TcpStream (kernel buffers; no userland
    // buffering), so we omit it here to keep the poll path free of a
    // gratuitous syscall. mp-tcp-bufsync's `write_msg` keeps the
    // explicit `flush()` for byte-identicality-by-history reasons.
}

/// Resolve the poll deadline in milliseconds. Default 30_000 ms (30 s);
/// override via `NUC_POLL_DEADLINE_MS`. Values <= 0 / unparsable fall
/// back to the default (a typo never silently disables the bound).
fn poll_deadline_ms() -> u128 {
    match std::env::var("NUC_POLL_DEADLINE_MS")
        .ok()
        .and_then(|s| s.parse::<u128>().ok())
    {
        Some(v) if v > 0 => v,
        _ => 30_000,
    }
}

/// Nonblocking-read header+payload exactly once. Returns the parsed
/// `(seq, payload)` once the FULL header + payload bytes are present;
/// otherwise returns `Ok(None)` to signal "not yet, try again". I/O
/// errors propagate `Err`.
///
/// The helper accumulates partial reads across calls via the
/// `header_buf` / `payload_buf` scratch buffers + the `phase` state
/// the caller carries — so a header that arrives split across two
/// `WouldBlock`-returning reads is reassembled correctly. A pure
/// `read_exact` style would not work in nonblocking mode (it would
/// EWOULDBLOCK on the first partial header byte).
fn try_read_msg_step(
    sock: &mut TcpStream,
    state: &mut ReadState,
) -> std::io::Result<Option<(u64, Vec<u8>)>> {
    loop {
        match state {
            ReadState::Header { buf, filled } => {
                let n = match sock.read(&mut buf[*filled..]) {
                    Ok(0) => {
                        return Err(std::io::Error::new(
                            ErrorKind::UnexpectedEof,
                            "wire: peer closed during header read",
                        ));
                    }
                    Ok(n) => n,
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => return Ok(None),
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                *filled += n;
                if *filled == HEADER_LEN {
                    let len = u64::from_le_bytes(buf[0..8].try_into().unwrap()) as usize;
                    let seq = u64::from_le_bytes(buf[8..16].try_into().unwrap());
                    *state = ReadState::Payload {
                        seq,
                        payload: vec![0u8; len],
                        filled: 0,
                    };
                    // Fall through to attempt payload read in the same call.
                    continue;
                }
            }
            ReadState::Payload {
                seq,
                payload,
                filled,
            } => {
                if payload.len() == *filled {
                    // Zero-length payload (e.g. barrier crossing): done.
                    let out = (*seq, std::mem::take(payload));
                    return Ok(Some(out));
                }
                let n = match sock.read(&mut payload[*filled..]) {
                    Ok(0) => {
                        return Err(std::io::Error::new(
                            ErrorKind::UnexpectedEof,
                            "wire: peer closed during payload read",
                        ));
                    }
                    Ok(n) => n,
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => return Ok(None),
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                *filled += n;
                if *filled == payload.len() {
                    let out = (*seq, std::mem::take(payload));
                    return Ok(Some(out));
                }
            }
        }
    }
}

/// Carry-state for `try_read_msg_step` — allows accumulating partial
/// header / payload bytes across nonblocking read attempts.
enum ReadState {
    Header {
        buf: [u8; HEADER_LEN],
        filled: usize,
    },
    Payload {
        seq: u64,
        payload: Vec<u8>,
        filled: usize,
    },
}

impl ReadState {
    fn fresh() -> Self {
        ReadState::Header {
            buf: [0u8; HEADER_LEN],
            filled: 0,
        }
    }
}

/// Poll-variant of [`read_msg_expect`]: nonblocking read loop with
/// `std::thread::yield_now` between cycles and a deadline-bound
/// loud-failure guarantee.
///
/// Contract: the caller MUST have called [`apply_nonblocking`] on
/// `sock` after the connect/accept (typically done once in the
/// emitted worker `main()` right after `apply_sock_buf`). A blocking
/// socket would never return `WouldBlock` so the yield-loop would
/// reduce to a blocking `read_exact` — defeating the purpose.
///
/// Deadline-exceeded surfaces as a loud `panic!` naming the expected
/// seq + elapsed milliseconds + the configured deadline — a
/// never-sending peer thus produces a clear error message rather
/// than a silent spin (AC#7 of TASK-0044.02.02). Seq-tag mismatch
/// preserves the existing fail-loud contract from
/// [`read_msg_expect`] (`panic!` with "seq tag mismatch").
pub fn read_msg_expect_poll(sock: &mut TcpStream, expect_seq: u64) -> Vec<u8> {
    let deadline_ms = poll_deadline_ms();
    let start = std::time::Instant::now();
    let mut state = ReadState::fresh();
    loop {
        match try_read_msg_step(sock, &mut state) {
            Ok(Some((seq, payload))) => {
                // Fail-loud cross-check — same contract as
                // `read_msg_expect`; the poll loop intentionally does NOT
                // mask this with a "wrong seq → keep waiting" path
                // (would mask contract violations under poll's
                // silent-spin safety profile; per memory
                // `project-mp-tcp-event-vs-bufsync-safety-profile`).
                if seq != expect_seq {
                    panic!(
                        "wire: seq tag mismatch (poll): receiver expected {expect_seq}, \
                         wire delivered {seq} — Push/Wait pairing diverged between the \
                         two generated endpoints (protocol v0 contract violation)"
                    );
                }
                return payload;
            }
            Ok(None) => {
                let elapsed = start.elapsed().as_millis();
                if elapsed >= deadline_ms {
                    panic!(
                        "wire: poll deadline exceeded waiting for seq={expect_seq}: \
                         elapsed {elapsed} ms >= NUC_POLL_DEADLINE_MS={deadline_ms} ms \
                         — peer did not send the expected frame (never-sending peer or \
                         crossed-up wire ordering; mp-tcp-poll bounded-retry contract, \
                         TASK-0044.02.02 AC#7)"
                    );
                }
                std::thread::yield_now();
            }
            Err(e) => panic!("wire: nonblocking read failed (poll, seq={expect_seq}): {e}"),
        }
    }
}

/// Poll-variant of [`barrier_cross`]: send-then-recv-with-poll the
/// zero-payload barrier token. Both the send leg
/// ([`write_msg_poll`]) and recv leg ([`read_msg_expect_poll`]) are
/// nonblocking-safe — the socket has had `apply_nonblocking` called
/// on it after connect/accept. The 16-byte header against any
/// reasonable kernel sendbuf never blocks in practice; the poll-write
/// is here for shape uniformity with the data path's
/// [`write_msg_poll`] (and so a single-direction WouldBlock surfaces
/// as deadline-exceeded, not a panic from `write_msg`'s
/// `unwrap_or_else`).
pub fn barrier_cross_poll(sock: &mut TcpStream, barrier_id: u64) {
    write_msg_poll(sock, barrier_id, &[]);
    let got = read_msg_expect_poll(sock, barrier_id);
    if !got.is_empty() {
        panic!("wire: barrier {barrier_id} (poll) carried a non-empty payload");
    }
}

// ---- Scalar / array LE encode-decode (AC#3: ordering fixed at -----
// ---- compile time; the backend emits the matched calls). ----------

macro_rules! le_scalar {
    ($enc:ident, $dec:ident, $ty:ty) => {
        /// Encode one scalar as fixed little-endian bytes.
        pub fn $enc(v: $ty) -> Vec<u8> {
            v.to_le_bytes().to_vec()
        }
        /// Decode one scalar from fixed little-endian bytes.
        pub fn $dec(b: &[u8]) -> $ty {
            const W: usize = std::mem::size_of::<$ty>();
            if b.len() != W {
                panic!(
                    "wire: scalar decode width mismatch: got {} bytes, want {}",
                    b.len(),
                    W
                );
            }
            <$ty>::from_le_bytes(b.try_into().unwrap())
        }
    };
}

le_scalar!(enc_i8, dec_i8, i8);
le_scalar!(enc_i16, dec_i16, i16);
le_scalar!(enc_i32, dec_i32, i32);
le_scalar!(enc_i64, dec_i64, i64);
le_scalar!(enc_u8, dec_u8, u8);
le_scalar!(enc_u16, dec_u16, u16);
le_scalar!(enc_u32, dec_u32, u32);
le_scalar!(enc_u64, dec_u64, u64);
le_scalar!(enc_f32, dec_f32, f32);
le_scalar!(enc_f64, dec_f64, f64);

/// Bool as a single byte (0/1). Strict decode: any other byte is a
/// protocol violation (fail loud).
pub fn enc_bool(v: bool) -> Vec<u8> {
    vec![u8::from(v)]
}
pub fn dec_bool(b: &[u8]) -> bool {
    if b.len() != 1 {
        panic!("wire: bool decode width mismatch: got {} bytes, want 1", b.len());
    }
    match b[0] {
        0 => false,
        1 => true,
        other => panic!("wire: bool decode saw byte {other}, expected 0 or 1"),
    }
}
/// `[bool] -> bytes`: one byte per element (matches `enc_vec`'s
/// per-element contract; `bool` has no `to_le_bytes`, hence the
/// dedicated pair).
pub fn enc_vec_bool(v: &[bool]) -> Vec<u8> {
    v.iter().map(|&b| u8::from(b)).collect()
}
pub fn dec_vec_bool(b: &[u8]) -> Vec<bool> {
    b.iter()
        .map(|&x| match x {
            0 => false,
            1 => true,
            other => panic!("wire: bool-vec decode saw byte {other}, expected 0 or 1"),
        })
        .collect()
}

/// Encode a `Vec<T>` as concatenated fixed-LE element bytes. `W` is
/// `size_of::<T>()`; `to_le` maps one element to its `W` bytes.
pub fn enc_vec<T: Copy, const W: usize>(v: &[T], to_le: fn(T) -> [u8; W]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * W);
    for &e in v {
        out.extend_from_slice(&to_le(e));
    }
    out
}

/// Decode concatenated fixed-LE bytes back into a `Vec<T>`. The byte
/// length must be an exact multiple of `W` (loud otherwise).
pub fn dec_vec<T, const W: usize>(b: &[u8], from_le: fn([u8; W]) -> T) -> Vec<T> {
    if b.len() % W != 0 {
        panic!(
            "wire: vec decode length {} is not a multiple of element width {}",
            b.len(),
            W
        );
    }
    let mut out = Vec::with_capacity(b.len() / W);
    let mut i = 0;
    while i < b.len() {
        let mut chunk = [0u8; W];
        chunk.copy_from_slice(&b[i..i + W]);
        out.push(from_le(chunk));
        i += W;
    }
    out
}
