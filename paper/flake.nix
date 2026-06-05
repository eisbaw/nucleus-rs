{
  description = "Nucleus — PhD dissertation (LaTeX/LuaTeX)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      # scheme-full is the comprehensive, single-attribute TeX Live set:
      # it includes lualatex, latexmk, biber, biblatex, memoir, pgf/tikz,
      # pgfplots, fontspec, microtype, cleveref, listings, etc. We favour
      # it over a hand-curated combine so the scaffold cannot fail on a
      # missing package; a later cycle may slim it to texlive.combine.
      tex = pkgs.texlive.combined.scheme-full;
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        # No shellHook: keep the dev shell silent (project convention).
        packages = [ tex pkgs.just ];
      };

      # `nix build` produces the dissertation PDF reproducibly.
      packages.${system}.default = pkgs.stdenvNoCC.mkDerivation {
        name = "nucleus-dissertation";
        src = ./.;
        nativeBuildInputs = [ tex ];
        # latexmk drives the multi-pass lualatex + biber build.
        buildPhase = ''
          export HOME=$TMPDIR
          latexmk -lualatex -interaction=nonstopmode -halt-on-error main.tex
        '';
        installPhase = ''
          mkdir -p $out
          cp main.pdf $out/nucleus-dissertation.pdf
        '';
      };
    };
}
