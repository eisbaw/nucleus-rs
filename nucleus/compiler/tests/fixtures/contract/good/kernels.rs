// Positive contract fixture (TASK-0012).
//
// Matches algo.nuc: same kernel names, arities, scalar types, all
// `pub`. Bodies are arbitrary; the contract checker doesn't read
// them beyond signature parsing + a rustc invocation that ensures
// the whole file compiles.

pub fn add(a: f32, b: f32) -> f32 {
    a + b
}

pub fn scale_clip(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

pub fn emit_log(_x: f32) {
    // unit return; the `-> ()` form on the Nuc side matches a
    // missing or explicit unit return on the Rust side.
}

pub fn pi() -> f64 {
    std::f64::consts::PI
}
