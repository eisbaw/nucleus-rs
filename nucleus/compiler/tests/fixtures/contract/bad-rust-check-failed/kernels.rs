// rustc-rejecting body: references an undeclared name. syn can
// still parse this (it's a valid AST), so the contract check
// proceeds to find `add` with the right signature, but rustc
// reports the unresolved name and we surface a RustCheckFailed.
pub fn add(a: f32, b: f32) -> f32 {
    a + b + undefined_symbol
}
