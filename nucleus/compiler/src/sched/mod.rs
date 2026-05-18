//! Schedule sublanguage (`*.sched.nuc`).
//!
//! Implements the grammar specified in `docs/grammar-sched.md` and
//! `nuc-nucleus/PRD.md` §6.3. The parser is hand-written on top of the
//! `chumsky` combinator library; see `parser.rs` for the choice
//! rationale. The library choice deliberately matches the algorithm
//! parser (`crate::algo`) — same combinator API, same error type, same
//! whitespace/comment handling.
//!
//! Public surface:
//! - [`parse_sched`]: parse a source string into [`SchedAst`].
//! - AST node types ([`SchedAst`], [`Directive`], [`WorkersDecl`],
//!   [`WorkerClassDecl`], [`MemoryRegionDecl`], [`PlaceDirective`],
//!   [`PlaceDataDirective`], [`LoopDirective`], [`TransferDirective`],
//!   [`CheckDirective`], plus the option enums).
//! - [`ParseError`]: failures carry `(line, column)` from the input.
//!   Shared with `crate::algo` via `crate::error` — see TASK-0008
//!   notes on the choice to factor out the type.
//!
//! What this module does NOT do (deliberately):
//! - Resolve identifiers. Forward references for `worker_class` /
//!   `memory_region`, kernel/data names that must match the
//!   algorithm, etc. are semantic-pass concerns (TASK-0010,
//!   TASK-0011).
//! - Reject mutually-exclusive option combinations (`sync` + `async`,
//!   duplicate `block=N`, etc.). Linker-pass concerns; the parser
//!   accepts (grammar §2 notes 5, 7).
//! - Track AST node spans beyond the top-level error position. Adding
//!   per-node spans is a follow-up task — same status as the
//!   algorithm parser.

pub mod ast;
pub mod ir;
pub mod lower;
pub mod parser;

pub use crate::error::{ParseError, ParseErrorKind};
pub use ast::{
    CheckAssert, CheckDirective, Directive, LoopDirective, LoopOption, MemoryAtom,
    MemoryRegionDecl, MemorySpec, NotifyKind, PartitionKind, PlaceDataDirective, PlaceDirective,
    PlaceTarget, SchedAst, SimdSpec, TimeLit, TimeUnit, TransferDirective, TransferOption,
    ViolationKind, WorkerClassDecl, WorkerEntry, WorkersDecl,
};
pub use ir::{
    ResolvedCheckAssert, ResolvedCheckDirective, ResolvedLoopDirective, ResolvedLoopOption,
    ResolvedMemoryRegion, ResolvedPlaceData, ResolvedPlaceTarget, ResolvedPlacement,
    ResolvedTransferDirective, ResolvedTransferOption, ResolvedWorker, ResolvedWorkerClass,
    SchedIR, SchedLowerError, DEFAULT_WORKER_CLASS,
};
pub use lower::lower_sched;
pub use parser::parse_sched;
