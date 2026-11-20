#!/bin/bash

# Deadlock API Ingest - macOS Installation Script
# Downloads the application and runs it in the background as a per-user LaunchAgent.
# Written for the bash 3.2 that macOS ships.

# -E: the ERR trap below also fires inside functions, where all the work runs.
set -Eeuo pipefail

# --- Configuration ---
APP_NAME="deadlock-api-ingest"
GITHUB_REPO="deadlock-api/deadlock-api-ingest"
# The latest release's assets, by name; no API call or JSON parsing needed.
RELEASE_URL="https://github.com/$GITHUB_REPO/releases/latest/download"
# One universal binary for Apple Silicon and Intel Macs.
MAIN_ASSET="${APP_NAME}-macos"

# Installation paths (user-level, no admin rights needed). INSTALL_DIR is also where the
# application keeps its logs and data.
INSTALL_DIR="$HOME/Library/Application Support/$APP_NAME"
EXECUTABLE="$INSTALL_DIR/$APP_NAME"
# Outside INSTALL_DIR, which the old installation's uninstaller deletes, and on the same
# filesystem, so the checked binary can be moved into place.
DOWNLOAD_PATH="$HOME/Library/Application Support/.$APP_NAME-download"
BIN_DIR="$HOME/.local/bin"
APP_BUNDLE="$HOME/Applications/Deadlock API Ingest.app"

# Same label and agent as the application writes itself (src/service.rs).
LABEL="com.deadlock-api.deadlock-api-ingest"
LAUNCH_AGENT="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$(id -u)"

LOG_FILE="$HOME/Library/Logs/$APP_NAME/install.log"

# --- Colors for output ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# --- Helper Functions ---

# Writes to the console (stderr, so command substitutions don't capture it) and the log file.
log() {
    local level="$1"
    local message="$2"
    local timestamp color
    timestamp=$(date '+%Y-%m-%d %H:%M:%S')

    case "$level" in
        "ERROR")   color="$RED" ;;
        "WARN")    color="$YELLOW" ;;
        "SUCCESS") color="$GREEN" ;;
        "INFO")    color="$BLUE" ;;
        *)         color="$NC" ;;
    esac

    local log_message="[$timestamp] [$level] $message"
    echo -e "${color}${log_message}${NC}" >&2
    mkdir -p "$(dirname "$LOG_FILE")"
    echo "$log_message" >> "$LOG_FILE"
}

# Downloads $1 to $2; $2 is replaced only once the download succeeds.
fetch() {
    local part="$2.part"
    if curl -fsSL --user-agent "Bash-Installer" -o "$part" "$1" 2>> "$LOG_FILE" && mv -f "$part" "$2"; then
        return 0
    fi
    echo "Download failed: $1" >> "$LOG_FILE"
    rm -f "$part"
    return 1
}

xml_escape() {
    printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

download_binary() {
    mkdir -p "$(dirname "$DOWNLOAD_PATH")"
    echo "Downloading from: $RELEASE_URL/$MAIN_ASSET" >> "$LOG_FILE"
    if ! fetch "$RELEASE_URL/$MAIN_ASSET" "$DOWNLOAD_PATH"; then
        log "ERROR" "Download failed (details in $LOG_FILE)."
        exit 1
    fi
    log "SUCCESS" "Download complete."
}

# The binary isn't signed by Apple. curl doesn't mark downloads as quarantined, so Gatekeeper
# doesn't stop it; the attribute is removed anyway in case something else added it.
# Running it once also stops the install, before the current installation is removed, if
# this Mac can't run it.
check_binary_runs() {
    local path="$1"
    chmod +x "$path"
    xattr -d com.apple.quarantine "$path" 2>/dev/null || true

    local version_output line
    if ! version_output=$("$path" --version 2>&1); then
        log "ERROR" "The downloaded binary does not run on this Mac:"
        while IFS= read -r line; do log "ERROR" "  $line"; done <<< "$version_output"
        rm -f "$path"
        exit 1
    fi
}

stop_existing() {
    launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
    rm -f "$LAUNCH_AGENT"
    # -f: macOS truncates process names to 16 characters, which "deadlock-api-ingest" exceeds.
    pkill -f "$EXECUTABLE" 2>/dev/null || true
}

install_launch_agent() {
    local exe
    exe=$(xml_escape "$EXECUTABLE")
    mkdir -p "$(dirname "$LAUNCH_AGENT")"
    # Started at login, restarted when it fails, but not after a clean exit (e.g. when
    # another copy already runs).
    cat > "$LAUNCH_AGENT" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>$exe</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>10</integer>
</dict>
</plist>
EOF
    chmod 644 "$LAUNCH_AGENT"

    if ! launchctl bootstrap "$DOMAIN" "$LAUNCH_AGENT" 2>> "$LOG_FILE"; then
        log "ERROR" "Could not load the LaunchAgent (details in $LOG_FILE)."
        exit 1
    fi
    sleep 3
    if pgrep -f "$EXECUTABLE" >/dev/null 2>&1; then
        log "SUCCESS" "Background service started; it starts automatically when you log in."
    else
        log "ERROR" "The background service did not start."
        log "INFO" "Its logs are in: $INSTALL_DIR/logs/"
        exit 1
    fi
}

# "Deadlock API Ingest" in ~/Applications (Launchpad, Spotlight): opens the status window,
# which also starts the background service if it isn't running. The Nix package builds the
# same bundle from the same files (default.nix).
create_app_bundle() {
    local contents="$APP_BUNDLE/Contents"
    local assets="https://raw.githubusercontent.com/$GITHUB_REPO/master/assets/macos"
    rm -rf "$APP_BUNDLE"
    mkdir -p "$contents/MacOS" "$contents/Resources"

    if ! fetch "$assets/Info.plist" "$contents/Info.plist"; then
        rm -rf "$APP_BUNDLE"
        return 1
    fi
    local launcher="$contents/MacOS/$APP_NAME-status"
    # printf %q quotes the path for the shell, spaces included.
    printf '#!/bin/sh\nexec %q --gui\n' "$EXECUTABLE" > "$launcher"
    chmod 755 "$launcher"
    if ! fetch "$assets/AppIcon.icns" "$contents/Resources/AppIcon.icns"; then
        log "WARN" "Could not download the app icon; the app uses a generic one."
    fi
    log "SUCCESS" "Status window app created: $APP_BUNDLE"
}

# Steam doesn't look in ~/.local/bin for launch options, so the hint names the full path,
# quoted for the space in "Application Support".
log_launch_option_hint() {
    log "INFO" "Starting the game with this Steam launch option also works:"
    log "INFO" "  \"$EXECUTABLE\" -- %command%"
}

# --- Main Installation Logic ---
main() {
    if [[ "$(uname -s)" != "Darwin" ]]; then
        echo -e "${RED}This installer is for macOS; on Linux use install-linux.sh.${NC}" >&2
        exit 1
    fi
    # Everything goes into the home of the user running the script, and the LaunchAgent is
    # that user's, so under sudo it would all be root's.
    if [[ $EUID -eq 0 ]]; then
        echo -e "${RED}Run the installer as your own user: run the same command again without sudo.${NC}" >&2
        exit 1
    fi

    log "INFO" "Starting Deadlock API Ingest installation..."
    log "INFO" "Log file is available at: $LOG_FILE"

    download_binary
    check_binary_runs "$DOWNLOAD_PATH"

    local existing_uninstall_script="$INSTALL_DIR/uninstall-macos.sh"
    if [[ -f "$existing_uninstall_script" ]]; then
        log "INFO" "Found existing installation. Running uninstall script..."
        if bash "$existing_uninstall_script" --silent >> "$LOG_FILE" 2>&1; then
            log "SUCCESS" "Previous installation uninstalled successfully."
        else
            log "WARN" "Uninstall script failed, continuing with manual cleanup."
        fi
    fi
    stop_existing

    mkdir -p "$INSTALL_DIR"
    mv -f "$DOWNLOAD_PATH" "$EXECUTABLE"
    mkdir -p "$BIN_DIR"
    ln -sf "$EXECUTABLE" "$BIN_DIR/$APP_NAME"
    log "SUCCESS" "Application installed to: $EXECUTABLE"

    local uninstall_script_url="https://raw.githubusercontent.com/$GITHUB_REPO/master/uninstall-macos.sh"
    if fetch "$uninstall_script_url" "$existing_uninstall_script"; then
        chmod +x "$existing_uninstall_script"
    else
        log "WARN" "Failed to download the uninstall script; get it from: $uninstall_script_url"
    fi

    install_launch_agent
    create_app_bundle || log "WARN" "Could not create the app. Open the status window with: $BIN_DIR/$APP_NAME --gui"
    log_launch_option_hint

    log "SUCCESS" "🚀 Deadlock API Ingest has been installed successfully!"

    {
        echo
        echo -e "${GREEN}Installation complete.${NC}"
        echo
        echo -e "Open ${YELLOW}Deadlock API Ingest${NC} from Launchpad or Spotlight to see the matches it has sent."
        echo
        echo -e "You can manage the background service with the following commands:"
        echo -e "  - Check status:  ${YELLOW}launchctl print $DOMAIN/$LABEL${NC}"
        echo -e "  - Log files:     ${YELLOW}~/Library/Application Support/$APP_NAME/logs/${NC}"
        echo -e "  - Stop service:  ${YELLOW}launchctl bootout $DOMAIN/$LABEL${NC}"
        echo -e "  - Start service: ${YELLOW}launchctl bootstrap $DOMAIN ~/Library/LaunchAgents/$LABEL.plist${NC}"
        echo
        echo -e "To uninstall, run: ${YELLOW}bash ~/Library/Application\\ Support/$APP_NAME/uninstall-macos.sh${NC}"
        echo
    } >&2
}

# Graceful error handling
trap 'log "ERROR" "An unexpected error occurred at line $LINENO. Installation failed."' ERR

main "$@"
