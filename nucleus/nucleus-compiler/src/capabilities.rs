//! Capability matrix loader and schedule-vs-backend compatibility
//! check (TASK-0019, PRD §7.4).
//!
//! Each backend crate ships a sibling `capabilities.toml` text file
//! that declares what it can do. The schema is documented in
//! `docs/capabilities-toml.md`. This module:
//!
//! - Defines the typed [`Capabilities`] struct that `capabilities.toml`
//!   deserialises into.
//! - Exposes [`load_capabilities`] to read a file off disk with
//!   contextual errors.
//! - Exposes [`check_schedule_compat`] to verify a [`SchedIR`] is
//!   satisfiable on the given backend; mismatches accumulate into a
//!   `Vec<CapMismatch>` so the user sees every offending field at
//!   once, not just the first.
//!
//! Design choices:
//!
//! - Closed enums for `transport` and `notify` element values. Typos
//!   in source fail loudly at parse time. PRD §7.4 example uses
//!   string values verbatim; serde's `rename_all = "kebab-case"` on
//!   the transport enum keeps the source surface human-readable
//!   (`"shared-memory"` not `"SharedMemory"`).
//!
//! - `deny_unknown_fields` on the top-level struct. Unknown keys are
//!   loud errors, so a forward-incompatible field addition surfaces
//!   instead of being silently ignored. TASK-0120 (cycle 77) added the
//!   `schema_version: u32` field as the version-gating substrate; the
//!   current `deny_unknown_fields` remains in effect for v1, and a
//!   future field addition will rev `SUPPORTED_SCHEMA_VERSIONS` and
//!   per-version-gate the unknown-field handling at that point.
//!
//! - The check is *batched*. PRD §13 explicitly calls for "all errors
//!   at once" reporting; same single-pass-collect-all-errors policy
//!   the link step uses (TASK-0011).
//!
//! What this module does NOT do (deliberately):
//!
//! - Resolve `capabilities.toml` relative to a backend crate's source
//!   path. That's the build driver's job; this module takes a `Path`
//!   the caller has already resolved.
//! - Cross-check `transport` against schedule directives. There is no
//!   schedule surface that selects a transport directly (PRD §6.3
//!   chooses *behaviours* via `transfer` options; the transport is a
//!   backend property). The field is parsed and exposed so codegen
//!   can branch on it.
//! - Validate the algorithm side. Capability checking happens after
//!   linking (TASK-0011) — we trust the SchedIR is fully lowered.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sched::{NotifyKind, ResolvedTransferOption, SchedIR, DEFAULT_WORKER_CLASS};

// --------------------------------------------------------------------
// Typed values
// --------------------------------------------------------------------

/// Wire / channel a backend uses for cross-worker data transfers.
/// PRD §7.1–7.3 cover the values; capabilities.toml uses kebab-case
/// strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    /// Shared-memory between threads / OS processes.
    SharedMemory,
    /// TCP sockets (loopback for tier-1 `mp-tcp-*`; cross-host
    /// elsewhere).
    Tcp,
    /// Unix domain sockets.
    Uds,
    /// MPI ranks.
    Mpi,
    /// Embedded DMA descriptors.
    EmbeddedDma,
}

/// Notification modes a backend supports. Broader than the schedule's
/// [`NotifyKind`] (which today only includes `event` and `poll`) — the
/// capabilities side declares the wider set so the schedule surface
/// can grow without recompiling the capability parser. See PRD §6.3.4
/// and the schema doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotifyMode {
    Event,
    Poll,
    Barrier,
    Blocking,
    Irq,
}

impl fmt::Display for NotifyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            NotifyMode::Event => "event",
            NotifyMode::Poll => "poll",
            NotifyMode::Barrier => "barrier",
            NotifyMode::Blocking => "blocking",
            NotifyMode::Irq => "irq",
        };
        f.write_str(s)
    }
}

impl From<NotifyKind> for NotifyMode {
    fn from(k: NotifyKind) -> Self {
        match k {
            NotifyKind::Event => NotifyMode::Event,
            NotifyKind::Poll => NotifyMode::Poll,
        }
    }
}

/// Supported `schema_version` values. The loader rejects any value
/// not in this list with [`CapError::UnsupportedSchemaVersion`].
/// Currently `&[1]`; a future schema revision appends a new version
/// here and gates the changed parsing rules on it (TASK-0120).
pub const SUPPORTED_SCHEMA_VERSIONS: &[u32] = &[1];

/// Default `schema_version` for older `capabilities.toml` files that
/// pre-date TASK-0120. Backward-compat: missing field deserialises to
/// v1 (the only version that ever existed pre-this-task), so existing
/// backend crates parse unchanged. Going forward, every new
/// `capabilities.toml` SHOULD declare `schema_version = N` explicitly
/// — the default exists only to avoid breaking older files.
fn default_schema_version() -> u32 {
    1
}

/// Default for the three topology/mediation flags (TASK-0455.09). They
/// default to `false` so a `capabilities.toml` written before this task
/// (or for a future shared-memory / native-w2w backend) parses unchanged
/// and selects NO host-mediation passes — the same behaviour the deleted
/// driver name-lists gave for any backend not in them. Every CURRENT
/// backend declares all three explicitly (the schema doc + the driver's
/// equivalence test require it); the default exists only so the field
/// addition is not a hard breaking change for off-tree files.
fn default_false() -> bool {
    false
}

/// The capability matrix declared by a single backend.
///
/// One-to-one with `capabilities.toml`. The serde-derived
/// `Deserialize` enforces the schema (closed enums, unknown fields
/// rejected). See `docs/capabilities-toml.md` for the field semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// Schema version of THIS `capabilities.toml` file (TASK-0120).
    /// Currently always `1`. Defaults to `1` if the field is missing
    /// (backward-compat with pre-TASK-0120 files). When future
    /// capability fields are added, they will be gated on
    /// `schema_version >= N`, and the loader will reject older files
    /// only if they need a feature not available in their declared
    /// version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Backend identifier; matches the crate name.
    pub name: String,
    /// Target tier (1, 2, or 3). Validated post-deserialise via
    /// `Capabilities::validate`; the raw type is `u8` because TOML
    /// integers parse to a signed range that we widen here.
    pub tier: u8,
    /// Wire / channel kind.
    pub transport: Transport,
    /// Supported notification modes.
    pub notify: Vec<NotifyMode>,
    /// `true` iff the backend supports `async` transfers.
    pub supports_async: bool,
    /// `true` iff the backend supports in-flight buffering (i.e.
    /// `buffer=N` with `N > 1`).
    pub supports_buffer: bool,
    /// Largest `buffer=N` the backend can satisfy.
    pub max_buffer: u32,
    /// Worker classes the backend can host. `"default"` matches the
    /// schedule's simple-form synthetic class
    /// ([`DEFAULT_WORKER_CLASS`]).
    pub worker_classes: Vec<String>,
    /// Memory regions the backend supports.
    pub memory_regions: Vec<String>,

    // --- Topology / mediation flags (TASK-0455.09) -------------------
    //
    // These three booleans encode the wire-topology facts that used to
    // be hard-coded as three backend-NAME lists in the driver's pass
    // selection (driver/src/main.rs). The driver now reads them off the
    // loaded `Capabilities` instead of string-matching the backend name,
    // so a new platform declares its topology HERE (one place, reviewed
    // with the rest of the capability surface) and cannot silently miss
    // a list — the silent-sibling failure mode the lists invited. Each
    // selects exactly one compiler pass; see `docs/capabilities-toml.md`
    // §"Topology / mediation flags" for the prose and the per-backend
    // table. `Capabilities::validate` rejects a logically impossible
    // combination (relay / push-reorder without host mediation).
    /// `true` iff the backend has a host-mediated STAR topology with no
    /// native worker-to-worker barrier channel: every host-EXCLUDING
    /// barrier must be re-routed through the elected host to lower (the
    /// driver runs `apply_host_mediation_inject`). `false` for backends
    /// whose barrier primitive handles host-excluding barriers natively
    /// (shared-memory `std::sync::Barrier`, MPI `Comm_split` sub-comm
    /// barrier, embedded stub).
    #[serde(default = "default_false")]
    pub star_topology_host_mediation: bool,
    /// `true` iff the backend has no native worker-to-worker DATA
    /// channel, so every worker-to-worker `Push`/`Wait` pair must be
    /// relayed through the elected host (the driver runs
    /// `apply_host_data_relay_inject`). Implies
    /// `star_topology_host_mediation` (you cannot relay through a host
    /// that is not a mediating hub); `validate` enforces that.
    #[serde(default = "default_false")]
    pub host_data_relay: bool,
    /// `true` iff the backend's wait primitive is per-(seq) DEMUXED (an
    /// inbound queue keyed by sequence, not a strict per-pair FIFO
    /// stream), so a hoistable worker-to-worker `Push` can be safely
    /// moved ahead of a preceding `Wait` to break the host-relay
    /// wait-before-push deadlock (the driver runs
    /// `apply_safe_push_reorder`). Strict-FIFO transports (bufsync,
    /// poll) must NOT set this — moving a push ahead of a wait would
    /// race host's own w2w waits on the shared stream. Implies
    /// `star_topology_host_mediation` (the reorder only matters on the
    /// host-relay path, which only host-mediated backends have);
    /// `validate` enforces that.
    #[serde(default = "default_false")]
    pub reorderable_push: bool,
}

impl Capabilities {
    /// Post-deserialise sanity check. Called by [`load_capabilities`]
    /// and the round-trip path. Catches things the serde schema can't
    /// (tier range; the topology/mediation-flag cross-field implication
    /// rule — TASK-0455.09; duplicate elements in lists are NOT flagged,
    /// only folded for membership testing).
    ///
    /// `pub` so callers that construct a [`Capabilities`] in memory (the
    /// TASK-0455.09 topology-flag negative tests) can exercise the same
    /// validation path the loader runs, without going through a TOML
    /// round-trip or the filesystem.
    pub fn validate(&self) -> Result<(), CapError> {
        if !SUPPORTED_SCHEMA_VERSIONS.contains(&self.schema_version) {
            return Err(CapError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: SUPPORTED_SCHEMA_VERSIONS.to_vec(),
            });
        }
        if !matches!(self.tier, 1..=3) {
            return Err(CapError::InvalidTier(self.tier));
        }
        // Topology/mediation flag consistency (TASK-0455.09). Both
        // host-data-relay and push-reorder are operations performed ON
        // the host-relay path, which only exists when the backend is
        // host-mediated. Declaring either WITHOUT
        // `star_topology_host_mediation` is a logically impossible
        // surface: the driver would route a w2w transfer through a host
        // that is not a mediating hub, mis-mediating silently. Reject it
        // loudly here so a fat-fingered cap-file is caught at load time,
        // not as a runtime topology mismatch (the failure class the
        // deleted driver name-lists invited). This is the schema's first
        // CROSS-FIELD consistency rule; the flat per-field checks above
        // cannot express it.
        if self.host_data_relay && !self.star_topology_host_mediation {
            return Err(CapError::InconsistentTopologyFlags {
                detail: "`host_data_relay = true` requires \
                         `star_topology_host_mediation = true` (a worker-to-worker \
                         transfer cannot be relayed through a host that is not a \
                         mediating hub)"
                    .to_string(),
            });
        }
        if self.reorderable_push && !self.star_topology_host_mediation {
            return Err(CapError::InconsistentTopologyFlags {
                detail: "`reorderable_push = true` requires \
                         `star_topology_host_mediation = true` (the safe-push reorder \
                         only applies on the host-relay path, which only host-mediated \
                         backends have)"
                    .to_string(),
            });
        }
        Ok(())
    }
}

// --------------------------------------------------------------------
// Loader
// --------------------------------------------------------------------

/// Load and validate a `capabilities.toml` from disk.
///
/// Failures (in order of precedence):
/// - I/O error reading the file -> [`CapError::Io`].
/// - TOML parse / serde error -> [`CapError::ParseFailed`]. The serde
///   layer's error string is preserved verbatim; it already names the
///   offending field / unknown variant / mistyped value. Pattern-match
///   on the error kind to learn whether it was an unknown transport
///   / notify mode / etc., or read the embedded message.
/// - Range failure on `tier` -> [`CapError::InvalidTier`].
pub fn load_capabilities(path: &Path) -> Result<Capabilities, CapError> {
    let src = fs::read_to_string(path).map_err(|e| CapError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    let caps: Capabilities = toml::from_str(&src).map_err(|e| classify_de_error(&e))?;
    caps.validate()?;
    Ok(caps)
}

/// Best-effort classification of a toml deserialisation error into a
/// more specific [`CapError`] variant. Falls back to `ParseFailed`
/// when the message doesn't match a known pattern.
///
/// This is string-scraping. The serde layer doesn't expose a structured
/// "which variant failed to deserialise" hook for closed enums in this
/// version of `toml`. The scraping is targeted: it looks for the
/// canonical error message shape that `toml`/serde produce for unknown
/// enum variants, and only the patterns we care about. Anything else
/// goes through as `ParseFailed`, which is honest: the user gets the
/// raw serde message, which already names the field.
fn classify_de_error(err: &toml::de::Error) -> CapError {
    let msg = err.to_string();
    let lower = msg.to_ascii_lowercase();
    // `toml`'s formatted error mentions the field name in context;
    // we use the message itself to route to the right variant.
    if lower.contains("unknown variant") && lower.contains("transport") {
        return CapError::UnknownTransport(extract_quoted(&msg));
    }
    if lower.contains("unknown variant") && lower.contains("notify") {
        return CapError::UnknownNotify(extract_quoted(&msg));
    }
    if lower.contains("missing field") {
        return CapError::MissingField(extract_quoted(&msg));
    }
    if lower.contains("unknown field") {
        return CapError::UnknownField(extract_quoted(&msg));
    }
    CapError::ParseFailed(msg)
}

/// Pull the first single-quoted token out of a serde error message
/// (e.g. `unknown variant `pigeon-mail`, expected one of ...` ->
/// `pigeon-mail`). Falls back to an empty string if the format
/// changes; the variant still carries the full message via the caller
/// preserving it on `ParseFailed`.
fn extract_quoted(msg: &str) -> String {
    let mut chars = msg.chars();
    while let Some(c) = chars.next() {
        if c == '`' {
            let rest: String = chars.by_ref().take_while(|&c| c != '`').collect();
            return rest;
        }
    }
    String::new()
}

// --------------------------------------------------------------------
// Compatibility check
// --------------------------------------------------------------------

/// One mismatch between a schedule directive and a backend capability.
///
/// Mirrors the variant set named in TASK-0019 plus
/// `WorkerClassNotSupported` and `MemoryRegionNotSupported` for the
/// worker_classes / memory_regions axes mentioned by the task and PRD
/// §7.4.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapMismatch {
    /// Schedule's transfer is `async` but the backend declares
    /// `supports_async = false`.
    AsyncNotSupported { data: String },
    /// Schedule's transfer asks for `buffer=N` with `N > 1` but the
    /// backend declares `supports_buffer = false`.
    BufferNotSupported { data: String, requested: u64 },
    /// Schedule's transfer asks for `buffer=N` with `N > max_buffer`.
    BufferTooLarge {
        data: String,
        requested: u64,
        max: u32,
    },
    /// Schedule's transfer asks for `notify=M` not in `caps.notify`.
    NotifyModeNotSupported { data: String, requested: NotifyMode },
    /// Schedule declares a worker class not in `caps.worker_classes`.
    WorkerClassNotSupported { class: String },
    /// Schedule's `place_data D in R` names a region not in
    /// `caps.memory_regions`.
    MemoryRegionNotSupported { data: String, region: String },
}

impl fmt::Display for CapMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapMismatch::AsyncNotSupported { data } => write!(
                f,
                "transfer `{data}` requests `async` but the backend does not \
                 support async transfers"
            ),
            CapMismatch::BufferNotSupported { data, requested } => write!(
                f,
                "transfer `{data}` requests `buffer={requested}` but the \
                 backend does not support buffering"
            ),
            CapMismatch::BufferTooLarge {
                data,
                requested,
                max,
            } => write!(
                f,
                "transfer `{data}` requests `buffer={requested}` but the \
                 backend supports at most `buffer={max}`"
            ),
            CapMismatch::NotifyModeNotSupported { data, requested } => write!(
                f,
                "transfer `{data}` requests `notify={requested}` but the \
                 backend does not support that notification mode"
            ),
            CapMismatch::WorkerClassNotSupported { class } => write!(
                f,
                "schedule uses worker class `{class}` but the backend does \
                 not support it"
            ),
            CapMismatch::MemoryRegionNotSupported { data, region } => write!(
                f,
                "`place_data {data} in {region}` names memory region \
                 `{region}` which the backend does not support"
            ),
        }
    }
}

/// Verify that every schedule demand is satisfied by the backend's
/// declared capabilities. Mismatches accumulate, sort, and dedup so
/// the caller sees every offending field at once.
///
/// The mapping from schedule directive to mismatch is:
///
/// - For each `ResolvedTransferDirective` in `sched.transfers`, look at
///   each option in source order:
///   * `Async` and `!caps.supports_async` -> `AsyncNotSupported`.
///   * `Buffer(N)` with `N > 1` and `!caps.supports_buffer` ->
///     `BufferNotSupported`.
///   * `Buffer(N)` with `N > caps.max_buffer` -> `BufferTooLarge`.
///   * `Notify(K)` with the lifted [`NotifyMode`] not in `caps.notify`
///     -> `NotifyModeNotSupported`.
/// - For each resolved worker, look up its class. If the class is the
///   synthetic [`DEFAULT_WORKER_CLASS`], it must satisfy `"default"
///   in caps.worker_classes`; otherwise the class name itself must be
///   in `caps.worker_classes`.
/// - For each `ResolvedPlaceData`, the region name must be in
///   `caps.memory_regions`.
pub fn check_schedule_compat(caps: &Capabilities, sched: &SchedIR) -> Result<(), Vec<CapMismatch>> {
    let mut mismatches: Vec<CapMismatch> = Vec::new();

    let notify_set: BTreeSet<NotifyMode> = caps.notify.iter().copied().collect();
    let class_set: BTreeSet<&str> = caps.worker_classes.iter().map(String::as_str).collect();
    let region_set: BTreeSet<&str> = caps.memory_regions.iter().map(String::as_str).collect();

    for (data, dir) in &sched.transfers {
        for opt in &dir.options {
            match opt {
                ResolvedTransferOption::Sync => {}
                ResolvedTransferOption::Async => {
                    if !caps.supports_async {
                        mismatches.push(CapMismatch::AsyncNotSupported { data: data.clone() });
                    }
                }
                ResolvedTransferOption::Buffer(n) => {
                    // N=1 is the default "no extra buffering"; only
                    // larger requests touch the buffer capability.
                    if *n > 1 && !caps.supports_buffer {
                        mismatches.push(CapMismatch::BufferNotSupported {
                            data: data.clone(),
                            requested: *n,
                        });
                    }
                    if *n > u64::from(caps.max_buffer) {
                        mismatches.push(CapMismatch::BufferTooLarge {
                            data: data.clone(),
                            requested: *n,
                            max: caps.max_buffer,
                        });
                    }
                }
                ResolvedTransferOption::Notify(k) => {
                    let mode: NotifyMode = (*k).into();
                    if !notify_set.contains(&mode) {
                        mismatches.push(CapMismatch::NotifyModeNotSupported {
                            data: data.clone(),
                            requested: mode,
                        });
                    }
                }
                // `mode=pio|dma` (TASK-0438.01): both transport modes are
                // accepted by every backend in this slice — codegen does
                // not yet diverge (TASK-0438.02). No capability requirement.
                // Exhaustive (no `_ =>`) on purpose: a future transport mode
                // that needs a capability gate must be considered here.
                ResolvedTransferOption::Transport(_) => {}
            }
        }
    }

    // Worker classes: every class that a worker references must be
    // supported. The synthetic default class collapses to "default"
    // on the capability side (schema doc).
    let mut seen_classes: BTreeSet<String> = BTreeSet::new();
    for worker in sched.workers.values() {
        let cap_class: &str = if worker.class == DEFAULT_WORKER_CLASS {
            "default"
        } else {
            worker.class.as_str()
        };
        if seen_classes.insert(cap_class.to_string()) && !class_set.contains(cap_class) {
            mismatches.push(CapMismatch::WorkerClassNotSupported {
                class: cap_class.to_string(),
            });
        }
    }

    // Memory regions: every place_data target must be a region the
    // backend supports.
    //
    // This check is LOAD-BEARING, not check-then-discard-dead. The
    // resolved region string is consumed *here* as an admission gate and
    // then not threaded further into codegen — but that is by design, not
    // a dropped fact: no backend yet consumes a region placement (the
    // `Event::Alloc`/`Region` contract surface is deliberately reserved,
    // see `crate::event` module-doc "DELIBERATELY RESERVED", TASK-0455.16).
    // The gate's job is to REJECT a schedule that requests physical
    // placement a backend cannot honour. Concretely it is exactly why
    // `14-hearing-aid/embedded_multimcu.sched.nuc` (which places four data
    // symbols `in sram_shared`) is rejected on the embedded backend
    // (`memory_regions = ["heap"]`), and why its sibling
    // `embedded_multimcu_sync.sched.nuc` — which carries no `place_data` —
    // is the variant the Renode gate compiles. When an accepted
    // `place_data` lands (the forward path in the `crate::event` note),
    // the resolved region becomes a sidecar fact the embedded render
    // reads; until then, gating admission is the whole job.
    for (data, pd) in &sched.place_data {
        if !region_set.contains(pd.region.as_str()) {
            mismatches.push(CapMismatch::MemoryRegionNotSupported {
                data: data.clone(),
                region: pd.region.clone(),
            });
        }
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        mismatches.sort();
        mismatches.dedup();
        Err(mismatches)
    }
}

// --------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------

/// Errors from loading or validating a `capabilities.toml`.
///
/// Variants are pattern-matchable; the embedded strings are for
/// formatting in the `Display` impl, not for re-parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapError {
    /// File could not be read (does not exist, permissions, ...).
    Io { path: PathBuf, message: String },
    /// TOML parse or serde error that we couldn't route to a more
    /// specific variant. The full message is preserved.
    ParseFailed(String),
    /// `transport = "..."` value is not in the closed enum.
    UnknownTransport(String),
    /// An element of `notify = [...]` is not in the closed enum.
    UnknownNotify(String),
    /// A required field is absent from the source.
    MissingField(String),
    /// A field name in the source is not part of the schema.
    UnknownField(String),
    /// `tier` is outside the allowed range `1..=3`.
    InvalidTier(u8),
    /// `schema_version` is not in [`SUPPORTED_SCHEMA_VERSIONS`]
    /// (TASK-0120). The file declares a version this build does not
    /// know how to interpret — most likely a newer file vs an older
    /// nucleus-compiler. `found` is the value in the source;
    /// `supported` is the list this build accepts.
    UnsupportedSchemaVersion { found: u32, supported: Vec<u32> },
    /// A logically impossible combination of the topology/mediation
    /// flags (TASK-0455.09): `host_data_relay` or `reorderable_push`
    /// set without `star_topology_host_mediation`. `detail` names the
    /// specific offending pair.
    InconsistentTopologyFlags { detail: String },
}

impl fmt::Display for CapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapError::Io { path, message } => {
                write!(f, "failed to read `{}`: {}", path.display(), message)
            }
            CapError::ParseFailed(m) => write!(f, "capability matrix parse error: {m}"),
            CapError::UnknownTransport(v) => {
                write!(f, "unknown `transport` value `{v}`")
            }
            CapError::UnknownNotify(v) => write!(f, "unknown `notify` value `{v}`"),
            CapError::MissingField(name) => {
                write!(f, "required field `{name}` missing from capabilities.toml")
            }
            CapError::UnknownField(name) => {
                write!(f, "unknown field `{name}` in capabilities.toml")
            }
            CapError::InvalidTier(t) => write!(f, "`tier = {t}` is invalid (allowed: 1, 2, 3)"),
            CapError::UnsupportedSchemaVersion { found, supported } => write!(
                f,
                "`schema_version = {found}` is not supported by this nucleus-compiler \
                 build (supported: {supported:?}) — most likely a capabilities.toml \
                 written for a newer schema; upgrade nucleus-compiler or downgrade the file"
            ),
            CapError::InconsistentTopologyFlags { detail } => write!(
                f,
                "inconsistent topology/mediation flags in capabilities.toml: {detail}"
            ),
        }
    }
}

impl std::error::Error for CapError {}
