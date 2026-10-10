// Embeds the Windows icon and version metadata (which also reduces antivirus false
// positives) and prepares the GUI's window icon.

use std::path::Path;

/// Edge length of the window icon taken from `icon.ico`; `src/gui` reads it at this size.
const WINDOW_ICON_SIZE: u32 = 64;

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=icon.ico");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let icon = Path::new(&manifest_dir).join("icon.ico");

    write_window_icon(&icon, &Path::new(&out_dir).join("window-icon.rgba"));

    // The target, not the host: cross builds from Linux need the resources too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let rc = Path::new(&out_dir).join("app.rc");
        std::fs::write(&rc, resource_script(&icon)).expect("write app.rc");
        // Fail the build instead of shipping an exe without icon and version info, which
        // is what winres did silently on the MinGW toolchain.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
            // embed-resource picks its compiler by the *host* toolchain, so on a Windows
            // host with MSVC it would produce an rc.exe library MinGW's linker can't read.
            embed_with_windres(&rc, &Path::new(&out_dir).join("app-res.o"));
        } else if let Err(e) =
            embed_resource::compile(&rc, embed_resource::NONE).manifest_required()
        {
            panic!("Failed to embed Windows resources: {e}");
        }
    }
}

/// Compiles `rc` with MinGW's windres into a COFF object and links it into the binaries.
fn embed_with_windres(rc: &Path, object: &Path) {
    let target = std::env::var("TARGET").expect("set by cargo");
    // Overridable like embed-resource; otherwise the cross-prefixed name, then plain windres
    // (what MinGW installs on Windows).
    let candidates: Vec<String> = std::env::var(format!("RC_{target}"))
        .or_else(|_| std::env::var("RC"))
        .map(|rc| vec![rc])
        .unwrap_or_else(|_| {
            vec![
                "x86_64-w64-mingw32-windres".to_owned(),
                "windres".to_owned(),
            ]
        });
    let mut errors = Vec::new();
    for windres in &candidates {
        let status = std::process::Command::new(windres)
            .arg("--input")
            .arg(rc)
            .args(["--input-format=rc", "--output-format=coff", "--output"])
            .arg(object)
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("cargo:rustc-link-arg-bins={}", object.display());
                return;
            }
            Ok(s) => errors.push(format!("{windres}: {s}")),
            Err(e) => errors.push(format!("{windres}: {e}")),
        }
    }
    panic!(
        "Failed to embed Windows resources, no working windres ({}). Install MinGW's binutils or set RC_{target}.",
        errors.join("; ")
    );
}

/// Raw RGBA pixels of the `WINDOW_ICON_SIZE` entry of `icon.ico`.
fn write_window_icon(icon: &Path, out: &Path) {
    let file = std::fs::File::open(icon).expect("open icon.ico");
    let dir = ico::IconDir::read(file).expect("parse icon.ico");
    let entry = dir
        .entries()
        .iter()
        .find(|e| e.width() == WINDOW_ICON_SIZE && e.height() == WINDOW_ICON_SIZE)
        .expect("icon.ico has a 64x64 image");
    let image = entry.decode().expect("decode icon.ico");
    std::fs::write(out, image.rgba_data()).expect("write window icon");
}

fn resource_script(icon: &Path) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let numeric: Vec<&str> = version
        .split(['-', '+'])
        .next()
        .unwrap_or(version)
        .split('.')
        .collect();
    let file_version = format!(
        "{},{},{},0",
        numeric.first().unwrap_or(&"0"),
        numeric.get(1).unwrap_or(&"0"),
        numeric.get(2).unwrap_or(&"0")
    );
    let icon = icon.display().to_string().replace('\\', "\\\\");
    format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {file_version}
PRODUCTVERSION {file_version}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Deadlock API"
      VALUE "FileDescription", "Deadlock API Ingest"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "deadlock-api-ingest"
      VALUE "LegalCopyright", "Copyright (c) 2025 Deadlock API Contributors"
      VALUE "OriginalFilename", "deadlock-api-ingest.exe"
      VALUE "ProductName", "Deadlock API Ingest"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    )
}
