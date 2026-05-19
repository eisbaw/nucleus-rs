//! TCP wire protocol v0 for the `mp-tcp-*` backends (TASK-0037).
//!
//! See `docs/wire-protocol-v0.md` for the byte layout and rationale.
//!
//! This crate has two roles, by design:
//!
//! 1. **Tested library.** The [`wire`] module is the protocol
//!    implementation; the unit tests below round-trip canned
//!    payloads through it (TASK-0037 AC#4).
//! 2. **Single source for emitted code.** [`WIRE_RUNTIME_SRC`] is the
//!    *exact text* of the file the [`wire`] module is `include!`-d
//!    from. The `mp-tcp-bufsync` backend writes that text verbatim
//!    into every generated project as `src/wire.rs`, so the protocol
//!    that ships in generated binaries is byte-identical to the
//!    protocol this crate's tests exercise — there is no second copy
//!    to drift (the drift risk flagged in TASK-0124 carried forward).

/// The framing + codec runtime, compiled into this crate so the
/// tests exercise it, and `include!`-d from the same source file the
/// backend emits verbatim.
#[allow(dead_code)]
pub mod wire {
    include!("wire_runtime.rs");
}

/// The verbatim source text of [`wire`], for the backend to copy into
/// generated projects as `src/wire.rs`. Single source of truth: this
/// is the same file `mod wire` is built from.
pub const WIRE_RUNTIME_SRC: &str = include_str!("wire_runtime.rs");

#[cfg(test)]
mod tests {
    use super::wire::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    #[test]
    fn scalar_round_trips() {
        assert_eq!(dec_i32(&enc_i32(-42)), -42);
        assert_eq!(dec_i32(&enc_i32(i32::MIN)), i32::MIN);
        assert_eq!(dec_u64(&enc_u64(u64::MAX)), u64::MAX);
        assert_eq!(dec_f64(&enc_f64(3.5)), 3.5);
        assert_eq!(dec_i8(&enc_i8(-1)), -1);
    }

    #[test]
    fn scalar_bytes_are_fixed_little_endian() {
        // Canned: 0x01020304 LE == [04 03 02 01]. Pins the on-wire
        // byte order so a backend that emits a mismatched decoder is
        // caught here, not by a flaky differential.
        assert_eq!(enc_i32(0x0102_0304), vec![0x04, 0x03, 0x02, 0x01]);
        assert_eq!(enc_u16(0xABCD), vec![0xCD, 0xAB]);
    }

    #[test]
    fn vec_round_trips() {
        let v: Vec<i32> = vec![0, 1, -1, 256, i32::MIN, i32::MAX];
        let bytes = enc_vec(&v, i32::to_le_bytes);
        assert_eq!(bytes.len(), v.len() * 4);
        let back: Vec<i32> = dec_vec(&bytes, i32::from_le_bytes);
        assert_eq!(back, v);

        let empty: Vec<i32> = vec![];
        assert_eq!(dec_vec::<i32, 4>(&enc_vec(&empty, i32::to_le_bytes), i32::from_le_bytes), empty);
    }

    #[test]
    #[should_panic(expected = "not a multiple of element width")]
    fn vec_decode_rejects_ragged_length() {
        let _: Vec<i32> = dec_vec(&[1, 2, 3], i32::from_le_bytes);
    }

    /// Round-trip canned payloads through the real framing over a
    /// loopback `TcpStream` pair. Deterministic: bind to port 0
    /// (kernel-assigned), hand the resolved port to the client; no
    /// sleeps, no fixed ports.
    #[test]
    fn framed_messages_round_trip_over_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();

        // Canned messages: (seq, payload).
        let canned: Vec<(u64, Vec<u8>)> = vec![
            (0, b"hello".to_vec()),
            (1, enc_vec(&[1i32, 2, 3, 4], i32::to_le_bytes)),
            (7, vec![]), // barrier-shaped: zero payload
            (42, vec![0xFF; 1024]),
        ];
        let expect = canned.clone();

        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            for (seq, payload) in &expect {
                let got = read_msg_expect(&mut s, *seq);
                assert_eq!(&got, payload, "payload mismatch at seq {seq}");
            }
            // echo a final ack so the client knows we read everything
            s.write_all(&[1]).unwrap();
        });

        let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        for (seq, payload) in &canned {
            write_msg(&mut c, *seq, payload);
        }
        let mut ack = [0u8; 1];
        c.read_exact(&mut ack).expect("ack");
        assert_eq!(ack[0], 1);
        server.join().expect("server thread");
    }

    /// The seq-tag cross-check must fire loudly on a mismatch. The
    /// check runs on the receiver thread; we join it and assert the
    /// panic payload contains the expected message (joining and
    /// inspecting the payload is more precise than `should_panic`,
    /// which would match the generic re-panic wrapper instead).
    #[test]
    fn read_msg_expect_rejects_wrong_seq() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            // Receiver thinks seq should be 99; sender sends 5.
            let _ = read_msg_expect(&mut s, 99);
        });
        let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        write_msg(&mut c, 5, b"x");
        let err = server.join().expect_err("receiver must panic on seq mismatch");
        let msg = err
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| err.downcast_ref::<&str>().copied())
            .unwrap_or("");
        assert!(
            msg.contains("seq tag mismatch"),
            "expected a seq-tag-mismatch panic, got: {msg:?}"
        );
    }

    /// Two-party barrier over one duplex stream completes from both
    /// sides without deadlock.
    #[test]
    fn barrier_cross_two_party() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            barrier_cross(&mut s, 0);
            barrier_cross(&mut s, 1);
        });
        let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        barrier_cross(&mut c, 0);
        barrier_cross(&mut c, 1);
        server.join().expect("server");
    }
}
