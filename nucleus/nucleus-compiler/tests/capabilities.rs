//! Integration tests for the capability matrix loader and the
//! schedule-vs-backend compatibility check (TASK-0019).
//!
//! Test strategy:
//!
//! - **Positive parse**: a synthetic `capabilities.toml` mirroring the
//!   PRD §7.4 example deserialises with every field correctly typed.
//! - **Round-trip**: serialise the in-memory struct back to TOML,
//!   re-parse, expect equality.
//! - **Negative parse**: 4+ malformed inputs. Each exercises a
//!   distinct [`CapError`] variant (unknown transport, unknown notify,
//!   missing required field, unknown extra field, invalid tier).
//! - **Compatibility — positive**: a synthetic schedule paired with a
//!   capable backend passes cleanly.
//! - **Compatibility — negative**: 4+ synthetic mismatches. Each pair
//!   exercises a distinct [`CapMismatch`] variant.
//!
//! Schedule fixtures use the schedule sublanguage's lowered IR
//! ([`SchedIR`]) hand-built in-memory; tests don't go through the
//! parser. Wiring through the parser is a larger integration that
//! belongs in M1's e2e harness — for unit-test purposes the
//! hand-built IR exercises the check pass at the same layer the
//! production caller will.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nucleus_compiler::capabilities::{
    check_schedule_compat, load_capabilities, CapError, CapMismatch, Capabilities, NotifyMode,
    Transport,
};
use nucleus_compiler::sched::{
    NotifyKind, ResolvedMemoryRegion, ResolvedPlaceData, ResolvedTransferDirective,
    ResolvedTransferOption, ResolvedWorker, ResolvedWorkerClass, SchedIR, DEFAULT_WORKER_CLASS,
};

// --------------------------------------------------------------------
// Fixture helpers
// --------------------------------------------------------------------

/// The PRD §7.4 example, verbatim, plus a tier and a name (which the
/// PRD example shows in the file-level comment, not as fields, but
/// our schema makes them mandatory).
const EXAMPLE_TOML: &str = r#"
name            = "mp-tcp-event"
tier            = 1
transport       = "tcp"
notify          = ["event"]
supports_async  = true
supports_buffer = true
max_buffer      = 1024
worker_classes  = ["default"]
memory_regions  = ["heap"]
"#;

fn example_caps() -> Capabilities {
    Capabilities {
        schema_version: 1,
        name: "mp-tcp-event".to_string(),
        tier: 1,
        transport: Transport::Tcp,
        notify: vec![NotifyMode::Event],
        supports_async: true,
        supports_buffer: true,
        max_buffer: 1024,
        worker_classes: vec!["default".to_string()],
        memory_regions: vec!["heap".to_string()],
    }
}

/// Build a minimal SchedIR with a single simple-form worker on the
/// synthetic default class, one transfer directive, and one
/// `place_data` entry into the `heap` region.
fn fixture_sched() -> SchedIR {
    let mut workers: BTreeMap<String, ResolvedWorker> = BTreeMap::new();
    workers.insert(
        "host".to_string(),
        ResolvedWorker {
            name: "host".to_string(),
            class: DEFAULT_WORKER_CLASS.to_string(),
        },
    );

    let mut worker_classes: BTreeMap<String, ResolvedWorkerClass> = BTreeMap::new();
    worker_classes.insert(
        DEFAULT_WORKER_CLASS.to_string(),
        ResolvedWorkerClass {
            name: DEFAULT_WORKER_CLASS.to_string(),
            simd: None,
            memory: None,
            is_default: true,
        },
    );

    let mut memory_regions: BTreeMap<String, ResolvedMemoryRegion> = BTreeMap::new();
    memory_regions.insert(
        "heap".to_string(),
        ResolvedMemoryRegion {
            name: "heap".to_string(),
            size_bytes: None,
            accessible_by: None,
            per_worker: None,
        },
    );

    let mut place_data: BTreeMap<String, ResolvedPlaceData> = BTreeMap::new();
    place_data.insert(
        "img".to_string(),
        ResolvedPlaceData {
            data: "img".to_string(),
            region: "heap".to_string(),
            // TASK-0099: hand-built test fixture has no source text.
            data_span: None,
        },
    );

    let mut transfers: BTreeMap<String, ResolvedTransferDirective> = BTreeMap::new();
    transfers.insert(
        "img".to_string(),
        ResolvedTransferDirective {
            data: "img".to_string(),
            options: vec![ResolvedTransferOption::Sync],
            // TASK-0099: hand-built test fixture has no source text.
            data_span: None,
        },
    );

    SchedIR {
        algo_path: "prog.algo.nuc".to_string(),
        worker_classes,
        memory_regions,
        workers,
        places: BTreeMap::new(),
        place_data,
        loops: BTreeMap::new(),
        transfers,
        checks: BTreeMap::new(),
    }
}

/// Set the option list on the single `transfer img : ...` directive.
fn set_transfer_options(sched: &mut SchedIR, options: Vec<ResolvedTransferOption>) {
    sched.transfers.get_mut("img").unwrap().options = options;
}

/// RAII wrapper around a temp file written from a string. Deletes on
/// drop. Tempfile crate avoided to keep the MSRV dependency surface
/// small (TASK-0019 notes).
struct TempToml {
    path: PathBuf,
}

impl TempToml {
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempToml {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn write_tmp(src: &str) -> TempToml {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let mut path = std::env::temp_dir();
    path.push(format!("nucleus-caps-{pid}-{n}.toml"));
    std::fs::write(&path, src).expect("temp file write");
    TempToml { path }
}

// --------------------------------------------------------------------
// Positive: parse the PRD example
// --------------------------------------------------------------------

#[test]
fn parse_prd_example_fields_correctly_typed() {
    let caps: Capabilities = toml::from_str(EXAMPLE_TOML).expect("parse");
    assert_eq!(caps, example_caps());
}

#[test]
fn load_capabilities_from_disk() {
    let f = write_tmp(EXAMPLE_TOML);
    let caps = load_capabilities(f.path()).expect("load");
    assert_eq!(caps, example_caps());
}

// --------------------------------------------------------------------
// Round-trip
// --------------------------------------------------------------------

#[test]
fn round_trip_serialize_then_parse() {
    let original = example_caps();
    let s = toml::to_string(&original).expect("serialize");
    let parsed: Capabilities = toml::from_str(&s).expect("re-parse");
    assert_eq!(original, parsed);
}

#[test]
fn round_trip_all_notify_variants() {
    let mut caps = example_caps();
    caps.notify = vec![
        NotifyMode::Event,
        NotifyMode::Poll,
        NotifyMode::Barrier,
        NotifyMode::Blocking,
        NotifyMode::Irq,
    ];
    let s = toml::to_string(&caps).expect("serialize");
    let parsed: Capabilities = toml::from_str(&s).expect("re-parse");
    assert_eq!(caps, parsed);
}

#[test]
fn round_trip_all_transport_variants() {
    for t in [
        Transport::SharedMemory,
        Transport::Tcp,
        Transport::Uds,
        Transport::Mpi,
        Transport::EmbeddedDma,
    ] {
        let mut caps = example_caps();
        caps.transport = t;
        let s = toml::to_string(&caps).expect("serialize");
        let parsed: Capabilities = toml::from_str(&s).expect("re-parse");
        assert_eq!(caps, parsed, "transport {t:?} round-trip");
    }
}

// --------------------------------------------------------------------
// Negative: malformed inputs
// --------------------------------------------------------------------

#[test]
fn negative_unknown_transport() {
    let src = EXAMPLE_TOML.replace(
        r#"transport       = "tcp""#,
        r#"transport       = "carrier-pigeon""#,
    );
    let f = write_tmp(&src);
    let err = load_capabilities(f.path()).unwrap_err();
    assert!(
        matches!(err, CapError::UnknownTransport(ref s) if s == "carrier-pigeon"),
        "unexpected error: {err:?}",
    );
}

#[test]
fn negative_unknown_notify_mode() {
    let src = EXAMPLE_TOML.replace(
        r#"notify          = ["event"]"#,
        r#"notify          = ["smoke-signal"]"#,
    );
    let f = write_tmp(&src);
    let err = load_capabilities(f.path()).unwrap_err();
    assert!(
        matches!(err, CapError::UnknownNotify(ref s) if s == "smoke-signal"),
        "unexpected error: {err:?}",
    );
}

#[test]
fn negative_missing_required_field() {
    // Drop the `name` line entirely.
    let src = EXAMPLE_TOML.replace("name            = \"mp-tcp-event\"\n", "");
    let f = write_tmp(&src);
    let err = load_capabilities(f.path()).unwrap_err();
    assert!(
        matches!(err, CapError::MissingField(ref s) if s == "name"),
        "unexpected error: {err:?}",
    );
}

#[test]
fn negative_invalid_tier() {
    let src = EXAMPLE_TOML.replace("tier            = 1", "tier            = 7");
    let f = write_tmp(&src);
    let err = load_capabilities(f.path()).unwrap_err();
    assert_eq!(err, CapError::InvalidTier(7));
}

#[test]
fn negative_unknown_extra_field() {
    let src = format!("{EXAMPLE_TOML}\nbonus_feature = \"sparkles\"\n");
    let f = write_tmp(&src);
    let err = load_capabilities(f.path()).unwrap_err();
    assert!(
        matches!(err, CapError::UnknownField(ref s) if s == "bonus_feature"),
        "unexpected error: {err:?}",
    );
}

#[test]
fn negative_io_missing_file() {
    let err =
        load_capabilities(std::path::Path::new("/this/path/does/not/exist.toml")).unwrap_err();
    assert!(
        matches!(err, CapError::Io { .. }),
        "unexpected error: {err:?}",
    );
}

#[test]
fn task_0120_schema_version_defaults_to_1_when_missing() {
    // Pre-TASK-0120 capabilities.toml files have no schema_version
    // field. The loader must accept them (backward-compat) and assign
    // schema_version=1 — the only version that ever existed.
    let src = EXAMPLE_TOML; // EXAMPLE_TOML has no schema_version line
    assert!(
        !src.contains("schema_version"),
        "EXAMPLE_TOML fixture must NOT contain schema_version for this test \
         to exercise the default — fixture drift broke the test"
    );
    let f = write_tmp(src);
    let caps = load_capabilities(f.path())
        .expect("missing schema_version must deserialise to default=1");
    assert_eq!(
        caps.schema_version, 1,
        "missing schema_version must default to 1 (TASK-0120 backward-compat); got {}",
        caps.schema_version
    );
}

#[test]
fn task_0120_schema_version_explicit_1_parses() {
    // Going forward, capabilities.toml SHOULD declare schema_version=1
    // explicitly. Verify the explicit form parses identically.
    let src = format!("schema_version  = 1\n{EXAMPLE_TOML}");
    let f = write_tmp(&src);
    let caps = load_capabilities(f.path())
        .expect("explicit schema_version = 1 must parse cleanly");
    assert_eq!(caps.schema_version, 1);
}

#[test]
fn task_0120_negative_unsupported_schema_version() {
    // A future-schema capabilities.toml that this build can't
    // interpret must fail LOUD with UnsupportedSchemaVersion, not
    // ParseFailed or silent acceptance. Version 999 is well outside
    // SUPPORTED_SCHEMA_VERSIONS.
    let src = format!("schema_version  = 999\n{EXAMPLE_TOML}");
    let f = write_tmp(&src);
    let err = load_capabilities(f.path())
        .expect_err("schema_version=999 must fail loud (not in SUPPORTED_SCHEMA_VERSIONS)");
    assert!(
        matches!(err, CapError::UnsupportedSchemaVersion { found: 999, ref supported } if supported == &vec![1]),
        "expected UnsupportedSchemaVersion {{found: 999, supported: [1]}}, got {err:?}"
    );
}

// --------------------------------------------------------------------
// Compatibility check — positive
// --------------------------------------------------------------------

#[test]
fn compat_default_schedule_passes() {
    let caps = example_caps();
    let sched = fixture_sched();
    check_schedule_compat(&caps, &sched).expect("compat ok");
}

#[test]
fn compat_async_buffer_event_passes_when_supported() {
    let caps = example_caps();
    let mut sched = fixture_sched();
    set_transfer_options(
        &mut sched,
        vec![
            ResolvedTransferOption::Async,
            ResolvedTransferOption::Buffer(8),
            ResolvedTransferOption::Notify(NotifyKind::Event),
        ],
    );
    check_schedule_compat(&caps, &sched).expect("compat ok");
}

#[test]
fn compat_buffer_eq_1_does_not_trip_supports_buffer_false() {
    // buffer=1 is the default "no extra buffering"; even on a backend
    // with supports_buffer=false it must pass.
    let mut caps = example_caps();
    caps.supports_buffer = false;
    let mut sched = fixture_sched();
    set_transfer_options(&mut sched, vec![ResolvedTransferOption::Buffer(1)]);
    check_schedule_compat(&caps, &sched).expect("compat ok");
}

// --------------------------------------------------------------------
// Compatibility check — negative (one test per CapMismatch variant)
// --------------------------------------------------------------------

#[test]
fn compat_negative_async_not_supported() {
    let mut caps = example_caps();
    caps.supports_async = false;
    let mut sched = fixture_sched();
    set_transfer_options(&mut sched, vec![ResolvedTransferOption::Async]);
    let errs = check_schedule_compat(&caps, &sched).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| matches!(e, CapMismatch::AsyncNotSupported { data } if data == "img")),
        "errs = {errs:?}",
    );
}

#[test]
fn compat_negative_buffer_not_supported() {
    let mut caps = example_caps();
    caps.supports_buffer = false;
    let mut sched = fixture_sched();
    set_transfer_options(&mut sched, vec![ResolvedTransferOption::Buffer(4)]);
    let errs = check_schedule_compat(&caps, &sched).unwrap_err();
    assert!(
        errs.iter().any(|e| matches!(
            e,
            CapMismatch::BufferNotSupported { data, requested: 4 } if data == "img"
        )),
        "errs = {errs:?}",
    );
}

#[test]
fn compat_negative_buffer_too_large() {
    let mut caps = example_caps();
    caps.max_buffer = 8;
    let mut sched = fixture_sched();
    set_transfer_options(&mut sched, vec![ResolvedTransferOption::Buffer(16)]);
    let errs = check_schedule_compat(&caps, &sched).unwrap_err();
    assert!(
        errs.iter().any(|e| matches!(
            e,
            CapMismatch::BufferTooLarge { data, requested: 16, max: 8 } if data == "img"
        )),
        "errs = {errs:?}",
    );
}

#[test]
fn compat_negative_notify_not_supported() {
    let mut caps = example_caps();
    caps.notify = vec![NotifyMode::Poll]; // schedule will ask for Event
    let mut sched = fixture_sched();
    set_transfer_options(
        &mut sched,
        vec![ResolvedTransferOption::Notify(NotifyKind::Event)],
    );
    let errs = check_schedule_compat(&caps, &sched).unwrap_err();
    assert!(
        errs.iter().any(|e| matches!(
            e,
            CapMismatch::NotifyModeNotSupported {
                data,
                requested: NotifyMode::Event,
            } if data == "img"
        )),
        "errs = {errs:?}",
    );
}

#[test]
fn compat_negative_worker_class_not_supported() {
    // Capability declares only `compute_core`; schedule has a worker
    // bound to the synthetic default class which is reported as
    // `"default"` on the capability side and not present.
    let mut caps = example_caps();
    caps.worker_classes = vec!["compute_core".to_string()];
    let sched = fixture_sched();
    let errs = check_schedule_compat(&caps, &sched).unwrap_err();
    assert!(
        errs.iter().any(|e| matches!(
            e,
            CapMismatch::WorkerClassNotSupported { class } if class == "default"
        )),
        "errs = {errs:?}",
    );
}

#[test]
fn compat_negative_memory_region_not_supported() {
    let mut caps = example_caps();
    caps.memory_regions = vec!["tcm".to_string()]; // not "heap"
    let sched = fixture_sched();
    let errs = check_schedule_compat(&caps, &sched).unwrap_err();
    assert!(
        errs.iter().any(|e| matches!(
            e,
            CapMismatch::MemoryRegionNotSupported { data, region }
                if data == "img" && region == "heap"
        )),
        "errs = {errs:?}",
    );
}

#[test]
fn compat_multiple_mismatches_reported_at_once() {
    // Three independent mismatches on one schedule: prove the pass
    // batches errors instead of bailing on the first.
    let mut caps = example_caps();
    caps.supports_async = false;
    caps.supports_buffer = false;
    caps.notify = vec![NotifyMode::Poll];
    let mut sched = fixture_sched();
    set_transfer_options(
        &mut sched,
        vec![
            ResolvedTransferOption::Async,
            ResolvedTransferOption::Buffer(4),
            ResolvedTransferOption::Notify(NotifyKind::Event),
        ],
    );
    let errs = check_schedule_compat(&caps, &sched).unwrap_err();
    assert!(
        errs.len() >= 3,
        "expected >=3 mismatches, got {} -> {errs:?}",
        errs.len(),
    );
    assert!(errs
        .iter()
        .any(|e| matches!(e, CapMismatch::AsyncNotSupported { .. })));
    assert!(errs
        .iter()
        .any(|e| matches!(e, CapMismatch::BufferNotSupported { .. })));
    assert!(errs
        .iter()
        .any(|e| matches!(e, CapMismatch::NotifyModeNotSupported { .. })));
}
