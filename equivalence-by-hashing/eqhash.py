"""
Equivalence-by-hashing — minimal sketch.

Core idea: hash AST nodes by *evaluating* the operator algebra over a large
integer ring, instead of hashing the syntactic tree structure. Commutativity,
associativity, distributivity, and identities then fall out for free — no
rewrite rules needed.

This is the §4.3.6.1 technique from the 2013 thesis, re-extracted as a
standalone artefact.

Caveats (state them, don't bury them):
  - Works for integer arithmetic and continuous bijective functions.
    NOT sound for floating point (associativity fails in IEEE 754).
    NOT applicable to lossy operators (comparisons, boolean ops).
  - "Equal hashes => equal expressions" is Monte Carlo, not proof.
    Collision probability per comparison ~ 2^-128 with SHA256.
    For safety in a real compiler: on a hash hit, do a structural recheck
    before treating it as a CSE/GVN opportunity.
  - Salt makes adversarial collisions require simultaneous collisions
    in two independent hashes. Cheap, keep it. Not a security claim.
"""

from __future__ import annotations
import hashlib
from dataclasses import dataclass
from typing import Union

# Work modulo a large prime so +,-,*,/ all live in one field.
# 2^521 - 1 (Mersenne prime). Comfortably larger than SHA256 output.
P = (1 << 521) - 1
SALT = b"nuc-eqhash-v0"


def _h(s: str, salt: bytes = b"") -> int:
    return int.from_bytes(hashlib.sha256(salt + s.encode()).digest(), "big") % P


# Two independent hashes per node — see "salting" discussion in the thesis.
# A malicious tree must collide in BOTH simultaneously.
Hash = tuple[int, int]


def _pair(s: str) -> Hash:
    return (_h(s), _h(s, SALT))


def _add(a: Hash, b: Hash) -> Hash:
    return ((a[0] + b[0]) % P, (a[1] + b[1]) % P)


def _sub(a: Hash, b: Hash) -> Hash:
    return ((a[0] - b[0]) % P, (a[1] - b[1]) % P)


def _mul(a: Hash, b: Hash) -> Hash:
    return ((a[0] * b[0]) % P, (a[1] * b[1]) % P)


def _neg(a: Hash) -> Hash:
    return ((-a[0]) % P, (-a[1]) % P)


# Minimal expression AST.
@dataclass(frozen=True)
class Const:
    n: int


@dataclass(frozen=True)
class Var:
    name: str


@dataclass(frozen=True)
class Bin:
    op: str  # '+', '-', '*'
    l: "Expr"
    r: "Expr"


@dataclass(frozen=True)
class Neg:
    x: "Expr"


Expr = Union[Const, Var, Bin, Neg]


def H(e: Expr) -> Hash:
    if isinstance(e, Const):
        return (e.n % P, e.n % P)
    if isinstance(e, Var):
        return _pair(e.name)
    if isinstance(e, Neg):
        return _neg(H(e.x))
    if isinstance(e, Bin):
        a, b = H(e.l), H(e.r)
        return {"+": _add, "-": _sub, "*": _mul}[e.op](a, b)
    raise TypeError(e)


def equiv(a: Expr, b: Expr) -> bool:
    """Monte Carlo equivalence. False => definitely unequal. True => almost
    certainly equal; recheck structurally if you need a proof."""
    return H(a) == H(b)


# --- Demo ---------------------------------------------------------------
if __name__ == "__main__":
    def v(n): return Var(n)
    def c(n): return Const(n)
    def add(*xs):
        r = xs[0]
        for x in xs[1:]:
            r = Bin("+", r, x)
        return r
    def mul(*xs):
        r = xs[0]
        for x in xs[1:]:
            r = Bin("*", r, x)
        return r
    sub = lambda a, b: Bin("-", a, b)
    neg = Neg

    a, b, cc, x, y, z = v("a"), v("b"), v("c"), v("x"), v("y"), v("z")

    cases = [
        # (lhs, rhs, expected)
        (add(c(1), c(1)),                  c(2),                                True),
        (add(mul(c(9), c(5)), c(2)),       c(47),                               True),  # 9*5+2=47
        (add(a, a),                        mul(c(2), a),                        True),
        (mul(x, add(x, y, z)),             add(mul(x, x), mul(x, y), mul(x, z)),True),
        (add(add(a, b), cc),               add(a, add(b, cc)),                  True),
        (add(y, x),                        add(x, y),                           True),
        (sub(a, a),                        c(0),                                True),
        (mul(c(1), a),                     a,                                   True),
        (a,                                b,                                   False),
        (add(a, b),                        sub(a, b),                           False),
    ]
    for lhs, rhs, expected in cases:
        got = equiv(lhs, rhs)
        tag = "ok " if got == expected else "FAIL"
        print(f"{tag}  equiv({lhs!s:40s}, {rhs!s:40s}) = {got}  (expected {expected})")
