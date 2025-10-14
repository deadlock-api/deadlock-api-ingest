#!/bin/bash

# Deadlock API Ingest - Linux Installation Script
# This script downloads and installs the deadlock-api-ingest application as a systemd service

set -euo pipefail

# --- Configuration ---
APP_NAME="deadlock-api-ingest"
GITHUB_REPO="deadlock-api/deadlock-api-ingest"
ASSET_KEYWORD="ubuntu-latest" # Keyword to find in the release asset filename

# Installation paths
INSTALL_DIR="/opt/$APP_NAME"
BIN_DIR="/usr/local/bin"
FINAL_EXECUTABLE_NAME=$APP_NAME

# Service and logging
SERVICE_NAME=$APP_NAME
LOG_FILE="/tmp/${APP_NAME}-install.log"
SYSTEMD_SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"

# Update functionality
UPDATE_SERVICE_NAME="${APP_NAME}-updater"
UPDATE_TIMER_NAME="${APP_NAME}-updater"
UPDATE_SCRIPT_PATH="$INSTALL_DIR/update-checker.sh"
UPDATE_LOG_FILE="/var/log/${APP_NAME}-updater.log"
VERSION_FILE="$INSTALL_DIR/version.txt"
CONFIG_FILE="$INSTALL_DIR/config.conf"
BACKUP_DIR="$INSTALL_DIR/backup"
UPDATE_SYSTEMD_SERVICE_FILE="/etc/systemd/system/${UPDATE_SERVICE_NAME}.service"
UPDATE_SYSTEMD_TIMER_FILE="/etc/systemd/system/${UPDATE_TIMER_NAME}.timer"

# --- Colors for output ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# --- Helper Functions ---

# Function to write to log and console
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

    # --- FIX APPLIED HERE ---
    # All console output is redirected to stderr (>&2) to prevent it from being
    # captured by command substitutions like `var=$(...)`.
    echo -e "${color}${log_message}${NC}" >&2

    # The log file is written to separately and is unaffected.
    echo "$log_message" >> "$LOG_FILE"
}

# Function to execute commands quietly while logging details
execute_quietly() {
    local cmd=("$@")

    # Execute command and capture both stdout and stderr to log file
    if "${cmd[@]}" >> "$LOG_FILE" 2>&1; then
        return 0
    else
        local exit_code=$?
        log "ERROR" "Command failed: ${cmd[*]}"
        return $exit_code
    fi
}

# Function to check if running with root/sudo privileges
check_privileges() {
    if [[ $EUID -ne 0 ]]; then
        log "INFO" "This script requires root privileges. Attempting to restart with sudo..."
        if ! command -v sudo >/dev/null 2>&1; then
            log "ERROR" "sudo is not available. Please run this script as root."
            exit 1
        fi
        exec sudo -- "$0" "$@"
    fi
}

# Function to install required dependencies
install_dependencies() {
    local pkgs_to_install=()
    for pkg in curl wget jq; do
        command -v "$pkg" >/dev/null 2>&1 || pkgs_to_install+=("$pkg")
    done

    local libpcap_pkg="libpcap"
    if command -v apt-get >/dev/null 2>&1; then
        libpcap_pkg="libpcap0.8"
    fi

    if command -v dpkg-query >/dev/null 2>&1; then
        dpkg-query -W -f='${Status}' "$libpcap_pkg" 2>/dev/null | grep -q "install ok installed" || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v rpm >/dev/null 2>&1; then
        rpm -q "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v pacman >/dev/null 2>&1; then
        pacman -Q "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v apk >/dev/null 2>&1; then
        apk info -e "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v pkg >/dev/null 2>&1; then
        pkg info "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v pkgutil >/dev/null 2>&1; then
        pkgutil --pkg-info "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v pkg_add >/dev/null 2>&1; then
        pkg_info -e "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v pkgin >/dev/null 2>&1; then
        pkgin list "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v pkgconf >/dev/null 2>&1; then
        pkgconf --exists "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    if command -v pkg-config >/dev/null 2>&1; then
        pkg-config --exists "$libpcap_pkg" >/dev/null 2>&1 || pkgs_to_install+=("$libpcap_pkg")
    fi

    # Symlink libpcap.so to libpcap.so.0.8 if it exists but the latter does not
    if [[ ! -f /usr/lib/libpcap.so.0.8 && ! -f /usr/lib64/libpcap.so.0.8 ]]; then
        libpcap_path=$(find /usr/lib /usr/lib64 /lib /lib64 /usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu -type f -name 'libpcap.so*' 2>/dev/null | head -n 1 || true)

        if [[ -n "$libpcap_path" ]]; then
            dest="$(dirname "$libpcap_path")/libpcap.so.0.8"
            if [[ ! -e "$dest" ]]; then
                ln -s "$libpcap_path" "$dest" 2>/dev/null || true
                log "INFO" "Created symlink: $dest -> $libpcap_path"
            fi
        fi
    fi

    if [[ ${#pkgs_to_install[@]} -eq 0 ]]; then
        log "SUCCESS" "All dependencies are already installed."
        return
    fi

    log "INFO" "Installing dependencies: ${pkgs_to_install[*]}"
    if command -v apt-get >/dev/null 2>&1; then
        execute_quietly apt-get update -qq
        execute_quietly apt-get install -y "${pkgs_to_install[@]}"
    elif command -v dnf >/dev/null 2>&1; then
        execute_quietly dnf install -y "${pkgs_to_install[@]}"
    elif command -v yum >/dev/null 2>&1; then
        execute_quietly yum install -y "${pkgs_to_install[@]}"
    elif command -v pacman >/dev/null 2>&1; then
        execute_quietly pacman -Sy --noconfirm "${pkgs_to_install[@]}"
    elif command -v apk >/dev/null 2>&1; then
        execute_quietly apk add --no-cache "${pkgs_to_install[@]}"
    elif command -v zypper >/dev/null 2>&1; then
        execute_quietly zypper install -y "${pkgs_to_install[@]}"
    elif command -v emerge >/dev/null 2>&1; then
        execute_quietly emerge -v "${pkgs_to_install[@]}"
    elif command -v xbps-install >/dev/null 2>&1; then
        execute_quietly xbps-install -y "${pkgs_to_install[@]}"
    else
        log "WARN" "Could not detect package manager. Please install missing packages manually: ${pkgs_to_install[*]}"
        return
    fi

    log "SUCCESS" "Dependencies installed successfully."
}

# Function to get latest release info from GitHub API
get_latest_release() {
    local api_url="https://api.github.com/repos/$GITHUB_REPO/releases/latest"

    local response
    response=$(curl -s -f --user-agent "Bash-Installer" "$api_url") || {
        log "ERROR" "Failed to fetch release information from GitHub API."
        exit 1
    }

    local asset_info
    asset_info=$(echo "$response" | jq --arg keyword "$ASSET_KEYWORD" '([.assets[] | select(.name | contains($keyword))])[0]')

    if [[ -z "$asset_info" || "$asset_info" == "null" ]]; then
        log "ERROR" "Could not find a release asset containing the keyword: '$ASSET_KEYWORD'"
        log "INFO" "Available assets are: $(echo "$response" | jq -r '.assets[].name')"
        exit 1
    fi

    local version download_url size
    version=$(echo "$response" | jq -r '.tag_name')
    download_url=$(echo "$asset_info" | jq -r '.browser_download_url')
    size=$(echo "$asset_info" | jq -r '.size')

    log "INFO" "Found version: $version"

    # This is the only line that should print to stdout, to be captured by the calling variable.
    echo "$version|$download_url|$size"
}

# Function to download file with progress
download_file() {
    local url="$1"
    local output_path="$2"
    local expected_size="$3"

    mkdir -p "$(dirname "$output_path")"

    # Log detailed info to file, show simple progress to user
    echo "Downloading from: $url" >> "$LOG_FILE"
    echo "Saving to: $output_path" >> "$LOG_FILE"

    if ! wget --progress=dot:giga --user-agent="Bash-Installer" -O "$output_path" "$url" 2>> "$LOG_FILE"; then
        log "ERROR" "Download failed."
        exit 1
    fi

    log "SUCCESS" "Download complete."

    local actual_size
    actual_size=$(stat -c%s "$output_path")

    if [[ "$actual_size" != "$expected_size" ]]; then
        log "ERROR" "File size mismatch! Expected: $expected_size bytes, Got: $actual_size bytes."
        exit 1
    fi
}

# Function to download the update checker script
download_update_checker() {
    local update_script_url="https://raw.githubusercontent.com/$GITHUB_REPO/master/update-checker.sh"

    if ! wget --quiet --user-agent="Bash-Installer" -O "$UPDATE_SCRIPT_PATH" "$update_script_url" 2>> "$LOG_FILE"; then
        log "ERROR" "Failed to download update checker script."
        exit 1
    fi

    chmod +x "$UPDATE_SCRIPT_PATH"
    log "SUCCESS" "Update checker script installed."
}

# Function to manage the systemd service
manage_service() {
    local action="$1"

    case "$action" in
        "remove")
            if systemctl is-active --quiet "$SERVICE_NAME"; then
                execute_quietly systemctl stop "$SERVICE_NAME"
            fi
            if systemctl is-enabled --quiet "$SERVICE_NAME"; then
                execute_quietly systemctl disable "$SERVICE_NAME"
            fi
            if [[ -f "$SYSTEMD_SERVICE_FILE" ]]; then
                rm -f "$SYSTEMD_SERVICE_FILE"
            fi
            systemctl daemon-reload 2>> "$LOG_FILE"
            ;;
        "create")
            local executable_path="$2"

            cat > "$SYSTEMD_SERVICE_FILE" << EOF
[Unit]
Description=Deadlock API Ingest Service
Documentation=https://github.com/$GITHUB_REPO
After=network.target
Wants=network.target

[Service]
Type=simple
User=root
Group=root
ExecStart=$executable_path
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=$SERVICE_NAME

# Security Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF
            chmod 644 "$SYSTEMD_SERVICE_FILE"
            systemctl daemon-reload 2>> "$LOG_FILE"
            log "SUCCESS" "System service created."
            ;;
        "start")
            execute_quietly systemctl enable "$SERVICE_NAME"
            execute_quietly systemctl start "$SERVICE_NAME"

            sleep 3

            if systemctl is-active --quiet "$SERVICE_NAME"; then
                log "SUCCESS" "Application service started successfully."
            else
                log "ERROR" "Service failed to start. Please check the logs."
                log "INFO" "To check status: systemctl status $SERVICE_NAME"
                log "INFO" "To view logs: journalctl -u $SERVICE_NAME -n 50"
                exit 1
            fi
            ;;
    esac
}

# Function to manage the update systemd service and timer
manage_update_service() {
    local action="$1"

    case "$action" in
        "remove")
            # Stop and disable timer
            if systemctl is-active --quiet "$UPDATE_TIMER_NAME.timer"; then
                execute_quietly systemctl stop "$UPDATE_TIMER_NAME.timer"
            fi
            if systemctl is-enabled --quiet "$UPDATE_TIMER_NAME.timer"; then
                execute_quietly systemctl disable "$UPDATE_TIMER_NAME.timer"
            fi

            # Stop and disable service
            if systemctl is-active --quiet "$UPDATE_SERVICE_NAME.service"; then
                execute_quietly systemctl stop "$UPDATE_SERVICE_NAME.service"
            fi
            if systemctl is-enabled --quiet "$UPDATE_SERVICE_NAME.service"; then
                execute_quietly systemctl disable "$UPDATE_SERVICE_NAME.service"
            fi

            # Remove files
            if [[ -f "$UPDATE_SYSTEMD_TIMER_FILE" ]]; then
                rm -f "$UPDATE_SYSTEMD_TIMER_FILE"
            fi
            if [[ -f "$UPDATE_SYSTEMD_SERVICE_FILE" ]]; then
                rm -f "$UPDATE_SYSTEMD_SERVICE_FILE"
            fi

            systemctl daemon-reload 2>> "$LOG_FILE"
            ;;
        "create")
            # Create the update service file
            cat > "$UPDATE_SYSTEMD_SERVICE_FILE" << EOF
[Unit]
Description=Deadlock API Ingest Update Checker
Documentation=https://github.com/$GITHUB_REPO
After=network.target
Wants=network.target

[Service]
Type=oneshot
User=root
Group=root
ExecStart=$UPDATE_SCRIPT_PATH
StandardOutput=journal
StandardError=journal
SyslogIdentifier=$UPDATE_SERVICE_NAME

# Security Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
EOF

            # Create the timer file
            cat > "$UPDATE_SYSTEMD_TIMER_FILE" << EOF
[Unit]
Description=Daily Update Check for Deadlock API Ingest
Requires=$UPDATE_SERVICE_NAME.service

[Timer]
OnCalendar=daily
RandomizedDelaySec=1800
Persistent=true

[Install]
WantedBy=timers.target
EOF

            chmod 644 "$UPDATE_SYSTEMD_SERVICE_FILE"
            chmod 644 "$UPDATE_SYSTEMD_TIMER_FILE"
            systemctl daemon-reload 2>> "$LOG_FILE"
            log "SUCCESS" "Automatic update service created."
            ;;
        "start")
            execute_quietly systemctl enable "$UPDATE_TIMER_NAME.timer"
            execute_quietly systemctl start "$UPDATE_TIMER_NAME.timer"

            if systemctl is-active --quiet "$UPDATE_TIMER_NAME.timer"; then
                log "SUCCESS" "Automatic updates enabled."
                # Log detailed timer info to file only
                echo "Next update check: $(systemctl list-timers --no-pager | grep "$UPDATE_TIMER_NAME" | awk '{print $1, $2}')" >> "$LOG_FILE"
            else
                log "ERROR" "Update timer failed to start."
                log "INFO" "To check status: systemctl status $UPDATE_TIMER_NAME.timer"
                exit 1
            fi
            ;;
    esac
}

# Function to create configuration file
create_config_file() {
    cat > "$CONFIG_FILE" << EOF
# Deadlock API Ingest Configuration
# This file controls various settings for the application and updater

# Automatic Updates
# Set to "false" to disable automatic updates
AUTO_UPDATE="true"

# Update Check Time
# The timer runs daily, but you can manually trigger updates with:
# systemctl start $UPDATE_SERVICE_NAME.service

# Backup Retention
# Number of backup versions to keep (default: 5)
BACKUP_RETENTION=5

# Update Log Level
# Options: INFO, WARN, ERROR
UPDATE_LOG_LEVEL="INFO"
EOF

    chmod 644 "$CONFIG_FILE"
    log "SUCCESS" "Configuration file created at $CONFIG_FILE"
}

# Function to store version information
store_version_info() {
    local version="$1"
    echo "$version" > "$VERSION_FILE"
    log "INFO" "Version information stored: $version"
}

# Function to prompt user for automatic updater setup
prompt_for_updater() {
    # Check if we're running in an interactive terminal
    if [[ ! -t 0 ]] || [[ ! -t 1 ]]; then
        log "INFO" "Non-interactive mode detected. Installing automatic updater by default."
        return 0
    fi

    log "INFO" "The automatic updater will check for new versions daily and install them automatically."
    log "INFO" "This helps keep your installation secure and up-to-date with the latest features."
    echo >&2

    local attempts=0
    local max_attempts=2

    while [[ $attempts -lt $max_attempts ]]; do
        echo -n "Would you like to set up automatic updates? (y/n): " >&2

        local response
        if read -t 10 -r response; then
            case "${response,,}" in
                y|yes)
                    return 0
                    ;;
                n|no)
                    return 1
                    ;;
                *)
                    attempts=$((attempts + 1))
                    if [[ $attempts -lt $max_attempts ]]; then
                        echo "Invalid response. Please enter 'y' for yes or 'n' for no." >&2
                    fi
                    ;;
            esac
        else
            log "INFO" "No response received within 10 seconds. Installing automatic updater by default."
            return 0
        fi
    done

    log "INFO" "Maximum attempts reached. Installing automatic updater by default."
    return 0
}

# --- Main Installation Logic ---
main() {
    log "INFO" "Starting Deadlock API Ingest installation..."
    log "INFO" "Log file is available at: $LOG_FILE"

    check_privileges "$@"
    install_dependencies

    local release_info
    release_info=$(get_latest_release)
    local version download_url size
    IFS='|' read -r version download_url size <<< "$release_info"

    if [[ -z "$version" || -z "$download_url" ]]; then
        log "ERROR" "Failed to parse release information. Cannot continue."
        exit 1
    fi

    # Remove existing services (both main and update)
    manage_service "remove"
    manage_update_service "remove"

    killall "$APP_NAME" 2>/dev/null || true

    mkdir -p "$INSTALL_DIR"
    mkdir -p "$BACKUP_DIR"

    local temp_download_path="$INSTALL_DIR/${APP_NAME}-${version}"
    download_file "$download_url" "$temp_download_path" "$size"

    local final_executable_path="$INSTALL_DIR/$FINAL_EXECUTABLE_NAME"
    mv "$temp_download_path" "$final_executable_path"
    chmod +x "$final_executable_path"

    local bin_symlink="$BIN_DIR/$FINAL_EXECUTABLE_NAME"
    ln -sf "$final_executable_path" "$bin_symlink"
    log "SUCCESS" "Application installed successfully."

    # Store version information
    store_version_info "$version"

    # Create configuration file
    create_config_file

    # Create and start main service
    manage_service "create" "$final_executable_path"
    manage_service "start"

    # Prompt user for automatic updater setup
    if prompt_for_updater; then
        # Download update checker script
        download_update_checker

        # Create and start update service/timer
        manage_update_service "create"
        manage_update_service "start"

        log "SUCCESS" "Automatic updater has been installed and configured."
    else
        log "INFO" "Skipping automatic updater installation as requested."
    fi

    log "SUCCESS" "🚀 Deadlock API Ingest ($version) has been installed successfully!"

    # The final messages should also be sent to stderr to not interfere with any potential scripting.
    {
        echo
        if systemctl is-enabled --quiet "$UPDATE_TIMER_NAME.timer" 2>/dev/null; then
            echo -e "${GREEN}Installation complete with automatic updates enabled.${NC}"
        else
            echo -e "${GREEN}Installation complete.${NC}"
        fi
        echo
        echo -e "You can manage the main service with the following commands:"
        echo -e "  - Check status:  ${YELLOW}systemctl status $SERVICE_NAME${NC}"
        echo -e "  - View logs:     ${YELLOW}journalctl -u $SERVICE_NAME -f${NC}"
        echo -e "  - Stop service:  ${YELLOW}systemctl stop $SERVICE_NAME${NC}"
        echo -e "  - Start service: ${YELLOW}systemctl start $SERVICE_NAME${NC}"
        echo

        if systemctl is-enabled --quiet "$UPDATE_TIMER_NAME.timer" 2>/dev/null; then
            echo -e "Automatic update functionality:"
            echo -e "  - Update timer:  ${YELLOW}systemctl status $UPDATE_TIMER_NAME.timer${NC}"
            echo -e "  - Update logs:   ${YELLOW}journalctl -u $UPDATE_SERVICE_NAME -f${NC}"
            echo -e "  - Manual update: ${YELLOW}systemctl start $UPDATE_SERVICE_NAME.service${NC}"
            echo -e "  - Disable updates: Edit ${YELLOW}$CONFIG_FILE${NC} and set AUTO_UPDATE=\"false\""
            echo
            echo -e "Update logs: ${YELLOW}$UPDATE_LOG_FILE${NC}"
        else
            echo -e "To enable automatic updates later, you can re-run this installer."
        fi
        echo
        echo -e "Configuration file: ${YELLOW}$CONFIG_FILE${NC}"
        echo -e "Version file: ${YELLOW}$VERSION_FILE${NC}"
        echo
    } >&2
}

# Graceful error handling
trap 'log "ERROR" "An unexpected error occurred at line $LINENO. Installation failed."' ERR

# Run the main function
main "$@"
