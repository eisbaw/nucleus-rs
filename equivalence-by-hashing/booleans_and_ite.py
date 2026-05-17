"""
Equivalence-by-hashing — Booleans and conditionals.

Extends the §4.3.6.1 trick to Boolean algebra and ite(c, t, e).

Key choice: work in GF(2) with the multilinear constraint x_i^2 = x_i for
every Boolean variable. This is Algebraic Normal Form (ANF). Every Boolean
function has a UNIQUE ANF, so:

    ANF(f) == ANF(g)   iff   f and g are functionally equivalent.

Equivalence is therefore exact, not Monte Carlo — a strict upgrade over the
arithmetic case (which was Schwartz-Zippel and Monte Carlo).

Cost: ANF can be exponential in the number of variables in the worst case
(every monomial of 2^n possible). For circuit-style expressions where
fan-in stays small, it's tractable. For pathological cases (e.g. parity of
many variables expressed via OR-of-ANDs), it blows up. Same fundamental
limit BDDs have, different shape.

Representation:
  - Monomial = frozenset of variable names. Empty set = constant 1.
              (Multilinear because x*x = x, so each var appears 0 or 1 times.)
  - Poly    = frozenset of Monomials. Empty set = constant 0.
              (XOR semantics, so each monomial appears 0 or 1 times.)
"""

from __future__ import annotations
import hashlib

Monomial = frozenset      # frozenset[str]
Poly     = frozenset      # frozenset[Monomial]

ZERO: Poly = frozenset()
ONE:  Poly = frozenset([frozenset()])


def var(name: str) -> Poly:
    return frozenset([frozenset([name])])


def XOR(p: Poly, q: Poly) -> Poly:
    # In GF(2): 1+1 = 0, so cancelling monomials drop out.
    return p ^ q   # frozenset symmetric difference


def AND(p: Poly, q: Poly) -> Poly:
    # Polynomial multiplication with x*x -> x baked in (frozenset union of vars).
    counts: dict[Monomial, int] = {}
    for m1 in p:
        for m2 in q:
            m = m1 | m2
            counts[m] = counts.get(m, 0) ^ 1
    return frozenset(m for m, c in counts.items() if c == 1)


def NOT(p: Poly) -> Poly:
    return XOR(ONE, p)


def OR(p: Poly, q: Poly) -> Poly:
    # a OR b = a XOR b XOR (a AND b)
    return XOR(XOR(p, q), AND(p, q))


def ITE(c: Poly, t: Poly, e: Poly) -> Poly:
    # ite(c, t, e) = c*t + (1-c)*e
    # In GF(2): (1-c) = (1+c) = NOT c, so this is (c AND t) XOR (NOT c AND e).
    return XOR(AND(c, t), AND(NOT(c), e))


def H(p: Poly) -> str:
    """Canonical hash. Two equal polys hash equal *exactly* (not probabilistically)
    because the representation is already canonical. SHA256 is overkill but
    keeps the spirit of the original technique."""
    canonical = sorted(tuple(sorted(m)) for m in p)
    return hashlib.sha256(repr(canonical).encode()).hexdigest()[:16]


def equiv(p: Poly, q: Poly) -> bool:
    return H(p) == H(q)


# --- Demo ---------------------------------------------------------------
if __name__ == "__main__":
    a, b, c, d = var("a"), var("b"), var("c"), var("d")
    x, y = var("x"), var("y")

    def pretty(p: Poly) -> str:
        if not p: return "0"
        parts = []
        for m in sorted(p, key=lambda s: (len(s), sorted(s))):
            parts.append("1" if not m else "*".join(sorted(m)))
        return " + ".join(parts)

    cases = [
        # name,                              lhs,                                       rhs,                                       expected
        ("idempotence AND",                  AND(a, a),                                 a,                                          True),
        ("idempotence OR",                   OR(a, a),                                  a,                                          True),
        ("self-XOR is 0",                    XOR(a, a),                                 ZERO,                                       True),
        ("identity AND 1",                   AND(a, ONE),                               a,                                          True),
        ("annihilator AND 0",                AND(a, ZERO),                              ZERO,                                       True),
        ("identity OR 0",                    OR(a, ZERO),                               a,                                          True),
        ("annihilator OR 1",                 OR(a, ONE),                                ONE,                                        True),
        ("double negation",                  NOT(NOT(a)),                               a,                                          True),
        ("commutativity AND",                AND(a, b),                                 AND(b, a),                                  True),
        ("commutativity OR",                 OR(a, b),                                  OR(b, a),                                   True),
        ("associativity AND",                AND(AND(a, b), c),                         AND(a, AND(b, c)),                          True),
        ("associativity OR",                 OR(OR(a, b), c),                           OR(a, OR(b, c)),                            True),
        ("distributivity AND/OR",            AND(a, OR(b, c)),                          OR(AND(a, b), AND(a, c)),                   True),
        ("distributivity OR/AND",            OR(a, AND(b, c)),                          AND(OR(a, b), OR(a, c)),                    True),
        ("absorption a&(a|b)=a",             AND(a, OR(a, b)),                          a,                                          True),
        ("absorption a|(a&b)=a",             OR(a, AND(a, b)),                          a,                                          True),
        ("DeMorgan AND",                     NOT(AND(a, b)),                            OR(NOT(a), NOT(b)),                         True),
        ("DeMorgan OR",                      NOT(OR(a, b)),                             AND(NOT(a), NOT(b)),                        True),
        ("excluded middle",                  OR(a, NOT(a)),                             ONE,                                        True),
        ("contradiction",                    AND(a, NOT(a)),                            ZERO,                                       True),
        ("XOR via OR/AND/NOT",               XOR(a, b),                                 OR(AND(a, NOT(b)), AND(NOT(a), b)),         True),
        # ite identities
        ("ite(c,t,t) = t",                   ITE(c, x, x),                              x,                                          True),
        ("ite(1,t,e) = t",                   ITE(ONE, x, y),                            x,                                          True),
        ("ite(0,t,e) = e",                   ITE(ZERO, x, y),                           y,                                          True),
        ("ite(c,1,0) = c",                   ITE(c, ONE, ZERO),                         c,                                          True),
        ("ite(c,0,1) = ~c",                  ITE(c, ZERO, ONE),                         NOT(c),                                     True),
        # mux flattening (the headline result)
        ("mux flatten then-branch",          ITE(c, ITE(c, a, b), d),                   ITE(c, a, d),                               True),
        ("mux flatten else-branch",          ITE(c, a, ITE(c, b, d)),                   ITE(c, a, d),                               True),
        # ite distributes over AND/OR/XOR when conditions match
        ("ite distributes over AND",         AND(ITE(c, a, b), ITE(c, x, y)),           ITE(c, AND(a, x), AND(b, y)),               True),
        ("ite distributes over OR",          OR(ITE(c, a, b), ITE(c, x, y)),            ITE(c, OR(a, x), OR(b, y)),                 True),
        ("ite distributes over XOR",         XOR(ITE(c, a, b), ITE(c, x, y)),           ITE(c, XOR(a, x), XOR(b, y)),               True),
        # negatives — must NOT hash equal
        ("a != b",                           a,                                         b,                                          False),
        ("a&b != a|b",                       AND(a, b),                                 OR(a, b),                                   False),
        ("ite(c,a,b) != ite(c,b,a)",         ITE(c, a, b),                              ITE(c, b, a),                               False),
        # A non-trivial circuit equivalence:
        # majority(a,b,c) has two classical forms — they're equal.
        ("majority — two forms",             OR(OR(AND(a, b), AND(a, c)), AND(b, c)),
                                             OR(AND(a, b), AND(c, OR(a, b))),                                                       True),
    ]

    width = max(len(n) for n, *_ in cases)
    fails = 0
    for name, lhs, rhs, expected in cases:
        got = equiv(lhs, rhs)
        tag = "ok  " if got == expected else "FAIL"
        if got != expected:
            fails += 1
        print(f"{tag}  {name:<{width}}   lhs={pretty(lhs):<40s}  rhs={pretty(rhs):<40s}")
    print()
    print(f"{len(cases) - fails}/{len(cases)} passed")
