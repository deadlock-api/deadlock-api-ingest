{
  lib,
  rustPlatform,
  pkg-config,
  openssl,
  stdenv,
  darwin,
  src,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "deadlock-api-ingest";
  version = "0.1.257-56ddcf0";

  inherit src;

  cargoHash = "sha256-mc6pW0AK6t4R9yXlGXLQ5uMdjfP4TGsvEyRDD6wQe3c=";

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    openssl
  ] ++ lib.optionals stdenv.isDarwin [
    darwin.apple_sdk.frameworks.Security
    darwin.apple_sdk.frameworks.SystemConfiguration
  ];

  # Run tests during build
  doCheck = true;

  # Additional cargo flags if needed
  # cargoTestFlags = [ "--all-features" ];

  meta = {
    description = "Monitors your Steam HTTP cache for Deadlock game replay files and automatically submits match metadata to the Deadlock API";
    homepage = "https://github.com/deadlock-api/deadlock-api-ingest";
    license = lib.licenses.mit; # or licenses.asl20, etc.
    mainProgram = "deadlock-api-ingest";
  };
})