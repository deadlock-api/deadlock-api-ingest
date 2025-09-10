# Deadlock API Ingest

A transparent, open-source desktop application that helps the Deadlock gaming community by automatically collecting match metadata when you play Valve's Deadlock game.

## 🎯 What This Application Does

This application runs quietly in your system tray and watches for when Deadlock downloads match replay files. When it detects these downloads, it extracts useful information (like match IDs and server details) and shares it with the community API at deadlock-api.com to help build better tools and statistics for players.

## 🚀 Installation and Setup

### Download Pre-built Binary
1. Visit the [Releases page](https://github.com/deadlock-api/deadlock-api-ingest/releases)
2. Download the appropriate binary for your platform
3. Run the installer or extract the archive

### Platform-Specific Setup

#### Linux
```bash
# Install libpcap development headers
# Ubuntu/Debian:
sudo apt-get install libpcap-dev

# Fedora/RHEL:
sudo dnf install libpcap-devel

# Run with appropriate permissions
sudo ./deadlock-api-ingest
```

#### Windows
```bash
# Run as Administrator (required for packet monitoring)
# Right-click the executable and select "Run as administrator"
```

## Disclaimer
This application is not affiliated with Valve Corporation. It's a community tool designed to help Deadlock players and developers build better tools and statistics.

---

**Built with ❤️ for the Deadlock community**
