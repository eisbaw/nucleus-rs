//! Hand-written parser for `*.sched.nuc` over the `chumsky` combinator
//! library. Implements the EBNF in `docs/grammar-sched.md`.
//!
//! # Parser-library choice
//!
//! Chosen: `chumsky` 0.9 — same as the algorithm parser
//! (`crate::algo::parser`). The schedule grammar is small, ASCII-only,
//! and has no operator-precedence complications, so the rationale from
//! TASK-0007 applies unchanged. The two parsers deliberately share the
//! same library, the same error type ([`crate::error::ParseError`]),
//! and the same comment/whitespace helpers' shape, so a reader who
//! understands one can read the other immediately.
//!
//! # Known limitations (TASK-0008 self-report)
//!
//! - Only the first parse error is reported (inherited from the
//!   chumsky usage style). Multi-error reporting is a follow-up.
//! - No error recovery — the parser bails on the first syntactic
//!   failure rather than skipping to the next plausible directive.
//! - AST nodes do not carry spans; only `ParseError` does. Adding
//!   per-node spans is a follow-up task — semantic passes (TASK-0010,
//!   TASK-0011) will want them for good diagnostics.
//! - Semantic checks are deliberately out of scope here:
//!   - `place` references a kernel that exists in the algorithm
//!     (TASK-0011).
//!   - `accessible_by` names resolve to declared classes/workers
//!     (TASK-0010).
//!   - Mutually exclusive options (`sync` + `async` in one transfer,
//!     duplicate `block=N`, `block=64` and `block=128` together) —
//!     all linker-pass concerns (grammar §2 notes 5, 7).
//!   - Forward-reference rejection for `worker_class` / `memory_region`
//!     (grammar §2 note 4). The parser accepts any order; the linker
//!     rejects forward references.
//! - The PRD `w0..w3` range-typed-worker shorthand is NOT supported.
//!   Grammar §5.6 explicitly excludes it pending a test case.

use chumsky::prelude::*;

use super::ast::{
    CheckAssert, CheckDirective, Directive, LoopDirective, LoopOption, MemoryAtom,
    MemoryRegionDecl, MemorySpec, NotifyKind, PartitionKind, PlaceDataDirective, PlaceDirective,
    PlaceTarget, SchedAst, SimdSpec, SpDirective, SpName, TimeLit, TimeUnit, TransferDirective,
    TransferOption, ViolationKind, WorkerClassDecl, WorkerEntry, WorkersDecl,
};
use crate::error::{map_first_chumsky_error, ParseError};
use crate::span::Spanned;

/// Parse a `*.sched.nuc` source string into a [`SchedAst`].
///
/// Errors carry `(line, column)` (1-based) of the first failure. The
/// parser does not recover; multiple-error reporting is a follow-up.
pub fn parse_sched(src: &str) -> Result<SchedAst, ParseError> {
    let parser = program_parser();
    match parser.parse(src) {
        Ok(ast) => Ok(ast),
        Err(errors) => Err(map_first_chumsky_error(src, errors)),
    }
}

// --------------------------------------------------------------------
// Grammar
// --------------------------------------------------------------------

/// Reserved words listed in `docs/grammar-sched.md` §6 note 7.
///
/// Identifiers may not collide with these. Note: the *value-side*
/// atoms (`none`, `shared`, `event`, `poll`, `rows`, `blocks2d`,
/// `workers`, `panic`, `log`, `count`, `true`, `false`) are also
/// reserved per the same note. We include them so a stray
/// `place none on host;` parses as a keyword collision rather than
/// silently as a kernel named `none`.
const KEYWORDS: &[&str] = &[
    // structural
    "schedule",
    "for",
    "worker_class",
    "memory_region",
    "workers",
    "place",
    "place_data",
    "on",
    "in",
    "loop",
    "transfer",
    "check",
    // class / region fields
    "simd",
    "memory",
    "size",
    "accessible_by",
    "per_worker",
    // loop options
    "block",
    "vectorize",
    "unroll",
    "pipeline",
    "reuse",
    "partition",
    // transfer options
    "sync",
    "async",
    "buffer",
    "notify",
    // check options
    "latency_max",
    "on_violation",
    // value atoms
    "none",
    "shared",
    "event",
    "poll",
    "rows",
    "blocks2d",
    "panic",
    "log",
    "count",
    "true",
    "false",
];

/// Whitespace + line comments. Grammar §1 lexical rules.
fn comment_or_ws() -> impl Parser<char, (), Error = Simple<char>> + Clone {
    let line_comment = just("//")
        .then(take_until(text::newline().or(end())))
        .ignored();
    line_comment
        .or(one_of(" \t\r\n").ignored())
        .repeated()
        .ignored()
}

/// Helper: token followed by trailing whitespace/comments.
fn pad<P, T>(p: P) -> impl Parser<char, T, Error = Simple<char>> + Clone
where
    P: Parser<char, T, Error = Simple<char>> + Clone,
{
    p.then_ignore(comment_or_ws())
}

/// Helper: capture a node's span on the bare token **before** the
/// trailing whitespace/comment padding is consumed, so the span is the
/// tight extent of the source text the node was parsed from (no
/// trailing layout). Leading layout is already consumed by the
/// *previous* token's pad, matching `ParseError`'s offset convention.
///
/// This is the span-correctness primitive (TASK-0086), mirroring the
/// algorithm parser's `padded_spanned`: `pad(p)` alone would, if
/// `.map_with_span`-wrapped on the outside, fold the trailing
/// whitespace into the span — wrong for a diagnostic that underlines a
/// token / directive. `padded_spanned(p)` wraps with the span fixed
/// first, then eats trailing layout off-span.
fn padded_spanned<P, T>(p: P) -> impl Parser<char, Spanned<T>, Error = Simple<char>> + Clone
where
    P: Parser<char, T, Error = Simple<char>> + Clone,
{
    p.map_with_span(Spanned::new).then_ignore(comment_or_ws())
}

/// A reserved-word matcher that ensures the keyword is not the prefix
/// of a longer identifier (e.g. `loop_var` does not start `loop`).
fn keyword(kw: &'static str) -> impl Parser<char, (), Error = Simple<char>> + Clone {
    just(kw)
        .then_ignore(
            none_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_").rewind(),
        )
        .ignored()
}

/// Identifier matcher. Rejects keywords. Yields an [`SpName`] whose
/// span is exactly the identifier token's byte range (no surrounding
/// whitespace — callers wrap this in `pad`, which consumes trailing
/// space *after* this combinator), so a "duplicate / undeclared `X`"
/// diagnostic underlines just `X` (TASK-0086 / TASK-0196).
fn ident() -> impl Parser<char, SpName, Error = Simple<char>> + Clone {
    let start = filter(|c: &char| c.is_ascii_alphabetic() || *c == '_');
    let cont = filter(|c: &char| c.is_ascii_alphanumeric() || *c == '_');
    start
        .chain(cont.repeated())
        .collect::<String>()
        .try_map(|s, span| {
            if KEYWORDS.contains(&s.as_str()) {
                Err(Simple::custom(
                    span,
                    format!("expected identifier, found keyword `{}`", s),
                ))
            } else {
                Ok(s)
            }
        })
        .map_with_span(Spanned::new)
}

/// Decimal integer literal. Grammar §1: `IntLit ::= '0'..'9'+`.
fn int_lit() -> impl Parser<char, u64, Error = Simple<char>> + Clone {
    filter(|c: &char| c.is_ascii_digit())
        .repeated()
        .at_least(1)
        .collect::<String>()
        .try_map(|s, span| {
            s.parse::<u64>()
                .map_err(|e| Simple::custom(span, format!("invalid integer `{}`: {}", s, e)))
        })
}

/// Quoted string literal — no escapes, no newlines (grammar §1
/// `StringChar`).
fn string_lit() -> impl Parser<char, String, Error = Simple<char>> + Clone {
    just('"')
        .ignore_then(
            filter(|c: &char| *c != '"' && *c != '\n')
                .repeated()
                .collect::<String>(),
        )
        .then_ignore(just('"'))
}

/// `BoolLit ::= 'true' | 'false'`.
fn bool_lit() -> impl Parser<char, bool, Error = Simple<char>> + Clone {
    choice((keyword("true").to(true), keyword("false").to(false)))
}

/// Time literal with unit suffix. Grammar §1: `TimeLit ::= IntLit TimeUnit`,
/// no whitespace between the two. We normalise to nanoseconds at parse
/// time (see [`TimeLit`]).
///
/// Restricting unit order longest-first prevents `ms` being matched as
/// `m` then `s` (no `m` unit exists, but the lexer must still pick the
/// right one; see also `size_lit`).
fn time_lit() -> impl Parser<char, TimeLit, Error = Simple<char>> + Clone {
    let unit = choice((
        just("ns").to(TimeUnit::Ns),
        just("us").to(TimeUnit::Us),
        just("ms").to(TimeUnit::Ms),
        just("s").to(TimeUnit::S),
    ))
    // Same trick as `keyword`: ensure the unit is not the prefix of a
    // longer identifier (e.g. `10msX` must NOT parse as `10ms` + `X`,
    // because that would silently swallow a typo). Without this, the
    // negative test `latency_max=10minutes` would parse `10m` + ...
    // and not fail at the unit. The `s` unit is also the prefix of
    // longer identifiers (e.g. `seconds`); the rewind catches them.
    .then_ignore(
        none_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_").rewind(),
    );

    int_lit().then(unit).try_map(|(value, unit), span| {
        let nanos = value.checked_mul(unit.nanos_per_unit()).ok_or_else(|| {
            Simple::custom(
                span,
                format!("time literal {}{:?} overflows u64 ns", value, unit),
            )
        })?;
        Ok(TimeLit {
            nanos,
            original_unit: unit,
            original_value: value,
        })
    })
}

/// Size literal with optional unit suffix. Grammar §1:
/// `SizeLit ::= IntLit SizeUnit?`, units are binary (`KB = 1024 B`).
fn size_lit() -> impl Parser<char, u64, Error = Simple<char>> + Clone {
    // Order matters in `choice`: `GB`, `MB`, `KB` first; the bare `B`
    // last so it doesn't shadow the multi-char suffixes.
    let unit = choice((
        just("GB").to(1024u64 * 1024 * 1024),
        just("MB").to(1024u64 * 1024),
        just("KB").to(1024u64),
        just("B").to(1u64),
    ))
    .then_ignore(
        none_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_").rewind(),
    );

    int_lit().then(unit.or_not()).try_map(|(value, mul), span| {
        let mul = mul.unwrap_or(1);
        value.checked_mul(mul).ok_or_else(|| {
            Simple::custom(
                span,
                format!("size literal {}*{} overflows u64", value, mul),
            )
        })
    })
}

// --------------------------------------------------------------------
// Worker topology
// --------------------------------------------------------------------

/// `SimdSpec ::= 'none' | Ident`.
fn simd_spec() -> impl Parser<char, SimdSpec, Error = Simple<char>> + Clone {
    let none_ = keyword("none").to(SimdSpec::None);
    // Note: `ident` rejects keywords, including `none`. So the `none_`
    // alternative must run first. The SIMD name is backend-interpreted
    // and never independently diagnosed by `SchedLowerError`, so we
    // keep it a bare `String` (see `crate::span` granularity docs) and
    // drop the identifier span here.
    choice((none_, ident().map(|sp| SimdSpec::Named(sp.node))))
}

/// `MemoryAtom ::= 'shared' | Ident ('[' SizeLit ']')?`.
fn memory_atom() -> impl Parser<char, MemoryAtom, Error = Simple<char>> + Clone {
    let shared = keyword("shared").to(MemoryAtom::Shared);
    let named = ident()
        .then_ignore(comment_or_ws())
        .then(
            pad(just('['))
                .ignore_then(pad(size_lit()))
                .then_ignore(pad(just(']')))
                .or_not(),
        )
        // The memory-atom name is never independently name-resolved by
        // `SchedLowerError`, so it stays a bare `String` (see
        // `crate::span` granularity docs); drop the identifier span.
        .map(|(name, size)| MemoryAtom::Named {
            name: name.node,
            size_bytes: size,
        });
    choice((shared, named))
}

/// `MemorySpec ::= MemoryAtom ('+' MemoryAtom)*`.
fn memory_spec() -> impl Parser<char, MemorySpec, Error = Simple<char>> + Clone {
    pad(memory_atom())
        .separated_by(pad(just('+')))
        .at_least(1)
        .map(|atoms| MemorySpec { atoms })
}

/// `worker_class IDENT { ClassField* };`.
fn worker_class_decl() -> impl Parser<char, WorkerClassDecl, Error = Simple<char>> + Clone {
    enum CField {
        Simd(SimdSpec),
        Memory(MemorySpec),
    }

    let simd_field = pad(keyword("simd"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(simd_spec()))
        .then_ignore(pad(just(';')))
        .map(CField::Simd);

    let memory_field = pad(keyword("memory"))
        .ignore_then(pad(just('=')))
        .ignore_then(memory_spec())
        .then_ignore(pad(just(';')))
        .map(CField::Memory);

    let field = choice((simd_field, memory_field));

    // Ends at the *bare* `;` (no trailing `pad`) so the
    // `padded_spanned` wrap in `directive_parser` fixes the directive
    // span tight at the terminator; trailing layout is consumed
    // off-span. Mirrors the algorithm parser's decl arms (TASK-0086).
    pad(keyword("worker_class"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just('{')))
        .then(field.repeated())
        .then_ignore(pad(just('}')))
        .then_ignore(just(';'))
        .map(|(name, fields)| {
            let mut decl = WorkerClassDecl {
                name,
                simd: None,
                memory: None,
            };
            for f in fields {
                match f {
                    // Duplicate-field detection is a linker-pass
                    // concern; last write wins at parse time.
                    CField::Simd(s) => decl.simd = Some(s),
                    CField::Memory(m) => decl.memory = Some(m),
                }
            }
            decl
        })
}

/// `memory_region IDENT { RegionField* };`.
fn memory_region_decl() -> impl Parser<char, MemoryRegionDecl, Error = Simple<char>> + Clone {
    enum RField {
        Size(u64),
        // `accessible_by` names are `SpName` so an undeclared-name
        // error (TASK-0196) can underline the offending token.
        AccessibleBy(Vec<SpName>),
        PerWorker(bool),
    }

    let size_field = pad(keyword("size"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(size_lit()))
        .then_ignore(pad(just(';')))
        .map(RField::Size);

    let accessible_field = pad(keyword("accessible_by"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(just('{')))
        .ignore_then(pad(ident()).separated_by(pad(just(','))).allow_trailing())
        .then_ignore(pad(just('}')))
        .then_ignore(pad(just(';')))
        .map(RField::AccessibleBy);

    let per_worker_field = pad(keyword("per_worker"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(bool_lit()))
        .then_ignore(pad(just(';')))
        .map(RField::PerWorker);

    let field = choice((size_field, accessible_field, per_worker_field));

    // Bare `;` terminator — see `worker_class_decl` (TASK-0086).
    pad(keyword("memory_region"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just('{')))
        .then(field.repeated())
        .then_ignore(pad(just('}')))
        .then_ignore(just(';'))
        .map(|(name, fields)| {
            let mut decl = MemoryRegionDecl {
                name,
                size_bytes: None,
                accessible_by: None,
                per_worker: None,
            };
            for f in fields {
                match f {
                    RField::Size(b) => decl.size_bytes = Some(b),
                    RField::AccessibleBy(ids) => decl.accessible_by = Some(ids),
                    RField::PerWorker(b) => decl.per_worker = Some(b),
                }
            }
            decl
        })
}

/// `workers = WorkersSet ;`.
///
/// Two surface forms (simple, typed) per grammar §1; collapsed to a
/// uniform [`WorkersDecl`] of [`WorkerEntry`] (see `sched/ast.rs`).
/// The parser disambiguates by checking whether the first non-trivial
/// element contains `:`.
fn workers_decl() -> impl Parser<char, WorkersDecl, Error = Simple<char>> + Clone {
    let typed_entry = pad(ident())
        .then_ignore(pad(just(':')))
        .then(pad(ident()))
        .map(|(name, class)| WorkerEntry {
            name,
            class: Some(class),
        });

    let simple_entry = pad(ident()).map(|name| WorkerEntry { name, class: None });

    // Grammar gives two alternatives at the top:
    //   SimpleWorkerList | TypedWorkerList
    // We try typed first because its trailing `:` distinguishes it
    // from the simple form, and chumsky's `choice` is left-biased.
    let typed_list = typed_entry
        .clone()
        .separated_by(pad(just(',')))
        .allow_trailing()
        .at_least(1);

    let simple_list = simple_entry
        .separated_by(pad(just(',')))
        .allow_trailing()
        .at_least(1);

    let empty_set = pad(just('{')).then(pad(just('}'))).map(|_| Vec::new());

    let nonempty_set = pad(just('{'))
        .ignore_then(typed_list.or(simple_list))
        .then_ignore(pad(just('}')));

    let set = choice((nonempty_set, empty_set));

    // Bare `;` terminator — see `worker_class_decl` (TASK-0086).
    pad(keyword("workers"))
        .ignore_then(pad(just('=')))
        .ignore_then(set)
        .then_ignore(just(';'))
        .map(|entries| WorkersDecl { entries })
}

// --------------------------------------------------------------------
// Placement
// --------------------------------------------------------------------

/// `place IDENT on PlaceTarget;`.
fn place_directive() -> impl Parser<char, PlaceDirective, Error = Simple<char>> + Clone {
    // PlaceTarget: single ident, OR `{ id, id, ... }` with one or
    // more idents. The empty `{ }` form is rejected here at the
    // grammar level — see the negative test `place X on { }`.
    let many = pad(just('{'))
        .ignore_then(
            pad(ident())
                .separated_by(pad(just(',')))
                .allow_trailing()
                .at_least(1),
        )
        .then_ignore(pad(just('}')))
        .map(PlaceTarget::Many);
    let one = pad(ident()).map(PlaceTarget::One);
    let target = choice((many, one));

    // Bare `;` terminator — see `worker_class_decl` (TASK-0086).
    pad(keyword("place"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(keyword("on")))
        .then(target)
        .then_ignore(just(';'))
        .map(|(kernel, target)| PlaceDirective { kernel, target })
}

/// `place_data IDENT in IDENT;`.
fn place_data_directive() -> impl Parser<char, PlaceDataDirective, Error = Simple<char>> + Clone {
    // Bare `;` terminator — see `worker_class_decl` (TASK-0086).
    pad(keyword("place_data"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(keyword("in")))
        .then(pad(ident()))
        .then_ignore(just(';'))
        .map(|(data, region)| PlaceDataDirective { data, region })
}

// --------------------------------------------------------------------
// Loop transformations
// --------------------------------------------------------------------

/// `LoopOpt`.
fn loop_option() -> impl Parser<char, LoopOption, Error = Simple<char>> + Clone {
    let with_int = |kw: &'static str, ctor: fn(u64) -> LoopOption| {
        pad(keyword(kw))
            .ignore_then(pad(just('=')))
            .ignore_then(pad(int_lit()))
            .map(ctor)
    };
    let partition_kind = choice((
        keyword("rows").to(PartitionKind::Rows),
        keyword("blocks2d").to(PartitionKind::Blocks2d),
        keyword("workers").to(PartitionKind::Workers),
    ));
    let partition = pad(keyword("partition"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(partition_kind))
        .map(LoopOption::Partition);

    choice((
        with_int("block", LoopOption::Block),
        with_int("vectorize", LoopOption::Vectorize),
        with_int("unroll", LoopOption::Unroll),
        with_int("pipeline", LoopOption::Pipeline),
        partition,
        pad(keyword("reuse")).to(LoopOption::Reuse),
    ))
}

/// `loop IDENT : LoopOpt (, LoopOpt)*;`.
fn loop_directive() -> impl Parser<char, LoopDirective, Error = Simple<char>> + Clone {
    // Bare `;` terminator — see `worker_class_decl` (TASK-0086).
    pad(keyword("loop"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just(':')))
        .then(loop_option().separated_by(pad(just(','))).at_least(1))
        .then_ignore(just(';'))
        .map(|(var, options)| LoopDirective { var, options })
}

// --------------------------------------------------------------------
// Transfer / IO semantics
// --------------------------------------------------------------------

fn transfer_option() -> impl Parser<char, TransferOption, Error = Simple<char>> + Clone {
    let notify_kind = choice((
        keyword("event").to(NotifyKind::Event),
        keyword("poll").to(NotifyKind::Poll),
    ));
    let notify = pad(keyword("notify"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(notify_kind))
        .map(TransferOption::Notify);
    let buffer = pad(keyword("buffer"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(int_lit()))
        .map(TransferOption::Buffer);

    choice((
        pad(keyword("sync")).to(TransferOption::Sync),
        pad(keyword("async")).to(TransferOption::Async),
        buffer,
        notify,
    ))
}

/// `transfer IDENT : XferOpt (, XferOpt)*;`.
fn transfer_directive() -> impl Parser<char, TransferDirective, Error = Simple<char>> + Clone {
    // Bare `;` terminator — see `worker_class_decl` (TASK-0086).
    pad(keyword("transfer"))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just(':')))
        .then(transfer_option().separated_by(pad(just(','))).at_least(1))
        .then_ignore(just(';'))
        .map(|(data, options)| TransferDirective { data, options })
}

// --------------------------------------------------------------------
// Runtime assertions
// --------------------------------------------------------------------

fn check_assert() -> impl Parser<char, CheckAssert, Error = Simple<char>> + Clone {
    let latency = pad(keyword("latency_max"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(time_lit()))
        .map(CheckAssert::LatencyMax);

    let violation_kind = choice((
        keyword("panic").to(ViolationKind::Panic),
        keyword("log").to(ViolationKind::Log),
        keyword("count").to(ViolationKind::Count),
    ));
    let on_violation = pad(keyword("on_violation"))
        .ignore_then(pad(just('=')))
        .ignore_then(pad(violation_kind))
        .map(CheckAssert::OnViolation);

    choice((latency, on_violation))
}

/// `check loop IDENT : CheckAssert (, CheckAssert)*;`.
///
/// The PRD-mandated form (PRD §6.3.5). The `loop` qualifier is
/// mandatory, not optional: keeping the `check`-qualifier slot
/// distinct reserves room for a future `check transfer X : ...;`
/// without a grammar break (TASK-0079 chose this over relaxing
/// `loop` to optional). All example schedules conform.
fn check_directive() -> impl Parser<char, CheckDirective, Error = Simple<char>> + Clone {
    // Bare `;` terminator — see `worker_class_decl` (TASK-0086).
    pad(keyword("check"))
        .ignore_then(pad(keyword("loop")))
        .ignore_then(pad(ident()))
        .then_ignore(pad(just(':')))
        .then(check_assert().separated_by(pad(just(','))).at_least(1))
        .then_ignore(just(';'))
        .map(|(var, asserts)| CheckDirective { var, asserts })
}

// --------------------------------------------------------------------
// Top level
// --------------------------------------------------------------------

/// Parse one schedule directive, wrapped in its tight source span.
///
/// Each inner directive parser ends at its *bare* `;` terminator (no
/// trailing `pad`); `padded_spanned` then fixes the directive's
/// [`SpDirective`] span at exactly `[keyword .. ';']` and consumes the
/// trailing layout *off-span*. This mirrors the algorithm parser's
/// `item_parser` (TASK-0086): a naive `pad(p).map_with_span` would
/// swallow the newline after `;` into the directive span and
/// mislocate a directive-level diagnostic (TASK-0196).
fn directive_parser() -> impl Parser<char, SpDirective, Error = Simple<char>> + Clone {
    let bare = choice((
        worker_class_decl().map(Directive::WorkerClass),
        memory_region_decl().map(Directive::MemoryRegion),
        workers_decl().map(Directive::Workers),
        place_data_directive().map(Directive::PlaceData),
        place_directive().map(Directive::Place),
        loop_directive().map(Directive::Loop),
        transfer_directive().map(Directive::Transfer),
        check_directive().map(Directive::Check),
    ));
    padded_spanned(bare)
}

/// `Program ::= 'schedule' 'for' StringLit '{' SchedItem* '}'`.
fn program_parser() -> impl Parser<char, SchedAst, Error = Simple<char>> {
    comment_or_ws()
        .ignore_then(pad(keyword("schedule")))
        .ignore_then(pad(keyword("for")))
        .ignore_then(pad(string_lit()))
        .then_ignore(pad(just('{')))
        .then(directive_parser().padded_by(comment_or_ws()).repeated())
        .then_ignore(pad(just('}')))
        .then_ignore(comment_or_ws())
        .then_ignore(end())
        .map(|(algo_path, directives)| SchedAst {
            algo_path,
            directives,
        })
}
