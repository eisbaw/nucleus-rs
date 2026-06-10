//! Generative property-based cross-backend differential testing harness
//! for the Nucleus compiler (TASK-0453.01, widened by TASK-0455.05 +
//! TASK-0453.01.01).
//!
//! # Why this exists
//!
//! The standing differential rig (`nucleus-e2e`) runs a CURATED corpus of
//! fixed examples. Chapter 10 of the thesis concedes the sharpest gap in
//! the validation story: "the specification IS the corpus", so coverage
//! bounds the claim. The fix the thesis itself names is a property-based
//! harness that SYNTHESISES structured single-assignment integer programs
//! and checks cross-backend byte-identity over them. This binary is that
//! harness.
//!
//! It does NOT change any correctness guarantee — it only ADDS evidence by
//! sampling a structured, provably-compilable subclass of the program
//! space and asserting that the relevant tier-1 backends, plus an
//! in-process Rust reference, agree byte-for-byte.
//!
//! # What it does, per generated program
//!
//!   1. GENERATE a random program (one of several families — see
//!      [`program`]) into a fresh scratch dir under
//!      `nucleus/target/diff-fuzz/` (so `cargo clean` sweeps it).
//!   2. COMPILE it across the program's backend set via the `nucleus`
//!      driver, then `cargo build --release` each emitted project.
//!   3. RUN each artefact against the generated `input.bin`, every command
//!      under a PER-COMMAND WALL-CLOCK TIMEOUT (a hang -> reported FAIL,
//!      never a stall — TASK-0453.01.01 / [`exec`]).
//!   4. ASSERT mutual byte-identity across the backend outputs AND
//!      agreement with an in-process Rust reference computed directly from
//!      the generated program.
//!
//! # Scope of the in-process reference (honest, feeds the thesis threats)
//!
//! The reference guards against COMPILER common-mode failure — all
//! backends mistranslating the SAME kernel identically. It does NOT guard
//! against SPECIFICATION common-mode: each operator's `apply` (reference)
//! and `kernel_body` (emitted kernel) are two transcriptions of the SAME
//! operator definition in this crate, so a conceptual error in an op's
//! definition would appear identically in both and escape. This is the
//! same author-intent common-mode bound the thesis already states for the
//! hand-written corpus oracles (ch10 W4).
//!
//! # The generated SUBCLASS (widened; honest residual)
//!
//! Five structured families, each modelled on a proven curated example
//! (see [`program::Family`]): 1-D elementwise pipeline; 2-D vertical
//! stencil forcing halo inference; partitioned binned reduction over all
//! six combine operators (with identity-element edge cases); multi-
//! compute-worker `partition=workers` map; and a bounded `for..until`
//! single-worker convergence shape.
//!
//! Four of the five compile + run byte-identically across all SEVEN
//! tier-1 backends. The `for..until` family is pthreads-sync ONLY (the
//! curated matrix itself skips `21-jacobi-converge` on the other six —
//! the cross-backend break differential is epic S7); it is checked for
//! self-consistency + reference agreement on that single backend. The
//! per-family backend set encodes this honestly.
//!
//! It still does NOT generate: prefix scans, data-dependent gather/scatter
//! beyond the binned-reduction mask, blocking/vectorising/buffered
//! transfer directives, or floating point. Extending further is future
//! work and the honest residual the thesis update should state — and the
//! thesis paragraph is updated only when an arm actually lands, never
//! pre-claimed.
//!
//! # Determinism
//!
//! Seeded splitmix64 RNG ([`rng`]); same seed => same programs => same
//! result. No wall-clock or unseeded randomness enters program generation.
//! `--k` (program count) and `--seed` are CLI flags; `--prog-seed`
//! reproduces a single failing program.

mod backend;
mod exec;
mod family;
mod op;
mod program;
mod rng;

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use backend::{check_program, nucleus_ws};
use program::Program;
use rng::Rng;

struct Args {
    seed: u64,
    k: u64,
    keep: bool,
    /// Set the RNG stream directly to this state and generate exactly ONE
    /// program (a true per-program reproducer). Overrides `--seed`/`--k`.
    prog_seed: Option<u64>,
}

fn parse_args() -> Result<Args, String> {
    let mut seed = 1u64;
    let mut k = 8u64;
    let mut keep = false;
    let mut prog_seed = None;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--seed" => {
                i += 1;
                seed = argv
                    .get(i)
                    .ok_or("--seed requires a value")?
                    .parse()
                    .map_err(|e| format!("--seed: {e}"))?;
            }
            "--k" => {
                i += 1;
                k = argv
                    .get(i)
                    .ok_or("--k requires a value")?
                    .parse()
                    .map_err(|e| format!("--k: {e}"))?;
            }
            "--prog-seed" => {
                i += 1;
                prog_seed = Some(
                    argv.get(i)
                        .ok_or("--prog-seed requires a value")?
                        .parse()
                        .map_err(|e| format!("--prog-seed: {e}"))?,
                );
            }
            "--keep" => keep = true,
            "-h" | "--help" => {
                eprintln!(
                    "diff_fuzz — generative cross-backend differential fuzzer\n\
                     \n\
                     USAGE: diff_fuzz [--seed N] [--k N] [--prog-seed N] [--keep]\n\
                     \n\
                     --seed N        run-wide RNG seed (default 1); reproduces the\n\
                     \x20               whole K-program sequence.\n\
                     --k N           number of programs to generate (default 8).\n\
                     --prog-seed N   regenerate exactly ONE program from the\n\
                     \x20               per-program seed printed in a failure report.\n\
                     --keep          do not delete per-program scratch on success.\n\
                     \n\
                     ENV: {}=<secs>  per-command wall-clock timeout (default {}s).\n\
                     \x20               A build/run that exceeds it is treated as a\n\
                     \x20               HANG and reported as FAIL.",
                    exec::TIMEOUT_ENV,
                    exec::DEFAULT_TIMEOUT_SECS,
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag `{other}`")),
        }
        i += 1;
    }
    Ok(Args {
        seed,
        k,
        keep,
        prog_seed,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("diff_fuzz: {e}");
            return ExitCode::FAILURE;
        }
    };

    let budget = match exec::resolve_timeout() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("diff_fuzz: {e}");
            return ExitCode::FAILURE;
        }
    };

    let ws = match nucleus_ws() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("diff_fuzz: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Best-effort GC of scratch left by a KILLED earlier run.
    sweep_dead_scratch(&ws.join("target/diff-fuzz"));

    let scratch_root = ws
        .join("target/diff-fuzz")
        .join(format!("seed-{}-pid-{}", args.seed, std::process::id()));
    if let Err(e) = fs::create_dir_all(&scratch_root) {
        eprintln!("diff_fuzz: cannot create scratch root: {e}");
        return ExitCode::FAILURE;
    }

    let mut rng = Rng::new(args.seed);
    let k = if let Some(ps) = args.prog_seed {
        rng.state = ps;
        1
    } else {
        args.k
    };

    println!(
        "diff_fuzz: seed={} k={} timeout={}s scratch={}",
        args.seed,
        k,
        budget.as_secs(),
        scratch_root.display()
    );

    for idx in 0..k {
        let prog_seed = rng.state;
        let prog = Program::generate(prog_seed, &mut rng);
        let gen_dir = scratch_root.join(format!("prog-{idx:03}"));
        print!("  [{:>3}/{}] {} ... ", idx + 1, k, prog.describe());
        use std::io::Write as _;
        let _ = std::io::stdout().flush();

        match check_program(&ws, &gen_dir, &prog, budget) {
            Ok(()) => {
                let nb = prog.backends().len();
                println!("OK ({nb}/{nb} backends + reference agree)");
                if !args.keep {
                    let _ = fs::remove_dir_all(&gen_dir);
                }
            }
            Err(f) => {
                println!("FAIL");
                eprintln!("\n=========================================================");
                eprintln!(
                    "diff_fuzz FAILURE — reproduce THIS program with: --prog-seed {} (scratch retained at {})",
                    prog_seed,
                    gen_dir.display()
                );
                eprintln!("=========================================================");
                eprintln!("{}", f.msg);
                // Retain scratch on failure regardless of --keep.
                return ExitCode::FAILURE;
            }
        }
    }

    if !args.keep {
        let _ = fs::remove_dir_all(&scratch_root);
    }
    println!(
        "diff_fuzz: ALL {} programs agree byte-for-byte (seed={})",
        k, args.seed
    );
    ExitCode::SUCCESS
}

/// Best-effort sweep of scratch dirs left by a killed earlier run. A dir is
/// named `seed-<seed>-pid-<pid>`; if `<pid>` is not a live process we
/// remove it. Errors are ignored — opportunistic disk hygiene.
fn sweep_dead_scratch(root: &Path) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(pid_str) = name.rsplit("-pid-").next() else {
            continue;
        };
        if pid_str == name {
            continue;
        }
        let Ok(pid) = pid_str.parse::<i32>() else {
            continue;
        };
        if !pid_is_alive(pid) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// True if a process with `pid` currently exists (via `/proc/<pid>`). If
/// `/proc` is absent we conservatively report "alive" so we never delete a
/// live run's scratch.
fn pid_is_alive(pid: i32) -> bool {
    let proc_root = Path::new("/proc");
    if !proc_root.is_dir() {
        return true;
    }
    proc_root.join(pid.to_string()).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_is_alive_for_self() {
        assert!(pid_is_alive(std::process::id() as i32));
    }

    #[test]
    fn pid_is_alive_false_for_impossible_pid() {
        // pid space is bounded; a huge pid is virtually never live.
        if Path::new("/proc").is_dir() {
            assert!(!pid_is_alive(0x7FFF_FFFF));
        }
    }
}
