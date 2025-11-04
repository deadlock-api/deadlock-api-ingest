{
  description = "Monitors your Steam HTTP cache for Deadlock game replay files and automatically submits match metadata to the Deadlock API";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        package = pkgs.callPackage ./default.nix { src = self; };
      in
      {

        apps.default = {
          type = "app";
          program = "${package}/bin/deadlock-api-ingest";
        };

        packages.default = package;

      });
}
