# Deadlock API Ingest

A network packet capture tool that monitors HTTP traffic for Deadlock game replay files and ingests metadata to the Deadlock API.

## Quick Installation

### Windows (PowerShell)

Run this command in an **elevated PowerShell** (Run as Administrator):

```powershell
irm https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1 | iex
```

Or download and run manually:

```powershell
Invoke-WebRequest -Uri "https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1" -OutFile "install-windows.ps1"
.\install-windows.ps1
```

### Linux (Bash)

Run this command with **sudo privileges**:

```bash
curl -fsSL https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-linux.sh | sudo bash
```

Or download and run manually:

```bash
wget https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-linux.sh
chmod +x install-linux.sh
sudo ./install-linux.sh
```

> **Note**: The installation scripts automatically download the latest release binaries from GitHub and set up the application as a system service.

## Docker Usage

The easiest way to run deadlock-api-ingest is using Docker. The application is available as a pre-built Docker image.

### Requirements

**Important**: This application requires special network capabilities to capture packets:
- `--network host` or equivalent network access to monitor traffic
- `--cap-add NET_RAW --cap-add NET_ADMIN` for packet capture capabilities
- Or `--privileged` for full access (less secure but simpler)

### Quick Start

Run the latest version with a simple command:

```bash
docker run -d --name deadlock-api-ingest \
  --restart unless-stopped \
  --network host \
  --cap-add NET_RAW \
  --cap-add NET_ADMIN \
  ghcr.io/deadlock-api/deadlock-api-ingest:latest
```

### Docker Compose

For easier management, use this `docker-compose.yml`:

```yaml
version: '3.8'

services:
  deadlock-api-ingest:
    image: ghcr.io/deadlock-api/deadlock-api-ingest:latest
    container_name: deadlock-api-ingest
    restart: unless-stopped
    network_mode: host
    cap_add:
      - NET_RAW
      - NET_ADMIN
    environment:
      - RUST_LOG=info
    # Optional: mount a volume for persistent data/logs
    # volumes:
    #   - ./data:/app/data

  # Optional: Watchtower for automatic updates
  watchtower:
    image: containrrr/watchtower:latest
    container_name: watchtower
    restart: unless-stopped
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    environment:
      - WATCHTOWER_CLEANUP=true
      - WATCHTOWER_POLL_INTERVAL=3600  # Check every hour
      - WATCHTOWER_INCLUDE_STOPPED=true
    command: deadlock-api-ingest
```

Start with: `docker-compose up -d`

### Environment Variables

- `RUST_LOG`: Set logging level (default: `info`, options: `error`, `warn`, `info`, `debug`, `trace`)
- `RUST_BACKTRACE`: Enable backtraces on panic (set to `1` or `full`)

### Viewing Logs

```bash
# View logs
docker logs deadlock-api-ingest

# Follow logs in real-time
docker logs -f deadlock-api-ingest

# View last 100 lines
docker logs --tail 100 deadlock-api-ingest
```

### Updating

```bash
# Pull latest image
docker pull ghcr.io/deadlock-api/deadlock-api-ingest:latest

# Restart container with new image
docker-compose down && docker-compose up -d

# Or manually
docker stop deadlock-api-ingest
docker rm deadlock-api-ingest
# Run the docker run command again
```

### Troubleshooting

If the container fails to start:

1. **Check permissions**: Ensure Docker has the necessary capabilities
2. **Network access**: Verify `--network host` is used
3. **Logs**: Check container logs for specific error messages
4. **Host networking**: Some systems may require additional network configuration

## Uninstallation

### Windows
```powershell
# Stop and remove scheduled tasks (main + updater)
Stop-ScheduledTask -TaskName "deadlock-api-ingest" -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName "deadlock-api-ingest" -Confirm:$false -ErrorAction SilentlyContinue

Stop-ScheduledTask -TaskName "deadlock-api-ingest-updater" -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName "deadlock-api-ingest-updater" -Confirm:$false -ErrorAction SilentlyContinue

# Stop any running process (if still running)
Stop-Process -Name "deadlock-api-ingest" -Force -ErrorAction SilentlyContinue

# Remove installation directory and related data
Remove-Item "$env:ProgramFiles\deadlock-api-ingest" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "$env:ProgramData\deadlock-api-ingest" -Recurse -Force -ErrorAction SilentlyContinue
```

### Linux
```bash
# Stop and disable main service
sudo systemctl stop deadlock-api-ingest || true
sudo systemctl disable deadlock-api-ingest || true

# Stop and disable automatic updater (if installed)
sudo systemctl stop deadlock-api-ingest-updater.timer || true
sudo systemctl disable deadlock-api-ingest-updater.timer || true
sudo systemctl stop deadlock-api-ingest-updater.service || true
sudo systemctl disable deadlock-api-ingest-updater.service || true

# Remove systemd unit files
sudo rm -f /etc/systemd/system/deadlock-api-ingest.service
sudo rm -f /etc/systemd/system/deadlock-api-ingest-updater.service
sudo rm -f /etc/systemd/system/deadlock-api-ingest-updater.timer

# Reload systemd state
sudo systemctl daemon-reload
sudo systemctl reset-failed || true

# Remove installation and symlink
sudo rm -rf /opt/deadlock-api-ingest
sudo rm -f /usr/local/bin/deadlock-api-ingest

# Optional: remove updater log file (if present)
sudo rm -f /var/log/deadlock-api-ingest-updater.log
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
2. Place it in `C:\Program Files\deadlock-api-ingest\`
3. Create a Windows service using `sc.exe` or install as a startup program

### Linux Manual Setup
1. Download `deadlock-api-ingest-ubuntu-latest`
2. Place it in `/opt/deadlock-api-ingest/` or `/usr/local/bin/`
3. Make it executable: `chmod +x deadlock-api-ingest`
4. Create a systemd service file in `/etc/systemd/system/`

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
