{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage rec {
  pname = "deadlock-api-ingest";
  version = "0.1.116-8f2cd8f";

  src = pkgs.fetchFromGitHub {
    owner = "deadlock-api";
    repo = "deadlock-api-ingest";
    rev = "v${version}";
    hash = "sha256-9h/+CsummSGA8GLfMJng/qRseZuQfrHzPCYNqRDt7C0=";
  };

  cargoHash = "sha256-iHhPe1rdk/nq0wFnKiG41QzMTOoeq6v583TyzXwHO0Q=";

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
