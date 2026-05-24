// TASK-0274 negative-fixture kernels. The CLI test never reaches code
// generation (the build fails at reuse_inference per TASK-0271 cycle
// 88), so these are scaffolding only — `nucleus build` requires
// --kernels to point at a file but does not invoke any of them in
// the failure path the test exercises.

pub fn pass(x: i32) -> i32 {
    x
}

pub fn load_src() -> Vec<i32> {
    vec![0; 4]
}

pub fn save_dst(_v: Vec<i32>) {}
