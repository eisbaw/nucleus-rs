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
//!
//! # Multi-error reporting (TASK-0200)
//!
//! Lowering does NOT stop at the first violation: it accumulates every
//! genuinely-independent [`SchedLowerError`] across the whole pass and
//! returns them together as [`SchedLowerErrors`]. The cascade
//! infrastructure mirrors the algorithm cycle-3 design (TASK-0092)
//! verbatim — a [`failed_decls`](Accum::failed_decls) `BTreeMap`
//! poisoned-name set, four reference-resolution variants
//! (`UnknownWorkerClass`, `UnknownMemoryRegion`, `UnknownPlaceWorker`,
//! `UnknownAccessibleByName`) recognised as cascade-candidates, and
//! transitive-poison case-1 logic.
//!
//! Today's cascade landscape has TWO paths (see [`Accum`] type docs):
//!
//! 1. `failed_decls`-keyed name cascade (the algo cycle-3 design,
//!    transferred verbatim): NO live trigger today — every sched
//!    decl that survives its Duplicate-* gate is unconditionally
//!    inserted into the symbol table (there is no sched analog of
//!    `const N = 1/0`), so `failed_decls` stays empty in practice.
//!    Forward-looking infrastructure.
//! 2. `workers_missing`-keyed UnknownPlaceWorker cascade: FIRES
//!    today on the unique MissingWorkersDecl path. With no
//!    `workers = ...` directive the workers symbol table is empty
//!    by construction; every `place X on W` would emit
//!    `UnknownPlaceWorker{W}` as a pure cascade of the
//!    already-reported root. Suppressed so the user sees one root
//!    diagnostic instead of N cascade lines.

use std::collections::BTreeMap;

use super::ast::{CheckAssert, Directive, LoopOption, PlaceTarget, SchedAst, TransferOption};
use super::ir::{
    ResolvedCheckAssert, ResolvedCheckDirective, ResolvedLoopDirective, ResolvedLoopOption,
    ResolvedMemoryRegion, ResolvedPlaceData, ResolvedPlaceTarget, ResolvedPlacement,
    ResolvedTransferDirective, ResolvedTransferOption, ResolvedWorker, ResolvedWorkerClass,
    SchedIR, SchedLowerError, SchedLowerErrorKind, SchedLowerErrors, DEFAULT_WORKER_CLASS,
};

/// Lower a parsed [`SchedAst`] into a validated [`SchedIR`].
///
/// # Multi-error reporting (TASK-0200)
///
/// Lowering does **not** abort on the first violation. It walks
/// `ast.directives` in source order and *accumulates* every
/// genuinely-independent [`SchedLowerError`], returning them all in
/// one [`SchedLowerErrors`] bundle so a user sees every violation in
/// one compile cycle rather than recompiling once per error. The `Ok`
/// type is unchanged ([`SchedIR`]): a schedule that lowers produces
/// the exact same IR as before — zero behaviour change for valid
/// input (the determinism gate proves this byte-for-byte).
///
/// # Independent-vs-cascade discipline (AC#3 — the cycle-3 transfer)
///
/// The algorithm-side TASK-0092 cycle-3 fix established the
/// independence-vs-cascade discipline: emit every genuinely-
/// independent violation, suppress *cascade* errors (a reference to a
/// declaration that itself failed), and **transitively poison**
/// cascade-decls so depth>1 chains collapse to their single root.
/// That design transfers verbatim to schedule lowering:
///
/// - **Cascade boundary = symbol-table membership.** A declaration
///   that fails to lower would (in the algo precedent) not be
///   inserted into `ir.worker_classes` / `ir.memory_regions` /
///   `ir.workers` etc., and its name would go into
///   [`Accum::failed_decls`].
/// - **Reference errors are suppressed when the referenced name is
///   poisoned.** The four reference-resolution variants —
///   [`SchedLowerErrorKind::UnknownWorkerClass`],
///   [`SchedLowerErrorKind::UnknownMemoryRegion`],
///   [`SchedLowerErrorKind::UnknownPlaceWorker`],
///   [`SchedLowerErrorKind::UnknownAccessibleByName`] — are the
///   cascade-candidate kinds; every other variant is independent.
/// - **Duplicate-decl errors do NOT poison** (first decl is valid in
///   the table; suppressing dependents here would be undercount).
/// - **Transitive poison** ([`Accum::record_decl_failure`] case 1):
///   a declaration that fails ONLY because it refers to an already-
///   poisoned upstream is ITSELF inserted into `failed_decls`.
///   Cascade-decls have no independent meaning; every downstream
///   reference is by definition a transitive cascade of the same
///   upstream root. Without this, depth>1 cascades would leak as
///   overcount (the 5th-recurrence defect TASK-0092 cycle-3 closed).
///
/// # Cascade landscape today (honest-partial — TWO paths)
///
/// The sched-lowering variant set is dominantly INDEPENDENT. Path 1
/// (`failed_decls`-keyed name cascade) has NO LIVE TRIGGER today:
/// every `worker_class`/`memory_region`/`workers` decl that survives
/// its duplicate check is unconditionally inserted into the symbol
/// table — there is no sched analog of `const N = 1/0` (no
/// arithmetic expression evaluation at this layer). So `failed_decls`
/// is empty in practice on today's variant set; the Path-1
/// suppression rule is forward-looking infrastructure for the day a
/// sched construct gains expression evaluation.
///
/// Path 2 (`workers_missing`-keyed `UnknownPlaceWorker` suppression)
/// FIRES TODAY: with no `workers = ...` directive, `ir.workers` stays
/// empty by construction, and every subsequent `place X on W`
/// necessarily fires `UnknownPlaceWorker{W}` as a pure cascade of
/// the already-reported `MissingWorkersDecl` root. Suppressed.
/// `UnknownAccessibleByName` is NOT suppressed at this path (the
/// name could be a class OR a worker — see
/// [`Accum::is_cascade_of_failed_decl`] for the soundness argument).
///
/// Per-variant classification:
///
/// | Variant                                | Class       | Triggerable today | Suppressed when |
/// |----------------------------------------|-------------|-------------------|-----------------|
/// | `DuplicateWorkerClass`                 | Independent | yes               | never (non-poisoning) |
/// | `DuplicateMemoryRegion`                | Independent | yes               | never |
/// | `DuplicateWorker`                      | Independent | yes               | never |
/// | `DuplicatePlace`                       | Independent | yes               | never |
/// | `DuplicatePlaceData`                   | Independent | yes               | never |
/// | `DuplicateLoop`                        | Independent | yes               | never |
/// | `DuplicateTransfer`                    | Independent | yes               | never |
/// | `DuplicateCheck`                       | Independent | yes               | never |
/// | `DuplicateWorkersDecl`                 | Independent | yes               | never |
/// | `MissingWorkersDecl`                   | Independent ROOT + **Path-2 trigger** | yes | never |
/// | `DuplicatePlaceWorker`                 | Independent | yes               | never |
/// | `DuplicateLoopOption` / `DuplicateTransferOption` | Independent | yes | never |
/// | `ConflictingTransferMode`              | Independent | yes               | never |
/// | `ZeroLoopOption` / `ZeroBufferOption`  | Independent | yes               | never |
/// | `UnitPipelineOption`                   | Independent | yes               | never |
/// | `ZeroLatencyMax` / `DuplicateCheckAssertion` | Independent | yes        | never |
/// | `MissingLatencyMax` / `CheckOnStripMinedLoop` | Independent | yes      | never |
/// | `UnknownWorkerClass`                   | Path-1 candidate | yes (no current Path-1 source) | Path-1 if referenced class is in `failed_decls` (dormant today) |
/// | `UnknownMemoryRegion`                  | Path-1 candidate | yes (no current Path-1 source) | Path-1 if referenced region is in `failed_decls` (dormant today) |
/// | `UnknownPlaceWorker`                   | Path-1 candidate + **Path-2 LIVE** | yes | Path-1 dormant; Path-2 active under `workers_missing` |
/// | `UnknownAccessibleByName`              | Path-1 candidate; NOT Path-2 | yes (no current Path-1 source) | Path-1 if referenced name is in `failed_decls` (dormant today); explicitly NOT Path-2 |
///
/// # Determinism (PRD §10.1)
///
/// Errors are pushed in source order; `failed_decls` is a `BTreeMap`;
/// there is no `HashMap`/`HashSet` iteration on the error path. The
/// emitted sequence is a pure deterministic function of the input.
///
/// Spans populated on the AST (TASK-0086) are threaded into each
/// [`SchedLowerError`] on the `Err` path only (TASK-0196). The
/// success path reads no span, so the determinism gate stays
/// byte-identical (positions populate only for errors).
pub fn lower_sched(ast: &SchedAst) -> Result<SchedIR, SchedLowerErrors> {
    let mut ir = SchedIR {
        algo_path: ast.algo_path.clone(),
        ..SchedIR::default()
    };
    let mut acc = Accum::default();

    // ----------------------------------------------------------------
    // Pass 1: collect declarations (classes, regions, workers).
    //
    // Done as a dedicated pass so that the order-insensitive grammar
    // (§2 note 2) doesn't force callers to declare classes before
    // `workers = ...` references them.
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
    // injected. We record the offending reference together with its
    // source span here, at the AST-walk site where the `Spanned` is in
    // scope, and validate from these tables once collection is
    // complete. Order of first error is preserved: `worker_class_refs`
    // is in worker-entry source order; `accessible_by_refs` is in
    // directive then list order.
    let mut worker_class_refs: Vec<(String, String, core::ops::Range<usize>)> = Vec::new();
    // (region_name, accessible_by_name, span) per `accessible_by` entry.
    let mut accessible_by_refs: Vec<(String, String, core::ops::Range<usize>)> = Vec::new();

    for d in &ast.directives {
        match &d.node {
            Directive::WorkerClass(c) => {
                if ir.worker_classes.contains_key(&c.name.node) {
                    // Duplicate — record (non-poisoning) and skip the
                    // insert (first decl wins, mirrors the algo
                    // precedent where Duplicate* does NOT poison).
                    acc.record_decl_failure(
                        &c.name.node,
                        SchedLowerError::at(
                            SchedLowerErrorKind::DuplicateWorkerClass(c.name.node.clone()),
                            c.name.span.clone(),
                        ),
                    );
                    continue;
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
                    acc.record_decl_failure(
                        &r.name.node,
                        SchedLowerError::at(
                            SchedLowerErrorKind::DuplicateMemoryRegion(r.name.node.clone()),
                            r.name.span.clone(),
                        ),
                    );
                    continue;
                }
                // Record each `accessible_by` name WITH its source
                // span before the span is stripped into the
                // plain-`String` IR (TASK-0196 option (b)).
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
                    // Second (or later) `workers = ...` directive:
                    // record the violation and skip ALL of this
                    // duplicate decl's entries (first decl wins,
                    // mirrors the original "second is invalid"
                    // semantics — concatenation is ambiguous).
                    acc.record_stmt_error(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicateWorkersDecl,
                        d.span.clone(),
                    ));
                    continue;
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
                        // Per-decl duplicate (`{ host, host }`):
                        // record (non-poisoning) and skip the entry.
                        acc.record_decl_failure(
                            &entry.name.node,
                            SchedLowerError::at(
                                SchedLowerErrorKind::DuplicateWorker(entry.name.node.clone()),
                                entry.name.span.clone(),
                            ),
                        );
                        continue;
                    }
                    if ir.workers.contains_key(&entry.name.node) {
                        // Cross-decl duplicate: same handling.
                        acc.record_decl_failure(
                            &entry.name.node,
                            SchedLowerError::at(
                                SchedLowerErrorKind::DuplicateWorker(entry.name.node.clone()),
                                entry.name.span.clone(),
                            ),
                        );
                        continue;
                    }

                    let class = match &entry.class {
                        Some(c) => c.node.clone(),
                        None => {
                            needs_default_class = true;
                            DEFAULT_WORKER_CLASS.to_string()
                        }
                    };
                    // Record the class reference WITH its source span
                    // so the `UnknownWorkerClass` check can be done
                    // post-collection with the offending `Spanned` in
                    // scope.
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
        // *absence* of a `workers = ...` directive — there is no
        // source token to underline.
        //
        // Set the `workers_missing` cascade flag (TASK-0200 honest
        // disclosure): with no workers decl, `ir.workers` stays
        // empty by construction, and every subsequent
        // `place X on W` would fire `UnknownPlaceWorker{W}` as a
        // pure cascade of THIS root. Those are suppressed by
        // `Accum::is_cascade_of_failed_decl` so the user sees the
        // single ROOT diagnostic (missing workers decl), not N
        // cascade place-on-unknown-worker lines.
        acc.workers_missing = true;
        acc.record_stmt_error(SchedLowerError::new(SchedLowerErrorKind::MissingWorkersDecl));
    }

    // Synthesise the default class only after we know whether any
    // simple-form entry exists. Collision with a user-declared class
    // of the same name is caught by `DuplicateWorkerClass`.
    if needs_default_class {
        if ir.worker_classes.contains_key(DEFAULT_WORKER_CLASS) {
            // User declared a class with the synthetic name — record
            // the collision (non-poisoning, position-less per the
            // type docs). Do NOT insert the synthetic class (would
            // overwrite); the user's class stays.
            acc.record_decl_failure(
                DEFAULT_WORKER_CLASS,
                SchedLowerError::new(
                    SchedLowerErrorKind::DuplicateWorkerClass(DEFAULT_WORKER_CLASS.to_string()),
                ),
            );
        } else {
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
    }

    // Validate every worker's class reference now that all classes
    // are collected. First-error ordering is preserved bit-for-bit by
    // sorting `worker_class_refs` by worker name — the old code
    // iterated `ir.workers.values()` (BTreeMap, sorted).
    worker_class_refs.sort_by(|a, b| a.0.cmp(&b.0));
    for (worker, class, span) in &worker_class_refs {
        if !ir.worker_classes.contains_key(class) {
            acc.record_stmt_error(SchedLowerError::at(
                SchedLowerErrorKind::UnknownWorkerClass {
                    worker: worker.clone(),
                    class: class.clone(),
                    suggestion: crate::error::suggest(
                        class,
                        ir.worker_classes.keys().map(String::as_str),
                    ),
                },
                span.clone(),
            ));
        }
    }

    // Validate `memory_region R { accessible_by = { ... } }` against
    // the union of declared `worker_class` and worker names.
    accessible_by_refs.sort_by(|a, b| a.0.cmp(&b.0));
    for (region, name, span) in &accessible_by_refs {
        let declared =
            ir.worker_classes.contains_key(name) || ir.workers.contains_key(name);
        if !declared {
            acc.record_stmt_error(SchedLowerError::at(
                SchedLowerErrorKind::UnknownAccessibleByName {
                    region: region.clone(),
                    name: name.clone(),
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
    //
    // Each per-directive function returns `Result<(), SchedLowerError>`
    // (single error per directive — same granularity as the algo
    // pass's "one error per item"). The caller catches each and
    // routes through the appropriate Accum hook: place/place_data are
    // decl-level (define a kernel/data placement that could in
    // principle be referenced — though no current reference path
    // exists), loop/transfer/check are statement-level.
    // ----------------------------------------------------------------

    for d in &ast.directives {
        match &d.node {
            // Already handled in pass 1.
            Directive::WorkerClass(_) | Directive::MemoryRegion(_) | Directive::Workers(_) => {}

            Directive::Place(p) => {
                if let Err(e) = lower_place(p, &mut ir) {
                    acc.record_decl_failure(&p.kernel.node, e);
                }
            }
            Directive::PlaceData(pd) => {
                if let Err(e) = lower_place_data(pd, &mut ir) {
                    acc.record_decl_failure(&pd.data.node, e);
                }
            }
            Directive::Loop(l) => {
                if let Err(e) = lower_loop(l, &mut ir) {
                    acc.record_stmt_error(e);
                }
            }
            Directive::Transfer(t) => {
                if let Err(e) = lower_transfer(t, &mut ir) {
                    acc.record_stmt_error(e);
                }
            }
            Directive::Check(c) => {
                if let Err(e) = lower_check(c, &mut ir) {
                    acc.record_stmt_error(e);
                }
            }
        }
    }

    // TASK-0052.02 review-gate finding #3: cross-check that no
    // `check loop V` targets a loop V that is strip-mined by
    // `loop V : block=N`. After strip-mining, `inject_check_frames`
    // would silently drop the check (the inner block-tile Event::Loop
    // is skipped by design; the source-loop V's `iter_var` is reused
    // for the inner loop). Reject here so the silent-loss case is
    // fail-loud, not deferred to a runtime that never runs.
    let strip_mined_vars: Vec<String> = ir
        .loops
        .iter()
        .filter_map(|(var, dir)| {
            if dir
                .options
                .iter()
                .any(|opt| matches!(opt, ResolvedLoopOption::Block(_)))
            {
                Some(var.clone())
            } else {
                None
            }
        })
        .collect();
    for var in &strip_mined_vars {
        if ir.checks.contains_key(var) {
            // Use the directive's loop_var span (the check directive's
            // var span) so the diagnostic points at the user-written
            // `check loop V` not the `loop V` directive — the latter
            // could remain (just drop the check), but the user's
            // intent is more likely the check is what they want.
            let span = ast
                .directives
                .iter()
                .find_map(|d| match &d.node {
                    super::ast::Directive::Check(c) if c.var.node == *var => {
                        Some(c.var.span.clone())
                    }
                    _ => None,
                })
                .unwrap_or(0..0);
            acc.record_stmt_error(SchedLowerError::at(
                SchedLowerErrorKind::CheckOnStripMinedLoop { var: var.clone() },
                span,
            ));
        }
    }

    match acc.into_errors() {
        Some(errors) => Err(SchedLowerErrors::from_nonempty(errors)),
        None => Ok(ir),
    }
}

/// Error accumulator with cascade-suppression bookkeeping.
///
/// `errors` is the source-ordered collected set. `failed_decls` is the
/// poisoned-name set (see [`lower_sched`] docs): a `BTreeMap` (NOT a
/// hash set) so the error path has no nondeterministic iteration —
/// though in fact we only ever *look up* by name, never iterate, the
/// ordered map keeps the intent unambiguous and the path
/// hash-iteration-free.
///
/// `workers_missing` is the ONE in-pass cascade trigger that fires
/// today: if the schedule had NO `workers = ...` directive, then
/// `ir.workers` is empty by construction and every downstream
/// `place X on W` is a pure cascade of
/// [`SchedLowerErrorKind::MissingWorkersDecl`] — the user already has
/// the root diagnostic. The flag is set alongside
/// [`SchedLowerErrorKind::MissingWorkersDecl`]; downstream
/// `UnknownPlaceWorker` errors are suppressed by
/// [`Accum::is_cascade_of_failed_decl`]. **NARROW**:
/// `UnknownAccessibleByName` is NOT suppressed at this path because
/// the referenced name could be a class OR a worker — see the
/// soundness argument in [`Accum::is_cascade_of_failed_decl`].
///
/// # Cascade landscape (TASK-0200, honest disclosure)
///
/// On today's variant set there are TWO suppression paths:
///
/// 1. `failed_decls`-keyed name suppression (the algo cycle-3 design
///    transferred verbatim): wired faithfully but **has no live
///    trigger** because no sched decl path actually
///    fails-and-fails-to-insert beyond the Duplicate-* gate (which is
///    non-poisoning by design — first decl wins). Forward-looking
///    infrastructure for the day a sched construct gains expression
///    evaluation.
/// 2. `workers_missing`-keyed `UnknownPlaceWorker` suppression: **fires
///    today** on the unique MissingWorkersDecl path — a schedule
///    without a `workers = ...` directive triggers
///    `MissingWorkersDecl` and then every subsequent
///    `UnknownPlaceWorker` is by definition a cascade of that root
///    (the workers symbol table is empty by construction).
///    `UnknownAccessibleByName` is NOT suppressed here (see the
///    function-doc on `is_cascade_of_failed_decl` for why).
#[derive(Default)]
struct Accum {
    errors: Vec<SchedLowerError>,
    failed_decls: BTreeMap<String, ()>,
    /// `true` iff the schedule has no `workers = ...` directive (a
    /// [`SchedLowerErrorKind::MissingWorkersDecl`] has been or will be
    /// recorded). When set, every downstream
    /// [`SchedLowerErrorKind::UnknownPlaceWorker`] is a cascade of that
    /// root and is suppressed by [`Accum::is_cascade_of_failed_decl`].
    /// `UnknownAccessibleByName` is NOT suppressed here — its
    /// referenced name could be a class OR a worker, and only the
    /// worker-side miss is a cascade. The single in-pass cascade
    /// trigger that fires today (see type docs).
    workers_missing: bool,
}

impl Accum {
    /// A declaration (`worker_class` / `memory_region` / worker entry
    /// / `place` / `place_data`) failed to lower.
    ///
    /// Three cases, in priority order — mirrors
    /// [`crate::algo::lower`] `Accum::record_decl_failure`:
    ///
    /// 1. The declaration's own failure is itself a *cascade* of an
    ///    already-poisoned name (a reference-resolution error naming
    ///    a poisoned upstream — see
    ///    [`Accum::is_cascade_of_failed_decl`]). Suppress the error,
    ///    AND **transitively poison this declaration's own name** so
    ///    every downstream reference to it (further decls,
    ///    statements) is also recognised as a cascade of the same
    ///    root and suppressed.
    ///
    ///    Soundness of the transitive poison: a name that *never*
    ///    successfully declared has no *independent* meaning — there
    ///    is no resolved class / region / worker / placement behind
    ///    it. Every downstream reference is, by definition, a
    ///    transitive cascade of the upstream root that was already
    ///    reported. Inserting `name` into [`failed_decls`] here makes
    ///    the existing cascade-suppression rule
    ///    ([`Accum::is_cascade_of_failed_decl`]) cover those
    ///    transitive references too. Without it, downstream uses
    ///    would emit `Unknown*(name)` and, since `name` wasn't in
    ///    `failed_decls`, the suppression rule would miss them — the
    ///    classic transitive overcount that bit the algo-side cycle-3
    ///    closer.
    ///
    ///    Today's sched-lowering surface has NO live trigger for case
    ///    1 (no decl path fails-and-fails-to-insert beyond the
    ///    Duplicate* gate, which is case 2). The branch is present
    ///    so the design is identical to the algo precedent and
    ///    forward-ready when a poison-source variant lands.
    ///
    /// 2. A duplicate-name collision: record the error but do NOT
    ///    poison — the *first* (valid) declaration is still in the
    ///    symbol table, so the name resolves for dependents; there is
    ///    no cascade to suppress.
    ///
    /// 3. A genuine independent evaluation failure: record the error
    ///    AND poison the name so its dependents' resulting reference
    ///    errors are recognised as cascade and suppressed.
    fn record_decl_failure(&mut self, name: &str, e: SchedLowerError) {
        // Case 1: a declaration that failed only because it references
        // an already-failed declaration is a cascade, not independent.
        // Suppress the error AND transitively poison this decl's name
        // so its own downstream references are also recognised as a
        // cascade of the same root (TASK-0092 cycle-3 transitive-
        // poison design, transferred verbatim by TASK-0200).
        if self.is_cascade_of_failed_decl(&e) {
            self.failed_decls.insert(name.to_string(), ());
            return;
        }
        let is_duplicate = matches!(
            e.kind,
            SchedLowerErrorKind::DuplicateWorkerClass(_)
                | SchedLowerErrorKind::DuplicateMemoryRegion(_)
                | SchedLowerErrorKind::DuplicateWorker(_)
                | SchedLowerErrorKind::DuplicatePlace { .. }
                | SchedLowerErrorKind::DuplicatePlaceData { .. }
        );
        if !is_duplicate {
            self.failed_decls.insert(name.to_string(), ());
        }
        self.errors.push(e);
    }

    /// A statement (a `loop` / `transfer` / `check` directive or a
    /// post-collection reference validation) failed to lower. If the
    /// error is a reference to a name that a *failed* declaration
    /// poisoned, it is a pure cascade of that already-reported root
    /// failure → suppress. Otherwise it is an independent violation
    /// → record.
    fn record_stmt_error(&mut self, e: SchedLowerError) {
        if self.is_cascade_of_failed_decl(&e) {
            return;
        }
        self.errors.push(e);
    }

    /// True iff `e` is a reference-resolution error that is a
    /// *secondary consequence* of an already-reported root failure.
    /// Two paths:
    ///
    /// 1. The referenced identifier is in [`Self::failed_decls`] (the
    ///    algo cycle-3 cascade-by-name rule, transferred verbatim).
    ///    Today no sched decl path populates `failed_decls` so this
    ///    branch is dormant — forward-looking infrastructure.
    ///
    /// 2. `self.workers_missing` is set AND the error is a
    ///    [`SchedLowerErrorKind::UnknownPlaceWorker`] — the schedule
    ///    forgot the `workers = ...` directive, the workers symbol
    ///    table is empty by construction, and every
    ///    `place X on W` necessarily fires `UnknownPlaceWorker{W}` as
    ///    a pure cascade of the already-reported
    ///    [`SchedLowerErrorKind::MissingWorkersDecl`] root.
    ///    Suppressed.
    ///
    ///    [`SchedLowerErrorKind::UnknownAccessibleByName`] is NOT
    ///    suppressed here because the referenced name could be a
    ///    class OR a worker — only the worker-side miss is a cascade
    ///    of MissingWorkersDecl; an unknown class is independent. We
    ///    cannot distinguish from the error alone, and a conservative
    ///    "report it" is the right honest-partial: the user gets one
    ///    extra line per truly-ambiguous case but no real cascade
    ///    leaks as an independent error.
    ///
    /// Every other error kind is an independent property of the
    /// directive itself.
    fn is_cascade_of_failed_decl(&self, e: &SchedLowerError) -> bool {
        // Path 2: MissingWorkersDecl-induced UnknownPlaceWorker
        // cascade. The empty workers symbol table makes every
        // `place X on W` an automatic UnknownPlaceWorker — pure
        // cascade of the already-reported root.
        if self.workers_missing
            && matches!(e.kind, SchedLowerErrorKind::UnknownPlaceWorker { .. })
        {
            return true;
        }
        // Path 1: failed_decls-keyed name cascade (algo cycle-3
        // design, forward-looking).
        let referenced = match &e.kind {
            SchedLowerErrorKind::UnknownWorkerClass { class, .. } => class.as_str(),
            SchedLowerErrorKind::UnknownMemoryRegion { region, .. } => region.as_str(),
            SchedLowerErrorKind::UnknownPlaceWorker { worker, .. } => worker.as_str(),
            SchedLowerErrorKind::UnknownAccessibleByName { name, .. } => name.as_str(),
            _ => return false,
        };
        self.failed_decls.contains_key(referenced)
    }

    /// Consume into the collected error set, or `None` if lowering
    /// succeeded (no errors).
    fn into_errors(self) -> Option<Vec<SchedLowerError>> {
        if self.errors.is_empty() {
            None
        } else {
            Some(self.errors)
        }
    }
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
            // NOT silently folded to a unique set.
            let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
            for w in ws {
                if seen.insert(w.as_str(), ()).is_some() {
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
    if !ir.workers.contains_key(&worker.node) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::UnknownPlaceWorker {
                kernel: kernel.to_string(),
                worker: worker.node.clone(),
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
    let data = &pd.data.node;
    let region = &pd.region.node;
    if ir.place_data.contains_key(data) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicatePlaceData {
                data: data.clone(),
            },
            pd.data.span.clone(),
        ));
    }
    if !ir.memory_regions.contains_key(region) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::UnknownMemoryRegion {
                data: data.clone(),
                region: region.clone(),
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
    let var = &l.var.node;
    let var_span = &l.var.span;
    if ir.loops.contains_key(var) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicateLoop { var: var.clone() },
            var_span.clone(),
        ));
    }
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
        // pipeline=D rejects BOTH D=0 (via `positive`) and D=1 (TASK-0134).
        // D=1 is a no-op pipeline that would silently lower to
        // initial_marking=1 — equivalent to the default. Force the user to
        // either omit `pipeline` or specify D >= 2.
        LoopOption::Pipeline(n) => {
            let d = positive(*n, "pipeline")?;
            if d == 1 {
                return Err(SchedLowerError::at(
                    SchedLowerErrorKind::UnitPipelineOption {
                        var: var.to_string(),
                    },
                    var_span.clone(),
                ));
            }
            ResolvedLoopOption::Pipeline(d)
        }
        LoopOption::Reuse => ResolvedLoopOption::Reuse,
        LoopOption::Partition(k) => ResolvedLoopOption::Partition(*k),
    })
}

fn lower_transfer(
    t: &super::ast::TransferDirective,
    ir: &mut SchedIR,
) -> Result<(), SchedLowerError> {
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
    let var = &c.var.node;
    if ir.checks.contains_key(var) {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::DuplicateCheck { var: var.clone() },
            c.var.span.clone(),
        ));
    }

    // TASK-0052.01: AC#2/AC#3 — each assertion kind is a unique slot
    // per check directive; AC#3 — latency_max=0 is degenerate. Both
    // checks happen here before the asserts vector is built so the
    // first offender is reported at its source span.
    //
    // TASK-0052.02 review-gate finding #1: also enforce that at least
    // one `latency_max` is present. The grammar requires >= 1 assert
    // but allows the on_violation-only directive `check loop V :
    // on_violation=panic;` which is semantically empty (on_violation
    // is the action when an assertion fails; without a measurement
    // there is nothing to violate). The check is at the end of the
    // walk so kind-duplicates / zero-value errors take precedence
    // (they name a specific bad token; missing-latency is the
    // catch-all when the whole directive lacks a measurement).
    let mut seen_latency = false;
    let mut seen_on_violation = false;
    for a in &c.asserts {
        match a {
            CheckAssert::LatencyMax(t) => {
                if t.nanos == 0 {
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::ZeroLatencyMax { var: var.clone() },
                        c.var.span.clone(),
                    ));
                }
                if seen_latency {
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicateCheckAssertion {
                            var: var.clone(),
                            kind: "latency_max".into(),
                        },
                        c.var.span.clone(),
                    ));
                }
                seen_latency = true;
            }
            CheckAssert::OnViolation(_) => {
                if seen_on_violation {
                    return Err(SchedLowerError::at(
                        SchedLowerErrorKind::DuplicateCheckAssertion {
                            var: var.clone(),
                            kind: "on_violation".into(),
                        },
                        c.var.span.clone(),
                    ));
                }
                seen_on_violation = true;
            }
        }
    }

    if !seen_latency {
        return Err(SchedLowerError::at(
            SchedLowerErrorKind::MissingLatencyMax { var: var.clone() },
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
