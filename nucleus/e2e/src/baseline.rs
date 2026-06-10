//! Baseline JSON parsing + per-cell timing-delta comparison.
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// Baseline comparator (TASK-0023.03 Stage 2)
// --------------------------------------------------------------------

/// One cell's wall-clock summary loaded back from a baseline JSON.
///
/// Only the fields the comparator actually consumes are kept — the
/// rich `CellResult` payload (status / detail / corrupted / etc.) is
/// not the baseline's job. The comparator joins on the identity triple
/// and reports `total_ms` deltas; anything else is Stage 3 (per-cell
/// thresholds) or downstream tooling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BaselineCell {
    pub(crate) example: String,
    pub(crate) schedule: String,
    pub(crate) backend: String,
    pub(crate) total_ms: u64,
}

/// Loud-fail JSON parse error carrying byte offset + the surrounding
/// snippet. Stage 1's emitter is deterministic, so a parse failure
/// here is almost always "wrong file fed in" — naming the offset and
/// what was expected makes that obvious without the developer having
/// to open the file in an editor.
#[derive(Debug)]
pub(crate) struct BaselineParseError {
    pub(crate) offset: usize,
    pub(crate) msg: String,
}

impl fmt::Display for BaselineParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "--baseline: JSON parse error at byte offset {}: {}",
            self.offset, self.msg
        )
    }
}

/// Minimal hand-rolled JSON reader, scoped EXACTLY to the schema that
/// `render_timings_json` emits. Deliberately NOT a general JSON parser:
///
///   * recognises only what Stage 1 emits: objects, arrays, strings
///     (with the same escape set as `json_escape_str`), integers, `null`,
///     `true`, `false`;
///   * ignores keys that aren't `example`, `schedule`, `backend`,
///     `total_ms` (so future Stage 3 fields don't break old baselines);
///   * loud-fails on structural errors with byte offset + snippet —
///     never silently treats a bad file as empty.
///
/// Stage 1 deliberately avoided serde_json; matching that constraint
/// here keeps the e2e crate's dep set minimal (one less compile-time
/// cost on every developer machine).
pub(crate) fn parse_baseline_json(src: &str) -> Result<Vec<BaselineCell>, BaselineParseError> {
    let bytes = src.as_bytes();
    let mut p = JsonCursor { bytes, pos: 0 };
    p.skip_ws();
    p.expect_byte(b'{')?;
    p.skip_ws();
    // Top-level object: we only consume the `cells` key; any other
    // future top-level field is silently skipped so old baselines stay
    // forward-compatible with new emitter additions.
    let mut cells: Option<Vec<BaselineCell>> = None;
    loop {
        p.skip_ws();
        if p.peek() == Some(b'}') {
            p.pos += 1;
            break;
        }
        let key = p.parse_string()?;
        p.skip_ws();
        p.expect_byte(b':')?;
        p.skip_ws();
        if key == "cells" {
            cells = Some(p.parse_cells_array()?);
        } else {
            p.skip_value()?;
        }
        p.skip_ws();
        match p.peek() {
            Some(b',') => p.pos += 1,
            Some(b'}') => {
                p.pos += 1;
                break;
            }
            _ => return Err(p.err("expected `,` or `}` after object member")),
        }
    }
    cells.ok_or_else(|| BaselineParseError {
        offset: 0,
        msg: "top-level object missing required `cells` array".to_string(),
    })
}

/// Byte-level cursor over the baseline JSON. Hand-rolled rather than
/// pulling in `nom` / `chumsky` — the schema is ~6 token kinds and the
/// emitter side fits in ~100 LoC, so the reader does too.
pub(crate) struct JsonCursor<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) pos: usize,
}

impl JsonCursor<'_> {
    pub(crate) fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    pub(crate) fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }
    pub(crate) fn err(&self, msg: &str) -> BaselineParseError {
        // Snippet aids the eyeball when offset alone isn't enough.
        let lo = self.pos.saturating_sub(20);
        let hi = (self.pos + 20).min(self.bytes.len());
        let snippet = String::from_utf8_lossy(&self.bytes[lo..hi]);
        BaselineParseError {
            offset: self.pos,
            msg: format!("{msg} (near `{snippet}`)"),
        }
    }
    pub(crate) fn expect_byte(&mut self, want: u8) -> Result<(), BaselineParseError> {
        match self.peek() {
            Some(b) if b == want => {
                self.pos += 1;
                Ok(())
            }
            Some(b) => Err(self.err(&format!("expected `{}`, got `{}`", want as char, b as char))),
            None => Err(self.err(&format!("expected `{}`, got EOF", want as char))),
        }
    }
    pub(crate) fn parse_string(&mut self) -> Result<String, BaselineParseError> {
        self.expect_byte(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or_else(|| self.err("trailing `\\`"))?;
                    self.pos += 1;
                    let ch = match esc {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000C}',
                        b'u' => {
                            // \uXXXX — the emitter uses this for
                            // control chars; we decode as a BMP char.
                            if self.pos + 4 > self.bytes.len() {
                                return Err(self.err("truncated \\u escape"));
                            }
                            let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
                                .map_err(|_| self.err("non-ASCII in \\u escape"))?;
                            let code = u32::from_str_radix(hex, 16)
                                .map_err(|_| self.err("\\u escape not hex"))?;
                            self.pos += 4;
                            char::from_u32(code)
                                .ok_or_else(|| self.err("invalid \\u code point"))?
                        }
                        other => {
                            return Err(self.err(&format!("unknown escape `\\{}`", other as char)))
                        }
                    };
                    out.push(ch);
                }
                Some(b) => {
                    self.pos += 1;
                    out.push(b as char);
                }
            }
        }
    }
    /// Parse a non-negative integer. The Stage-1 emitter never emits a
    /// negative `total_ms` (it's a `Duration::as_millis` cast); a `-`
    /// in the wild is a corrupt baseline and we fail LOUD.
    pub(crate) fn parse_u64(&mut self) -> Result<u64, BaselineParseError> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.err("expected unsigned integer"));
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.err("integer not ASCII"))?;
        s.parse::<u64>()
            .map_err(|e| self.err(&format!("u64 parse: {e}")))
    }
    /// Skip a JSON value the comparator doesn't care about. Recursive
    /// for nested objects/arrays so the seek stays correct.
    pub(crate) fn skip_value(&mut self) -> Result<(), BaselineParseError> {
        self.skip_ws();
        match self.peek() {
            Some(b'"') => {
                let _ = self.parse_string()?;
            }
            Some(b'{') => self.skip_object()?,
            Some(b'[') => self.skip_array()?,
            Some(b't') | Some(b'f') | Some(b'n') => {
                // true / false / null — advance past the literal.
                while let Some(b) = self.peek() {
                    if b.is_ascii_alphabetic() {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            Some(b) if b == b'-' || b.is_ascii_digit() => {
                if b == b'-' {
                    self.pos += 1;
                }
                while let Some(b) = self.peek() {
                    if b.is_ascii_digit()
                        || b == b'.'
                        || b == b'e'
                        || b == b'E'
                        || b == b'+'
                        || b == b'-'
                    {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => return Err(self.err("expected JSON value")),
        }
        Ok(())
    }
    fn skip_object(&mut self) -> Result<(), BaselineParseError> {
        self.expect_byte(b'{')?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return Ok(());
            }
            let _ = self.parse_string()?;
            self.skip_ws();
            self.expect_byte(b':')?;
            self.skip_value()?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err(self.err("expected `,` or `}` in object")),
            }
        }
    }
    fn skip_array(&mut self) -> Result<(), BaselineParseError> {
        self.expect_byte(b'[')?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(());
            }
            self.skip_value()?;
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(());
                }
                _ => return Err(self.err("expected `,` or `]` in array")),
            }
        }
    }
    /// Parse the `cells` array — the ONE shape the comparator cares
    /// about. Each element is a `{example, schedule, backend, total_ms,
    /// ...}` object. Unknown keys are silently skipped so a Stage-3
    /// emitter can extend the schema without breaking Stage-2 readers.
    fn parse_cells_array(&mut self) -> Result<Vec<BaselineCell>, BaselineParseError> {
        self.expect_byte(b'[')?;
        let mut cells: Vec<BaselineCell> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(cells);
            }
            cells.push(self.parse_cell_object()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(cells);
                }
                _ => return Err(self.err("expected `,` or `]` in cells array")),
            }
        }
    }
    fn parse_cell_object(&mut self) -> Result<BaselineCell, BaselineParseError> {
        self.expect_byte(b'{')?;
        let mut example: Option<String> = None;
        let mut schedule: Option<String> = None;
        let mut backend: Option<String> = None;
        let mut total_ms: Option<u64> = None;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect_byte(b':')?;
            self.skip_ws();
            match key.as_str() {
                "example" => example = Some(self.parse_string()?),
                "schedule" => schedule = Some(self.parse_string()?),
                "backend" => backend = Some(self.parse_string()?),
                "total_ms" => total_ms = Some(self.parse_u64()?),
                _ => self.skip_value()?,
            }
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected `,` or `}` in cell object")),
            }
        }
        Ok(BaselineCell {
            example: example.ok_or_else(|| self.err("cell missing `example`"))?,
            schedule: schedule.ok_or_else(|| self.err("cell missing `schedule`"))?,
            backend: backend.ok_or_else(|| self.err("cell missing `backend`"))?,
            total_ms: total_ms.ok_or_else(|| self.err("cell missing `total_ms`"))?,
        })
    }
}

/// One row of the delta table. `baseline_ms`/`current_ms` are `Option`
/// because a cell can be new (no baseline) or removed (no current),
/// and we still want to render the row rather than crash.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DeltaRow {
    pub(crate) example: String,
    pub(crate) schedule: String,
    pub(crate) backend: String,
    pub(crate) baseline_ms: Option<u64>,
    pub(crate) current_ms: Option<u64>,
    /// Percentage change vs baseline, ONLY when both sides exist.
    /// `current / baseline - 1`; rounded to one decimal in output.
    /// Sentinel `None` means "(new)" or "(removed)".
    pub(crate) delta_pct: Option<f64>,
    /// Per-cell perf-regression threshold, plumbed from
    /// `PlannedCell::perf_threshold_pct` (TASK-0023.03.02 Stage 3).
    /// `None` ⇒ no gate; an absent baseline match (the cell wasn't in
    /// this run's planned set, e.g. "(removed)" rows) also surfaces as
    /// `None`. Honest: rows the planner never produced cannot be gated.
    pub(crate) perf_threshold_pct: Option<f64>,
    /// `true` iff this cell is `required` in the active milestone band.
    /// Drives the exit-code wiring: a threshold breach only flips the
    /// harness exit code when `required && regression`.
    pub(crate) required: bool,
    /// Set once at row-construction time. `true` iff
    /// `(threshold, delta_pct) = (Some(t), Some(p))` AND `p > t`.
    /// Computing this here (not in the renderer) keeps the rendering
    /// path side-effect-free and the exit-code wiring a simple
    /// `rows.iter().any(...)`.
    pub(crate) regression: bool,
}

impl DeltaRow {
    /// Comparator-only sort key: largest regression (positive delta)
    /// first; new/removed sink to the bottom so the eye lands on
    /// real regressions first. Ties broken by cell identity so the
    /// output is deterministic across runs.
    pub(crate) fn sort_key(&self) -> (i32, i64, String, String, String) {
        // Tier: 0 = real delta (most informative), 1 = removed
        // (cell still in baseline but gone), 2 = new (cell only in
        // current — not a regression). Within tier 0, sort by
        // delta DESCENDING (largest regression first).
        let (tier, neg_pct_milli) = match (self.baseline_ms, self.current_ms, self.delta_pct) {
            (Some(_), Some(_), Some(p)) => (0_i32, -(p * 1000.0) as i64),
            (Some(_), None, _) => (1, 0),
            (None, Some(_), _) => (2, 0),
            _ => (3, 0),
        };
        (
            tier,
            neg_pct_milli,
            self.example.clone(),
            self.schedule.clone(),
            self.backend.clone(),
        )
    }
}

/// Build the delta table by joining current results to the baseline on
/// the identity triple. `planned` is an optional carrier of per-cell
/// metadata (perf threshold, required flag) joined on the same triple
/// (TASK-0023.03.02 Stage 3). Pass an empty slice to disable gating
/// (the cycle-55 informational-only behaviour: every row's `regression`
/// flag will be `false` and `required` defaults to `false`).
///
/// Order of returned rows is sorted by `DeltaRow::sort_key` — largest
/// regression first; new/removed cells land at the bottom.
pub(crate) fn compute_delta_rows(
    baseline: &[BaselineCell],
    current: &[CellResult],
    planned: &[PlannedCell],
) -> Vec<DeltaRow> {
    use std::collections::HashMap;
    type Key = (String, String, String);
    let key_for_baseline =
        |b: &BaselineCell| -> Key { (b.example.clone(), b.schedule.clone(), b.backend.clone()) };
    let key_for_current = |r: &CellResult| -> Key {
        (
            r.cell.example.clone(),
            r.cell.schedule.clone(),
            r.cell.backend.clone(),
        )
    };
    let key_for_planned = |p: &PlannedCell| -> Key {
        (
            p.cell.example.clone(),
            p.cell.schedule.clone(),
            p.cell.backend.clone(),
        )
    };
    let base_map: HashMap<Key, &BaselineCell> =
        baseline.iter().map(|b| (key_for_baseline(b), b)).collect();
    let cur_map: HashMap<Key, &CellResult> =
        current.iter().map(|r| (key_for_current(r), r)).collect();
    // Plan-side metadata: threshold + required flag. Cell-not-in-map
    // ⇒ no threshold AND not-required (defensive default), so a stray
    // row (e.g. a "(removed)" cell only in the baseline) cannot ever
    // gate the exit code by accident.
    let plan_map: HashMap<Key, &PlannedCell> =
        planned.iter().map(|p| (key_for_planned(p), p)).collect();

    // Build one row. Centralised so the threshold/regression rule is
    // applied identically to current-cell rows and (defensively) removed
    // rows. A removed cell has `delta_pct = None`, so `regression` is
    // unconditionally `false` there — a vanished cell is not a perf bite.
    let mk_row = |example: String,
                  schedule: String,
                  backend: String,
                  baseline_ms: Option<u64>,
                  current_ms: Option<u64>,
                  delta_pct: Option<f64>|
     -> DeltaRow {
        let k: Key = (example.clone(), schedule.clone(), backend.clone());
        let (perf_threshold_pct, required) = match plan_map.get(&k) {
            Some(p) => (p.perf_threshold_pct, p.required),
            None => (None, false),
        };
        let regression = matches!(
            (perf_threshold_pct, delta_pct),
            (Some(t), Some(p)) if p > t
        );
        DeltaRow {
            example,
            schedule,
            backend,
            baseline_ms,
            current_ms,
            delta_pct,
            perf_threshold_pct,
            required,
            regression,
        }
    };

    let mut rows: Vec<DeltaRow> = Vec::new();
    // First, every current cell — flagged as "(new)" if absent in
    // baseline, else a real delta. Drives output ordering for the
    // common case (current is what the developer just ran).
    for r in current {
        let k = key_for_current(r);
        let current_ms = r.timings.total().as_millis() as u64;
        match base_map.get(&k) {
            Some(b) => {
                let baseline_ms = b.total_ms;
                let delta_pct = if baseline_ms == 0 {
                    // Avoid div-by-zero — a 0ms baseline is rare
                    // (SKIPPED or a near-instant cell). Treat any
                    // non-zero current against 0 baseline as "(new
                    // measurable)" rather than ∞. Honest limit
                    // noted in the cycle-55 deliverable.
                    if current_ms == 0 {
                        Some(0.0)
                    } else {
                        None
                    }
                } else {
                    Some(
                        ((current_ms as f64) - (baseline_ms as f64)) / (baseline_ms as f64) * 100.0,
                    )
                };
                rows.push(mk_row(
                    r.cell.example.clone(),
                    r.cell.schedule.clone(),
                    r.cell.backend.clone(),
                    Some(baseline_ms),
                    Some(current_ms),
                    delta_pct,
                ));
            }
            None => {
                rows.push(mk_row(
                    r.cell.example.clone(),
                    r.cell.schedule.clone(),
                    r.cell.backend.clone(),
                    None,
                    Some(current_ms),
                    None,
                ));
            }
        }
    }
    // Then, every baseline cell missing from current — "(removed)".
    for b in baseline {
        let k = key_for_baseline(b);
        if !cur_map.contains_key(&k) {
            rows.push(mk_row(
                b.example.clone(),
                b.schedule.clone(),
                b.backend.clone(),
                Some(b.total_ms),
                None,
                None,
            ));
        }
    }
    rows.sort_by_key(|r| r.sort_key());
    rows
}

/// Render the delta table as a multi-line String. Colorise iff
/// `color` is true; plain otherwise. Output is human-targeted: the
/// `--emit-timings` JSON is the machine-readable counterpart.
pub(crate) fn render_delta_table(rows: &[DeltaRow], color: bool) -> String {
    use std::fmt::Write as _;
    const RED: &str = "\x1b[31m";
    const GREEN: &str = "\x1b[32m";
    const DIM: &str = "\x1b[2m";
    const RESET: &str = "\x1b[0m";

    // Bright red for required-cell regressions (exit-code-impacting,
    // demands attention); a dim red for informational threshold breaches
    // on skip-band cells (signal, not blocker).
    const BRIGHT_RED: &str = "\x1b[1;31m";

    let mut out = String::new();
    let _ = writeln!(out, "--- baseline diff ({} row(s)) ---", rows.len());
    // Header is preserved verbatim from cycle 55 so a bare `--baseline`
    // invocation (no thresholds in the matrix yet) renders byte-identical
    // to cycle 55. The optional "[threshold=…]" / "[REGRESSION …]" suffix
    // is only appended per-row when a threshold was actually plumbed
    // through, so an absent-threshold run stays a no-op on output.
    let _ = writeln!(
        out,
        "  example | schedule | backend | baseline_ms -> current_ms (Δ%)"
    );
    for r in rows {
        let cell_id = format!("  {} | {} | {} | ", r.example, r.schedule, r.backend);
        let body = match (r.baseline_ms, r.current_ms, r.delta_pct) {
            (Some(b), Some(c), Some(p)) => {
                let pct_str = format!("{:+.1}%", p);
                let painted = if !color {
                    pct_str
                } else if p > 0.0 {
                    format!("{RED}{pct_str}{RESET}")
                } else if p < 0.0 {
                    format!("{GREEN}{pct_str}{RESET}")
                } else {
                    pct_str
                };
                format!("{b} -> {c} ({painted})")
            }
            (Some(b), Some(c), None) => {
                // baseline_ms == 0 with non-zero current — sentinel.
                let tag = if color {
                    format!("{DIM}(baseline=0 ms; Δ undefined){RESET}")
                } else {
                    "(baseline=0 ms; Δ undefined)".to_string()
                };
                format!("{b} -> {c} {tag}")
            }
            (None, Some(c), _) => {
                let tag = if color {
                    format!("{DIM}(new){RESET}")
                } else {
                    "(new)".to_string()
                };
                format!("- -> {c} {tag}")
            }
            (Some(b), None, _) => {
                let tag = if color {
                    format!("{DIM}(removed){RESET}")
                } else {
                    "(removed)".to_string()
                };
                format!("{b} -> - {tag}")
            }
            (None, None, _) => "- -> -".to_string(),
        };
        // Threshold/REGRESSION suffix (TASK-0023.03.02 Stage 3). Three
        // visual tiers:
        //   * required-cell breach     -> "[REGRESSION threshold=N%]" in
        //                                 BRIGHT RED (exit-code-impacting)
        //   * skip-band-cell breach    -> "[regression threshold=N%]" in
        //                                 dim red (informational only)
        //   * threshold set, no breach -> "[threshold=N%]" dim text
        //                                 (so a reviewer can see the gate
        //                                 was active and the cell stayed
        //                                 under it)
        //   * no threshold             -> nothing appended (byte-identical
        //                                 to cycle 55 output)
        let suffix = match (r.perf_threshold_pct, r.regression, r.required) {
            (Some(t), true, true) => {
                let s = format!(" [REGRESSION threshold={:+.1}%]", t);
                if color {
                    format!("{BRIGHT_RED}{s}{RESET}")
                } else {
                    s
                }
            }
            (Some(t), true, false) => {
                let s = format!(" [regression threshold={:+.1}%]", t);
                if color {
                    format!("{RED}{DIM}{s}{RESET}")
                } else {
                    s
                }
            }
            (Some(t), false, _) => {
                let s = format!(" [threshold={:+.1}%]", t);
                if color {
                    format!("{DIM}{s}{RESET}")
                } else {
                    s
                }
            }
            (None, _, _) => String::new(),
        };
        let _ = writeln!(out, "{cell_id}{body}{suffix}");
    }
    out
}

/// Drive the baseline comparator: read `path`, parse it, compute the
/// delta table, render with ANSI iff stderr is a TTY, write to STDERR.
///
/// STDERR specifically: stdout may carry `--format=junit` XML, and
/// corrupting that XML with delta-table text would break a CI
/// consumer's parse. The Stage-1 emitter's "post-summary, pre-gate"
/// position is preserved — this call site too.
pub(crate) fn compare_against_baseline(
    path: &std::path::Path,
    current: &[CellResult],
    planned: &[PlannedCell],
) -> Result<usize, String> {
    let src = fs::read_to_string(path)
        .map_err(|e| format!("--baseline: cannot read `{}`: {e}", path.display()))?;
    let baseline = parse_baseline_json(&src).map_err(|e| {
        // Carry the parse-time offset/snippet up; the prefix already
        // names the flag so the developer knows which file to look at.
        format!("{e} in `{}`", path.display())
    })?;
    let rows = compute_delta_rows(&baseline, current, planned);
    let use_color = {
        use std::io::IsTerminal as _;
        std::io::stderr().is_terminal()
    };
    let table = render_delta_table(&rows, use_color);
    // `eprint!` not `eprintln!` — `render_delta_table` already emits
    // a trailing newline per row, so an extra newline would double-
    // space the output.
    eprint!("{table}");
    // Count required-cell threshold breaches. Returned to the caller so
    // it can flip the exit code without re-running the join (TASK-
    // 0023.03.02 AC#3 — required-cell regression = HARD FAIL). Skip-row
    // regressions are deliberately NOT counted here: they're flagged
    // visually but exit-code-neutral.
    let required_regressions = rows.iter().filter(|r| r.regression && r.required).count();
    if required_regressions > 0 {
        eprintln!(
            "nucleus-e2e: --baseline: {required_regressions} required-cell \
             perf threshold breach(es) — HARD FAIL"
        );
    }
    let _ = std::io::stderr().flush();
    Ok(required_regressions)
}

