# TCP wire protocol v0 (mp-tcp-* backends)

Task: TASK-0037. Shared by every `mp-tcp-*` backend (the first
consumer is `mp-tcp-bufsync`, TASK-0036). Implemented in the
`mp-tcp-common` crate (`nucleus/mp-tcp-common`) and *copied* into
every generated multi-process project so the generated binaries
depend on nothing exotic (same self-containment rule the other
backends follow).

## Byte layout (AC#1)

Every cross-worker message is exactly:

```
+-------------------+-------------------+----------------------+
| length: u64 LE    | seq_tag: u64 LE   | payload: length bytes|
| (8 bytes)         | (8 bytes)         |                      |
+-------------------+-------------------+----------------------+
```

- `length` — number of payload bytes that follow the 16-byte header,
  little-endian `u64`.
- `seq_tag` — the `compiler::event::SeqTag` value the projection
  attached to the matched `Event::Push` / `Event::Wait` pair, copied
  verbatim, little-endian `u64`. It is **not** interpreted by the
  transport; it travels so a receiver can assert it received the
  message it expected (a fail-loud cross-check, not routing).
- `payload` — the encoded data value (see below).

There is exactly one fixed header shape. The same 16-byte header also
prefixes the zero-length **barrier token** (`length = 0`, `seq_tag`
carries the pre-order barrier id) — a barrier is just a 16-byte
message with no payload, so the framing code is the only framing code.

## No framing beyond the length prefix (AC#2)

v0 has:

- **No checksums.** The transport is loopback TCP (`127.0.0.1`). TCP
  already provides ordering and a 16-bit checksum on the segment; the
  kernel does not corrupt loopback traffic. Adding an application
  checksum would be defensive code for a failure mode that cannot
  occur on the only supported transport, and it would have to be
  byte-stable across backends for the differential — dead weight.
- **No version byte / no negotiation.** Both endpoints are emitted by
  the *same* `nucleus build` invocation from the *same* compiler.
  There is no mixed-version deployment to negotiate against. A
  version byte would imply a compatibility story v0 explicitly does
  not have.
- **No per-message worker-id tag.** There is exactly one TCP
  connection per `(host, worker)` ordered pair, and the event order
  on that connection is fixed by the schedule. The endpoint *is* the
  routing; a worker-id tag would be redundant and a second,
  drift-prone source of truth for "who is this for".

This format is **loopback-only**. It is not authenticated, not
encrypted, not robust against a hostile or lossy peer. It must never
carry traffic to an untrusted host. That is an explicit v0
non-goal, not an oversight (AC#2, AC#6 limitation).

## Byte ordering agreed at compile time (AC#3)

Sender and receiver never exchange type metadata on the wire. The
Nucleus IR's `NameSidecar` type info (`ResolvedType` /`ScalarType`)
is known to *both* generated endpoints at codegen time, so the
encoder and decoder for a given data symbol are emitted as a matched
pair. The chosen encoding is **fixed little-endian** for every scalar
width, element by element for arrays (`Vec<T>` is `len` elements of
`T`, each as its native-width LE bytes, concatenated; the 8-byte
`length` header already carries the byte count so no element count is
sent). LE is chosen because every example datum is produced/consumed
on the same x86-64 host (loopback); a backend that ever targets a
mixed-endian pair would bump the protocol version (which v0 does not
have — see above), not silently mis-decode.

## Design questions (AC#5, recorded)

- *Why no checksum / version byte / worker-id tag?* — above; each is
  a redundant or impossible-failure-mode feature for a loopback-only,
  single-compiler, one-connection-per-pair v0.
- *Why carry `seq_tag` at all if it is not routing?* — it is the
  cheapest possible fail-loud cross-check that the deterministic
  event order on a connection actually matches between the two
  independently-emitted endpoints. A `seq_tag` mismatch means the
  projection paired Push/Wait differently than the connection
  delivered them — a contract regression worth a hard error, not a
  silent wrong result.

## Honest limitations (AC#6, recorded)

- Loopback-only; unsuitable for cross-host transport (no auth, no
  integrity beyond TCP's, no endianness negotiation).
- One allocation per transfer (the payload `Vec<u8>`); no buffer-pool
  reuse at M3. Performance is explicitly not a goal (PRD §7.1, M3 is
  about *differential correctness*, not throughput).
- The `length` field is a `u64` but a single message must fit in
  memory on both ends (it is read into one `Vec<u8>`); there is no
  streaming/chunked mode in v0.
