---
id: TASK-0037
title: TCP wire protocol v0 for mp-tcp backends
status: Done
assignee:
  - '@mped'
created_date: '2026-05-17 23:07'
updated_date: '2026-05-19 00:51'
labels:
  - M3
  - backend
  - docs
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define the wire format: length prefix + opaque payload + SeqTag. Shared by all mp-tcp-* backends. Keep small; no extension headers in v0.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 docs/wire-protocol-v0.md describes byte layout: 8 bytes length (little-endian u64), 8 bytes SeqTag (LE u64), then payload bytes.
- [x] #2 No framing, no checksums, no version negotiation. Loopback only; not over the wire to untrusted hosts.
- [x] #3 Sender and receiver agree on byte ordering at compile time from the Nucleus IR's type info.
- [x] #4 Test: a Rust unit test round-trips canned payloads through the protocol; the test lives in a shared mp-tcp-common crate.
- [x] #5 Implementation notes record design questions (e.g. why no checksum, why no version byte, why no per-message worker-id tag).
- [x] #6 Implementation notes record honest limitations (loopback-only assumption; the format is unsuitable for cross-host transport).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0037 done. docs/wire-protocol-v0.md describes the exact byte layout: 8B LE u64 length + 8B LE u64 SeqTag + payload (AC#1). v0 has no framing/checksums/version negotiation, loopback-only, explicitly unsuitable for untrusted hosts (AC#2). Sender/receiver agree on byte order at compile time from the NameSidecar ResolvedType (fixed little-endian per scalar width; AC#3). Round-trip unit tests live in the shared mp-tcp-common crate (nucleus/mp-tcp-common): scalar/vec round-trips, fixed-LE byte pinning, framed messages over a real loopback TcpStream pair, seq-tag-mismatch fail-loud, two-party barrier (AC#4). Single source of truth: wire_runtime.rs is include!-d by the lib for tests AND exposed as WIRE_RUNTIME_SRC for the backend to copy verbatim into generated projects — the tested bytes ARE the emitted bytes (drift risk from TASK-0124 structurally eliminated).

AC#5 design questions: no checksum (loopback TCP already checksums; an app checksum would be dead weight that must also be byte-stable for the differential); no version byte (both endpoints emitted by the SAME nucleus build — no mixed-version deployment to negotiate); no per-message worker-id tag (exactly one connection per (host,worker) ordered pair — the endpoint IS the routing; a tag would be a second drift-prone source of truth). seq_tag travels not for routing but as the cheapest fail-loud cross-check that the deterministic event order matches between the two independently-emitted endpoints.

AC#6 honest limitations: loopback-only (no auth/integrity beyond TCP/endianness negotiation); one Vec<u8> allocation per transfer (no buffer-pool reuse at M3, perf not a goal); a single message must fit in memory on both ends (no streaming/chunked mode in v0).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Defined and implemented TCP wire protocol v0 for all mp-tcp-* backends.

What: docs/wire-protocol-v0.md (byte layout + rationale + limitations) and the mp-tcp-common crate (nucleus/mp-tcp-common). Wire frame = [8B LE u64 length][8B LE u64 SeqTag][payload]; barrier token = a zero-payload frame. Fixed little-endian scalar/array codecs, agreed at compile time from the NameSidecar type info.

Why: a small, shared, drift-free transport so the second tier-1 backend (mp-tcp-bufsync, TASK-0036) and any future mp-tcp-* backend lower Push/Wait/Sync identically.

Single source of truth: src/wire_runtime.rs is include!-d by the lib (so the unit tests exercise exactly these bytes) AND exposed as WIRE_RUNTIME_SRC for the backend to copy verbatim into generated projects.

Tests: 7 unit tests in mp-tcp-common (scalar/vec round-trip, fixed-LE byte pinning, framed round-trip over a real loopback TcpStream pair, seq-tag-mismatch fail-loud, ragged-length reject, two-party barrier). Non-flaky across repeated runs.

Limitations (recorded): loopback-only, no auth/integrity beyond TCP, one alloc per transfer, whole-message-in-memory. Perf is not a goal at M3.
<!-- SECTION:FINAL_SUMMARY:END -->
