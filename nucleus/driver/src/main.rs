//! `nucleus` pre-compiler binary.
//!
//! Drives the M1 pipeline end-to-end:
//!
//!   parse algo + sched -> lower -> link -> build ACFG ->
//!   inject syncs -> inject transfers -> load backend capabilities ->
//!   check schedule/backend compat -> Petri-net soundness gate
//!   (boundedness + deadlock; TASK-0368) -> backend `emit(...)` ->
//!   emit a `run.sh` to the output directory.
//!
//! The Petri-net soundness gate (TASK-0368, PRD §8.1 / §8.4) runs on
//! every build: it builds the global net from the final ACFG and
//! rejects any net that overflows a place's capacity or deadlocks. A
//! failure is a compile error. The check is exact-replay over one
//! deterministic firing order — sound for v2's statically-ordered
//! restricted nets, not a general reachability engine.
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
//! Registered backends (one paragraph per backend so docs and clippy
//! agree on list structure): `pthreads-sync` is the M1 shared-memory-
//! threads backend; `mp-tcp-bufsync` is the M3 OS-processes-over-TCP-
//! loopback backend (TASK-0036); `pthreads-async` is the M4 shared-
//! memory + per-(DataId, SeqTag) ring-buffer + Condvar backend
//! (TASK-0042.01); `mp-tcp-event` is the M4 OS-processes + TCP-
//! loopback + mio-reactor + per-(seq, peer) outbound-queue + per-seq
//! inbound-queue backend (TASK-0042.05 / Stage 3 of TASK-0042.02
//! landed cycle 79); `openmp-rs` is the M6 rayon-threads backend
//! (single-worker arm landed cycle 191, multi-worker landed cycle 196
//! via TASK-0044.01.01); `mp-tcp-poll` is the M6 OS-processes +
//! TCP-loopback + nonblocking-poll backend (single-worker arm landed
//! cycle 192, multi-worker landed cycle 195 via TASK-0044.02.02);
//! `mp-uds-event` is the M6 OS-processes + Unix-domain-sockets + mio
//! backend (single-worker arm landed cycle 194, multi-worker landed
//! cycle 197 via TASK-0044.03.01).
//!
//! The four shipped (M1-M4) backends consume the identical
//! EventList contract; the cross-backend differential (same source
//! -> bit-identical output.bin) is the M3 headline (four-way), the
//! M4 headline (four-way as of cycle 165 — the AC#4 fourth column
//! 13/pipeline_parallel × mp-tcp-event was promoted bit-identical
//! via TASK-0329.01.02.01 after the CTRL arm was lifted upstream by
//! cycle 160's `apply_host_mediation_inject` pass — TASK-0329 marked
//! Done — and the in-`Repeat`-body DATA arm by cycles 163-164b's
//! `apply_host_data_relay_inject` — TASK-0329.01.02). The original
//! combined TASK-0175 worker-to-worker filing was split into
//! TASK-0327 (DATA top-level, cycles 148/149) and TASK-0329 (CTRL)
//! at cycle 148/149; both arms are now lifted upstream of every
//! tier-1 backend, so the per-backend `ContractGap` rejections at
//! `Plan::build` for host-excluding barriers / unmediated w↔w
//! `Push`-`Wait` are defense-in-depth, not load-bearing. All three
//! M6 backends are fully landed as of cycle 197 (single-worker +
//! multi-worker arms both live) — openmp-rs cycles 191/196,
//! mp-tcp-poll cycles 192/195, mp-uds-event cycles 194/197; each
//! participates in the cross-backend differential bit-identical
//! against its template (openmp-rs vs pthreads-sync, mp-tcp-poll
//! vs mp-tcp-bufsync, mp-uds-event vs mp-tcp-event). e2e baseline
//! at the cycle-197b M6-matrix-complete milestone:
//! 210/190/0/20/0 (total/pass/fail/skip/required-fail).

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use backend_common::elect_host_from_name_workers;
use nucleus_compiler::{
    acfg_to_events, acfg_to_net, apply_block_transforms, apply_halo_inference_partition_aware,
    apply_host_data_relay_inject, apply_host_mediation_inject, apply_partition_blocks2d,
    apply_partition_rows, apply_partition_workers, apply_reuse_inference, apply_safe_push_reorder,
    build_acfg, build_sidecar, check_kernels_contract, check_net_sound, check_schedule_compat,
    inject_check_frames, inject_syncs, inject_transfers, link, load_capabilities,
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
                           [--emit-pn FILE] [--shim NAME]\n\
         \n\
         FLAGS:\n    \
             --emit-pn FILE   Write the global Petri net to FILE as Graphviz DOT.\n    \
                              Makes --out optional (inspection-only build).\n    \
             --shim NAME      Tier-3 target shim (embedded-pattern only). `stm32h7`\n    \
                              emits a Renode-runnable no_std BIN; omit for the M9\n    \
                              compile-only no_std LIB.\n\
         \n\
         BACKENDS:\n    \
             pthreads-sync   shared-memory threads (tier 1)\n    \
             mp-tcp-bufsync  OS processes over TCP loopback (tier 1)\n    \
             pthreads-async  shared-memory + ring buffer (tier 1)\n    \
             mp-tcp-event    OS processes + TCP loopback + mio (tier 1)\n    \
             openmp-rs       rayon threads (tier 1, single-worker + multi-worker landed cycles 191/196)\n    \
             mp-tcp-poll     OS processes + TCP loopback + nonblocking poll (tier 1, single-worker + multi-worker landed cycles 192/195)\n    \
             mp-uds-event    OS processes + Unix domain sockets + mio (tier 1, single-worker + multi-worker landed cycles 194/197)\n    \
             embedded-pattern  no_std lib + NucleusShim trait (tier 3, compile-only; check via `just check-embedded`).\n    \
                               LIB path: single-worker (M9) OR multi-worker (M11 slice A, TASK-0049.04 — one lib per worker, Push/Wait/Sync -> stub-shim hooks).\n    \
                               With `--shim stm32h7`: Renode-runnable no_std bin (M10, single-worker only; `just renode-embedded <example>`; examples 1/5/9)\n    \
             mpi-blocking    SPMD MPI (tier 2, M7); one rank-dispatched binary + rsmpi. Builds/runs under `nix develop .#mpi` via `just check-mpi`.\n    \
                               Single-worker SPMD arm landed; multi-worker (rank Send/Recv + MPI_Barrier) is TASK-0045.01.\n"
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
    // Target shim selector (PRD §10.3 quad: algorithm, schedule, tier-3
    // backend, target shim). M10 (TASK-0048.01): `--shim stm32h7` makes
    // the embedded-pattern backend emit a Renode-runnable no_std BIN
    // instead of the M9 compile-only LIB. No `--shim` => the M9 lib.
    shim: Option<String>,
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
            "--shim" => {
                a.shim = Some(val()?.clone());
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
    // Optional target-shim selector (M10, TASK-0048.01). Captured here
    // because `a` is consumed field-by-field below; the embedded-pattern
    // dispatch arm reads it to pick the bin vs lib emit mode.
    let shim = a.shim.clone();
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
    let algo_ast = nucleus_compiler::algo::parse_algo(&algo_src).map_err(|errs| {
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
    let sched_ast = nucleus_compiler::sched::parse_sched(&sched_src).map_err(|errs| {
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
    let algo_ir = nucleus_compiler::algo::lower_algo(&algo_ast).map_err(|errs| {
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
    let sched_ir = nucleus_compiler::sched::lower_sched(&sched_ast).map_err(|errs| {
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

    // ---- Contract check (best-effort; surfaced for visibility but
    //      does NOT fail the build). Issues can have MULTIPLE distinct
    //      causes — e.g. aggregate-typed I/O still reports a non-fatal
    //      TypeMismatch (TASK-0012, until aggregate matching lands).
    //      The warning text below is deliberately GENERIC so it does
    //      not misattribute every issue to one cause: each individual
    //      issue is printed below the header (TASK-0363 — a dotted-stem
    //      kernels-file rustc rejection was previously conflated with
    //      the TASK-0012 aggregate gap). ----
    if !kernels_path.exists() {
        return Err(format!(
            "could not find kernels.rs at {}\n\
             (pass --kernels to override the default lookup)",
            kernels_path.display()
        ));
    }
    if let Err(errs) = check_kernels_contract(&algo_ir, &kernels_path) {
        eprintln!(
            "warning: contract check reported {} issue(s) (proceeding; see individual issues below):",
            errs.len()
        );
        for e in errs.iter().take(8) {
            eprintln!("  - {e}");
        }
        if errs.len() > 8 {
            eprintln!("  - (and {} more)", errs.len() - 8);
        }
    }

    // `link` collects ALL cross-reference / coverage / cross-worker /
    // pipeline violations in one pass (PRD §12, no fail-fast). Surface
    // every one — each carries its own byte span PLUS a
    // `LinkErrorSource` tag identifying whether the span indexes into
    // the algorithm or schedule source string (TASK-0099; the link
    // step takes both IRs, so its diagnostics can point at either
    // side). `display_with_src` resolves the byte offset to `line:col`
    // against the right source, mirroring how `LowerError` /
    // `SchedLowerError` are surfaced (TASK-0090 / TASK-0196).
    let linked = link(algo_ir, sched_ir).map_err(|errs| {
        let mut s = format!("link error(s) ({}):", errs.len());
        for e in &errs {
            s.push_str("\n  - ");
            s.push_str(&e.display_with_src(&algo_src, &sched_src));
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
    // Partition-rows row-band rewrite (TASK-0258): consume any
    // `loop X : partition=rows` directive whose outer-of-2D-nest body
    // is multi-worker. Writes into the SAME sidecar field
    // `partition_worker_ranges` as apply_partition_workers — downstream
    // consumers (sync_inject, petri_to_events, the backend walkers)
    // do not distinguish which directive produced the per-worker
    // range. The two passes target disjoint IterVar keys by grammar
    // construction (at most one `partition=` option per loop), so
    // order is observationally irrelevant; this call sits immediately
    // after apply_partition_workers for diagnostic clarity.
    let acfg =
        apply_partition_rows(&linked, acfg).map_err(|e| format!("partition-rows error: {e}"))?;
    // Partition-blocks2d 2D-grid rewrite (TASK-0259): consume any
    // `loop X : partition=blocks2d` directive whose outer-of-2D-nest
    // body is multi-worker. Writes TWO entries into
    // `partition_worker_ranges` (one under the outer iter_var for the
    // per-worker y-band, one under the inner iter_var for the per-
    // worker x-band) — the walker's independent per-iter_var lookup
    // applies each on the appropriate Repeat. Order between the three
    // partition passes is observationally irrelevant (grammar accepts
    // at most one `partition=` per loop → disjoint IterVar keys). This
    // call sits immediately after apply_partition_rows for diagnostic
    // clarity.
    let acfg = apply_partition_blocks2d(&linked, acfg)
        .map_err(|e| format!("partition-blocks2d error: {e}"))?;
    // Halo-region inference (TASK-0260 Stage 1 + TASK-0275 promotion):
    // walk `linked.algo.stmts` and infer per-(KernelId, IterVar) halo
    // widths from affine kernel-arg DataRef indices. Populates
    // `ACFG::halo_widths` for the transfer_inject consumer (TASK-0263,
    // cycle 83, commit cf2f9ac) to extend per-tile transfer ranges.
    //
    // DRIVER POLICY (TASK-0275): (B) PARTITION-POLICY-AWARE. For each
    // typed `HaloInferenceError` the walker raises, the entry point
    // looks at the enclosing-loop scope at the error-push site. If
    // ANY iv in that scope carries a `partition=` directive in the
    // schedule, the error is FATAL — the transfer_inject consumer
    // would otherwise silently emit wrong-output tiles (missing halo
    // strips on a partition boundary). Otherwise the error goes to an
    // advisory bucket and lowering proceeds; the walker's partial halo
    // widths for unaffected kernels stay committed.
    //
    // Why (B) and NOT (A) strict (the TASK-0271 reuse precedent):
    // the transfer_inject halo consumer is CONDITIONAL on the iv being
    // partitioned. The reuse Tier 1 marker (TASK-0265) by contrast
    // fires for EVERY recognised slot regardless of partition, so for
    // reuse the (B) predicate "is this slot consumed?" was trivially
    // true and (B) degenerated into (A). For halo it does not — the
    // example-11 `step_or_seed` body reads
    // `grid[(t + ITERS) % (ITERS + 1)]` (a constant Mod wrap the
    // affine detector cannot fold), and the schedule carries ZERO
    // `partition=` directives. A naive (A) strict mirror would
    // newly-reject example 11; (B) keeps the cells PASS while still
    // failing loud on the cases where backend output would be wrong.
    // The decision context is TASK-0263 cycle-89 verification +
    // TASK-0275's full reasoning.
    //
    // E2E impact at promotion: 92/77/0/15/0 byte-identical. The only
    // shipped halo-affecting partition= directive is
    // `05-stencil/distributed`'s y-loop (`partition=workers`) whose
    // `blur3` body is fully affine; example 11's two cells stay PASS
    // because no `partition=` is attached to the Mod-indexed iv.
    let (acfg, halo_errors_advisory) = apply_halo_inference_partition_aware(&linked, acfg)
        .map_err(|e| format!("halo-inference error (under partitioned iv): {e}"))?;
    for e in &halo_errors_advisory {
        nucleus_compiler::nuc_trace!(
            "halo_inference: advisory (no `partition=` directive in scope, transfer_inject \
             halo consumer will not fire on the affected iv — lowering proceeds): {e}"
        );
    }
    // Reuse loop-option inference (TASK-0261 Stage 1 + TASK-0271
    // promotion): walk every `for V : reuse;` loop in
    // `linked.algo.stmts` and infer per-(IterVar, DataId, axis)
    // delay-line slots from affine `iv + b` DataRef accesses. Populates
    // `ACFG::reuse_widths` for the Tier 1 marker consumer (TASK-0265
    // cycle 87) and the forthcoming circular-buffer codegen (TASK-0269
    // pthreads-sync + TASK-0270 multi-worker walker).
    //
    // STAGE 2 DRIVER POLICY (TASK-0271): STRICT. The cycle-87 Tier 1
    // landing wired a walker-side marker consumer at the `Event::Loop`
    // emit site (`render_reuse_marker_comment`), so EVERY recognised
    // slot is consumed by the backend today. The cost-of-silent-swallow
    // is therefore real: a non-affine `loop V : reuse;` body produces
    // no marker line (and tomorrow no buffer code) without warning,
    // surprising the user who wrote `reuse;`. We promote to the strict
    // entry point — any typed `ReuseInferenceError` is a fatal compile
    // error, surfaced via the existing `Display` impl (variant docs at
    // `passes::reuse_inference::ReuseInferenceError`).
    //
    // Why strict over partition-policy-aware (option B in TASK-0271):
    // every reuse slot already has a consumer (the Tier 1 marker), so
    // the (B) predicate "is this slot consumed?" is trivially TRUE for
    // every slot — (B) degenerates into (A). The narrower (B) rule
    // "iv carries partition=" would still silently drop non-affine
    // reuse on non-partitioned loops (exactly the 05-stencil/reuse
    // shape), recreating the silent-failure mode. Strict closes both.
    //
    // E2E impact at promotion: 92/77/0/15/0 byte-identical. The only
    // shipped non-skipped reuse loop (`05-stencil/reuse`, single-host
    // 3x3 stencil over `img_in[y±1][x±1]`) is fully affine and lowers
    // cleanly. `05-stencil/distributed` also carries `reuse;` but is
    // SKIP across all backends per TASK-0267/TASK-0268 (unrelated to
    // reuse codegen); when those clear, its `blur3` body is the same
    // affine shape and will lower cleanly too.
    //
    // The sibling halo driver call above was promoted to (B)
    // partition-policy-aware in TASK-0275 (cycle 96), NOT to (A) strict
    // as this reuse call. The asymmetry is intentional: halo's
    // `transfer_inject` consumer is conditional on the iv being
    // partitioned, so naive (A) strict would newly-reject example 11
    // (Mod-indexed `step_or_seed` with no partition= in scope). Reuse's
    // Tier 1 marker fires on every recognised slot, so for reuse (B)
    // degenerated into (A). The two driver calls' five-line shapes are
    // similar (`apply_X(...).map_err(|e| ...)?`) but the entry-point
    // names + the role of any returned advisory bucket differ — the
    // speculative `iv_diag_policy` helper from cycle-87 review still
    // has no real substance to lift.
    let acfg =
        apply_reuse_inference(&linked, acfg).map_err(|e| format!("reuse-inference error: {e}"))?;
    let acfg = inject_syncs(acfg).map_err(|e| format!("sync-injection error: {e}"))?;
    let acfg =
        inject_transfers(&linked, acfg).map_err(|e| format!("transfer-injection error: {e}"))?;

    // TASK-0329 cycle 160 — host-mediation injection (CTRL arm of the
    // cycle-148/149 split of the original TASK-0175 combined filing).
    // For `mp-tcp-bufsync` and `mp-tcp-event`, the
    // one-CTRL-stream-per-(host,worker) star topology cannot lower a
    // host-excluding barrier without a worker-to-worker mesh. Adding
    // host as a mediating hub turns each host-excluding barrier into
    // a star-shaped N+1-party rendezvous through host, which the
    // existing barrier-shim emitter handles transparently. The pass
    // is structurally idempotent and a no-op for ACFGs whose every
    // barrier already includes host. pthreads-sync / pthreads-async
    // do NOT apply this pass — their shared-memory barrier primitives
    // handle host-excluding barriers natively (std::sync::Barrier on
    // an Arc shared only among the listed participants).
    //
    // Applied AFTER inject_syncs / inject_transfers (the passes that
    // emit barriers) and BEFORE acfg_to_events (so the projection
    // naturally places host's Sync at the structurally correct
    // position, preserving any enclosing Repeat / Sequence nesting).
    //
    // **Cycle 195 (TASK-0044.02.02)**: gate widened to include
    // `mp-tcp-poll` because the poll backend's multi-worker arm
    // inherits the same one-CTRL-stream-per-(host,worker) star
    // topology as mp-tcp-bufsync — its `Plan::build` carries the same
    // defensive ContractGap on host-excluding barriers. The
    // host-mediation pass is needed for any mp-tcp-poll schedule with
    // a host-excluding barrier (e.g. 03-reduction/distributed) to
    // lower correctly.
    //
    // **Cycle 197 (TASK-0044.03.01)**: gate widened to include
    // `mp-uds-event` for the same reason — UDS-star topology is
    // structurally identical to TCP-star (one-CTRL-stream-per-(host,
    // worker) over UnixStream instead of TcpStream); capability
    // surface mirrors mp-tcp-event. The mp-uds-event Plan::build
    // carries the SAME defensive ContractGap on host-excluding
    // barriers, so the pass is needed end-to-end for cells like
    // 03-reduction/distributed × mp-uds-event.
    let acfg = if backend == "mp-tcp-bufsync"
        || backend == "mp-tcp-event"
        || backend == "mp-tcp-poll"
        || backend == "mp-uds-event"
    {
        // WHY this pass needs the SAME host the backend will elect:
        // a schedule whose "host" worker is declared but has zero
        // projected events (unusual but possible) would otherwise
        // mediate against an ID the backend does not elect as host,
        // and the backend's defensive rejection would re-fire
        // against the BACKEND-elected host (cycle-160 architect
        // P1.1). The rule itself lives in `backend_common::host_election`;
        // here we only build the `used` set the rule consumes.
        //
        // We project ONCE here (preview) to elect, then mediate, then
        // re-project later — the mediation may add Sync events to
        // host's list, making the post-mediation projection the
        // authoritative per_worker. Cost: one extra `acfg_to_events`
        // call (O(ACFG nodes)) — cheap vs the cross-backend skew this
        // would otherwise leak into the differential.
        let preview = acfg_to_events(&acfg);
        let used: std::collections::BTreeSet<_> = preview
            .iter()
            .filter(|(_, evs)| !evs.is_empty())
            .map(|(w, _)| *w)
            .collect();
        // Host election: shared helper (TASK-0336 cycle 164). See
        // `backend_common::host_election` for the canonical rule
        // (memory feedback-driver-must-mirror-backend-election-exactly).
        let host = elect_host_from_name_workers(&acfg.name_workers, &used);
        match host {
            Some(h) => apply_host_mediation_inject(acfg, h),
            // No `used_workers` (every per_worker entry empty). The
            // ACFG is degenerate (no barriers possible); pass through.
            None => acfg,
        }
    } else {
        acfg
    };

    // TASK-0329.01.02 slice 2 — host-mediated data-relay injection,
    // mp-tcp-event ONLY at cycle 163; widened to ALSO include
    // mp-uds-event cycle 197 (TASK-0044.03.01). AC#5 bufsync audit:
    // bufsync 09/13 cells are capability-gated, so behavioral
    // verification is impossible; applying the pass on bufsync would
    // have no defensible gain today. For every Push/Wait pair whose
    // endpoints are BOTH non-host, replaces the pair with four sibling
    // Xfers routing the transfer through host. The new host endpoints
    // project naturally onto host's per-worker event list including
    // INSIDE Repeat bodies — this is what unblocks
    // 09-producer-consumer/pipelined × mp-tcp-event (per-iter w2w
    // Push inside `for n in 0..16`), which the TASK-0330 defensive
    // guard at `collect_w2w_pushes` would otherwise reject. The same
    // defensive guard lives in mp-uds-event's collect_w2w_pushes
    // sibling (cycle 197 copy-of-mp-tcp-event); without this pass
    // running on mp-uds-event, 09/pipelined × mp-uds-event would hit
    // the SAME ContractGap.
    //
    // Applied AFTER `apply_host_mediation_inject` so the CTRL-arm and
    // DATA-arm passes compose cleanly (Sync participants are already
    // host-augmented when this pass runs; this pass only touches Xfer
    // nodes). Applied BEFORE the capability check + `acfg_to_events`
    // so the projection sees the rewritten ACFG.
    //
    // Host election: same shared helper as the cycle-160 wiring above
    // (rule lives in `backend_common::host_election`; same mirroring
    // requirement vs the backend's Plan::build).
    let acfg = if backend == "mp-tcp-event" || backend == "mp-uds-event" {
        // Re-derive `used_workers` from the post-mediation ACFG. The
        // host_mediation pass may have added host to Sync participants
        // but doesn't change which workers project events; nonetheless
        // we re-project to get the authoritative view (cheap, matches
        // the slice-1 safe_push_reorder host election pattern).
        let preview = acfg_to_events(&acfg);
        let used: std::collections::BTreeSet<_> = preview
            .iter()
            .filter(|(_, evs)| !evs.is_empty())
            .map(|(w, _)| *w)
            .collect();
        // Host election: shared helper. See
        // `backend_common::host_election` module docstring for the
        // canonical rule (TASK-0336 cycle 164 lift).
        let host = elect_host_from_name_workers(&acfg.name_workers, &used);
        match host {
            Some(h) => apply_host_data_relay_inject(acfg, h),
            None => acfg,
        }
    } else {
        acfg
    };

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

    // ---- Petri-net soundness gate (TASK-0368, PRD §8.1 / §8.4) ----
    //
    // Build the global net from the FINAL post-transform ACFG (after
    // every pass, including the backend-conditional host-mediation /
    // host-data-relay injections above) and check it is bounded and
    // deadlock-free. This is what makes PRD §8's "analyses fall out as
    // standard properties; failures are compile errors" literally true
    // of the shipping compiler, not just the test suite.
    //
    // The gate runs on EVERY build — both inspection-only (`--emit-pn`,
    // no `--out`) and full codegen builds — and deliberately AFTER the
    // `--emit-pn` DOT dump above, so a user inspecting an unsound net
    // still gets the DOT file for debugging before the build errors.
    //
    // `acfg_to_net` is recomputed here (the net inside the `--emit-pn`
    // block is scoped to that `if let`); construction is O(net) and the
    // replay is O(firing_order), so a fresh build is cheap. The gate is
    // exact-replay over one deterministic firing order — sound for v2's
    // statically-ordered restricted nets, NOT a general reachability
    // engine (see `passes::net_soundness` module doc). On the shipping
    // pipeline it is a provably-dead-today tripwire: the structural
    // inject-pass guards mean no valid schedule produces an unsound net
    // (empirically verified over all examples x schedules x 7 tier-1
    // backends, TASK-0368). The negative test in
    // `nucleus-compiler/tests/net_soundness.rs` pins the reject path at
    // the function level so a future inject-pass regression that DID
    // emit an unsound net would surface as a compile error here rather
    // than as a runtime hang or buffer overrun.
    let gate_net = acfg_to_net(&acfg);
    check_net_sound(&gate_net).map_err(|e| format!("petri-net soundness check failed: {e}"))?;

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
    // TASK-0329.01.01 slice 1 — safe push-before-wait reordering for
    // mp-tcp-event (cycle 162) and mp-uds-event (cycle 197 widening
    // for TASK-0044.03.01). Hoists hoistable worker-to-worker `Push`
    // events above preceding w2w `Wait` events within each non-host
    // worker's top-level boundaries, breaking the wait-before-push
    // deadlock cycle on the synchronous host-relay (cycle-149) for
    // schedules like 05-stencil/distributed-2d. mp-uds-event inherits
    // the SAME per-seq-demux wait primitive + synchronous host-relay
    // shape from its mp-tcp-event sibling (cycle 197 multi_worker/
    // copy), so the same hoist is needed. Per AC#3 of TASK-0329.01.01:
    // only the per-seq-demux event backends (mp-tcp-event +
    // mp-uds-event) can safely move ahead of the first wait-bearing
    // Sync; mp-tcp-bufsync's / mp-tcp-poll's constraint 3 (per-pair
    // FIFO stream + host's own w2w Waits would race the moved relay)
    // makes the analogous lift unsafe on those backends. Other
    // backends do NOT have the host-relay deadlock surface
    // (pthreads-* use direct w↔w channels; openmp-rs uses rayon
    // shared-memory Slots). Pass is observationally a no-op for
    // schedules that have no wait-before-push shape — preserves
    // bit-identity for every currently-passing mp-tcp-event /
    // mp-uds-event cell.
    let per_worker = if backend == "mp-tcp-event" || backend == "mp-uds-event" {
        // Host election: shared helper. See
        // `backend_common::host_election` module docstring for the
        // canonical rule (TASK-0336 cycle 164 lift). `used` here =
        // workers with non-empty event lists (the same projection
        // the backend's Plan::build will see).
        let used: std::collections::BTreeSet<_> = per_worker
            .iter()
            .filter(|(_, evs)| !evs.is_empty())
            .map(|(w, _)| *w)
            .collect();
        let host = elect_host_from_name_workers(&acfg.name_workers, &used);
        match host {
            Some(h) => apply_safe_push_reorder(per_worker, h),
            None => per_worker,
        }
    } else {
        per_worker
    };
    let sidecar = build_sidecar(&linked, &acfg).map_err(|e| format!("sidecar error: {e}"))?;
    // Reverse name tables: invert acfg.name_* (name -> id) to
    // (id -> name) — the join key the EventList / sidecar use. Built
    // ONCE; both tier-1 backends share the identical tables (the
    // cross-backend differential requires identical inputs).
    // TASK-0238 (cycle 25): the 5-field composition collapsed into
    // the centralized constructor.
    let names = nucleus_compiler::NameTables::from_acfg(&acfg);

    // ---- Overlapping-write accumulator algorithm-level cross-check
    //      (TASK-0343.03; hardens the cycle-189 structural detector
    //      TASK-0343). ----
    //
    // The backends classify the overlapping-write accumulator fan-in
    // pattern PURELY STRUCTURALLY (per worker, >=2 whole-array Waits on
    // one data symbol ⇒ element-wise sum combine at the host —
    // `backend_common::multi_worker_walker::collect_accumulate_waits`).
    // For every shipped schedule that structural shape coincides with
    // the algorithm-level accumulator shape (LHS-appears-in-RHS, e.g.
    // 08-histogram's `histogram[b] <-- bin_inc(histogram[b], ...)`), so
    // this gate is a NO-OP on the entire e2e matrix. It exists to FAIL
    // LOUD if an exotic schedule ever emits multiple whole-array pushes
    // for NON-accumulator semantics, which the structural detector would
    // otherwise silently mis-combine as a sum (a silent miscompile).
    //
    // Gated ONCE here, BEFORE the codegen dispatch below: the structural
    // detector is shared across all backends (backend-common), the
    // algorithm-IR is the same for any backend choice, and `per_worker` /
    // `sidecar` / `names` are already built. The check reuses the EXACT
    // structural detector the backends consume (`collect_pair_tiles` +
    // `collect_accumulate_waits`) — no duplicated detection logic — and
    // consults `linked.algo` for the LHS-appears-in-RHS accumulator shape
    // via `names.data` (DataId -> name) as the bridge between the codegen
    // DataId space and the algorithm-IR String-name space.
    //
    // NOTE on the `per_worker` it reads: for mp-tcp-event / mp-uds-event
    // this is the `safe_push_reorder`-transformed map (built above), not
    // the raw projection. That is fine — the detector is order-insensitive
    // (`walk_waits` count + `.all()` whole-array predicate + `BTreeSet`
    // output) and the reorder never changes a Wait's data/seq/tile — so
    // the cross-check RESULT is backend-independent even though the input
    // map differs per backend.
    backend_common::multi_worker_walker::check_accumulator_consistency(
        &linked.algo,
        &per_worker,
        &sidecar,
        &names.data,
    )
    .map_err(|e| format!("accumulator cross-check error: {e}"))?;

    // `--shim` is a tier-3 selector (M10, TASK-0048.01). A `--shim` on a
    // backend that has no shim concept is a user error — reject it loudly
    // rather than silently ignore it (PRD fail-fast rule). Only the
    // embedded-pattern backend consults `shim` below.
    if shim.is_some() && backend != "embedded-pattern" {
        return Err(format!(
            "--shim is only meaningful for the embedded-pattern backend \
             (a tier-3 target shim, PRD \u{00A7}10.3); backend `{backend}` does \
             not take a shim"
        ));
    }

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
        // Fourth tier-1 backend (TASK-0042.02 / TASK-0042.05): OS
        // processes + TCP loopback + mio reactor + per-(seq, peer)
        // outbound queue + per-seq inbound queue. SYNC -> ASYNC
        // upgrade of mp-tcp-bufsync (same shape pthreads-async is to
        // pthreads-sync). Stages 1+2 (skeleton + single-worker
        // delegation) landed cycle 41; Stage 3 (multi-worker mio
        // reactor + Chan<T>) landed cycle 79 — verified bit-identical
        // against reference.bin on 3 cells.
        //
        // Worker-to-worker Pushes were LIFTED in TASK-0327 cycle 149
        // via host-relay (Reactor::relay_one + Plan::render_relay_phase),
        // plus TASK-0329.01.02 cycles 163-164b for in-`Repeat`-body
        // pairs. Host-excluding barriers were lifted cycle 160 via
        // TASK-0329's `apply_host_mediation_inject` pass — marked Done.
        // The backend's ContractGap rejection in `Plan::build` (wire
        // text still cites TASK-0175 for test-pin compatibility) is
        // now defense-in-depth — should not fire for ACFGs that came
        // through the driver's pipeline.
        "mp-tcp-event" => {
            let result = mp_tcp_event::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
                .map_err(|e| format!("mp-tcp-event codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            for (i, b) in result.worker_bins.iter().enumerate() {
                println!("worker_bin{i} = {}", b.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("wire_rs     = {}", result.wire_rs.display());
            if let Some(r) = &result.runtime_rs {
                println!("runtime_rs  = {}", r.display());
            }
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Fifth tier-1 backend (TASK-0044.01, M6): rayon threads +
        // shared memory + barrier + sync — same capability surface as
        // pthreads-sync (sync + shared-memory + barrier/blocking
        // notify), differing only in runtime substrate (rayon scope
        // instead of std::thread). Status as of cycle 197 (M6 matrix
        // complete): single-worker arm landed cycle 191 (delegates to
        // pthreads-sync's `render_single_worker_main` + backend-common's
        // project-skeleton, byte-identical to pthreads-sync /
        // pthreads-async); multi-worker arm landed cycle 196 via
        // TASK-0044.01.01 (rayon-scope codegen + 8 [[required]] cells
        // bit-identical vs pthreads-sync template).
        "openmp-rs" => {
            let result = openmp_rs::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
                .map_err(|e| format!("openmp-rs codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // Sixth tier-1 backend (TASK-0044.02, M6): OS processes + TCP
        // loopback + nonblocking poll + sync — same capability surface
        // as mp-tcp-bufsync, differing only in the wait primitive
        // (nonblocking-read poll loop instead of blocking recv). Status
        // as of cycle 197 (M6 matrix complete): single-worker arm landed
        // cycle 192 (delegates to pthreads-sync's
        // `render_single_worker_main_with_kernels_attr` +
        // backend-common's multi_binary skeleton, byte-identical to
        // mp-tcp-bufsync's single-process output); multi-worker arm
        // landed cycle 195 via TASK-0044.02.02 (nonblocking-poll
        // codegen + 8 [[required]] cells bit-identical vs mp-tcp-bufsync
        // template). Multi-binary shape (same dispatch fields as
        // mp-tcp-bufsync / mp-tcp-event).
        "mp-tcp-poll" => {
            let result = mp_tcp_poll::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
                .map_err(|e| format!("mp-tcp-poll codegen error: {e}"))?;
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
        // Seventh tier-1 backend (TASK-0044.03, M6): OS processes +
        // Unix domain sockets + mio/epoll + async + buffer — same
        // capability surface as mp-tcp-event, differing only in
        // transport (UDS instead of TCP loopback). Status as of cycle
        // 197 (M6 matrix complete): single-worker arm landed cycle 194
        // (delegates to pthreads-sync's
        // `render_single_worker_main_with_kernels_attr` +
        // backend-common's multi_binary skeleton, byte-identical to
        // mp-tcp-event's single-process output; wire.rs emitted from
        // mp_tcp_common::WIRE_RUNTIME_SRC verbatim for shape uniformity,
        // unused because the single-process bin does not `mod wire;`);
        // multi-worker arm landed cycle 197 via TASK-0044.03.01
        // (UDS-reactor codegen + 13 [[required]] cells bit-identical
        // vs mp-tcp-event template; transport-layer lift filed as
        // TASK-0044.03.02 follow-up). Multi-binary shape with optional
        // runtime_rs (same as mp-tcp-event).
        "mp-uds-event" => {
            let result = mp_uds_event::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
                .map_err(|e| format!("mp-uds-event codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            for (i, b) in result.worker_bins.iter().enumerate() {
                println!("worker_bin{i} = {}", b.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("wire_rs     = {}", result.wire_rs.display());
            if let Some(r) = &result.runtime_rs {
                println!("runtime_rs  = {}", r.display());
            }
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        // First tier-3 backend (TASK-0047, M9): the generic
        // `embedded-pattern` backend. The LIB path emits COMPILE-ONLY
        // `no_std` lib project(s) (Cargo.toml + src/lib.rs only — no
        // main.rs, no run.sh: there is nothing to RUN for a compile-only
        // lib; a Renode-runnable bin is M10's job, TASK-0048). It lowers
        // the per-worker EventList against a `NucleusShim` trait +
        // do-nothing stub shim. As of TASK-0049.04 (M11 backend slice A)
        // the LIB path is MULTI-worker: a multi-worker schedule emits ONE
        // lib per used worker (under out_dir/<worker>/) and lowers the
        // cross-worker Push/Wait/Sync to the stub-shim hooks; a
        // single-worker schedule still emits ONE project at out_dir root.
        // Acceptance: `cargo check --target thumbv7em-none-eabihf` (run
        // under `nix develop .#embedded` via `just check-embedded`). This
        // backend is NOT in the e2e-matrix.toml backends list — the
        // tier-1 runtime differential runs+diffs host binaries, which is
        // wrong for a compile-only no_std backend. The Renode MULTI-MCU
        // bin (`--shim stm32h7` multi-worker) is M11 slice B (TASK-0049.05).
        "embedded-pattern" => {
            // --shim selects the emit mode (PRD §10.3 quad's "target
            // shim", M10 TASK-0048.01):
            //   --shim stm32h7  -> Renode-runnable no_std BIN (cortex-m-rt
            //                      entry + panic handler + memory.x +
            //                      real-input load from the injected region
            //                      + raw USART1 output streaming).
            //   (no --shim)     -> the M9 compile-only no_std LIB
            //                      (UNCHANGED — `just check-embedded`).
            // An unrecognised shim name is a typed (not panicking) error.
            match shim.as_deref() {
                None => {
                    // TASK-0049.04: emit() now returns ONE lib project
                    // per used worker (single-worker -> one project at
                    // out_dir root; multi-worker -> one under
                    // out_dir/<worker_name>/ each). Print every project
                    // so a caller (e.g. `just check-embedded`) can locate
                    // and cross-compile each one.
                    let result = embedded_pattern::emit(
                        &per_worker,
                        &names,
                        &sidecar,
                        &kernels_path,
                        &out_dir,
                    )
                    .map_err(|e| format!("embedded-pattern codegen error: {e}"))?;
                    println!("nucleus: ok");
                    println!("worker_projects = {}", result.workers.len());
                    for w in &result.workers {
                        match &w.worker_name {
                            Some(name) => println!("worker      = {name}"),
                            None => println!("worker      = (single)"),
                        }
                        println!("project_dir = {}", w.project_dir.display());
                        println!("cargo_toml  = {}", w.cargo_toml.display());
                        println!("lib_rs      = {}", w.lib_rs.display());
                    }
                    Ok(())
                }
                Some("stm32h7") => {
                    let result = embedded_pattern::emit_bin(
                        &per_worker,
                        &names,
                        &sidecar,
                        &kernels_path,
                        &out_dir,
                    )
                    .map_err(|e| format!("embedded-pattern (stm32h7 bin) codegen error: {e}"))?;
                    println!("nucleus: ok");
                    println!("project_dir  = {}", result.project_dir.display());
                    println!("cargo_toml   = {}", result.cargo_toml.display());
                    println!("main_rs      = {}", result.main_rs.display());
                    println!("memory_x     = {}", result.memory_x.display());
                    println!("build_rs     = {}", result.build_rs.display());
                    println!("cargo_config = {}", result.cargo_config.display());
                    Ok(())
                }
                Some(other) => Err(format!(
                    "unknown --shim `{other}` for backend embedded-pattern; \
                     registered shims: `stm32h7` (Renode-runnable no_std bin, \
                     M10 TASK-0048.01). Omit --shim for the M9 compile-only lib."
                )),
            }
        }
        // FIRST tier-2 backend (TASK-0045, M7): SPMD MPI. ONE
        // rank-dispatched binary (MPI_Comm_rank), output is hosted Rust +
        // rsmpi. Single-worker SPMD arm landed cycle M7-entry (reuses the
        // shared single-worker renderer for byte-identical compute,
        // wrapped in MPI_Init/Finalize + a rank==0 guard); the
        // multi-worker arm (rank-dispatched Send/Recv + MPI_Barrier)
        // returns ContractGap forward-linking TASK-0045.01. The generated
        // project builds + runs only under `nix develop .#mpi`
        // (`just check-mpi`), NOT the tier-1 runtime differential.
        "mpi-blocking" => {
            let result =
                mpi_blocking::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
                    .map_err(|e| format!("mpi-blocking codegen error: {e}"))?;
            println!("nucleus: ok");
            println!("project_dir = {}", result.project_dir.display());
            println!("cargo_toml  = {}", result.cargo_toml.display());
            println!("main_rs     = {}", result.main_rs.display());
            if let Some(compute_rs) = &result.compute_rs {
                // Single-worker arm only; the multi-worker arm (TASK-0045.01)
                // emits the whole rank-dispatched program in main.rs.
                println!("compute_rs  = {}", compute_rs.display());
            }
            println!("kernels_rs  = {}", result.kernels_rs.display());
            println!("run_sh      = {}", result.run_sh.display());
            Ok(())
        }
        other => Err(format!(
            "unknown backend `{other}`; registered: `pthreads-sync`, \
             `mp-tcp-bufsync`, `pthreads-async`, `mp-tcp-event`, \
             `openmp-rs`, `mp-tcp-poll`, `mp-uds-event`, `embedded-pattern`, \
             `mpi-blocking`"
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
