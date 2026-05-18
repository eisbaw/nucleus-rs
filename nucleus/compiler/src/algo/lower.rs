//! AST → AlgoIR lowering pass.
//!
//! See [`super::ir`] for the IR types and [`LowerError`] variants.
//!
//! # Algorithm
//!
//! Lowering proceeds in source order over [`AlgoAst::items`]:
//!
//! 1. `Item::Const` — evaluate the value expression as integer
//!    arithmetic. Recursive references to earlier consts are resolved
//!    by recursive evaluation through a visiting set (cycle detection
//!    even though declarations-before-use would normally prevent
//!    cycles — defence in depth).
//! 2. `Item::Data` — evaluate each shape dimension as an integer
//!    expression. Only consts may appear in shape dims.
//! 3. `Item::Kernel` — evaluate shape dims of every parameter and
//!    return type. Stash the resolved kernel.
//! 4. `Item::Stmt` — lower the statement, threading a `Scope`
//!    that tracks loop variables and assigned data symbols.
//!
//! Single-assignment is enforced per scope. The top-level program is
//! one scope; each `for` body is a nested scope. A nested scope
//! inherits enclosing assigned-set membership only for **read**
//! checking, not for assignment-prevention: assigning to `feat1` in
//! the top scope and then *again* inside a loop is still a double
//! assignment, because PRD §6.2.1 talks about single-assignment per
//! *data symbol* across its life — not literal lexical scope.
//!
//! In practice the way the existing examples are written, every data
//! symbol is assigned in exactly one place. We enforce that.
//!
//! Iteration-variable scoping is strictly lexical: a `for y` introduces
//! `y` for the body only; exit pops it. An out-of-scope reference is
//! an error.

use std::collections::{BTreeMap, HashSet};

use super::ast::{
    AlgoAst, BinOp, ConstDecl, DataDecl, Expr, IndexedLValue, Item, KernelDecl, Stmt, Type, UnaryOp,
};
use super::ir::{
    AlgoIR, IndexedRef, IrBinOp, IrExpr, IrStmt, LowerError, ResolvedConst, ResolvedData,
    ResolvedKernel, ResolvedType,
};

/// Lower a parsed [`AlgoAst`] into a validated [`AlgoIR`].
///
/// Errors are returned on the first violation encountered. Multi-error
/// reporting is a follow-up (parallels the parser's single-error
/// limitation).
pub fn lower_algo(ast: &AlgoAst) -> Result<AlgoIR, LowerError> {
    let mut ir = AlgoIR::default();

    // Top-level pass: declarations and statements in source order.
    // We process each item in order so a reference into an earlier
    // const / data / kernel resolves; references to later ones fail
    // (declarations-before-use, in line with the existing grammar
    // doc note that "semantic passes enforce declarations-before-use").
    let mut top_scope = Scope::new_top_level();

    for item in &ast.items {
        match item {
            Item::Const(c) => lower_const(c, &mut ir)?,
            Item::Data(d) => lower_data(d, &mut ir)?,
            Item::Kernel(k) => lower_kernel(k, &mut ir)?,
            Item::Stmt(s) => {
                let lowered = lower_stmt(s, &ir, &mut top_scope)?;
                ir.stmts.push(lowered);
            }
        }
    }

    Ok(ir)
}

// --------------------------------------------------------------------
// Declaration lowering
// --------------------------------------------------------------------

fn lower_const(c: &ConstDecl, ir: &mut AlgoIR) -> Result<(), LowerError> {
    if ir.consts.contains_key(&c.name) {
        return Err(LowerError::DuplicateConst(c.name.clone()));
    }
    // Other namespaces (data, kernel) may not collide either —
    // identifiers share one global symbol table at the algorithm level.
    if ir.data.contains_key(&c.name) || ir.kernels.contains_key(&c.name) {
        return Err(LowerError::DuplicateConst(c.name.clone()));
    }

    // Evaluate the value expression. The visitor stack starts with
    // this name so any self-reference is caught as a cycle.
    let mut visiting: Vec<String> = vec![c.name.clone()];
    let value = eval_const_expr(&c.value, &c.name, ir, &mut visiting)?;

    ir.consts.insert(
        c.name.clone(),
        ResolvedConst {
            name: c.name.clone(),
            ty: c.ty.clone(),
            value,
        },
    );
    Ok(())
}

fn lower_data(d: &DataDecl, ir: &mut AlgoIR) -> Result<(), LowerError> {
    if ir.data.contains_key(&d.name)
        || ir.consts.contains_key(&d.name)
        || ir.kernels.contains_key(&d.name)
    {
        return Err(LowerError::DuplicateData(d.name.clone()));
    }
    let ty = resolve_type(&d.ty, &d.name, ir)?;
    ir.data.insert(
        d.name.clone(),
        ResolvedData {
            name: d.name.clone(),
            ty,
        },
    );
    Ok(())
}

fn lower_kernel(k: &KernelDecl, ir: &mut AlgoIR) -> Result<(), LowerError> {
    if ir.kernels.contains_key(&k.name)
        || ir.consts.contains_key(&k.name)
        || ir.data.contains_key(&k.name)
    {
        return Err(LowerError::DuplicateKernel(k.name.clone()));
    }
    let params = k
        .sig
        .params
        .iter()
        .map(|t| resolve_type(t, &k.name, ir))
        .collect::<Result<Vec<_>, _>>()?;
    let ret = k
        .sig
        .ret
        .as_ref()
        .map(|t| resolve_type(t, &k.name, ir))
        .transpose()?;
    ir.kernels.insert(
        k.name.clone(),
        ResolvedKernel {
            name: k.name.clone(),
            params,
            ret,
            purity: k.purity,
        },
    );
    Ok(())
}

/// Resolve a [`Type`] (AST) into a [`ResolvedType`] by evaluating
/// every dimension expression. `decl_name` is the owning declaration's
/// name, used for error messages.
fn resolve_type(t: &Type, decl_name: &str, ir: &AlgoIR) -> Result<ResolvedType, LowerError> {
    let mut dims = Vec::with_capacity(t.dims.len());
    for dim_expr in &t.dims {
        let v = eval_shape_expr(dim_expr, decl_name, ir)?;
        if v <= 0 {
            return Err(LowerError::NonPositiveDim {
                decl: decl_name.to_string(),
                value: v,
            });
        }
        // Cast to usize: we just proved v > 0.
        dims.push(v as usize);
    }
    Ok(ResolvedType {
        scalar: t.scalar.clone(),
        dims,
    })
}

// --------------------------------------------------------------------
// Const evaluation
// --------------------------------------------------------------------

/// Evaluate a const expression. Permitted constructs:
/// - integer literal,
/// - unary minus,
/// - binary +/-/*///%,
/// - reference to a previously declared const,
/// - parentheses (implicit via expression structure).
///
/// Forbidden: kernel calls, data references, anything else.
///
/// `visiting` carries the chain of const names currently being
/// evaluated, for cycle detection. In the typical declarations-before-
/// use case this is never needed (a cycle would require a forward
/// reference), but we keep the check so a future relaxation of the
/// order rule doesn't introduce a silent infinite loop.
fn eval_const_expr(
    expr: &Expr,
    in_const: &str,
    ir: &AlgoIR,
    visiting: &mut Vec<String>,
) -> Result<i64, LowerError> {
    match expr {
        Expr::IntLit(n) => Ok(*n),
        Expr::Unary(UnaryOp::Neg, inner) => {
            let v = eval_const_expr(inner, in_const, ir, visiting)?;
            v.checked_neg().ok_or_else(|| LowerError::ConstOverflow {
                in_const: in_const.to_string(),
                op: "negate".into(),
            })
        }
        Expr::Binary(op, lhs, rhs) => {
            let l = eval_const_expr(lhs, in_const, ir, visiting)?;
            let r = eval_const_expr(rhs, in_const, ir, visiting)?;
            checked_binop(*op, l, r).map_err(|e| match e {
                BinopErr::Overflow(s) => LowerError::ConstOverflow {
                    in_const: in_const.to_string(),
                    op: s,
                },
                BinopErr::DivByZero => LowerError::ConstDivByZero {
                    in_const: in_const.to_string(),
                },
            })
        }
        Expr::Ident(name) => eval_const_ident(name, in_const, ir, visiting),
        Expr::Call(_) => Err(LowerError::NonIntegerConstExpr {
            in_const: in_const.to_string(),
            reason: "kernel calls are not allowed in const expressions".into(),
        }),
        Expr::LValue(lv) => {
            // The parser models a bare identifier as `LValue` with an
            // empty index list (see `parser.rs::ident_or_call`); only
            // *indexed* lvalues are data references proper.
            if lv.indices.is_empty() {
                eval_const_ident(&lv.name, in_const, ir, visiting)
            } else {
                Err(LowerError::NonIntegerConstExpr {
                    in_const: in_const.to_string(),
                    reason: "data references are not allowed in const expressions".into(),
                })
            }
        }
    }
}

fn eval_const_ident(
    name: &str,
    in_const: &str,
    ir: &AlgoIR,
    visiting: &mut Vec<String>,
) -> Result<i64, LowerError> {
    if visiting.iter().any(|n| n == name) {
        let mut path = visiting.clone();
        path.push(name.to_string());
        return Err(LowerError::ConstCycle(path));
    }
    let Some(c) = ir.consts.get(name) else {
        return Err(LowerError::ConstRefersToNonConst {
            in_const: in_const.to_string(),
            unknown_ident: name.to_string(),
        });
    };
    visiting.push(name.to_string());
    let v = c.value;
    visiting.pop();
    Ok(v)
}

/// Evaluate a shape dimension expression. Same constructs as a const
/// expression, with errors tagged for the owning declaration.
fn eval_shape_expr(expr: &Expr, decl: &str, ir: &AlgoIR) -> Result<i64, LowerError> {
    match expr {
        Expr::IntLit(n) => Ok(*n),
        Expr::Unary(UnaryOp::Neg, inner) => {
            let v = eval_shape_expr(inner, decl, ir)?;
            v.checked_neg().ok_or_else(|| LowerError::ShapeOverflow {
                decl: decl.to_string(),
                op: "negate".into(),
            })
        }
        Expr::Binary(op, lhs, rhs) => {
            let l = eval_shape_expr(lhs, decl, ir)?;
            let r = eval_shape_expr(rhs, decl, ir)?;
            checked_binop(*op, l, r).map_err(|e| match e {
                BinopErr::Overflow(s) => LowerError::ShapeOverflow {
                    decl: decl.to_string(),
                    op: s,
                },
                BinopErr::DivByZero => LowerError::ShapeDivByZero {
                    decl: decl.to_string(),
                },
            })
        }
        Expr::Ident(name) => eval_shape_ident(name, decl, ir),
        Expr::Call(_) => Err(LowerError::NonIntegerShapeExpr {
            decl: decl.to_string(),
            reason: "kernel calls are not allowed in shape dimensions".into(),
        }),
        Expr::LValue(lv) => {
            if lv.indices.is_empty() {
                eval_shape_ident(&lv.name, decl, ir)
            } else {
                Err(LowerError::NonIntegerShapeExpr {
                    decl: decl.to_string(),
                    reason: "data references are not allowed in shape dimensions".into(),
                })
            }
        }
    }
}

fn eval_shape_ident(name: &str, decl: &str, ir: &AlgoIR) -> Result<i64, LowerError> {
    let Some(c) = ir.consts.get(name) else {
        return Err(LowerError::ShapeRefersToNonConst {
            decl: decl.to_string(),
            unknown_ident: name.to_string(),
        });
    };
    Ok(c.value)
}

enum BinopErr {
    Overflow(String),
    DivByZero,
}

fn checked_binop(op: BinOp, lhs: i64, rhs: i64) -> Result<i64, BinopErr> {
    match op {
        BinOp::Add => lhs
            .checked_add(rhs)
            .ok_or_else(|| BinopErr::Overflow("add".into())),
        BinOp::Sub => lhs
            .checked_sub(rhs)
            .ok_or_else(|| BinopErr::Overflow("sub".into())),
        BinOp::Mul => lhs
            .checked_mul(rhs)
            .ok_or_else(|| BinopErr::Overflow("mul".into())),
        BinOp::Div => {
            if rhs == 0 {
                Err(BinopErr::DivByZero)
            } else {
                lhs.checked_div(rhs)
                    .ok_or_else(|| BinopErr::Overflow("div".into()))
            }
        }
        BinOp::Mod => {
            if rhs == 0 {
                Err(BinopErr::DivByZero)
            } else {
                lhs.checked_rem(rhs)
                    .ok_or_else(|| BinopErr::Overflow("mod".into()))
            }
        }
    }
}

// --------------------------------------------------------------------
// Statement lowering and scoping
// --------------------------------------------------------------------

/// Lexical scope tracking.
///
/// The scope chain is a stack of frames; each frame carries:
/// - `iter_vars`: iteration variables introduced at this frame
///   (top-level has none).
/// - `description`: a short human-readable tag used in
///   double-assignment error messages.
///
/// The set of assigned data symbols is **global** across the whole
/// program: PRD §6.2.1 single-assignment is per-symbol over the
/// program's lifetime, not per-lexical-scope. We track it on the
/// scope object so the lifetime matches the lowering pass.
struct Scope {
    /// Frames in stack order; index 0 is the top level.
    frames: Vec<Frame>,
    /// Globally assigned data names with the scope description that
    /// owned each assignment. Used to report informative errors.
    assigned: BTreeMap<String, String>,
    /// Names of every iteration variable introduced anywhere during
    /// this lowering pass, regardless of whether its frame is still
    /// live. Used to give a useful diagnostic when an iteration
    /// variable is referenced after its loop has lexically closed.
    seen_iter: HashSet<String>,
}

struct Frame {
    iter_vars: HashSet<String>,
    description: String,
}

impl Scope {
    fn new_top_level() -> Self {
        Self {
            frames: vec![Frame {
                iter_vars: HashSet::new(),
                description: "<top-level>".into(),
            }],
            assigned: BTreeMap::new(),
            seen_iter: HashSet::new(),
        }
    }

    fn current_description(&self) -> &str {
        self.frames
            .last()
            .expect("scope frame stack is never empty")
            .description
            .as_str()
    }

    /// Push a new loop frame with iteration variable `var`.
    fn push_loop(&mut self, var: String) {
        let mut iter_vars = HashSet::new();
        iter_vars.insert(var.clone());
        self.seen_iter.insert(var.clone());
        self.frames.push(Frame {
            iter_vars,
            description: format!("for {var}"),
        });
    }

    fn pop(&mut self) {
        self.frames.pop();
    }

    /// True if `name` is an iteration variable in the current scope
    /// chain.
    fn iter_var_in_scope(&self, name: &str) -> bool {
        self.frames.iter().any(|f| f.iter_vars.contains(name))
    }
}

fn lower_stmt(stmt: &Stmt, ir: &AlgoIR, scope: &mut Scope) -> Result<IrStmt, LowerError> {
    match stmt {
        Stmt::Dataflow { lhs, rhs } => {
            // The LHS must be a declared data symbol.
            if !ir.data.contains_key(&lhs.name) {
                // Distinguish "iter var on LHS" from "totally unknown"
                // for a clearer message: iteration variables and
                // consts are not assignable.
                if scope.iter_var_in_scope(&lhs.name)
                    || ir.consts.contains_key(&lhs.name)
                    || ir.kernels.contains_key(&lhs.name)
                {
                    return Err(LowerError::AssignmentTargetNotData(lhs.name.clone()));
                }
                return Err(LowerError::UnknownIdent(lhs.name.clone()));
            }

            // Single-assignment check. Record the assignment against
            // the data symbol; reject a re-assignment.
            if let Some(prev_scope) = scope.assigned.get(&lhs.name) {
                return Err(LowerError::DoubleAssignment {
                    data: lhs.name.clone(),
                    scope: prev_scope.clone(),
                });
            }
            scope
                .assigned
                .insert(lhs.name.clone(), scope.current_description().to_string());

            // Lower indices (must be valid expressions in the current
            // scope: iter vars + consts).
            let indices = lower_indices(&lhs.indices, ir, scope)?;
            let rhs_ir = lower_rvalue(rhs, ir, scope)?;
            Ok(IrStmt::Dataflow {
                lhs: IndexedRef {
                    name: lhs.name.clone(),
                    indices,
                },
                rhs: rhs_ir,
            })
        }
        Stmt::Effect(call) => {
            // Bare-call statement. The callee must be a declared
            // kernel; we don't enforce purity here (that's a later
            // pass), only that the name exists.
            if !ir.kernels.contains_key(&call.callee) {
                return Err(LowerError::UnknownIdent(call.callee.clone()));
            }
            let args = call
                .args
                .iter()
                .map(|a| lower_rvalue(a, ir, scope))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(IrStmt::Effect {
                callee: call.callee.clone(),
                args,
            })
        }
        Stmt::For { var, lo, hi, body } => {
            // Loop bounds are evaluated in the *enclosing* scope; the
            // iteration variable is only visible inside the body.
            //
            // We additionally reject shadowing a declared name with a
            // loop variable. PRD §6.2.3 allows shadowing in general,
            // but shadowing a const or data symbol with a loop
            // variable is a strong code smell — flag it. (If a real
            // example needs this, relax with a follow-up task.)
            if ir.consts.contains_key(var)
                || ir.data.contains_key(var)
                || ir.kernels.contains_key(var)
            {
                return Err(LowerError::IterVarShadowsDecl {
                    var: var.clone(),
                    shadows: var.clone(),
                });
            }
            // Loop bounds are read-time expressions: they can refer to
            // consts and to any enclosing iteration variables.
            let lo_ir = lower_index_expr(lo, ir, scope)?;
            let hi_ir = lower_index_expr(hi, ir, scope)?;

            scope.push_loop(var.clone());
            let result = (|| -> Result<Vec<IrStmt>, LowerError> {
                let mut body_ir = Vec::with_capacity(body.len());
                for s in body {
                    body_ir.push(lower_stmt(s, ir, scope)?);
                }
                Ok(body_ir)
            })();
            scope.pop();
            let body_ir = result?;
            Ok(IrStmt::For {
                var: var.clone(),
                lo: lo_ir,
                hi: hi_ir,
                body: body_ir,
            })
        }
    }
}

fn lower_indices(indices: &[Expr], ir: &AlgoIR, scope: &Scope) -> Result<Vec<IrExpr>, LowerError> {
    indices
        .iter()
        .map(|e| lower_index_expr(e, ir, scope))
        .collect()
}

/// Lower an expression appearing as an index, a loop bound, or an
/// argument-position integer-shaped expression. Allowed references:
/// integer literals, consts, iteration variables in scope, arithmetic,
/// and (for kernel arguments) data references and nested calls.
///
/// For index/loop-bound positions, calls and data-refs are rejected
/// (they would be runtime values, not iteration-space arithmetic).
fn lower_index_expr(expr: &Expr, ir: &AlgoIR, scope: &Scope) -> Result<IrExpr, LowerError> {
    match expr {
        Expr::IntLit(n) => Ok(IrExpr::IntLit(*n)),
        Expr::Unary(UnaryOp::Neg, inner) => {
            Ok(IrExpr::Neg(Box::new(lower_index_expr(inner, ir, scope)?)))
        }
        Expr::Binary(op, lhs, rhs) => Ok(IrExpr::BinOp(
            ast_binop_to_ir(*op),
            Box::new(lower_index_expr(lhs, ir, scope)?),
            Box::new(lower_index_expr(rhs, ir, scope)?),
        )),
        Expr::Ident(name) => resolve_ident(name, ir, scope),
        Expr::Call(_) => Err(LowerError::NonIntegerShapeExpr {
            decl: "<index/loop-bound expression>".into(),
            reason: "kernel calls are not allowed here".to_string(),
        }),
        Expr::LValue(lv) => {
            // Bare identifier (parser models it as zero-indexed
            // lvalue). At an index/loop-bound position, an indexed
            // data reference is illegal.
            if lv.indices.is_empty() {
                resolve_ident(&lv.name, ir, scope)
            } else {
                Err(LowerError::NonIntegerShapeExpr {
                    decl: "<index/loop-bound expression>".into(),
                    reason: "data references are not allowed here".to_string(),
                })
            }
        }
    }
}

/// Lower an expression appearing as an RHS / kernel argument. This is
/// the most permissive form: data refs and calls are legal, on top of
/// everything `lower_index_expr` allows.
fn lower_rvalue(expr: &Expr, ir: &AlgoIR, scope: &Scope) -> Result<IrExpr, LowerError> {
    match expr {
        Expr::IntLit(n) => Ok(IrExpr::IntLit(*n)),
        Expr::Unary(UnaryOp::Neg, inner) => {
            Ok(IrExpr::Neg(Box::new(lower_rvalue(inner, ir, scope)?)))
        }
        Expr::Binary(op, lhs, rhs) => Ok(IrExpr::BinOp(
            ast_binop_to_ir(*op),
            Box::new(lower_rvalue(lhs, ir, scope)?),
            Box::new(lower_rvalue(rhs, ir, scope)?),
        )),
        Expr::Ident(name) => resolve_ident(name, ir, scope),
        Expr::Call(c) => {
            if !ir.kernels.contains_key(&c.callee) {
                return Err(LowerError::UnknownIdent(c.callee.clone()));
            }
            let args = c
                .args
                .iter()
                .map(|a| lower_rvalue(a, ir, scope))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(IrExpr::Call {
                callee: c.callee.clone(),
                args,
            })
        }
        Expr::LValue(lv) => lower_data_ref(lv, ir, scope),
    }
}

fn lower_data_ref(lv: &IndexedLValue, ir: &AlgoIR, scope: &Scope) -> Result<IrExpr, LowerError> {
    // If indices are present, the base must be a data symbol. If no
    // indices, it could be a bare data reference (whole array copy)
    // or a scalar use of a const / iter var.
    if lv.indices.is_empty() {
        // Bare ident reuse path.
        return resolve_ident(&lv.name, ir, scope);
    }
    if !ir.data.contains_key(&lv.name) {
        return Err(LowerError::UnknownIdent(lv.name.clone()));
    }
    let indices = lv
        .indices
        .iter()
        .map(|e| lower_index_expr(e, ir, scope))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(IrExpr::DataRef(IndexedRef {
        name: lv.name.clone(),
        indices,
    }))
}

/// Resolve a bare identifier into an IR expression. The name must be
/// either a previously declared const, a data symbol (interpreted as
/// a whole-array reference — IR keeps the textual name on
/// [`IrExpr::DataRef`] with no indices), or an iteration variable
/// currently in scope.
///
/// Iteration variables out of scope produce a dedicated error so the
/// caller can distinguish "you misspelled" from "you used `y` after
/// its loop ended".
fn resolve_ident(name: &str, ir: &AlgoIR, scope: &Scope) -> Result<IrExpr, LowerError> {
    if scope.iter_var_in_scope(name) {
        return Ok(IrExpr::Ident(name.to_string()));
    }
    if ir.consts.contains_key(name) {
        return Ok(IrExpr::Ident(name.to_string()));
    }
    if ir.data.contains_key(name) {
        // Bare data reference; legal at the surface (identity copy).
        return Ok(IrExpr::DataRef(IndexedRef {
            name: name.to_string(),
            indices: vec![],
        }));
    }
    // Distinguish "iter var that has gone out of scope" from "totally
    // unknown" by checking whether *any* enclosing-or-completed loop
    // ever introduced this name. The Scope.frames stack only knows
    // about currently-live frames; once a loop pops, the variable
    // becomes indistinguishable from a misspelling.
    //
    // To preserve the more useful error, the lowering pass tracks
    // every iteration variable name it has *ever* seen at this depth,
    // separately from currently-live frames. We piggy-back on a
    // simple heuristic: if the name appears in `scope.assigned`-style
    // bookkeeping for past frames, that's a positive signal.
    //
    // Implementation: we keep a side set on `Scope` called `seen_iter`
    // that records names of all iteration variables introduced during
    // this lowering pass, regardless of whether their frame is still
    // alive. Populated in `push_loop`.
    if scope.was_iter_var(name) {
        return Err(LowerError::IterVarOutOfScope(name.to_string()));
    }
    Err(LowerError::UnknownIdent(name.to_string()))
}

fn ast_binop_to_ir(op: BinOp) -> IrBinOp {
    match op {
        BinOp::Add => IrBinOp::Add,
        BinOp::Sub => IrBinOp::Sub,
        BinOp::Mul => IrBinOp::Mul,
        BinOp::Div => IrBinOp::Div,
        BinOp::Mod => IrBinOp::Mod,
    }
}

impl Scope {
    /// Has `name` been the iteration variable of any (possibly
    /// already-popped) loop during this lowering pass?
    fn was_iter_var(&self, name: &str) -> bool {
        self.seen_iter.contains(name)
    }
}
