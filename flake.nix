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
    in
    {
      packages = forAllSystems (pkgs: {
        lian-li-linux = pkgs.callPackage ./packaging/nix/package.nix { src = self; };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.lian-li-linux;
      });

      overlays.default = final: _: {
        lian-li-linux = final.callPackage ./packaging/nix/package.nix { src = self; };
      };

      nixosModules.default = {
        imports = [ ./packaging/nix/module.nix ];
        nixpkgs.overlays = [ self.overlays.default ];
      };

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
