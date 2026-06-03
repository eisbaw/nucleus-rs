// THROWAWAY feasibility spike for TASK-0341.02.01.01 (grammar epic S0).
// NOT production code. Reproduces the ONE question that can invalidate
// the data-dependent-loop-termination epic:
//
//   On a convergence loop `for t : 0 .. N until COND { ... }` that
//   EARLY-EXITS at generation k < N, does the 16-jacobi final gather
//   read generation-k (correct) or generation-N (wrong)?
//
// It replicates the EXACT index arithmetic of the shipped 16-jacobi
// single-worker codegen (verified against the generated src/main.rs):
//   field : flat vec![0; (ITERS+1)*H*W]; generation t at slice stride
//   H*W; prev-gen index (t+ITERS)%(ITERS+1); the fixed-iteration final
//   extraction reads the HARD-CODED slice `field[ITERS]`.
//
// Run: rustc -O jacobi_earlyexit_spike.rs -o /tmp/jac && /tmp/jac

const H: usize = 8;
const W: usize = 8;
const ITERS: usize = 4;
const SLICE: usize = H * W; // 64

// jacobi5_or_seed: t==0 -> seed_yx ; t>=1 -> (n+s+e+w)/4 (integer trunc).
fn jacobi5_or_seed(n: i32, s: i32, e: i32, w: i32, seed_yx: i32, t: i32) -> i32 {
    if t == 0 {
        seed_yx
    } else {
        (n + s + e + w) / 4
    }
}

// Run the generation nest, but BREAK after computing generation
// `break_gen` (simulating `until COND` firing at t == break_gen).
// Returns the full field buffer (all ITERS+1 slices).
fn run_jacobi(seed: &[i32], break_gen: usize) -> Vec<i32> {
    let mut field = vec![0i32; (ITERS + 1) * SLICE];
    for t in 0..(ITERS + 1) {
        for y in 1..(H - 1) {
            for x in 1..(W - 1) {
                let pg = (t + ITERS) % (ITERS + 1); // prev-gen slice
                let n = field[pg * SLICE + (y - 1) * W + x];
                let s = field[pg * SLICE + (y + 1) * W + x];
                let e = field[pg * SLICE + y * W + (x - 1)];
                let w = field[pg * SLICE + y * W + (x + 1)];
                field[t * SLICE + y * W + x] =
                    jacobi5_or_seed(n, s, e, w, seed[y * W + x], t as i32);
            }
        }
        // `until COND` early-exit: stop after the converged generation.
        if t == break_gen {
            break;
        }
    }
    field
}

fn extract(field: &[i32], slice: usize) -> Vec<i32> {
    let mut r = vec![0i32; SLICE];
    for ry in 0..H {
        for rx in 0..W {
            r[ry * W + rx] = field[slice * SLICE + ry * W + rx];
        }
    }
    r
}

fn main() {
    // Non-trivial seed: interior ramp so generations are distinct & nonzero.
    let mut seed = vec![0i32; SLICE];
    for y in 1..(H - 1) {
        for x in 1..(W - 1) {
            seed[y * W + x] = (y * W + x) as i32 * 7 % 251 + 1;
        }
    }

    let break_gen = 2usize; // converged early, at generation 2 (< ITERS=4)
    let field = run_jacobi(&seed, break_gen);

    // (A) What the CURRENT fixed-iteration codegen does: read field[ITERS].
    let codegen_read = extract(&field, ITERS);
    // (B) The CORRECT answer for an early-exit at k: read field[break_gen].
    let correct_read = extract(&field, break_gen);

    let codegen_all_zero = codegen_read.iter().all(|&v| v == 0);
    let correct_nonzero = correct_read.iter().any(|&v| v != 0);
    let differ = codegen_read != correct_read;

    println!("break_gen (k)            = {}", break_gen);
    println!("ITERS (cap N)            = {}", ITERS);
    println!("codegen read slice       = field[{}] (hard-coded)", ITERS);
    println!("codegen_read all-zero    = {}", codegen_all_zero);
    println!("correct_read (gen k) nz  = {}", correct_nonzero);
    println!("codegen_read != correct  = {}", differ);
    println!("correct_read[1*W+1..1*W+5] = {:?}", &correct_read[W + 1..W + 5]);
    println!("codegen_read[1*W+1..1*W+5] = {:?}", &codegen_read[W + 1..W + 5]);

    assert!(
        codegen_all_zero,
        "FINDING REFUTED: slice ITERS was written even on early-exit"
    );
    assert!(correct_nonzero, "sanity: generation-k must be nonzero");
    assert!(
        differ,
        "FINDING REFUTED: codegen read == correct read (no bug)"
    );
    println!(
        "\nFINDING CONFIRMED: early-exit at k={} leaves field[ITERS={}] \
         UNWRITTEN (all-zero); the fixed-iteration final-read field[ITERS] \
         is WRONG under early-exit. The convergence variant MUST read the \
         runtime-current generation slice field[k], not the static \
         field[ITERS].",
        break_gen, ITERS
    );
}
