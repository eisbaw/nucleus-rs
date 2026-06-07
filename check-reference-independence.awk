# check-reference-independence.awk — enforce docs/reference-impl-policy.md §2.
#
# The differential argument's credibility hinges on the reference oracle
# being CODE-INDEPENDENT of the compiler: if a reference shared the
# compiler middle-end (or any backend crate, or a Nuc-generated file), a
# common-mode bug could corrupt the generated code AND the reference the
# same way, and the byte-identity differential could not see it (PRD
# §10.1 "all backends wrong the same way"; thesis ch10
# sec:disc-shortcomings "Agreement could be common-mode").
#
# §2 makes code-independence a HARD RULE; before this fence it was
# enforced only by a reviewer checklist (policy §6/§7: "Not a CI-enforced
# check"). This scanner makes it mechanical. It reads one or more
# reference Cargo.toml files on argv and exits 1 (printing FAIL lines) on
# any violation, 0 if all are clean.
#
# Load-bearing observation: the Nucleus crates (nucleus-compiler,
# backend-common, every backend) are UNPUBLISHED workspace members. The
# ways a reference could reach one are a `path =` / `git =` dependency, a
# `workspace =` parent-link, OR a `[patch]` / `[replace]` source override
# that redirects an innocuous crates.io name to a local Nucleus path (Cargo
# honours `[patch]` even in a standalone non-workspace crate) — none of
# which a legitimate crates.io reference dependency (e.g. byteorder) ever
# needs. So forbidding path/git in any dependency OR patch/replace section,
# plus a `workspace =` link, plus the Nucleus crate names by spelling,
# structurally enforces §2 with zero false positives on a clean tree.
#
# ALLOWED and NOT flagged: an empty `[workspace]` TABLE header (the
# isolation mechanism that makes each reference its own workspace root,
# the OPPOSITE of a parent link); a `[[bin]] path = "src/main.rs"` target
# path (not a dependency); plain crates.io deps (`byteorder = "1"`).

function strip(s) { gsub(/^[ \t]+|[ \t]+$/, "", s); return s }

function is_dep_section(s) {
    return (s ~ /(^|\.)dependencies$/) ||
           (s ~ /(^|\.)dev-dependencies$/) ||
           (s ~ /(^|\.)build-dependencies$/) ||
           (s ~ /(^|\.)(dev-|build-)?dependencies\./)
}

# [patch.<src>] / [patch.<src>.<crate>] / [replace] redirect a dependency
# to another source. A `[patch.crates-io] foo = { path = "...nucleus..." }`
# pulls a Nucleus crate's CODE in while only a crates.io name is declared,
# so these sections are path/git-bearing for §2 purposes too.
function is_patch_section(s) {
    return (s ~ /(^|\.)patch(\.|$)/) || (s ~ /(^|\.)replace(\.|$)/)
}

# Any section whose entries can name or path-redirect a dependency.
function is_scanned_section(s) {
    return is_dep_section(s) || is_patch_section(s)
}

# A dependency crate name that belongs to the Nucleus workspace.
function is_nucleus_crate(n,  nn) {
    nn = n; gsub(/["']/, "", nn); nn = strip(nn)
    if (nn == "") return 0
    return (nn == "nucleus" || nn == "nucleus-compiler" ||
            nn == "backend-common" || nn == "test-common" ||
            nn ~ /^nuc-/ || nn ~ /^mp-tcp/ || nn ~ /^mp-uds/ ||
            nn ~ /^pthreads/ || nn ~ /^openmp/ || nn ~ /^mpi-/ ||
            nn ~ /^embedded-/)
}

BEGIN { bad = 0 }
FNR == 1 { section = "" }      # reset section state per file

/^[ \t]*#/ { next }            # whole-line comment
/^[ \t]*$/ { next }            # blank

# Section header: [name], [[name]], [dependencies.foo], etc.
/^[ \t]*\[/ {
    hdr = $0; sub(/#.*/, "", hdr); hdr = strip(hdr)
    gsub(/^\[+|\]+$/, "", hdr)
    section = hdr
    # A dependency/patch/replace SUBTABLE whose trailing crate name is a
    # Nucleus crate: [dependencies.nucleus-compiler],
    # [build-dependencies.mp-tcp-event], [patch.crates-io.backend-common].
    if (is_scanned_section(section) && section ~ /\./) {
        name = section; sub(/.*\./, "", name)   # last dotted component
        if (is_nucleus_crate(name)) {
            printf("FAIL: %s: forbidden Nucleus-crate in section [%s] (policy §2)\n", FILENAME, section)
            bad = 1
        }
    }
    next
}

# A `workspace =` KEY anywhere is a parent-workspace link (forbidden).
# The empty `[workspace]` table header has no `=`, so it is not matched.
/^[ \t]*workspace[ \t]*=/ {
    printf("FAIL: %s: forbidden 'workspace =' parent-workspace link: %s (policy §2)\n", FILENAME, strip($0))
    bad = 1; next
}

# Entries inside any dependency, patch, or replace section.
{
    if (is_scanned_section(section)) {
        e = $0; sub(/#.*/, "", e); e = strip(e)
        if (e == "") next
        if (e ~ /(^|[,{[ \t])path[ \t]*=/) {
            printf("FAIL: %s [%s]: path-dependency forbidden — references are standalone crates.io-only (policy §2): %s\n", FILENAME, section, e)
            bad = 1
        }
        if (e ~ /(^|[,{[ \t])git[ \t]*=/) {
            printf("FAIL: %s [%s]: git-dependency forbidden (policy §2): %s\n", FILENAME, section, e)
            bad = 1
        }
        key = e; sub(/[ \t]*=.*/, "", key); key = strip(key)
        if (is_nucleus_crate(key)) {
            printf("FAIL: %s [%s]: forbidden Nucleus-crate dependency '%s' (policy §2)\n", FILENAME, section, key)
            bad = 1
        }
    }
}

END { exit (bad ? 1 : 0) }
