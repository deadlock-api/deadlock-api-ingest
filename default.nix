{ pkgs ? import <nixpkgs> {} }:

pkgs.stdenv.mkDerivation rec {
  pname = "deadlock-api-ingest";
  version = "0.1.140-8659e00";

  src = pkgs.fetchurl {
    url = "https://github.com/deadlock-api/deadlock-api-ingest/releases/download/v${version}/deadlock-api-ingest-ubuntu-latest";
    sha256 = "sha256-bZIHTdhfX1UgH30i0+Sn2mAw7fNpg6OYBEr4oX+9P/8=="; 
  };

  nativeBuildInputs = with pkgs; [
    autoPatchelfHook
  ];

  buildInputs = with pkgs; [
    libpcap
    libgcc
  ];

  unpackPhase = "true";

  installPhase = ''
    mkdir -p $out/bin
    install -m755 $src $out/bin/deadlock-api-ingest
    patchelf --replace-needed libpcap.so.0.8 libpcap.so.1 $out/bin/deadlock-api-ingest
  '';

  meta = {
    description = "A network packet capture tool that monitors HTTP traffic for Deadlock game replay files and ingests metadata to the Deadlock API.";
    homepage = "https://github.com/deadlock-api/deadlock-api-ingest";
    license = pkgs.lib.licenses.mit;
    # maintainers = with pkgs.lib.maintainers; [ ];
    mainProgram = "deadlock-api-ingest";
  };
}
