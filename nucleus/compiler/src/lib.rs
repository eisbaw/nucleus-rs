//! `compiler` crate — Nucleus v2 pre-compiler library surface.
//!
//! At M0 this exposes the algorithm-sublanguage parser. Subsequent
//! milestones will add typechecking, AlgoIR lowering, schedule parsing,
//! the link step, and codegen. See `nuc-nucleus/PRD.md` §12.2.
//!
//! The public surface is intentionally minimal: re-export `algo` only.
//! Internal modules stay private until a caller needs them.

pub mod algo;
