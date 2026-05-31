//! TASK-0341.02.02.01.03 cycle 213: pin the cumulative-array
//! discriminator that drives the COPY-not-accumulate combine for
//! 16-jacobi/distributed. The discriminator MUST classify jacobi's
//! cross-iteration `field` as cumulative while leaving histogram's
//! same-index read-modify accumulator NON-cumulative (so 08-histogram
//! / 05-stencil keep their accumulate / non-fan-in behaviour).

use super::collect_cumulative_data_names;
use crate::algo::{IndexedRef, IrExpr, IrStmt};
use std::collections::BTreeSet;

fn ident(s: &str) -> IrExpr {
    IrExpr::Ident(s.to_string())
}

/// jacobi shape: `field[t][y][x] <-- f(field[(t+ITERS)%(ITERS+1)][y-1][x], ...)`
/// nested inside `for t { for y { for x { ... } } }`. The self-read
/// dim-0 index differs from the LHS dim-0 index `t` ⇒ cumulative.
fn jacobi_like_stmts() -> Vec<IrStmt> {
    use crate::algo::IrBinOp::{Add, Mod, Sub};
    let prev_t = IrExpr::BinOp(
        Mod,
        Box::new(IrExpr::BinOp(
            Add,
            Box::new(ident("t")),
            Box::new(ident("ITERS")),
        )),
        Box::new(IrExpr::BinOp(
            Add,
            Box::new(ident("ITERS")),
            Box::new(IrExpr::IntLit(1)),
        )),
    );
    let self_read = IrExpr::DataRef(IndexedRef {
        name: "field".to_string(),
        indices: vec![
            prev_t,
            IrExpr::BinOp(Sub, Box::new(ident("y")), Box::new(IrExpr::IntLit(1))),
            ident("x"),
        ],
    });
    let rhs = IrExpr::Call {
        callee: "jacobi5_or_seed".to_string(),
        args: vec![self_read, ident("t")],
    };
    let df = IrStmt::Dataflow {
        lhs: IndexedRef {
            name: "field".to_string(),
            indices: vec![ident("t"), ident("y"), ident("x")],
        },
        rhs,
    };
    vec![IrStmt::For {
        var: "t".to_string(),
        lo: IrExpr::IntLit(0),
        hi: ident("ITERS"),
        body: vec![IrStmt::For {
            var: "y".to_string(),
            lo: IrExpr::IntLit(1),
            hi: ident("H"),
            body: vec![IrStmt::For {
                var: "x".to_string(),
                lo: IrExpr::IntLit(1),
                hi: ident("W"),
                body: vec![df],
            }],
        }],
    }]
}

/// histogram shape: `histogram[b] <-- bin_inc(histogram[b], input[i], b)`
/// nested inside `for i { ... }`. The self-read index `[b]` is
/// IDENTICAL to the LHS index ⇒ NOT cumulative (disjoint single-pass
/// accumulator — stays `wrapping_add` fan-in).
fn histogram_like_stmts() -> Vec<IrStmt> {
    let self_read = IrExpr::DataRef(IndexedRef {
        name: "histogram".to_string(),
        indices: vec![ident("b")],
    });
    let input_read = IrExpr::DataRef(IndexedRef {
        name: "input".to_string(),
        indices: vec![ident("i")],
    });
    let rhs = IrExpr::Call {
        callee: "bin_inc".to_string(),
        args: vec![self_read, input_read, ident("b")],
    };
    let df = IrStmt::Dataflow {
        lhs: IndexedRef {
            name: "histogram".to_string(),
            indices: vec![ident("b")],
        },
        rhs,
    };
    vec![IrStmt::For {
        var: "i".to_string(),
        lo: IrExpr::IntLit(0),
        hi: ident("N"),
        body: vec![df],
    }]
}

/// 05-stencil shape: `img_out[y][x] <-- blur3(img_in[y-1][x], ...)`.
/// img_out is NOT self-read (reads img_in, a different symbol) ⇒ NOT
/// cumulative (and not even an accumulator).
fn stencil_like_stmts() -> Vec<IrStmt> {
    use crate::algo::IrBinOp::Sub;
    let in_read = IrExpr::DataRef(IndexedRef {
        name: "img_in".to_string(),
        indices: vec![
            IrExpr::BinOp(Sub, Box::new(ident("y")), Box::new(IrExpr::IntLit(1))),
            ident("x"),
        ],
    });
    let df = IrStmt::Dataflow {
        lhs: IndexedRef {
            name: "img_out".to_string(),
            indices: vec![ident("y"), ident("x")],
        },
        rhs: IrExpr::Call {
            callee: "blur3".to_string(),
            args: vec![in_read],
        },
    };
    vec![IrStmt::For {
        var: "y".to_string(),
        lo: IrExpr::IntLit(1),
        hi: ident("H"),
        body: vec![IrStmt::For {
            var: "x".to_string(),
            lo: IrExpr::IntLit(1),
            hi: ident("W"),
            body: vec![df],
        }],
    }]
}

/// 11-game-of-life shape: `grid[t][i] <-- step_or_seed(grid[(t+ITERS)
/// %(ITERS+1)][(i+N-1)%N], ...)` nested inside `for t { for i { ... }
/// }`. The self-read dim-0 index `(t+ITERS)%(ITERS+1)` differs from
/// the LHS dim-0 index `t` ⇒ cumulative — STRUCTURALLY IDENTICAL to
/// jacobi (cycle-213 architect P2: the cumulative set is NOT
/// "16-jacobi only"). game-of-life ships no partitioned schedule, so
/// the classification is inert downstream — but it MUST be pinned so a
/// future `partition=` game-of-life schedule inherits the correct
/// COPY-not-accumulate behaviour rather than silently regressing.
fn game_of_life_like_stmts() -> Vec<IrStmt> {
    use crate::algo::IrBinOp::{Add, Mod};
    let prev_t = IrExpr::BinOp(
        Mod,
        Box::new(IrExpr::BinOp(
            Add,
            Box::new(ident("t")),
            Box::new(ident("ITERS")),
        )),
        Box::new(IrExpr::BinOp(
            Add,
            Box::new(ident("ITERS")),
            Box::new(IrExpr::IntLit(1)),
        )),
    );
    let self_read = IrExpr::DataRef(IndexedRef {
        name: "grid".to_string(),
        indices: vec![prev_t, ident("i")],
    });
    let rhs = IrExpr::Call {
        callee: "step_or_seed".to_string(),
        args: vec![self_read, ident("t")],
    };
    let df = IrStmt::Dataflow {
        lhs: IndexedRef {
            name: "grid".to_string(),
            indices: vec![ident("t"), ident("i")],
        },
        rhs,
    };
    vec![IrStmt::For {
        var: "t".to_string(),
        lo: IrExpr::IntLit(0),
        hi: ident("ITERS"),
        body: vec![IrStmt::For {
            var: "i".to_string(),
            lo: IrExpr::IntLit(0),
            hi: ident("N"),
            body: vec![df],
        }],
    }]
}

#[test]
fn jacobi_field_is_cumulative() {
    let mut out = BTreeSet::new();
    collect_cumulative_data_names(&jacobi_like_stmts(), &[], &mut out);
    assert!(
        out.contains("field"),
        "jacobi's cross-iteration `field` (self-read at a SHIFTED dim-0 index) \
         must be classified cumulative ⇒ COPY combine; got {out:?}"
    );
    assert_eq!(out.len(), 1, "only `field` should be cumulative; got {out:?}");
}

#[test]
fn histogram_accumulator_is_not_cumulative() {
    let mut out = BTreeSet::new();
    collect_cumulative_data_names(&histogram_like_stmts(), &[], &mut out);
    assert!(
        out.is_empty(),
        "histogram's same-index read-modify accumulator must NOT be classified \
         cumulative (it stays an accumulate fan-in); got {out:?}"
    );
}

#[test]
fn game_of_life_grid_is_cumulative() {
    // cycle-213 architect P2 pin: game-of-life's `grid` is the SECOND
    // cumulative symbol in the tree (the discriminator is not
    // jacobi-specific). Inert today (no partitioned game-of-life
    // schedule) but must classify correctly for a future one.
    let mut out = BTreeSet::new();
    collect_cumulative_data_names(&game_of_life_like_stmts(), &[], &mut out);
    assert!(
        out.contains("grid"),
        "game-of-life's cross-iteration `grid` (self-read at a SHIFTED dim-0 \
         index) must be classified cumulative, exactly like jacobi's `field`; \
         got {out:?}"
    );
    assert_eq!(out.len(), 1, "only `grid` should be cumulative; got {out:?}");
}

#[test]
fn stencil_output_is_not_cumulative() {
    let mut out = BTreeSet::new();
    collect_cumulative_data_names(&stencil_like_stmts(), &[], &mut out);
    assert!(
        out.is_empty(),
        "05-stencil's img_out (not self-read) must NOT be cumulative; got {out:?}"
    );
}

#[test]
fn top_level_self_read_outside_for_is_not_cumulative() {
    // A self-read at top level (no enclosing `for`) is NOT a
    // cross-iteration read — the enclosing-for guard excludes it.
    let df = IrStmt::Dataflow {
        lhs: IndexedRef {
            name: "acc".to_string(),
            indices: vec![IrExpr::IntLit(0)],
        },
        rhs: IrExpr::Call {
            callee: "f".to_string(),
            args: vec![IrExpr::DataRef(IndexedRef {
                name: "acc".to_string(),
                indices: vec![IrExpr::IntLit(1)],
            })],
        },
    };
    let mut out = BTreeSet::new();
    collect_cumulative_data_names(&[df], &[], &mut out);
    assert!(
        out.is_empty(),
        "a top-level (non-iterated) self-read must NOT be cumulative; got {out:?}"
    );
}
