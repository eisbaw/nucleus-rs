// TCP wire protocol v0 runtime — see docs/wire-protocol-v0.md
// (TASK-0037). This file is the SINGLE SOURCE of the framing code:
//   - `mp-tcp-common`'s lib `include!`s it so the round-trip unit
//     test (AC#4) exercises exactly these bytes;
//   - the mp-tcp-bufsync backend copies this file verbatim into each
//     generated multi-process project as `src/wire.rs`.
// Keep it dependency-free (std only) and panic-on-protocol-violation
// (fail loud: a framing mismatch is a codegen bug, never recoverable).

use std::io::{Read, Write};
use std::net::TcpStream;

/// Header: 8-byte LE length + 8-byte LE seq tag, then `length` bytes.
const HEADER_LEN: usize = 16;

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
