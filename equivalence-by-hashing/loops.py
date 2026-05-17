"""
Equivalence-by-hashing — Loops.

Tier 1 from the discussion: extend the arithmetic (Z/p, Schwartz-Zippel) case
to for-loops by:

  1. Hashing the body with the iter variable bound to a depth-indexed
     canonical symbol  ->  alpha-renaming falls out for free.

  2. Rebasing the iter space to start at 0 by substituting (iter_var + lo)
     for iter_var in the body  ->  iter-space translation equivalences
     fall out for free (because the body's arithmetic hash already handles
     simplification like ($+5)-4 == $+1).

What this catches (demonstrated below):
  - Alpha-renaming:        for i {f(i)}   ==   for j {f(j)}
  - Body equivalence:      for i {i+i}    ==   for i {2*i}
  - Iter-space shift:      for i:1..N {i} ==   for j:0..N-1 {j+1}
  - Symmetric shifts:      for i:5..N+4 {f(i-4)} == for j:1..N {f(j)}
  - Nested alpha-renaming: for i { for j {f(i,j)} } == for a { for b {f(a,b)} }

What this does NOT catch (different tool needed):
  - Loop fusion / fission / interchange / tiling  -> use polyhedral analysis.
  - Direction-flip equivalence of commutative folds (Fold abstraction would
    add a tag for "this body is a commutative-associative reduction"; that
    would let forward and backward iter spaces hash equal, but the source
    representation already has to opt in. Not implemented here.)
  - Equivalence between a loop and its closed form (sum_{i=1..N} i = N(N+1)/2)
    -> would require Faulhaber / symbolic summation, separate machinery.
"""

from __future__ import annotations
import hashlib
from dataclasses import dataclass
from typing import Union

P = (1 << 521) - 1
SALT = b"nuc-eqhash-loops-v0"


def _h(s: str, salt: bytes = b"") -> int:
    return int.from_bytes(hashlib.sha256(salt + s.encode()).digest(), "big") % P


Hash = tuple[int, int]


# --- AST ---------------------------------------------------------------
@dataclass(frozen=True)
class Const:
    n: int

@dataclass(frozen=True)
class Var:
    name: str

@dataclass(frozen=True)
class IterVar:
    """A reference to an iteration variable. Bound by an enclosing Loop."""
    name: str

@dataclass(frozen=True)
class Bin:
    op: str       # '+', '-', '*'
    l: "Expr"
    r: "Expr"

@dataclass(frozen=True)
class Neg:
    x: "Expr"

@dataclass(frozen=True)
class Call:
    """Uninterpreted function application — opaque to hashing except by name+args."""
    f: str
    args: tuple

@dataclass(frozen=True)
class Loop:
    var: str
    lo: "Expr"
    hi: "Expr"        # inclusive
    body: "Expr"

Expr = Union[Const, Var, IterVar, Bin, Neg, Call, Loop]


# --- Substitution ------------------------------------------------------
def subst(e: Expr, name: str, repl: Expr) -> Expr:
    """Capture-avoiding substitution of IterVar(name) := repl. Loop binders
    shadow correctly (a loop reusing `name` blocks substitution under it)."""
    if isinstance(e, (Const, Var)):
        return e
    if isinstance(e, IterVar):
        return repl if e.name == name else e
    if isinstance(e, Neg):
        return Neg(subst(e.x, name, repl))
    if isinstance(e, Bin):
        return Bin(e.op, subst(e.l, name, repl), subst(e.r, name, repl))
    if isinstance(e, Call):
        return Call(e.f, tuple(subst(a, name, repl) for a in e.args))
    if isinstance(e, Loop):
        if e.var == name:
            # `name` is shadowed inside this loop body; only rewrite bounds.
            return Loop(e.var, subst(e.lo, name, repl), subst(e.hi, name, repl), e.body)
        return Loop(e.var,
                    subst(e.lo, name, repl),
                    subst(e.hi, name, repl),
                    subst(e.body, name, repl))
    raise TypeError(e)


# --- Hashing -----------------------------------------------------------
def _add(a: Hash, b: Hash) -> Hash: return ((a[0] + b[0]) % P, (a[1] + b[1]) % P)
def _sub(a: Hash, b: Hash) -> Hash: return ((a[0] - b[0]) % P, (a[1] - b[1]) % P)
def _mul(a: Hash, b: Hash) -> Hash: return ((a[0] * b[0]) % P, (a[1] * b[1]) % P)
def _neg(a: Hash)          -> Hash: return ((-a[0]) % P, (-a[1]) % P)


def H(e: Expr, env: dict[str, Hash] | None = None, depth: int = 0) -> Hash:
    if env is None:
        env = {}

    if isinstance(e, Const):
        return (e.n % P, e.n % P)
    if isinstance(e, Var):
        return (_h(e.name), _h(e.name, SALT))
    if isinstance(e, IterVar):
        # Bound by an enclosing loop -> use the canonical hash from env.
        # Free -> hash by name (unusual but well-defined).
        return env.get(e.name, (_h("free:" + e.name), _h("free:" + e.name, SALT)))
    if isinstance(e, Neg):
        return _neg(H(e.x, env, depth))
    if isinstance(e, Bin):
        a, b = H(e.l, env, depth), H(e.r, env, depth)
        return {'+': _add, '-': _sub, '*': _mul}[e.op](a, b)
    if isinstance(e, Call):
        # Uninterpreted: hash positionally with the function name as a salt.
        h0 = _h("call:" + e.f)
        h1 = _h("call:" + e.f, SALT)
        for i, a in enumerate(e.args):
            ah = H(a, env, depth)
            # Position-sensitive mixing — argument order matters.
            h0 = (h0 + (i + 1) * ah[0]) % P
            h1 = (h1 + (i + 1) * ah[1]) % P
        return (h0, h1)
    if isinstance(e, Loop):
        # Step 1: rename the user's iter var to a depth-indexed canonical
        # name. This kills alpha-renaming differences and avoids env clashes
        # under nesting (outer "i", inner "i" both bind cleanly to $0, $1).
        canonical = f"${depth}"

        # Step 2: rebase the body so the iter space starts at 0.
        # Replace user's iter_var with (canonical + lo) throughout body,
        # and the new range becomes [0 .. hi-lo].
        rebased_body = subst(e.body, e.var, Bin('+', IterVar(canonical), e.lo))
        new_hi = Bin('-', e.hi, e.lo)

        # Step 3: bind the canonical iter var to a fresh hash symbol in env.
        canon_hash = (_h("iter:" + canonical), _h("iter:" + canonical, SALT))
        new_env = {**env, canonical: canon_hash}

        body_h = H(rebased_body, new_env, depth + 1)
        hi_h   = H(new_hi, env, depth)

        # Step 4: combine. "LOOP" marker prevents collision with non-loop
        # exprs that happen to hash similarly. Coefficients are arbitrary
        # but fixed (any injective combination works).
        marker = _h(f"LOOP@{depth}")
        return ((marker + 17 * hi_h[0] + 23 * body_h[0]) % P,
                (marker + 17 * hi_h[1] + 23 * body_h[1]) % P)
    raise TypeError(e)


def equiv(a: Expr, b: Expr) -> bool:
    return H(a) == H(b)


# --- Demo --------------------------------------------------------------
if __name__ == "__main__":
    def i(name="i"): return IterVar(name)
    def v(n): return Var(n)
    def c(n): return Const(n)
    def add(a, b): return Bin('+', a, b)
    def sub(a, b): return Bin('-', a, b)
    def mul(a, b): return Bin('*', a, b)
    def call(f, *args): return Call(f, tuple(args))

    N = v("N")
    a_arr = lambda idx: call("a", idx)   # array a[idx] as opaque call
    f     = lambda x: call("f", x)
    g     = lambda x, y: call("g", x, y)

    cases = [
        # ---------- alpha-renaming ----------
        ("alpha-rename: i vs j",
            Loop("i", c(0), N, i("i")),
            Loop("j", c(0), N, i("j")),
            True),

        ("alpha-rename: f(i) vs f(j)",
            Loop("i", c(0), N, f(i("i"))),
            Loop("j", c(0), N, f(i("j"))),
            True),

        # ---------- body equivalence (arithmetic carries through) ----------
        ("body: i+i  ==  2*i",
            Loop("i", c(0), N, add(i("i"), i("i"))),
            Loop("i", c(0), N, mul(c(2), i("i"))),
            True),

        ("body: (i+1)*(i+1)  ==  i*i + 2*i + 1",
            Loop("i", c(0), N, mul(add(i("i"), c(1)), add(i("i"), c(1)))),
            Loop("i", c(0), N, add(add(mul(i("i"), i("i")), mul(c(2), i("i"))), c(1))),
            True),

        # ---------- iter-space rebasing ----------
        ("rebase: for i:1..N {i}  ==  for j:0..N-1 {j+1}",
            Loop("i", c(1), N, i("i")),
            Loop("j", c(0), sub(N, c(1)), add(i("j"), c(1))),
            True),

        ("rebase: for i:5..N+4 {f(i-4)}  ==  for j:1..N {f(j)}",
            Loop("i", c(5), add(N, c(4)), f(sub(i("i"), c(4)))),
            Loop("j", c(1), N,            f(i("j"))),
            True),

        ("rebase: for i:0..N {a[i]}  ==  for j:1..N+1 {a[j-1]}",
            Loop("i", c(0), N,            a_arr(i("i"))),
            Loop("j", c(1), add(N, c(1)), a_arr(sub(i("j"), c(1)))),
            True),

        # ---------- nested loops ----------
        ("nested alpha-rename: ij  ==  ab",
            Loop("i", c(0), N, Loop("j", c(0), N, g(i("i"), i("j")))),
            Loop("a", c(0), N, Loop("b", c(0), N, g(i("a"), i("b")))),
            True),

        ("nested rebase: outer 1..N inner 1..N  ==  outer 0..N-1 inner 0..N-1 with +1",
            Loop("i", c(1), N,
                Loop("j", c(1), N, g(i("i"), i("j")))),
            Loop("i", c(0), sub(N, c(1)),
                Loop("j", c(0), sub(N, c(1)), g(add(i("i"), c(1)), add(i("j"), c(1))))),
            True),

        # ---------- negatives ----------
        ("neg: different iter space upper bound",
            Loop("i", c(0), N,            f(i("i"))),
            Loop("i", c(0), add(N, c(1)), f(i("i"))),
            False),

        ("neg: different body f vs g",
            Loop("i", c(0), N, f(i("i"))),
            Loop("i", c(0), N, call("g_one_arg", i("i"))),
            False),

        ("neg: a[i] vs a[i+1] -- index shift without bound shift",
            Loop("i", c(0), N, a_arr(i("i"))),
            Loop("i", c(0), N, a_arr(add(i("i"), c(1)))),
            False),

        ("neg: loop interchange is NOT detected (correctly flagged unequal)",
            Loop("i", c(0), N, Loop("j", c(0), N, g(i("i"), i("j")))),
            Loop("j", c(0), N, Loop("i", c(0), N, g(i("i"), i("j")))),
            False),  # honest: these are semantically equal for many g, but hashing alone won't catch it.
    ]

    width = max(len(name) for name, *_ in cases)
    fails = 0
    for name, lhs, rhs, expected in cases:
        got = equiv(lhs, rhs)
        tag = "ok  " if got == expected else "FAIL"
        if got != expected:
            fails += 1
        print(f"{tag}  {name:<{width}}   -> equiv={got}  (expected {expected})")
    print()
    print(f"{len(cases) - fails}/{len(cases)} passed")
