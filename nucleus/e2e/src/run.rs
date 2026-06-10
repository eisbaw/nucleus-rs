//! Per-cell execution: invoke the driver, build the emitted project,
//! run it against `input.bin`, and byte-diff `output.bin`.
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// Cell execution
// --------------------------------------------------------------------

/// Resolve the algorithm-source path a schedule file targets via its
/// `schedule for "<path>"` directive (TASK-0049.03).
///
/// The sched AST stores this string verbatim as `SchedAst::algo_path`
/// and the *driver* ignores it (`--algo` is authoritative — see
/// `nucleus-compiler/src/sched/ast.rs:60-80`). This resolver is a
/// pure e2e-harness convenience: it lets a cell drive a non-default
/// algorithm (e.g. example 14's `prog.embedded.algo.nuc`) instead of
/// the hardcoded `prog.algo.nuc`, by honouring the schedule's own
/// declaration. It does NOT change driver/compiler behaviour.
///
/// Scanning rule (NOT a full parse — deliberately a thin line-scan):
///   - read the file; a read error propagates as `Err`;
///   - the FIRST line whose trimmed start is NOT a `//` line-comment,
///     begins with the `schedule` keyword, also contains `for`, and
///     contains a `"` is the directive.
///   - the path is the substring between the first `"` and the next
///     `"` on that directive line.
///
/// On comment handling (TASK-0049.03): a comment line is rejected by
/// TWO independent gates — the explicit `//`-skip (which runs FIRST in
/// the loop) AND the directive gate, which requires the trimmed line
/// to START with the `schedule` keyword (a `//` or `/* */` line fails
/// that too, since its trimmed start is `//` / `/*`, not `schedule`).
/// Either gate alone rejects every comment; both are kept as cheap
/// defence-in-depth, and neither is "the sole" defence. Several repo
/// scheds DO mention `schedule for` in prose comments (e.g. `// Naive
/// schedule for example 02-split-add`), so comment-rejection is
/// load-bearing, not hypothetical — it is the directive's leading
/// `schedule` keyword plus a quoted path that distinguishes the real
/// directive from any such prose. (No per-file comment census is kept
/// here: a count of "which scheds mention it" would go stale.)
///
/// Resolution base is the sched file's PARENT directory:
/// `sched_path.parent().join(extracted)`. For `"../prog.algo.nuc"`
/// with a sched at `ex_dir/schedules/x.sched.nuc` this yields
/// `ex_dir/schedules/../prog.algo.nuc` — functionally
/// `ex_dir/prog.algo.nuc`, identical to the path the harness used to
/// hardcode. The `..` is deliberately NOT canonicalised: canonicalize
/// would fail if the target did not yet exist and would diverge from
/// the `..`-bearing string the downstream existence check + `nucleus
/// build` already accept.
///
/// FAIL-LOUD: a sched with no `schedule for "..."` directive (or an
/// unterminated quote) returns `Err`; the resolver never silently
/// falls back to `prog.algo.nuc` — a malformed sched must surface.
pub(crate) fn resolve_algo_path(sched_path: &std::path::Path) -> Result<PathBuf, String> {
    let src = fs::read_to_string(sched_path)
        .map_err(|e| format!("read sched {}: {e}", sched_path.display()))?;

    for line in src.lines() {
        let trimmed = line.trim_start();
        // Skip `//` line-comments (defence-in-depth — see the fn
        // docstring; the keyword pair `schedule`/`for` is common
        // prose and a comment could carry a quoted example path).
        if trimmed.starts_with("//") {
            continue;
        }
        // The directive's trimmed line must START with the `schedule`
        // keyword and also contain `for`. This gate independently
        // rejects every comment too (a `//` / `/* */` line's trimmed
        // start is not `schedule`), so together with the `//`-skip
        // above it is two-gate defence-in-depth — neither is the sole
        // defence, and the `//`-skip is the one that fires first at
        // runtime. Requiring the keyword (plus a quote below) avoids
        // latching onto unrelated lines.
        if !(trimmed.starts_with("schedule") && trimmed.contains("for")) {
            continue;
        }
        let Some(open_rel) = line.find('"') else {
            continue;
        };
        let after_open = &line[open_rel + 1..];
        let Some(close_rel) = after_open.find('"') else {
            return Err(format!(
                "unterminated `schedule for \"...\"` quote in {}",
                sched_path.display()
            ));
        };
        let extracted = &after_open[..close_rel];
        let base = sched_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));
        return Ok(base.join(extracted));
    }

    Err(format!(
        "no `schedule for \"...\"` directive in {}",
        sched_path.display()
    ))
}

/// Derive the kernels-source FILENAME paired with a resolved algorithm
/// path (TASK-0049.08) — the silent sibling of `resolve_algo_path`,
/// which selected the algo only and left the kernels file hardcoded.
///
/// The repo follows a strict naming convention: an algorithm file named
/// `prog<variant>.algo.nuc` pairs with a kernels file named
/// `kernels<variant>.rs`, where `<variant>` is the empty string for the
/// default pair (`prog.algo.nuc` <-> `kernels.rs`) or a dotted suffix
/// for a variant (e.g. `prog.embedded.algo.nuc` <-> `kernels.embedded.rs`,
/// the no_std/stateful kernels of the embedded multi-MCU path).
///
/// Derivation rule (a pure function of the algo path's FINAL component —
/// `file_name()` is used so the `..`-bearing resolved path that
/// `resolve_algo_path` returns does not perturb the result):
///   - take `algo_path.file_name()`;
///   - if it ends with `.algo.nuc` and the remaining stem is `prog`
///     (default) or `prog` followed by a DOTTED variant suffix (e.g.
///     `prog.embedded`), the variant is whatever follows `prog`
///     (`""` for the default, `.embedded` for the embedded pair);
///   - the kernels filename is then `format!("kernels{variant}.rs")`.
///
/// The variant is required to be empty or to begin with `.` (TASK-0049.08
/// architect P3.1): `strip_prefix("prog")` alone is a loose PREFIX match,
/// so a hypothetical `program.algo.nuc` would otherwise mis-derive
/// `kernelsram.rs`. Requiring a dotted (or empty) variant makes a
/// non-conventional `prog`-prefixed name fall back cleanly to the default
/// rather than fabricating a garbled kernels filename.
///
/// FALLBACK: any filename that does NOT match the `prog[<variant>].algo.nuc`
/// shape (a missing `file_name()`, a non-`prog` stem, or no `.algo.nuc`
/// suffix) yields the historical universal default `"kernels.rs"`. This
/// preserves the harness's pre-TASK-0049.08 behaviour for any
/// unconventional algo, and is safe because the caller's
/// fixture-existence check still validates the derived path — a wrong
/// derivation surfaces as a "missing kernels at `<path>`" failure, never
/// a silent miscompile against the wrong kernels. Unlike
/// `resolve_algo_path`, this does NOT fail loud: the default is the
/// honest, behaviour-preserving choice for an off-convention name.
pub(crate) fn kernels_filename_for_algo(algo_path: &std::path::Path) -> String {
    const DEFAULT: &str = "kernels.rs";
    const ALGO_SUFFIX: &str = ".algo.nuc";
    const PROG_STEM: &str = "prog";

    let Some(name) = algo_path.file_name().and_then(|n| n.to_str()) else {
        return DEFAULT.to_string();
    };
    // Strip the `.algo.nuc` suffix; a name without it is off-convention.
    let Some(stem) = name.strip_suffix(ALGO_SUFFIX) else {
        return DEFAULT.to_string();
    };
    // The stem must begin with `prog`; the variant is the remainder
    // (`""` for the default pair, `.embedded` for the embedded pair).
    let Some(variant) = stem.strip_prefix(PROG_STEM) else {
        return DEFAULT.to_string();
    };
    // Guard the loose prefix match (TASK-0049.08 architect P3.1): the
    // variant must be empty or DOTTED, else `program.algo.nuc` would
    // mis-derive `kernelsram.rs`. A non-dotted remainder is an
    // off-convention name → fall back to the default.
    if !(variant.is_empty() || variant.starts_with('.')) {
        return DEFAULT.to_string();
    }
    format!("kernels{variant}.rs")
}

/// Return the first declared fault-report substring NOT present in
/// `stderr`, or `None` if every substring is present (TASK-0369).
/// Pure, so the substring contract is unit-testable without building an
/// artefact (`run_cell` calls it on the real run stderr after the
/// output.bin diff passes). Order of `needles` is preserved so the
/// reported missing substring is deterministic.
pub(crate) fn missing_fault_substring<'a>(stderr: &str, needles: &'a [String]) -> Option<&'a str> {
    needles
        .iter()
        .map(String::as_str)
        .find(|needle| !stderr.contains(needle))
}

pub(crate) fn run_cell(paths: &Paths, planned: &PlannedCell) -> CellResult {
    let cell = planned.cell.clone();
    // Set true only by the harness-side NUC_XBACKEND_NEGATIVE
    // post-process below (TASK-0183), and only for an mp-tcp-bufsync
    // cell whose emitted `src/wire.rs` was actually rewritten. Every
    // early-return constructor before that point carries this still-
    // `false` value, exactly as TASK-0187 threads `did_perturb`
    // through `check_cell_determinism`.
    let mut corrupted = false;

    // Manifest-declared skip wins before we touch the filesystem.
    if let Some(reason) = &planned.pre_skip {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Skipped {
                reason: reason.clone(),
            },
            timings: Timings::default(),
            corrupted,
        };
    }

    // Capabilities sniff: the harness does not duplicate
    // nucleus-compiler's capability matcher (that's
    // `check_schedule_compat`, and the driver invokes it for us).
    // We only check that the
    // backend's `capabilities.toml` exists and parses; if it doesn't
    // exist, the cell is SKIPPED (capability) — likely a missing
    // backend crate. The full schedule/backend compat check is the
    // driver's responsibility and will surface as a compile-phase
    // failure if it mismatches.
    let caps_path = paths.backend_caps(&cell.backend);
    if !caps_path.exists() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Skipped {
                reason: format!("no capabilities.toml at {}", caps_path.display()),
            },
            timings: Timings::default(),
            corrupted,
        };
    }
    // Best-effort load; an unparseable capabilities file is a hard
    // error on the *driver* side. The harness only sniffs existence.
    let caps_src = match fs::read_to_string(&caps_path) {
        Ok(s) => s,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("read {}: {e}", caps_path.display()),
                },
                timings: Timings::default(),
                corrupted,
            }
        }
    };
    let caps: CapabilitiesSniff = match toml::from_str(&caps_src) {
        Ok(c) => c,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("parse {}: {e}", caps_path.display()),
                },
                timings: Timings::default(),
                corrupted,
            }
        }
    };

    // Sanity-check the example fixtures exist before we burn time on
    // a doomed compile.
    let ex_dir = paths.example_dir(&cell.example);
    // `sched` is computed BEFORE `algo` (TASK-0049.03): the algorithm
    // source is resolved FROM the schedule's `schedule for "<path>"`
    // directive, not hardcoded to `prog.algo.nuc`. This lets a cell
    // drive a non-default algo (e.g. ex14's prog.embedded.algo.nuc).
    let sched = ex_dir
        .join("schedules")
        .join(format!("{}.sched.nuc", cell.schedule));
    let algo = match resolve_algo_path(&sched) {
        Ok(p) => p,
        Err(detail) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail,
                },
                timings: Timings::default(),
                corrupted,
            };
        }
    };
    // Kernels file is derived from the resolved algo's variant
    // (TASK-0049.08): `prog.algo.nuc` -> `kernels.rs`,
    // `prog.embedded.algo.nuc` -> `kernels.embedded.rs`. This closes the
    // silent sibling of the TASK-0049.03 algo-selection fix; the
    // fixture-existence check below validates the derived path.
    let kernels = ex_dir.join(kernels_filename_for_algo(&algo));
    let input_bin = ex_dir.join("input.bin");
    let reference_bin = ex_dir.join("reference.bin");
    for (label, p) in [
        ("algo", &algo),
        ("sched", &sched),
        ("kernels", &kernels),
        ("input.bin", &input_bin),
        ("reference.bin", &reference_bin),
    ] {
        if !p.exists() {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("missing {} at {}", label, p.display()),
                },
                timings: Timings::default(),
                corrupted,
            };
        }
    }

    let scratch = match paths.scratch_dir(&cell.example, &cell.schedule, &cell.backend) {
        Ok(p) => p,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: e,
                },
                timings: Timings::default(),
                corrupted,
            }
        }
    };

    let mut timings = Timings::default();

    // ---- Phase 1: nucleus build ----------------------------------------
    let t0 = Instant::now();
    let compile = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus")
        .arg("--")
        .arg("build")
        .arg("--algo")
        .arg(&algo)
        .arg("--sched")
        .arg(&sched)
        .arg("--kernels")
        .arg(&kernels)
        .arg("--backend")
        .arg(&cell.backend)
        .arg("--out")
        .arg(&scratch)
        .current_dir(paths.nucleus_ws())
        .output();
    timings.compile = Some(t0.elapsed());
    let compile = match compile {
        Ok(o) => o,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Compile,
                    detail: format!("spawn cargo run: {e}"),
                },
                timings,
                corrupted,
            }
        }
    };
    if !compile.status.success() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Compile,
                detail: short_tail(&compile.stderr, &compile.stdout, 4),
            },
            timings,
            corrupted,
        };
    }

    // ---- NUC_XBACKEND_NEGATIVE post-emit corruption (TASK-0183). -------
    //
    // Relocated here from mp-tcp-bufsync `lib.rs` (TASK-0178's inline
    // `maybe_corrupt_wire` on the production `wire.rs` emission path,
    // deleted in TASK-0183). Mirrors the sibling NUC_NONDET_TEST seam
    // (TASK-0157/0187): the e2e harness is the SOLE consumer of
    // NUC_XBACKEND_NEGATIVE (only `just xbackend-check-negative` sets
    // it), so production codegen needs no test hook at all — keeping
    // mp-tcp-bufsync fully branch-free is the strongest AC#1.
    //
    // It runs AFTER `nucleus build` (the emitted tree exists) and
    // BEFORE the `cargo build` below (the corruption must be compiled
    // in). `wire.rs` is mp-tcp-EXCLUSIVE: pthreads-sync emits no wire
    // at all, so `maybe_corrupt_wire_for_xbackend` is invoked ONLY for
    // mp-tcp-bufsync cells — a pthreads cell legitimately has no
    // wire.rs and must NOT Err. Under the gate, a missing wire.rs /
    // drifted enc_vec anchor is a HARD failure (Failed(Compile)), NOT
    // a silent skip, so the falsifier can never be silently neutered;
    // the matrix-wide zero-corruption guard in `run()` then forces a
    // CLEAN exit so the inverting recipe FAILs loud. Gate unset =>
    // strict no-op (function does not read or touch the tree), so bare
    // `just e2e` emits a byte-identical pristine `wire.rs`.
    if cell.backend == "mp-tcp-bufsync" {
        match maybe_corrupt_wire_for_xbackend(&scratch) {
            Ok(did) => corrupted = did,
            Err(e) => {
                return CellResult {
                    cell,
                    required: planned.required,
                    status: Status::Failed {
                        phase: Phase::Compile,
                        detail: format!("NUC_XBACKEND_NEGATIVE corruption: {e}"),
                    },
                    timings,
                    corrupted,
                };
            }
        }
    }

    // ---- Phase 2: cargo build the emitted project ----------------------
    let t1 = Instant::now();
    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&scratch)
        .output();
    timings.build = Some(t1.elapsed());
    let build = match build {
        Ok(o) => o,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Build,
                    detail: format!("spawn cargo build: {e}"),
                },
                timings,
                corrupted,
            }
        }
    };
    if !build.status.success() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Build,
                detail: short_tail(&build.stderr, &build.stdout, 4),
            },
            timings,
            corrupted,
        };
    }

    // ---- Phase 3: run the generated artefact ---------------------------
    //
    // Single-binary backend (pthreads-sync, `transport=shared-memory`):
    // exec `target/release/nuc-generated` directly — unchanged from
    // the pre-TASK-0036 path, so backend #1 cannot regress here.
    //
    // Multi-process backend (mp-tcp-bufsync, `transport=tcp`): there
    // is no single `nuc-generated`; the emitted `run.sh` is the entry
    // point — it launches one OS process per worker, wires the
    // loopback ports, waits, and exits non-zero if any worker fails
    // (TASK-0038). The harness invokes `run.sh INPUT OUTPUT` and
    // diffs `output.bin` exactly as for the single-binary case, so
    // the cross-backend differential is a real apples-to-apples
    // comparison (same reference oracle, same diff).
    let output_bin = scratch.join("output.bin");
    let t2 = Instant::now();
    let run = if caps.is_single_binary() {
        let exe = scratch.join("target/release/nuc-generated");
        if !exe.exists() {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Build,
                    detail: format!("expected `nuc-generated` at {}", exe.display()),
                },
                timings,
                corrupted,
            };
        }
        Command::new(&exe)
            .env("NUC_INPUT_PATH", &input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin)
            .output()
    } else {
        let run_sh = scratch.join("run.sh");
        if !run_sh.exists() {
            let detail = format!(
                "multi-process backend `{}` emitted no run.sh at {}",
                cell.backend,
                run_sh.display()
            );
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Build,
                    detail,
                },
                timings,
                corrupted,
            };
        }
        Command::new("bash")
            .arg(&run_sh)
            .arg(&input_bin)
            .arg(&output_bin)
            .current_dir(&scratch)
            .env("NUC_INPUT_PATH", &input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin)
            .output()
    };
    timings.run = Some(t2.elapsed());
    let run = match run {
        Ok(o) => o,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Run,
                    detail: format!("spawn run artefact: {e}"),
                },
                timings,
                corrupted,
            }
        }
    };
    if !run.status.success() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Run,
                detail: short_tail(&run.stderr, &run.stdout, 4),
            },
            timings,
            corrupted,
        };
    }

    // ---- Phase 4: diff output vs reference -----------------------------
    let expected = match fs::read(&reference_bin) {
        Ok(b) => b,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Diff,
                    detail: format!("read {}: {e}", reference_bin.display()),
                },
                timings,
                corrupted,
            }
        }
    };
    let actual = match fs::read(&output_bin) {
        Ok(b) => b,
        Err(e) => {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Diff,
                    detail: format!("read {}: {e}", output_bin.display()),
                },
                timings,
                corrupted,
            }
        }
    };
    if actual.len() != expected.len() {
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Diff,
                detail: format!("length {} != reference {}", actual.len(), expected.len()),
            },
            timings,
            corrupted,
        };
    }
    if actual != expected {
        // Find first differing byte for a useful error message.
        let mismatch_at = actual
            .iter()
            .zip(expected.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        return CellResult {
            cell,
            required: planned.required,
            status: Status::Failed {
                phase: Phase::Diff,
                detail: format!("first byte differs at offset {mismatch_at}"),
            },
            timings,
            corrupted,
        };
    }

    // ---- Phase 5: fault-report stderr assertion (TASK-0369) ------------
    //
    // Runs ONLY when this cell carries a `[[fault_assert]]` declaration
    // (an empty list — every cell without one — skips this block, so the
    // pre-feature pass/fail behaviour is byte-for-byte preserved). The
    // fault report (`check loop ... on_violation = count|log`) is written
    // to STDERR by design (PRD §6.3.5), so the output.bin diff above is
    // INDIFFERENT to it; this phase is what actually exercises the fault
    // path cross-backend. We assert SUBSTRING presence (not a full match)
    // so the timing-derived count/ns tail — and any incidental build
    // noise a multi-process `run.sh` rebuild prints — never make the
    // assertion flaky (TASK-0369 AC#3 pins only the timing-INDEPENDENT
    // shape: presence + loop-var + threshold echo).
    if !planned.fault_assert.is_empty() {
        let stderr = String::from_utf8_lossy(&run.stderr);
        if let Some(missing) = missing_fault_substring(&stderr, &planned.fault_assert) {
            return CellResult {
                cell,
                required: planned.required,
                status: Status::Failed {
                    phase: Phase::Fault,
                    detail: format!(
                        "expected fault-report substring not found in run stderr: {missing:?} \
                         (output.bin matched reference; the fault path did not surface its \
                         report). stderr tail: {}",
                        short_tail(&run.stderr, &run.stdout, 4)
                    ),
                },
                timings,
                corrupted,
            };
        }
    }

    CellResult {
        cell,
        required: planned.required,
        status: Status::Pass,
        timings,
        corrupted,
    }
}

