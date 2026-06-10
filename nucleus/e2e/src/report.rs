//! Summary / JUnit / timings-JSON reporters + ANSI colour.
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// Reporting
// --------------------------------------------------------------------

/// ANSI colour codes. Only used when stdout is a TTY — falls back to
/// plain text under redirection (CI logs stay greppable).
pub(crate) mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const GREEN: &str = "\x1b[32m";
    pub const RED: &str = "\x1b[31m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const DIM: &str = "\x1b[2m";
}

pub(crate) fn use_color() -> bool {
    // We deliberately use *no* extra crate for isatty detection.
    // Honour `NO_COLOR` (the de facto opt-out) and assume colour off
    // when the harness is not invoked from a terminal context. The
    // best signal we have without a crate is `CARGO_TERM_COLOR`
    // (cargo sets `auto`/`always`/`never`) and the absence of
    // `CI`/`NO_COLOR`.
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    match env::var("CARGO_TERM_COLOR").as_deref() {
        Ok("never") => return false,
        Ok("always") => return true,
        _ => {}
    }
    // Default ON. The cells run under `just e2e` which the user
    // typically watches; ANSI codes in `tee` outputs are a minor
    // annoyance compared to losing colour in the common case.
    true
}

pub(crate) fn print_summary(results: &[CellResult]) {
    let colour = use_color();
    let pass = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::GREEN, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let fail = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::RED, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let skip = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::YELLOW, ansi::RESET)
        } else {
            s.to_string()
        }
    };
    let dim = |s: &str| {
        if colour {
            format!("{}{s}{}", ansi::DIM, ansi::RESET)
        } else {
            s.to_string()
        }
    };

    // Column widths sized off the longest entry, with a minimum so
    // headers don't crowd.
    let ex_w = results
        .iter()
        .map(|r| r.cell.example.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let sc_w = results
        .iter()
        .map(|r| r.cell.schedule.len())
        .max()
        .unwrap_or(8)
        .max(8);
    let be_w = results
        .iter()
        .map(|r| r.cell.backend.len())
        .max()
        .unwrap_or(7)
        .max(7);

    println!();
    println!("e2e matrix:");
    println!(
        "  {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   detail",
        "example",
        "schedule",
        "backend",
        "status",
        "time",
        ex_w = ex_w,
        sc_w = sc_w,
        be_w = be_w
    );
    println!(
        "  {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   {}",
        "-".repeat(ex_w),
        "-".repeat(sc_w),
        "-".repeat(be_w),
        "-".repeat(10),
        "-".repeat(8),
        "-".repeat(20),
        ex_w = ex_w,
        sc_w = sc_w,
        be_w = be_w
    );

    for r in results {
        let (status_str, detail) = match &r.status {
            Status::Pass => (pass("PASS"), String::new()),
            Status::Failed { phase, detail } => (fail(&format!("FAIL/{phase}")), detail.clone()),
            Status::Skipped { reason } => (skip("SKIPPED"), dim(reason)),
        };
        let mark = if r.required {
            // Required cells get an asterisk so a skim sees what
            // gates the exit code.
            "*"
        } else {
            " "
        };
        println!(
            "{mark} {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   {}",
            r.cell.example,
            r.cell.schedule,
            r.cell.backend,
            status_str,
            format_duration(r.timings.total()),
            detail,
            ex_w = ex_w,
            sc_w = sc_w,
            be_w = be_w
        );
    }
    println!();
    let total: usize = results.len();
    let passed: usize = results
        .iter()
        .filter(|r| matches!(r.status, Status::Pass))
        .count();
    let failed: usize = results
        .iter()
        .filter(|r| matches!(r.status, Status::Failed { .. }))
        .count();
    let skipped: usize = results
        .iter()
        .filter(|r| matches!(r.status, Status::Skipped { .. }))
        .count();
    let required_failed: usize = results
        .iter()
        .filter(|r| r.required && matches!(r.status, Status::Failed { .. }))
        .count();
    println!(
        "  total: {total}   pass: {passed}   fail: {failed}   skipped: {skipped}   \
         required-fail: {required_failed}"
    );
    println!("  (* = required cell)");
    println!();
}

pub(crate) fn format_duration(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s >= 10.0 {
        format!("{s:5.1}s")
    } else if s >= 1.0 {
        format!("{s:5.2}s")
    } else {
        format!("{:5}ms", d.as_millis())
    }
}

// --------------------------------------------------------------------
// TASK-0023.02: JUnit XML summary
// --------------------------------------------------------------------

/// Escape the five XML 1.0 special characters in a `<testcase>`-level
/// attribute or element value. `name`/`classname`/`message` attribute
/// values cannot contain `<`, `>`, `&`, `"`; element text/CDATA cannot
/// contain `<`/`&` unwrapped. Cell identifiers (example/schedule/
/// backend) are constrained by the manifest to ASCII identifiers
/// today, but a future manifest change could relax that — so be
/// defensive here rather than silently emit malformed XML if a name
/// gains a `&`.
pub(crate) fn xml_escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Render a failure `<detail>` payload safely inside a CDATA block.
/// A `]]>` sequence inside CDATA would end it prematurely, so split
/// it into two CDATA sections (`]]` + `]]>` ⇒ `]]` `]]>` ⇒ no early
/// terminator).
pub(crate) fn xml_escape_cdata(s: &str) -> String {
    s.replace("]]>", "]]]]><![CDATA[>")
}

/// Emit the matrix as a JUnit XML `<testsuites>` document on stdout.
///
/// Schema (TASK-0023.02 AC#2/#3):
///
///   * one `<testsuite>` wrapping every cell, `tests`/`failures`/
///     `errors=0`/`skipped` attributes;
///   * one `<testcase>` per cell with `classname="<example>.<schedule>"`,
///     `name="<backend>"`, `time="<elapsed_seconds>"`;
///   * PASS → empty element;
///   * SKIPPED → `<skipped message="<reason>"/>`;
///   * FAILED → `<failure type="<phase>">` with the detail wrapped in
///     CDATA. The redundant `message=<phase>` attr (cycle-53) was
///     dropped in TASK-0248 because it duplicated `type=` verbatim;
///     `message` is optional in JUnit and consumers fall back to the
///     `type` attr / body.
///
/// Bytes are written via `println!` so the output goes to stdout where
/// CI runners look for it.
pub(crate) fn print_summary_junit(results: &[CellResult], wall_clock: Option<Duration>) {
    let total = results.len();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, Status::Failed { .. }))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, Status::Skipped { .. }))
        .count();
    // TASK-0248: prefer the executor-measured wall-clock (honest under
    // --jobs N>=2). Fall back to summing per-cell elapsed when the
    // caller can't supply one — that path matches the old (cycle-53)
    // emit, which is still schema-legal and only overstates parallel
    // runs.
    let suite_time: f64 = match wall_clock {
        Some(d) => d.as_secs_f64(),
        None => results
            .iter()
            .map(|r| r.timings.total().as_secs_f64())
            .sum(),
    };

    println!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    println!(
        "<testsuites tests=\"{total}\" failures=\"{failed}\" errors=\"0\" \
         skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    println!(
        "  <testsuite name=\"nucleus-e2e\" tests=\"{total}\" failures=\"{failed}\" \
         errors=\"0\" skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    for r in results {
        let classname = xml_escape_attr(&format!("{}.{}", r.cell.example, r.cell.schedule));
        let name = xml_escape_attr(&r.cell.backend);
        let time_s = r.timings.total().as_secs_f64();
        match &r.status {
            Status::Pass => {
                // Empty element — JUnit consumers treat the absence of
                // <failure>/<skipped> children as a pass. No CDATA
                // body needed.
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\"/>"
                );
            }
            Status::Skipped { reason } => {
                let msg = xml_escape_attr(reason);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                println!("      <skipped message=\"{msg}\"/>");
                println!("    </testcase>");
            }
            Status::Failed { phase, detail } => {
                // TASK-0248: drop the redundant `message=` attribute —
                // the previous emit set message= to the same string as
                // type= (a comment-lie: two attrs with the same content
                // pretending to mean different things). `message` is
                // optional in JUnit; CI consumers fall back to either
                // the type attr or the CDATA body, so the structural
                // failure phase still surfaces via `type=`.
                let phase_attr = xml_escape_attr(&phase.to_string());
                let detail_cdata = xml_escape_cdata(detail);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                println!(
                    "      <failure type=\"{phase_attr}\"><![CDATA[{detail_cdata}]]></failure>"
                );
                println!("    </testcase>");
            }
        }
    }
    println!("  </testsuite>");
    println!("</testsuites>");
}

/// Emit the determinism-mode matrix (TASK-0033) as a JUnit XML
/// `<testsuites>` document. Mirrors [`print_summary_junit`] but reads
/// from `DetCellResult` — Failed carries a `DetMismatch` rather than a
/// `Phase`+detail, so the `<failure type=...>` is hard-coded to
/// `"determinism"` and the body is the mismatch description.
pub(crate) fn print_determinism_summary_junit(results: &[DetCellResult], wall_clock: Option<Duration>) {
    let total = results.len();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Failed(_)))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Skipped { .. }))
        .count();
    // TASK-0248: see [`print_summary_junit`] for the wall-clock-vs-sum
    // rationale. Determinism mode runs each cell twice back-to-back
    // (single-cell-twice timing inside `check_cell_determinism`), so
    // the per-cell `elapsed` field captures BOTH runs and the parallel
    // overstatement is the same shape — same fix.
    let suite_time: f64 = match wall_clock {
        Some(d) => d.as_secs_f64(),
        None => results.iter().map(|r| r.elapsed.as_secs_f64()).sum(),
    };

    println!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    println!(
        "<testsuites tests=\"{total}\" failures=\"{failed}\" errors=\"0\" \
         skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    println!(
        "  <testsuite name=\"nucleus-e2e-determinism\" tests=\"{total}\" failures=\"{failed}\" \
         errors=\"0\" skipped=\"{skipped}\" time=\"{suite_time:.3}\">"
    );
    for r in results {
        let classname = xml_escape_attr(&format!("{}.{}", r.cell.example, r.cell.schedule));
        let name = xml_escape_attr(&r.cell.backend);
        let time_s = r.elapsed.as_secs_f64();
        match &r.status {
            DetCellStatus::Pass { .. } => {
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\"/>"
                );
            }
            DetCellStatus::Skipped { reason } => {
                let msg = xml_escape_attr(reason);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                println!("      <skipped message=\"{msg}\"/>");
                println!("    </testcase>");
            }
            DetCellStatus::Failed(m) => {
                let body = format!(
                    "{} at {} (offset {}): {}",
                    m.kind,
                    m.relative_path.display(),
                    m.offset,
                    m.detail
                );
                let detail_cdata = xml_escape_cdata(&body);
                println!(
                    "    <testcase classname=\"{classname}\" name=\"{name}\" time=\"{time_s:.3}\">"
                );
                // TASK-0248: drop the redundant `message=` attribute
                // (see `print_summary_junit` for rationale). The
                // structural failure kind is exposed via `type=`.
                println!(
                    "      <failure type=\"determinism\"><![CDATA[{detail_cdata}]]></failure>"
                );
                println!("    </testcase>");
            }
        }
    }
    println!("  </testsuite>");
    println!("</testsuites>");
}

// --------------------------------------------------------------------
// Per-cell timings JSON (TASK-0023.03 Stage 1)
// --------------------------------------------------------------------

/// Escape one string for inclusion as a JSON string literal (RFC 8259
/// §7). We escape the strictly-required set — backslash, double quote,
/// and the C0 controls (`< 0x20`) — using the short forms for `\b \f \n
/// \r \t` and `\u00XX` for the rest. Non-ASCII is passed through
/// unchanged (valid UTF-8 in -> valid UTF-8 out); no need to escape
/// `/` or non-ASCII chars (the RFC permits but does not require it).
/// Defensive: cell identifiers + Status payloads come from the
/// manifest and the driver's `String` error messages, and the latter
/// can contain arbitrary bytes (compiler panics, OS errors).
pub(crate) fn json_escape_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Serialise a single `CellResult` as one JSON object. Schema:
///
/// ```text
///   { "example": "...", "schedule": "...", "backend": "...",
///     "required": bool,
///     "status": "PASS" | "FAIL" | "SKIPPED",
///     "fail_phase": "compile|build|run|diff"   (FAIL only),
///     "detail":     "..."                       (FAIL only),
///     "skip_reason":"..."                       (SKIPPED only),
///     "phase_times_ms": { "compile": N|null, "build": N|null,
///                         "run": N|null },
///     "total_ms": N,
///     "corrupted": bool }
/// ```
///
/// `phase_times_ms` mirrors the `Timings` struct (compile/build/run);
/// `null` is emitted where a phase did not execute (e.g. SKIPPED).
/// Missing-vs-zero matters: a phase that ran in 0 ms is `0`, not
/// `null`. The PRD-spec key for the JSON consumer is `phase_times_ms`,
/// keeping in line with the millisecond resolution `Duration::as_millis`
/// produces. Out-of-range values (>= 2^53) would round-trip lossily in
/// JS consumers, but per-phase millis are vastly below that.
///
/// **Spec-vs-source deviation** (architect review cycle 54): TASK-0023.03
/// AC#1 names the phases as `{build, run, diff}`. The actual `Timings`
/// struct in this crate carries `{compile, build, run}` — there is no
/// separate `diff` phase timer; the diff-check work is folded into the
/// `run` phase's wall-clock. JSON emission matches the SOURCE struct
/// rather than the spec phrasing, which is the right call (a `null`
/// for a phase that doesn't exist in source would be a comment-doc lie).
/// TASK-0023.03.01 (the Stage-2 baseline comparator follow-up) carries
/// the precise scope reference if future readers wonder about this.
pub(crate) fn cell_result_to_json(out: &mut String, r: &CellResult) {
    out.push('{');
    out.push_str("\"example\":");
    json_escape_str(out, &r.cell.example);
    out.push_str(",\"schedule\":");
    json_escape_str(out, &r.cell.schedule);
    out.push_str(",\"backend\":");
    json_escape_str(out, &r.cell.backend);
    out.push_str(",\"required\":");
    out.push_str(if r.required { "true" } else { "false" });

    out.push_str(",\"status\":");
    match &r.status {
        Status::Pass => out.push_str("\"PASS\""),
        Status::Failed { .. } => out.push_str("\"FAIL\""),
        Status::Skipped { .. } => out.push_str("\"SKIPPED\""),
    }

    // Status-specific payload, named so the consumer never needs to
    // pattern-match: presence of `fail_phase` <=> Failed; presence of
    // `skip_reason` <=> Skipped. PASS carries neither.
    match &r.status {
        Status::Pass => {}
        Status::Failed { phase, detail } => {
            out.push_str(",\"fail_phase\":");
            json_escape_str(out, &phase.to_string());
            out.push_str(",\"detail\":");
            json_escape_str(out, detail);
        }
        Status::Skipped { reason } => {
            out.push_str(",\"skip_reason\":");
            json_escape_str(out, reason);
        }
    }

    // phase_times_ms with explicit nulls — see fn-doc.
    out.push_str(",\"phase_times_ms\":{");
    out.push_str("\"compile\":");
    match r.timings.compile {
        Some(d) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{}", d.as_millis());
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"build\":");
    match r.timings.build {
        Some(d) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{}", d.as_millis());
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"run\":");
    match r.timings.run {
        Some(d) => {
            use std::fmt::Write as _;
            let _ = write!(out, "{}", d.as_millis());
        }
        None => out.push_str("null"),
    }
    out.push('}');

    {
        use std::fmt::Write as _;
        let _ = write!(out, ",\"total_ms\":{}", r.timings.total().as_millis());
    }

    out.push_str(",\"corrupted\":");
    out.push_str(if r.corrupted { "true" } else { "false" });
    out.push('}');
}

/// Render the full `Vec<CellResult>` to a JSON document with a
/// top-level `{"mode": "run", "cells": [...]}` object. Cells appear in planned
/// (deterministic) order — `execute_cells_parallel` re-sorts results
/// to planned order before returning. Newlines between objects so a
/// quick `grep` can scan one cell per line, but no trailing newline
/// inside the array (keeps the document compact).
pub(crate) fn render_timings_json(results: &[CellResult]) -> String {
    let mut out = String::with_capacity(results.len() * 256);
    // TASK-0023.03.03 cycle-57: explicit top-level `"mode": "run"` so a
    // downstream consumer can branch RUN vs DETERMINISM schema (they
    // differ on per-cell payload: phase_times_ms here vs files_compared
    // / det_mismatch / single elapsed_ms in the det emitter). Cycle-55's
    // hand-rolled `parse_baseline_json` silently skips unknown top-level
    // keys via `skip_value`, so this is backward-compatible with any
    // baseline written before this cycle.
    out.push_str("{\n  \"mode\": \"run\",\n  \"cells\": [\n");
    for (i, r) in results.iter().enumerate() {
        out.push_str("    ");
        cell_result_to_json(&mut out, r);
        if i + 1 < results.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// Write the timings JSON to `path`. Creates parent directories on
/// demand (a baseline directory under `nucleus/target/` typically
/// does not pre-exist). Returns a string error mirroring the rest of
/// the harness's error idiom so the caller can plumb it into the
/// existing `run() -> Result<i32, String>` top-level.
///
/// Failure modes (all surface as `Err`, none silent):
///   * parent dir is unwritable / not a dir;
///   * file write fails partway (we write atomically into a sibling
///     `.tmp` then rename, so a crash never leaves a truncated JSON
///     that a later `--baseline` would happily compare against).
pub(crate) fn write_timings_json(path: &std::path::Path, results: &[CellResult]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "--emit-timings: cannot create parent dir `{}`: {e}",
                    parent.display()
                )
            })?;
        }
    }
    let doc = render_timings_json(results);
    // Atomic write: tmp + rename. Same dir as `path` so rename is
    // never cross-filesystem (POSIX guarantees atomicity within a fs).
    let tmp = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => {
            return Err(format!(
                "--emit-timings: path `{}` has no file name component",
                path.display()
            ));
        }
    };
    // Architect review cycle 54: explicit fsync of the tmp file
    // before rename so a power-loss can't land the rename before
    // data hits disk (which would leave a zero-byte JSON
    // survivor — CI baseline corruption). Belt-and-braces over
    // POSIX rename atomicity.
    use std::io::Write as _;
    let mut f = fs::File::create(&tmp)
        .map_err(|e| format!("--emit-timings: create `{}`: {e}", tmp.display()))?;
    f.write_all(doc.as_bytes())
        .map_err(|e| format!("--emit-timings: write `{}`: {e}", tmp.display()))?;
    f.sync_all()
        .map_err(|e| format!("--emit-timings: fsync `{}`: {e}", tmp.display()))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| {
        format!(
            "--emit-timings: rename `{}` -> `{}`: {e}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

// --------------------------------------------------------------------
// TASK-0023.03.03 Stage 1.5 — `--emit-timings` under `--check-determinism`.
//
// Schema notes (intentionally DIFFERENT from RUN mode, branched by the
// top-level `"mode"` key):
//
//   * RUN mode (cell_result_to_json) — phase_times_ms{compile,build,run}
//     + total_ms, plus status-specific fail_phase/skip_reason.
//   * DETERMINISM mode (det_cell_result_to_json) — single elapsed_ms
//     (det has one Duration, not three phases). PASS carries
//     files_compared; FAIL carries a det_mismatch object mirroring
//     the DetMismatch Display impl; SKIPPED carries skip_reason and
//     elapsed_ms = null (manifest pre-skips short-circuit before any
//     compile, so the duration is uninformative — null is honest).
//
// Why two emitters instead of one polymorphic: the source structs
// (CellResult vs DetCellResult) are intentionally disjoint, no shared
// trait — collapsing them would force a lossy intermediate and obscure
// the per-mode contract. Cheaper to keep them parallel.

/// Serialize a single `DetCellResult` to a JSON object appended to
/// `out`. Mirrors `cell_result_to_json` shape but emits the det-mode
/// payload (see module-level note above for schema differences).
///
/// `required` is included for parity with RUN-mode (downstream
/// regression / dashboards branch on it the same way in both modes).
pub(crate) fn det_cell_result_to_json(out: &mut String, r: &DetCellResult) {
    out.push('{');
    out.push_str("\"example\":");
    json_escape_str(out, &r.cell.example);
    out.push_str(",\"schedule\":");
    json_escape_str(out, &r.cell.schedule);
    out.push_str(",\"backend\":");
    json_escape_str(out, &r.cell.backend);
    out.push_str(",\"required\":");
    out.push_str(if r.required { "true" } else { "false" });

    out.push_str(",\"status\":");
    match &r.status {
        DetCellStatus::Pass { .. } => out.push_str("\"PASS\""),
        DetCellStatus::Failed(_) => out.push_str("\"FAIL\""),
        DetCellStatus::Skipped { .. } => out.push_str("\"SKIPPED\""),
    }

    // Status-specific payload + elapsed_ms. PASS/FAIL carry a real
    // wall-clock; SKIPPED is null on purpose — see module-level note.
    match &r.status {
        DetCellStatus::Pass { files_compared } => {
            use std::fmt::Write as _;
            let _ = write!(out, ",\"files_compared\":{files_compared}");
            let _ = write!(out, ",\"elapsed_ms\":{}", r.elapsed.as_millis());
        }
        DetCellStatus::Failed(m) => {
            // det_mismatch mirrors the four DetMismatch fields. `kind`
            // is the Display impl of DetMismatchKind (stable lowercase
            // phrase, also visible in --format=junit XML — keeping
            // the two stable surfaces in lockstep).
            out.push_str(",\"det_mismatch\":{");
            out.push_str("\"relative_path\":");
            json_escape_str(out, &m.relative_path.display().to_string());
            out.push_str(",\"kind\":");
            json_escape_str(out, &m.kind.to_string());
            {
                use std::fmt::Write as _;
                let _ = write!(out, ",\"offset\":{}", m.offset);
            }
            out.push_str(",\"detail\":");
            json_escape_str(out, &m.detail);
            out.push('}');
            {
                use std::fmt::Write as _;
                let _ = write!(out, ",\"elapsed_ms\":{}", r.elapsed.as_millis());
            }
        }
        DetCellStatus::Skipped { reason } => {
            out.push_str(",\"skip_reason\":");
            json_escape_str(out, reason);
            // null (not 0) — the duration of a pre-compile manifest
            // skip carries no signal; emitting it as a number would
            // bait a downstream consumer into averaging meaningless 0s.
            out.push_str(",\"elapsed_ms\":null");
        }
    }

    // perturbed is observable in det-mode only and only ever true
    // under NUC_NONDET_TEST=1; emit it so a future regression script
    // can correlate JSON output with the NUC_NONDET_PERTURBED_CELLS
    // line on STDOUT (TASK-0188).
    out.push_str(",\"perturbed\":");
    out.push_str(if r.perturbed { "true" } else { "false" });
    out.push('}');
}

/// Render the full `Vec<DetCellResult>` to a JSON document with a
/// top-level `{"mode": "determinism", "cells": [...]}` object. Cells
/// appear in planned order (the parallel executor re-sorts results
/// before returning).
pub(crate) fn render_det_timings_json(results: &[DetCellResult]) -> String {
    let mut out = String::with_capacity(results.len() * 256);
    out.push_str("{\n  \"mode\": \"determinism\",\n  \"cells\": [\n");
    for (i, r) in results.iter().enumerate() {
        out.push_str("    ");
        det_cell_result_to_json(&mut out, r);
        if i + 1 < results.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// Write the det-mode timings JSON to `path`. Same atomic
/// tmp+fsync+rename contract as `write_timings_json` — a power-loss
/// during write must NEVER leave a partial JSON survivor that a
/// downstream consumer might mistake for a clean baseline.
pub(crate) fn write_det_timings_json(path: &std::path::Path, results: &[DetCellResult]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "--emit-timings: cannot create parent dir `{}`: {e}",
                    parent.display()
                )
            })?;
        }
    }
    let doc = render_det_timings_json(results);
    let tmp = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => {
            return Err(format!(
                "--emit-timings: path `{}` has no file name component",
                path.display()
            ));
        }
    };
    use std::io::Write as _;
    let mut f = fs::File::create(&tmp)
        .map_err(|e| format!("--emit-timings: create `{}`: {e}", tmp.display()))?;
    f.write_all(doc.as_bytes())
        .map_err(|e| format!("--emit-timings: write `{}`: {e}", tmp.display()))?;
    f.sync_all()
        .map_err(|e| format!("--emit-timings: fsync `{}`: {e}", tmp.display()))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| {
        format!(
            "--emit-timings: rename `{}` -> `{}`: {e}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

