#!/usr/bin/env bash
# Fixture + cell helpers for scripts/perf-study-run.sh (TASK-0455.04).
#
# Sourced by the runner. Each gen_<example>_fixture function writes a
# PARAMETERIZED copy of a committed corpus example into a scratch dir:
# the algorithm (prog.algo.nuc), the kernel bodies (kernels.rs), an
# INDEPENDENT std-only reference oracle, and an INDEPENDENT std-only
# input generator — all sized by the function's argument. The arithmetic
# is identical to the committed sibling; only the dimension constants
# (and, for schedule variants, the workers list) change. Nothing under
# nuc-nucleus/examples/ or docs/case-study/ is edited.
#
# The reference oracle and generator are deliberately RE-DERIVED from the
# committed siblings' arithmetic (not byte-copied) only where the
# committed version hardcodes a size; the load-bearing arithmetic
# (matmul wrapping madd, blur3 wrapping sum /9, reduction wrapping sum)
# is reproduced faithfully so byte-identity against the generated
# reference is the same correctness claim the corpus makes.
#
# Each build_cell / run_* helper builds the generated project inside the
# already-active dev shell (cargo on PATH via the just recipe), points it
# at the generated input, and asserts byte-identity vs the generated
# reference BEFORE recording any timing.

# ---------------------------------------------------------------------
# Shared build helpers. nucleus_emit runs the compiler (codegen only);
# cargo_build_emitted compiles the generated Rust. build_project does
# both (the common path). Splitting them lets run_dist_cell read the
# emitted NUC_SO_BUF (the backend's authoritative socket-buffer request)
# from run.sh BEFORE paying the cargo cost on a cell the wmem cap will
# block anyway. All three hard-fail on error.
# ---------------------------------------------------------------------
nucleus_emit() {
    local algo="$1" sched="$2" kernels="$3" backend="$4" out="$5"
    rm -rf "$out"
    "$DRIVER" build --algo "$algo" --sched "$sched" --kernels "$kernels" \
        --backend "$backend" --out "$out" >/dev/null 2>"$out.build.log" || {
        echo "perf-study: FAIL — nucleus build $backend failed:" >&2
        sed 's/^/    /' "$out.build.log" >&2
        exit 1
    }
}

cargo_build_emitted() {
    local out="$1" backend="$2"
    ( cd "$out" && cargo build --release --quiet 2>"$out.cargo.log" ) || {
        echo "perf-study: FAIL — cargo build of generated $backend project failed:" >&2
        tail -20 "$out.cargo.log" >&2
        exit 1
    }
}

build_project() {
    local algo="$1" sched="$2" kernels="$3" backend="$4" out="$5"
    nucleus_emit "$algo" "$sched" "$kernels" "$backend" "$out"
    cargo_build_emitted "$out" "$backend"
}

# run_binary_into OUTFILE INPUT DIR BACKEND  -> runs the cell once,
# writing its output to OUTFILE. Single-binary backends run the one
# binary; multi-binary (mp-tcp-*) backends run via the emitted run.sh.
# This is the CORRECTNESS run (untimed); it deliberately uses the emitted
# run.sh (which carries a no-op `cargo build` line) so every binary is
# definitely built before any timing happens. The TIMED path (timed_cmd)
# uses a cargo-stripped run-timed.sh — see strip_cargo_from_run_sh.
run_binary_into() {
    local outfile="$1" input="$2" dir="$3" backend="$4"
    case "$backend" in
        mp-tcp-*)
            NUC_INPUT_PATH="$input" NUC_OUTPUT_PATH="$outfile" \
                timeout 300 bash "$dir/run.sh" "$input" "$outfile" >/dev/null 2>&1
            ;;
        *)
            NUC_INPUT_PATH="$input" NUC_OUTPUT_PATH="$outfile" \
                timeout 300 "$dir/target/release/nuc-generated" >/dev/null 2>&1
            ;;
    esac
}

# strip_cargo_from_run_sh DIR  -> writes DIR/run-timed.sh: a copy of the
# emitted DIR/run.sh with its single `(cd "$here" && cargo build ...)`
# line removed, so the TIMED launch contains no cargo invocation at all.
#
# WHY (TASK-0455.04 P1 remediation): the emitted run.sh runs
# `(cd "$here" && cargo build --release --quiet)` on every invocation. On
# a warm tree that is a ~28 ms no-op fingerprint check on this machine —
# 25-40% of a ~100 ms mp-tcp sample — so timing `bash run.sh` charged the
# mp-tcp arm for cargo work the pthreads arm (which times the binary
# directly) never paid: asymmetric arms. By the time this runs, the
# untimed byte-checked run (run_binary_into) has already built every
# binary, so the cargo line is pure overhead to strip. Mirrors the case
# study, which likewise times a cargo-free launch.
#
# HARD-FAIL on zero matches (TASK-0187 lineage): if the emitted run.sh
# stops carrying exactly one cargo-build line (emit shape changed), this
# must fail loud rather than silently time the un-stripped script.
strip_cargo_from_run_sh() {
    local dir="$1"
    local run="$dir/run.sh" timed="$dir/run-timed.sh"
    [ -f "$run" ] || { echo "perf-study: FAIL — $run missing (mp-tcp emit?)" >&2; exit 1; }
    local pat='(cd "$here" && cargo build --release --quiet)'
    local n; n=$(grep -cF "$pat" "$run" || true)  # || true: keep the n=0 diagnostic reachable under set -e (review fix)
    if [ "$n" -ne 1 ]; then
        echo "perf-study: FAIL — expected exactly 1 cargo-build line in $run," \
             "found $n (emit shape changed; update strip_cargo_from_run_sh's" \
             "pinned pattern, do NOT silently time the un-stripped script)." >&2
        exit 1
    fi
    grep -vF "$pat" "$run" > "$timed"
}

# run_cmd_for_backend DIR BACKEND  -> echoes the timed command (as a
# single bash -c string) so time_run can repeat it. INPUT/OUTPUT come
# from the caller's env exports. The mp-tcp arm times the CARGO-STRIPPED
# run-timed.sh (built by strip_cargo_from_run_sh after the correctness
# run), so no warm no-op `cargo build` enters any wall sample — the
# pthreads arm already times its binary directly, and this makes the two
# arms symmetric (TASK-0455.04 P1).
timed_cmd() {
    local dir="$1" backend="$2"
    case "$backend" in
        mp-tcp-*) echo "bash '$dir/run-timed.sh' \"\$NUC_INPUT_PATH\" \"\$NUC_OUTPUT_PATH\"" ;;
        *)        echo "'$dir/target/release/nuc-generated'" ;;
    esac
}

# =====================================================================
# MATMUL fixture (param N). Identical arithmetic to 07-matmul:
# C[i][j] = sum_k A[i][k]*B[k][j], wrapping_mul + wrapping_add.
# =====================================================================
gen_matmul_fixture() {
    local N="$1" dir="$2"
    mkdir -p "$dir/ref/src" "$dir/gen/src"
    cat > "$dir/prog.algo.nuc" <<NUC
const N : usize = $N;
data a : i32[N][N];
data b : i32[N][N];
data c : i32[N][N];
kernel madd   : (i32, i32, i32) -> i32 pure;
kernel load_a : ()         -> i32[N][N] effectful;
kernel load_b : ()         -> i32[N][N] effectful;
kernel save_c : (i32[N][N]) -> ()       effectful;
a <-- load_a();
b <-- load_b();
for i : 0 .. N {
for j : 0 .. N {
for k : 0 .. N {
    c[i][j] <-- madd(c[i][j], a[i][k], b[k][j]);
}}}
save_c(c);
NUC
    cat > "$dir/kernels.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const N: usize = $N; const ELEMS: usize = N*N; const BPW: usize = 4;
const MB: usize = ELEMS*BPW; const IB: usize = 2*MB;
pub fn madd(acc: i32, x: i32, y: i32) -> i32 { acc.wrapping_add(x.wrapping_mul(y)) }
fn read_in() -> Vec<u8> {
    let p = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".into());
    let b = fs::read(&p).unwrap_or_else(|e| panic!("load: {}: {}", p, e));
    assert!(b.len() >= IB, "load: {} has {} need {}", p, b.len(), IB); b
}
fn decode(b: &[u8], off: usize) -> Vec<i32> {
    let mut o = Vec::with_capacity(ELEMS);
    for k in 0..ELEMS { let f = off+k*BPW;
        o.push(i32::from_le_bytes([b[f],b[f+1],b[f+2],b[f+3]])); } o
}
pub fn load_a() -> Vec<i32> { decode(&read_in(), 0) }
pub fn load_b() -> Vec<i32> { decode(&read_in(), MB) }
pub fn save_c(c: Vec<i32>) {
    assert_eq!(c.len(), ELEMS);
    let p = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".into());
    let mut by = Vec::with_capacity(MB);
    for v in &c { by.extend_from_slice(&v.to_le_bytes()); }
    let mut f = fs::File::create(&p).unwrap_or_else(|e| panic!("save: {}: {}", p, e));
    f.write_all(&by).unwrap();
}
RS
    # Independent generator: bounded spatially-varying integer matrices.
    cat > "$dir/gen/Cargo.toml" <<TOML
[package]
name = "mm-gen"
version = "0.1.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "mm-gen"
path = "src/main.rs"
TOML
    cat > "$dir/gen/src/main.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const N: usize = $N; const ELEMS: usize = N*N;
fn av(i: usize, j: usize) -> i32 { (((i*7 + j*3) % 11) as i32) - 5 }
fn bv(i: usize, j: usize) -> i32 { (((i*5 + j*13) % 11) as i32) - 5 }
fn main() {
    let mut out: Option<String> = None;
    let a: Vec<String> = env::args().collect(); let mut k=1;
    while k<a.len() { if a[k]=="--out" { k+=1; out=Some(a[k].clone()); } k+=1; }
    let p = out.expect("--out");
    let mut by = Vec::with_capacity(2*ELEMS*4);
    for i in 0..N { for j in 0..N { by.extend_from_slice(&av(i,j).to_le_bytes()); } }
    for i in 0..N { for j in 0..N { by.extend_from_slice(&bv(i,j).to_le_bytes()); } }
    fs::File::create(&p).unwrap().write_all(&by).unwrap();
}
RS
    # Independent reference oracle: re-derived triple loop, wrapping ops.
    cat > "$dir/ref/Cargo.toml" <<TOML
[package]
name = "mm-ref"
version = "0.1.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "mm-ref"
path = "src/main.rs"
[profile.release]
panic = "abort"
TOML
    cat > "$dir/ref/src/main.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const N: usize = $N; const ELEMS: usize = N*N; const BPW: usize=4;
const MB: usize = ELEMS*BPW;
fn main() {
    let a: Vec<String> = env::args().collect();
    let (mut ip, mut op) = (None, None); let mut k=1;
    while k<a.len() { match a[k].as_str() {
        "--in" => { k+=1; ip=Some(a[k].clone()); }
        "--out"=> { k+=1; op=Some(a[k].clone()); } _=>{} } k+=1; }
    let (ip, op) = (ip.expect("--in"), op.expect("--out"));
    let by = fs::read(&ip).unwrap();
    let dec = |off: usize| { let mut o=vec![0i32;ELEMS];
        for k in 0..ELEMS { let f=off+k*BPW;
            o[k]=i32::from_le_bytes([by[f],by[f+1],by[f+2],by[f+3]]); } o };
    let (am, bm) = (dec(0), dec(MB));
    let mut c = vec![0i32; ELEMS];
    for i in 0..N { for j in 0..N {
        let mut acc = 0i32;
        for k in 0..N { acc = acc.wrapping_add(am[i*N+k].wrapping_mul(bm[k*N+j])); }
        c[i*N+j] = acc; } }
    let mut o = Vec::with_capacity(MB);
    for v in &c { o.extend_from_slice(&v.to_le_bytes()); }
    fs::File::create(&op).unwrap().write_all(&o).unwrap();
}
RS
    # Generate input + reference once per size.
    cargo run --release --quiet --manifest-path "$dir/gen/Cargo.toml" -- --out "$dir/input.bin"
    cargo run --release --quiet --manifest-path "$dir/ref/Cargo.toml" -- --in "$dir/input.bin" --out "$dir/reference.bin"
}

# Schedule variant: matmul outer-i partition across W workers.
gen_matmul_sched() {
    local N="$1" W="$2" dir="$3"
    local wl=""
    for i in $(seq 0 $((W-1))); do wl="$wl, w$i"; done
    wl="${wl#, }"
    local pl="$wl"
    cat > "$dir/dist-$W.sched.nuc" <<NUC
schedule for "prog.algo.nuc" {
    workers = { host, $wl };
    place load_a on host;
    place load_b on host;
    place save_c on host;
    place madd   on { $pl };
    loop i : partition=workers;
    transfer a : sync;
    transfer b : sync;
    transfer c : sync;
}
NUC
}

# =====================================================================
# STENCIL fixture (param H, W). Identical arithmetic to docs/case-study:
# 3x3 box blur, wrapping sum of nine taps, truncating /9.
# =====================================================================
gen_stencil_fixture() {
    local H="$1" W="$2" dir="$3"
    mkdir -p "$dir/ref/src" "$dir/gen/src"
    cat > "$dir/prog.algo.nuc" <<NUC
const H : usize = $H;
const W : usize = $W;
data img_in  : i32[H][W];
data img_out : i32[H][W];
kernel blur3 : (i32, i32, i32, i32, i32, i32, i32, i32, i32) -> i32 pure;
kernel load_image : ()          -> i32[H][W] effectful;
kernel save_image : (i32[H][W]) -> ()         effectful;
img_in <-- load_image();
for y : 1 .. H-1 {
for x : 1 .. W-1 {
    img_out[y][x] <-- blur3(
        img_in[y-1][x-1], img_in[y-1][x], img_in[y-1][x+1],
        img_in[y  ][x-1], img_in[y  ][x], img_in[y  ][x+1],
        img_in[y+1][x-1], img_in[y+1][x], img_in[y+1][x+1]
    );
}}
save_image(img_out);
NUC
    cat > "$dir/kernels.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const H: usize = $H; const W: usize = $W; const N: usize = H*W;
pub fn blur3(p0:i32,p1:i32,p2:i32,p3:i32,p4:i32,p5:i32,p6:i32,p7:i32,p8:i32)->i32 {
    let s = p0.wrapping_add(p1).wrapping_add(p2).wrapping_add(p3).wrapping_add(p4)
        .wrapping_add(p5).wrapping_add(p6).wrapping_add(p7).wrapping_add(p8);
    s / 9
}
pub fn load_image() -> Vec<i32> {
    let p = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".into());
    let b = fs::read(&p).unwrap_or_else(|e| panic!("load: {}: {}", p, e));
    assert!(b.len() >= N*4, "load: {} has {} need {}", p, b.len(), N*4);
    let mut o = Vec::with_capacity(N);
    for i in 0..N { let f=i*4; o.push(i32::from_le_bytes([b[f],b[f+1],b[f+2],b[f+3]])); } o
}
pub fn save_image(img: Vec<i32>) {
    assert_eq!(img.len(), N);
    let p = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".into());
    let mut by = Vec::with_capacity(N*4);
    for v in &img { by.extend_from_slice(&v.to_le_bytes()); }
    fs::File::create(&p).unwrap().write_all(&by).unwrap();
}
RS
    cat > "$dir/gen/Cargo.toml" <<TOML
[package]
name = "st-gen"
version = "0.1.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "st-gen"
path = "src/main.rs"
TOML
    cat > "$dir/gen/src/main.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const H: usize = $H; const W: usize = $W; const N: usize = H*W;
fn pixel(y: usize, x: usize) -> i32 {
    let ramp = y.wrapping_mul(7).wrapping_add(x.wrapping_mul(13));
    let plaid = (y/32).wrapping_mul(x/32).wrapping_mul(37);
    let checker = if ((y/16)+(x/16))%2==0 {4096} else {0};
    (ramp.wrapping_add(plaid).wrapping_add(checker) & 0xFFFF) as i32
}
fn main() {
    let mut out: Option<String> = None;
    let a: Vec<String> = env::args().collect(); let mut k=1;
    while k<a.len() { if a[k]=="--out" { k+=1; out=Some(a[k].clone()); } k+=1; }
    let p = out.expect("--out");
    let mut by = Vec::with_capacity(N*4);
    for y in 0..H { for x in 0..W { by.extend_from_slice(&pixel(y,x).to_le_bytes()); } }
    fs::File::create(&p).unwrap().write_all(&by).unwrap();
}
RS
    cat > "$dir/ref/Cargo.toml" <<TOML
[package]
name = "st-ref"
version = "0.1.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "st-ref"
path = "src/main.rs"
[profile.release]
panic = "abort"
TOML
    cat > "$dir/ref/src/main.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const H: usize = $H; const W: usize = $W; const N: usize = H*W;
fn idx(y: usize, x: usize) -> usize { y*W + x }
fn main() {
    let a: Vec<String> = env::args().collect();
    let (mut ip, mut op) = (None, None); let mut k=1;
    while k<a.len() { match a[k].as_str() {
        "--in"=>{k+=1; ip=Some(a[k].clone());}
        "--out"=>{k+=1; op=Some(a[k].clone());} _=>{} } k+=1; }
    let (ip, op) = (ip.expect("--in"), op.expect("--out"));
    let by = fs::read(&ip).unwrap();
    let mut im = vec![0i32; N];
    for k in 0..N { let f=k*4; im[k]=i32::from_le_bytes([by[f],by[f+1],by[f+2],by[f+3]]); }
    let mut out = vec![0i32; N];
    for y in 1..H-1 { for x in 1..W-1 {
        let taps = [ im[idx(y-1,x-1)], im[idx(y-1,x)], im[idx(y-1,x+1)],
                     im[idx(y,x-1)], im[idx(y,x)], im[idx(y,x+1)],
                     im[idx(y+1,x-1)], im[idx(y+1,x)], im[idx(y+1,x+1)] ];
        let s = taps.iter().fold(0i32, |a,&t| a.wrapping_add(t));
        out[idx(y,x)] = s / 9;
    } }
    let mut o = Vec::with_capacity(N*4);
    for v in &out { o.extend_from_slice(&v.to_le_bytes()); }
    fs::File::create(&op).unwrap().write_all(&o).unwrap();
}
RS
    cargo run --release --quiet --manifest-path "$dir/gen/Cargo.toml" -- --out "$dir/input.bin"
    cargo run --release --quiet --manifest-path "$dir/ref/Cargo.toml" -- --in "$dir/input.bin" --out "$dir/reference.bin"
}

# Schedule variant: stencil row-band partition across W workers, async.
gen_stencil_sched() {
    local W="$1" dir="$2"
    local wl=""
    for i in $(seq 0 $((W-1))); do wl="$wl, w$i"; done
    wl="${wl#, }"
    local pl="$wl"
    cat > "$dir/dist-$W.sched.nuc" <<NUC
schedule for "prog.algo.nuc" {
    workers = { host, $wl };
    place load_image on host;
    place save_image on host;
    place blur3      on { $pl };
    loop y : partition=rows;
    transfer img_in  : async, buffer=2, notify=event;
    transfer img_out : sync;
}
NUC
}

# =====================================================================
# REDUCTION fixture (param N total length, FIXED 4 workers). Identical
# arithmetic to 03-reduction: two-phase wrapping sum. NUM_WORKERS is
# baked into the array shape AND the phase-2 tree, so only N scales.
# =====================================================================
gen_reduction_fixture() {
    local N="$1" dir="$2"
    local PS=$((N / 4))   # PARTITION_SIZE; N must be divisible by 4
    mkdir -p "$dir/ref/src" "$dir/gen/src"
    cat > "$dir/prog.algo.nuc" <<NUC
const N             : usize = $N;
const NUM_WORKERS   : usize = 4;
const PARTITION_SIZE: usize = N / NUM_WORKERS;
data a        : i32[NUM_WORKERS][PARTITION_SIZE];
data partials : i32[NUM_WORKERS];
data half1    : i32;
data half2    : i32;
data result   : i32;
kernel load_input  : ()    -> i32[NUM_WORKERS][PARTITION_SIZE] effectful;
kernel save_output : (i32) -> ()                               effectful;
kernel accumulate  : (i32, i32) -> i32 pure;
kernel combine     : (i32, i32) -> i32 pure;
a <-- load_input();
for w : 0 .. NUM_WORKERS {
    for i : 0 .. PARTITION_SIZE {
        partials[w] <-- accumulate(partials[w], a[w][i]);
    }
}
half1  <-- combine(partials[0], partials[1]);
half2  <-- combine(partials[2], partials[3]);
result <-- combine(half1, half2);
save_output(result);
NUC
    cat > "$dir/kernels.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const N: usize = $N;
pub fn accumulate(acc: i32, x: i32) -> i32 { acc.wrapping_add(x) }
pub fn combine(a: i32, b: i32) -> i32 { a.wrapping_add(b) }
pub fn load_input() -> Vec<i32> {
    let p = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".into());
    let b = fs::read(&p).unwrap_or_else(|e| panic!("load: {}: {}", p, e));
    assert!(b.len() >= N*4, "load: {} has {} need {}", p, b.len(), N*4);
    let mut o = Vec::with_capacity(N);
    for i in 0..N { let f=i*4; o.push(i32::from_le_bytes([b[f],b[f+1],b[f+2],b[f+3]])); } o
}
pub fn save_output(s: i32) {
    let p = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".into());
    fs::File::create(&p).unwrap().write_all(&s.to_le_bytes()).unwrap();
}
RS
    cat > "$dir/gen/Cargo.toml" <<TOML
[package]
name = "rd-gen"
version = "0.1.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "rd-gen"
path = "src/main.rs"
TOML
    cat > "$dir/gen/src/main.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const N: usize = $N;
fn val(i: usize) -> i32 { (((i*2654435761usize) >> 13) & 0xFF) as i32 - 128 }
fn main() {
    let mut out: Option<String> = None;
    let a: Vec<String> = env::args().collect(); let mut k=1;
    while k<a.len() { if a[k]=="--out" { k+=1; out=Some(a[k].clone()); } k+=1; }
    let p = out.expect("--out");
    let mut by = Vec::with_capacity(N*4);
    for i in 0..N { by.extend_from_slice(&val(i).to_le_bytes()); }
    fs::File::create(&p).unwrap().write_all(&by).unwrap();
}
RS
    cat > "$dir/ref/Cargo.toml" <<TOML
[package]
name = "rd-ref"
version = "0.1.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "rd-ref"
path = "src/main.rs"
[profile.release]
panic = "abort"
TOML
    # Re-derived: a single flat left-to-right wrapping fold. The two-phase
    # tree is associative+commutative under wrapping_add (two's-complement
    # integer add is associative), so the flat fold equals the tree fold
    # bit-for-bit — a deliberately DIFFERENT structure from the kernel's
    # two-phase shape, which is the independence the differential needs.
    cat > "$dir/ref/src/main.rs" <<RS
use std::env; use std::fs; use std::io::Write;
const N: usize = $N;
fn main() {
    let a: Vec<String> = env::args().collect();
    let (mut ip, mut op) = (None, None); let mut k=1;
    while k<a.len() { match a[k].as_str() {
        "--in"=>{k+=1; ip=Some(a[k].clone());}
        "--out"=>{k+=1; op=Some(a[k].clone());} _=>{} } k+=1; }
    let (ip, op) = (ip.expect("--in"), op.expect("--out"));
    let by = fs::read(&ip).unwrap();
    let ps = N/4;
    // Mirror the two-phase decomposition's ORDER exactly: per-partition
    // fold then tree combine, so the wrapping-add associativity argument
    // is exercised the same way the kernel exercises it.
    let mut partials = [0i32; 4];
    for w in 0..4 { let mut acc=0i32;
        for i in 0..ps { let f=(w*ps+i)*4;
            acc = acc.wrapping_add(i32::from_le_bytes([by[f],by[f+1],by[f+2],by[f+3]])); }
        partials[w]=acc; }
    let h1 = partials[0].wrapping_add(partials[1]);
    let h2 = partials[2].wrapping_add(partials[3]);
    let result = h1.wrapping_add(h2);
    fs::File::create(&op).unwrap().write_all(&result.to_le_bytes()).unwrap();
}
RS
    cargo run --release --quiet --manifest-path "$dir/gen/Cargo.toml" -- --out "$dir/input.bin"
    cargo run --release --quiet --manifest-path "$dir/ref/Cargo.toml" -- --in "$dir/input.bin" --out "$dir/reference.bin"
    gen_reduction_sched "$dir"
}

# Schedule for reduction — FIXED 4 workers (the algorithm's phase-2 tree
# is hardwired to 4 partials, so the worker count cannot vary without
# editing the algorithm). Mirrors the committed 03-reduction/distributed.
gen_reduction_sched() {
    local dir="$1"
    cat > "$dir/dist-4.sched.nuc" <<NUC
schedule for "prog.algo.nuc" {
    workers = { host, w0, w1, w2, w3 };
    place load_input  on host;
    place save_output on host;
    place accumulate on { w0, w1, w2, w3 };
    place combine on host;
    loop w : partition=workers;
    transfer a        : sync;
    transfer partials : sync;
}
NUC
}

# =====================================================================
# CELL RUNNERS — build, byte-check, time, record.
# =====================================================================

# run_naive_cell example size dir backend
run_naive_cell() {
    local ex="$1" size="$2" dir="$3" be="$4"
    local out="$dir/naive-$be"
    # Always (re)generate the single-worker naive schedule, then build.
    gen_naive_sched_then_build "$ex" "$dir" "$be" "$out"
    local res="$dir/naive-$be.out"
    local bx; bx=$(run_for_correctness "$res" "$dir/input.bin" "$out" "$be" "$dir/reference.bin")
    local t
    if [ "$bx" = "RUNFAIL" ]; then
        t="RUNFAIL RUNFAIL RUNFAIL"
    else
        # Naive cells are pthreads-async in this study (single binary, timed
        # directly), but if a future naive mp-tcp cell is added the same
        # cargo-strip applies — keep the arms symmetric (TASK-0455.04 P1).
        case "$be" in mp-tcp-*) strip_cargo_from_run_sh "$out" ;; esac
        export NUC_INPUT_PATH="$dir/input.bin" NUC_OUTPUT_PATH="$res"
        t=$(time_run "$REPS" bash -c "$(timed_cmd "$out" "$be")")
        unset NUC_INPUT_PATH NUC_OUTPUT_PATH
    fi
    record_row "$ex" "$size" "$be(naive)" 1 $t "$bx"
}

# run_for_correctness RES INPUT OUT BACKEND REFERENCE -> echoes PASS|FAIL|RUNFAIL
# Retries the run on a transient failure (empty/missing output), then
# compares bytes. A non-empty diverging output is FAIL (hard); a
# persistently empty/failed run is RUNFAIL (transient robustness).
run_for_correctness() {
    local res="$1" input="$2" out="$3" be="$4" reference="$5"
    local i
    for i in 1 2 3; do
        rm -f "$res"
        run_binary_into "$res" "$input" "$out" "$be"
        if [ -s "$res" ]; then
            if cmp -s "$res" "$reference"; then echo PASS; else echo FAIL; fi
            return 0
        fi
    done
    echo RUNFAIL
}

# gen_naive_sched_then_build writes a single-worker naive schedule for
# the example then builds it. Called when the generated dir has no
# committed naive.sched.nuc.
gen_naive_sched_then_build() {
    local ex="$1" dir="$2" be="$3" out="$4"
    case "$ex" in
        matmul) cat > "$dir/naive.sched.nuc" <<NUC
schedule for "prog.algo.nuc" {
    workers = { host };
    place load_a on host; place load_b on host; place save_c on host; place madd on host;
}
NUC
            ;;
        stencil) cat > "$dir/naive.sched.nuc" <<NUC
schedule for "prog.algo.nuc" {
    workers = { host };
    place load_image on host; place save_image on host; place blur3 on host;
}
NUC
            ;;
        reduction) cat > "$dir/naive.sched.nuc" <<NUC
schedule for "prog.algo.nuc" {
    workers = { host };
    place load_input on host; place save_output on host;
    place accumulate on host; place combine on host;
}
NUC
            ;;
    esac
    build_project "$dir/prog.algo.nuc" "$dir/naive.sched.nuc" "$dir/kernels.rs" "$be" "$out"
}

# emitted_so_buf RUN_SH  -> the integer the backend exported as
# `export NUC_SO_BUF=<N>` in the emitted run.sh, i.e. the largest single
# per-channel socket payload the cell will `setsockopt(SO_SNDBUF)` to.
# Reading the EMITTED line makes the backend's own sizing the single
# source of truth for the wmem-cap predicate, rather than re-deriving the
# band arithmetic by hand here (which would drift from the codegen). The
# previous hand-derived estimate matched exactly on the swept grid, but a
# hand copy is a second source of truth that can silently rot — TASK-0187
# lineage. Hard-fails if the line is absent (emit shape changed).
emitted_so_buf() {
    local run_sh="$1"
    local v; v=$(grep -oP '(?<=^export NUC_SO_BUF=)[0-9]+' "$run_sh" | head -1 || true)  # || true: keep the missing-line diagnostic reachable under set -e (review fix)
    if [ -z "$v" ]; then
        echo "perf-study: FAIL — no 'export NUC_SO_BUF=<N>' line in $run_sh;" \
             "the mp-tcp emit shape changed — update emitted_so_buf." >&2
        exit 1
    fi
    echo "$v"
}

# run_dist_cell example size dir backend workers mode
run_dist_cell() {
    local ex="$1" size="$2" dir="$3" be="$4" W="$5" mode="$6"
    local sched="$dir/dist-$W.sched.nuc"
    local out="$dir/dist-$be-$W"
    # Emit (codegen only) first — cheap and flat in N. For mp-tcp-* this
    # produces the run.sh whose `export NUC_SO_BUF=` line is the backend's
    # authoritative socket-buffer request; we read it and CAPSKIP the cell
    # (skipping the cargo build + run) when it exceeds this sandbox's
    # un-raisable net.core.wmem_max. Above the cap the host would panic on
    # setsockopt and the cell cannot RUN here (the documented wmem wall —
    # see docs/perf-study.md); recording CAPSKIP is deterministic, not a
    # flake. pthreads-* has no socket buffer and never CAPSKIPs.
    nucleus_emit "$dir/prog.algo.nuc" "$sched" "$dir/kernels.rs" "$be" "$out"
    case "$be" in
        mp-tcp-*)
            local cap need
            cap="$(cat /proc/sys/net/core/wmem_max 2>/dev/null || echo 4194304)"
            need="$(emitted_so_buf "$out/run.sh")"
            if [ "$need" -gt "$cap" ]; then
                record_row "$ex" "$size" "$be" "$W" CAPSKIP CAPSKIP CAPSKIP CAPSKIP
                echo "    (wmem wall: emitted NUC_SO_BUF=${need}B > cap ${cap}B" \
                     "— net.core.wmem_max un-raisable here; cell cannot run)" >&2
                return
            fi
            ;;
    esac
    cargo_build_emitted "$out" "$be"
    local res="$dir/dist-$be-$W.out"
    local bx; bx=$(run_for_correctness "$res" "$dir/input.bin" "$out" "$be" "$dir/reference.bin")
    local t
    if [ "$bx" = "RUNFAIL" ]; then
        t="RUNFAIL RUNFAIL RUNFAIL"
    else
        # Correctness run (above) built every binary; for mp-tcp-* derive a
        # cargo-free run-timed.sh so no warm no-op `cargo build` enters the
        # wall samples (TASK-0455.04 P1).
        case "$be" in mp-tcp-*) strip_cargo_from_run_sh "$out" ;; esac
        export NUC_INPUT_PATH="$dir/input.bin" NUC_OUTPUT_PATH="$res"
        t=$(time_run "$REPS" bash -c "$(timed_cmd "$out" "$be")")
        unset NUC_INPUT_PATH NUC_OUTPUT_PATH
    fi
    record_row "$ex" "$size" "$be" "$W" $t "$bx"
}

# =====================================================================
# WIRE MEASUREMENT — measured bytes on a message-passing backend.
#
# Method: strace the cell's worker processes for sendto/sendmsg and sum
# the byte returns. The /proc/<pid>/io wchar route was tried first and
# FALSIFIED (Rust TcpStream sends go through sendto(2), whose bytes the
# kernel does not add to wchar — a transfer that moved KB showed ~20B of
# wchar). strace -e trace=sendto,sendmsg captures the exact per-call byte
# count on the syscall return value. Run UNDER strace is a SEPARATE run
# from timing (ptrace inflates wall, not byte counts), so the measured
# wall numbers in the results table are never taken from a straced run.
# See scripts/perf-study-wire.py for the parse + caveats.
# =====================================================================
# measure_wire is a HARD step: a green study must not be able to ship an
# empty WIRE table. Every failure mode below (missing cell, missing
# strace, diverged output, zero data bytes) is a fail-loud `exit 1`, not a
# silent `return` (TASK-0187 lineage — see scripts/perf-study-wire.py).
measure_wire() {
    local ex="$1" size="$2" dir="$3" be="$4" W="$5"
    local out="$dir/dist-$be-$W"
    # The cell was already built in the sweep; ensure it exists.
    [ -d "$out" ] || { echo "perf-study: FAIL — wire $ex/$be missing built cell $out" >&2; exit 1; }
    # strace must be present: an absent tracer would otherwise let the run
    # "succeed" with an empty log and report a vacuous 0% — a loud failure,
    # never a warning (TASK-0455.04 P2: strace-absent becomes a hard fail).
    command -v strace >/dev/null 2>&1 || {
        echo "perf-study: FAIL — wire $ex/$be needs strace and it is not on PATH;" \
             "a green PASS must not carry an empty WIRE table." >&2; exit 1; }
    local res="$dir/wire-$be-$W.out"
    local log="$dir/wire-$be-$W.strace"
    # Run once under strace, capturing socket sends. We use the emitted
    # run.sh (with its no-op cargo build) here, NOT run-timed.sh: this is a
    # separate, deliberately UNtimed run (strace inflates wall by 2-10x),
    # so the cargo no-op is harmless and keeping run.sh means the wire path
    # does not depend on strip_cargo_from_run_sh having been called first.
    ( cd "$out" && NUC_INPUT_PATH="$dir/input.bin" NUC_OUTPUT_PATH="$res" \
        strace -f -e trace=sendto,sendmsg -e signal=none -qq -o "$log" \
        bash run.sh "$dir/input.bin" "$res" >/dev/null 2>&1 ) || {
        echo "perf-study: FAIL — wire $ex/$be strace run failed. If this is the" \
             "4 MiB net.core.wmem_max socket cap, the runner should have CAPSKIP'd" \
             "the cell upstream (this size is meant to be cap-safe)." >&2; exit 1; }
    cmp -s "$res" "$dir/reference.bin" || {
        echo "perf-study: FAIL — wire $ex/$be diverged from reference (byte mismatch)." >&2; exit 1; }
    # Parse measured DATA bytes (control frames reported separately).
    # perf-study-wire.py itself hard-fails on zero data sends; we ALSO
    # guard data_bytes>0 here so a future refactor of the parser cannot
    # reintroduce a silent empty-table degradation.
    local parsed; parsed=$(python3 "$ROOT/scripts/perf-study-wire.py" --log "$log" --histogram) || {
        echo "perf-study: FAIL — wire $ex/$be parse failed (see above)." >&2; exit 1; }
    local data_bytes; data_bytes=$(echo "$parsed" | cut -f1)
    local ctrl_bytes; ctrl_bytes=$(echo "$parsed" | cut -f2)
    if ! [ "${data_bytes:-0}" -gt 0 ] 2>/dev/null; then
        echo "perf-study: FAIL — wire $ex/$be measured ${data_bytes:-?} data bytes" \
             "(<=0); a cell that ran byte-exact MUST have moved array data." >&2; exit 1; fi
    echo "    wire-raw: $ex/$size/$be w=$W  measured_data=${data_bytes}B  control=${ctrl_bytes}B" >&2
    # Compare measured narrowed DATA against the static whole-array baseline.
    python3 "$ROOT/scripts/perf-study-wirebase.py" \
        --src-dir "$out" --example "$ex" --size "$size" \
        --measured "$data_bytes" --workers "$W" --backend "$be" > "$WIRE.tmp" || {
        echo "perf-study: FAIL — wire $ex/$be baseline comparison failed." >&2; exit 1; }
    tail -1 "$WIRE.tmp" | column -t -s $'\t' | sed 's/^/    /'
    cat "$WIRE.tmp" >> "$WIRE"; rm -f "$WIRE.tmp"
}
