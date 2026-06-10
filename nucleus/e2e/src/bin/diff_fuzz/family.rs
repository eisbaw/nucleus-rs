//! Per-family generators + source emitters for the diff-fuzz subclass.
//!
//! Each submodule owns ONE structured family: its parameter struct, its
//! seed-deterministic `generate`, its `bundle` (algo/sched/kernels/input +
//! the in-process reference), and a one-line `describe`. The dispatch over
//! families lives in [`crate::program`]. Splitting per family keeps every
//! file well under the 1000-LoC mega-file fence and keeps each shape's
//! emission auditable in isolation.

pub(crate) mod io_scaffold;
pub(crate) mod partition;
pub(crate) mod pipeline;
pub(crate) mod reduction;
pub(crate) mod stencil;
pub(crate) mod until;
