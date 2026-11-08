{
  description = "Fan, RGB and LCD control for Lian Li hardware";

  inputs = {
    self.submodules = true;
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      # github: flake refs come without submodules, so fetch the same upstream commit again with them.
      # Forks with their own commits build with git+https://github.com/<fork>/lian-li-linux?submodules=1.
      hasSubmodules =
        builtins.pathExists "${self}/vendor/tinyuz/compress/tuz_enc.h"
        && builtins.pathExists "${self}/vendor/HDiffPatch/libHDiffPatch/HDiff/private_diff/libdivsufsort/divsufsort.cpp";
      src =
        if hasSubmodules then
          self
        else if self ? rev then
          builtins.fetchGit {
            url = "https://github.com/sgtaziz/lian-li-linux";
            inherit (self) rev;
            submodules = true;
            allRefs = true;
          }
        else
          throw "lian-li-linux needs its git submodules. Run `git submodule update --init`, or use git+https://github.com/sgtaziz/lian-li-linux?submodules=1.";
    in
    {
      packages = forAllSystems (pkgs: {
        lian-li-linux = pkgs.callPackage ./packaging/nix/package.nix { inherit src; };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.lian-li-linux;
      });

      overlays.default = final: _: {
        lian-li-linux = final.callPackage ./packaging/nix/package.nix { inherit src; };
      };

      nixosModules.default = {
        imports = [ ./packaging/nix/module.nix ];
        nixpkgs.overlays = [ self.overlays.default ];
      };

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
