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
        ];
      in
      {
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

        # Nothing buildable from the flake yet; placeholder formatter so
        # `nix flake check` has something neutral to chew on.
        formatter = pkgs.nixpkgs-fmt;
      });
}
