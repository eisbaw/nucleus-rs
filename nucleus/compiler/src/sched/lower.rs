//! AST → SchedIR lowering pass.
//!
//! See [`super::ir`] for the IR types and the [`SchedLowerError`]
//! variants. This pass mirrors the algorithm side
//! ([`crate::algo::lower::lower_algo`]) in shape:
//!
//! 1. Walk directives in source order. Pass 1 collects declarations
//!    (worker classes, memory regions, workers).
//! 2. Synthesise the default worker class if any simple-form worker
//!    is present.
//! 3. Pass 2 walks the remaining directives and lowers them, checking
//!    references against the symbol tables built in pass 1.
//!
//! A single-pass walk is tempting but breaks the validation rule for
//! `accessible_by` (which references classes that may be declared
//! later in source) and for `place X on W` (where `workers` may
//! appear after `place`). The grammar accepts any directive order
//! (§2 note 2); separating declaration collection from reference
//! validation is the cleanest way to honour that.

use std::collections::BTreeMap;

use super::ast::{CheckAssert, Directive, LoopOption, PlaceTarget, SchedAst, TransferOption};
use super::ir::{
    ResolvedCheckAssert, ResolvedCheckDirective, ResolvedLoopDirective, ResolvedLoopOption,
    ResolvedMemoryRegion, ResolvedPlaceData, ResolvedPlaceTarget, ResolvedPlacement,
    ResolvedTransferDirective, ResolvedTransferOption, ResolvedWorker, ResolvedWorkerClass,
    SchedIR, SchedLowerError, SchedLowerErrorKind, DEFAULT_WORKER_CLASS,
};

/// Lower a parsed [`SchedAst`] into a validated [`SchedIR`].
///
/// Returns the first violation encountered. Multi-error reporting
/// follows the algorithm-side precedent (single-error only — see
/// the algorithm lowering's known limitation).
pub fn lower_sched(ast: &SchedAst) -> Result<SchedIR, SchedLowerError> {
    let mut ir = SchedIR {
        algo_path: ast.algo_path.clone(),
        ..SchedIR::default()
    };

    // ----------------------------------------------------------------
    // Pass 1: collect declarations (classes, regions, workers).
    //
    // Done as a dedicated pass so that the order-insensitive grammar
    // (§2 note 2) doesn't force callers to declare classes before
    // `workers = ...` references them. The negative tests still pin
    // failures to the right kind via [`SchedLowerError`].
    // ----------------------------------------------------------------

    // Track whether any worker entry used the simple (class-less)
    // form. We need this to know whether to inject the synthetic
    // default class into `worker_classes`.
    let mut needs_default_class = false;
    // Track whether we have already seen a `workers = ...` directive.
    // The grammar nominally allows multiple; the IR rejects more than
    // one because semantics for "concatenate two workers sets" are
    // ambiguous (PRD §6.3.1 phrases the workers decl as singular).
    let mut workers_seen = false;

    // (b) TASK-0196 side-tables: the `UnknownWorkerClass` and
    // `UnknownAccessibleByName` checks must run AFTER pass 1 has
    // collected every class/worker AND the synthetic default class is
    // injected (a name may refer to a class declared later in source,
    // or to the simple-form default). The old code iterated the
    // post-strip `ir.workers` / `ir.memory_regions` (plain `String`,
    // no span). To locate these diagnostics WITHOUT putting a span
    // into the codegen-feeding SchedIR (decision: option (b) in
    // TASK-0196 — keep SchedIR span-free, mirror the algo IR), we
    // record the offending reference together with its source span
    // here, at the AST-walk site where the `Spanned` is in scope, and
    // validate from these tables once collection is complete. Order of
    // first error is preserved: `worker_class_refs` is in worker-entry
    // source order; `accessible_by_refs` is in directive then list
    // order — and the post-collection validation iterates the same
    // deterministic structures the old code did.
    let mut worker_class_refs: Vec<(String, String, core::ops::Range<usize>)> = Vec::new();
    // (region_name, accessible_by_name, span) per `accessible_by` entry.
    let mut accessible_by_refs: Vec<(String, String, core::ops::Range<usize>)> = Vec::new();

    // NOTE (TASK-0086 + TASK-0196): the AST is span-carrying —
    // directives are `SpDirective` and identifier fields are `SpName`.
    // The IR stays plain-`String` (SchedIR feeds codegen and must not
    // change shape — determinism). The *value* of every check still
    // comes from `.node`; what TASK-0196 adds is reading the offending
    // node's `.span` (byte `Range`) at the err site and threading it
    // into a located `SchedLowerError` via `SchedLowerError::at`. Spans
    // populate ONLY on the `Err` path, so valid schedules lower
    // byte-identically (proven by the determinism gate). `match
    // &d.node` (not `match d`) because `Deref` does not apply to
    // `match`, so the span cannot leak into a value position by
    // accident.
    for d in &ast.directives {
        match &d.node {
            Directive::WorkerClass(c) => {
                if ir.worker_classes.contains_key(&c.name.node) {
                    // Located at the duplicate decl's identifier token.
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicateWorkerClass(c.name.node.clone()),
                        c.name.span.clone(),
                    ));
                }
                ir.worker_classes.insert(
                    c.name.node.clone(),
                    ResolvedWorkerClass {
                        name: c.name.node.clone(),
                        simd: c.simd.clone(),
                        memory: c.memory.clone(),
                        is_default: false,
                    },
                );
            }
            Directive::MemoryRegion(r) => {
                if ir.memory_regions.contains_key(&r.name.node) {
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicateMemoryRegion(r.name.node.clone()),
                        r.name.span.clone(),
                    ));
                }
                // (b) TASK-0196: record each `accessible_by` name WITH
                // its source span before the span is stripped into the
                // plain-`String` IR, so the post-collection
                // `UnknownAccessibleByName` check can locate the
                // offending name token without the SchedIR carrying a
                // span. Recorded in (directive, list) source order.
                if let Some(names) = &r.accessible_by {
                    for n in names {
                        accessible_by_refs.push((
                            r.name.node.clone(),
                            n.node.clone(),
                            n.span.clone(),
                        ));
                    }
                }
                ir.memory_regions.insert(
                    r.name.node.clone(),
                    ResolvedMemoryRegion {
                        name: r.name.node.clone(),
                        size_bytes: r.size_bytes,
                        // `accessible_by` is `Vec<SpName>`; the IR keeps
                        // plain `Vec<String>`. Strip spans here — the
                        // located check uses `accessible_by_refs`
                        // (TASK-0196 option (b): SchedIR stays
                        // span-free, consistent with the algo IR).
                        accessible_by: r
                            .accessible_by
                            .as_ref()
                            .map(|names| names.iter().map(|n| n.node.clone()).collect()),
                        per_worker: r.per_worker,
                    },
                );
            }
            Directive::Workers(w) => {
                if workers_seen {
                    // The whole second `workers = ...` directive is the
                    // offending node; point at the directive span (the
                    // `SpDirective` wrapper `d`, TASK-0086).
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicateWorkersDecl,
                        d.span.clone(),
                    ));
                }
                workers_seen = true;

                // Track per-decl duplicates separately so the error
                // message for `{ host, host }` points at the literal
                // collision, not at a phantom cross-decl one.
                let mut seen_in_this_decl: BTreeMap<String, ()> = BTreeMap::new();

                for entry in &w.entries {
                    if seen_in_this_decl
                        .insert(entry.name.node.clone(), ())
                        .is_some()
                    {
                        // Per-decl duplicate (`{ host, host }`): point
                        // at the offending entry's identifier token.
                        return Err(SchedLowerError::at(
                            SchedLowerErrorKind::DuplicateWorker(entry.name.node.clone()),
                            entry.name.span.clone(),
                        ));
                    }
                    if ir.workers.contains_key(&entry.name.node) {
                        // Cross-decl duplicate: same — point at this
                        // (later) entry's identifier.
                        return Err(SchedLowerError::at(
                            SchedLowerErrorKind::DuplicateWorker(entry.name.node.clone()),
                            entry.name.span.clone(),
                        ));
                    }

                    let class = match &entry.class {
                        Some(c) => c.node.clone(),
                        None => {
                            needs_default_class = true;
                            DEFAULT_WORKER_CLASS.to_string()
                        }
                    };
                    // (b) TASK-0196: record the class reference WITH its
                    // source span so the `UnknownWorkerClass` check can
                    // be done here-style (after all classes are
                    // collected + default injected) while still having
                    // the offending `Spanned` in scope — instead of
                    // iterating the post-strip span-free `ir.workers`.
                    // This keeps SchedIR span-free (consistent with the
                    // algo IR; no codegen-feeding shape change). The
                    // span of the unresolved class is `entry.class.span`
                    // when the class was written explicitly; for a
                    // simple-form entry the class is the synthetic
                    // default (which always resolves once injected, so
                    // it never reaches the unknown-class error) — we
                    // fall back to `entry.name.span` so the recorded
                    // tuple is always located.
                    let class_span = match &entry.class {
                        Some(c) => c.span.clone(),
                        None => entry.name.span.clone(),
                    };
                    worker_class_refs.push((
                        entry.name.node.clone(),
                        class.clone(),
                        class_span,
                    ));
                    ir.workers.insert(
                        entry.name.node.clone(),
                        ResolvedWorker {
                            name: entry.name.node.clone(),
                            class,
                        },
                    );
                }
            }
            // Other directive kinds are deferred to pass 2.
            _ => {}
        }
    }

    if !workers_seen {
        // Genuinely position-less (TASK-0196): the error is the
        // *absence* of a `workers = ...` directive — there is no source
        // token to underline. `span: None`, documented on
        // `SchedLowerError` and pinned by
        // `position_less_variants_have_no_span`.
        return Err(SchedLowerError::new(SchedLowerErrorKind::MissingWorkersDecl));
    }

    // Synthesise the default class only after we know whether any
    // simple-form entry exists. Collision with a user-declared class
    // of the same name is caught by `DuplicateWorkerClass` — we keep
    // the same loud-failure principle as the algorithm lowering.
    if needs_default_class {
        if ir.worker_classes.contains_key(DEFAULT_WORKER_CLASS) {
            // A user declared a class with the synthetic name. That's
            // a name collision we must surface; otherwise simple-form
            // entries would silently inherit the user's class.
            //
            // Genuinely position-less (TASK-0196): the collision is
            // between a real user `worker_class __default` decl and the
            // *synthesised* default class, which has no source token.
            // This branch iterates the post-collected class table and
            // does not have the user decl's `Spanned` in scope, so
            // `span: None` — documented on `SchedLowerError` and pinned
            // by `position_less_variants_have_no_span`. (The common
            // `DuplicateWorkerClass` from two real decls IS located —
            // see the pass-1 `WorkerClass` arm.) Re-deriving the user
            // decl's span here is possible but would duplicate the
            // pass-1 walk for a pathological corner; the honest `None`
            // is the documented behaviour, not a TODO.
            return Err(SchedLowerError::new(
                SchedLowerErrorKind::DuplicateWorkerClass(DEFAULT_WORKER_CLASS.to_string()),
            ));
        }
        ir.worker_classes.insert(
            DEFAULT_WORKER_CLASS.to_string(),
            ResolvedWorkerClass {
                name: DEFAULT_WORKER_CLASS.to_string(),
                simd: None,
                memory: None,
                is_default: true,
            },
        );
    }

    // Validate every worker's class reference now that all classes
    // are collected.
    //
    // (b) TASK-0196: validated from `worker_class_refs` (collected
    // during the pass-1 worker walk, where the class `Spanned` is in
    // scope) instead of iterating the post-strip span-free
    // `ir.workers`. This is what makes `UnknownWorkerClass` located
    // while keeping SchedIR span-free (consistent with the algo IR; no
    // codegen-feeding shape change). First-error ordering is preserved
    // bit-for-bit: the old code iterated `ir.workers.values()`, i.e.
    // the `BTreeMap` in worker-name sorted order, so we sort
    // `worker_class_refs` by worker name before validating — for any
    // input the SAME worker is reported first as before.
    worker_class_refs.sort_by(|a, b| a.0.cmp(&b.0));
    for (worker, class, span) in &worker_class_refs {
        if !ir.worker_classes.contains_key(class) {
            return Err(SchedLowerError::at(
                SchedLowerErrorKind::UnknownWorkerClass {
                    worker: worker.clone(),
                    class: class.clone(),
                    // TASK-0198: closest declared `worker_class`.
                    // `ir.worker_classes` is a `BTreeMap`, so its
                    // key iteration is deterministic; `suggest`
                    // additionally sorts — no hash-order in the path.
                    suggestion: crate::error::suggest(
                        class,
                        ir.worker_classes.keys().map(String::as_str),
                    ),
                },
                span.clone(),
            ));
        }
    }

    // TASK-0095: validate `memory_region R { accessible_by = { ... } }`.
    // Grammar §2 note 4 phrases name resolution as "the linker's job",
    // but for `accessible_by` every legal target — a `worker_class`
    // or a worker name — is declared in this same schedule, so the
    // resolution is purely schedule-internal and is done here in
    // lowering (not deferred to the linker). Scope: an "undeclared
    // name" typed error, now carrying a deterministic did-you-mean
    // suggestion (TASK-0198, the schedule sibling of the link-step
    // TASK-0096) computed against the union of declared `worker_class`
    // and worker names — see the `UnknownAccessibleByName` arm below.
    // Runs after the synthetic default class is injected and all
    // workers are collected, so a name referring to a simple-form
    // worker or the default class resolves correctly.
    //
    // (b) TASK-0196: validated from `accessible_by_refs` (collected at
    // the pass-1 `MemoryRegion` walk, where each name's `Spanned` is in
    // scope) instead of iterating the post-strip span-free
    // `ir.memory_regions`. Makes `UnknownAccessibleByName` located
    // while keeping SchedIR span-free. First-error ordering preserved
    // bit-for-bit: the old code iterated `ir.memory_regions.values()`
    // (BTreeMap, region-name sorted) then each region's `accessible_by`
    // in list order; region names are unique (dups rejected above), so
    // a STABLE sort by region name reproduces exactly that order
    // (recording was in directive-then-list order, so list order is
    // intact within a region).
    accessible_by_refs.sort_by(|a, b| a.0.cmp(&b.0));
    for (region, name, span) in &accessible_by_refs {
        let declared =
            ir.worker_classes.contains_key(name) || ir.workers.contains_key(name);
        if !declared {
            return Err(SchedLowerError::at(
                SchedLowerErrorKind::UnknownAccessibleByName {
                    region: region.clone(),
                    name: name.clone(),
                    // TASK-0198: candidate set is the DETERMINISTIC
                    // union of declared `worker_class` names and
                    // worker names — exactly this variant's own
                    // validity rule (a name is legal iff it is a
                    // declared class OR a declared worker). Both
                    // `ir.worker_classes` and `ir.workers` are
                    // `BTreeMap`s, so chaining their key iterators is
                    // deterministic; `suggest` sorts the merged set
                    // again — no `HashMap`/`HashSet` anywhere in the
                    // selection path (reproducibility gate).
                    suggestion: crate::error::suggest(
                        name,
                        ir.worker_classes
                            .keys()
                            .chain(ir.workers.keys())
                            .map(String::as_str),
                    ),
                },
                span.clone(),
            ));
        }
    }

    // ----------------------------------------------------------------
    // Pass 2: lower place / place_data / loop / transfer / check.
    // ----------------------------------------------------------------

    for d in &ast.directives {
        match &d.node {
            // Already handled in pass 1.
            Directive::WorkerClass(_) | Directive::MemoryRegion(_) | Directive::Workers(_) => {}

            Directive::Place(p) => lower_place(p, &mut ir)?,
            Directive::PlaceData(pd) => lower_place_data(pd, &mut ir)?,
            Directive::Loop(l) => lower_loop(l, &mut ir)?,
            Directive::Transfer(t) => lower_transfer(t, &mut ir)?,
            Directive::Check(c) => lower_check(c, &mut ir)?,
        }
    }

    Ok(ir)
}

// --------------------------------------------------------------------
// Per-directive lowering
// --------------------------------------------------------------------

fn lower_place(p: &super::ast::PlaceDirective, ir: &mut SchedIR) -> Result<(), SchedLowerError> {
    // TASK-0196: the *value* of each check comes from `.node`; the
    // located error reads the offending node's `.span`.
    let kernel = &p.kernel.node;
    if ir.places.contains_key(kernel) {
        // Duplicate `place` for this kernel: point at this directive's
        // kernel identifier token.
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicatePlace {
                kernel: kernel.clone(),
            },
            p.kernel.span.clone(),
        ));
    }
    let target = match &p.target {
        PlaceTarget::One(w) => {
            // Undeclared worker -> point at the worker token (`w.span`).
            check_worker_declared(kernel, w, ir)?;
            ResolvedPlaceTarget::One(w.node.clone())
        }
        PlaceTarget::Many(ws) => {
            // TASK-0094: a worker named twice in one placement set
            // (`place k on { w0, w0 }`) is rejected as a hard error,
            // NOT silently folded to a unique set. Rationale: a
            // repeated worker in a distributed placement is a user
            // mistake; a silent fold would change the placement the
            // user wrote without telling them (fail-fast;
            // decision-0003 — user-diagnosable input -> typed Result).
            // Checked before the undeclared-worker check so the
            // duplicate gets its specific message even when the
            // repeated name is also undeclared.
            let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
            for w in ws {
                // `w.as_str()` via `Deref` (Spanned<String> -> str).
                if seen.insert(w.as_str(), ()).is_some() {
                    // Point at the *repeated* occurrence's token.
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicatePlaceWorker {
                            kernel: kernel.clone(),
                            worker: w.node.clone(),
                        },
                        w.span.clone(),
                    ));
                }
            }
            for w in ws {
                check_worker_declared(kernel, w, ir)?;
            }
            ResolvedPlaceTarget::Many(ws.iter().map(|w| w.node.clone()).collect())
        }
    };
    ir.places.insert(
        kernel.clone(),
        ResolvedPlacement {
            kernel: kernel.clone(),
            target,
        },
    );
    Ok(())
}

fn check_worker_declared(
    kernel: &str,
    worker: &super::ast::SpName,
    ir: &SchedIR,
) -> Result<(), SchedLowerError> {
    // TASK-0196: takes the worker `SpName` (not `&str`) so the
    // undeclared-worker diagnostic points at the worker token
    // (`worker.span`). The semantic check is still on `worker.node`.
    if !ir.workers.contains_key(&worker.node) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::UnknownPlaceWorker {
                kernel: kernel.to_string(),
                worker: worker.node.clone(),
                // TASK-0198: closest declared worker name.
                // `ir.workers` is a `BTreeMap` → deterministic key
                // order; `suggest` also sorts — no hash-order.
                suggestion: crate::error::suggest(
                    &worker.node,
                    ir.workers.keys().map(String::as_str),
                ),
            },
            worker.span.clone(),
        ));
    }
    Ok(())
}

fn lower_place_data(
    pd: &super::ast::PlaceDataDirective,
    ir: &mut SchedIR,
) -> Result<(), SchedLowerError> {
    // TASK-0196: value from `.node`, location from the offending
    // node's `.span`.
    let data = &pd.data.node;
    let region = &pd.region.node;
    if ir.place_data.contains_key(data) {
        // Duplicate `place_data` for this data symbol: point at the
        // data token.
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicatePlaceData {
                data: data.clone(),
            },
            pd.data.span.clone(),
        ));
    }
    if !ir.memory_regions.contains_key(region) {
        // The undeclared thing is the *region* — point at the region
        // token.
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::UnknownMemoryRegion {
                data: data.clone(),
                region: region.clone(),
                // TASK-0198: closest declared `memory_region` name.
                // `ir.memory_regions` is a `BTreeMap` → deterministic
                // key order; `suggest` also sorts — no hash-order.
                suggestion: crate::error::suggest(
                    region,
                    ir.memory_regions.keys().map(String::as_str),
                ),
            },
            pd.region.span.clone(),
        ));
    }
    ir.place_data.insert(
        data.clone(),
        ResolvedPlaceData {
            data: data.clone(),
            region: region.clone(),
        },
    );
    Ok(())
}

fn lower_loop(l: &super::ast::LoopDirective, ir: &mut SchedIR) -> Result<(), SchedLowerError> {
    // TASK-0196: value from `.node`. Option-level errors
    // (`DuplicateLoopOption`, `ZeroLoopOption`) are located at the
    // owning loop variable's token (`l.var.span`): the option enums
    // are deliberately NOT span-wrapped by TASK-0086 (its documented
    // granularity — see `crate::span`), so the loop variable is the
    // finest located node available. Widening to option-level spans
    // would require extending TASK-0086 scope first; not done here.
    let var = &l.var.node;
    let var_span = &l.var.span;
    if ir.loops.contains_key(var) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicateLoop { var: var.clone() },
            var_span.clone(),
        ));
    }
    // TASK-0093: the grammar treats the option list as an unordered
    // *set* (§2 note 7 / §5.1: `block=64, block=128` is a semantic
    // conflict rejected post-parse). Each value-bearing keyword may
    // appear at most once. `reuse` is a bare idempotent flag — note 7
    // only calls out *value* conflicts, so a repeated `reuse` is
    // harmless redundancy, not an error; we deliberately do not flag
    // it (interpretation recorded: note 7 is silent on bare-flag
    // repetition, and folding an idempotent flag is not the
    // value-ambiguity the note targets).
    {
        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        for opt in &l.options {
            if let Some(kw) = loop_option_keyword(opt) {
                if seen.insert(kw, ()).is_some() {
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicateLoopOption {
                            var: var.clone(),
                            option: kw.to_string(),
                        },
                        var_span.clone(),
                    ));
                }
            }
        }
    }
    let mut options = Vec::with_capacity(l.options.len());
    for opt in &l.options {
        options.push(lower_loop_option(var, var_span, opt)?);
    }
    ir.loops.insert(
        var.clone(),
        ResolvedLoopDirective {
            var: var.clone(),
            options,
        },
    );
    Ok(())
}

/// The keyword for a value-bearing loop option, for at-most-once
/// duplicate detection (TASK-0093). `reuse` returns `None`: it is a
/// bare idempotent flag and a repeated `reuse` is not the value
/// conflict grammar §2 note 7 targets.
fn loop_option_keyword(opt: &LoopOption) -> Option<&'static str> {
    match opt {
        LoopOption::Block(_) => Some("block"),
        LoopOption::Vectorize(_) => Some("vectorize"),
        LoopOption::Unroll(_) => Some("unroll"),
        LoopOption::Pipeline(_) => Some("pipeline"),
        LoopOption::Partition(_) => Some("partition"),
        LoopOption::Reuse => None,
    }
}

fn lower_loop_option(
    var: &str,
    var_span: &core::ops::Range<usize>,
    opt: &LoopOption,
) -> Result<ResolvedLoopOption, SchedLowerError> {
    // Numeric options share the same "strictly positive" rule. The
    // small helper keeps the variant list short and obvious.
    // TASK-0196: `ZeroLoopOption` is located at the loop variable's
    // token (`var_span`) — the option literal is not span-wrapped
    // (TASK-0086 granularity; see `lower_loop`).
    let positive = |n: u64, keyword: &str| -> Result<u64, SchedLowerError> {
        if n == 0 {
            Err(SchedLowerError::at(
                SchedLowerErrorKind::ZeroLoopOption {
                    var: var.to_string(),
                    option: keyword.to_string(),
                },
                var_span.clone(),
            ))
        } else {
            Ok(n)
        }
    };

    Ok(match opt {
        LoopOption::Block(n) => ResolvedLoopOption::Block(positive(*n, "block")?),
        LoopOption::Vectorize(n) => ResolvedLoopOption::Vectorize(positive(*n, "vectorize")?),
        LoopOption::Unroll(n) => ResolvedLoopOption::Unroll(positive(*n, "unroll")?),
        LoopOption::Pipeline(n) => ResolvedLoopOption::Pipeline(positive(*n, "pipeline")?),
        LoopOption::Reuse => ResolvedLoopOption::Reuse,
        LoopOption::Partition(k) => ResolvedLoopOption::Partition(*k),
    })
}

fn lower_transfer(
    t: &super::ast::TransferDirective,
    ir: &mut SchedIR,
) -> Result<(), SchedLowerError> {
    // TASK-0196: value from `.node`. Like loop options, transfer
    // option-level errors are located at the owning data symbol's
    // token (`t.data.span`) — the option enums are not span-wrapped
    // (TASK-0086 granularity; see `lower_loop`).
    let data = &t.data.node;
    let data_span = &t.data.span;
    if ir.transfers.contains_key(data) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicateTransfer {
                data: data.clone(),
            },
            data_span.clone(),
        ));
    }
    // TASK-0093: like loop options, the transfer option list is an
    // unordered set. Two rules from the grammar §2 notes:
    //  - note 5 / §5.3: `sync` and `async` are mutually exclusive in
    //    one `TransferStmt`. We also reject a repeated mode flag
    //    (`sync, sync`) under the same variant — it is the same user
    //    error class (a transfer has exactly one mode).
    //  - note 7 (same set semantics as loops): `buffer` and `notify`
    //    are value-bearing and may appear at most once.
    {
        let mut mode_flags = 0usize;
        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        for opt in &t.options {
            match opt {
                TransferOption::Sync | TransferOption::Async => {
                    mode_flags += 1;
                    if mode_flags > 1 {
                        return Err(SchedLowerError::at(
                            SchedLowerErrorKind::ConflictingTransferMode {
                                data: data.clone(),
                            },
                            data_span.clone(),
                        ));
                    }
                }
                TransferOption::Buffer(_) => {
                    if seen.insert("buffer", ()).is_some() {
                        return Err(SchedLowerError::at(
                            SchedLowerErrorKind::DuplicateTransferOption {
                                data: data.clone(),
                                option: "buffer".to_string(),
                            },
                            data_span.clone(),
                        ));
                    }
                }
                TransferOption::Notify(_) => {
                    if seen.insert("notify", ()).is_some() {
                        return Err(SchedLowerError::at(
                            SchedLowerErrorKind::DuplicateTransferOption {
                                data: data.clone(),
                                option: "notify".to_string(),
                            },
                            data_span.clone(),
                        ));
                    }
                }
            }
        }
    }
    let mut options = Vec::with_capacity(t.options.len());
    for opt in &t.options {
        options.push(lower_transfer_option(data, data_span, opt)?);
    }
    ir.transfers.insert(
        data.clone(),
        ResolvedTransferDirective {
            data: data.clone(),
            options,
        },
    );
    Ok(())
}

fn lower_transfer_option(
    data: &str,
    data_span: &core::ops::Range<usize>,
    opt: &TransferOption,
) -> Result<ResolvedTransferOption, SchedLowerError> {
    Ok(match opt {
        TransferOption::Sync => ResolvedTransferOption::Sync,
        TransferOption::Async => ResolvedTransferOption::Async,
        TransferOption::Buffer(n) => {
            if *n == 0 {
                // TASK-0196: located at the data symbol's token; the
                // `buffer=0` literal is not span-wrapped (TASK-0086).
                return Err(SchedLowerError::at(
                    SchedLowerErrorKind::ZeroBufferOption {
                        data: data.to_string(),
                    },
                    data_span.clone(),
                ));
            }
            ResolvedTransferOption::Buffer(*n)
        }
        TransferOption::Notify(k) => ResolvedTransferOption::Notify(*k),
    })
}

fn lower_check(c: &super::ast::CheckDirective, ir: &mut SchedIR) -> Result<(), SchedLowerError> {
    // TASK-0196: value from `.node`, location from `c.var.span`.
    let var = &c.var.node;
    if ir.checks.contains_key(var) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicateCheck { var: var.clone() },
            c.var.span.clone(),
        ));
    }
    let asserts = c
        .asserts
        .iter()
        .map(|a| match a {
            CheckAssert::LatencyMax(t) => ResolvedCheckAssert::LatencyMax(*t),
            CheckAssert::OnViolation(v) => ResolvedCheckAssert::OnViolation(*v),
        })
        .collect();
    ir.checks.insert(
        var.clone(),
        ResolvedCheckDirective {
            var: var.clone(),
            asserts,
        },
    );
    Ok(())
}
