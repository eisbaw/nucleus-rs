# Algorithm sublanguage grammar (`*.algo.nuc`)

Status: descriptive EBNF (informative). No parser is auto-generated from
this file in v2. The reference parser (TASK-0006, TASK-0007) is
hand-written against this spec; conformance is asserted by behavioural
tests, not by grammar derivation.

Scope: this document fixes the surface syntax of the algorithm
sublanguage as described in
[PRD §6.2](../nuc-nucleus/PRD.md#62-algorithm-sublanguage). The schedule
sublanguage (`*.sched.nuc`) is documented separately and is explicitly
out of scope here (see §[What this grammar does not cover](#what-this-grammar-does-not-cover)).

## 1. EBNF

Notation:

- `'literal'` — terminal string.
- `A | B` — alternation.
- `A?` — zero or one.
- `A*` — zero or more.
- `A+` — one or more.
- `(A B)` — grouping.
- Identifiers in `UPPER_SNAKE` are nonterminals defined below.

```ebnf
(* ---------- top level ---------- *)

Program        ::= TopItem* ;

TopItem        ::= ConstDecl
                 | DataDecl
                 | KernelDecl
                 | Stmt ;     (* top-level statements are dataflow / for / call *)

(* ---------- declarations ---------- *)

ConstDecl      ::= 'const' Ident ':' ScalarType '=' ConstExpr ';' ;

DataDecl       ::= 'data' Ident ':' DataType ';' ;

KernelDecl     ::= 'kernel' Ident ':' KernelSig Purity ';' ;

KernelSig      ::= '(' KernelParamList? ')' '->' KernelRetType ;
KernelParamList::= KernelParamType (',' KernelParamType)* ;
KernelParamType::= DataType ;
KernelRetType  ::= DataType | '(' ')' ;

Purity         ::= 'pure' | 'effectful' ;

(* ---------- types ---------- *)

ScalarType     ::= 'usize'
                 | 'isize'
                 | 'u8' | 'u16' | 'u32' | 'u64'
                 | 'i8' | 'i16' | 'i32' | 'i64'
                 | 'f32' | 'f64'
                 | 'bool' ;

DataType       ::= ScalarType DimList? ;
DimList        ::= ('[' ConstExpr ']')+ ;

(* Shape dimensions are compile-time const expressions. *)

(* ---------- statements (algorithm body) ---------- *)

Stmt           ::= DataflowStmt
                 | EffectStmt
                 | ForStmt ;

DataflowStmt   ::= LValue '<--' RValue ';' ;

EffectStmt     ::= CallExpr ';' ;          (* bare side-effecting kernel call *)

ForStmt        ::= 'for' Ident ':' ConstExpr '..' ConstExpr '{' Stmt* '}' ;

(* ---------- LHS / RHS ---------- *)

LValue         ::= Ident IndexSuffix* ;
IndexSuffix    ::= '[' IndexExpr ']' ;

RValue         ::= CallExpr
                 | LValue ;                (* bare data reference is allowed
                                              as RHS for identity copies *)

CallExpr       ::= Ident '(' ArgList? ')' ;
ArgList        ::= RValue (',' RValue)* ;

(* ---------- expressions ---------- *)

(* Indexing expressions: integer arithmetic over loop vars, consts, literals.
   Same surface as ConstExpr; in fact the hand-written parser MERGES the two
   (algo/parser.rs `expr_parser`, "covers IndexExpr and ConstExpr"). The only
   difference is which identifiers are in scope (loop vars are in scope only
   inside their for-body) — and which atoms a LATER pass accepts (see below). *)
IndexExpr      ::= AddExpr ;
ConstExpr      ::= AddExpr ;

AddExpr        ::= MulExpr (('+' | '-') MulExpr)* ;
MulExpr        ::= UnaryExpr (('*' | '/' | '%') UnaryExpr)* ;
UnaryExpr      ::= ('-')? Atom ;
(* An Atom is an int literal, a parenthesised expr, OR an `Ident`-prefixed
   tail: a CallExpr `Ident(args)` or an indexed LValue `Ident IndexSuffix*`
   (a nested DATA read). The parser's `ident_or_call` builds both, and
   `IndexSuffix` recurses on the full expression, so a nested data read can
   appear in ANY expression position SYNTACTICALLY — including inside an
   index. The restriction that a data-dependent (gather) read is accepted
   in INDEX position but REJECTED in const/shape position is NOT grammatical:
   it is a semantic / lowering rule (`lower_index_expr`'s `allow_gather`,
   TASK-0341.03.01; `eval_const` returns None for a DataRef). See §6 item 8. *)
Atom           ::= IntLit
                 | '(' AddExpr ')'
                 | CallExpr
                 | LValue ;                (* LValue = Ident IndexSuffix*, a nested data read *)

(* ---------- lexical ---------- *)

Ident          ::= IdentStart IdentCont* ;
IdentStart     ::= 'a'..'z' | 'A'..'Z' | '_' ;
IdentCont      ::= IdentStart | '0'..'9' ;

IntLit         ::= '0'..'9'+ ;            (* decimal only; no hex/oct/bin in v2 *)

(* Comments: line comments only. *)
LineComment    ::= '//' (^'\n')* '\n' ;

(* Whitespace: spaces, tabs, newlines. Insignificant between tokens. *)
Whitespace     ::= (' ' | '\t' | '\r' | '\n')+ ;
```

## 2. Semantics-relevant notes (informative)

These are not part of the grammar but constrain valid programs. The
parser accepts; later passes reject.

1. **Single-assignment.** A given `LValue` in a given iteration may
   appear on the left of `<--` at most once. Re-assigning means a
   fresh `data` declaration. Enforced post-parse (PRD §6.2.1).
2. **Loop variable scope.** A name introduced by `for IDENT : ...`
   is in scope only inside the loop body. Shadowing is permitted;
   resolution is by lexical scope (PRD §6.2.3). No `@`-prefix.
3. **`const` evaluation.** `ConstExpr` is evaluated at compile time
   using integer arithmetic. Division (`/`) is integer division
   (Rust `i64`-style truncation toward zero). Overflow is a compile
   error.
4. **`pure` vs `effectful` is part of the type, not a decoration.**
   The parser stores it on the `KernelDecl` node; later passes use
   it to gate reordering / duplication.
5. **Bare-call statements (`EffectStmt`)** are only valid when the
   called kernel is `effectful`. A pure kernel called for its value
   thrown away is a warning (likely error) at type-check time.
6. **Index expressions vs. const expressions** share the same
   grammar but differ in scope: an `IndexExpr` may reference loop
   variables; a `ConstExpr` may not.

## 3. What this grammar does not cover

By design, per PRD §6.2.4. The parser MUST reject these tokens with a
hint that they belong in the schedule:

- **No worker names.** No `w0::`, no `host::`, no `::` operator at all.
- **No schedule directives in the algorithm.** None of
  `block=`, `vectorize=`, `unroll=`, `pipeline=`, `reuse`,
  `partition=`, `transfer=`, `buffer=`, `notify=`, `place`,
  `place_data`.
- **No `@y` prefix.** Name resolution is by lexical scope (PRD edit;
  see §[Design questions](#5-design-questions)).
- **No conditionals.** No `if`/`else`/`match`. Control flow inside
  the algorithm is loops only.
- **No recursion.** Kernels cannot call kernels from within the
  algorithm file; only the top-level dataflow does. Rust kernel
  bodies are free to be as complex as Rust allows — that is invisible
  to Nuc.
- **No closures, no higher-order functions, no generics.**
- **No exceptions / panics / `Result` in Nuc surface.** Errors are
  codegen-time only.
- **No module system, no imports.** Exactly one `.algo.nuc` per
  program (PRD §3). Kernel bodies live in an adjacent `kernels.rs`,
  resolved by the Rust toolchain — not by Nuc.
- **No string, char, struct, enum, tuple, or pointer types.**
  `ScalarType` is the closed set above. Arrays of arrays are
  expressed by the `DimList`; there are no nested type constructors.

A small negative example (must be rejected):

```nuc
// REJECTED: schedule directive in algorithm file.
for y : 1 .. H-1 {
    block = 64;                          // not in algorithm grammar
    img_out[y] <-- blur3(img_in[y]);
}
```

The parser sees `block` as an `Ident` followed by `=` and fails with
"unexpected '=' in for body; did you mean to put `block=` in a
`*.sched.nuc` file?". Same treatment for `vectorize=`, `transfer=`,
`buffer=`, `notify=`, `place`, `place_data`, etc.

## 4. Conformance check against existing examples

The grammar must accept every existing `prog.algo.nuc` under
`nuc-nucleus/examples/`. I walked through two examples by hand:

### 4.1 `examples/13-cnn-inference/prog.algo.nuc`

| Line(s)   | Construct                              | Rule(s)                                |
| --------- | -------------------------------------- | -------------------------------------- |
| 19–25     | `const B : usize = 16;` etc.           | `ConstDecl`                            |
| 29–32     | `data input : f32[B][C0][H][W];`       | `DataDecl`, `DimList` with `Ident` and `ConstExpr` |
| 31        | `data feat1 : f32[B][C1][H/2][W/2];`   | `DimList` with `H/2` — `ConstExpr` → `AddExpr` → `MulExpr` (`H` `/` `2`) |
| 35–36     | `kernel load_input : () -> f32[B][C0][H][W] effectful;` | `KernelDecl`, empty `KernelParamList`, `KernelRetType = DataType`, `Purity = 'effectful'` |
| 36        | `kernel save_output : (f32[B][N_CLASSES]) -> () effectful;` | `KernelRetType = '(' ')'` (unit) |
| 40–42     | `kernel conv_block_1 : (f32[C0][H][W]) -> f32[C1][H/2][W/2] pure;` | `KernelDecl` with `Purity = 'pure'` |
| 45        | `input <-- load_input();`              | `DataflowStmt`, `LValue` (no index), `CallExpr` with empty `ArgList` |
| 50–54     | `for n : 0 .. B { ... }`               | `ForStmt`, bounds are `ConstExpr` |
| 51        | `feat1[n] <-- conv_block_1(input[n]);` | `DataflowStmt`, `LValue` with one `IndexSuffix`, `CallExpr` with `RValue = LValue` (indexed) |
| 56        | `save_output(output);`                 | `EffectStmt` — bare call as a statement |

Every construct in this file is covered.

### 4.2 `examples/14-hearing-aid/prog.algo.nuc`

| Line(s)   | Construct                                          | Rule(s)                                |
| --------- | -------------------------------------------------- | -------------------------------------- |
| 33–34     | `const SAMPLES_PER_FRAME : usize = 256;`           | `ConstDecl`                            |
| 38–41     | `data mic_in : f32[N_FRAMES][SAMPLES_PER_FRAME];`  | `DataDecl`, `DimList` with two `ConstExpr` (each is a bare `Ident`) |
| 47–50     | `kernel fe_capture : () -> f32[SAMPLES_PER_FRAME] effectful;` etc. | `KernelDecl`, `Purity = 'effectful'` |
| 54–55     | Multi-line `kernel mix2 : (f32[..], f32[..]) -> f32[..] pure;` | `KernelParamList` with two types, whitespace insignificant across the line break |
| 67        | `for frame : 0 .. N_FRAMES {`                      | `ForStmt`, bounds are `ConstExpr` |
| 68–69     | `mic_in[frame] <-- fe_capture();`                  | `DataflowStmt` |
| 72        | `bt_out[frame] <-- denoise(mic_in[frame]);`        | `DataflowStmt` with nested `IndexSuffix` in argument |
| 73        | `rf_transmit(bt_out[frame]);`                      | `EffectStmt` — bare effectful call |
| 76        | `spk_out[frame] <-- denoise(mix2(mic_in[frame], bt_in[frame]));` | `CallExpr` argument is itself a `CallExpr` (nested calls supported by `ArgList → RValue → CallExpr`) |
| 77        | `fe_emit(spk_out[frame]);`                         | `EffectStmt` |

Every construct in this file is covered.

### 4.3 `examples/05-stencil/prog.algo.nuc` — KNOWN DIVERGENCE

`05-stencil/prog.algo.nuc` is **not** accepted by this grammar.
It uses the legacy 2013-style kernel-with-inline-body syntax:

```nuc
kernel blur3(a, b, c, d, e, f, g, h, i) -> out  where pure {{
    ${out} = (${a} + ${b} + ${c} + ...) * (1.0 / 9.0);
}};
```

This is incompatible with PRD §6.2.2, which states that v2 kernels are
real Rust functions with a shape-typed signature in `.algo.nuc` and a
body in an adjacent Rust file, and that Nucleus "does **not** substitute
text into kernel bodies". The `{{ ${out} = ... }}` substitution form is
the v1/2013 syntax this PRD section explicitly retires (see also the
delta table in PRD §2: "Kernels as text fragments via `${1}`
substitution" → "Kernels as real Rust functions with shape-typed
signatures.").

Resolution: this grammar tracks the PRD (v2). `05-stencil/prog.algo.nuc`
is stale and must be rewritten to the v2 form. Follow-up filed: see
implementation notes on TASK-0005.

## 5. Design questions

Documented for the record; resolutions are baked into the EBNF above.

### 5.1 Should shapes admit arbitrary `ConstExpr` (e.g. `H/2`)?

Yes. Example 13 (`f32[B][C1][H/2][W/2]`) requires it. The expression
is evaluated at compile time; the grammar reuses `AddExpr` so both
shape dims and loop bounds share one expression language. Integer
arithmetic only — no floats in shapes.

Trade-off: this admits `f32[A%B][C-D]`, which most users won't write,
but it keeps the surface uniform. Rejecting weird-but-legal const
expressions is a semantic concern (e.g. "shape evaluated to ≤ 0"),
not a grammatical one.

### 5.2 How is the `for` body expressed recursively?

`ForStmt ::= 'for' Ident ':' ConstExpr '..' ConstExpr '{' Stmt* '}'`.
The body is `Stmt*` — any sequence of dataflow / effect / nested
`for` statements. This naturally permits the nested style in
05-stencil:

```nuc
for y : 1 .. H-1 {
for x : 1 .. W-1 {
    img_out[y][x] <-- blur3(...);
}}
```

Closing braces may be adjacent (`}}`) or separated by whitespace; the
lexer treats them as two `}` tokens.

### 5.3 Semicolons: required or optional?

Required after every declaration and every statement, except after a
`ForStmt`'s closing `}` (loops are statements terminated by their own
brace). This matches all existing examples. Reasons:

- Trivially LL(k)-parseable.
- Cheap, unambiguous diff hunks.
- Matches Rust convention, which users will recognise.

Rejected alternative: newline-terminated statements (Python / Go
style). Too much hidden state in the lexer for too little keystroke
saving.

### 5.4 Allow bare `LValue` as RHS (identity copy)?

The grammar admits `data_out <-- data_in;`. None of the current
examples use this form; it costs nothing to permit and avoids a
forced `kernel id(x) -> x  pure;` declaration for trivial cases. If
this turns out to invite confusion, restrict in a later revision.

### 5.5 Should the grammar fix the order of top-level items?

No. The grammar allows `ConstDecl`, `DataDecl`, `KernelDecl`, and
statements in any interleaving. Semantic passes will enforce
"declarations before use" and reject forward references. Keeping the
grammar order-free avoids surprises when documentation or formatter
output reorders items.

### 5.6 What about `()` as a return type?

Explicit unit return is `'(' ')'`. Used in 13 and 14 for effectful
"save" / "transmit" kernels. The grammar admits it as a distinct
alternative of `KernelRetType` rather than treating `()` as a zero-
arity tuple type — because Nuc has no tuple types, this avoids
sneaking one in.

### 5.7 Why no `mut`, no `let`, no `fn`?

Because data declarations are at the `data` level (whole-program
arrays) and intermediates are introduced by `<--` to single-assignment
arrays. Adding a scalar `let` would introduce a third storage class
for nothing — composing into a 1-element `data` is the same surface
with no special case.

## 6. Limitations (honest)

1. **EBNF is descriptive only.** No parser generator runs on this
   file. The hand-written parser in TASK-0006/TASK-0007 is the actual
   conformance artefact. Drift between this doc and the parser is a
   real risk; mitigation is to add a parser-vs-doc test in TASK-0007
   that round-trips at least the §4 examples.
2. **No formal precedence table for `IndexExpr`.** The EBNF above
   gives the standard arithmetic precedence (`*`, `/`, `%` over `+`,
   `-`; unary `-` higher than both). Documented in prose, not as a
   separate precedence table. If we ever add `&`, `|`, `<<`, the
   precedence table moves into its own section.
3. **No location tracking syntax.** Line/column reporting is a parser
   concern, not a grammar concern. The grammar says nothing about how
   to emit errors.
4. **Reserved words are not exhaustively listed.** Two distinct
   reserved sets apply (both decided in one place,
   `algo/parser.rs::ident_collision_message`, shared by `ident` and the
   `for_loop_var` loop-variable parser):
   (a) the *grammar* keywords — `const`, `data`, `kernel`, `pure`,
   `effectful`, `for`, plus the scalar type names — which have
   syntactic meaning; adding a future grammar keyword (e.g. a new
   statement form) needs a grammar revision. (b) the *Rust-keyword*
   reserved set (`nucleus-compiler/src/reserved.rs::RUST_RESERVED`):
   an identifier equal to a Rust strict/reserved keyword (`in`, `let`,
   `match`, `move`, `loop`, `fn`, `crate`, `self`, …) is rejected with
   a codegen-collision diagnostic at the identifier's source span,
   because every backend emits the identifier verbatim as a bare Rust
   binding/path segment (`let mut {name}`) and `rustc` would otherwise
   fail on generated source the user never wrote (TASK-0433). The
   grammar reject is checked first, so the overlap (`const`, `for`)
   keeps its grammar-specific message. The `for VAR :` loop-variable
   position is anchored too: a dedicated `algo/parser.rs::for_loop_var`
   parser (TASK-0434) routes through the same reject decision
   (`ident_collision_message`) and, on a collision, consumes the rest
   of the for-head through the opening `{` so its error out-reaches the
   block-`{` in chumsky 0.9's furthest-position error-merge — while
   pinning the display span at `VAR`. So both a Rust-reserved `VAR`
   (`for loop :`) and a grammar-keyword `VAR` (`for const :`) report a
   span-anchored message at `VAR`, with the same wording the data /
   kernel / worker positions emit for the same word.
5. **No Unicode identifier policy.** ASCII identifiers only in v2.
   Documented in the lexical section.
6. **One existing example (`05-stencil`) does not conform.** Tracked
   as a follow-up; see §4.3.
7. **Comments inside multi-line tokens are not specified.** `// ...`
   ends at the next `\n`. No block comments. No nested comments. If
   a future kernel-doc convention emerges, revisit.
8. **The grammar does NOT distinguish index position from const/shape
   position.** `expr_parser` (algo/parser.rs) parses one expression
   surface for both `IndexExpr` and `ConstExpr`, and an `Atom` may be a
   nested data read (`Ident IndexSuffix*`) or a call. So a
   data-dependent (gather) index such as `x[col[k]]` is *grammatical*
   in every expression position. Whether such a read is ACCEPTED is a
   semantic decision made by later passes, not the grammar: lowering
   admits a gather only in index position (`lower_index_expr`'s
   `allow_gather`, TASK-0341.03.01), and `eval_const` returns `None` for
   a `DataRef`/`Call`, so a data read in a const/shape position is
   rejected there. Earlier revisions of this section claimed the
   *grammar* admitted a data read only in index position — that was
   imprecise; the distinction lives in lowering / const-eval.

## 7. Pointers

- PRD §6.2 — algorithm sublanguage spec.
- PRD §6.2.4 — explicit exclusions (mirrored in §3 above).
- `nuc-nucleus/examples/13-cnn-inference/prog.algo.nuc` —
  canonical v2 example for kernel declarations.
- `nuc-nucleus/examples/14-hearing-aid/prog.algo.nuc` —
  canonical v2 example for nested calls and bare effect statements.
- TASK-0006 / TASK-0007 — parser implementation and tests.
