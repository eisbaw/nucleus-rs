//! `--check-determinism` mode + the negative-path perturbation
//! injectors (non-det, cross-backend wire corruption, required-coverage).
//!
//! Carved from `main.rs` (TASK-0460 content-preserving mega-file
//! split) along the section-banner seams. Sibling-module symbols are
//! reached through the crate root's glob re-exports via `use super::*`.

use super::*;

// --------------------------------------------------------------------
// Determinism check (TASK-0033)
// --------------------------------------------------------------------
//
// For each cell, invoke `nucleus build` twice into two distinct out
// dirs and byte-compare every generated file. PRD §1 / §10.1 promise:
//   same source + same backend = same emitted code, byte-for-byte.
//
// Pre-skip cells (manifest [[skip]]) and capability-mismatched cells
// are reported as SKIPPED — same as the run-mode contract. We do not
// run the determinism check on cells the run-mode pipeline itself
// would reject; the check is meaningful only on cells that currently
// PASS, because that's the contract the PRD makes.
//
// What is compared:
//   - every regular file under the out dir, walked recursively;
//     paths are relativised against the out dir root so the
//     containing prefix (`out_dir_a/` vs `out_dir_b/`) doesn't
//     spuriously diverge.
//
// What is intentionally NOT compared:
//   - `cargo build` artefacts. Determinism is asserted on the
//     codegen output, not on `target/`. We never invoke `cargo
//     build` in this mode — the cell only goes through phase 1
//     (`nucleus build`) twice, no phase 2/3/4.
//
// Failure mode is loud: the first differing file gets a relative
// path, an offset, and the surrounding byte context (decoded as
// UTF-8 if possible — generated files in v2 are .rs/.toml/.sh, all
// UTF-8). The "fix" for any failure is in the codegen, not the test:
// the most common culprits are HashMap iteration order leaking into
// emitted names/arms, time-of-day comments, or random temp paths.

/// One file's determinism verdict. We don't materialise the full
/// content of every emitted file — for a green run we just need to
/// know how many bytes were compared; for a red run the
/// `MismatchDetail` carries enough context to locate the offending
/// codegen site.
#[derive(Debug, Clone)]
pub(crate) enum DetCellStatus {
    Pass {
        /// Number of regular files compared. Reported so a green run
        /// still shows that *something* was actually compared (zero
        /// files would also be "no mismatch" but is uninteresting).
        files_compared: usize,
    },
    Failed(DetMismatch),
    Skipped {
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct DetMismatch {
    /// Relative path within the out dir that diverged. If the file
    /// exists in one tree but not the other, `kind` is `OnlyInA`/
    /// `OnlyInB` and `offset` is unused.
    pub(crate) relative_path: PathBuf,
    pub(crate) kind: DetMismatchKind,
    /// First differing byte offset (only meaningful for
    /// `BytesDiffer`).
    pub(crate) offset: usize,
    /// Up to ~80 bytes of context around the offset from each tree,
    /// decoded lossy. For OnlyIn* this names the side that *did* have
    /// the file.
    pub(crate) detail: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DetMismatchKind {
    BytesDiffer,
    LengthDiffers,
    OnlyInA,
    OnlyInB,
}

impl fmt::Display for DetMismatchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DetMismatchKind::BytesDiffer => "bytes differ",
            DetMismatchKind::LengthDiffers => "length differs",
            DetMismatchKind::OnlyInA => "file only in run A",
            DetMismatchKind::OnlyInB => "file only in run B",
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DetCellResult {
    pub(crate) cell: Cell,
    pub(crate) required: bool,
    pub(crate) status: DetCellStatus,
    /// Combined wall-clock of both `nucleus build` invocations.
    pub(crate) elapsed: Duration,
    /// `true` iff `maybe_perturb_for_nondet_test` actually mutated this
    /// cell's `dir_b` tree (only ever true under `NUC_NONDET_TEST=1`).
    /// Aggregated across the matrix to enforce the TASK-0187 AC#2
    /// invariant: under the negative env gate, the `--check-determinism`
    /// run must exit non-zero unless at least one tree was genuinely
    /// perturbed — a uniform `Skipped` must NOT be invertible to OK.
    pub(crate) perturbed: bool,
}

/// Drive the determinism check for one cell. Caller has already
/// filtered the manifest down to cells worth checking. Returns the
/// verdict; never panics.
pub(crate) fn check_cell_determinism(paths: &Paths, planned: &PlannedCell) -> DetCellResult {
    let cell = planned.cell.clone();
    let started = Instant::now();

    // Manifest skip wins — same contract as the run-mode harness.
    if let Some(reason) = &planned.pre_skip {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: reason.clone(),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }

    // Capabilities sniff: a missing or unparseable capabilities file
    // shows up as a SKIPPED (rather than a hard FAIL) in determinism
    // mode, because we have no way to know whether the cell would
    // PASS without it. The run-mode harness treats this as
    // Failed(Compile); for determinism, the cell is simply not in
    // scope.
    let caps_path = paths.backend_caps(&cell.backend);
    if !caps_path.exists() {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: format!("no capabilities.toml at {}", caps_path.display()),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }

    // Sanity-check fixtures (subset of run-mode's check — we don't
    // need input.bin/reference.bin since we never run the binary).
    let ex_dir = paths.example_dir(&cell.example);
    // `sched` first, then resolve `algo` from its `schedule for`
    // directive (TASK-0049.03) — same convention as run_cell. A
    // malformed sched is reported as Skipped here (mirroring this
    // site's missing-fixture early-return), since determinism mode
    // never hard-fails on fixture issues.
    let sched = ex_dir
        .join("schedules")
        .join(format!("{}.sched.nuc", cell.schedule));
    let algo = match resolve_algo_path(&sched) {
        Ok(p) => p,
        Err(reason) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped { reason },
                elapsed: started.elapsed(),
                perturbed: false,
            };
        }
    };
    // Kernels file derived from the resolved algo variant
    // (TASK-0049.08) — same convention as run_cell; the
    // fixture-existence check below validates the derived path.
    let kernels = ex_dir.join(kernels_filename_for_algo(&algo));
    for (label, p) in [("algo", &algo), ("sched", &sched), ("kernels", &kernels)] {
        if !p.exists() {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("missing {} at {}", label, p.display()),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            };
        }
    }

    // Build into two distinct out dirs.
    let dir_a = match paths.determinism_dir(&cell.example, &cell.schedule, &cell.backend, "a") {
        Ok(p) => p,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("scratch a: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            }
        }
    };
    let dir_b = match paths.determinism_dir(&cell.example, &cell.schedule, &cell.backend, "b") {
        Ok(p) => p,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("scratch b: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            }
        }
    };

    if let Err(e) = run_nucleus_build(paths, &cell, &algo, &sched, &kernels, &dir_a) {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: format!("nucleus build a failed: {e}"),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }
    if let Err(e) = run_nucleus_build(paths, &cell, &algo, &sched, &kernels, &dir_b) {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Skipped {
                reason: format!("nucleus build b failed: {e}"),
            },
            elapsed: started.elapsed(),
            perturbed: false,
        };
    }

    // ---- NUC_NONDET_TEST negative-gate perturbation (TASK-0157). ----
    //
    // Relocated here from pthreads-sync `multi_worker.rs` (TASK-0145's
    // inline per-process nonce). Rationale: the e2e determinism harness
    // is the SOLE consumer of NUC_NONDET_TEST (only the justfile
    // `determinism-check-negative` recipe sets it), so production
    // codegen needs no test hook at all — keeping it fully branch-free
    // is the strongest form of AC#1. The old branch made two `nucleus
    // build` *processes* differ via a per-process nonce; the harness
    // already builds twice (dir_a / dir_b), so the clean analogue is to
    // perturb exactly ONE tree post-emit. dir_a is left pristine, dir_b
    // gets a per-process nonce appended -> the trees diverge -> the
    // diff below reports Failed -> `--check-determinism` exits non-zero
    // -> `determinism-check-negative` correctly says "OK ... bit".
    //
    // Runtime env gate (not a cargo feature / `cfg!`): unchanged
    // reasoning from the relocated site — a nested `cargo --features`
    // inside the harness's own `cargo run` does not reliably rebuild
    // against the shared target cache; an env var is read at run time
    // and needs no rebuild. Gated on the exact string "1"; a loud
    // stderr banner so a non-reproducible run is never silent. The bare
    // `determinism-check` path (env unset) does not touch either tree.
    let did_perturb = match maybe_perturb_for_nondet_test(&dir_b) {
        Ok(p) => p,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("NUC_NONDET_TEST perturbation: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: false,
            };
        }
    };

    // Diff. We walk dir_a, look each file up in dir_b. After that we
    // sweep dir_b to catch files that exist only in b.
    let files_a = match enumerate_files(&dir_a) {
        Ok(v) => v,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("walk dir a: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            }
        }
    };
    let files_b = match enumerate_files(&dir_b) {
        Ok(v) => v,
        Err(e) => {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Skipped {
                    reason: format!("walk dir b: {e}"),
                },
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            }
        }
    };

    let set_a: BTreeSet<&PathBuf> = files_a.iter().collect();
    let set_b: BTreeSet<&PathBuf> = files_b.iter().collect();

    // OnlyInA: take the first one in BTreeSet order so the failure
    // surface is deterministic.
    if let Some(rel) = set_a.difference(&set_b).next() {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Failed(DetMismatch {
                relative_path: (*rel).clone(),
                kind: DetMismatchKind::OnlyInA,
                offset: 0,
                detail: format!(
                    "present in `{}` but not in `{}`",
                    dir_a.display(),
                    dir_b.display()
                ),
            }),
            elapsed: started.elapsed(),
            perturbed: did_perturb,
        };
    }
    if let Some(rel) = set_b.difference(&set_a).next() {
        return DetCellResult {
            cell,
            required: planned.required,
            status: DetCellStatus::Failed(DetMismatch {
                relative_path: (*rel).clone(),
                kind: DetMismatchKind::OnlyInB,
                offset: 0,
                detail: format!(
                    "present in `{}` but not in `{}`",
                    dir_b.display(),
                    dir_a.display()
                ),
            }),
            elapsed: started.elapsed(),
            perturbed: did_perturb,
        };
    }

    // Now compare bytes. Iterate set_a (== set_b at this point) in
    // BTreeSet order so any failure surface is deterministic.
    let mut files_compared = 0usize;
    for rel in &set_a {
        let path_a = dir_a.join(rel);
        let path_b = dir_b.join(rel);
        let bytes_a = match fs::read(&path_a) {
            Ok(b) => b,
            Err(e) => {
                return DetCellResult {
                    cell,
                    required: planned.required,
                    status: DetCellStatus::Skipped {
                        reason: format!("read {}: {e}", path_a.display()),
                    },
                    elapsed: started.elapsed(),
                    perturbed: did_perturb,
                }
            }
        };
        let bytes_b = match fs::read(&path_b) {
            Ok(b) => b,
            Err(e) => {
                return DetCellResult {
                    cell,
                    required: planned.required,
                    status: DetCellStatus::Skipped {
                        reason: format!("read {}: {e}", path_b.display()),
                    },
                    elapsed: started.elapsed(),
                    perturbed: did_perturb,
                }
            }
        };
        if bytes_a.len() != bytes_b.len() {
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Failed(DetMismatch {
                    relative_path: (*rel).clone(),
                    kind: DetMismatchKind::LengthDiffers,
                    offset: 0,
                    detail: format!("len a={} b={}", bytes_a.len(), bytes_b.len()),
                }),
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            };
        }
        if bytes_a != bytes_b {
            let off = bytes_a
                .iter()
                .zip(bytes_b.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            return DetCellResult {
                cell,
                required: planned.required,
                status: DetCellStatus::Failed(DetMismatch {
                    relative_path: (*rel).clone(),
                    kind: DetMismatchKind::BytesDiffer,
                    offset: off,
                    detail: byte_context(&bytes_a, &bytes_b, off),
                }),
                elapsed: started.elapsed(),
                perturbed: did_perturb,
            };
        }
        files_compared += 1;
    }

    DetCellResult {
        cell,
        required: planned.required,
        status: DetCellStatus::Pass { files_compared },
        elapsed: started.elapsed(),
        perturbed: did_perturb,
    }
}

/// Negative-gate hook for `determinism-check-negative` (TASK-0157,
/// relocated from TASK-0145's inline `multi_worker.rs` branch).
///
/// When `NUC_NONDET_TEST=1`, append a per-process nonce comment to the
/// emitted `Cargo.toml` in `tree` (the *b* build only — `a` is left
/// pristine), so the two determinism trees diverge and the diff bites.
///
/// Returns `Ok(true)` if a tree was actually perturbed, `Ok(false)` if
/// the env gate was unset/other (a strict no-op — the function does not
/// read or touch the tree), `Err` if the gate was set but the target
/// file was missing.
///
/// Why `Cargo.toml` and not `src/main.rs` (TASK-0187, fixing the
/// TASK-0157 partial-silent-neuter): EVERY backend emits `Cargo.toml`
/// at the project root (pthreads-sync lib.rs ~272, mp-tcp-bufsync
/// lib.rs ~132). `src/main.rs` is pthreads-ONLY — mp-tcp-bufsync emits
/// `src/bin/<worker>.rs` and no `main.rs`, so the old target silently
/// `Skipped` all ~13 mp-tcp cells on every run. `Cargo.toml` is
/// backend-layout-agnostic. The injected line is a `#` TOML COMMENT
/// (NOT a Rust `//` comment): `# NUC_NONDET_TEST nonce: …` is valid,
/// inert TOML, so any downstream `cargo` parse of a generated project
/// is unaffected, while `enumerate_files` (which walks the emitted
/// out-dir, Cargo.toml at its root) still diffs it and the negative
/// gate bites. Same per-process nonce (pid+nanos), same exact-`"1"`
/// gate, same loud stderr banner as the relocated TASK-0145/0157 site.
pub(crate) fn maybe_perturb_for_nondet_test(tree: &std::path::Path) -> Result<bool, String> {
    if std::env::var("NUC_NONDET_TEST").as_deref() != Ok("1") {
        return Ok(false);
    }
    eprintln!(
        "nucleus-e2e: WARNING: NUC_NONDET_TEST=1 — injecting a \
         per-process nonce into ONE emitted determinism tree ON PURPOSE \
         to test the determinism check. This run is NOT reproducible. \
         Never set this in a real build (TASK-0145 / TASK-0157 / \
         TASK-0187)."
    );
    let cargo_toml = tree.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(format!(
            "expected emitted `{}` not found — codegen layout drifted; \
             every backend must emit Cargo.toml (TASK-0187)",
            cargo_toml.display()
        ));
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut src = fs::read_to_string(&cargo_toml)
        .map_err(|e| format!("read {}: {e}", cargo_toml.display()))?;
    // `#` comment: valid, inert TOML. A Rust `//` line here would make
    // the generated Cargo.toml unparseable — do not change this.
    src.push_str(&format!(
        "\n# NUC_NONDET_TEST nonce: pid={} nanos={nanos}\n",
        std::process::id()
    ));
    fs::write(&cargo_toml, src).map_err(|e| format!("write {}: {e}", cargo_toml.display()))?;
    Ok(true)
}

/// Negative-gate hook for `xbackend-check-negative` (TASK-0178,
/// relocated harness-side in TASK-0183 from mp-tcp-bufsync `lib.rs`'s
/// inline `maybe_corrupt_wire` — deleted there so production codegen
/// carries no test-only branch). Sibling of
/// `maybe_perturb_for_nondet_test`; identical discipline.
///
/// When `NUC_XBACKEND_NEGATIVE=1`, deterministically corrupt the
/// emitted `src/wire.rs` in `tree` (an mp-tcp-bufsync project dir):
/// rewrite `enc_vec` so the single trailing byte of each encoded vec
/// payload is incremented (wrapping). `wire.rs` is the TCP wire
/// protocol, **mp-tcp-EXCLUSIVE** — pthreads-sync emits no wire at
/// all. A multi-process mp-tcp cell that ships array data
/// worker→worker then decodes wrong values and its `output.bin`
/// diverges from the committed hand-written `reference.bin` oracle,
/// while pthreads-sync (same oracle) stays byte-identical. That
/// asymmetry — mp-tcp ≠ reference, pthreads-sync == reference — is
/// precisely the *cross-backend* differential biting, not a global
/// break. Caller invokes this ONLY for `cell.backend ==
/// "mp-tcp-bufsync"`, so a pthreads tree (no wire.rs) is never
/// touched and never Errs.
///
/// Returns `Ok(true)` if the tree was actually corrupted, `Ok(false)`
/// if the env gate was unset/other (a strict no-op — the function
/// does not read or touch the tree, keeping bare `just e2e`'s emitted
/// `wire.rs` byte-identical to `mp_tcp_common::WIRE_RUNTIME_SRC`),
/// `Err` if the gate was set but `wire.rs` was missing OR the
/// `enc_vec` anchor drifted. The caller maps `Err` to
/// `Failed(Compile)` (a HARD, gate-visible failure — never a silent
/// skip that neuters the falsifier); the matrix-wide zero-corruption
/// guard in `run()` then forces a CLEAN exit so the inverting recipe
/// FAILs loud (the TASK-0187 recipe-inversion lesson).
///
/// Deterministic by construction: a fixed source rewrite (string
/// replace), no PID/clock/RNG/hash-order entropy. Env gate (not a
/// cargo feature / `cfg!`): a nested `cargo --features` inside the
/// harness's own `cargo run` does not reliably rebuild against the
/// shared target cache; an env var is read at run time, no rebuild.
/// Exact-`"1"` gate, loud stderr banner, anchor-drift hard-failure —
/// all preserved verbatim from the deleted mp-tcp-bufsync site.
pub(crate) fn maybe_corrupt_wire_for_xbackend(tree: &std::path::Path) -> Result<bool, String> {
    if std::env::var("NUC_XBACKEND_NEGATIVE").as_deref() != Ok("1") {
        return Ok(false);
    }
    eprintln!(
        "nucleus-e2e: WARNING: NUC_XBACKEND_NEGATIVE=1 — deliberately \
         corrupting mp-tcp wire encode (enc_vec) in an emitted tree ON \
         PURPOSE to test the cross-backend differential. This run is \
         NOT a real build and its mp-tcp output is intentionally wrong. \
         Never set this in a real build (TASK-0178 / TASK-0183)."
    );
    let wire_rs = tree.join("src").join("wire.rs");
    if !wire_rs.exists() {
        return Err(format!(
            "expected emitted `{}` not found — codegen layout drifted; \
             mp-tcp-bufsync must emit src/wire.rs (TASK-0183); refusing \
             to let a missing-file become a silently-uncorrupted build",
            wire_rs.display()
        ));
    }
    let src =
        fs::read_to_string(&wire_rs).map_err(|e| format!("read {}: {e}", wire_rs.display()))?;
    // The single body line of `enc_vec` in wire_runtime.rs. We append
    // a deterministic last-byte tweak after the buffer is filled.
    // Anchored on the exact source text so a refactor of wire_runtime
    // fails this replace LOUD (Err below -> Failed(Compile)) rather
    // than silently emitting a non-corrupted build that would make the
    // negative recipe a false PASS. (Was a `panic!` at the deleted
    // mp-tcp-bufsync site; harness-side a typed Err is the gate-visible
    // analogue — the caller turns it into Failed(Compile) and the
    // zero-corruption guard makes the recipe FAIL loud.)
    const ANCHOR: &str =
        "    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    out\n}";
    const CORRUPT: &str = "    for &e in v {\n        out.extend_from_slice(&to_le(e));\n    }\n    if let Some(last) = out.last_mut() {\n        *last = last.wrapping_add(1); // NUC_XBACKEND_NEGATIVE deliberate corruption (TASK-0178 / TASK-0183)\n    }\n    out\n}";
    if !src.contains(ANCHOR) {
        return Err(format!(
            "enc_vec anchor not found in `{}` — the negative-arm \
             injection point has drifted. Update ANCHOR in \
             maybe_corrupt_wire_for_xbackend (TASK-0183) so the \
             cross-backend negative test keeps biting; refusing to \
             emit a silently-uncorrupted build",
            wire_rs.display()
        ));
    }
    let corrupted = src.replacen(ANCHOR, CORRUPT, 1);
    fs::write(&wire_rs, corrupted).map_err(|e| format!("write {}: {e}", wire_rs.display()))?;
    Ok(true)
}

/// Sentinel schedule name used by `maybe_inject_required_coverage_negative`.
/// MUST NOT match any real `examples/<example>/schedules/<schedule>.sched.nuc`
/// file — its purpose is to be unfindable by `plan_cells`, so the synthetic
/// `[[required]]` entry below cannot be planned and therefore appears as a
/// gap from `required_coverage_gaps`. Used to attribute gaps to THIS injection
/// (and not to any unrelated gap that might appear for other reasons) when
/// computing `NUC_REQUIRED_COVERAGE_GAP_DETECTED` in `run_inner`.
///
/// Renaming this constant is a contract change: justfile's
/// `required-coverage-check-negative` recipe parses the harness output by the
/// stdout key, not by this string, but `run_inner`'s attribution filter does
/// compare against this exact value — keep the two in lockstep.
pub(crate) const REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE: &str = "__nuc_typo_negative_schedule__";

/// Negative-gate hook for `required-coverage-check-negative` (TASK-0168).
/// Sibling of `maybe_perturb_for_nondet_test` (NUC_NONDET_TEST) and
/// `maybe_corrupt_wire_for_xbackend` (NUC_XBACKEND_NEGATIVE); identical
/// discipline.
///
/// When `NUC_REQUIRED_COVERAGE_NEGATIVE=1`, append a single synthetic
/// `[[required]]` entry to the in-memory `Manifest` whose `schedule` is the
/// `REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE` sentinel — a name that
/// cannot match any discovered `*.sched.nuc` file. The synthetic entry's
/// `example`, `backend`, and `milestone` are taken from the first real
/// required entry IN THE ACTIVE TIER (TASK-0446: the first entry whose
/// `is_mpi_backend` matches `with_mpi` — tier-1 by default, an mpi backend
/// under `--with-mpi`) so:
///   * `example` is in `runnable_examples` (otherwise `plan_cells` would not
///     iterate over it and the cell would silently leave the coverage scope),
///   * `backend` is in the active tier's backend list, so the
///     `required_coverage_gaps` tier filter (`active_backends(with_mpi)`)
///     keeps the synthetic cell in scope (otherwise it is filtered out and
///     the negative arm tests nothing),
///   * `milestone` lies in any `--milestone` band that includes at least one
///     real required cell (bare `just e2e` has no milestone filter, so any
///     value is fine; the recipe runs without `--milestone`, but mirroring an
///     existing entry keeps the injection robust against future CI matrices).
///
/// Fallback (no real `[[required]]` entries in the manifest): pick
/// `runnable_examples[0]`, `backends[0]`, milestone `"M1"` (hard-coded —
/// best-effort, NOT robust to a future scheme that retires `M1` from
/// `Milestone::parse`'s accepted set; if that ever happens, this branch
/// returns a milestone string that `Milestone::parse` would reject
/// downstream, making the recipe FAIL loud rather than silently
/// no-op). This branch is purely defensive — today's `e2e-matrix.toml`
/// has 18+ required entries so the anchored path always wins — but it
/// avoids a NEW failure mode (panic / Err) on an exotic manifest. If
/// the manifest is truly degenerate (no examples OR no backends), Err
/// loud rather than silently no-op.
///
/// CRITICAL — does NOT mutate existing entries. AC#2 of TASK-0168 demands
/// "no committed broken manifest"; appending one synthetic entry at
/// runtime, after the on-disk file has been parsed, satisfies that contract
/// exactly the way `maybe_perturb_for_nondet_test` and
/// `maybe_corrupt_wire_for_xbackend` do for their respective gates (the env
/// flag is the perturbation seam; the on-disk artefact stays clean).
///
/// Returns `Ok(true)` if the manifest was actually mutated, `Ok(false)` if
/// the env gate was unset/other (a strict no-op — the function does not
/// read or touch the manifest, keeping bare `just e2e` byte-for-byte
/// unaffected), `Err` if the gate was set but the manifest is too degenerate
/// to inject into. Deterministic by construction: same input manifest plus
/// same env state = identical injection (no clock/PID/RNG).
pub(crate) fn maybe_inject_required_coverage_negative(
    manifest: &mut Manifest,
    with_mpi: bool,
) -> Result<bool, String> {
    if std::env::var("NUC_REQUIRED_COVERAGE_NEGATIVE").as_deref() != Ok("1") {
        return Ok(false);
    }
    eprintln!(
        "nucleus-e2e: WARNING: NUC_REQUIRED_COVERAGE_NEGATIVE=1 — injecting a \
         synthetic [[required]] entry with a non-existent schedule \
         (`{REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE}`) into the in-memory \
         manifest ON PURPOSE to test the TASK-0163 required-coverage guard. \
         This run is NOT a real build and is expected to exit non-zero. Never \
         set this in a real build (TASK-0168)."
    );

    // Pick (example, backend, milestone) from the first real required entry
    // IN THE ACTIVE TIER so the synthetic cell survives `cell_matches_filters`,
    // the active milestone gate, AND the `required_coverage_gaps` tier filter.
    // Fallback path is only relevant on a degenerate manifest — see docstring.
    //
    // TASK-0446 tier-awareness: the synthetic cell's backend MUST be in the
    // tier this run actually scopes to (`active_backends(with_mpi)`), or
    // `required_coverage_gaps` filters it out and the `attributed == 0`
    // backstop forces a clean exit (recipe FAILs loud, the safe direction,
    // but the arm tests nothing). The first overall `[[required]]` entry is a
    // TIER-1 (`backends`) cell, so `.first()` alone is tier-1-only by
    // construction — which is correct for the default-shell
    // `required-coverage-check-negative` recipe (no `--with-mpi`) but wrong
    // for the mpi arm. Select the first required entry whose
    // `is_mpi_backend` matches `with_mpi`, so:
    //   * `--with-mpi` unset  → first tier-1 required cell (unchanged behaviour);
    //   * `--with-mpi` set    → first mpi-tier required cell, so the synthetic
    //     gap is attributable under the mpi-tier coverage gate
    //     (`required-coverage-check-negative-mpi`, run under `.#mpi`).
    let anchor = manifest
        .required
        .iter()
        .find(|e| manifest.is_mpi_backend(&e.backend) == with_mpi)
        .map(|e| (e.example.clone(), e.backend.clone(), e.milestone.clone()));
    let (example, backend, milestone) = if let Some(anchored) = anchor {
        anchored
    } else {
        // Degenerate / no in-tier required entry: anchor example on
        // `runnable_examples` and backend on the active tier's backend list.
        // Milestone "M1" is a hard-coded best-effort default (see docstring's
        // fallback caveat) — never reached on today's manifest (both tiers
        // carry real `[[required]]` entries).
        let example = manifest.runnable_examples.first().cloned().ok_or_else(|| {
            "NUC_REQUIRED_COVERAGE_NEGATIVE=1 but manifest has no \
             runnable_examples to anchor a synthetic required entry against \
             (degenerate manifest)"
                .to_string()
        })?;
        let active_tier = if with_mpi {
            &manifest.mpi_backends
        } else {
            &manifest.backends
        };
        let backend = active_tier.first().cloned().ok_or_else(|| {
            format!(
                "NUC_REQUIRED_COVERAGE_NEGATIVE=1 but manifest has no {} to \
                 anchor a synthetic required entry against (degenerate \
                 manifest for the {} tier)",
                if with_mpi { "mpi_backends" } else { "backends" },
                if with_mpi { "mpi" } else { "default" },
            )
        })?;
        (example, backend, "M1".to_string())
    };

    manifest.required.push(RequiredEntry {
        example,
        schedule: REQUIRED_COVERAGE_NEGATIVE_SENTINEL_SCHEDULE.to_string(),
        backend,
        milestone,
        perf_threshold_pct: None,
    });
    Ok(true)
}

/// Invoke `nucleus build` for one cell into `out_dir`. Returns Err
/// with a short tail of the compiler's stderr on failure.
pub(crate) fn run_nucleus_build(
    paths: &Paths,
    cell: &Cell,
    algo: &std::path::Path,
    sched: &std::path::Path,
    kernels: &std::path::Path,
    out_dir: &std::path::Path,
) -> Result<(), String> {
    use test_common::proc_timeout::{run_timed, Timed};
    // Same per-phase wall-clock budget as the curated cell-run path
    // (`run::e2e_phase_budget`), driven through the SHARED group-kill
    // machinery so a wedged determinism re-build fails loud instead of
    // stalling `just e2e` (TASK-0466). A malformed env value is rejected
    // by the shared resolver before any spawn.
    let budget = crate::run::e2e_phase_budget()?;
    let mut cmd = Command::new("cargo");
    cmd.arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus")
        .arg("--")
        .arg("build")
        .arg("--algo")
        .arg(algo)
        .arg("--sched")
        .arg(sched)
        .arg("--kernels")
        .arg(kernels)
        .arg("--backend")
        .arg(&cell.backend)
        .arg("--out")
        .arg(out_dir)
        .current_dir(paths.nucleus_ws());
    let out = match run_timed(cmd, budget).map_err(|e| format!("spawn cargo: {e}"))? {
        Timed::Completed(out) => out,
        Timed::Timeout {
            elapsed,
            budget,
            partial_stdout,
            partial_stderr,
        } => {
            return Err(format!(
                "nucleus build (determinism re-build) TIMED OUT after {:.1}s (budget {:.0}s, \
                 set {} to adjust) — treated as a HANG/FAIL. Last output: {}",
                elapsed.as_secs_f64(),
                budget.as_secs_f64(),
                crate::run::E2E_TIMEOUT_ENV,
                short_tail(&partial_stderr, &partial_stdout, 4),
            ));
        }
    };
    if !out.status.success() {
        return Err(short_tail(&out.stderr, &out.stdout, 4));
    }
    Ok(())
}

/// Walk `root` recursively, return every regular file's path *relative
/// to root*. Deterministic order is left to the caller (we hand back
/// a `Vec` so a `BTreeSet` can be built outside; this keeps the walk
/// allocation light). Skips no entries — generated trees in v2 are
/// small and we want false positives to be loud.
pub(crate) fn enumerate_files(root: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("read_dir `{}`: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("readdir error: {e}"))?;
            let path = entry.path();
            let ty = entry
                .file_type()
                .map_err(|e| format!("file_type `{}`: {e}", path.display()))?;
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| format!("strip_prefix on `{}`: {e}", path.display()))?
                    .to_path_buf();
                out.push(rel);
            }
            // symlinks etc. are intentionally not followed — codegen
            // doesn't emit them in v2.
        }
    }
    Ok(out)
}

/// Render up to 24 bytes of UTF-8-ish context around byte `off` in
/// both trees. Generated files are .rs/.toml/.sh, so a lossy-decode
/// gives the developer something readable to grep on.
pub(crate) fn byte_context(a: &[u8], b: &[u8], off: usize) -> String {
    let span = 24usize;
    let start = off.saturating_sub(span / 2);
    let end_a = (off + span / 2).min(a.len());
    let end_b = (off + span / 2).min(b.len());
    let snip_a = String::from_utf8_lossy(&a[start..end_a]).replace('\n', "\\n");
    let snip_b = String::from_utf8_lossy(&b[start..end_b]).replace('\n', "\\n");
    format!("a={snip_a:?} b={snip_b:?}")
}

/// Print a determinism-summary table. Same general shape as
/// `print_summary` for the run-mode harness so the two outputs are
/// visually consistent; we don't share code because the columns
/// differ.
pub(crate) fn print_determinism_summary(results: &[DetCellResult]) {
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
    println!("e2e determinism matrix (TASK-0033):");
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
            DetCellStatus::Pass { files_compared } => (
                pass("PASS"),
                dim(&format!("{files_compared} file(s) byte-identical")),
            ),
            DetCellStatus::Failed(m) => (
                fail("FAIL"),
                format!(
                    "{}: {} (offset {}, {})",
                    m.relative_path.display(),
                    m.kind,
                    m.offset,
                    m.detail
                ),
            ),
            DetCellStatus::Skipped { reason } => (skip("SKIPPED"), dim(reason)),
        };
        let mark = if r.required { "*" } else { " " };
        println!(
            "{mark} {:<ex_w$}  {:<sc_w$}  {:<be_w$}  {:<10}  {:>8}   {}",
            r.cell.example,
            r.cell.schedule,
            r.cell.backend,
            status_str,
            format_duration(r.elapsed),
            detail,
            ex_w = ex_w,
            sc_w = sc_w,
            be_w = be_w
        );
    }
    println!();
    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Pass { .. }))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Failed(_)))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, DetCellStatus::Skipped { .. }))
        .count();
    println!("  total: {total}   pass: {passed}   fail: {failed}   skipped: {skipped}");
    println!("  (* = required cell; any FAIL is a hard failure regardless of required-bit)");
    println!();
}

/// Return the last `n` non-empty lines of stderr (preferred) or
/// stdout, joined by `; `. Keeps the table compact while still
/// surfacing actionable error context.
pub(crate) fn short_tail(stderr: &[u8], stdout: &[u8], n: usize) -> String {
    let s = if !stderr.is_empty() { stderr } else { stdout };
    let text = String::from_utf8_lossy(s);
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let tail = if lines.len() > n {
        &lines[lines.len() - n..]
    } else {
        &lines[..]
    };
    tail.join("; ")
}

