// Unix-domain-socket wire protocol v0 runtime — UDS counterpart of
// `mp_tcp_common::wire_runtime`. This file is the SINGLE SOURCE of
// the UDS framing code: `mp-uds-event`'s lib `include!`s it via the
// `#[cfg(test)] mod wire_runtime;` compile-coverage hook (host crate
// `cargo test --workspace`) AND copies it verbatim into each
// generated multi-process project as `src/wire.rs`.
//
// Why not reuse `mp_tcp_common::WIRE_RUNTIME_SRC` verbatim?
// `mp_tcp_common`'s public API takes a mutable TCP stream reference
// (the legacy TcpStream type); UDS uses
// `std::os::unix::net::UnixStream`. Rust's type system does not let
// the same function accept both; lifting `mp_tcp_common` to be
// transport-parametric is a 3-consumer refactor candidate (filed as
// follow-up of TASK-0044.02.03). For cycle 197 we take AC#7
// option (c): inline a UDS-specific runtime here.
//
// The wire BYTES — `[len u64 LE][seq u64 LE][payload]` — are
// IDENTICAL to `mp_tcp_common`'s; only the socket type changes. The
// header length invariant is pinned by the
// `wire_runtime_compile_check::header_len_matches_tcp_common` host
// test below — if the TCP wire protocol bumps versions, this UDS
// runtime MUST bump too in lockstep (the wire-layer contract is
// transport-agnostic at v0; future versions may differ).
//
// Keep dependency-free (std only) and panic-on-protocol-violation
// (fail loud: a framing mismatch is a codegen bug, never recoverable).

#![allow(dead_code, unused_imports)]

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

/// Header: 8-byte LE length + 8-byte LE seq tag, then `length` bytes.
/// Held in lockstep with `mp_tcp_common::wire_runtime`'s HEADER_LEN
/// (pinned by `header_len_matches_tcp_common` test below + by
/// `runtime_src::tests::header_len_matches_wire_runtime` for the UDS
/// reactor's mirror constant).
pub const HEADER_LEN: usize = 16;

/// Resolve the host-side handshake (accept) wall-clock deadline in
/// milliseconds. Default 30_000 ms (30 s); override via
/// `NUC_HANDSHAKE_DEADLINE_MS`. Values <= 0 / unparsable fall back to the
/// default (a typo never silently disables the bound). Mirrors
/// `mp_tcp_common::wire_runtime::handshake_deadline_ms` exactly — same
/// env-var name, default, and warn-and-default policy — because the UDS
/// runtime is a separate inlined file (`mp_tcp_common`'s helper is
/// `TcpListener`-typed and cannot bound a `UnixListener`).
///
/// # Why this exists (TASK-0461.01)
///
/// The generated host's `UnixListener::accept()` was an UNBOUNDED
/// blocking call: a non-host worker that never connected left the host
/// blocked in `accept()` FOREVER at 0% CPU — the 10.5h-night-eater
/// signature. This is the silent sibling of the mp-tcp-bufsync/poll fix
/// in TASK-0461.
pub fn handshake_deadline_ms() -> u128 {
    match std::env::var("NUC_HANDSHAKE_DEADLINE_MS") {
        Err(_) => 30_000,
        Ok(raw) => match raw.parse::<u128>() {
            Ok(v) if v > 0 => v,
            // Warn-and-default rather than silently coercing: a knob typo
            // must not silently disable the bound, but aborting a
            // generated PROGRAM over a bad env var would turn a typo into
            // a run failure (same policy as the TCP variant).
            _ => {
                eprintln!(
                    "[wire(uds)] WARNING: NUC_HANDSHAKE_DEADLINE_MS={raw:?} is not a \
                     positive integer; using the 30000 ms default"
                );
                30_000
            }
        },
    }
}

/// `accept()` one connection from a `UnixListener`, bounded by the
/// [`handshake_deadline_ms`] wall-clock deadline. The listener is put in
/// nonblocking mode for the poll loop and restored to blocking before the
/// accepted stream is returned, so the caller's subsequent std->mio
/// conversion (which sets the stream nonblocking itself) is unaffected.
/// `role` and `who` name the channel + peer for the loud-failure message.
///
/// On a never-arriving connection this panics with a clear diagnostic
/// rather than blocking forever (TASK-0461.01 AC#2: the UnixListener
/// mirror of `mp_tcp_common::wire_runtime::accept_with_deadline`).
pub fn accept_with_deadline(listener: &UnixListener, role: &str, who: &str) -> UnixStream {
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
                         connected (it died, raced on a stale rendezvous path, or wedged \
                         before connect). Bounded-accept fail-loud (TASK-0461.01)."
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => panic!("host: accept {role} from {who} failed: {e}"),
        }
    };
    // Restore blocking on the listener (defensive: a later accept on a
    // sibling listener path must see a clean mode) and ALSO make the
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

/// UDS sockets honour SO_*BUF the same way TCP does, but loopback UDS
/// buffer defaults (typically 212 KB) are large enough for every
/// in-tree per-channel payload that mp-uds-event currently handles.
/// We keep the apply_sock_buf call shape for emit-symmetry with
/// mp-tcp-event but make it a no-op — the UDS-specific NUC_SO_BUF
/// behaviour is intentionally NOT exposed at cycle 197 to keep the
/// surface minimal. If a future schedule's per-channel payload
/// exceeds the kernel UDS default, a typed setsockopt path mirroring
/// `mp_tcp_common::wire_runtime::apply_sock_buf` can be added here
/// without changing the emitted call sites.
pub fn apply_sock_buf(_sock: &UnixStream) {}

/// Write one framed message: `[len u64 LE][seq u64 LE][payload]`.
/// Fail-loud on any I/O error (the generated program has no recovery
/// path — a broken UDS socket is an abort-worthy bug, same shape as
/// mp_tcp_common's TCP variant).
pub fn write_msg(sock: &mut UnixStream, seq: u64, payload: &[u8]) {
    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[8..16].copy_from_slice(&seq.to_le_bytes());
    sock.write_all(&header)
        .unwrap_or_else(|e| panic!("wire(uds): header write failed (seq={seq}): {e}"));
    sock.write_all(payload)
        .unwrap_or_else(|e| panic!("wire(uds): payload write failed (seq={seq}): {e}"));
    sock.flush()
        .unwrap_or_else(|e| panic!("wire(uds): flush failed (seq={seq}): {e}"));
}

/// Read one framed message. Returns `(seq_tag, payload)`. The caller
/// knows the expected `seq` at compile time and asserts it (fail-loud
/// cross-check; see protocol doc).
pub fn read_msg(sock: &mut UnixStream) -> (u64, Vec<u8>) {
    let mut header = [0u8; HEADER_LEN];
    sock.read_exact(&mut header)
        .unwrap_or_else(|e| panic!("wire(uds): header read failed: {e}"));
    let len = u64::from_le_bytes(header[0..8].try_into().unwrap()) as usize;
    let seq = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let mut payload = vec![0u8; len];
    sock.read_exact(&mut payload)
        .unwrap_or_else(|e| panic!("wire(uds): payload read failed (seq={seq}, len={len}): {e}"));
    (seq, payload)
}

/// Read a message and assert its seq tag matches what the schedule
/// said this receive must be. A mismatch means the deterministic
/// event order diverged between the two independently-emitted
/// endpoints — a contract regression, never silently tolerated.
pub fn read_msg_expect(sock: &mut UnixStream, expect_seq: u64) -> Vec<u8> {
    let (seq, payload) = read_msg(sock);
    if seq != expect_seq {
        panic!(
            "wire(uds): seq tag mismatch: receiver expected {expect_seq}, \
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
/// 2-party barrier on a duplex UDS stream — both writes fit in the
/// socket buffer (16 bytes) so neither write blocks before the peer
/// reads.
pub fn barrier_cross(sock: &mut UnixStream, barrier_id: u64) {
    write_msg(sock, barrier_id, &[]);
    let got = read_msg_expect(sock, barrier_id);
    if !got.is_empty() {
        panic!("wire(uds): barrier {barrier_id} carried a non-empty payload");
    }
}

// ---- Scalar / array LE encode-decode (transport-independent; bytes
// ---- are identical to mp_tcp_common's macros). Inlined here so the
// ---- generated project's `src/wire.rs` is self-contained — the
// ---- emitted bin's `wire::enc_*` / `wire::dec_*` calls resolve here.

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
                    "wire(uds): scalar decode width mismatch: got {} bytes, want {}",
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
        panic!(
            "wire(uds): bool decode width mismatch: got {} bytes, want 1",
            b.len()
        );
    }
    match b[0] {
        0 => false,
        1 => true,
        other => panic!("wire(uds): bool decode saw byte {other}, expected 0 or 1"),
    }
}
pub fn enc_vec_bool(v: &[bool]) -> Vec<u8> {
    v.iter().map(|&b| u8::from(b)).collect()
}
pub fn dec_vec_bool(b: &[u8]) -> Vec<bool> {
    b.iter()
        .map(|&x| match x {
            0 => false,
            1 => true,
            other => panic!("wire(uds): bool-vec decode saw byte {other}, expected 0 or 1"),
        })
        .collect()
}

pub fn enc_vec<T: Copy, const W: usize>(v: &[T], to_le: fn(T) -> [u8; W]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * W);
    for &e in v {
        out.extend_from_slice(&to_le(e));
    }
    out
}

pub fn dec_vec<T, const W: usize>(b: &[u8], from_le: fn([u8; W]) -> T) -> Vec<T> {
    if b.len() % W != 0 {
        panic!(
            "wire(uds): vec decode length {} is not a multiple of element width {}",
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

// =====================================================================
// Host-side compile-check tests (cycle-197 cycle-194-F1-style guard).
// =====================================================================
//
// These only build under the host crate's `#[cfg(test)] mod
// wire_runtime;` declaration in `lib.rs`. The generated project's
// copy of this file (`src/wire.rs` in the emitted Cargo project) is
// NOT compiled with `cfg(test)`, so these tests do not bleed into
// emitted code.

#[cfg(test)]
mod tests {
    use super::HEADER_LEN;

    /// HEADER_LEN drift between THIS file (UDS wire runtime) and
    /// `mp_tcp_common::wire_runtime`'s codec is a load-bearing
    /// invariant: the wire-protocol v0 byte layout is
    /// transport-agnostic; if the TCP wire bumps versions, this UDS
    /// runtime MUST bump too in lockstep. Re-derives the TCP constant
    /// by scanning the public `WIRE_RUNTIME_SRC` string for the
    /// authoritative declaration.
    #[test]
    fn header_len_matches_tcp_common() {
        let needle = "const HEADER_LEN: usize = ";
        let src = mp_tcp_common::WIRE_RUNTIME_SRC;
        let i = src
            .find(needle)
            .expect("mp_tcp_common WIRE_RUNTIME_SRC lost its HEADER_LEN declaration");
        let rest = &src[i + needle.len()..];
        let end = rest
            .find(';')
            .expect("mp_tcp_common HEADER_LEN declaration missing terminator");
        let tcp_header_len: usize = rest[..end]
            .trim()
            .parse()
            .expect("mp_tcp_common HEADER_LEN value did not parse as usize");
        assert_eq!(
            tcp_header_len, HEADER_LEN,
            "TCP wire HEADER_LEN (={tcp_header_len}) and UDS wire HEADER_LEN \
             (={HEADER_LEN}) drifted. The wire byte layout is transport-agnostic \
             at v0; bump both together if a protocol version landed."
        );
    }

    /// Encode/decode round-trip on a representative scalar (u32) and
    /// a 4-element vec. Catches a regression in the inlined macros
    /// without a generated project in the loop.
    #[test]
    fn scalar_and_vec_roundtrip() {
        let bytes = super::enc_u32(0xDEADBEEF);
        assert_eq!(super::dec_u32(&bytes), 0xDEADBEEF);

        let v = vec![1i32, -2, 3, -4];
        let bytes = super::enc_vec(&v, i32::to_le_bytes);
        let back: Vec<i32> = super::dec_vec(&bytes, i32::from_le_bytes);
        assert_eq!(back, v);
    }

    /// Deadline path (TASK-0461.01 AC#4): NO client ever connects, so
    /// the bounded UDS accept must PANIC loud naming the worker + the
    /// deadline env var, instead of blocking forever (the night-eater
    /// signature). Mirrors `mp_tcp_common`'s
    /// `accept_with_deadline_panics_loud_when_no_peer`; a tiny
    /// `NUC_HANDSHAKE_DEADLINE_MS` override keeps it fast. Serialised
    /// against the deadline-default test via `DEADLINE_ENV_SERIAL`
    /// because both mutate the shared `NUC_HANDSHAKE_DEADLINE_MS` env.
    #[test]
    fn accept_with_deadline_panics_loud_when_no_peer() {
        use std::os::unix::net::UnixListener;
        let _serial = DEADLINE_ENV_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Unique per-pid+nanos socket path under target/ scratch; never
        // reused, so no remove/create race (matches the test scratch
        // convention — create-once-never-remove).
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let sock_path =
            std::env::temp_dir().join(format!("nuc-uds-accept-{}-{}.sock", std::process::id(), nanos));
        let _ = std::fs::remove_file(&sock_path);
        let listener = UnixListener::bind(&sock_path).expect("bind UDS listener");
        std::env::set_var("NUC_HANDSHAKE_DEADLINE_MS", "50");
        let probe = std::thread::spawn(move || {
            let _ = super::accept_with_deadline(&listener, "DATA", "w0");
        });
        let err = probe
            .join()
            .expect_err("accept_with_deadline must panic when no peer connects");
        std::env::remove_var("NUC_HANDSHAKE_DEADLINE_MS");
        let _ = std::fs::remove_file(&sock_path);
        let msg = err
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| err.downcast_ref::<&str>().copied())
            .unwrap_or("");
        assert!(
            msg.contains("accept DATA from w0")
                && msg.contains("NUC_HANDSHAKE_DEADLINE_MS=50")
                && msg.contains("timed out"),
            "expected a bounded-accept timeout panic naming the worker + deadline; got: {msg:?}"
        );
    }

    /// `handshake_deadline_ms` honours the env override and falls back to
    /// the 30s default for an unset / bogus value (typo never silently
    /// disables the bound). Mirror of the TCP variant's
    /// `handshake_deadline_ms_env_and_default`.
    #[test]
    fn handshake_deadline_ms_env_and_default() {
        let _serial = DEADLINE_ENV_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("NUC_HANDSHAKE_DEADLINE_MS");
        assert_eq!(super::handshake_deadline_ms(), 30_000, "default must be 30s");
        std::env::set_var("NUC_HANDSHAKE_DEADLINE_MS", "1234");
        assert_eq!(super::handshake_deadline_ms(), 1234, "env override must win");
        std::env::set_var("NUC_HANDSHAKE_DEADLINE_MS", "not-a-number");
        assert_eq!(
            super::handshake_deadline_ms(),
            30_000,
            "bogus value must fall back to default, never 0/unbounded"
        );
        std::env::remove_var("NUC_HANDSHAKE_DEADLINE_MS");
    }

    /// Serialises the two tests that mutate the process-global
    /// `NUC_HANDSHAKE_DEADLINE_MS` env var (cargo runs tests in the same
    /// process across threads; an unsynchronised set/remove race would
    /// flake them).
    static DEADLINE_ENV_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
