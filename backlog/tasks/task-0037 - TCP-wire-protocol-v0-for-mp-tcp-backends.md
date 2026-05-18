---
id: TASK-0037
title: TCP wire protocol v0 for mp-tcp backends
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
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
- [ ] #1 docs/wire-protocol-v0.md describes byte layout: 8 bytes length (little-endian u64), 8 bytes SeqTag (LE u64), then payload bytes.
- [ ] #2 No framing, no checksums, no version negotiation. Loopback only; not over the wire to untrusted hosts.
- [ ] #3 Sender and receiver agree on byte ordering at compile time from the Nucleus IR's type info.
- [ ] #4 Test: a Rust unit test round-trips canned payloads through the protocol; the test lives in a shared mp-tcp-common crate.
- [ ] #5 Implementation notes record design questions (e.g. why no checksum, why no version byte, why no per-message worker-id tag).
- [ ] #6 Implementation notes record honest limitations (loopback-only assumption; the format is unsuitable for cross-host transport).
<!-- AC:END -->
