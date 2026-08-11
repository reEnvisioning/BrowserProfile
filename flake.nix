{
  description = "BrowserProfile (bp), safe Firefox-family profile management";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let systems = [ "x86_64-linux" "aarch64-linux" ];
    in {
      packages = builtins.listToAttrs (map (system: {
        name = system;
        value.default = nixpkgs.legacyPackages.${system}.callPackage ./package.nix { };
      }) systems);
      checks = builtins.listToAttrs (map (system: {
        name = system;
        value.build = self.packages.${system}.default;
      }) systems);
    };
}
