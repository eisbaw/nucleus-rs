{
  # Nucleus v2 dev shell. See nuc-nucleus/PRD.md §12.1.
  #
  # Provides a pinned Rust toolchain (rustc + cargo + clippy + rustfmt),
  # just, and rust-analyzer. No verbose shellHook by design.
  #
  # MSRV is pinned here (rustChannel below). When a Cargo.toml lands it
  # MUST NOT re-declare rust-version; the flake is the single source of truth.

  description = "Nucleus v2 — reproducible dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    # Fenix gives precise Rust channel pinning with official binaries.
    # Chosen over oxalica/rust-overlay because fromToolchainFile / combine
    # semantics are cleaner and the nix-community provenance is acceptable.
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # MSRV pin. Bump in lockstep with the policy in PRD §13:
        # "stable, ~6 months before the current milestone".
        # When bumping, update both the channel string and the sha256.
        # To get a new sha256, set it to lib.fakeHash, run `nix develop`,
        # and copy the correct hash from the error message.
        rustChannel = "1.83.0";

        rustToolchain = (fenix.packages.${system}.toolchainOf {
          channel = rustChannel;
          sha256 = "sha256-s1RPtyvDGJaX/BisLT+ifVfuhDT1nZkZ1NcK8sbwELM=";
        }).withComponents [
          "rustc"
          "cargo"
          "clippy"
          "rustfmt"
          "rust-src"
        ];
        # Single source of truth for the tier-1 dev toolchain. The renode
        # shell inherits this list verbatim and adds heavier tier-3 tools
        # on top, so the MSRV pin and tooling stay aligned across tiers.
        basePackages = [
          rustToolchain
          fenix.packages.${system}.rust-analyzer
          pkgs.just
          pkgs.ripgrep
        ];

        # Tier-3 (M9+) cross-compile toolchain. Adds the
        # `thumbv7em-none-eabihf` rust-std (Cortex-M7 / STM32H7, the M9
        # reference target per PRD §7.3) on top of the same MSRV-pinned
        # host toolchain. fenix's `combine` is the documented way to
        # union a host toolchain with per-target rust-std components.
        # See PRD §7.3 and TASK-0062.
        embeddedToolchain = fenix.packages.${system}.combine [
          rustToolchain
          (fenix.packages.${system}.targets.thumbv7em-none-eabihf.toolchainOf {
            channel = rustChannel;
            sha256 = "sha256-s1RPtyvDGJaX/BisLT+ifVfuhDT1nZkZ1NcK8sbwELM=";
          }).rust-std
        ];
      in
      {
        # Tier-1 dev shell (PRD §12.1). DELIBERATELY MINIMAL — only the
        # toolchain + just + git. Tier-specific heavy closures live in
        # opt-in sibling shells: `.#renode` (tier-3 runtime, M10),
        # `.#embedded` (tier-3 cross-compile, M9), and `.#mpi` (tier-2,
        # M7, TASK-0063 — landed when M7 started). Do NOT pile MPI /
        # Renode / embedded toolchains into this shell — every
        # contributor would then download hundreds of MB they don't
        # need (TASK-0068).
        devShells.default = pkgs.mkShell {
          packages = basePackages;

          # Silent on purpose. See PRD §12.1 and ~/.claude/CLAUDE.md.
          shellHook = "";
        };

        # Tier-3 (M10+) runtime validation shell. Opt-in via
        # `nix develop .#renode`. Renode is Mono-based and pulls hundreds
        # of MB of closure, so it is deliberately kept out of the default
        # shell that CI and day-to-day tier-1 dev use. See PRD §10.3, §12.1
        # and TASK-0064.
        devShells.renode = pkgs.mkShell {
          packages = basePackages ++ [ pkgs.renode ];
          # Silent on purpose. See PRD §12.1 and ~/.claude/CLAUDE.md.
          shellHook = "";
        };

        # Tier-3 (M9+) embedded cross-compile shell. Opt-in via
        # `nix develop .#embedded`. Adds `thumbv7em-none-eabihf` rust-std
        # to the host toolchain (Cortex-M7 / STM32H7 reference target per
        # PRD §7.3) plus probe-rs for on-chip flashing/debug. Kept out of
        # the default shell because per-target rust-std + probe-rs pull a
        # non-trivial closure that tier-1 dev does not need. The packages
        # list is spelled out (rather than `basePackages ++ ...`) because
        # `embeddedToolchain` replaces — not augments — the plain
        # `rustToolchain` from `basePackages`. See PRD §7.3 and TASK-0062.
        devShells.embedded = pkgs.mkShell {
          packages = [
            embeddedToolchain
            fenix.packages.${system}.rust-analyzer
            pkgs.just
            pkgs.probe-rs-tools
          ];
          # Silent on purpose. See PRD §12.1 and ~/.claude/CLAUDE.md.
          shellHook = "";
        };

        # Tier-2 (M7+) HPC-cluster shell. Opt-in via `nix develop .#mpi`.
        # Adds an MPI implementation + the rsmpi (`mpi` crate) build
        # dependencies on top of the same MSRV-pinned host toolchain.
        # Kept out of the default shell because OpenMPI pulls a non-
        # trivial closure (ucx / libfabric / pmix / hwloc) that tier-1
        # dev does not need (TASK-0068 tiered-shell rule; same precedent
        # as `.#renode` / `.#embedded`). Closure size is reproducible via
        # `nix path-info -Sh nixpkgs#openmpi` rather than pinned here.
        #
        # MPI impl decision (TASK-0063 AC#4): OpenMPI, not MPICH.
        # Rationale — (1) nixpkgs `openmpi` is the better-maintained,
        # cache-populated pick (the whole closure is in the binary cache,
        # zero source builds); (2) it ships a working `mpicc` wrapper +
        # `ompi-c.pc` pkg-config file, which is exactly what rsmpi's
        # `mpi-sys` build probe (`build-probe-mpi`) consumes to discover
        # compile/link flags; (3) `mpirun` localhost launcher works out
        # of the box for the PRD §10.2 localhost-MPI CI bar. MPICH would
        # work too but offers no advantage here and is less exercised in
        # nixpkgs CI.
        #
        # rsmpi build deps: `mpi-sys` runs `bindgen` over `mpi.h`, which
        # needs libclang at build time. `LIBCLANG_PATH` points bindgen at
        # the nix libclang; `BINDGEN_EXTRA_CLANG_ARGS` adds the clang
        # resource-dir + libc headers so bindgen can resolve the system
        # includes `mpi.h` pulls in (the recurring NixOS bindgen gotcha:
        # without these, `stddef.h` / `stdint.h` are not found).
        devShells.mpi = pkgs.mkShell {
          packages = basePackages ++ [
            pkgs.openmpi
            pkgs.llvmPackages.libclang
            pkgs.clang
          ];

          # bindgen (mpi-sys build.rs) needs libclang on a stable path.
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          # Give bindgen the clang builtin headers + glibc dev headers so
          # `#include <stddef.h>` etc. inside mpi.h resolve under Nix.
          BINDGEN_EXTRA_CLANG_ARGS =
            "-isystem ${pkgs.llvmPackages.libclang.lib}/lib/clang/${pkgs.lib.getVersion pkgs.llvmPackages.libclang}/include "
            + "-isystem ${pkgs.glibc.dev}/include";

          # Silent on purpose. See PRD §12.1 and ~/.claude/CLAUDE.md.
          shellHook = "";
        };

        # Docs / presentation shell. Opt-in via `nix develop .#docs`.
        # Adds the Marp CLI (`marp`) for rendering the Marpit decks under
        # `docs/presentation/` to self-contained HTML (`just slides` /
        # `marp deck.md -o deck.html`). Kept out of the default shell
        # because it pulls a Node-based closure tier-1 compiler dev does
        # not need (same tiered-shell rule as `.#renode` / `.#embedded` /
        # `.#mpi`, TASK-0068).
        devShells.docs = pkgs.mkShell {
          # marp-cli renders the deck; nodejs runs `bundle.mjs`, which
          # inlines the SVG assets as base64 data: URIs so `index.html`
          # is a single self-contained file (node is already in the
          # marp-cli closure, so this adds ~nothing).
          packages = basePackages ++ [ pkgs.marp-cli pkgs.nodejs ];
          # Silent on purpose. See PRD §12.1 and ~/.claude/CLAUDE.md.
          shellHook = "";
        };

        # Nothing buildable from the flake yet; placeholder formatter so
        # `nix flake check` has something neutral to chew on.
        formatter = pkgs.nixpkgs-fmt;
      });
}
