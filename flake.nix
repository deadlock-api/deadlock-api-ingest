{
  description = "A network packet capture tool that monitors HTTP traffic for Deadlock game replay files and ingests metadata to the Deadlock API.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };
        package = import ./default.nix { inherit pkgs; };
      in {
        packages.default = package;
      });
}

