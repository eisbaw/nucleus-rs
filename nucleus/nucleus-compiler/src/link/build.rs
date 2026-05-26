//! Link-step entry point: [`link`] resolves schedule references
//! against the algorithm and emits a [`LinkedIR`]. Collects all
//! errors before returning — never fails fast (PRD §12).

use std::collections::BTreeMap;

use super::dataflow::{analyse_dataflow, collect_loop_vars};
use super::errors::{LinkError, LinkErrorKind, LinkErrorSource};
use super::pipeline::check_pipeline_buffer_constraints;
use super::types::{LinkedIR, WorkerEntity};
use crate::algo::AlgoIR;
use crate::sched::{ResolvedPlacement, SchedIR};

/// Resolve schedule references against the algorithm and emit a
/// [`LinkedIR`]. Collects all errors before returning — never fails
/// fast (PRD §12).
///
/// Order of checks within the single pass:
/// 1. `place` directives: name a kernel that doesn't exist (UnknownKernel).
/// 2. `place_data` directives: name a data symbol that doesn't exist
///    (UnknownData).
/// 3. `transfer` directives: name a data symbol that doesn't exist
///    (UnknownTransferData).
/// 4. `loop` and `check loop` directives: name a loop variable not in
///    the algorithm (UnknownLoop).
/// 5. Coverage: every algorithm kernel has a `place` (UnplacedKernel).
/// 6. Cross-worker transfers: every data symbol whose producer and
///    consumer differ has a transfer directive
///    (MissingCrossWorkerTransfer).
///
/// Steps 1–5 can each report many errors. Step 6 depends on the
/// per-kernel placement having a meaningful worker entity, so we only
/// run cross-worker analysis on kernels whose placement IS in the
/// schedule (skipping kernels reported as UnplacedKernel). This is
/// the deliberate "best-effort downstream check" choice: if you
/// haven't placed a kernel, we can't infer where it produces or
/// consumes, so we don't make up workers to chase a follow-on error.
pub fn link(algo: AlgoIR, sched: SchedIR) -> Result<LinkedIR, Vec<LinkError>> {
    let mut errors: Vec<LinkError> = Vec::new();

    // --- 1, 2, 3: name resolution against algorithm declarations ---

    for placement in sched.places.values() {
        if !algo.kernels.contains_key(&placement.kernel) {
            // TASK-0099: span comes from the offending `place K on ...`
            // schedule kernel token (`PlaceDirective.kernel.span` ->
            // ResolvedPlacement.kernel_span). `maybe_at` keeps the
            // hand-built-test path (kernel_span: None) producing an
            // honest position-less error rather than a fabricated 0:0.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownKernel {
                    name: placement.kernel.clone(),
                    suggestion: crate::error::suggest(
                        &placement.kernel,
                        algo.kernels.keys().map(String::as_str),
                    ),
                },
                placement.kernel_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    for pd in sched.place_data.values() {
        if !algo.data.contains_key(&pd.data) {
            // TASK-0099: span from `place_data D in R` data token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownData {
                    name: pd.data.clone(),
                    suggestion: crate::error::suggest(
                        &pd.data,
                        algo.data.keys().map(String::as_str),
                    ),
                },
                pd.data_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    for tx in sched.transfers.values() {
        if !algo.data.contains_key(&tx.data) {
            // TASK-0099: span from `transfer D : ...` data token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownTransferData {
                    name: tx.data.clone(),
                    suggestion: crate::error::suggest(
                        &tx.data,
                        algo.data.keys().map(String::as_str),
                    ),
                },
                tx.data_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    // --- 4: loop variable resolution ---

    let loop_vars = collect_loop_vars(&algo);
    for loop_dir in sched.loops.values() {
        if !loop_vars.contains(&loop_dir.var) {
            // TASK-0099: span from `loop V : ...` loop-var token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownLoop {
                    name: loop_dir.var.clone(),
                    suggestion: crate::error::suggest(
                        &loop_dir.var,
                        loop_vars.iter().map(String::as_str),
                    ),
                },
                loop_dir.var_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }
    for check in sched.checks.values() {
        if !loop_vars.contains(&check.var) {
            // Same diagnostic — the `check loop VAR` and `loop VAR`
            // both name the same algorithm-side variable.
            // TASK-0099: span from `check loop V : ...` loop-var token.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnknownLoop {
                    name: check.var.clone(),
                    suggestion: crate::error::suggest(
                        &check.var,
                        loop_vars.iter().map(String::as_str),
                    ),
                },
                check.var_span.clone(),
                LinkErrorSource::Schedule,
            ));
        }
    }

    // --- 5: coverage — every kernel has a place ---

    for (kernel_name, kernel_decl) in algo.kernels.iter() {
        if !sched.places.contains_key(kernel_name) {
            // TASK-0099: span comes from the algorithm-side
            // `kernel K : ...` decl identifier — this is the only
            // located link variant whose span is into the ALGORITHM
            // source (not the schedule). The `LinkErrorSource::Algorithm`
            // tag tells `display_with_src` to resolve against the
            // algorithm source string at render time.
            errors.push(LinkError::maybe_at(
                LinkErrorKind::UnplacedKernel(kernel_name.clone()),
                kernel_decl.name_span.clone(),
                LinkErrorSource::Algorithm,
            ));
        }
    }

    // --- Build the convenience maps and producer/consumer index ---
    //
    // Even when there are errors above, we still build placements/
    // workers for the kernels we CAN resolve — the cross-worker check
    // below uses them. Kernels with no placement are simply skipped.

    let placements: BTreeMap<String, ResolvedPlacement> = sched
        .places
        .iter()
        .filter(|(k, _)| algo.kernels.contains_key(*k))
        .map(|(k, p)| (k.clone(), p.clone()))
        .collect();

    let kernel_workers: BTreeMap<String, WorkerEntity> = placements
        .iter()
        .map(|(k, p)| (k.clone(), WorkerEntity::from_target(&p.target)))
        .collect();

    let (data_producers, data_consumers) = analyse_dataflow(&algo, &kernel_workers);

    // --- 6: cross-worker transfer existence ---

    for (data, producer) in data_producers.iter() {
        if let Some(consumers) = data_consumers.get(data) {
            for consumer in consumers.iter() {
                if consumer != producer && !sched.transfers.contains_key(data) {
                    // TASK-0099: `MissingCrossWorkerTransfer` is the
                    // sole genuinely position-less variant. The error
                    // joins algorithm dataflow + schedule placements +
                    // the *absence* of a transfer directive; there is
                    // no single offending source token (the actionable
                    // fix is "add a transfer directive", not "fix this
                    // token"). A documented missing position is
                    // honest; a fabricated one is not. See type docs.
                    errors.push(LinkError::new(LinkErrorKind::MissingCrossWorkerTransfer {
                        data: data.clone(),
                        producer_worker: producer.display(),
                        consumer_worker: consumer.display(),
                    }));
                }
            }
        }
    }

    // --- 7: pipeline depth vs buffer capacity (TASK-0134) ---
    //
    // For each `loop V : pipeline=D` directive, find every data
    // symbol referenced inside `for V : ...`'s body. For each such
    // symbol that has a `transfer DATA : buffer=N` directive, assert
    // `D <= N`. Otherwise the buffer place's initial_marking would
    // exceed its capacity and `acfg_to_petri` would emit a Petri net
    // the boundedness pass rejects on the spot — moving the
    // diagnostic earlier preserves the {loop_var, data, depth,
    // buffer} naming.
    //
    // We iterate `sched.loops` in BTreeMap order and emit at most one
    // PipelineExceedsBuffer per (loop_var, data) pair, in source-stable
    // order — same determinism contract as every other check above.
    check_pipeline_buffer_constraints(&algo, &sched, &mut errors);

    // Deduplicate identical errors. Possible because two consumers on
    // the same different-entity could each emit the same
    // "MissingCrossWorkerTransfer" if the loop above visited them as
    // separate entries (it won't, BTreeSet collapses; but defensive).
    // Also catches the degenerate "report each kind once" pattern.
    //
    // TASK-0099: sort by `e.kind` debug-format, NOT by `e` debug-format.
    // After the LinkError{kind,span,source} restructure, deriving Debug
    // on the wrapper folds span+source into `{e:?}`, which (a) would
    // sort two same-kind errors at different offsets into different
    // sort buckets (non-determinism leaking via the byte-offset jitter
    // a future test refactor could introduce), and (b) would break
    // `errors.dedup()`: dedup uses our hand-written `PartialEq` (kind
    // only — span+source EXCLUDED), so two same-kind errors are equal
    // by `==` but would be non-adjacent under the wrapper-debug sort,
    // letting the dup survive. Sorting by `.kind` Debug only keeps the
    // pre-TASK-0099 invariant: same-kind errors are adjacent, dedup
    // collapses them.
    errors.sort_by_key(|e| format!("{:?}", e.kind));
    errors.dedup();

    if errors.is_empty() {
        Ok(LinkedIR {
            algo,
            sched,
            placements,
            kernel_workers,
            data_producers,
            data_consumers,
        })
    } else {
        Err(errors)
    }
}
