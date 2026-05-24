//! Tests for `NameSidecar::reuse_widths` (TASK-0261, Stage 1) —
//! contract pin BEFORE Stage 2 (TASK-0265) wires a consumer.
//!
//! Pins three load-bearing invariants:
//!
//! 1. A 3x3 stencil algorithm whose schedule carries `loop x : reuse;`
//!    (single-host `reuse.sched.nuc`) produces a non-trivial entry on
//!    axis 1 (`x`) for `img_in`. The pipeline must populate the ACFG
//!    sidecar AND mirror onto the NameSidecar (the codegen contract
//!    surface) so the Stage 2 consumer has a path to read it.
//!
//! 2. The serde round-trip preserves `reuse_widths` byte-for-byte AND
//!    an older payload (synthesised by dropping the field) deserialises
//!    with `reuse_widths` defaulting to an empty map. Pins the additive
//!    contract before Stage 2 codegen consumes the JSON shape.
//!    The shape is a TRIPLE-NESTED `BTreeMap<IterVar, BTreeMap<DataId,
//!    BTreeMap<u64, ReuseSlot>>>`; the deep nest is load-bearing for
//!    serde-JSON (tuple keys would not round-trip). This test catches
//!    any future refactor that flattens the shape.
//!
//! 3. Defensive variant coverage: `ReuseInferenceError::UnknownLoopVar`
//!    and `UnknownDataInRef` are cross-pass invariant guards that fire
//!    only on an inconsistent `(LinkedIR, ACFG)` pair. Six of eight
//!    variants are covered in the unit tests; these two close the gap
//!    (cycle-82 review item #4).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use nucleus_compiler::{
    algo::{lower_algo, parse_algo},
    apply_block_transforms, apply_partition_blocks2d, apply_partition_rows,
    apply_partition_workers, apply_reuse_inference, build_acfg, build_sidecar, inject_syncs,
    inject_transfers, link,
    sched::{lower_sched, parse_sched},
    DataId, IterVar, ReuseInferenceError,
};

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("two ancestors above compiler crate")
}

/// Run the full lower-link-inject pipeline plus the strict reuse pass.
/// Mirrors `sidecar_halo.rs::lower` but routes through
/// `apply_reuse_inference` (strict) so a test fixture's reuse_widths is
/// populated as a Stage-2 consumer will see it.
fn lower_with_reuse(
    ex_rel: &str,
    sched_rel: &str,
) -> (nucleus_compiler::link::LinkedIR, nucleus_compiler::ACFG) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples").join(ex_rel);
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).expect("read algo");
    let sched_src = fs::read_to_string(ex.join(sched_rel)).expect("read sched");

    let algo_ir = lower_algo(&parse_algo(&algo_src).expect("parse_algo")).expect("lower_algo");
    let sched_ir =
        lower_sched(&parse_sched(&sched_src).expect("parse_sched")).expect("lower_sched");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block_transforms");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition_workers");
    let acfg = apply_partition_rows(&linked, acfg).expect("partition_rows");
    let acfg = apply_partition_blocks2d(&linked, acfg).expect("partition_blocks2d");
    let acfg = apply_reuse_inference(&linked, acfg).expect("reuse_inference");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    (linked, acfg)
}

#[test]
fn stencil_3x3_reuse_on_x_records_length_3_axis1_slot_on_img_in() {
    // Example 05 / 3x3 stencil, reuse.sched.nuc (single-host with
    // `loop x : reuse;` on the inner loop). The algorithm reads
    // `img_in[y±1][x-1]`, `img_in[y±1][x]`, `img_in[y±1][x+1]` — the
    // axis-1 (x) offset set for `img_in` is {-1, 0, +1}; reuse
    // inference records ReuseSlot { min_offset: -1, length: 3 } at
    // (x_iv, img_in, axis=1).
    //
    // The y-axis offsets {-1, 0, +1} are present too but `y` does NOT
    // carry reuse, so no entry under y_iv. This pins the "only the
    // tagged iv gets a slot" half of the contract.
    let (linked, acfg) = lower_with_reuse("05-stencil", "schedules/reuse.sched.nuc");

    let x_iv = *acfg.name_iter_vars.get("x").expect("x in ACFG");
    let y_iv = *acfg.name_iter_vars.get("y").expect("y in ACFG");
    let img_in = *acfg.name_data.get("img_in").expect("img_in in ACFG");

    // The reuse slot for (x_iv, img_in, axis=1).
    let slot = acfg
        .reuse_widths
        .get(&x_iv)
        .and_then(|m| m.get(&img_in))
        .and_then(|m| m.get(&1u64))
        .copied();
    assert_eq!(
        slot.map(|s| (s.min_offset, s.length)),
        Some((-1i64, 3u64)),
        "reuse_widths[x][img_in][axis=1] must be ReuseSlot{{min=-1,length=3}}; \
         got map {:?}",
        acfg.reuse_widths
    );

    // No entry under y_iv — y does not carry reuse.
    assert!(
        !acfg.reuse_widths.contains_key(&y_iv),
        "y does not carry reuse; reuse_widths must have no y_iv entry; got {:?}",
        acfg.reuse_widths
    );

    // The codegen-contract surface (NameSidecar) must mirror the ACFG
    // sidecar verbatim — Stage 2 reads off NameSidecar.
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert_eq!(
        sidecar.reuse_widths, acfg.reuse_widths,
        "NameSidecar.reuse_widths must mirror ACFG.reuse_widths"
    );
}

#[test]
fn naive_schedule_no_reuse_directive_empty_map() {
    // 05-stencil/naive carries NO reuse directive. The pipeline must
    // produce an EMPTY reuse_widths map — pins the additive-only
    // contract (existing examples remain byte-identical because no
    // consumer wired through, and the map stays empty when the user
    // doesn't ask for reuse).
    let (linked, acfg) = lower_with_reuse("05-stencil", "schedules/naive.sched.nuc");
    assert!(
        acfg.reuse_widths.is_empty(),
        "no reuse directive => empty reuse_widths; got {:?}",
        acfg.reuse_widths
    );
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert!(
        sidecar.reuse_widths.is_empty(),
        "NameSidecar must also have empty reuse_widths"
    );
}

#[cfg(feature = "serde")]
#[test]
fn reuse_widths_serde_roundtrip() {
    // Round-trip the NameSidecar through serde JSON; the triple-nested
    // reuse_widths map must survive byte-for-byte. The deep-nest shape
    // (`BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64, ReuseSlot>>>`)
    // is load-bearing for the JSON wire — a tuple-keyed flat map cannot
    // round-trip. This test would catch any future flattening regression
    // BEFORE Stage 2 codegen consumes the format (cycle-82 review #3).
    let (linked, acfg) = lower_with_reuse("05-stencil", "schedules/reuse.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    assert!(
        !sidecar.reuse_widths.is_empty(),
        "fixture must produce non-trivial reuse_widths for the round-trip test"
    );
    let json = serde_json::to_string(&sidecar).expect("serialise NameSidecar");
    let back: nucleus_compiler::NameSidecar =
        serde_json::from_str(&json).expect("deserialise NameSidecar");
    assert_eq!(
        back.reuse_widths, sidecar.reuse_widths,
        "reuse_widths must survive serde JSON round-trip"
    );

    // Pin the SHAPE explicitly: top-level key is the IterVar id (as a
    // string-quoted number — serde_json serialises BTreeMap<u64-newtype,
    // _> keys as strings). The middle key is DataId; the inner is the
    // axis u64. Three levels of object nesting.
    let value: serde_json::Value =
        serde_json::to_value(&sidecar.reuse_widths).expect("serialise reuse_widths");
    let outer = value.as_object().expect("top-level object");
    assert!(
        !outer.is_empty(),
        "reuse_widths must serialise as a non-empty JSON object"
    );
    let (_iv_key, mid_val) = outer.iter().next().expect("at least one iv entry");
    let mid = mid_val.as_object().expect("DataId-keyed inner object");
    let (_data_key, inner_val) = mid.iter().next().expect("at least one data entry");
    let inner = inner_val.as_object().expect("axis-keyed innermost object");
    let (_axis_key, slot_val) = inner.iter().next().expect("at least one axis entry");
    let slot = slot_val.as_object().expect("ReuseSlot as JSON object");
    assert!(
        slot.contains_key("min_offset"),
        "ReuseSlot JSON shape must carry `min_offset`"
    );
    assert!(
        slot.contains_key("length"),
        "ReuseSlot JSON shape must carry `length`"
    );
}

#[cfg(feature = "serde")]
#[test]
fn reuse_widths_serde_default_on_missing_field() {
    // An "old" wire payload that omits the `reuse_widths` field must
    // deserialise to an empty map (additive backward-compat contract,
    // mirroring `halo_widths` / `transfer_buffer_for_seq` /
    // `partition_worker_ranges`). Synthesise the "old" payload by
    // round-tripping a real NameSidecar through JSON and stripping the
    // `reuse_widths` key — every OTHER field stays present, so the test
    // isolates the additive claim to the new field alone.
    let (linked, acfg) = lower_with_reuse("05-stencil", "schedules/naive.sched.nuc");
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let value: serde_json::Value =
        serde_json::to_value(&sidecar).expect("serialise NameSidecar to Value");
    let mut obj = value
        .as_object()
        .expect("NameSidecar serialises to JSON object")
        .clone();
    obj.remove("reuse_widths");
    let pruned = serde_json::Value::Object(obj);
    let stripped_json = serde_json::to_string(&pruned).expect("re-serialise");
    let back: nucleus_compiler::NameSidecar = serde_json::from_str(&stripped_json)
        .expect("payload without reuse_widths must deserialise");
    assert!(
        back.reuse_widths.is_empty(),
        "missing reuse_widths field must default to empty map"
    );
}

// ----------------------------------------------------------------------
// Defensive-variant coverage (cycle-82 review #4)
// ----------------------------------------------------------------------
//
// `UnknownLoopVar` and `UnknownDataInRef` are link-pass invariant
// guards: a link-valid (LinkedIR, ACFG) pair cannot trip them, so the
// inference unit-test fixtures that go via `link()` cannot cover them.
// The tests below construct the inconsistent pair by hand — bypassing
// `link()` for the iv-name half (UnknownLoopVar) or the data-name half
// (UnknownDataInRef) — and exercise the strict entry point so the
// defensive `Err` arm fires.

#[test]
fn defensive_unknown_loop_var_returns_typed_err() {
    // Synthesise an inconsistent (LinkedIR, ACFG) pair: link a body
    // that uses iv `j` (so `name_iter_vars` carries `j`), then
    // hand-edit the schedule's reuse directive to name `i` (which the
    // ACFG `name_iter_vars` does NOT contain). The strict entry point
    // must return `UnknownLoopVar { var: "i" }`.
    use nucleus_compiler::algo::{
        AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, Purity, ResolvedData, ResolvedKernel,
        ResolvedType, ScalarType,
    };
    use nucleus_compiler::sched::{
        ResolvedLoopDirective, ResolvedLoopOption, ResolvedPlaceTarget, ResolvedPlacement,
        ResolvedWorker, SchedIR, DEFAULT_WORKER_CLASS,
    };

    // Build a valid algo + sched whose loop var is `j`.
    let mut data = BTreeMap::new();
    data.insert(
        "grid".to_string(),
        ResolvedData {
            name: "grid".to_string(),
            ty: ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16],
            },
        },
    );
    data.insert(
        "out".to_string(),
        ResolvedData {
            name: "out".to_string(),
            ty: ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16],
            },
        },
    );
    let mut kernels = BTreeMap::new();
    kernels.insert(
        "K".to_string(),
        ResolvedKernel {
            name: "K".to_string(),
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
            purity: Purity::Pure,
            name_span: None,
        },
    );
    let stmts = vec![IrStmt::For {
        var: "j".to_string(),
        lo: IrExpr::IntLit(1),
        hi: IrExpr::IntLit(15),
        body: vec![IrStmt::Dataflow {
            lhs: IndexedRef {
                name: "out".to_string(),
                indices: vec![IrExpr::Ident("j".to_string())],
            },
            rhs: IrExpr::Call {
                callee: "K".to_string(),
                args: vec![IrExpr::DataRef(IndexedRef {
                    name: "grid".to_string(),
                    indices: vec![IrExpr::BinOp(
                        IrBinOp::Sub,
                        Box::new(IrExpr::Ident("j".to_string())),
                        Box::new(IrExpr::IntLit(1)),
                    )],
                })],
            },
        }],
    }];
    let algo = AlgoIR {
        consts: BTreeMap::new(),
        data,
        kernels,
        stmts,
    };

    let mut workers: BTreeMap<String, ResolvedWorker> = BTreeMap::new();
    workers.insert(
        "w0".to_string(),
        ResolvedWorker {
            name: "w0".to_string(),
            class: DEFAULT_WORKER_CLASS.to_string(),
        },
    );
    let mut places: BTreeMap<String, ResolvedPlacement> = BTreeMap::new();
    places.insert(
        "K".to_string(),
        ResolvedPlacement {
            kernel: "K".to_string(),
            target: ResolvedPlaceTarget::One("w0".to_string()),
            kernel_span: None,
        },
    );
    // The sched carries reuse on `j` (valid). We will EDIT
    // `linked.sched.loops` post-link to swap the key to `i` so the
    // (LinkedIR, ACFG) pair is inconsistent — exactly the link-pass
    // invariant violation the variant defends against.
    let mut loops: BTreeMap<String, ResolvedLoopDirective> = BTreeMap::new();
    loops.insert(
        "j".to_string(),
        ResolvedLoopDirective {
            var: "j".to_string(),
            options: vec![ResolvedLoopOption::Reuse],
            var_span: None,
        },
    );
    let sched = SchedIR {
        algo_path: String::new(),
        worker_classes: BTreeMap::new(),
        memory_regions: BTreeMap::new(),
        workers,
        places,
        place_data: BTreeMap::new(),
        loops,
        transfers: BTreeMap::new(),
        checks: BTreeMap::new(),
    };
    let mut linked = link(algo, sched).expect("link must succeed for the j-form base fixture");
    // Build ACFG from the link-valid pair (its name_iter_vars carries
    // `j` only — NOT `i`).
    let acfg = build_acfg(&linked).expect("acfg build");

    // Now poison the sched: swap the reuse directive key from `j` to
    // `i`. `i` is NOT in ACFG.name_iter_vars, so the strict pass must
    // surface UnknownLoopVar.
    linked.sched.loops.clear();
    linked.sched.loops.insert(
        "i".to_string(),
        ResolvedLoopDirective {
            var: "i".to_string(),
            options: vec![ResolvedLoopOption::Reuse],
            var_span: None,
        },
    );

    let err = apply_reuse_inference(&linked, acfg).unwrap_err();
    match err {
        ReuseInferenceError::UnknownLoopVar { var } => {
            assert_eq!(var, "i", "UnknownLoopVar must name the missing iv");
        }
        other => panic!("expected UnknownLoopVar, got {other:?}"),
    }
}

#[test]
fn defensive_unknown_data_in_ref_returns_typed_err() {
    // Synthesise an inconsistent (LinkedIR, ACFG) pair: link a body
    // that reads `grid` (so `name_data` carries `grid`), then
    // hand-mutate the algo body's DataRef name to `phantom` (not in
    // name_data) and run the strict pass on the ORIGINAL ACFG. The
    // strict pass must return UnknownDataInRef.
    use nucleus_compiler::algo::{
        AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, Purity, ResolvedData, ResolvedKernel,
        ResolvedType, ScalarType,
    };
    use nucleus_compiler::sched::{
        ResolvedLoopDirective, ResolvedLoopOption, ResolvedPlaceTarget, ResolvedPlacement,
        ResolvedWorker, SchedIR, DEFAULT_WORKER_CLASS,
    };

    let mut data = BTreeMap::new();
    data.insert(
        "grid".to_string(),
        ResolvedData {
            name: "grid".to_string(),
            ty: ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16],
            },
        },
    );
    data.insert(
        "out".to_string(),
        ResolvedData {
            name: "out".to_string(),
            ty: ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![16],
            },
        },
    );
    let mut kernels = BTreeMap::new();
    kernels.insert(
        "K".to_string(),
        ResolvedKernel {
            name: "K".to_string(),
            params: vec![ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }],
            ret: Some(ResolvedType {
                scalar: ScalarType::I32,
                dims: vec![],
            }),
            purity: Purity::Pure,
            name_span: None,
        },
    );
    let stmts = vec![IrStmt::For {
        var: "i".to_string(),
        lo: IrExpr::IntLit(1),
        hi: IrExpr::IntLit(15),
        body: vec![IrStmt::Dataflow {
            lhs: IndexedRef {
                name: "out".to_string(),
                indices: vec![IrExpr::Ident("i".to_string())],
            },
            rhs: IrExpr::Call {
                callee: "K".to_string(),
                args: vec![IrExpr::DataRef(IndexedRef {
                    name: "grid".to_string(),
                    indices: vec![IrExpr::BinOp(
                        IrBinOp::Sub,
                        Box::new(IrExpr::Ident("i".to_string())),
                        Box::new(IrExpr::IntLit(1)),
                    )],
                })],
            },
        }],
    }];
    let algo = AlgoIR {
        consts: BTreeMap::new(),
        data,
        kernels,
        stmts,
    };
    let mut workers: BTreeMap<String, ResolvedWorker> = BTreeMap::new();
    workers.insert(
        "w0".to_string(),
        ResolvedWorker {
            name: "w0".to_string(),
            class: DEFAULT_WORKER_CLASS.to_string(),
        },
    );
    let mut places: BTreeMap<String, ResolvedPlacement> = BTreeMap::new();
    places.insert(
        "K".to_string(),
        ResolvedPlacement {
            kernel: "K".to_string(),
            target: ResolvedPlaceTarget::One("w0".to_string()),
            kernel_span: None,
        },
    );
    let mut loops: BTreeMap<String, ResolvedLoopDirective> = BTreeMap::new();
    loops.insert(
        "i".to_string(),
        ResolvedLoopDirective {
            var: "i".to_string(),
            options: vec![ResolvedLoopOption::Reuse],
            var_span: None,
        },
    );
    let sched = SchedIR {
        algo_path: String::new(),
        worker_classes: BTreeMap::new(),
        memory_regions: BTreeMap::new(),
        workers,
        places,
        place_data: BTreeMap::new(),
        loops,
        transfers: BTreeMap::new(),
        checks: BTreeMap::new(),
    };
    let mut linked = link(algo, sched).expect("link must succeed for the grid base fixture");
    let acfg = build_acfg(&linked).expect("acfg build");

    // Now poison the algo body: rewrite the inner DataRef name `grid`
    // -> `phantom`. The original ACFG `name_data` has `grid` + `out`
    // but NOT `phantom`, so the visitor surfaces UnknownDataInRef.
    fn rename_ref(e: &mut IrExpr) {
        match e {
            IrExpr::DataRef(r) => {
                if r.name == "grid" {
                    r.name = "phantom".to_string();
                }
                for ix in &mut r.indices {
                    rename_ref(ix);
                }
            }
            IrExpr::Call { args, .. } => {
                for a in args {
                    rename_ref(a);
                }
            }
            IrExpr::Neg(inner) => rename_ref(inner),
            IrExpr::BinOp(_, l, r) => {
                rename_ref(l);
                rename_ref(r);
            }
            IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
        }
    }
    fn rename_stmt(s: &mut IrStmt) {
        match s {
            IrStmt::Dataflow { rhs, .. } => rename_ref(rhs),
            IrStmt::Effect { args, .. } => {
                for a in args {
                    rename_ref(a);
                }
            }
            IrStmt::For { body, .. } => {
                for s2 in body {
                    rename_stmt(s2);
                }
            }
        }
    }
    for s in &mut linked.algo.stmts {
        rename_stmt(s);
    }

    let err = apply_reuse_inference(&linked, acfg).unwrap_err();
    match err {
        ReuseInferenceError::UnknownDataInRef { ref_name } => {
            assert_eq!(
                ref_name, "phantom",
                "UnknownDataInRef must name the missing data symbol"
            );
        }
        other => panic!("expected UnknownDataInRef, got {other:?}"),
    }
}

// ----------------------------------------------------------------------
// IterVar / DataId imports
// ----------------------------------------------------------------------
//
// Re-export the types we use in assert types — keeping the imports
// near the test so a reader doesn't have to chase `use` paths to
// confirm the keys are the public newtypes the codegen contract
// pins.
#[allow(dead_code)]
fn _type_pin(_iv: IterVar, _d: DataId) {}
