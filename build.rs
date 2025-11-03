// Build script to embed Windows resources and metadata
// This helps reduce antivirus false positives by making the executable look more legitimate

fn main() {
    // Only compile resources on Windows
    #[cfg(target_os = "windows")]
    {
        // Embed Windows resource file with version info and metadata
        let mut res = winres::WindowsResource::new();

        res.set_icon("icon.ico")
            .set("ProductName", "Deadlock API Ingest")
            .set(
                "FileDescription",
                "Monitors your Steam HTTP cache for Deadlock game replay files and automatically submits match metadata to the Deadlock API",
            )
            .set("CompanyName", "Deadlock API")
            .set(
                "LegalCopyright",
                "Copyright (c) 2025 Deadlock API Contributors",
            )
            .set("OriginalFilename", "deadlock-api-ingest.exe")
            .set("InternalName", "deadlock-api-ingest")
            .set("ProductVersion", env!("CARGO_PKG_VERSION"))
            .set("FileVersion", env!("CARGO_PKG_VERSION"));

        // Compile the resource file
        if let Err(e) = res.compile() {
            // Don't fail the build if icon is missing, just warn
            eprintln!("Warning: Failed to compile Windows resources: {}", e);
            eprintln!("This is not critical, but the executable will lack version metadata.");
        }
    }

    // Rerun if Cargo.toml changes (for version updates)
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=icon.ico");
}
