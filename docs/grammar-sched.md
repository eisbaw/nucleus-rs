# Schedule sublanguage grammar (`*.sched.nuc`)

Status: descriptive EBNF (informative). No parser is auto-generated from
this file in v2. The reference parser (TASK-0010, TASK-0011) is
hand-written against this spec; conformance is asserted by behavioural
tests, not by grammar derivation.

Scope: this document fixes the surface syntax of the schedule
sublanguage as described in
[PRD §6.3](../nuc-nucleus/PRD.md#63-schedule-sublanguage). The algorithm
sublanguage (`*.algo.nuc`) is documented separately in
[`grammar-algo.md`](grammar-algo.md) and is explicitly out of scope here
(see §[What this grammar does not cover](#3-what-this-grammar-does-not-cover)).

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

Program        ::= ScheduleBlock ;

ScheduleBlock  ::= 'schedule' 'for' StringLit '{' SchedItem* '}' ;

(* Order of SchedItems is free. Semantic passes enforce
   "declare worker_class / memory_region before reference". *)
SchedItem      ::= WorkerClassDecl
                 | MemoryRegionDecl
                 | WorkersDecl
                 | PlaceStmt
                 | PlaceDataStmt
                 | LoopStmt
                 | TransferStmt
                 | CheckStmt ;

(* ---------- worker topology ---------- *)

WorkerClassDecl ::= 'worker_class' Ident '{' ClassField* '}' ';' ;

ClassField     ::= 'simd'   '=' SimdSpec   ';'
                 | 'memory' '=' MemorySpec ';' ;

SimdSpec       ::= 'none' | Ident ;            (* e.g. 'neon128', 'avx2' *)

MemorySpec     ::= MemoryAtom ('+' MemoryAtom)* ;
MemoryAtom     ::= 'shared'
                 | Ident ('[' SizeLit ']')? ;  (* e.g. 'tightly_coupled[64KB]' *)

MemoryRegionDecl ::= 'memory_region' Ident '{' RegionField* '}' ';' ;

RegionField    ::= 'size'           '=' SizeLit                   ';'
                 | 'accessible_by'  '=' '{' IdentList? '}'        ';'
                 | 'per_worker'     '=' BoolLit                   ';' ;

WorkersDecl    ::= 'workers' '=' WorkersSet ';' ;

(* Two forms. Simple: bare names. Typed: 'name : class' pairs.
   The two forms are mutually exclusive within one '{ ... }'; the
   parser decides by looking for ':' in the first non-trivial element.
   Trailing comma is permitted in both forms (see §4 conformance walk). *)
WorkersSet     ::= '{' SimpleWorkerList? '}'
                 | '{' TypedWorkerList?  '}' ;

SimpleWorkerList ::= Ident (',' Ident)* ','? ;
TypedWorkerList  ::= TypedWorker (',' TypedWorker)* ','? ;
TypedWorker      ::= Ident ':' Ident ;          (* worker_name : class_name *)

(* ---------- kernel placement ---------- *)

PlaceStmt      ::= 'place' Ident 'on' PlaceTarget ';' ;

PlaceTarget    ::= Ident                              (* single worker      *)
                 | '{' IdentList '}' ;                (* distributed over set *)

(* ---------- data placement ---------- *)

PlaceDataStmt  ::= 'place_data' Ident 'in' Ident ';' ;   (* data in region *)

(* ---------- loop transformations ---------- *)

LoopStmt       ::= 'loop' Ident ':' LoopOptList ';' ;

LoopOptList    ::= LoopOpt (',' LoopOpt)* ;

LoopOpt        ::= 'block'     '=' IntLit
                 | 'vectorize' '=' IntLit
                 | 'unroll'    '=' IntLit
                 | 'pipeline'  '=' IntLit
                 | 'reuse'
                 | 'partition' '=' PartitionKind ;

PartitionKind  ::= 'rows' | 'blocks2d' | 'workers' ;

(* ---------- transfer / IO semantics ---------- *)

TransferStmt   ::= 'transfer' Ident ':' XferOptList ';' ;

XferOptList    ::= XferOpt (',' XferOpt)* ;

XferOpt        ::= 'sync'
                 | 'async'
                 | 'buffer'   '=' IntLit
                 | 'notify'   '=' NotifyKind ;

NotifyKind     ::= 'event' | 'poll' ;

(* ---------- runtime assertions ---------- *)

(* A 'check' applies to a previously-named loop variable. Multiple
   assertions on the same loop combine into one CheckStmt with a
   comma-separated assertion list. *)
CheckStmt      ::= 'check' 'loop' Ident ':' CheckAssertList ';' ;

CheckAssertList ::= CheckAssert (',' CheckAssert)* ;

CheckAssert    ::= 'latency_max'  '=' TimeLit
                 | 'on_violation' '=' ViolationKind ;

ViolationKind  ::= 'panic' | 'log' | 'count' ;

(* ---------- shared lexicals ---------- *)

IdentList      ::= Ident (',' Ident)* ','? ;

(* ---------- literals ---------- *)

StringLit      ::= '"' StringChar* '"' ;       (* file path to the algorithm *)
StringChar     ::= any character except '"' or '\n' ;

IntLit         ::= '0'..'9'+ ;                 (* decimal only; no hex *)

BoolLit        ::= 'true' | 'false' ;

(* Time literals carry a unit suffix. No space between number and unit. *)
TimeLit        ::= IntLit TimeUnit ;
TimeUnit       ::= 'ns' | 'us' | 'ms' | 's' ;

(* Size literals are integer counts of bytes or binary multiples.
   No space between number and unit. *)
SizeLit        ::= IntLit SizeUnit? ;
SizeUnit       ::= 'B' | 'KB' | 'MB' | 'GB' ;  (* binary: KB = 1024 B *)

(* ---------- lexical ---------- *)

Ident          ::= IdentStart IdentCont* ;
IdentStart     ::= 'a'..'z' | 'A'..'Z' | '_' ;
IdentCont      ::= IdentStart | '0'..'9' ;

(* Comments: line comments only. *)
LineComment    ::= '//' (^'\n')* '\n' ;

(* Whitespace: spaces, tabs, newlines. Insignificant between tokens. *)
Whitespace     ::= (' ' | '\t' | '\r' | '\n')+ ;
```

## 2. Semantics-relevant notes (informative)

These are not part of the grammar but constrain valid programs. The
parser accepts; later passes reject.

1. **Every algorithm kernel must have exactly one `place`.** A kernel
   referenced in the algorithm but not placed by the schedule is a
   compile error. A `place` for a kernel that does not exist in the
   algorithm is also a compile error (PRD §6.3.2, §13 "Schedule
   completeness checking"). The grammar permits anything; the linker
   catches the mismatch.
2. **Cross-worker `transfer` is mandatory.** If a data symbol is
   produced on one worker and consumed on another, a `transfer`
   directive for that symbol must be present. Omitting it is a
   compile error citing the offending data symbol (PRD §6.3.4).
   Intra-worker data needs no `transfer` and emits no event.
3. **No implicit defaults for `place`, `transfer`, or `loop`.** PRD §3
   explicitly forbids a default schedule. Each kernel is placed
   explicitly; each cross-worker data is transferred explicitly. The
   grammar deliberately does not provide fallback syntax.
4. **Worker class / memory region references resolve by name.**
   `workers = { fe : fe_core, ... }` requires `fe_core` to be declared
   as `worker_class fe_core { ... };` earlier in the same schedule;
   `place_data img_in in shared_sram;` requires `shared_sram` to be a
   declared `memory_region`. Forward references are rejected.
5. **`sync` and `async` are mutually exclusive in one `TransferStmt`.**
   Both belong to `XferOpt` and the grammar allows them to appear in
   the same list — a semantic pass rejects `transfer x : sync, async;`
   with a hint that they conflict. See §5.3.
6. **`buffer=N` on a `sync` transfer.** Allowed by the grammar.
   Whether it is *useful* depends on the backend. Some backends
   (e.g. `pthreads-sync`) treat `sync` as zero-buffer regardless;
   others (e.g. `mp-tcp-bufsync`) honour `buffer=N` even under sync
   semantics. See §5.4.
7. **Loop option composition order is not significant.** `loop x :
   block=64, vectorize=8, reuse;` and `loop x : reuse, vectorize=8,
   block=64;` parse to the same set. Semantic conflicts (e.g.
   `block=64, block=128` on the same loop) are rejected at link time.
   See §5.1.
8. **`check` references a loop variable, not a `loop`-directive.**
   The schedule can have `check loop frame : latency_max = 10ms;`
   without ever issuing `loop frame : ...;`. The check applies to the
   iteration boundary of `frame` in the algorithm. See §5.2.
9. **String literal in `schedule for "..."` is a path relative to the
   schedule file.** All existing examples use `"../prog.algo.nuc"`.
   The parser stores the raw string; the build driver resolves it.

## 3. What this grammar does not cover

By design, per PRD §6.3.6. The parser MUST reject these tokens with a
hint that they belong in the algorithm:

- **No kernel bodies.** No function bodies, no `${}` substitution, no
  expression statements. Kernel implementations live in `kernels.rs`,
  not in the schedule.
- **No `data` declarations.** Data symbols are introduced by the
  algorithm; the schedule only references them by name in `place_data`
  and `transfer` directives.
- **No dataflow edges (`<--`).** The algorithm declares dataflow; the
  schedule attaches IO semantics to the *inferred* edges.
- **No control flow.** No `for`, no `if`, no `else`, no `match`, no
  `while`. A schedule with a `for` loop is rejected with "control
  flow belongs in the algorithm file".
- **No expressions in directives.** `block=64` takes an `IntLit`, not
  an `AddExpr`. The schedule is configuration, not computation. If a
  value needs to be computed, compute it in the algorithm as a `const`
  and reference it… but in v2 the schedule does not see algorithm
  consts either. Constants in directives are literal integers,
  period.
- **No conditionals across worker boundaries** (also forbidden in the
  algorithm — restated here for completeness).
- **No imports, no schedule includes.** Exactly one `*.sched.nuc` per
  build (PRD §3).
- **No automatic defaults.** A schedule omitting a `place` for an
  algorithm kernel does not compile. A schedule omitting a `transfer`
  for a cross-worker data symbol does not compile. These are linker
  errors, not parser errors, but the grammar admits no fallback
  syntax that would make them silently legal.

A small negative example (must be rejected):

```nuc
// REJECTED: control flow in schedule file.
schedule for "../prog.algo.nuc" {
    workers = { host, w0 };
    place blur3 on w0;
    for y : 0..H {                    // not in schedule grammar
        loop y : block=64;
    }
}
```

The parser sees `for` as an unexpected keyword in `SchedItem` position
and fails with "unexpected `for` in schedule body; control flow belongs
in `*.algo.nuc`". Same treatment for `if`, `while`, `<--`, `data`,
`kernel`.

## 4. Conformance check against existing examples

The grammar must accept every existing `*.sched.nuc` under
`nuc-nucleus/examples/`. Spot-check walks of two files follow; the
remaining five files use only the same constructs.

### 4.1 `examples/05-stencil/schedules/distributed.sched.nuc`

Exercises the simple worker form, distributed placement, two loop
directives with combined options, and two transfer directives with
contrasting semantics.

| Line(s) | Construct                                              | Rule(s) |
| ------- | ------------------------------------------------------ | ------- |
| 7       | `schedule for "../prog.algo.nuc" {`                    | `ScheduleBlock`, `StringLit` |
| 8       | `workers = { host, w0, w1, w2, w3 };`                  | `WorkersDecl` → `SimpleWorkerList` |
| 10      | `place load_image on host;`                            | `PlaceStmt` with `PlaceTarget = Ident` |
| 12      | `place blur3 on { w0, w1, w2, w3 };`                   | `PlaceStmt` with `PlaceTarget = '{' IdentList '}'` |
| 19      | `loop y : partition=rows;`                             | `LoopStmt`, `LoopOpt = 'partition' '=' 'rows'` |
| 24      | `loop x : block=64, vectorize=8, reuse;`               | `LoopStmt`, three `LoopOpt`s — `block=IntLit`, `vectorize=IntLit`, bare `reuse` |
| 29      | `transfer img_in : async, buffer=2, notify=event;`     | `TransferStmt`, three `XferOpt`s — bare `async`, `buffer=IntLit`, `notify=event` |
| 33      | `transfer img_out : sync;`                             | `TransferStmt`, one `XferOpt = 'sync'` |

Every construct in this file is covered.

### 4.2 `examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc` — the demanding case

Exercises typed workers, three worker classes, two memory regions
(one with `per_worker = true`), `place_data` directives, async
transfers with buffering, and the `check` directive.

| Line(s) | Construct                                                  | Rule(s) |
| ------- | ---------------------------------------------------------- | ------- |
| 17      | `schedule for "../prog.algo.nuc" {`                        | `ScheduleBlock`, `StringLit` |
| 21–24   | `worker_class fe_core { simd = none; memory = shared; };`  | `WorkerClassDecl` with two `ClassField`s; `SimdSpec = 'none'`; `MemorySpec` is a single `MemoryAtom = 'shared'`; closing `};` |
| 26–29   | `worker_class dsp_core { simd = neon128; memory = tightly_coupled[64KB] + shared; };` | `SimdSpec = Ident('neon128')`; `MemorySpec = MemoryAtom '+' MemoryAtom`, the first atom is `tightly_coupled[64KB]` (`Ident '[' SizeLit ']'`), the second is `shared` |
| 31–34   | `worker_class rf_core { ... };`                            | as `fe_core` |
| 36–39   | `memory_region sram_shared { size = 128KB; accessible_by = { fe_core, dsp_core, rf_core }; };` | `MemoryRegionDecl` with two `RegionField`s; `size` field carries `SizeLit = 128KB`; `accessible_by` field carries an `IdentList` inside `{ }`; closing `};` |
| 41–45   | `memory_region dsp_tcm { size = 64KB; accessible_by = { dsp_core }; per_worker = true; };` | three `RegionField`s including `per_worker = BoolLit('true')` |
| 47–51   | `workers = { fe : fe_core, dsp : dsp_core, rf : rf_core, };` | `WorkersDecl` → `TypedWorkerList`, three `TypedWorker`s, **trailing comma** |
| 57–60   | `place_data mic_in in sram_shared;` (and three siblings)   | `PlaceDataStmt`, `Ident 'in' Ident` |
| 64–71   | `place fe_capture on fe;` etc.                             | `PlaceStmt` with `PlaceTarget = Ident` |
| 78      | `loop frame : pipeline=3;`                                 | `LoopStmt`, one `LoopOpt = 'pipeline' '=' IntLit` |
| 84–90   | `transfer mic_in : async, buffer=2, notify=event;` (×4)    | `TransferStmt`, three `XferOpt`s each |
| 105     | `check frame : latency_max = 10ms;`                        | `CheckStmt`, **note:** existing file omits the `loop` keyword that the grammar requires (see §4.3) |

### 4.3 KNOWN DIVERGENCE: `check` without `loop` keyword

The PRD (§6.3.5) specifies the `check` form as:

```
check loop VAR : assertion [ , assertion ]* ;
```

(`loop` keyword between `check` and the variable name.) The grammar
above mirrors the PRD.

`examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc` line 105
writes:

```nuc
check frame : latency_max = 10ms;
```

— without the `loop` keyword. This file does **not** conform to the
grammar as specified.

Two possible resolutions:

1. **Relax the grammar** to make `loop` optional after `check`:
   `CheckStmt ::= 'check' 'loop'? Ident ':' CheckAssertList ';'` .
   Cheap, matches the example, but introduces a small ambiguity if
   future `check` targets are not loop variables (e.g. per-transfer
   checks — PRD §6.3.5 mentions `buffer_max` as a future variant that
   would naturally attach to a transfer).
2. **Fix the example** to write `check loop frame : latency_max = 10ms;`
   per PRD.

This grammar tracks the PRD (option 2). The example is stale; follow-up
filed to reconcile (see implementation notes on TASK-0006).

The other six existing schedule files are fully covered. None use
`check`, so the divergence is isolated to this one file.

## 5. Design questions

Documented for the record; resolutions are baked into the EBNF above.

### 5.1 Composition order of loop options

`LoopOptList` is comma-separated without ordering constraints. The
grammar treats `loop x : block=64, vectorize=8, reuse` and `loop x :
reuse, vectorize=8, block=64` as equally well-formed. The schedule's
*meaning* — what transforms apply in what order — is determined by
the compiler's lowering pass, not by source order.

Trade-off: this is the right choice for users (no surprising
syntactic ordering) but makes the parser produce a *set* of options
rather than a *list*, which means duplicate options must be detected
post-parse. Detection is a one-pass check at link time; not a
grammar concern.

Rejected alternative: enforce a canonical order
(`block`, `unroll`, `vectorize`, `pipeline`, `partition`, `reuse`).
Buys nothing for users; adds a reformatter task for no payoff.

### 5.2 Is `check` grammatically tied to a `loop=` directive?

No. A `check` statement names a loop variable from the algorithm;
the schedule does not need to issue a corresponding `loop VAR : ...`
directive for that variable. The `check` and the `loop` are
independent statements that happen to share a loop-variable name.

This matches the PRD `embedded_multimcu` example, which has both
`loop frame : pipeline=3;` and `check loop frame : latency_max = 10ms;`
— two statements, same `frame` variable.

A grammar that required `check` to nest inside `loop` (e.g.
`loop frame : pipeline=3 check latency_max = 10ms;`) was considered
and rejected: it conflates two concerns (transform vs assertion) into
one statement and makes parsing harder for no readability gain.

### 5.3 Should `sync` + `async` in one `TransferStmt` be a parse error or a semantic error?

The grammar allows both. A semantic pass rejects the combination with
a clear message ("transfer cannot be both sync and async"). Rationale:
keeping the grammar option-list-flat means every option lives in one
`XferOpt` alternative; pushing mutual exclusion into the grammar would
require splitting into "sync-only options" vs "async-only options"
subsets and the readability cost is not paid back.

### 5.4 Is `buffer=N` on `sync` allowed?

Yes, syntactically. The semantic interpretation is backend-dependent.
For tier-1 `pthreads-sync`, `buffer=N` on a `sync` transfer is a
no-op (the synchronous handshake permits at most one in-flight slot
regardless). For `mp-tcp-bufsync`, `buffer=N` on a `sync` transfer
configures a buffered handoff with blocking semantics at the
application level. The capability matrix per backend (PRD §7.4)
declares which combinations are honoured; mismatches are rejected at
compile time.

This is one of the design tensions the schedule sublanguage exposes
on purpose: not every combination is meaningful on every backend, but
the *schedule* (not the *algorithm*) is the place where the
combination is asked for, so the schedule's grammar must permit the
combination and let backend-specific resolution reject it.

### 5.5 Trailing commas in `{ ... }` lists

Permitted in `SimpleWorkerList`, `TypedWorkerList`, and `IdentList`.
The 14-hearing-aid `workers = { fe : fe_core, dsp : dsp_core, rf :
rf_core, };` exercises the trailing comma. Cost: trivial. Benefit:
clean diffs when adding workers/regions one at a time.

### 5.6 PRD's `w0..w3` range shorthand

PRD §6.3.1 shows the typed-worker form using a range:

```
workers = {
    host       : control_core,
    w0..w3     : compute_core,
};
```

No existing example file uses this syntax. The grammar above does
**not** include it — adding syntax without an example to test against
is the kind of speculative surface area v2 should avoid. If a tier-3
example lands that wants compact range syntax, extend
`TypedWorker ::= Ident ('..' Ident)? ':' Ident` and the conformance
walk gets a new entry.

Documented here so future readers don't think the omission is
accidental.

### 5.7 Numeric units: `KB` vs `KiB`, `ms` vs `msec`

The grammar takes `KB` (binary, 1024 B) and `ms`/`us`/`ns`/`s` (SI
prefixes on the second). This matches existing examples
(`size = 128KB`, `latency_max = 10ms`). Alternatives (`KiB`, `MiB`,
`msec`) are rejected — one canonical form per dimension. If the
existing `KB` convention is ambiguous to the reader, prose
documentation clarifies; the grammar does not multiply.

### 5.8 Semicolons after closing braces of `worker_class` / `memory_region`

Required. Every existing example writes `};` to terminate
`worker_class { ... }` and `memory_region { ... }` blocks. The outer
`schedule for "..." { ... }` block has *no* trailing semicolon
(it is the top-level form).

The asymmetry is intentional: the inner block declarations are
sequenceable inside the schedule block, so they need a statement
terminator. The outer block is the whole file, so it does not.

### 5.9 Free ordering of `SchedItem`s

The grammar imposes no order on `SchedItem`s. Conventional ordering in
existing examples is: worker classes → memory regions → workers →
place_data → place → loop → transfer → check. Semantic passes enforce
"declare before reference" but not stylistic ordering. A formatter
could canonicalise; the grammar does not.

## 6. Limitations (honest)

1. **EBNF is descriptive only.** No parser generator runs on this
   file. The hand-written parser in TASK-0010 / TASK-0011 is the
   actual conformance artefact. Drift between this doc and the parser
   is a real risk; mitigation is to add a parser-vs-doc test in
   TASK-0011 that round-trips at least the §4 examples.
2. **Semantic checks are out of scope.** "Every algorithm kernel is
   placed", "every cross-worker data has a `transfer`", "the
   `worker_class` named in `workers = { x : C }` exists", "no
   duplicate `place` for one kernel", "no conflicting loop options" —
   all of these land in later tasks (TASK-0010 for the linker,
   TASK-0011 for the integrated checks). The grammar accepts a
   schedule that violates each of these; the linker rejects it.
3. **`check` is loop-scoped only.** The grammar supports only
   `check loop VAR : ...;`. PRD §6.3.5 anticipates future per-transfer
   checks (`buffer_max`) and end-to-end latency checks; neither has
   syntax in v2. Adding them is a grammar revision, not a relaxation.
4. **One existing example (`embedded_multimcu`) does not conform.**
   The `check` form omits the `loop` keyword the PRD specifies. See
   §4.3 and the follow-up task.
5. **`SizeLit` is integer-only.** No `1.5KB`, no `2.5MB`. Memory
   region sizes are byte counts; fractional binary multipliers are
   nonsense. Same for `TimeLit` — `10ms` not `10.5ms`. If a real
   schedule needs `1500us` rather than `1.5ms`, that is the form the
   grammar requires.
6. **No location tracking syntax.** Line/column reporting is a parser
   concern, not a grammar concern.
7. **Reserved words are not exhaustively listed.** The implicit list
   is `schedule`, `for`, `worker_class`, `memory_region`, `workers`,
   `place`, `place_data`, `on`, `in`, `loop`, `transfer`, `check`,
   `simd`, `memory`, `size`, `accessible_by`, `per_worker`, plus the
   option keywords (`block`, `vectorize`, `unroll`, `pipeline`,
   `reuse`, `partition`, `sync`, `async`, `buffer`, `notify`,
   `latency_max`, `on_violation`) and the value-side atoms (`none`,
   `shared`, `event`, `poll`, `rows`, `blocks2d`, `workers`, `panic`,
   `log`, `count`, `true`, `false`). Adding a future keyword needs a
   grammar revision.
8. **No Unicode identifier policy.** ASCII identifiers only in v2.
9. **Comments inside multi-line tokens are not specified.** `// ...`
   ends at the next `\n`. No block comments. No nested comments.
10. **The `notify=poll` / `notify=event` distinction is syntactic
    only.** Which one is actually supported on a given backend lives
    in `capabilities.toml` (PRD §7.4). The grammar cannot prevent a
    user from asking for an unsupported notification mode; the
    compiler must.

## 7. Pointers

- PRD §6.3 — schedule sublanguage spec.
- PRD §6.3.6 — explicit exclusions (mirrored in §3 above).
- PRD §13 — schedule completeness checking, capability mismatch.
- [`grammar-algo.md`](grammar-algo.md) — sibling document for the
  algorithm sublanguage.
- `nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc` —
  canonical simple-workers + loop + transfer example.
- `nuc-nucleus/examples/13-cnn-inference/schedules/pipeline_parallel.sched.nuc`
  — canonical pipelining example with multiple transfer directives.
- `nuc-nucleus/examples/14-hearing-aid/schedules/embedded_multimcu.sched.nuc`
  — canonical typed-workers + memory-regions + place_data + check
  example (the demanding case).
- TASK-0010 / TASK-0011 — schedule parser and linker; behavioural
  conformance lives there.
