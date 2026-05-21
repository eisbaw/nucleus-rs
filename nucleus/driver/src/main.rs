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
//!     [--capabilities PATH/capabilities.toml] \
//!     [--emit-pn PATH/schedule.dot]
//!
//! When `--kernels` is omitted, the driver looks for `kernels.rs`
//! next to the algorithm file. When `--capabilities` is omitted, it
//! looks for `nucleus/backends/<backend>/capabilities.toml` walking
//! up from the current working directory. Both defaults match how
//! the e2e tests invoke the binary.
//!
//! `--emit-pn PATH` (PRD §8.5, TASK-0035) writes the global Petri net
//! as a Graphviz DOT file. The pipeline still runs up through the
//! transfer-injection pass to produce a net, but `--out` becomes
//! optional in this mode — the user can ask for inspection without
//! also triggering backend codegen. If both `--out` and `--emit-pn`
//! are given, both outputs are produced.
//!
//! Registered backends: `pthreads-sync` (M1, shared-memory threads),
//! `mp-tcp-bufsync` (M3, OS processes over TCP loopback —
//! TASK-0036), and `pthreads-async` (M4, shared-memory + per-(DataId,
//! SeqTag) ring buffer + Condvar — TASK-0042.01; SKELETON only in
//! cycle 16, codegen body is TASK-0226). All three consume the
//! identical EventList contract; the cross-backend differential (same
//! source -> bit-identical output.bin) is the M3 headline (two-way
//! today) and the M4 headline (three-way once TASK-0229 lands the
//! pthreads-async e2e cells).

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use compiler::{
    acfg_to_events, acfg_to_net, apply_block_transforms, apply_partition_workers, build_acfg,
    build_sidecar, check_kernels_contract, check_schedule_compat, inject_check_frames,
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
             nucleus build --algo FILE --sched FILE --backend NAME \\\n    \
                           [--out DIR] [--kernels FILE] [--capabilities FILE] \\\n    \
                           [--emit-pn FILE]\n\
         \n\
         FLAGS:\n    \
             --emit-pn FILE   Write the global Petri net to FILE as Graphviz DOT.\n    \
                              Makes --out optional (inspection-only build).\n\
         \n\
         BACKENDS:\n    \
             pthreads-sync   shared-memory threads (tier 1)\n    \
             mp-tcp-bufsync  OS processes over TCP loopback (tier 1)\n    \
             pthreads-async  shared-memory + ring buffer (tier 1)\n                     \
                             (SKELETON — codegen is TASK-0226; capability\n                     \
                             matrix is real, codegen body is not yet wired)\n"
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
    emit_pn: Option<PathBuf>,
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
            "--emit-pn" => {
                a.emit_pn = Some(PathBuf::from(val()?));
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
    // --out is required for codegen, optional when --emit-pn alone is
    // requested (inspection-only build, TASK-0035 / PRD §8.5).
    let out_dir = match (&a.out, &a.emit_pn) {
        (Some(o), _) => Some(o.clone()),
        (None, Some(_)) => None,
        (None, None) => {
            return Err(
                "missing required --out (or use --emit-pn for an inspection-only build)".into(),
            );
        }
    };
    let emit_pn = a.emit_pn.clone();

    let kernels_path = match a.kernels {
        Some(p) => p,
        None => default_kernels_path(&algo_path),
    };

    // ---- Parse + lower + link ----
    let algo_src = read_file(&algo_path)?;
    let sched_src = read_file(&sched_path)?;

    // `parse_algo` recovers at statement/item boundaries and returns
    // ALL parse errors in one pass (TASK-0080 / TASK-0081). Surface
    // every one — each already carries its own correct 1-based
    // `line:col` — using the same header + one-line-per-error shape
    // the link / contract paths use, so a user fixing a syntactically
    // broken program sees every error at once rather than one
    // recompile per error. (TASK-0092 lowering multi-error will reuse
    // this surfacing template.)
    let algo_ast = compiler::algo::parse_algo(&algo_src).map_err(|errs| {
        let mut s = format!(
            "algorithm parse error(s) in {} ({}):",
            algo_path.display(),
            errs.errors().len()
        );
        for e in errs.errors() {
            s.push_str("\n  - ");
            s.push_str(&e.to_string());
        }
        s
    })?;
    // `parse_sched` recovers at the directive `;` boundary and
    // returns ALL parse errors in one pass (TASK-0087), mirroring the
    // algorithm parser above. Surface every one — each already carries
    // its own correct 1-based `line:col` — with the same header +
    // one-line-per-error shape, so a user fixing a syntactically
    // broken schedule sees every error at once rather than one
    // recompile per error.
    let sched_ast = compiler::sched::parse_sched(&sched_src).map_err(|errs| {
        let mut s = format!(
            "schedule parse error(s) in {} ({}):",
            sched_path.display(),
            errs.errors().len()
        );
        for e in errs.errors() {
            s.push_str("\n  - ");
            s.push_str(&e.to_string());
        }
        s
    })?;

    // `lower_algo` accumulates ALL genuinely-independent lowering
    // violations in one pass and returns them as `LowerErrors`
    // (TASK-0092) — it does NOT abort on the first, and it suppresses
    // cascade errors (a reference to a declaration that itself failed).
    // Surface every one — each carries its own byte span, resolved
    // here to `line:col` via `display_with_src` against the algorithm
    // source (TASK-0090; the driver holds the source, lowering does
    // not) — using the same header + one-line-per-error shape the
    // `parse_algo` / link / contract paths use, so a user fixing a
    // semantically broken program sees every error at once rather than
    // one recompile per error.
    let algo_ir = compiler::algo::lower_algo(&algo_ast).map_err(|errs| {
        let mut s = format!(
            "algorithm lower error(s) in {} ({}):",
            algo_path.display(),
            errs.errors().len()
        );
        for e in errs.errors() {
            s.push_str("\n  - ");
            s.push_str(&e.display_with_src(&algo_src));
        }
        s
    })?;
    // `lower_sched` accumulates ALL genuinely-independent lowering
    // violations in one pass and returns them as `SchedLowerErrors`
    // (TASK-0200, the schedule sibling of the algorithm-side
    // TASK-0092) — it does NOT abort on the first, and (when the
    // cascade infrastructure has a live trigger) it suppresses cascade
    // errors (a reference to a declaration that itself failed). Surface
    // every one — each carries its own byte span, resolved here to
    // `line:col` via `display_with_src` against the schedule source
    // (TASK-0196; the driver holds the source, lowering does not) —
    // using the same header + one-line-per-error shape the `parse_algo`
    // / `parse_sched` / `lower_algo` / link / contract paths use, so a
    // user fixing a semantically broken schedule sees every error at
    // once rather than one recompile per error.
    let sched_ir = compiler::sched::lower_sched(&sched_ast).map_err(|errs| {
        let mut s = format!(
            "schedule lower error(s) in {} ({}):",
            sched_path.display(),
            errs.errors().len()
        );
        for e in errs.errors() {
            s.push_str("\n  - ");
            s.push_str(&e.display_with_src(&sched_src));
        }
        s
    })?;

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
    let acfg = build_acfg(&linked).map_err(|e| format!("acfg build error: {e}"))?;
    let acfg =
        apply_block_transforms(&linked, acfg).map_err(|e| format!("block-transform error: {e}"))?;
    // Partition-workers loop-bound rewrite (TASK-0212): consume any
    // `loop X : partition=workers` directive whose body is multi-worker
    // and record a per-worker range override on the ACFG sidecar.
    // `petri_to_events` honours the override at projection time. Runs
    // after block-transform so the iter_var ids it records are the
    // final ones, and before sync/transfer injection (which do not
    // consult the sidecar — order is for diagnostic clarity).
    let acfg = apply_partition_workers(&linked, acfg)
        .map_err(|e| format!("partition-workers error: {e}"))?;
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

    // ---- Optional Petri-net dump (TASK-0035, PRD §8.5) ----
    //
    // Independent of backend codegen: any backend choice still
    // produces the same global net at this point in the pipeline.
    // We emit *before* codegen so a codegen failure on an
    // inspection-only build (no --out) doesn't suppress the DOT
    // file the user actually asked for.
    if let Some(pn_path) = &emit_pn {
        let net = acfg_to_net(&acfg);
        let title = format!(
            "{} | {} | {}",
            algo_path.display(),
            sched_path.display(),
            backend
        );
        let dot = net.serialize_to_dot_styled(Some(&title));
        write_emit_pn(pn_path, &dot)?;
        println!("emit_pn     = {}", pn_path.display());
    }

    // ---- Dispatch on backend (skipped for inspection-only builds) ----
    let Some(out_dir) = out_dir else {
        // --emit-pn without --out: net was written above; no codegen
        // to run. Print a minimal summary so callers can still parse
        // the success line.
        println!("nucleus: ok");
        return Ok(());
    };

    // Project the post-pass ACFG to the per-worker EventList contract
    // and build the codegen sidecar. EVERY EventList-consuming
    // backend takes exactly these + the reverse name tables — no
    // ACFG / LinkedIR (TASK-0124 AC#1/AC#2, carried to TASK-0036).
    //
    // build_sidecar can return a typed SidecarError (same-name loops
    // with different bounds — TASK-0170); surface it via the String
    // error channel exactly like apply_block_transforms above. The
    // EventList path is panic-safe (never aborts the process).
    let per_worker = acfg_to_events(&acfg);
    // TASK-0052.02: real-time `check loop V : latency_max=T` projection.
    // Joins `sched_ir.checks` (keyed by loop NAME) against the ACFG's
    // `name_iter_vars` (NAME -> IterVar id) and annotates each outer
    // source `Event::Loop` whose iter_var matches. The pass is a no-op
    // when `sched_ir.checks` is empty — preserves the pre-TASK-0052.02
    // e2e baseline byte-identically (no tier-1 cell uses `check loop`).
    let per_worker = inject_check_frames(per_worker, &linked.sched.checks, &acfg.name_iter_vars);
    let sidecar = build_sidecar(&linked, &acfg).map_err(|e| format!("sidecar error: {e}"))?;
    // Reverse name tables: invert acfg.name_* (name -> id) to
    // (id -> name) — the join key the EventList / sidecar use. Built
    // ONCE; both tier-1 backends share the identical tables (the
    // cross-backend differential requires identical inputs).
    let names = pthreads_sync::NameTables {
        data: acfg.name_data.iter().map(|(n, i)| (*i, n.clone())).collect(),
        kernel: acfg
            .name_kernels
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        worker: acfg
            .name_workers
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        iter_var: acfg
            .name_iter_vars
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        // The inner intra-tile loop iter-vars block_transform
        // produced (it reuses the source loop's IterVar on the inner
        // loop and iterates 0..N — the backend must rebind the
        // absolute index; TASK-0124).
        inner_block_iter_vars: acfg.inner_block_iter_vars.clone(),
    };

    match backend.as_str() {
        "pthreads-sync" => {
            let result =
                pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
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
        // Second tier-1 backend (TASK-0036/0037/0038): multi-process
        // over TCP loopback. Same contract; the only difference is
        // the transport. `run_sh` is the always-present entry point
        // (single-process AND multi-process) — the e2e harness keys
        // its multi-process run path off the backend's
        // `transport = "tcp"` capability + this run.sh.
        "mp-tcp-bufsync" => {
            let result =
                mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
                    .map_err(|e| format!("mp-tcp-bufsync codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            for (i, b) in result.worker_bins.iter().enumerate() {
                println!("worker_bin{i} = {}", b.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("wire_rs     = {}", result.wire_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Third tier-1 backend (TASK-0042.01): shared-memory + ring
        // buffer per (DataId, SeqTag). SKELETON in this cycle (16) —
        // `pthreads_async::emit` returns ContractGap until TASK-0226
        // lands the ring-buffer + Condvar codegen. The capability
        // matrix + dispatch wiring are real so that schedule authoring
        // can target this backend now; the user-facing error from this
        // arm carries the precise forward-link.
        "pthreads-async" => {
            let result =
                pthreads_async::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
                    .map_err(|e| format!("pthreads-async codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        other => Err(format!(
            "unknown backend `{other}`; registered: `pthreads-sync`, \
             `mp-tcp-bufsync`, `pthreads-async`"
        )),
    }
}

/// Write the rendered DOT string to `path`. Fails loudly with the
/// underlying io error and the path (PRD design rule: fail-fast,
/// contextual). If `path`'s parent directory does not exist we do
/// NOT silently create it — that would mask a typo (`--emit-pn
/// out/typo/x.dot` when the user meant `out/typoX/x.dot`). The
/// caller chose the path; if the parent is missing we surface the
/// raw filesystem error.
fn write_emit_pn(path: &Path, dot: &str) -> Result<(), String> {
    std::fs::write(path, dot)
        .map_err(|e| format!("cannot write Petri-net DOT to {}: {e}", path.display()))
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
