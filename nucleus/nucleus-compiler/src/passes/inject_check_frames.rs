//! Inject real-time `check loop V : latency_max=T` frames onto
//! `Event::Loop`s (PRD §6.3.5 / TASK-0052.02).
//!
//! Runs as a post-projection pass over the per-worker EventList,
//! AFTER `passes::petri_to_events::acfg_to_events`. The join is:
//!
//! ```text
//!   sched_ir.checks: BTreeMap<String, ResolvedCheckDirective>
//!                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//!                    keyed by loop-variable NAME ("frame", "n", ...)
//!
//!   acfg.name_iter_vars: BTreeMap<String, IterVar>
//!                        ^^^^^^^^^^^^^^^^^^^^^^^^^
//!                        same name -> opaque IterVar id
//!
//!   Event::Loop.iter_var: IterVar  ←  the join key into per-worker
//! ```
//!
//! Why a separate pass and not folded into `acfg_to_events`:
//!
//! 1. `acfg_to_events` has 30+ test call sites that pass only the
//!    ACFG (no SchedIR available). Extending the signature would
//!    cascade through every one.
//! 2. The schedule-only fact ("does this loop have a `check` directive")
//!    is genuinely a *post*-projection enrichment — it does not change
//!    the loop NEST that `acfg_to_events` produces, only adds an
//!    annotation on existing loops.
//! 3. Composability (MPED principle): the pass takes
//!    `(per_worker, checks, name_iter_vars)` and returns the enriched
//!    per_worker — no hidden state, no captive framework.
//!
//! ## Which loops get a check_frame
//!
//! Only OUTER user-source loops (`block_tag.is_none()`). A
//! strip-mined inner loop synthesised by `block_transform` reuses the
//! SOURCE iter-var but is implementation detail — the user wrote
//! `check loop frame`, meaning "the user-source `frame` loop", not
//! "an inner block tile". The injection pass explicitly skips
//! `block_tag.is_some()` loops, so the assertion attaches to the
//! outer source loop ONCE per worker (not per block iteration).
//!
//! ## Determinism
//!
//! The pass is a pure function of its inputs. `BTreeMap` iteration is
//! sorted; `IterVar` ids are assigned deterministically by
//! `build_acfg`. Two runs produce identical output.
//!
//! ## What happens to a `check loop V` whose `V` is never an
//! `Event::Loop.iter_var`
//!
//! Nothing: the projection drops it silently for THIS pass (the loop
//! does not exist on this worker, or the loop variable refers to an
//! algorithm symbol that did not produce an `Event::Loop`). The link
//! step (`nucleus_compiler::link`) already rejects a `check loop V` whose `V`
//! is not declared as an algorithm loop variable — so by the time we
//! reach this pass, `V` IS an algorithm loop variable. A `V` that
//! resolves to an `IterVar` but produces NO `Event::Loop` (e.g. the
//! algorithm loop body produced no observable events) is acceptable:
//! the assertion vacuously holds (there is no iteration body to
//! measure).

use std::collections::BTreeMap;

use crate::event::{CheckFrame, Event, IterVar, ViolationKind, WorkerId};
use crate::sched::ast::ViolationKind as AstViolationKind;
use crate::sched::ir::{ResolvedCheckAssert, ResolvedCheckDirective};

/// Inject `check_frame` annotations onto every outer `Event::Loop`
/// whose `iter_var` resolves to a loop name that has a `check loop`
/// directive in the schedule.
///
/// `name_iter_vars` is `acfg.name_iter_vars` (name -> IterVar).
/// `checks` is `sched_ir.checks` (name -> ResolvedCheckDirective).
///
/// The function consumes `per_worker` and returns the enriched map.
/// (Owned-input style mirrors `inject_syncs` / `inject_transfers`.)
pub fn inject_check_frames(
    per_worker: BTreeMap<WorkerId, Vec<Event>>,
    checks: &BTreeMap<String, ResolvedCheckDirective>,
    name_iter_vars: &BTreeMap<String, IterVar>,
) -> BTreeMap<WorkerId, Vec<Event>> {
    if checks.is_empty() {
        // Fast path / common case: no `check loop` directives in the
        // schedule. Nothing to inject; return unchanged. This keeps
        // the pre-TASK-0052.02 e2e baseline byte-identical (no schedule
        // in the e2e tier-1 matrix uses `check loop` today).
        return per_worker;
    }

    // Pre-resolve check name -> (IterVar, CheckFrame) once. A
    // BTreeMap<IterVar, CheckFrame> keyed on the resolved id is the
    // join key for the walk. A directive whose name does not resolve
    // to an IterVar is silently dropped here (see module docs); the
    // link step is the gate that rejects unknown names.
    let mut by_iter_var: BTreeMap<IterVar, CheckFrame> = BTreeMap::new();
    for (name, directive) in checks {
        let Some(iv) = name_iter_vars.get(name) else {
            // Name resolves to an algorithm loop but produced no
            // IterVar (e.g. a loop the compiler eliminated). Skip;
            // the assertion has no loop to bind to.
            continue;
        };
        let frame = resolve_check_directive(name, directive);
        by_iter_var.insert(*iv, frame);
    }

    // Walk each worker's event list and inject on matching outer loops.
    per_worker
        .into_iter()
        .map(|(wid, events)| (wid, inject_events(events, &by_iter_var)))
        .collect()
}

/// Resolve the user-source `ResolvedCheckDirective` to the
/// codegen-layer `CheckFrame`. Materialises the on_violation default
/// (`Panic`, per PRD §6.3.5) at THIS seam (the IR stays faithful to
/// what the user wrote; the default lives at codegen — TASK-0052.01
/// forward-carry note).
///
/// Multiple asserts of the same kind are structurally impossible —
/// `sched_lower::lower_check` rejects duplicate `latency_max` or
/// `on_violation` at lowering time (`DuplicateCheckAssertion`).
fn resolve_check_directive(loop_var: &str, directive: &ResolvedCheckDirective) -> CheckFrame {
    let mut latency_max_ns: Option<u64> = None;
    let mut on_violation: Option<ViolationKind> = None;
    for a in &directive.asserts {
        match a {
            ResolvedCheckAssert::LatencyMax(t) => {
                latency_max_ns = Some(t.nanos);
            }
            ResolvedCheckAssert::OnViolation(k) => {
                on_violation = Some(match k {
                    AstViolationKind::Panic => ViolationKind::Panic,
                    AstViolationKind::Log => ViolationKind::Log,
                    AstViolationKind::Count => ViolationKind::Count,
                });
            }
        }
    }
    // `latency_max` is mandatory on a `check loop` directive. The
    // grammar permits `check loop V : on_violation=panic;` (asserts
    // are `.at_least(1)` but the variant is a choice — on_violation
    // alone is parseable). Sched-lower's `MissingLatencyMax` gate
    // (TASK-0052.02 review-gate finding #1) rejects that case at the
    // user-input layer, so by the time inject_check_frames runs,
    // every `ResolvedCheckDirective` is guaranteed to contain
    // exactly one `LatencyMax(_)`. `unreachable!()` here is a
    // compiler-invariant assertion, NOT a user-input handler:
    let latency_max_ns = latency_max_ns.unwrap_or_else(|| {
        unreachable!(
            "inject_check_frames: `check loop {loop_var}` has no `latency_max` assert; \
             sched_lower::lower_check is supposed to reject this case via \
             SchedLowerErrorKind::MissingLatencyMax — if you see this panic, \
             the sched-lower gate has regressed (TASK-0052.02 review-gate finding #1)"
        )
    });
    CheckFrame {
        latency_max_ns,
        on_violation: on_violation.unwrap_or(ViolationKind::Panic),
        loop_var: loop_var.to_string(),
    }
}

/// Walk `events` and inject `check_frame` onto every outer
/// `Event::Loop` (block_tag.is_none()) whose iter_var matches an
/// entry in `by_iter_var`. Recurses into loop bodies so a
/// `check loop V` whose V is a NESTED source loop still attaches.
///
/// A loop with `block_tag.is_some()` is a strip-mined inner loop and
/// is skipped — but we still recurse into its body (a nested source
/// loop inside a tiled outer loop is the intended target of a
/// `check` directive on the nested loop's name).
fn inject_events(
    events: Vec<Event>,
    by_iter_var: &BTreeMap<IterVar, CheckFrame>,
) -> Vec<Event> {
    events
        .into_iter()
        .map(|e| inject_event(e, by_iter_var))
        .collect()
}

fn inject_event(event: Event, by_iter_var: &BTreeMap<IterVar, CheckFrame>) -> Event {
    match event {
        Event::Loop {
            iter_var,
            range,
            body,
            block_tag,
            check_frame: existing,
        } => {
            // Recurse first so a nested loop's frame is set even if the
            // outer one is not a match.
            let body = inject_events(body, by_iter_var);
            // Defensive: `acfg_to_events` and every other pass leave
            // `check_frame == None` (this pass is the SOLE producer).
            // A pre-existing Some(_) would mean the pass ran twice on
            // the same input or someone else populated the field —
            // both are bugs. Pass through verbatim (idempotent on the
            // value side) but assert the invariant in debug builds.
            debug_assert!(
                existing.is_none(),
                "inject_check_frames: Event::Loop already has check_frame set \
                 (idempotency / single-source-of-truth invariant violated)"
            );
            let check_frame = if block_tag.is_some() {
                // Strip-mined inner loop: user `check loop V` does NOT
                // attach to a synthesised inner block loop (the V the
                // user wrote names the SOURCE loop; the strip-mine is
                // implementation detail). Carry no frame here; the
                // outer source loop (recursed up the call chain) gets
                // it.
                None
            } else {
                by_iter_var.get(&iter_var).cloned()
            };
            Event::Loop {
                iter_var,
                range,
                body,
                block_tag,
                check_frame,
            }
        }
        // Non-Loop events are pass-through: a `check loop` directive
        // attaches to a LOOP, never to a Fire / Sync / Push / Wait /
        // Alloc / Free.
        other => other,
    }
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, IterVar, KernelId, IterTile, FireBinding, WorkerId};
    use crate::sched::ast::TimeLit;
    use crate::sched::ast::TimeUnit;

    fn ms(n: u64) -> TimeLit {
        TimeLit {
            nanos: n * 1_000_000,
            original_unit: TimeUnit::Ms,
            original_value: n,
        }
    }

    fn fire() -> Event {
        Event::Fire {
            kernel: KernelId(1),
            tile: IterTile::empty(),
            bindings: FireBinding::none(),
        }
    }

    #[test]
    fn no_checks_is_passthrough() {
        // An empty `checks` map is the e2e baseline: the pass MUST
        // produce a byte-identical EventList (no e2e cell uses
        // `check loop` today).
        let pw: BTreeMap<WorkerId, Vec<Event>> = vec![(
            WorkerId(0),
            vec![Event::loop_over(IterVar(0), 0..10, vec![fire()])],
        )]
        .into_iter()
        .collect();
        let names: BTreeMap<String, IterVar> = vec![("n".to_string(), IterVar(0))]
            .into_iter()
            .collect();
        let out = inject_check_frames(pw.clone(), &BTreeMap::new(), &names);
        assert_eq!(out, pw, "no `check` directives -> pass is identity");
    }

    #[test]
    fn matching_outer_loop_gets_check_frame_default_panic() {
        // `check loop n : latency_max = 10ms;` with no on_violation
        // ⇒ default Panic, threshold 10_000_000 ns.
        let pw: BTreeMap<WorkerId, Vec<Event>> = vec![(
            WorkerId(0),
            vec![Event::loop_over(IterVar(0), 0..10, vec![fire()])],
        )]
        .into_iter()
        .collect();
        let names: BTreeMap<String, IterVar> = vec![("n".to_string(), IterVar(0))]
            .into_iter()
            .collect();
        let checks: BTreeMap<String, ResolvedCheckDirective> = vec![(
            "n".to_string(),
            ResolvedCheckDirective {
                var: "n".to_string(),
                asserts: vec![ResolvedCheckAssert::LatencyMax(ms(10))],
                // TASK-0099: hand-built test fixture has no source text.
                var_span: None,
            },
        )]
        .into_iter()
        .collect();
        let out = inject_check_frames(pw, &checks, &names);
        let w0 = &out[&WorkerId(0)];
        match &w0[0] {
            Event::Loop { check_frame, .. } => {
                let cf = check_frame.as_ref().expect("outer loop must have check_frame");
                assert_eq!(cf.latency_max_ns, 10_000_000);
                assert_eq!(cf.on_violation, ViolationKind::Panic);
                assert_eq!(cf.loop_var, "n");
            }
            other => panic!("expected Event::Loop, got {other:?}"),
        }
    }

    #[test]
    fn unknown_check_name_silently_dropped() {
        // A `check loop X` whose `X` is not in `name_iter_vars` is
        // dropped. The link step is the gate that rejects unknown
        // names; reaching THIS pass with one means the link step let
        // it through (compiler bug elsewhere). The pass must NOT
        // panic — it skips, the gate elsewhere is the failure mode.
        let pw: BTreeMap<WorkerId, Vec<Event>> = vec![(
            WorkerId(0),
            vec![Event::loop_over(IterVar(0), 0..10, vec![fire()])],
        )]
        .into_iter()
        .collect();
        let names: BTreeMap<String, IterVar> = vec![("n".to_string(), IterVar(0))]
            .into_iter()
            .collect();
        let checks: BTreeMap<String, ResolvedCheckDirective> = vec![(
            "x".to_string(), // not in names
            ResolvedCheckDirective {
                var: "x".to_string(),
                asserts: vec![ResolvedCheckAssert::LatencyMax(ms(10))],
                // TASK-0099: hand-built test fixture has no source text.
                var_span: None,
            },
        )]
        .into_iter()
        .collect();
        let out = inject_check_frames(pw, &checks, &names);
        let w0 = &out[&WorkerId(0)];
        match &w0[0] {
            Event::Loop { check_frame, .. } => {
                assert_eq!(*check_frame, None, "unknown name does not inject");
            }
            other => panic!("expected Event::Loop, got {other:?}"),
        }
    }

    #[test]
    fn strip_mined_inner_loop_not_targeted() {
        // A `block_tag.is_some()` inner loop must NOT receive the
        // check_frame even when its `iter_var` matches: the user's V
        // names the SOURCE loop, the inner block tile is
        // implementation detail.
        use crate::event::BlockTag;
        let inner = Event::loop_over_tagged(
            IterVar(0),
            0..4,
            vec![fire()],
            BlockTag {
                block_n: 4,
                num_full: 2,
                is_partial: false,
            },
        );
        let pw: BTreeMap<WorkerId, Vec<Event>> =
            vec![(WorkerId(0), vec![inner])].into_iter().collect();
        let names: BTreeMap<String, IterVar> = vec![("n".to_string(), IterVar(0))]
            .into_iter()
            .collect();
        let checks: BTreeMap<String, ResolvedCheckDirective> = vec![(
            "n".to_string(),
            ResolvedCheckDirective {
                var: "n".to_string(),
                asserts: vec![ResolvedCheckAssert::LatencyMax(ms(10))],
                // TASK-0099: hand-built test fixture has no source text.
                var_span: None,
            },
        )]
        .into_iter()
        .collect();
        let out = inject_check_frames(pw, &checks, &names);
        let w0 = &out[&WorkerId(0)];
        match &w0[0] {
            Event::Loop {
                block_tag,
                check_frame,
                ..
            } => {
                assert!(block_tag.is_some(), "inner loop kept its block_tag");
                assert_eq!(
                    *check_frame, None,
                    "strip-mined inner loop must NOT carry check_frame"
                );
            }
            other => panic!("expected Event::Loop, got {other:?}"),
        }
    }

    #[test]
    fn explicit_on_violation_is_carried() {
        let pw: BTreeMap<WorkerId, Vec<Event>> = vec![(
            WorkerId(0),
            vec![Event::loop_over(IterVar(0), 0..10, vec![fire()])],
        )]
        .into_iter()
        .collect();
        let names: BTreeMap<String, IterVar> = vec![("n".to_string(), IterVar(0))]
            .into_iter()
            .collect();
        let checks: BTreeMap<String, ResolvedCheckDirective> = vec![(
            "n".to_string(),
            ResolvedCheckDirective {
                var: "n".to_string(),
                asserts: vec![
                    ResolvedCheckAssert::LatencyMax(ms(5)),
                    ResolvedCheckAssert::OnViolation(AstViolationKind::Log),
                ],
                // TASK-0099: hand-built test fixture has no source text.
                var_span: None,
            },
        )]
        .into_iter()
        .collect();
        let out = inject_check_frames(pw, &checks, &names);
        let w0 = &out[&WorkerId(0)];
        match &w0[0] {
            Event::Loop { check_frame, .. } => {
                let cf = check_frame.as_ref().expect("frame populated");
                assert_eq!(cf.on_violation, ViolationKind::Log);
                assert_eq!(cf.latency_max_ns, 5_000_000);
            }
            other => panic!("expected Event::Loop, got {other:?}"),
        }
    }

    #[test]
    fn nested_loop_with_inner_check_matches() {
        // `check loop inner : latency_max=1ms;` attaches to the inner
        // (but still source) loop even when nested inside another
        // source loop without a check.
        let inner = Event::loop_over(IterVar(1), 0..4, vec![fire()]);
        let outer = Event::loop_over(IterVar(0), 0..10, vec![inner]);
        let pw: BTreeMap<WorkerId, Vec<Event>> =
            vec![(WorkerId(0), vec![outer])].into_iter().collect();
        let names: BTreeMap<String, IterVar> = vec![
            ("outer".to_string(), IterVar(0)),
            ("inner".to_string(), IterVar(1)),
        ]
        .into_iter()
        .collect();
        let checks: BTreeMap<String, ResolvedCheckDirective> = vec![(
            "inner".to_string(),
            ResolvedCheckDirective {
                var: "inner".to_string(),
                asserts: vec![ResolvedCheckAssert::LatencyMax(TimeLit {
                    nanos: 1_000_000,
                    original_unit: TimeUnit::Ms,
                    original_value: 1,
                })],
                // TASK-0099: hand-built test fixture has no source text.
                var_span: None,
            },
        )]
        .into_iter()
        .collect();
        let out = inject_check_frames(pw, &checks, &names);
        let w0 = &out[&WorkerId(0)];
        // Outer: no check.
        match &w0[0] {
            Event::Loop {
                check_frame, body, ..
            } => {
                assert_eq!(*check_frame, None, "outer loop without a `check` directive");
                // Inner: check IS attached.
                match &body[0] {
                    Event::Loop { check_frame, .. } => {
                        let cf = check_frame.as_ref().expect("inner check_frame");
                        assert_eq!(cf.loop_var, "inner");
                        assert_eq!(cf.latency_max_ns, 1_000_000);
                    }
                    other => panic!("expected inner Event::Loop, got {other:?}"),
                }
            }
            other => panic!("expected outer Event::Loop, got {other:?}"),
        }
    }
}
