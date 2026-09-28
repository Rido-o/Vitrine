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
    inherit (nixpkgs) lib;
    systems = ["x86_64-linux" "aarch64-linux"];
    forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

    # typescript and the @girs type packages from package-lock.json (types only)
    nodeModules = pkgs:
      pkgs.importNpmLock.buildNodeModules {
        npmRoot = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [./package.json ./package-lock.json];
        };
        inherit (pkgs) nodejs;
      };

    # node_modules in the current directory: the npm packages plus gnim
    # (whose package.json exports ./dist, which its build copies from ./src).
    # gnim ships .ts source, which tsc would type-check against our @girs
    # versions; mark it @ts-nocheck (skipLibCheck only covers .d.ts).
    linkNodeModules = pkgs: ''
      rm -rf node_modules
      mkdir node_modules
      ln -s ${nodeModules pkgs}/node_modules/* ${nodeModules pkgs}/node_modules/.bin node_modules/
      cp -r ${gnim} node_modules/gnim
      chmod -R u+w node_modules/gnim
      ln -s src node_modules/gnim/dist
      find node_modules/gnim/src -name '*.ts' -exec sed -i '1i // @ts-nocheck' {} +
    '';
  in {
    packages = forAllSystems (pkgs: {
      default = pkgs.callPackage ./package.nix {inherit gnim;};
      # The gtk4-rs spike (rust/) and the TypeScript app's benchmark build.
      spike = pkgs.callPackage ./rust/package.nix {};
      bench-ts = pkgs.callPackage ./package.nix {
        inherit gnim;
        probe = true;
      };
    });

    homeManagerModules.default = import ./hm-module.nix self;

    checks = forAllSystems (pkgs: {
      typecheck = pkgs.stdenvNoCC.mkDerivation {
        name = "vitrine-typecheck";
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [./src ./tsconfig.json];
        };
        nativeBuildInputs = [pkgs.nodejs];
        buildPhase = ''
          ${linkNodeModules pkgs}
          node node_modules/typescript/bin/tsc --pretty -p .
        '';
        installPhase = "touch $out";
      };
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.default];
        packages = [pkgs.alejandra pkgs.nodejs];
        shellHook = ''
          ${linkNodeModules pkgs}
          export PATH="$PWD/node_modules/.bin:$PATH"
        '';
      };
      spike = pkgs.mkShell {
        inputsFrom = [self.packages.${pkgs.stdenv.hostPlatform.system}.spike];
        packages = [pkgs.cargo pkgs.clippy pkgs.rustfmt pkgs.rust-analyzer];
      };
    });

    formatter = forAllSystems (pkgs: pkgs.alejandra);
  };
}
