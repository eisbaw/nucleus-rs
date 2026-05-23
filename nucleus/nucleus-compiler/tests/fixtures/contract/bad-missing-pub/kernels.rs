// Not `pub`. The PRD §6.2.2 example uses `pub fn ...`; the codegen
// wrapper imports kernels by path and so requires visibility.
fn add(a: f32, b: f32) -> f32 {
    a + b
}
