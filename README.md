# Deadlock API Ingest

A transparent, open-source desktop application that helps the Deadlock gaming community by automatically collecting match metadata when you play Valve's Deadlock game.

## 🎯 What This Application Does

**In Simple Terms:** This application runs quietly in your system tray and watches for when Deadlock downloads match replay files. When it detects these downloads, it extracts useful information (like match IDs and server details) and shares it with the community API at deadlock-api.com to help build better tools and statistics for players.

**Technical Summary:** A Tauri-based cross-platform desktop application that monitors HTTP traffic on port 80 to detect Valve's Deadlock replay file downloads (.meta.bz2 files), extracts metadata (cluster ID, match ID, and metadata salt), and submits this information to the Deadlock API service.

## 🔍 Complete Transparency: What Data We Access

### Network Monitoring
- **What we monitor:** HTTP traffic on port 80 (standard web traffic)
- **Why we need this:** Deadlock downloads replay files over HTTP, and we need to detect these downloads
- **What we capture:** Only HTTP requests to Valve's replay servers (replay*.valve.net)
- **What we ignore:** All other network traffic is discarded immediately

### Data We Extract and Send
From Deadlock replay URLs like `http://replay404.valve.net/1422450/37959196_937530290.meta.bz2`, we extract:
- **Cluster ID** (404) - Which Valve server hosted the match
- **Match ID** (37959196) - Unique identifier for the match
- **Metadata Salt** (937530290) - A cryptographic value used by Valve

### External API Calls
- **Destination:** `https://api.deadlock-api.com/v1/matches/salts`
- **Purpose:** Share match metadata with the community API
- **Data sent:** Only the three values above (cluster ID, match ID, metadata salt)
- **No personal data:** We never send your IP, username, or any personal information

## 🛡️ Privacy and Security Measures

### What We DON'T Collect
- Personal information (usernames, emails, etc.)
- Game statistics or performance data
- Other websites you visit
- Files on your computer
- Any data from other applications

### Security Features
- **No unsafe code:** The application uses `#![forbid(unsafe_code)]` to prevent memory safety issues
- **Minimal permissions:** Only requests network monitoring capabilities
- **Open source:** All code is publicly auditable
- **No data storage:** We don't store any data locally beyond temporary deduplication

### Why These Permissions Are Needed
- **Network monitoring:** Required to detect Deadlock replay downloads
- **System tray access:** To run quietly in the background
- **Autostart capability:** To automatically start with your computer (optional)

## 🏗️ Architecture and Design Decisions

### Cross-Platform Design
We use different network monitoring approaches for each platform:
- **Linux:** Uses `pcap` library for packet capture
- **Windows:** Uses `pktmon` for packet monitoring
- **Why different approaches:** Each OS has different capabilities and security models

### Tauri Framework Choice
- **Why Tauri:** Provides secure, lightweight desktop apps with web technologies
- **Benefits:** Small binary size, security-focused, cross-platform compatibility
- **Trade-offs:** More complex than native apps, but much more secure than Electron

### Network Monitoring Strategy
- **Packet-level monitoring:** More reliable than browser extensions or game hooks
- **HTTP-only focus:** We only look at unencrypted HTTP traffic (port 80)
- **Minimal processing:** Packets are processed immediately and discarded

## 🚀 Installation and Setup

### Prerequisites
- **Windows:** Windows 10 or later
- **Linux:** Modern distribution with libpcap support
- **Administrator/root access:** Required for network monitoring

### Installation Options

#### Option 1: Download Pre-built Binary
1. Visit the [Releases page](https://github.com/deadlock-api/deadlock-api-ingest/releases)
2. Download the appropriate binary for your platform
3. Run the installer or extract the archive

#### Option 2: Build from Source (Recommended for Security)
```bash
# Clone the repository
git clone https://github.com/deadlock-api/deadlock-api-ingest.git
cd deadlock-api-ingest

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Tauri CLI
cargo install tauri-cli

# Build the application
cargo tauri build
```

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

## 🎮 Usage

### Starting the Application
1. Launch the application (requires administrator/root privileges)
2. Look for the Deadlock API icon in your system tray
3. The application will automatically start monitoring when Deadlock is running

### What You'll See
- **System tray icon:** Indicates the application is running
- **Log messages:** (if running from terminal) Shows when matches are detected
- **No UI:** The application runs silently in the background

### Stopping the Application
- Right-click the system tray icon and select "Quit"
- Or close from Task Manager/Activity Monitor

## 🔧 Configuration and Customization

### Environment Variables
```bash
# Enable debug logging
RUST_LOG=debug ./deadlock-api-ingest

# Custom log levels
RUST_LOG=deadlock_api_ingest=info,reqwest=warn ./deadlock-api-ingest
```

### Autostart Configuration
The application can automatically start with your computer:
- **Enable:** The autostart plugin is included by default
- **Disable:** Remove the autostart plugin from the build configuration

## 🛠️ Development and Building

### Development Setup
```bash
# Clone and enter directory
git clone https://github.com/deadlock-api/deadlock-api-ingest.git
cd deadlock-api-ingest

# Install dependencies
cargo fetch

# Run in development mode
cargo tauri dev
```

### Build Process Verification
Our build process is designed to be reproducible:

1. **Rust toolchain:** Pinned to specific version in `rust-toolchain.toml`
2. **Dependencies:** Locked versions in `Cargo.lock`
3. **Build script:** Minimal `build.rs` with no external downloads
4. **No network access:** Build process doesn't download additional code

### Code Quality Standards
- **Strict linting:** Multiple clippy lint levels enabled
- **No unsafe code:** Memory safety guaranteed
- **Error handling:** Comprehensive error handling with `anyhow`
- **Logging:** Structured logging with `tracing`

## 📋 Dependencies Explained

### Core Dependencies
- **`tauri`** (v2): Cross-platform desktop app framework - chosen for security and small binary size
- **`reqwest`** (v0.12.23): HTTP client for API calls - industry standard, well-audited
- **`serde_json`** (v1): JSON serialization - required for API communication
- **`anyhow`** (v1.0.99): Error handling - provides better error messages
- **`tracing`** (v0.1.41): Structured logging - helps with debugging and monitoring

### Platform-Specific Dependencies
- **`pcap`** (v2.3.0, Linux): Packet capture library - standard for network monitoring on Unix
- **`pktmon`** (v0.6.2, Windows): Windows packet monitoring - uses Windows built-in packet monitor
- **`regex`** (v1.11.2): Pattern matching for URL parsing - needed to extract match data

### Build Dependencies
- **`tauri-build`** (v2): Build-time code generation for Tauri apps

### Why These Dependencies
Each dependency was chosen for:
- **Security:** Well-maintained, audited libraries
- **Reliability:** Stable APIs with good track records
- **Minimal footprint:** Avoiding unnecessary bloat
- **Platform support:** Cross-platform compatibility

## 🚨 Troubleshooting

### Common Issues

#### "Permission Denied" Errors
**Problem:** Application can't monitor network traffic
**Solution:** 
- **Linux:** Run with `sudo`
- **Windows:** Run as Administrator
- **Why needed:** Network monitoring requires elevated privileges

#### "No Network Device Found" (Linux)
**Problem:** Can't find network interface to monitor
**Solutions:**
- Check if `libpcap-dev` is installed
- Verify network interfaces with `ip link show`
- Try running with different network interface

#### "Failed to Start Packet Monitor" (Windows)
**Problem:** Windows packet monitoring fails
**Solutions:**
- Ensure running as Administrator
- Check if Windows Packet Monitor service is available
- Verify Windows version compatibility (Windows 10+)

#### No Matches Detected
**Problem:** Application runs but doesn't detect Deadlock matches
**Possible causes:**
- Deadlock not downloading replay files (some matches don't generate replays)
- Network traffic encrypted or on different port
- Firewall blocking packet capture

### Debug Mode
Enable detailed logging to diagnose issues:
```bash
RUST_LOG=debug ./deadlock-api-ingest
```

### Verifying Operation
To confirm the application is working:
1. Check system tray for the icon
2. Look for log messages when starting
3. Monitor network activity during Deadlock matches

## 🤝 Contributing and Governance

### Project Governance
- **Open Source:** MIT licensed, fully transparent
- **Community driven:** Accepts contributions from the community
- **Issue tracking:** GitHub Issues for bug reports and feature requests
- **Code review:** All changes reviewed before merging

### How to Contribute
1. **Report issues:** Use GitHub Issues for bugs or suggestions
2. **Submit code:** Fork, create feature branch, submit pull request
3. **Documentation:** Help improve this README or add code comments
4. **Testing:** Test on different platforms and report compatibility

### Development Process
- **Code standards:** Strict linting and formatting requirements
- **Testing:** Comprehensive testing before releases
- **Security review:** All network-related code carefully reviewed
- **Transparency:** All development discussions public

## 📄 Legal and Compliance

### License
This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

### Compliance
- **No personal data collection:** GDPR/CCPA compliant by design
- **Open source:** Fully auditable code
- **Community benefit:** Data shared for community tools and statistics

### Disclaimer
This application is not affiliated with Valve Corporation. It's a community tool designed to help Deadlock players and developers build better tools and statistics.

## 🔗 Links and Resources

- **GitHub Repository:** https://github.com/deadlock-api/deadlock-api-ingest
- **Issue Tracker:** https://github.com/deadlock-api/deadlock-api-ingest/issues
- **Deadlock API:** https://api.deadlock-api.com
- **Tauri Framework:** https://tauri.app
- **Rust Language:** https://rust-lang.org

## 📞 Support and Contact

- **Issues:** Create a GitHub issue for bugs or feature requests
- **Security concerns:** Email security issues privately to the maintainers
- **General questions:** Use GitHub Discussions

---

**Built with ❤️ for the Deadlock community**

*This README serves as both technical documentation and a transparency report. We believe in complete openness about what our software does and why.*
