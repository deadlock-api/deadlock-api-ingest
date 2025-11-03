{
  description = "Monitors your Steam HTTP cache for Deadlock game replay files and automatically submits match metadata to the Deadlock API";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05"; # Adjust if needed
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

        apps.default = {
          type = "app";
          program = "${package}/bin/deadlock-api-ingest";
        };

        packages.default = package;

      });
}
