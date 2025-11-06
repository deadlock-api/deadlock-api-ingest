{
  description = "Monitors your Steam HTTP cache for Deadlock game replay files and automatically submits match metadata to the Deadlock API";

  # Automatically use the binary cache
  nixConfig = {
    extra-substituters = [
      "https://deadlock-api-ingest.cachix.org"
    ];
    extra-trusted-public-keys = [
      "deadlock-api-ingest.cachix.org-1:UvvF0vXYqgpZVJaCiVPi90GKTGTXxs4znl6FsJzH+uU="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      naersk,
    }:
    {
      # Export the NixOS module
      nixosModules.default = import ./module.nix;
      nixosModules.deadlock-api-ingest = import ./module.nix;
    }
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        naersk-lib = pkgs.callPackage naersk {};

        package = pkgs.callPackage ./default.nix { 
          src = self;
          naersk-lib = naersk-lib;
        };
      in
      {

        apps.default = {
          type = "app";
          program = "${package}/bin/deadlock-api-ingest";
        };

        packages.default = package;

      }
    );
}