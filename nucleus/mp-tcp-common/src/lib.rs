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
    use std::sync::Mutex;
    use std::thread;

    // Serialise the 4 poll-helper tests that read or write the
    // process-wide `NUC_POLL_DEADLINE_MS` env var. cargo test runs
    // tests in parallel; without this, the deadline test's
    // 50ms-override could land mid-handshake of one of the other 3
    // poll tests, causing it to spuriously hit the deadline. The
    // deadline test acquires the lock for its full set_var..panic..
    // remove_var window; the 3 normal poll tests just block on it.
    // (Review-gate cycle-195 architect P2.1 fold-back.)
    static POLL_ENV_SERIAL: Mutex<()> = Mutex::new(());

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
        assert_eq!(
            dec_vec::<i32, 4>(&enc_vec(&empty, i32::to_le_bytes), i32::from_le_bytes),
            empty
        );
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
        let err = server
            .join()
            .expect_err("receiver must panic on seq mismatch");
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

    // ---- TASK-0174 (A): the SO_*BUF fail-loud DECISION, proven -----
    // ---- deterministically without a kernel cap. These pin the ----
    // ---- exact clear-error behaviour TASK-0038 AC#5 asks for: an ---
    // ---- OS cap below the schedule requirement => a CLEAR error ----
    // ---- naming net.core.wmem_max/rmem_max. apply_sock_buf still ---
    // ---- panics with this exact string (refactor is behaviour- ----
    // ---- identical); see check_effective_sock_buf docs. ------------

    const SO_SNDBUF: i32 = 7;
    const SO_RCVBUF: i32 = 8;

    /// Enough buffer (effective got/2 >= want) => Ok, no error.
    #[test]
    fn sock_buf_check_ok_when_kernel_grants_enough() {
        // Kernel doubled the 64 KiB request to 128 KiB; effective
        // capacity = 65536 >= 65536 want. Boundary: exactly equal.
        assert_eq!(check_effective_sock_buf(65536, 131072, SO_SNDBUF), Ok(()));
        // Comfortably above.
        assert_eq!(check_effective_sock_buf(4096, 262144, SO_RCVBUF), Ok(()));
    }

    /// OS cap clamped the buffer below the schedule requirement =>
    /// Err with the CLEAR message naming the OS cap, the requested
    /// size, the granted effective size, and the failing option.
    #[test]
    fn sock_buf_check_fails_loud_naming_os_cap_when_clamped() {
        // Schedule needs 1 MiB; a lowered net.core.wmem_max clamps the
        // kernel to ~4096 total => 2048 effective. Must reject.
        let err = check_effective_sock_buf(1_048_576, 4096, SO_SNDBUF)
            .expect_err("a clamped-below-requirement buffer MUST be rejected");
        // The error must NAME the OS cap by its exact sysctl key so a
        // user knows precisely what to raise.
        assert!(
            err.contains("net.core.wmem_max") && err.contains("rmem_max"),
            "clear error must name the OS cap (net.core.wmem_max / rmem_max); got: {err:?}"
        );
        // It must report the requested and the effective-granted size
        // so the gap is diagnosable without rerunning.
        assert!(
            err.contains("NUC_SO_BUF=1048576") && err.contains("2048 effective bytes"),
            "error must report requested + granted sizes; got: {err:?}"
        );
        assert!(
            err.contains("socket buffer too small") && err.contains("opt=7"),
            "error must identify the failing socket option; got: {err:?}"
        );
    }

    /// The got/2 doubling boundary is exact: one byte of effective
    /// shortfall (odd value rounding down) still rejects, proving the
    /// Linux-doubling subtlety is handled, not approximated.
    #[test]
    fn sock_buf_check_boundary_is_exact_on_the_doubling() {
        // want=2049; got=4096 => got/2 = 2048 < 2049 => reject.
        assert!(check_effective_sock_buf(2049, 4096, SO_RCVBUF).is_err());
        // want=2048; got=4096 => got/2 = 2048 >= 2048 => ok.
        assert_eq!(check_effective_sock_buf(2048, 4096, SO_RCVBUF), Ok(()));
        // Odd got rounds DOWN (integer div): got=4097 => got/2 = 2048.
        assert_eq!(check_effective_sock_buf(2048, 4097, SO_RCVBUF), Ok(()));
        assert!(check_effective_sock_buf(2049, 4097, SO_RCVBUF).is_err());
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

    // ---- TASK-0044.02.02: nonblocking-poll wire helpers -----------
    //
    // Mirrors the existing blocking-recv tests above. mp-tcp-poll's
    // emitted code consumes these exact helpers via the verbatim
    // `wire.rs` copy.

    /// Round-trip canned payloads through `write_msg_poll` +
    /// `read_msg_expect_poll` over a loopback `TcpStream` pair. Both
    /// ends call `apply_nonblocking` (the contract of the poll
    /// helpers) so the entire exchange exercises the WouldBlock /
    /// yield_now path under normal conditions.
    #[test]
    fn framed_messages_round_trip_poll() {
        let _serial = POLL_ENV_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();

        let canned: Vec<(u64, Vec<u8>)> = vec![
            (0, b"hello".to_vec()),
            (1, enc_vec(&[1i32, 2, 3, 4], i32::to_le_bytes)),
            (7, vec![]), // barrier-shaped: zero payload
            (42, vec![0xFF; 4096]),
        ];
        let expect = canned.clone();

        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            apply_nonblocking(&s);
            for (seq, payload) in &expect {
                let got = read_msg_expect_poll(&mut s, *seq);
                assert_eq!(&got, payload, "payload mismatch at seq {seq}");
            }
            // Ack via a one-byte poll write so the client knows the
            // server is done.
            write_msg_poll(&mut s, u64::MAX, &[0x55]);
        });

        let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        apply_nonblocking(&c);
        for (seq, payload) in &canned {
            write_msg_poll(&mut c, *seq, payload);
        }
        let ack = read_msg_expect_poll(&mut c, u64::MAX);
        assert_eq!(ack, vec![0x55]);
        server.join().expect("server thread");
    }

    /// The seq-tag cross-check fires loudly under poll too. Same shape
    /// as `read_msg_expect_rejects_wrong_seq` for the blocking path —
    /// poll must NOT silently swallow a seq mismatch as "wrong frame,
    /// keep waiting" (would mask contract violations; see memory
    /// `project-mp-tcp-event-vs-bufsync-safety-profile`).
    #[test]
    fn read_msg_expect_poll_rejects_wrong_seq() {
        let _serial = POLL_ENV_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            apply_nonblocking(&s);
            let _ = read_msg_expect_poll(&mut s, 99);
        });
        let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        apply_nonblocking(&c);
        write_msg_poll(&mut c, 5, b"x");
        let err = server
            .join()
            .expect_err("receiver must panic on seq mismatch under poll");
        let msg = err
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| err.downcast_ref::<&str>().copied())
            .unwrap_or("");
        assert!(
            msg.contains("seq tag mismatch") && msg.contains("poll"),
            "expected a poll seq-tag-mismatch panic, got: {msg:?}"
        );
    }

    /// Deadline-exceeded surfaces as a loud `panic!` naming the
    /// expected seq + elapsed + deadline (AC#7 of TASK-0044.02.02).
    /// Drives the never-sending-peer scenario with a tiny override
    /// (`NUC_POLL_DEADLINE_MS=50`) so the test runs in <1 s.
    #[test]
    fn read_msg_expect_poll_panics_loud_on_deadline() {
        // Lock the env-serial for the full set_var..panic..remove_var
        // window so the 3 other poll tests don't see a 50ms deadline
        // mid-handshake (architect cycle-195 P2.1 fold-back).
        let _serial = POLL_ENV_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Hold a listener open without ever sending so the client's
        // read_msg_expect_poll exhausts its deadline.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            // Accept the connection and then idle. The bound listener
            // + accepted socket are dropped when the closure returns
            // (which the client's panic timing controls indirectly).
            let _accepted = listener.accept().expect("accept");
            // Hold the socket open long enough for the client deadline
            // to expire deterministically.
            thread::sleep(std::time::Duration::from_millis(500));
        });

        let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        apply_nonblocking(&c);

        // Edition 2021: set_var is safe. We're already serialised
        // against the other poll tests via POLL_ENV_SERIAL, so no
        // reader race during this test's window.
        std::env::set_var("NUC_POLL_DEADLINE_MS", "50");
        // Drive the deadline panic on a worker thread so we can catch
        // it via thread::join (instead of #[should_panic], which
        // would also accept *any* panic — we want the exact message).
        // We MUST move `c` into the thread; std::panic::catch_unwind
        // requires UnwindSafe and the easiest way is to spawn.
        let probe = thread::spawn(move || {
            let _ = read_msg_expect_poll(&mut c, 7);
        });
        let err = probe
            .join()
            .expect_err("read_msg_expect_poll must panic on deadline");
        let msg = err
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| err.downcast_ref::<&str>().copied())
            .unwrap_or("");
        // Restore the env so subsequent tests pick up the default.
        std::env::remove_var("NUC_POLL_DEADLINE_MS");
        assert!(
            msg.contains("poll deadline exceeded")
                && msg.contains("NUC_POLL_DEADLINE_MS=50")
                && msg.contains("seq=7")
                && msg.contains("elapsed"),
            "expected the deadline-exceeded panic naming seq+deadline+elapsed; got: {msg:?}"
        );
        // Clean up the server thread (it sleeps unconditionally).
        let _ = server.join();
    }

    /// Two-party poll-barrier completes without deadlock. Companion
    /// to `barrier_cross_two_party` for the blocking sibling.
    #[test]
    fn barrier_cross_poll_two_party() {
        let _serial = POLL_ENV_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            apply_nonblocking(&s);
            barrier_cross_poll(&mut s, 0);
            barrier_cross_poll(&mut s, 1);
        });
        let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        apply_nonblocking(&c);
        barrier_cross_poll(&mut c, 0);
        barrier_cross_poll(&mut c, 1);
        server.join().expect("server");
    }
}
