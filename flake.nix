{
  description = "Vitrine: a GTK4 image viewer and wallpaper picker";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    gnim.url = "github:aylur/gnim";
    gnim.flake = false;
  };

  outputs = {
    self,
    nixpkgs,
    gnim,
  }: let
    systems = ["x86_64-linux" "aarch64-linux"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
  in {
    packages = forAllSystems (pkgs: {
      default = pkgs.callPackage ./package.nix {inherit gnim;};
    });

    homeManagerModules.default = import ./hm-module.nix self;

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.default];
        packages = [pkgs.alejandra];
      };
    });

    formatter = forAllSystems (pkgs: pkgs.alejandra);
  };
}
