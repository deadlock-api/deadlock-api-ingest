{
  lib,
  naersk-lib,
  pkg-config,
  protobuf,
  openssl,
  stdenv,
  darwin,
  makeWrapper,
  makeDesktopItem,
  libxkbcommon,
  wayland,
  
  libX11,
  libXcursor,
  libXi,
  libXrandr,

  src,
}:

let
  cargoToml = builtins.fromTOML (builtins.readFile (src + /Cargo.toml));

  # The status window's entry, as install-linux.sh writes it: Wayland desktops find a
  # window's icon only through the desktop file named like its app_id.
  statusDesktopItem = makeDesktopItem {
    name = "deadlock-api-ingest-status";
    desktopName = "Deadlock API Ingest";
    comment = "See the Deadlock matches you have sent to trackers";
    exec = "deadlock-api-ingest --gui";
    icon = "deadlock-api-ingest";
    categories = [ "Utility" ];
    keywords = [ "deadlock" "api" "ingest" "steam" "matches" "replay" "tracker" ];
    startupWMClass = "deadlock-api-ingest-status";
  };
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
  ] ++ lib.optionals stdenv.hostPlatform.isDarwin [
    darwin.apple_sdk.frameworks.Security
    darwin.apple_sdk.frameworks.SystemConfiguration
  ];

  # `--gui` (winit) dlopens the windowing libraries at runtime.
  postInstall = lib.optionalString stdenv.hostPlatform.isLinux ''
    wrapProgram $out/bin/deadlock-api-ingest \
      --prefix LD_LIBRARY_PATH : ${
        lib.makeLibraryPath [
          libxkbcommon
          wayland
          libX11
          libXcursor
          libXi
          libXrandr
        ]
      }
    install -Dm644 ${src}/assets/icon.png $out/share/icons/hicolor/256x256/apps/deadlock-api-ingest.png
    install -Dm644 -t $out/share/applications ${statusDesktopItem}/share/applications/*.desktop
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
