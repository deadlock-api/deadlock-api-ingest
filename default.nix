{
  lib,
  naersk-lib,
  pkg-config,
  protobuf,
  openssl,
  stdenv,
  darwin,
  makeWrapper,
  libxkbcommon,
  wayland,
  xorg,
  src,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile (src + /Cargo.toml));
in
naersk-lib.buildPackage {
  pname = "deadlock-api-ingest";
  version = cargoToml.package.version;

  inherit src;

  nativeBuildInputs = [
    pkg-config
    protobuf
    makeWrapper
  ];

  buildInputs = [
    openssl
  ] ++ lib.optionals stdenv.isDarwin [
    darwin.apple_sdk.frameworks.Security
    darwin.apple_sdk.frameworks.SystemConfiguration
  ];

  # `--gui` (winit) dlopens the windowing libraries at runtime.
  postInstall = lib.optionalString stdenv.isLinux ''
    wrapProgram $out/bin/deadlock-api-ingest \
      --prefix LD_LIBRARY_PATH : ${
        lib.makeLibraryPath [
          libxkbcommon
          wayland
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
        ]
      }
  '';

  # naersk runs tests by default
  # doCheck = true;

  meta = {
    description = "Monitors your Steam HTTP cache for Deadlock game replay files and automatically submits match metadata to the Deadlock API";
    homepage = "https://github.com/deadlock-api/deadlock-api-ingest";
    license = lib.licenses.mit; # or licenses.asl20, etc.
    mainProgram = "deadlock-api-ingest";
  };
}