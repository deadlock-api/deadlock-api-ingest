{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage rec {
  pname = "deadlock-api-ingest";
  version = "0.1.102-fc2d778";

  src = pkgs.fetchFromGitHub {
    owner = "deadlock-api";
    repo = "deadlock-api-ingest";
    rev = "v${version}";
    hash = "sha256-IXUUoWrXz/HZVkLZJ5vyMro9tboFW83q7zjUy4PLLrU=";
  };

  cargoHash = "sha256-H61PDhu+zy641aKnxYT2cn9Cu2RHuWJQHaUz28nSAFk=";

  doCheck = false; # compiles twice for a `cargo check`

  nativeBuildInputs = with pkgs; [
    pkg-config
  ];
  buildInputs = with pkgs; [
    libpcap
    openssl
  ];

  meta = {
    description = "A network packet capture tool that monitors HTTP traffic for Deadlock game replay files and ingests metadata to the Deadlock API.";
    homepage = "https://github.com/deadlock-api/deadlock-api-ingest";
    license = pkgs.lib.licenses.mit;
    # maintainers = with pkgs.lib.maintainers; [ ];
    mainProgram = "deadlock-api-ingest";
  };
}

