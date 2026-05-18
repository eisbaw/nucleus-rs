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

use super::ast::{
    CheckAssert, Directive, LoopOption, PlaceTarget, SchedAst, TransferOption,
};
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

    for d in &ast.directives {
        match d {
            Directive::WorkerClass(c) => {
                if ir.worker_classes.contains_key(&c.name) {
                    return Err(SchedLowerError::DuplicateWorkerClass(c.name.clone()));
                }
                ir.worker_classes.insert(
                    c.name.clone(),
                    ResolvedWorkerClass {
                        name: c.name.clone(),
                        simd: c.simd.clone(),
                        memory: c.memory.clone(),
                        is_default: false,
                    },
                );
            }
            Directive::MemoryRegion(r) => {
                if ir.memory_regions.contains_key(&r.name) {
                    return Err(SchedLowerError::DuplicateMemoryRegion(r.name.clone()));
                }
                ir.memory_regions.insert(
                    r.name.clone(),
                    ResolvedMemoryRegion {
                        name: r.name.clone(),
                        size_bytes: r.size_bytes,
                        accessible_by: r.accessible_by.clone(),
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
                    if seen_in_this_decl.insert(entry.name.clone(), ()).is_some() {
                        return Err(SchedLowerError::DuplicateWorker(entry.name.clone()));
                    }
                    if ir.workers.contains_key(&entry.name) {
                        return Err(SchedLowerError::DuplicateWorker(entry.name.clone()));
                    }

                    let class = match &entry.class {
                        Some(c) => c.clone(),
                        None => {
                            needs_default_class = true;
                            DEFAULT_WORKER_CLASS.to_string()
                        }
                    };
                    ir.workers.insert(
                        entry.name.clone(),
                        ResolvedWorker {
                            name: entry.name.clone(),
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

    // ----------------------------------------------------------------
    // Pass 2: lower place / place_data / loop / transfer / check.
    // ----------------------------------------------------------------

    for d in &ast.directives {
        match d {
            // Already handled in pass 1.
            Directive::WorkerClass(_)
            | Directive::MemoryRegion(_)
            | Directive::Workers(_) => {}

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

fn lower_place(
    p: &super::ast::PlaceDirective,
    ir: &mut SchedIR,
) -> Result<(), SchedLowerError> {
    if ir.places.contains_key(&p.kernel) {
        return Err(SchedLowerError::DuplicatePlace {
            kernel: p.kernel.clone(),
        });
    }
    let target = match &p.target {
        PlaceTarget::One(w) => {
            check_worker_declared(&p.kernel, w, ir)?;
            ResolvedPlaceTarget::One(w.clone())
        }
        PlaceTarget::Many(ws) => {
            for w in ws {
                check_worker_declared(&p.kernel, w, ir)?;
            }
            ResolvedPlaceTarget::Many(ws.clone())
        }
    };
    ir.places.insert(
        p.kernel.clone(),
        ResolvedPlacement {
            kernel: p.kernel.clone(),
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
    if ir.place_data.contains_key(&pd.data) {
        return Err(SchedLowerError::DuplicatePlaceData {
            data: pd.data.clone(),
        });
    }
    if !ir.memory_regions.contains_key(&pd.region) {
        return Err(SchedLowerError::UnknownMemoryRegion {
            data: pd.data.clone(),
            region: pd.region.clone(),
        });
    }
    ir.place_data.insert(
        pd.data.clone(),
        ResolvedPlaceData {
            data: pd.data.clone(),
            region: pd.region.clone(),
        },
    );
    Ok(())
}

fn lower_loop(
    l: &super::ast::LoopDirective,
    ir: &mut SchedIR,
) -> Result<(), SchedLowerError> {
    if ir.loops.contains_key(&l.var) {
        return Err(SchedLowerError::DuplicateLoop {
            var: l.var.clone(),
        });
    }
    let mut options = Vec::with_capacity(l.options.len());
    for opt in &l.options {
        options.push(lower_loop_option(&l.var, opt)?);
    }
    ir.loops.insert(
        l.var.clone(),
        ResolvedLoopDirective {
            var: l.var.clone(),
            options,
        },
    );
    Ok(())
}

fn lower_loop_option(
    var: &str,
    opt: &LoopOption,
) -> Result<ResolvedLoopOption, SchedLowerError> {
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
    if ir.transfers.contains_key(&t.data) {
        return Err(SchedLowerError::DuplicateTransfer {
            data: t.data.clone(),
        });
    }
    let mut options = Vec::with_capacity(t.options.len());
    for opt in &t.options {
        options.push(lower_transfer_option(&t.data, opt)?);
    }
    ir.transfers.insert(
        t.data.clone(),
        ResolvedTransferDirective {
            data: t.data.clone(),
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

fn lower_check(
    c: &super::ast::CheckDirective,
    ir: &mut SchedIR,
) -> Result<(), SchedLowerError> {
    if ir.checks.contains_key(&c.var) {
        return Err(SchedLowerError::DuplicateCheck {
            var: c.var.clone(),
        });
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
        c.var.clone(),
        ResolvedCheckDirective {
            var: c.var.clone(),
            asserts,
        },
    );
    Ok(())
}
