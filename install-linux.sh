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
    log "INFO" "Running with sufficient privileges."
}

# Function to install required dependencies
install_dependencies() {
    log "INFO" "Checking and installing required dependencies (curl, wget, jq, libpcap)..."

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

    if [[ ${#pkgs_to_install[@]} -eq 0 ]]; then
        log "INFO" "All dependencies are already installed."
        return
    fi

    log "INFO" "Attempting to install: ${pkgs_to_install[*]}"
    if command -v apt-get >/dev/null 2>&1; then
        apt-get update -qq && apt-get install -y "${pkgs_to_install[@]}"
    elif command -v dnf >/dev/null 2>&1; then
        dnf install -y "${pkgs_to_install[@]}"
    elif command -v yum >/dev/null 2>&1; then
        yum install -y "${pkgs_to_install[@]}"
    elif command -v pacman >/dev/null 2>&1; then
        pacman -Sy --noconfirm "${pkgs_to_install[@]}"
    else
        log "WARN" "Could not detect package manager. Please install missing packages manually: ${pkgs_to_install[*]}"
    fi
}

# Function to get latest release info from GitHub API
get_latest_release() {
    log "INFO" "Fetching latest release from repository: $GITHUB_REPO"
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

    log "INFO" "Downloading from: $url"
    log "INFO" "Saving to: $output_path"
    mkdir -p "$(dirname "$output_path")"

    if ! wget --progress=bar:force --user-agent="Bash-Installer" -O "$output_path" "$url"; then
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

    log "SUCCESS" "File integrity verified."
}

# Function to manage the systemd service
manage_service() {
    local action="$1"

    case "$action" in
        "remove")
            if systemctl is-active --quiet "$SERVICE_NAME"; then
                log "INFO" "Stopping existing service..."
                systemctl stop "$SERVICE_NAME"
            fi
            if systemctl is-enabled --quiet "$SERVICE_NAME"; then
                log "INFO" "Disabling existing service..."
                systemctl disable "$SERVICE_NAME"
            fi
            if [[ -f "$SYSTEMD_SERVICE_FILE" ]]; then
                log "INFO" "Removing existing service file..."
                rm -f "$SYSTEMD_SERVICE_FILE"
            fi
            systemctl daemon-reload
            ;;
        "create")
            local executable_path="$2"

            log "INFO" "Creating systemd service file..."
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
            systemctl daemon-reload
            log "SUCCESS" "Systemd service created."
            ;;
        "start")
            log "INFO" "Enabling and starting the service..."
            systemctl enable "$SERVICE_NAME"
            systemctl start "$SERVICE_NAME"

            sleep 3

            if systemctl is-active --quiet "$SERVICE_NAME"; then
                log "SUCCESS" "Service started successfully."
            else
                log "ERROR" "Service failed to start. Please check the logs."
                log "INFO" "To check status: systemctl status $SERVICE_NAME"
                log "INFO" "To view logs: journalctl -u $SERVICE_NAME -n 50"
                exit 1
            fi
            ;;
    esac
}

# --- Main Installation Logic ---
main() {
    >"$LOG_FILE"

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

    manage_service "remove"

    log "INFO" "Setting up installation directory: $INSTALL_DIR"
    mkdir -p "$INSTALL_DIR"

    local temp_download_path="$INSTALL_DIR/${APP_NAME}-${version}"
    download_file "$download_url" "$temp_download_path" "$size"

    local final_executable_path="$INSTALL_DIR/$FINAL_EXECUTABLE_NAME"
    log "INFO" "Installing executable to $final_executable_path"
    mv "$temp_download_path" "$final_executable_path"
    chmod +x "$final_executable_path"

    local bin_symlink="$BIN_DIR/$FINAL_EXECUTABLE_NAME"
    log "INFO" "Creating symlink for easy access at $bin_symlink"
    ln -sf "$final_executable_path" "$bin_symlink"

    manage_service "create" "$final_executable_path"
    manage_service "start"

    log "SUCCESS" "🚀 Deadlock API Ingest ($version) has been installed successfully!"
    # The final messages should also be sent to stderr to not interfere with any potential scripting.
    {
        echo
        echo -e "${GREEN}Installation complete.${NC}"
        echo
        echo -e "You can manage the service with the following commands:"
        echo -e "  - Check status:  ${YELLOW}systemctl status $SERVICE_NAME${NC}"
        echo -e "  - View logs:     ${YELLOW}journalctl -u $SERVICE_NAME -f${NC}"
        echo -e "  - Stop service:  ${YELLOW}systemctl stop $SERVICE_NAME${NC}"
        echo -e "  - Start service: ${YELLOW}systemctl start $SERVICE_NAME${NC}"
        echo
    } >&2
}

# Graceful error handling
trap 'log "ERROR" "An unexpected error occurred at line $LINENO. Installation failed."' ERR

# Run the main function
main "$@"
