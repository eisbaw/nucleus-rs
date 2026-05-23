---
id: TASK-0120
title: capabilities.toml schema_version field
status: Done
assignee: []
created_date: '2026-05-18 01:58'
updated_date: '2026-05-23 21:20'
labels:
  - M3
  - backend
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0019 follow-up: capabilities.toml has no schema_version field today, and the parser uses serde's deny_unknown_fields so any forward-incompatible field addition is rejected loudly. Add a top-level schema_version field (probably u32) that the parser reads first, then relaxes unknown-field handling per version-gated rules. This unblocks future capability additions without breaking older backend crates.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Capabilities struct gains schema_version: u32 (TASK-0120); SUPPORTED_SCHEMA_VERSIONS constant defines allowed set; #[serde(default = "default_schema_version")] backward-compats pre-existing capabilities.toml
- [x] #2 All 4 tier-1 backend capabilities.toml files declare schema_version = 1 explicitly (going-forward convention; default exists only for backward-compat with older files)
- [x] #3 CapError::UnsupportedSchemaVersion { found, supported } variant + Display rejects future-schema files with a precise message
- [x] #4 3 new tests in nucleus-compiler/tests/capabilities.rs pin: default-when-missing, explicit-=1, negative-unsupported-version
- [x] #5 Gate clean: cargo check + clippy + cargo test --test capabilities (24/24, was 21) + just e2e 88/70/0/18 unchanged
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed orchestrator-direct (cycle 77 continuation). Implementation: added 'pub schema_version: u32' as the first field of Capabilities with #[serde(default = "default_schema_version")] backing the v1 default; added SUPPORTED_SCHEMA_VERSIONS const &[1]; added CapError::UnsupportedSchemaVersion { found, supported } + Display; wired validate() to check membership in SUPPORTED_SCHEMA_VERSIONS BEFORE the existing tier check (so a clearly-wrong file fails on schema_version, not on incidental field errors). All 4 tier-1 backend capabilities.toml files updated to declare schema_version = 1 explicitly. 3 new tests pin the contract. Module-level doc-block (line 25-29) updated from 'a future schema_version field is needed' to 'TASK-0120 (cycle 77) added the schema_version: u32 field as the version-gating substrate'. Forward-compat path: when a future field is added in v2 of the schema, append 2 to SUPPORTED_SCHEMA_VERSIONS, gate the new field's deserialise on schema_version >= 2 via a custom impl, and the v1 default continues to work for files that don't need the new field.
<!-- SECTION:FINAL_SUMMARY:END -->
