---
id: TASK-0415
title: >-
  check-include-str-coverage uses bashism (done < <(...)) despite POSIX /bin/sh
  portability claim (TASK-0395 cycle architect P3-1)
status: Done
assignee:
  - '@me'
created_date: '2026-06-01 21:07'
updated_date: '2026-06-02 22:05'
labels:
  - tooling
  - ci
  - doc-lie
  - posix
  - cycle-0395-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P3-1 from the TASK-0395 review (commit 89025a4). The doc-fence recipe headers (justfile ~504/676/992) advertise POSIX-shell portability (just runs /bin/sh -cu; no bash arrays / process substitution). But check-include-str-coverage (justfile ~326) uses `done < <(...)` process substitution, a BASHISM that dash/posh would reject. This is a doc-vs-code inconsistency in the comment-doc-lie class (CLAUDE.md recurring defect #1): either the recipe is not actually POSIX (and the portability comment overclaims) or just is running these under bash, not /bin/sh, on this system.

NOT introduced by TASK-0395 (pre-existing) and that recipe uses explicit nucleus/.../src roots so it has NO untracked-scratch footgun — purely a portability-claim consistency nit. Fix options: (1) rewrite the `< <(...)` as a POSIX `while read` over a mktemp temp file (like the doc-citation fences do), or (2) if just genuinely runs bash here, correct the recipe-header comments that claim /bin/sh POSIX-cleanliness. Verify which by checking just's shell setting (settings / shell:) and testing under dash. LOW / OPTIONAL: zero functional effect today (the CI host has a bash-compatible sh).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-244 orchestrator in-thread fix. Chose option (1) of the task (make the recipe ACTUALLY POSIX) over option (2) (weaken the portability claim) — root fix, and it makes check-include-str-coverage consistent with every sibling fence (check-doc-test-name-staleness:805, check-doc-cell-path-staleness:904, check-mega-files:1004, the doc-citation fences:530/689) which ALL already use the mktemp temp-file + while-read + trap-EXIT POSIX idiom. Rewrote the bashism done < <(rg ...) to: rg ... > $inc_f (mktemp) with trap "rm -f $inc_f" EXIT, then while read ... done < $inc_f; added empty-line guard. Added a POSIX-portability comment header matching siblings.

GROUND TRUTH: /bin/sh on this NixOS host -> bash-interactive 5.3, and justfile has NO set shell (just defaults to sh -cu) — so the bashism worked HERE but would break on dash/ash/busybox-sh, exactly as the ~504/676/992 portability-claim headers warn. PROVEN: (a) new recipe body runs clean under nix dash (real POSIX sh) parsing all include_str! sites; (b) old done < <(...) under dash = Syntax error: redirection unexpected. So this was a real gap, not cosmetic. Recipe still passes on current tree (OK: every include_str! has compile coverage). No Rust touched -> build/clippy/test/test-release/e2e unaffected.

REVIEW GATE (cycle 244, orchestrator-independent): qa-test-runner GO + mped-architect GO.

qa (re-run): check-include-str-coverage prints OK; new body parses+runs under nixpkgs dash; old done < <(...) form = dash Syntax error: redirection unexpected; recipe still wired into just ci (line ~189); tree clean except tracker md. No Rust delta so heavy arms logically unaffected (not re-measured).

architect: GO. Correctness PASS (temp-file redirect keeps the while-loop in the current shell so fail=1 persists — a pipe would have lost it; || true guards set -e on rg no-match; fence still bites). Honesty PASS (new header accurate; cross-checked sibling headers 513/686 are true).

SILENT-SIBLING SWEEP (architect P2/P3) — FOLDED IN-THREAD (commit affc935), NOT deferred:
- P2: check-mega-files set -o pipefail (justfile:1013) removed. *** ORCHESTRATOR CORRECTED THE ARCHITECT MECHANISM ***: architect claimed dash REJECTS pipefail; EMPIRICALLY FALSE — pipefail entered POSIX in Issue 8 (2024) and BOTH nixpkgs dash AND this bash-compat busybox sh ACCEPT it (verified set -o pipefail -> exit 0; this busybox even accepts <()). Removal is still correct but for the HONEST reason: pipefail exit status is unused here (results read via comm from temp file) + unsupported on pre-2024/non-bash-compat shells the header names. Header rewritten with the accurate mechanism (avoided introducing a fresh comment-doc-lie).
- P3: check-mega-files comment printf-fed bash array (justfile:998) is a doc-lie — it is POSIX printf positional args, not a bash array. Corrected to printf-fed positional list.

LESSON (CLAUDE.md recurring #5 cheap-empirical-verification + feedback-implementer-disclosure-mechanism-wrong generalized to reviewer-subagent): a reviewer-subagent mechanism claim must be empirically checked before fold-back. The pipefail-is-POSIX-2024 fact would have shipped as a NEW doc-lie had I transcribed the architect mechanism verbatim. Caught by running the actual shells.

Verification: check-include-str-coverage OK; check-mega-files OK (no non-allow-listed >1000 LoC, no stale entry); both bodies run clean under nixpkgs dash. justfile-only, no Rust touched.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made check-include-str-coverage genuinely POSIX (option 1 — root fix, not claim-weakening): replaced the done < <(rg ...) process-substitution bashism with the mktemp temp-file + while-read + trap-EXIT idiom every sibling fence uses; added a POSIX-portability header. Review gate GO/GO. Folded the architect silent-sibling sweep in-thread (commit affc935): removed an unused set -o pipefail from check-mega-files and fixed a printf-fed bash array doc-lie there, after empirically CORRECTING the architect mechanism (pipefail is POSIX-2024, accepted by modern dash/busybox — not dash-rejected). Proven: new bodies run clean under nixpkgs dash; old < <(...) is a dash syntax error. Commits 04f8c7c + affc935; justfile-only, no Rust delta (build/clippy/test/test-release/e2e unaffected).
<!-- SECTION:FINAL_SUMMARY:END -->
