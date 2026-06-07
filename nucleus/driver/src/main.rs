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
    acfg_to_events, acfg_to_net, analyze_net_soundness_symbolic, apply_host_data_relay_inject,
    apply_host_mediation_inject, apply_safe_push_reorder, build_sidecar, check_kernels_contract,
    check_net_sound, check_schedule_compat, inject_check_frames, link, load_capabilities,
    run_pre_mediation_passes, PreMediationError, SymbolicSoundness,
};

// Driver sub-modules (TASK-0388: carved out of this file to hold it
// below the 1000-LoC `check-mega-files` fence). `args` = CLI parsing;
// `dispatch` = per-backend `emit(...)` routing.
mod args;
mod dispatch;
mod gate;

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
            args::print_help();
            ExitCode::SUCCESS
        }
        other => die(&format!("unknown subcommand `{other}`; try `--help`")),
    }
}

fn cmd_build(argv: &[String]) -> Result<(), String> {
    let a = args::parse_build_args(argv)?;
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

    // ---- Build ACFG + block transforms + partition + halo/reuse
    //      inference + inject syncs + inject transfers ----
    //
    // This whole backend-agnostic pre-mediation pass chain
    // (build_acfg -> block_transforms -> partition_{workers,rows,
    // blocks2d} -> halo-inference -> reuse-inference -> inject_syncs ->
    // inject_transfers) is single-sourced in
    // `nucleus_compiler::run_pre_mediation_passes` (TASK-0422.01.01.01).
    // The driver and the `test_support` corpus-sweep helper both
    // delegate to it, so the chain cannot drift between test and
    // production. The driver-specific policy is just the two things the
    // shared fn deliberately leaves to its caller:
    //
    //  (1) ERROR MAPPING — each pass's typed error becomes a DISTINCT
    //      user-facing string (these strings are USER-FACING and
    //      TEST-PINNED: `cli_reuse_strict.rs` asserts the
    //      `reuse-inference error:` prefix; `task0371_partition_insufficient_work_reject.rs` asserts
    //      the `partition-workers error:` prefix + variant substrings — see
    //      the per-arm note below). Block-transform runs *between* ACFG
    //      construction and the sync/transfer injection passes
    //      (TASK-0030): for schedules with no `block=` directives
    //      (examples 01-03 at M2) it is a pure identity and the
    //      downstream ACFG is bit-identical.
    //
    //      Halo-inference DRIVER POLICY (TASK-0275): (B) PARTITION-
    //      POLICY-AWARE. A typed `HaloInferenceError` is FATAL only when
    //      its affected iv carries a `partition=` directive in scope
    //      (the `transfer_inject` consumer would otherwise silently emit
    //      wrong-output tiles — missing halo strips on a partition
    //      boundary). The non-partition-scoped errors come back in the
    //      advisory bucket (handled in (2)) and lowering proceeds; the
    //      walker's partial halo widths for unaffected kernels stay
    //      committed. Chosen over (A) strict because the halo consumer
    //      is CONDITIONAL on the iv being partitioned — a naive (A)
    //      mirror would newly-reject example 11 (`step_or_seed` reads
    //      `grid[(t + ITERS) % (ITERS + 1)]`, a constant Mod wrap the
    //      affine detector cannot fold, with ZERO `partition=` in the
    //      schedule). The advisory/fatal split lives inside
    //      `apply_halo_inference_partition_aware`.
    //
    //      Reuse-inference DRIVER POLICY (TASK-0271): STRICT — any typed
    //      `ReuseInferenceError` is fatal. The cycle-87 Tier 1 landing
    //      wired a walker-side marker consumer
    //      (`render_reuse_marker_comment`) so EVERY recognised reuse
    //      slot is consumed today; a silently-swallowed non-affine
    //      `loop V : reuse;` would produce no marker line (and tomorrow
    //      no buffer code) without warning. The asymmetry with halo's
    //      (B) is intentional: reuse's marker fires on every recognised
    //      slot, so (B)'s "is this slot consumed?" predicate is
    //      trivially true and degenerates into (A). E2E at both
    //      promotions: 92/77/0/15/0 byte-identical.
    //
    //  (2) HALO ADVISORY — the shared fn threads the non-fatal
    //      halo-advisory bucket out; the driver `nuc_trace!`s each entry
    //      (the test helper discards them).
    let (acfg, halo_errors_advisory) =
        run_pre_mediation_passes(&linked).map_err(|e| match e {
            PreMediationError::AcfgBuild(e) => format!("acfg build error: {e}"),
            PreMediationError::BlockTransform(e) => format!("block-transform error: {e}"),
            PreMediationError::PartitionWorkers(e) => format!("partition-workers error: {e}"),
            PreMediationError::PartitionRows(e) => format!("partition-rows error: {e}"),
            PreMediationError::PartitionBlocks2d(e) => format!("partition-blocks2d error: {e}"),
            PreMediationError::HaloInference(e) => {
                format!("halo-inference error (under partitioned iv): {e}")
            }
            PreMediationError::ReuseInference(e) => format!("reuse-inference error: {e}"),
            PreMediationError::SyncInject(e) => format!("sync-injection error: {e}"),
            PreMediationError::TransferInject(e) => format!("transfer-injection error: {e}"),
        })?;
    for e in &halo_errors_advisory {
        nucleus_compiler::nuc_trace!(
            "halo_inference: advisory (no `partition=` directive in scope, transfer_inject \
             halo consumer will not fire on the affected iv — lowering proceeds): {e}"
        );
    }

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
    // Symbolic fast path (TASK-0453.04, rigour epic P4). For the
    // buffer-free subclass (single-worker / no-cross-worker-transfer
    // programs — including the matmul triple loop whose expanded net is
    // ~2 nodes per kernel firing) the net is provably bounded,
    // deadlock-free and conflict-free directly from the ROLLED ACFG, in
    // time independent of the iteration counts. Proving it that way
    // avoids building (and replaying) the linear-in-firings expanded net
    // entirely. This is a fast path, never a weakening: every net the
    // expanded gate could reject (capacity overflow / stall / free-choice
    // conflict) requires a buffer place, hence an Xfer, hence is
    // classified `NeedsExpansion` and routed to the unchanged expanded
    // gate below. See `passes::net_soundness_symbolic` for the theorem +
    // soundness-equivalence argument.
    match analyze_net_soundness_symbolic(&acfg) {
        SymbolicSoundness::ProvenSound => {
            nucleus_compiler::nuc_trace!(
                "soundness gate: net is buffer-free; proved bounded/deadlock-free/conflict-free \
                 symbolically from the rolled ACFG without expanding it over the iteration space \
                 (TASK-0453.04)"
            );
        }
        SymbolicSoundness::NeedsExpansion(reason) => {
            nucleus_compiler::nuc_trace!(
                "soundness gate: {reason}; running the expanded single-order replay gate"
            );
            let gate_net = acfg_to_net(&acfg);
            check_net_sound(&gate_net)
                .map_err(|e| format!("petri-net soundness check failed: {e}"))?;
        }
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

    // ---- Post-projection EventList gates (TASK-0422.02, cycle-245):
    //      overlapping-write accumulator cross-check (TASK-0343.03) +
    //      full PRD §8.3 event-contract validation (TASK-0422/0423). ----
    //
    // Factored into `gate::gate_per_worker_for_dispatch` so the reject
    // arm is unit-testable (it is undriveable from any real `.nuc`
    // source — the corpus is contract-clean). The order (accumulator
    // THEN validate), the error strings, and the `?`-propagation are
    // byte-preserved by the extraction; see that fn's docstring for the
    // full rationale. This call line is now COMPILE-TIME load-bearing
    // (TASK-0440): it yields the `GatedPerWorker` witness that
    // `dispatch_backend` requires, so deleting/bypassing the gate makes
    // the driver fail to BUILD — a strictly stronger guarantee than the
    // earlier "one visible call site" structural argument. ONE site, all
    // 7 backends.
    let gated =
        gate::gate_per_worker_for_dispatch(&linked.algo, &per_worker, &sidecar, &names.data)?;

    dispatch::dispatch_backend(
        &backend,
        gated,
        &names,
        &sidecar,
        &kernels_path,
        &out_dir,
        shim.as_deref(),
    )
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
