# Deadlock API Ingest

A network packet capture tool that monitors HTTP traffic for Deadlock game replay files and ingests metadata to the Deadlock API.

## Quick Installation

### Windows (PowerShell)

**No administrator privileges required!** Run this command in PowerShell:

```powershell
irm https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1 | iex
```

Or download and run manually:

```powershell
Invoke-WebRequest -Uri "https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1" -OutFile "install-windows.ps1"
.\install-windows.ps1
```

### Linux (Bash)

**No sudo/root privileges required!** Run this command:

```bash
curl -fsSL https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-linux.sh | bash
```

Or download and run manually:

```bash
wget https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-linux.sh
chmod +x install-linux.sh
./install-linux.sh
```

> **Note**: The installation scripts automatically download the latest release binaries from GitHub and set up the application to run on user login. The application installs to your user directory and does not require elevated privileges.

### NixOS

You can add it as a flake and add systemd service files so it will run on boot, or simply run the binary whenever you need it

```
nix run github:deadlock-api/deadlock-api-ingest
```

Example systemd user service file (`~/.config/systemd/user/deadlock-api-ingest.service`):

```toml
[Unit]
Description=Deadlock API Ingest Service
Documentation=https://github.com/deadlock-api/deadlock-api-ingest
After=network.target
Wants=network.target

[Service]
Type=simple
ExecStart=/nix/store/...-deadlock-api-ingest/bin/deadlock-api-ingest
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=deadlock-api-ingest

[Install]
WantedBy=default.target
```

Then enable and start with: `systemctl --user enable --now deadlock-api-ingest`

## Uninstallation

The installer automatically creates an uninstall script next to the binary.

### Windows
Simply run the uninstall script:
```powershell
& "$env:LOCALAPPDATA\deadlock-api-ingest\uninstall.ps1"
```

Or navigate to `%LOCALAPPDATA%\deadlock-api-ingest\` and double-click `uninstall.ps1`.

### Linux
Simply run the uninstall script:
```bash
~/.local/share/deadlock-api-ingest/uninstall.sh
```

## Automated Releases

This project uses automated releases that are created on every push to the master branch. The GitHub Actions workflow:

1. **Builds cross-platform binaries** for Windows and Linux
2. **Generates semantic versions** based on commit count and SHA
3. **Creates GitHub releases** with properly named assets:
   - `deadlock-api-ingest-windows-latest.exe` - Windows executable
   - `deadlock-api-ingest-ubuntu-latest` - Linux executable
4. **Provides installation instructions** in each release

The installation scripts automatically fetch the latest release, so you always get the most up-to-date version.

## Manual Installation

If you prefer to install manually, you can download the appropriate binary from the [releases page](https://github.com/deadlock-api/deadlock-api-ingest/releases) and set it up as a service yourself.

### Windows Manual Setup
1. Download `deadlock-api-ingest-windows-latest.exe`
2. Place it in `%LOCALAPPDATA%\deadlock-api-ingest\`
3. Create a scheduled task to run on user login (no admin required)

### Linux Manual Setup
1. Download `deadlock-api-ingest-ubuntu-latest`
2. Place it in `~/.local/share/deadlock-api-ingest/` or `~/.local/bin/`
3. Make it executable: `chmod +x deadlock-api-ingest`
4. Create a systemd user service file in `~/.config/systemd/user/`

## Privacy & Security

- **Local Processing**: All packet analysis is performed locally on your machine
- **Minimal Data**: Only extracts match metadata (IDs and salts) from replay URLs
- **No Personal Data**: Does not capture, store, or transmit personal information
- **Open Source**: Full source code is available for review

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support

If you encounter any issues, please open an issue on the [GitHub repository](https://github.com/deadlock-api/deadlock-api-ingest/issues)
