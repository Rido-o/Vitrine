{
  description = "Vitrine: a GTK4 image viewer and wallpaper picker";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    inherit (nixpkgs) lib;
    systems = ["x86_64-linux" "aarch64-linux"];
    forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
  in {
    packages = forAllSystems (pkgs: {
      default = pkgs.callPackage ./package.nix {};
    });

    homeManagerModules.default = import ./hm-module.nix self;

    checks = forAllSystems (pkgs: let
      vitrine = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
    in {
      # The package's build with clippy instead, warnings as errors.
      clippy = vitrine.overrideAttrs (old: {
        pname = "vitrine-clippy";
        nativeBuildInputs = old.nativeBuildInputs ++ [pkgs.clippy];
        buildPhase = ''
          runHook preBuild
          cargo clippy --release --offline --all-targets -- -D warnings
          runHook postBuild
        '';
        doCheck = false;
        installPhase = "touch $out";
        postInstall = "";
        postFixup = "";
      });
      # rustfmt itself: cargo fmt would resolve the dependencies first.
      rustfmt = pkgs.runCommand "vitrine-rustfmt" {nativeBuildInputs = [pkgs.rustfmt];} ''
        rustfmt --check --edition 2024 ${./build.rs} ${./src}/*.rs
        touch $out
      '';
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.default];
        packages = [pkgs.alejandra pkgs.cargo pkgs.clippy pkgs.rustfmt pkgs.rust-analyzer];
      };
    });

    formatter = forAllSystems (pkgs: pkgs.alejandra);
  };
}
