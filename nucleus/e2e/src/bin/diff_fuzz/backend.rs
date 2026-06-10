//! Per-program build/run orchestration + the cross-backend + reference
//! agreement check.
//!
//! This re-derives the e2e harness's build/run flow (Phase 1 nucleus
//! build, Phase 2 cargo build --release, Phase 3 run) rather than calling
//! into it. The duplication is DELIBERATE: a differential oracle that
//! shared its execution harness with the system under comparison would
//! couple the evidence to the thing being validated. A future "DRY this
//! up" must not collapse the oracle into the SUT.
//!
//! Every spawned command goes through [`crate::exec::run_timed`] so a
//! HANG (a deadlocked backend) becomes a reported FAILURE rather than a
//! stall (TASK-0453.01.01 AC#1).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::exec::{run_timed, Timed};
use crate::program::{Program, SourceBundle};

/// A failure carrying a fully-reproducing report.
pub(crate) struct Failure {
    pub(crate) msg: String,
}

/// Write the three generated source files + input.bin into `gen_dir`.
pub(crate) fn write_program(gen_dir: &Path, bundle: &SourceBundle) -> Result<(), String> {
    fs::create_dir_all(gen_dir).map_err(|e| format!("mkdir {}: {e}", gen_dir.display()))?;
    let w = |name: &str, content: &str| -> Result<(), String> {
        let p = gen_dir.join(name);
        fs::write(&p, content).map_err(|e| format!("write {}: {e}", p.display()))
    };
    w("prog.algo.nuc", &bundle.algo)?;
    w("prog.sched.nuc", &bundle.sched)?;
    w("kernels.rs", &bundle.kernels)?;
    fs::write(gen_dir.join("input.bin"), &bundle.input)
        .map_err(|e| format!("write input.bin: {e}"))?;
    Ok(())
}

/// Tail of combined stderr+stdout for error messages.
fn tail(stderr: &[u8], stdout: &[u8], lines: usize) -> String {
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(stderr));
    if !stdout.is_empty() {
        combined.push('\n');
        combined.push_str(&String::from_utf8_lossy(stdout));
    }
    let v: Vec<&str> = combined.lines().collect();
    let start = v.len().saturating_sub(lines);
    v[start..].join("\n")
}

/// Build a `Command`, run it under the wall-clock `budget`, and map a
/// timeout to a typed error string naming the phase. On a non-zero exit it
/// returns the phase-tagged tail; on success it returns the captured
/// stdout/stderr for the caller (most callers ignore it).
fn timed_phase(
    cmd: Command,
    budget: Duration,
    phase: &str,
    backend: &str,
) -> Result<std::process::Output, String> {
    match run_timed(cmd, budget).map_err(|e| format!("[{backend}] {phase} spawn: {e}"))? {
        Timed::Completed(out) => {
            if out.status.success() {
                Ok(out)
            } else {
                Err(format!(
                    "[{backend}] {phase} FAILED:\n{}",
                    tail(&out.stderr, &out.stdout, 8)
                ))
            }
        }
        Timed::Timeout {
            elapsed,
            budget,
            partial_stdout,
            partial_stderr,
        } => Err(format!(
            "[{backend}] {phase} TIMED OUT after {:.1}s (budget {:.0}s) — treated as a HANG/FAIL. \
             Set {} to adjust. Last output:\n{}",
            elapsed.as_secs_f64(),
            budget.as_secs_f64(),
            crate::exec::TIMEOUT_ENV,
            tail(&partial_stderr, &partial_stdout, 8)
        )),
    }
}

/// Compile + build + run one backend with a per-command timeout; return
/// the produced output.bin bytes.
pub(crate) fn run_backend(
    ws: &Path,
    gen_dir: &Path,
    backend: &str,
    single_binary: bool,
    input_bin: &Path,
    budget: Duration,
) -> Result<Vec<u8>, String> {
    let out_dir = gen_dir.join(format!("out-{backend}"));
    let _ = fs::remove_dir_all(&out_dir);

    // Phase 1: nucleus build.
    let mut compile = Command::new("cargo");
    compile
        .args(["run", "--quiet", "--bin", "nucleus", "--", "build"])
        .arg("--algo")
        .arg(gen_dir.join("prog.algo.nuc"))
        .arg("--sched")
        .arg(gen_dir.join("prog.sched.nuc"))
        .arg("--kernels")
        .arg(gen_dir.join("kernels.rs"))
        .arg("--backend")
        .arg(backend)
        .arg("--out")
        .arg(&out_dir)
        .current_dir(ws);
    timed_phase(compile, budget, "nucleus build", backend)?;

    // Phase 2: cargo build --release the emitted project.
    let mut build = Command::new("cargo");
    build
        .args(["build", "--release", "--quiet"])
        .current_dir(&out_dir);
    timed_phase(build, budget, "cargo build", backend)?;

    // Phase 3: run.
    let output_bin = out_dir.join("output.bin");
    let _ = fs::remove_file(&output_bin);
    if single_binary {
        let exe = out_dir.join("target/release/nuc-generated");
        if !exe.exists() {
            return Err(format!(
                "[{backend}] expected nuc-generated at {}",
                exe.display()
            ));
        }
        let mut run = Command::new(&exe);
        run.env("NUC_INPUT_PATH", input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin);
        timed_phase(run, budget, "run", backend)?;
    } else {
        let run_sh = out_dir.join("run.sh");
        if !run_sh.exists() {
            return Err(format!("[{backend}] expected run.sh at {}", run_sh.display()));
        }
        let mut run = Command::new("bash");
        run.arg(&run_sh)
            .arg(input_bin)
            .arg(&output_bin)
            .current_dir(&out_dir)
            .env("NUC_INPUT_PATH", input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin);
        timed_phase(run, budget, "run", backend)?;
    }

    fs::read(&output_bin).map_err(|e| format!("[{backend}] read output.bin: {e}"))
}

/// First differing byte offset between two slices.
fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

/// Run one generated program through all of ITS backends + the in-process
/// reference; `Ok(())` on full agreement, `Err(Failure)` with a fully-
/// reproducing report otherwise. The backend SET is per-program (the
/// for..until family is single-backend — see `Program::backends`).
pub(crate) fn check_program(
    ws: &Path,
    gen_dir: &Path,
    prog: &Program,
    budget: Duration,
) -> Result<(), Failure> {
    let bundle = prog.bundle();
    let report = |msg: String| Failure {
        msg: format!(
            "DIVERGENCE / FAILURE\n  {}\n\n--- generated program ---\n{}\n--- prog.algo.nuc ---\n{}\n--- prog.sched.nuc ---\n{}\n--- kernels.rs ---\n{}",
            msg,
            prog.describe(),
            bundle.algo,
            bundle.sched,
            bundle.kernels,
        ),
    };

    if let Err(e) = write_program(gen_dir, &bundle) {
        return Err(report(format!("could not write program: {e}")));
    }
    let input_bin = gen_dir.join("input.bin");
    let reference = &bundle.reference;

    let mut first_output: Option<(String, Vec<u8>)> = None;
    for (backend, single_binary) in prog.backends().iter() {
        let out = match run_backend(ws, gen_dir, backend, *single_binary, &input_bin, budget) {
            Ok(o) => o,
            Err(e) => return Err(report(e)),
        };

        // Agreement with the in-process reference (common-mode guard).
        if &out != reference {
            let off = first_diff(&out, reference).unwrap_or(0);
            return Err(report(format!(
                "backend `{backend}` DISAGREES WITH REFERENCE: lengths backend={} ref={}, first differing byte at offset {off}",
                out.len(),
                reference.len()
            )));
        }

        // Mutual byte-identity against the first backend's output.
        match &first_output {
            None => first_output = Some((backend.to_string(), out)),
            Some((first_name, first_bytes)) => {
                if &out != first_bytes {
                    let off = first_diff(&out, first_bytes).unwrap_or(0);
                    return Err(report(format!(
                        "backend `{backend}` DISAGREES WITH `{first_name}`: lengths {}={} {}={}, first differing byte at offset {off}",
                        backend,
                        out.len(),
                        first_name,
                        first_bytes.len()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Locate the repo's `nucleus/` workspace dir by walking up from cwd
/// looking for the `nucleus/` + `nuc-nucleus/` sibling pair.
pub(crate) fn nucleus_ws() -> Result<PathBuf, String> {
    let mut dir = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    loop {
        if dir.join("nucleus").is_dir() && dir.join("nuc-nucleus").is_dir() {
            return Ok(dir.join("nucleus"));
        }
        if dir.file_name().map(|n| n == "nucleus").unwrap_or(false)
            && dir.join("Cargo.toml").is_file()
            && dir
                .parent()
                .map(|p| p.join("nuc-nucleus").is_dir())
                .unwrap_or(false)
        {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err("could not locate repo root (need `nucleus/` + `nuc-nucleus/`)".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_diff_finds_length_then_byte() {
        assert_eq!(first_diff(b"abc", b"abc"), None);
        assert_eq!(first_diff(b"abc", b"abd"), Some(2));
        assert_eq!(first_diff(b"abc", b"ab"), Some(2)); // length mismatch
    }

    #[test]
    fn tail_returns_last_lines() {
        let t = tail(b"l1\nl2\nl3\nl4", b"", 2);
        assert_eq!(t, "l3\nl4");
    }
}
