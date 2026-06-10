//! Generative property-based cross-backend differential testing harness
//! for the Nucleus compiler (TASK-0453.01).
//!
//! # Why this exists
//!
//! The standing differential rig (`nucleus-e2e`) runs a CURATED corpus of
//! 27 fixed examples. Chapter 10 of the thesis concedes the sharpest gap in
//! the validation story: "the specification IS the corpus", so coverage
//! bounds the claim. The fix the thesis itself names is a property-based
//! harness that SYNTHESISES affine, single-assignment, integer programs and
//! checks cross-backend byte-identity over them. This binary is that
//! harness.
//!
//! It does NOT change any correctness guarantee — it only ADDS evidence by
//! sampling a (structured, provably-compilable) subclass of the program
//! space and asserting that all seven tier-1 backends, plus an in-process
//! Rust reference, agree byte-for-byte.
//!
//! # What it does, per generated program
//!
//!   1. GENERATE a random program (algo + schedule + kernels.rs) from the
//!      subclass below into a fresh scratch directory under
//!      `nucleus/target/diff-fuzz/` (so `cargo clean` sweeps it).
//!   2. COMPILE it across all 7 tier-1 backends via the `nucleus` driver
//!      (`cargo run --bin nucleus -- build ...`) then `cargo build
//!      --release` in each emitted project.
//!   3. RUN each emitted artefact against the generated `input.bin`.
//!   4. ASSERT mutual byte-identity across all 7 backend outputs AND
//!      agreement with an in-process Rust reference computed directly from
//!      the generated program. SCOPE OF THE REFERENCE (be precise — this
//!      feeds the thesis threats-to-validity): the reference guards against
//!      COMPILER common-mode failure — all seven backends mistranslating
//!      the SAME kernel identically. It does NOT guard against
//!      SPECIFICATION common-mode: `Op::apply` (the reference) and
//!      `Op::kernel_body` (the emitted kernel) are two transcriptions of
//!      the same `Op` variant in this file, so a conceptual error in an
//!      op's definition would appear identically in both and escape. This
//!      is the same author-intent common-mode bound the thesis already
//!      states for the hand-written corpus oracles (ch10 W4).
//!   5. On ANY divergence / compile-failure / run-failure: print the SEED,
//!      the full generated program, and exactly which backend diverged with
//!      a byte diff, then exit non-zero. Honest-failure discipline: a
//!      failure is never masked.
//!
//! # The generated SUBCLASS (honest residual)
//!
//! This harness generates exactly ONE structural family, modelled on the
//! proven cross-worker shape of `nuc-nucleus/examples/02-split-add`:
//!
//!   - 1-D ELEMENTWISE INTEGER PIPELINES over a random array length `N`.
//!   - A random-length pipeline of `S` stages. Stage 0 reads the two input
//!     arrays `a` and `b`; each later stage reads the previous stage's
//!     output array and `b`. Every stage is a pure scalar `i32` kernel.
//!   - Each stage's op is drawn from a bit-deterministic integer set:
//!     wrapping_add / wrapping_sub / wrapping_mul / bitand / bitor / bitxor
//!     / min / max / affine `x*k+m` (random constants).
//!   - Random i32 input values, written to `input.bin` in the same LE
//!     layout the kernels read.
//!   - A `host + w0` split schedule with one `transfer` per crossing data
//!     symbol — exactly like `split.sched.nuc`. The compute loop(s) cross a
//!     REAL worker boundary, so the differential is meaningful.
//!
//! It does NOT yet generate: 2-D / stencil / halo shapes, reductions,
//! prefix scans, data-dependent gather/scatter, multi-compute-worker
//! partition (`place ... on {w0,w1}` / `partition=workers`), blocking /
//! vectorising / buffered transfer directives, or floating point. Those are
//! deliberately out of scope here: the MVP is a subclass we can PROVE
//! compiles green across all 7 backends rather than a broad-but-flaky
//! generator. Extending the subclass is future work (and the honest
//! residual the thesis update should state).
//!
//! # Known limitations of the harness itself
//!
//!   - NO per-command timeout. A backend that DEADLOCKS (the exact failure
//!     class the soundness gate targets) would HANG this harness rather
//!     than failing loud — a hang reads as "still running", not "FAIL".
//!     The compile-time soundness gate is what actually guards against
//!     deadlock; this differential harness is a value-correctness
//!     instrument, not a liveness one. A wall-clock timeout converting a
//!     hang into a reported failure is filed as follow-up (TASK-0453.01
//!     residual).
//!   - The generated subclass is single-cross-worker-boundary (host + w0);
//!     a multi-compute-worker partition form is the most pointed extension.
//!
//! # Determinism
//!
//! Seeded splitmix64 RNG; same seed => same programs => same result. No
//! wall-clock or unseeded randomness enters program generation. The seed is
//! printed at start. `K` (program count) and `seed` are CLI flags.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

// --------------------------------------------------------------------
// Seeded RNG — splitmix64. Tiny, dependency-free, deterministic.
// --------------------------------------------------------------------

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Rng { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        // splitmix64 (public-domain reference constants).
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Approximately uniform in `[lo, hi]` inclusive. Requires `lo <= hi`.
    /// NOTE: modulo reduction is biased for spans that do not divide 2^64;
    /// for the tiny spans used here (3, 4, ~505) the bias is negligible and
    /// does not affect the differential property. Not a CSPRNG.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        debug_assert!(lo <= hi);
        let span = hi - lo + 1;
        lo + (self.next_u64() % span)
    }

    fn i32_value(&mut self) -> i32 {
        self.next_u64() as i32
    }

    fn choice<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.range(0, (items.len() - 1) as u64) as usize]
    }
}

// --------------------------------------------------------------------
// The generated program model.
// --------------------------------------------------------------------

/// One scalar i32 op. `apply` is the in-process reference oracle; it MUST
/// match the Rust body emitted into kernels.rs exactly (both are written
/// from the same source of truth — see `Op::kernel_body`).
#[derive(Clone, Copy)]
enum Op {
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    BitAnd,
    BitOr,
    BitXor,
    Min,
    Max,
    /// affine: `x * k + m` on the FIRST argument (the second is ignored so
    /// the stage signature stays uniformly `(i32, i32) -> i32`).
    Affine(i32, i32),
}

impl Op {
    const SIMPLE: [Op; 8] = [
        Op::WrappingAdd,
        Op::WrappingSub,
        Op::WrappingMul,
        Op::BitAnd,
        Op::BitOr,
        Op::BitXor,
        Op::Min,
        Op::Max,
    ];

    /// In-process reference. `wrapping_*` so overflow is two's-complement
    /// deterministic, matching the emitted kernel bodies (PRD §10.1).
    fn apply(&self, x: i32, y: i32) -> i32 {
        match self {
            Op::WrappingAdd => x.wrapping_add(y),
            Op::WrappingSub => x.wrapping_sub(y),
            Op::WrappingMul => x.wrapping_mul(y),
            Op::BitAnd => x & y,
            Op::BitOr => x | y,
            Op::BitXor => x ^ y,
            Op::Min => x.min(y),
            Op::Max => x.max(y),
            Op::Affine(k, m) => x.wrapping_mul(*k).wrapping_add(*m),
        }
    }

    /// The Rust expression body for a kernel `fn(a: i32, b: i32) -> i32`.
    /// Identical arithmetic to `apply` — the single source of truth is the
    /// op variant, so a divergence between this and `apply` would be a bug
    /// in THIS file, not the compiler. Keeping them adjacent makes that
    /// audit one screenful.
    fn kernel_body(&self) -> String {
        match self {
            Op::WrappingAdd => "a.wrapping_add(b)".to_string(),
            Op::WrappingSub => "a.wrapping_sub(b)".to_string(),
            Op::WrappingMul => "a.wrapping_mul(b)".to_string(),
            Op::BitAnd => "a & b".to_string(),
            Op::BitOr => "a | b".to_string(),
            Op::BitXor => "a ^ b".to_string(),
            Op::Min => "a.min(b)".to_string(),
            Op::Max => "a.max(b)".to_string(),
            // `b` is named in the signature but unused for affine; prefix
            // with `_` is not possible (signature is fixed), so reference
            // it in a no-op to avoid an unused-variable warning in the
            // generated crate (which builds with default warnings).
            Op::Affine(k, m) => {
                format!("{{ let _ = b; a.wrapping_mul({k}).wrapping_add({m}) }}")
            }
        }
    }

    fn random(rng: &mut Rng) -> Op {
        // ~1-in-3 chance of an affine op (with random constants); else a
        // simple two-arg op.
        if rng.range(0, 2) == 0 {
            Op::Affine(rng.i32_value(), rng.i32_value())
        } else {
            *rng.choice(&Op::SIMPLE)
        }
    }
}

/// A generated 1-D elementwise pipeline program.
struct Program {
    /// The RNG state SNAPSHOT taken immediately before this program was
    /// generated. Reproduce exactly this program with
    /// `diff-fuzz --prog-seed <this>` (which sets the stream to this state
    /// and generates a single program). This is a true per-program
    /// reproducer — distinct from the run-wide `--seed`, which only
    /// reproduces the whole K-program sequence from the start.
    seed: u64,
    n: usize,
    /// One op per pipeline stage. `stages.len() >= 1`.
    stages: Vec<Op>,
    a: Vec<i32>,
    b: Vec<i32>,
}

impl Program {
    fn generate(seed: u64, rng: &mut Rng) -> Program {
        // Keep N modest: each program does 7x `cargo build --release`, and
        // the array sizes are dwarfed by build cost anyway. A range that
        // still exercises non-trivial loop bounds.
        let n = rng.range(8, 512) as usize;
        let n_stages = rng.range(1, 4) as usize;
        let stages: Vec<Op> = (0..n_stages).map(|_| Op::random(rng)).collect();
        let a: Vec<i32> = (0..n).map(|_| rng.i32_value()).collect();
        let b: Vec<i32> = (0..n).map(|_| rng.i32_value()).collect();
        Program {
            seed,
            n,
            stages,
            a,
            b,
        }
    }

    /// The names of the per-stage output arrays. Stage `i` writes `s{i}`;
    /// the final stage's array is the program output.
    fn stage_out(&self, i: usize) -> String {
        format!("s{i}")
    }

    fn output_array(&self) -> String {
        self.stage_out(self.stages.len() - 1)
    }

    /// Compute the expected output bytes (LE i32), the reference oracle.
    /// Stage 0: `s0[i] = op0(a[i], b[i])`.
    /// Stage k>0: `sk[i] = opk(s{k-1}[i], b[i])`.
    fn reference_output(&self) -> Vec<u8> {
        let mut prev = self.a.clone();
        for op in &self.stages {
            let mut cur = vec![0i32; self.n];
            for i in 0..self.n {
                cur[i] = op.apply(prev[i], self.b[i]);
            }
            prev = cur;
        }
        let mut bytes = Vec::with_capacity(self.n * 4);
        for v in &prev {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    /// `input.bin` layout: array `a` (N words) then array `b` (N words),
    /// little-endian i32 — exactly what the generated kernels read.
    fn input_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.n * 8);
        for v in &self.a {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for v in &self.b {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    /// Human-readable dump for failure reports (and a compact summary
    /// line on success).
    fn describe(&self) -> String {
        let mut s = format!("seed={} N={} stages=[", self.seed, self.n);
        for (i, op) in self.stages.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&op.kernel_body());
        }
        s.push(']');
        s
    }

    // ---- Source emission -------------------------------------------

    fn algo_src(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "// GENERATED by diff_fuzz (seed={}). 1-D elementwise integer pipeline.",
            self.seed
        );
        let _ = writeln!(s, "const N : usize = {};", self.n);
        let _ = writeln!(s);
        let _ = writeln!(s, "data a : i32[N];");
        let _ = writeln!(s, "data b : i32[N];");
        for i in 0..self.stages.len() {
            let _ = writeln!(s, "data {} : i32[N];", self.stage_out(i));
        }
        let _ = writeln!(s);
        for i in 0..self.stages.len() {
            let _ = writeln!(s, "kernel stage{i} : (i32, i32) -> i32 pure;");
        }
        let _ = writeln!(s, "kernel load_input   : ()       -> i32[N] effectful;");
        let _ = writeln!(s, "kernel load_input_b : ()       -> i32[N] effectful;");
        let _ = writeln!(s, "kernel save_output  : (i32[N]) -> ()     effectful;");
        let _ = writeln!(s);
        let _ = writeln!(s, "a <-- load_input();");
        let _ = writeln!(s, "b <-- load_input_b();");
        let _ = writeln!(s);
        for i in 0..self.stages.len() {
            let src = if i == 0 {
                "a".to_string()
            } else {
                self.stage_out(i - 1)
            };
            let _ = writeln!(s, "for i : 0 .. N {{");
            let _ = writeln!(
                s,
                "    {}[i] <-- stage{i}({src}[i], b[i]);",
                self.stage_out(i)
            );
            let _ = writeln!(s, "}}");
        }
        let _ = writeln!(s);
        let _ = writeln!(s, "save_output({});", self.output_array());
        s
    }

    fn sched_src(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "// GENERATED by diff_fuzz (seed={}).", self.seed);
        let _ = writeln!(s, "// host + w0 split, modelled on 02-split-add.");
        let _ = writeln!(s, "schedule for \"./prog.algo.nuc\" {{");
        let _ = writeln!(s, "    workers = {{ host, w0 }};");
        let _ = writeln!(s);
        let _ = writeln!(s, "    place load_input    on host;");
        let _ = writeln!(s, "    place load_input_b  on host;");
        let _ = writeln!(s, "    place save_output   on host;");
        for i in 0..self.stages.len() {
            let _ = writeln!(s, "    place stage{i}        on w0;");
        }
        let _ = writeln!(s);
        // One transfer per crossing data symbol: a, b (host->w0) and the
        // final output array (w0->host). The intermediate stage arrays
        // (s0..s{n-2}) stay entirely on w0, so they do NOT cross and must
        // NOT carry a transfer (omitting is correct; adding one would be a
        // transfer for a non-crossing symbol).
        let _ = writeln!(s, "    transfer a : sync;");
        let _ = writeln!(s, "    transfer b : sync;");
        let _ = writeln!(s, "    transfer {} : sync;", self.output_array());
        let _ = writeln!(s, "}}");
        s
    }

    fn kernels_src(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "// GENERATED by diff_fuzz (seed={}).", self.seed);
        let _ = writeln!(s, "use std::env;");
        let _ = writeln!(s, "use std::fs;");
        let _ = writeln!(s, "use std::io::Write;");
        let _ = writeln!(s);
        let _ = writeln!(s, "const N: usize = {};", self.n);
        let _ = writeln!(s);
        for (i, op) in self.stages.iter().enumerate() {
            let _ = writeln!(s, "pub fn stage{i}(a: i32, b: i32) -> i32 {{");
            let _ = writeln!(s, "    {}", op.kernel_body());
            let _ = writeln!(s, "}}");
            let _ = writeln!(s);
        }
        let _ = writeln!(s, "pub fn load_input() -> Vec<i32> {{");
        let _ = writeln!(
            s,
            "    let path = env::var(\"NUC_INPUT_PATH\").unwrap_or_else(|_| \"input.bin\".to_string());"
        );
        let _ = writeln!(s, "    read_i32_le_slice(&path, 0, N)");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(s, "pub fn load_input_b() -> Vec<i32> {{");
        let _ = writeln!(
            s,
            "    let path = env::var(\"NUC_INPUT_PATH\").unwrap_or_else(|_| \"input.bin\".to_string());"
        );
        let _ = writeln!(s, "    read_i32_le_slice(&path, N, N)");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(s, "pub fn save_output(data: Vec<i32>) {{");
        let _ = writeln!(s, "    assert_eq!(data.len(), N);");
        let _ = writeln!(
            s,
            "    let path = env::var(\"NUC_OUTPUT_PATH\").unwrap_or_else(|_| \"output.bin\".to_string());"
        );
        let _ = writeln!(s, "    let mut bytes = Vec::with_capacity(data.len() * 4);");
        let _ = writeln!(s, "    for v in &data {{");
        let _ = writeln!(s, "        bytes.extend_from_slice(&v.to_le_bytes());");
        let _ = writeln!(s, "    }}");
        let _ = writeln!(
            s,
            "    let mut f = fs::File::create(&path).expect(\"create output\");"
        );
        let _ = writeln!(s, "    f.write_all(&bytes).expect(\"write output\");");
        let _ = writeln!(s, "}}");
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "fn read_i32_le_slice(path: &str, start: usize, count: usize) -> Vec<i32> {{"
        );
        let _ = writeln!(s, "    let bytes = fs::read(path).expect(\"read input\");");
        let _ = writeln!(s, "    let need = (start + count) * 4;");
        let _ = writeln!(s, "    assert!(bytes.len() >= need);");
        let _ = writeln!(s, "    let mut out = Vec::with_capacity(count);");
        let _ = writeln!(s, "    for i in 0..count {{");
        let _ = writeln!(s, "        let off = (start + i) * 4;");
        let _ = writeln!(s, "        out.push(i32::from_le_bytes([");
        let _ = writeln!(
            s,
            "            bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3],"
        );
        let _ = writeln!(s, "        ]));");
        let _ = writeln!(s, "    }}");
        let _ = writeln!(s, "    out");
        let _ = writeln!(s, "}}");
        s
    }
}

// --------------------------------------------------------------------
// The 7 tier-1 backends and whether each is a single-binary backend.
// --------------------------------------------------------------------

/// (backend name, is_single_binary). `is_single_binary` mirrors the e2e
/// harness's `transport == "shared-memory"` rule: shared-memory backends
/// emit a single `target/release/nuc-generated`; the multi-process
/// (tcp/uds) backends emit a `run.sh` launcher instead.
const BACKENDS: [(&str, bool); 7] = [
    ("pthreads-sync", true),
    ("pthreads-async", true),
    ("openmp-rs", true),
    ("mp-tcp-bufsync", false),
    ("mp-tcp-event", false),
    ("mp-tcp-poll", false),
    ("mp-uds-event", false),
];

// --------------------------------------------------------------------
// Per-program orchestration.
// --------------------------------------------------------------------

struct Failure {
    msg: String,
}

/// Locate the repo's `nucleus/` workspace dir. The binary may be invoked
/// from anywhere; walk up from CARGO_MANIFEST_DIR / cwd looking for the
/// `nucleus/` + `nuc-nucleus/` sibling pair (same rule as the e2e harness).
fn nucleus_ws() -> Result<PathBuf, String> {
    // CARGO_MANIFEST_DIR is `<repo>/nucleus/e2e` at build time; at runtime
    // we prefer cwd-walk so the binary is relocatable.
    let mut dir = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    loop {
        if dir.join("nucleus").is_dir() && dir.join("nuc-nucleus").is_dir() {
            return Ok(dir.join("nucleus"));
        }
        // Also accept being run from inside `nucleus/` itself.
        if dir.file_name().map(|n| n == "nucleus").unwrap_or(false)
            && dir.join("Cargo.toml").is_file()
            && dir
                .parent()
                .map(|p| p.join("nuc-nucleus").is_dir())
                .unwrap_or(false)
        {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err("could not locate repo root (need `nucleus/` + `nuc-nucleus/`)".into());
        }
    }
}

/// Write the three generated source files + input.bin into `gen_dir`.
fn write_program(gen_dir: &Path, prog: &Program) -> Result<(), String> {
    fs::create_dir_all(gen_dir).map_err(|e| format!("mkdir {}: {e}", gen_dir.display()))?;
    let w = |name: &str, content: &str| -> Result<(), String> {
        let p = gen_dir.join(name);
        fs::write(&p, content).map_err(|e| format!("write {}: {e}", p.display()))
    };
    w("prog.algo.nuc", &prog.algo_src())?;
    w("prog.sched.nuc", &prog.sched_src())?;
    w("kernels.rs", &prog.kernels_src())?;
    fs::write(gen_dir.join("input.bin"), prog.input_bytes())
        .map_err(|e| format!("write input.bin: {e}"))?;
    Ok(())
}

/// Tail of combined stderr+stdout for error messages.
fn tail(stderr: &[u8], stdout: &[u8], lines: usize) -> String {
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(stderr));
    if !stdout.is_empty() {
        combined.push('\n');
        combined.push_str(&String::from_utf8_lossy(stdout));
    }
    let v: Vec<&str> = combined.lines().collect();
    let start = v.len().saturating_sub(lines);
    v[start..].join("\n")
}

/// Compile + build + run one backend; return the produced output.bin bytes.
///
/// This re-derives the e2e harness's build/run flow (`main.rs` Phase 1/2/3)
/// rather than calling into it. The duplication is DELIBERATE: a differential
/// oracle that shared its execution harness with the system under comparison
/// would couple the evidence to the thing being validated. The load-bearing
/// details (single-binary-vs-run.sh rule, run.sh args, env vars) were
/// verified to agree with `main.rs`; a future "DRY this up" must not collapse
/// the oracle into the SUT. Disk: each backend builds its own release target
/// tree under `out-<backend>`, so peak is ~7 trees at once (swept per program
/// on success), not 7×K.
fn run_backend(
    ws: &Path,
    gen_dir: &Path,
    backend: &str,
    single_binary: bool,
    input_bin: &Path,
) -> Result<Vec<u8>, String> {
    let out_dir = gen_dir.join(format!("out-{backend}"));
    let _ = fs::remove_dir_all(&out_dir);

    // Phase 1: nucleus build.
    let compile = Command::new("cargo")
        .args(["run", "--quiet", "--bin", "nucleus", "--", "build"])
        .arg("--algo")
        .arg(gen_dir.join("prog.algo.nuc"))
        .arg("--sched")
        .arg(gen_dir.join("prog.sched.nuc"))
        .arg("--kernels")
        .arg(gen_dir.join("kernels.rs"))
        .arg("--backend")
        .arg(backend)
        .arg("--out")
        .arg(&out_dir)
        .current_dir(ws)
        .output()
        .map_err(|e| format!("spawn nucleus build [{backend}]: {e}"))?;
    if !compile.status.success() {
        return Err(format!(
            "[{backend}] nucleus build FAILED:\n{}",
            tail(&compile.stderr, &compile.stdout, 8)
        ));
    }

    // Phase 2: cargo build --release the emitted project.
    let build = Command::new("cargo")
        .args(["build", "--release", "--quiet"])
        .current_dir(&out_dir)
        .output()
        .map_err(|e| format!("spawn cargo build [{backend}]: {e}"))?;
    if !build.status.success() {
        return Err(format!(
            "[{backend}] cargo build FAILED:\n{}",
            tail(&build.stderr, &build.stdout, 8)
        ));
    }

    // Phase 3: run.
    let output_bin = out_dir.join("output.bin");
    let _ = fs::remove_file(&output_bin);
    let run = if single_binary {
        let exe = out_dir.join("target/release/nuc-generated");
        if !exe.exists() {
            return Err(format!(
                "[{backend}] expected nuc-generated at {}",
                exe.display()
            ));
        }
        Command::new(&exe)
            .env("NUC_INPUT_PATH", input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin)
            .output()
    } else {
        let run_sh = out_dir.join("run.sh");
        if !run_sh.exists() {
            return Err(format!("[{backend}] expected run.sh at {}", run_sh.display()));
        }
        Command::new("bash")
            .arg(&run_sh)
            .arg(input_bin)
            .arg(&output_bin)
            .current_dir(&out_dir)
            .env("NUC_INPUT_PATH", input_bin)
            .env("NUC_OUTPUT_PATH", &output_bin)
            .output()
    }
    .map_err(|e| format!("spawn run [{backend}]: {e}"))?;
    if !run.status.success() {
        return Err(format!(
            "[{backend}] run FAILED:\n{}",
            tail(&run.stderr, &run.stdout, 8)
        ));
    }

    fs::read(&output_bin).map_err(|e| format!("[{backend}] read output.bin: {e}"))
}

/// First differing byte offset between two slices of equal interpretation.
fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

/// Run one generated program through all backends + the reference; return
/// Ok(()) on full agreement, Err(Failure) with a fully-reproducing report
/// otherwise.
fn check_program(ws: &Path, gen_dir: &Path, prog: &Program) -> Result<(), Failure> {
    let report = |msg: String| Failure {
        msg: format!(
            "DIVERGENCE / FAILURE\n  {}\n\n--- generated program ---\n{}\n--- prog.algo.nuc ---\n{}\n--- prog.sched.nuc ---\n{}\n--- kernels.rs ---\n{}",
            msg,
            prog.describe(),
            prog.algo_src(),
            prog.sched_src(),
            prog.kernels_src(),
        ),
    };

    if let Err(e) = write_program(gen_dir, prog) {
        return Err(report(format!("could not write program: {e}")));
    }
    let input_bin = gen_dir.join("input.bin");
    let reference = prog.reference_output();

    let mut first_output: Option<(String, Vec<u8>)> = None;
    for (backend, single_binary) in BACKENDS.iter() {
        let out = match run_backend(ws, gen_dir, backend, *single_binary, &input_bin) {
            Ok(o) => o,
            Err(e) => return Err(report(e)),
        };

        // Agreement with the in-process reference (common-mode guard).
        if out != reference {
            let off = first_diff(&out, &reference).unwrap_or(0);
            return Err(report(format!(
                "backend `{backend}` DISAGREES WITH REFERENCE: lengths backend={} ref={}, first differing byte at offset {off}",
                out.len(),
                reference.len()
            )));
        }

        // Mutual byte-identity against the first backend's output.
        match &first_output {
            None => first_output = Some((backend.to_string(), out)),
            Some((first_name, first_bytes)) => {
                if &out != first_bytes {
                    let off = first_diff(&out, first_bytes).unwrap_or(0);
                    return Err(report(format!(
                        "backend `{backend}` DISAGREES WITH `{first_name}`: lengths {}={} {}={}, first differing byte at offset {off}",
                        backend,
                        out.len(),
                        first_name,
                        first_bytes.len()
                    )));
                }
            }
        }
    }
    Ok(())
}

// --------------------------------------------------------------------
// CLI
// --------------------------------------------------------------------

struct Args {
    seed: u64,
    k: u64,
    keep: bool,
    /// If set, set the RNG stream directly to this state and generate
    /// exactly ONE program (a true per-program reproducer; see
    /// `Program::seed`). Overrides `--seed`/`--k`.
    prog_seed: Option<u64>,
}

fn parse_args() -> Result<Args, String> {
    let mut seed = 1u64;
    let mut k = 8u64;
    let mut keep = false;
    let mut prog_seed = None;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--seed" => {
                i += 1;
                seed = argv
                    .get(i)
                    .ok_or("--seed requires a value")?
                    .parse()
                    .map_err(|e| format!("--seed: {e}"))?;
            }
            "--k" => {
                i += 1;
                k = argv
                    .get(i)
                    .ok_or("--k requires a value")?
                    .parse()
                    .map_err(|e| format!("--k: {e}"))?;
            }
            "--prog-seed" => {
                i += 1;
                prog_seed = Some(
                    argv.get(i)
                        .ok_or("--prog-seed requires a value")?
                        .parse()
                        .map_err(|e| format!("--prog-seed: {e}"))?,
                );
            }
            "--keep" => keep = true,
            "-h" | "--help" => {
                eprintln!(
                    "diff_fuzz — generative cross-backend differential fuzzer\n\
                     \n\
                     USAGE: diff_fuzz [--seed N] [--k N] [--prog-seed N] [--keep]\n\
                     \n\
                     --seed N        run-wide RNG seed (default 1); reproduces the\n\
                     \x20               whole K-program sequence.\n\
                     --k N           number of programs to generate (default 8).\n\
                     --prog-seed N   regenerate exactly ONE program from the\n\
                     \x20               per-program seed printed in a failure report.\n\
                     --keep          do not delete per-program scratch on success."
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown flag `{other}`")),
        }
        i += 1;
    }
    Ok(Args {
        seed,
        k,
        keep,
        prog_seed,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("diff_fuzz: {e}");
            return ExitCode::FAILURE;
        }
    };

    let ws = match nucleus_ws() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("diff_fuzz: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Best-effort GC of scratch left by a KILLED earlier run (SIGKILL or
    // ENOSPC mid-build leaves a `seed-*-pid-*` dir, each potentially many
    // release target trees — the documented e2e ENOSPC gotcha, multiplied
    // by 7 emitted projects). Sweep siblings whose pid is no longer alive.
    sweep_dead_scratch(&ws.join("target/diff-fuzz"));

    // Scratch root under nucleus/target/ so `cargo clean` sweeps it.
    let scratch_root = ws
        .join("target/diff-fuzz")
        .join(format!("seed-{}-pid-{}", args.seed, std::process::id()));
    if let Err(e) = fs::create_dir_all(&scratch_root) {
        eprintln!("diff_fuzz: cannot create scratch root: {e}");
        return ExitCode::FAILURE;
    }

    // --prog-seed reproduces a single program: seed the stream to the given
    // state and generate exactly one. Otherwise run the K-program sequence.
    let mut rng = Rng::new(args.seed);
    let k = if let Some(ps) = args.prog_seed {
        rng.state = ps;
        1
    } else {
        args.k
    };

    println!(
        "diff_fuzz: seed={} k={} backends={} scratch={}",
        args.seed,
        k,
        BACKENDS.len(),
        scratch_root.display()
    );

    for idx in 0..k {
        // Snapshot the stream state BEFORE drawing, so the program stamps a
        // true per-program reproducer (`--prog-seed <this>`).
        let prog_seed = rng.state;
        let prog = Program::generate(prog_seed, &mut rng);
        let gen_dir = scratch_root.join(format!("prog-{idx:03}"));
        print!("  [{:>3}/{}] {} ... ", idx + 1, k, prog.describe());
        // Flush so progress is visible before the slow 7x build.
        use std::io::Write as _;
        let _ = std::io::stdout().flush();

        match check_program(&ws, &gen_dir, &prog) {
            Ok(()) => {
                println!("OK (7/7 backends + reference agree)");
                if !args.keep {
                    let _ = fs::remove_dir_all(&gen_dir);
                }
            }
            Err(f) => {
                println!("FAIL");
                eprintln!("\n=========================================================");
                eprintln!("diff_fuzz FAILURE — reproduce THIS program with: --prog-seed {} (scratch retained at {})", prog_seed, gen_dir.display());
                eprintln!("=========================================================");
                eprintln!("{}", f.msg);
                // Retain scratch on failure for debugging regardless of --keep.
                return ExitCode::FAILURE;
            }
        }
    }

    // All passed: sweep the per-run scratch root (the per-program dirs were
    // already removed unless --keep).
    if !args.keep {
        let _ = fs::remove_dir_all(&scratch_root);
    }
    println!(
        "diff_fuzz: ALL {} programs agree byte-for-byte across {} backends + reference (seed={})",
        k,
        BACKENDS.len(),
        args.seed
    );
    ExitCode::SUCCESS
}

/// Best-effort sweep of scratch dirs left by a killed earlier run. A dir is
/// named `seed-<seed>-pid-<pid>`; if `<pid>` is not a live process we remove
/// it. Never touches the current run's dir (its pid is alive). Errors are
/// ignored — this is opportunistic disk hygiene, not a correctness step.
fn sweep_dead_scratch(root: &Path) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return, // root may not exist yet — nothing to sweep.
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Parse the trailing `-pid-<n>` segment.
        let Some(pid_str) = name.rsplit("-pid-").next() else {
            continue;
        };
        if pid_str == name {
            continue; // no `-pid-` marker; not our dir.
        }
        let Ok(pid) = pid_str.parse::<i32>() else {
            continue;
        };
        if !pid_is_alive(pid) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// True if a process with `pid` currently exists. Uses `/proc/<pid>` which is
/// present on the Linux dev/CI environment this harness runs in; if `/proc`
/// is absent we conservatively report "alive" so we never delete a live run's
/// scratch.
fn pid_is_alive(pid: i32) -> bool {
    let proc_root = Path::new("/proc");
    if !proc_root.is_dir() {
        return true; // can't tell — be conservative.
    }
    proc_root.join(pid.to_string()).exists()
}

// --------------------------------------------------------------------
// Unit tests — the reference oracle and source emission are pure, so they
// are cheap to pin without building any artefact.
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_for_a_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn same_seed_same_program() {
        let mut r1 = Rng::new(7);
        let p1 = Program::generate(7, &mut r1);
        let mut r2 = Rng::new(7);
        let p2 = Program::generate(7, &mut r2);
        assert_eq!(p1.n, p2.n);
        assert_eq!(p1.a, p2.a);
        assert_eq!(p1.b, p2.b);
        assert_eq!(p1.algo_src(), p2.algo_src());
        assert_eq!(p1.kernels_src(), p2.kernels_src());
    }

    #[test]
    fn op_apply_matches_intent() {
        assert_eq!(Op::WrappingAdd.apply(2, 3), 5);
        assert_eq!(Op::WrappingMul.apply(i32::MAX, 2), -2); // two's-complement wrap
        assert_eq!(Op::BitXor.apply(0b1010, 0b0110), 0b1100);
        assert_eq!(Op::Min.apply(-5, 3), -5);
        assert_eq!(Op::Affine(3, 1).apply(4, 999), 13); // 4*3+1, second arg ignored
    }

    #[test]
    fn reference_matches_staged_pipeline() {
        // Two-stage pipeline: s0 = a+b, s1 = s0 max b.
        let prog = Program {
            seed: 0,
            n: 3,
            stages: vec![Op::WrappingAdd, Op::Max],
            a: vec![1, 2, 3],
            b: vec![10, -5, 0],
        };
        let out = prog.reference_output();
        // s0 = [11, -3, 3]; s1 = [max(11,10), max(-3,-5), max(3,0)] = [11,-3,3]
        let expect: Vec<i32> = vec![11, -3, 3];
        let mut bytes = Vec::new();
        for v in expect {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(out, bytes);
    }

    #[test]
    fn single_stage_output_array_is_s0() {
        let prog = Program {
            seed: 0,
            n: 1,
            stages: vec![Op::BitOr],
            a: vec![0],
            b: vec![0],
        };
        assert_eq!(prog.output_array(), "s0");
        assert!(prog.sched_src().contains("transfer s0 : sync;"));
        // Intermediate-only arrays do not get a transfer; single stage has
        // none, so only a/b/s0 transfer lines appear.
        assert_eq!(prog.sched_src().matches("transfer").count(), 3);
    }

    #[test]
    fn affine_kernel_body_references_b() {
        // Guards the unused-variable warning fix in the generated crate.
        let body = Op::Affine(2, 3).kernel_body();
        assert!(body.contains("let _ = b"));
    }
}
