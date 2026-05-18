//! `nucleus` pre-compiler binary.
//!
//! Drives the M1 pipeline end-to-end:
//!
//!   parse algo + sched -> lower -> link -> build ACFG ->
//!   inject syncs -> inject transfers -> load backend capabilities ->
//!   check schedule/backend compat -> backend `emit(...)` ->
//!   emit a `run.sh` to the output directory.
//!
//! Usage:
//!
//!   nucleus build \
//!     --algo PATH/prog.algo.nuc \
//!     --sched PATH/some.sched.nuc \
//!     --backend pthreads-sync \
//!     --out OUT_DIR \
//!     [--kernels PATH/kernels.rs] \
//!     [--capabilities PATH/capabilities.toml]
//!
//! When `--kernels` is omitted, the driver looks for `kernels.rs`
//! next to the algorithm file. When `--capabilities` is omitted, it
//! looks for `nucleus/backends/<backend>/capabilities.toml` walking
//! up from the current working directory. Both defaults match how
//! the e2e tests invoke the binary.
//!
//! The driver only knows about the pthreads-sync backend at M1. New
//! backends added via TASK-0036+ will register here.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compiler::{
    apply_block_transforms, build_acfg, check_kernels_contract, check_schedule_compat,
    inject_syncs, inject_transfers, link, load_capabilities,
};

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().collect();
    if argv.len() < 2 {
        return die("missing subcommand; try `nucleus build --help`");
    }

    match argv[1].as_str() {
        "build" => match cmd_build(&argv[2..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(msg) => die(&msg),
        },
        "-h" | "--help" => {
            print_help();
            ExitCode::SUCCESS
        }
        other => die(&format!("unknown subcommand `{other}`; try `--help`")),
    }
}

fn print_help() {
    eprintln!(
        "nucleus — Nuc v2 pre-compiler\n\
         \n\
         USAGE:\n    \
             nucleus build --algo FILE --sched FILE --backend NAME --out DIR \\\n    \
                           [--kernels FILE] [--capabilities FILE]\n\
         \n\
         BACKENDS:\n    \
             pthreads-sync\n"
    );
}

#[derive(Default)]
struct BuildArgs {
    algo: Option<PathBuf>,
    sched: Option<PathBuf>,
    backend: Option<String>,
    out: Option<PathBuf>,
    kernels: Option<PathBuf>,
    capabilities: Option<PathBuf>,
}

fn parse_build_args(argv: &[String]) -> Result<BuildArgs, String> {
    let mut a = BuildArgs::default();
    let mut i = 0;
    while i < argv.len() {
        let cur = argv[i].as_str();
        let val = || -> Result<&String, String> {
            argv.get(i + 1)
                .ok_or_else(|| format!("flag `{cur}` requires a value"))
        };
        match cur {
            "--algo" => {
                a.algo = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--sched" => {
                a.sched = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--backend" => {
                a.backend = Some(val()?.clone());
                i += 2;
            }
            "--out" => {
                a.out = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--kernels" => {
                a.kernels = Some(PathBuf::from(val()?));
                i += 2;
            }
            "--capabilities" => {
                a.capabilities = Some(PathBuf::from(val()?));
                i += 2;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    Ok(a)
}

fn cmd_build(argv: &[String]) -> Result<(), String> {
    let a = parse_build_args(argv)?;
    let algo_path = a.algo.ok_or("missing required --algo")?;
    let sched_path = a.sched.ok_or("missing required --sched")?;
    let backend = a.backend.ok_or("missing required --backend")?;
    let out_dir = a.out.ok_or("missing required --out")?;

    let kernels_path = match a.kernels {
        Some(p) => p,
        None => default_kernels_path(&algo_path),
    };

    // ---- Parse + lower + link ----
    let algo_src = read_file(&algo_path)?;
    let sched_src = read_file(&sched_path)?;

    let algo_ast = compiler::algo::parse_algo(&algo_src)
        .map_err(|e| format!("algorithm parse error in {}: {e}", algo_path.display()))?;
    let sched_ast = compiler::sched::parse_sched(&sched_src)
        .map_err(|e| format!("schedule parse error in {}: {e}", sched_path.display()))?;

    let algo_ir =
        compiler::algo::lower_algo(&algo_ast).map_err(|e| format!("algorithm lower error: {e}"))?;
    let sched_ir = compiler::sched::lower_sched(&sched_ast)
        .map_err(|e| format!("schedule lower error: {e}"))?;

    // ---- Contract check (best-effort; aggregate kernels report
    //      TypeMismatch — see TASK-0012; we surface for visibility
    //      but do not fail the build, as M1 example 01's aggregate
    //      I/O kernels intentionally trip this until aggregate
    //      matching lands). ----
    if !kernels_path.exists() {
        return Err(format!(
            "could not find kernels.rs at {}\n\
             (pass --kernels to override the default lookup)",
            kernels_path.display()
        ));
    }
    if let Err(errs) = check_kernels_contract(&algo_ir, &kernels_path) {
        eprintln!(
            "warning: contract check reported {} issue(s) (proceeding; aggregate-typed I/O is a known gap, TASK-0012):",
            errs.len()
        );
        for e in errs.iter().take(8) {
            eprintln!("  - {e}");
        }
        if errs.len() > 8 {
            eprintln!("  - (and {} more)", errs.len() - 8);
        }
    }

    let linked = link(algo_ir, sched_ir).map_err(|errs| {
        let mut s = format!("link error(s) ({}):", errs.len());
        for e in &errs {
            s.push_str("\n  - ");
            s.push_str(&e.to_string());
        }
        s
    })?;

    // ---- Build ACFG + block transforms + inject syncs + inject transfers ----
    //
    // Block-transform runs *between* ACFG construction and the
    // sync/transfer injection passes (TASK-0030). For schedules with
    // no `block=` directives (examples 01-03 at M2), this pass is a
    // pure identity and the downstream ACFG is bit-identical.
    let acfg = build_acfg(&linked);
    let acfg = apply_block_transforms(&linked, acfg)
        .map_err(|e| format!("block-transform error: {e}"))?;
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);

    // ---- Capability check ----
    let caps_path = match a.capabilities {
        Some(p) => p,
        None => find_default_capabilities(&backend).ok_or_else(|| {
            format!(
                "could not find capabilities.toml for backend `{backend}`; \
                 pass --capabilities to specify"
            )
        })?,
    };
    let caps = load_capabilities(&caps_path)
        .map_err(|e| format!("capabilities load error ({}): {e}", caps_path.display()))?;

    if let Err(mismatches) = check_schedule_compat(&caps, &linked.sched) {
        let mut s = format!("backend `{backend}` cannot satisfy this schedule:");
        for m in &mismatches {
            s.push_str("\n  - ");
            s.push_str(&m.to_string());
        }
        return Err(s);
    }

    // ---- Dispatch on backend ----
    match backend.as_str() {
        "pthreads-sync" => {
            let result = pthreads_sync::emit(&acfg, &linked, &kernels_path, &out_dir)
                .map_err(|e| format!("pthreads-sync codegen error: {e}"))?;
            // Print a deterministic, machine-parseable summary so the
            // e2e harness can pick up the run.sh path.
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        other => Err(format!(
            "unknown backend `{other}`; only `pthreads-sync` is registered at M1"
        )),
    }
}

fn default_kernels_path(algo_path: &Path) -> PathBuf {
    algo_path
        .parent()
        .map(|p| p.join("kernels.rs"))
        .unwrap_or_else(|| PathBuf::from("kernels.rs"))
}

/// Walk up from the current working directory looking for a
/// `nucleus/backends/<backend>/capabilities.toml` (the in-repo
/// canonical layout). If running outside the repo, returns None and
/// the user must pass `--capabilities` explicitly.
///
/// This is a convenience to keep the CLI short for the common case
/// (the e2e tests run from the workspace root and want to refer to a
/// sibling backend crate by name). It is deliberately *not*
/// load-bearing for correctness — the explicit flag always wins.
fn find_default_capabilities(backend: &str) -> Option<PathBuf> {
    let mut here = env::current_dir().ok()?;
    loop {
        let candidate = here
            .join("nucleus")
            .join("backends")
            .join(backend)
            .join("capabilities.toml");
        if candidate.exists() {
            return Some(candidate);
        }
        // Also try without the leading `nucleus/` (in case the user
        // is already inside the workspace dir).
        let candidate2 = here
            .join("backends")
            .join(backend)
            .join("capabilities.toml");
        if candidate2.exists() {
            return Some(candidate2);
        }
        if !here.pop() {
            return None;
        }
    }
}

fn read_file(p: &Path) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))
}

fn die(msg: &str) -> ExitCode {
    eprintln!("nucleus: error: {msg}");
    ExitCode::FAILURE
}
