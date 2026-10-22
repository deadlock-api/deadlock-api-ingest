# Deadlock API Ingest

A lightweight background tool that monitors your Steam HTTP cache for Deadlock game replay files and automatically submits match metadata to the Deadlock API. This helps build a comprehensive database of Deadlock matches for the community.

## How It Works

The application scans Steam's local HTTP cache directory (`Steam/appcache/httpcache/`) for Deadlock replay URLs (`.meta.bz2` and `.dem.bz2` files). When it finds replay file references, it extracts the match IDs and salts, then submits them to the Deadlock API at `api.deadlock-api.com`. This allows the API to fetch and process match data from Valve's servers.

While Deadlock runs, it also reads match salts from the game's memory, read-only: the replies the game gets when it loads a match, and replay addresses it keeps. That covers matches whose files Steam hasn't downloaded. It never writes to or injects anything into the game. Pass `--no-memory` to turn this off.

While Deadlock is closed, every 30 minutes, it also asks Steam's Game Coordinator for match salts deadlock-api doesn't have yet. It logs in with the saved session of every Steam account on the PC that has "remember me" on (the refresh token in Steam's `local.vdf`), checks which of that account's Steam friends play Deadlock, and fetches their new matches first, then matches from deadlock-api's list of missing ones. Each account makes at most 40 metadata requests and 20 friend match-history requests a day. The salts go to deadlock-api like the cache ones, credited to the account that fetched them. Pass `--no-gc` to turn this off; the launch option below never runs it.

**Key Features:**
- 🔒 **Privacy-focused**: Sends only match ids and salts, your Steam account ID and, for the Steam requests, your friends' account IDs
- ⚡ **Lightweight**: Minimal CPU and memory usage
- 🔄 **Automatic**: Continuously monitors for new matches as you play
- 📦 **Runs without admin**: Application runs with standard user permissions (admin only needed for auto-start setup on Windows)

## Resources

- [DeepWiki Documentation](https://deepwiki.com/deadlock-api/deadlock-api-ingest)
## Quick Installation

### Windows (Installer)

Download [`deadlock-api-ingest-setup.exe`](https://github.com/deadlock-api/deadlock-api-ingest/releases/latest/download/deadlock-api-ingest-setup.exe) and run it. No admin rights or PowerShell needed:

- It installs to `%LOCALAPPDATA%\deadlock-api-ingest\`, starts automatically when you sign in, and adds a "Deadlock API Ingest" Start menu and desktop shortcut.
- When it finishes, the status window opens (see [Status Window](#status-window)).
- To remove it, use **Settings → Apps → Installed apps**.

> If Windows SmartScreen says it protected your PC, click **More info**, then **Run anyway**.

### Windows (PowerShell)

Run this command in PowerShell:

```powershell
irm https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1 | iex
```

Or download and run manually:

```powershell
Invoke-WebRequest -Uri "https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1" -OutFile "install-windows.ps1"
.\install-windows.ps1
```

> **⚠️ Auto-Start Permissions**: If you want the application to start automatically on system boot, you'll need to run PowerShell as Administrator. However, **the application itself runs without admin privileges** - you only need admin rights to create the scheduled task for auto-start. If you run the installer without admin rights, you can still install and run the application manually.
>
> The scheduled task runs while you are logged in. It needs your interactive logon to decrypt the saved Steam session (Windows DPAPI) for match-salt recovery via the Steam Game Coordinator.

### Linux (Bash)

Run this command:

```bash
curl -fsSL https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-linux.sh | bash
```

Without curl (Ubuntu's desktop install has wget instead):

```bash
wget -nv -O- https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-linux.sh | bash
```

Or download and run manually:

```bash
wget https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-linux.sh
chmod +x install-linux.sh
./install-linux.sh
```

> **Note**: The installation scripts automatically download the latest release binaries from GitHub and set up the application to run on user login. The application installs to your user directory and does not require elevated privileges.
>
> **Reading the game's memory**: most distributions only let a program read another's memory if it started it, and Steam starts Deadlock. The installer therefore offers a small helper, `/usr/local/libexec/deadlock-api-ingest/deadlock-api-ingest-memgrant`, which asks for your password once. It is the only part holding that right (`cap_sys_ptrace`), opens nothing but Deadlock's memory, read-only and for your user, and hands it to the application. Without it, starting the game through the [launch option](#steam-launch-option-alternative-to-background-service) works too.
>
> **Packaging**: a distribution package can ship the helper as `/usr/libexec/deadlock-api-ingest/deadlock-api-ingest-memgrant` or `/usr/lib/deadlock-api-ingest/deadlock-api-ingest-memgrant`, root-owned with `cap_sys_ptrace=ep`; the application looks there too.

### Docker

Run the pre-built Docker image:

```bash
docker run -d --restart unless-stopped --name deadlock-api-ingest \
  -v ~/.steam/steam:/root/.steam/steam:ro \
  ghcr.io/deadlock-api/deadlock-api-ingest:latest
```

> **Note**: The command mounts the whole Steam directory, read-only: the tool only recognizes it with its `steamapps` folder, and reads your account ID from `config/loginusers.vdf` to credit your uploads. `~/.steam/steam` is Steam's default on Linux; use `~/.var/app/com.valvesoftware.Steam/.local/share/Steam` for the Flatpak and `~/Library/Application Support/Steam` on macOS. The container cannot see the game's process, so it only reads Steam's cache, not the game's memory. With the whole Steam directory mounted it also runs the Steam requests described in [How It Works](#how-it-works) when it finds a saved session there; add `--no-gc` after the image name to turn them off.

### NixOS

#### Option 1: Using the NixOS Module (Recommended)

Add this flake to your NixOS configuration:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    deadlock-api-ingest.url = "github:deadlock-api/deadlock-api-ingest";
  };

  outputs = { self, nixpkgs, deadlock-api-ingest, ... }: {
    nixosConfigurations.your-hostname = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        # Import the module
        deadlock-api-ingest.nixosModules.default
        
        # Configure the service
        {
          services.deadlock-api-ingest = {
            enable = true;
            # IMPORTANT: Set this to the user who has Steam installed
            user = "your-steam-username";
            group = "users";
          };
        }
      ];
    };
  };
}
```

**Important:** Replace `"your-steam-username"` with your actual username that has Steam installed.

Then rebuild your system:

```bash
sudo nixos-rebuild switch --flake .#your-hostname
```

The service will automatically start and run in the background, monitoring your Steam cache.

#### Option 2: Run Directly

You can also run it directly without installing:

```bash
nix run github:deadlock-api/deadlock-api-ingest
```

#### Option 3: User Service (Manual)

For a user-level systemd service without the NixOS module:

```nix
# In your home-manager or systemd user services
systemd.user.services.deadlock-api-ingest = {
  Unit = {
    Description = "Deadlock API Ingest Service";
    After = [ "graphical-session.target" ];
  };

  Service = {
    ExecStart = "${pkgs.deadlock-api-ingest}/bin/deadlock-api-ingest";
    Restart = "on-failure";
    RestartSec = "10s";
  };

  Install = {
    WantedBy = [ "default.target" ];
  };
};
```

Then enable: `systemctl --user enable --now deadlock-api-ingest`

## Status Window

```bash
deadlock-api-ingest --gui
```

This opens a small window that shows:

- whether the background tool is running
- how many of your matches it has submitted (in total, in the last 24 hours and in the last 7 days)
- your most recent matches

Only your own matches are counted: those from your Steam cache, and your match history recovered with `--own-matches`. Matches the tool fetches for other players through the Steam Game Coordinator are not shown.

If the background tool isn't running, the window sets it up and starts it: a systemd user service on Linux, a per-user autostart entry on Windows. The window itself never uploads anything, so closing it changes nothing.

## Steam Launch Option (Alternative to Background Service)

Instead of running the ingest service as a persistent background process, you can configure it to run only while Deadlock is active by using Steam's launch options. The service will start when you launch the game and automatically stop when the game exits.

1. Download the binary to a known location (e.g., `~/.local/bin/deadlock-api-ingest` on Linux or `%LOCALAPPDATA%\deadlock-api-ingest\deadlock-api-ingest.exe` on Windows)
2. In Steam, right-click **Deadlock** → **Properties** → **General** → **Launch Options**
3. Set the launch option:

**Linux:**
```
/home/YOUR_USER/.local/bin/deadlock-api-ingest -- %command%
```

**Windows:**
```
"C:\Users\YOUR_USER\AppData\Local\deadlock-api-ingest\deadlock-api-ingest.exe" -- %command%
```

> **Note:** If you use this approach, you should disable or remove any existing background service (systemd, Task Scheduler, etc.) to avoid running two instances simultaneously.

## Uninstallation

### Windows

If you installed with `deadlock-api-ingest-setup.exe`, remove it under **Settings → Apps → Installed apps**. Otherwise run:

```powershell
& "$env:LOCALAPPDATA\deadlock-api-ingest\uninstall-windows.ps1"
```
**Or navigate to** `%LOCALAPPDATA%\deadlock-api-ingest\` and double-click `uninstall-windows.ps1`.

**Older Versions:**
```powershell
irm https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/uninstall-windows.ps1 | iex
```

### Linux

**Run the local uninstall script:**
```bash
~/.local/share/deadlock-api-ingest/uninstall-linux.sh
```

**Older Versions:**
```bash
curl -fsSL https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/uninstall-linux.sh | bash
```

> **Note:** The installer automatically copies the uninstall script to the installation directory for offline use.

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
3. Create a scheduled task that runs on user login, only while the user is logged on (no admin required)

### Linux Manual Setup
1. Download `deadlock-api-ingest-ubuntu-latest`
2. Place it in `~/.local/share/deadlock-api-ingest/` or `~/.local/bin/`
3. Make it executable: `chmod +x deadlock-api-ingest`
4. Create a systemd user service file in `~/.config/systemd/user/`

## Privacy & Security

- Only reads Steam's local cache files and, while Deadlock runs, the game's memory (read-only, `--no-memory` turns it off)
- **Minimal Data**: Looks only for match IDs and salts. It sends those and your Steam account ID (to credit your uploads) to the Deadlock API, and keeps a local log of what it submitted (`fetched-salts.jsonl`)
- **Steam sessions**: The Steam requests use the saved sessions of the Steam accounts on the PC. The sessions go only to Steam, never to deadlock-api. To find friends' new matches, the tool sends your Steam friends' account IDs to deadlock-api. `--no-gc` turns all of this off
- **Read-Only Access**: Never modifies files and never writes to or injects anything into the game
- **Open Source**: Full source code is available for review and audit

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support

If you encounter any issues, please open an issue on the [GitHub repository](https://github.com/deadlock-api/deadlock-api-ingest/issues)
