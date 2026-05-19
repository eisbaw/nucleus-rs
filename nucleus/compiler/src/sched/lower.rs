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
    SchedIR, SchedLowerError, DEFAULT_WORKER_CLASS,
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

    // NOTE (TASK-0086): the AST is now span-carrying — directives are
    // `SpDirective` and identifier fields are `SpName`. Lowering still
    // *ignores* spans entirely: every use below projects through
    // `.node` (or `&**`) to the inner value, the IR + `SchedLowerError`
    // keep their plain-`String` shapes, and behaviour is byte-identical
    // (proven by the determinism gate). Threading the `.span` into a
    // located `SchedLowerError` is the separate TASK-0196; this pass
    // must NOT do it. `match &d.node` (not `match d`) because `Deref`
    // does not apply to `match`, so a future span use cannot leak in
    // by accident.
    for d in &ast.directives {
        match &d.node {
            Directive::WorkerClass(c) => {
                if ir.worker_classes.contains_key(&c.name.node) {
                    return Err(SchedLowerError::DuplicateWorkerClass(c.name.node.clone()));
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
                    return Err(SchedLowerError::DuplicateMemoryRegion(r.name.node.clone()));
                }
                ir.memory_regions.insert(
                    r.name.node.clone(),
                    ResolvedMemoryRegion {
                        name: r.name.node.clone(),
                        size_bytes: r.size_bytes,
                        // `accessible_by` is `Vec<SpName>`; the IR keeps
                        // plain `Vec<String>`. Strip spans here (the
                        // located variant is TASK-0196's job).
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
                    return Err(SchedLowerError::DuplicateWorkersDecl);
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
                        return Err(SchedLowerError::DuplicateWorker(entry.name.node.clone()));
                    }
                    if ir.workers.contains_key(&entry.name.node) {
                        return Err(SchedLowerError::DuplicateWorker(entry.name.node.clone()));
                    }

                    let class = match &entry.class {
                        Some(c) => c.node.clone(),
                        None => {
                            needs_default_class = true;
                            DEFAULT_WORKER_CLASS.to_string()
                        }
                    };
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
        return Err(SchedLowerError::MissingWorkersDecl);
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
            return Err(SchedLowerError::DuplicateWorkerClass(
                DEFAULT_WORKER_CLASS.to_string(),
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
    for worker in ir.workers.values() {
        if !ir.worker_classes.contains_key(&worker.class) {
            return Err(SchedLowerError::UnknownWorkerClass {
                worker: worker.name.clone(),
                class: worker.class.clone(),
            });
        }
    }

    // TASK-0095: validate `memory_region R { accessible_by = { ... } }`.
    // Grammar §2 note 4 phrases name resolution as "the linker's job",
    // but for `accessible_by` every legal target — a `worker_class`
    // or a worker name — is declared in this same schedule, so the
    // resolution is purely schedule-internal and is done here in
    // lowering (not deferred to the linker). Scope: a plain
    // "undeclared name" typed error. Did-you-mean fuzzy suggestions
    // are deliberately out of scope — that is TASK-0096.
    // Runs after the synthetic default class is injected and all
    // workers are collected, so a name referring to a simple-form
    // worker or the default class resolves correctly. Iteration is
    // over the BTreeMap (deterministic order) so the first-error
    // report is stable.
    for region in ir.memory_regions.values() {
        if let Some(names) = &region.accessible_by {
            for name in names {
                let declared = ir.worker_classes.contains_key(name)
                    || ir.workers.contains_key(name);
                if !declared {
                    return Err(SchedLowerError::UnknownAccessibleByName {
                        region: region.name.clone(),
                        name: name.clone(),
                    });
                }
            }
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
    // `.node` everywhere: lowering ignores spans (TASK-0086); the
    // located `SchedLowerError` is TASK-0196.
    let kernel = &p.kernel.node;
    if ir.places.contains_key(kernel) {
        return Err(SchedLowerError::DuplicatePlace {
            kernel: kernel.clone(),
        });
    }
    let target = match &p.target {
        PlaceTarget::One(w) => {
            check_worker_declared(kernel, &w.node, ir)?;
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
                    return Err(SchedLowerError::DuplicatePlaceWorker {
                        kernel: kernel.clone(),
                        worker: w.node.clone(),
                    });
                }
            }
            for w in ws {
                check_worker_declared(kernel, &w.node, ir)?;
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

fn check_worker_declared(kernel: &str, worker: &str, ir: &SchedIR) -> Result<(), SchedLowerError> {
    if !ir.workers.contains_key(worker) {
        return Err(SchedLowerError::UnknownPlaceWorker {
            kernel: kernel.to_string(),
            worker: worker.to_string(),
        });
    }
    Ok(())
}

fn lower_place_data(
    pd: &super::ast::PlaceDataDirective,
    ir: &mut SchedIR,
) -> Result<(), SchedLowerError> {
    // `.node`: lowering ignores spans (TASK-0086 / TASK-0196).
    let data = &pd.data.node;
    let region = &pd.region.node;
    if ir.place_data.contains_key(data) {
        return Err(SchedLowerError::DuplicatePlaceData {
            data: data.clone(),
        });
    }
    if !ir.memory_regions.contains_key(region) {
        return Err(SchedLowerError::UnknownMemoryRegion {
            data: data.clone(),
            region: region.clone(),
        });
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
    // `.node`: lowering ignores spans (TASK-0086 / TASK-0196).
    let var = &l.var.node;
    if ir.loops.contains_key(var) {
        return Err(SchedLowerError::DuplicateLoop { var: var.clone() });
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
                    return Err(SchedLowerError::DuplicateLoopOption {
                        var: var.clone(),
                        option: kw.to_string(),
                    });
                }
            }
        }
    }
    let mut options = Vec::with_capacity(l.options.len());
    for opt in &l.options {
        options.push(lower_loop_option(var, opt)?);
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

fn lower_loop_option(var: &str, opt: &LoopOption) -> Result<ResolvedLoopOption, SchedLowerError> {
    // Numeric options share the same "strictly positive" rule. The
    // small helper keeps the variant list short and obvious.
    let positive = |n: u64, keyword: &str| -> Result<u64, SchedLowerError> {
        if n == 0 {
            Err(SchedLowerError::ZeroLoopOption {
                var: var.to_string(),
                option: keyword.to_string(),
            })
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
    // `.node`: lowering ignores spans (TASK-0086 / TASK-0196).
    let data = &t.data.node;
    if ir.transfers.contains_key(data) {
        return Err(SchedLowerError::DuplicateTransfer {
            data: data.clone(),
        });
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
                        return Err(SchedLowerError::ConflictingTransferMode {
                            data: data.clone(),
                        });
                    }
                }
                TransferOption::Buffer(_) => {
                    if seen.insert("buffer", ()).is_some() {
                        return Err(SchedLowerError::DuplicateTransferOption {
                            data: data.clone(),
                            option: "buffer".to_string(),
                        });
                    }
                }
                TransferOption::Notify(_) => {
                    if seen.insert("notify", ()).is_some() {
                        return Err(SchedLowerError::DuplicateTransferOption {
                            data: data.clone(),
                            option: "notify".to_string(),
                        });
                    }
                }
            }
        }
    }
    let mut options = Vec::with_capacity(t.options.len());
    for opt in &t.options {
        options.push(lower_transfer_option(data, opt)?);
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
    opt: &TransferOption,
) -> Result<ResolvedTransferOption, SchedLowerError> {
    Ok(match opt {
        TransferOption::Sync => ResolvedTransferOption::Sync,
        TransferOption::Async => ResolvedTransferOption::Async,
        TransferOption::Buffer(n) => {
            if *n == 0 {
                return Err(SchedLowerError::ZeroBufferOption {
                    data: data.to_string(),
                });
            }
            ResolvedTransferOption::Buffer(*n)
        }
        TransferOption::Notify(k) => ResolvedTransferOption::Notify(*k),
    })
}

fn lower_check(c: &super::ast::CheckDirective, ir: &mut SchedIR) -> Result<(), SchedLowerError> {
    // `.node`: lowering ignores spans (TASK-0086 / TASK-0196).
    let var = &c.var.node;
    if ir.checks.contains_key(var) {
        return Err(SchedLowerError::DuplicateCheck { var: var.clone() });
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
