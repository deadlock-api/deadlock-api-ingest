#!/bin/bash

# Deadlock API Ingest - Linux Installation Script
# This script downloads and installs the deadlock-api-ingest application as a systemd user service

set -euo pipefail

# --- Configuration ---
APP_NAME="deadlock-api-ingest"
GITHUB_REPO="deadlock-api/deadlock-api-ingest"
ASSET_KEYWORD="ubuntu-latest" # Keyword to find in the release asset filename

# Installation paths (User-level, no root required)
INSTALL_DIR="$HOME/.local/share/$APP_NAME"
BIN_DIR="$HOME/.local/bin"
FINAL_EXECUTABLE_NAME=$APP_NAME

# Service and logging
SERVICE_NAME=$APP_NAME
LOG_FILE="/tmp/${APP_NAME}-install.log"
SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
SYSTEMD_SERVICE_FILE="$SYSTEMD_USER_DIR/${SERVICE_NAME}.service"



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

# Function to check for required dependencies
check_dependencies() {
    local missing_deps=()
    for pkg in curl wget jq; do
        command -v "$pkg" >/dev/null 2>&1 || missing_deps+=("$pkg")
    done

    if [[ ${#missing_deps[@]} -eq 0 ]]; then
        log "SUCCESS" "All dependencies are already installed."
        return 0
    fi

    log "WARN" "Missing dependencies: ${missing_deps[*]}"
    log "INFO" "Please install them using your package manager. For example:"

    if command -v apt-get >/dev/null 2>&1; then
        log "INFO" "  sudo apt-get install ${missing_deps[*]}"
    elif command -v dnf >/dev/null 2>&1; then
        log "INFO" "  sudo dnf install ${missing_deps[*]}"
    elif command -v yum >/dev/null 2>&1; then
        log "INFO" "  sudo yum install ${missing_deps[*]}"
    elif command -v pacman >/dev/null 2>&1; then
        log "INFO" "  sudo pacman -S ${missing_deps[*]}"
    elif command -v apk >/dev/null 2>&1; then
        log "INFO" "  sudo apk add ${missing_deps[*]}"
    elif command -v zypper >/dev/null 2>&1; then
        log "INFO" "  sudo zypper install ${missing_deps[*]}"
    else
        log "INFO" "  Use your system's package manager to install: ${missing_deps[*]}"
    fi

    log "ERROR" "Cannot proceed without required dependencies."
    exit 1
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

# Function to create desktop shortcut
create_desktop_shortcut() {
    local executable_path="$1"
    local arguments="${2:-}"
    local shortcut_name="${3:-Deadlock API Ingest}"
    local comment="${4:-Monitors Steam cache for Deadlock match replays}"

    log "INFO" "Creating desktop shortcut: $shortcut_name..."

    # Use the current user's local applications directory
    local desktop_dir="$HOME/.local/share/applications"

    # Create the directory if it doesn't exist
    if ! mkdir -p "$desktop_dir" 2>/dev/null; then
        log "WARN" "Could not create applications directory. Desktop shortcut will not be created."
        return 1
    fi

    # Create a safe filename from the shortcut name
    local safe_name
    safe_name=$(echo "$shortcut_name" | tr ' ' '-' | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9-]//g')
    local desktop_file="$desktop_dir/${safe_name}.desktop"

    # Build the Exec line with arguments if provided
    local exec_line="$executable_path"
    if [[ -n "$arguments" ]]; then
        exec_line="$executable_path $arguments"
    fi

    # Create the .desktop file
    cat > "$desktop_file" << EOF
[Desktop Entry]
Version=1.0
Type=Application
Name=$shortcut_name
Comment=$comment
Exec=$exec_line
Terminal=false
Keywords=deadlock;api;steam;
EOF

    chmod 644 "$desktop_file"

    # Try to update desktop database if available
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$desktop_dir" 2>> "$LOG_FILE" || true
    fi

    log "SUCCESS" "Desktop shortcut created at: $desktop_file"
    return 0
}

# Function to manage the systemd service
manage_service() {
    local action="$1"

    case "$action" in
        "remove")
            if systemctl --user is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
                execute_quietly systemctl --user stop "$SERVICE_NAME"
            fi
            if systemctl --user is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
                execute_quietly systemctl --user disable "$SERVICE_NAME"
            fi
            if [[ -f "$SYSTEMD_SERVICE_FILE" ]]; then
                rm -f "$SYSTEMD_SERVICE_FILE"
            fi
            systemctl --user daemon-reload 2>> "$LOG_FILE" || true
            ;;
        "create")
            local executable_path="$2"

            # Create systemd user directory if it doesn't exist
            mkdir -p "$SYSTEMD_USER_DIR"

            cat > "$SYSTEMD_SERVICE_FILE" << EOF
[Unit]
Description=Deadlock API Ingest - Monitors Steam cache for match replays
Documentation=https://github.com/deadlock-api/deadlock-api-ingest

[Service]
Type=simple
ExecStart=$executable_path
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=$SERVICE_NAME

# Security Hardening
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=default.target
EOF
            chmod 644 "$SYSTEMD_SERVICE_FILE"
            systemctl --user daemon-reload 2>> "$LOG_FILE"
            log "SUCCESS" "User service created."
            ;;
        "start")
            execute_quietly systemctl --user enable "$SERVICE_NAME"
            execute_quietly systemctl --user start "$SERVICE_NAME"

            sleep 3

            if systemctl --user is-active --quiet "$SERVICE_NAME"; then
                log "SUCCESS" "Application service started successfully."
            else
                log "ERROR" "Service failed to start. Please check the logs."
                log "INFO" "To check status: systemctl --user status $SERVICE_NAME"
                log "INFO" "To view logs: journalctl --user -u $SERVICE_NAME -n 50"
                exit 1
            fi
            ;;
    esac
}







# Function to prompt user for auto-start setup
prompt_for_autostart() {
    # Check if we're running in an interactive terminal
    if [[ ! -t 0 ]] || [[ ! -t 1 ]]; then
        log "INFO" "Non-interactive mode detected. Enabling auto-start by default."
        return 0
    fi

    log "INFO" "Auto-start will automatically start the service when the system boots."
    log "INFO" "This ensures the application is always running in the background."
    echo >&2

    local attempts=0
    local max_attempts=2

    while [[ $attempts -lt $max_attempts ]]; do
        echo -n "Would you like to enable auto-start on system boot? (y/n): " >&2

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
            log "INFO" "No response received within 10 seconds. Enabling auto-start by default."
            return 0
        fi
    done

    log "INFO" "Maximum attempts reached. Enabling auto-start by default."
    return 0
}

# --- Main Installation Logic ---
main() {
    log "INFO" "Starting Deadlock API Ingest installation..."
    log "INFO" "Log file is available at: $LOG_FILE"

    check_dependencies

    local release_info
    release_info=$(get_latest_release)
    local version download_url size
    IFS='|' read -r version download_url size <<< "$release_info"

    if [[ -z "$version" || -z "$download_url" ]]; then
        log "ERROR" "Failed to parse release information. Cannot continue."
        exit 1
    fi

    # Try to run uninstall script if it exists (clean uninstall before fresh install)
    local existing_uninstall_script="$INSTALL_DIR/uninstall-linux.sh"
    if [[ -f "$existing_uninstall_script" ]]; then
        log "INFO" "Found existing installation. Running uninstall script..."
        if "$existing_uninstall_script" --silent 2>/dev/null; then
            log "SUCCESS" "Previous installation uninstalled successfully."
        else
            log "WARN" "Uninstall script failed, continuing with manual cleanup."
        fi
    fi

    # Remove existing service (in case uninstall script didn't exist or failed)
    manage_service "remove"

    # Clean up any remaining processes
    killall "$APP_NAME" 2>/dev/null || true

    # Ensure installation directory exists
    mkdir -p "$INSTALL_DIR"

    local temp_download_path="$INSTALL_DIR/${APP_NAME}-${version}"
    download_file "$download_url" "$temp_download_path" "$size"

    local final_executable_path="$INSTALL_DIR/$FINAL_EXECUTABLE_NAME"
    mv "$temp_download_path" "$final_executable_path"
    chmod +x "$final_executable_path"

    # Create bin directory if it doesn't exist
    mkdir -p "$BIN_DIR"

    local bin_symlink="$BIN_DIR/$FINAL_EXECUTABLE_NAME"
    ln -sf "$final_executable_path" "$bin_symlink"
    log "SUCCESS" "Application installed successfully."

    # Download uninstall script
    log "INFO" "Downloading uninstall script..."
    local uninstall_script_path="$INSTALL_DIR/uninstall-linux.sh"
    local uninstall_script_url="https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/uninstall-linux.sh"

    if curl -fsSL "$uninstall_script_url" -o "$uninstall_script_path"; then
        chmod +x "$uninstall_script_path"
        log "SUCCESS" "Uninstall script downloaded to: $uninstall_script_path"
    else
        log "WARN" "Failed to download uninstall script, but continuing installation."
        log "INFO" "You can manually download it from: $uninstall_script_url"
    fi

    # Create the main service (but don't enable/start it yet)
    manage_service "create" "$final_executable_path"

    # Prompt user for auto-start setup
    if prompt_for_autostart; then
        manage_service "start"
        log "SUCCESS" "Auto-start enabled. The service will start automatically on user login."
    else
        log "INFO" "Auto-start disabled. You can start the service manually with: systemctl --user start $SERVICE_NAME"
        log "INFO" "To enable auto-start later, run: systemctl --user enable $SERVICE_NAME"

        # Offer to create a desktop shortcut instead
        echo >&2
        log "INFO" "Would you like to create a desktop shortcut instead?"

        local create_shortcut=false
        local attempts=0
        local max_attempts=2

        # Check if we're running in an interactive terminal
        if [[ ! -t 0 ]] || [[ ! -t 1 ]]; then
            log "INFO" "Non-interactive mode detected. Creating desktop shortcut by default."
            create_shortcut=true
        else
            while [[ $attempts -lt $max_attempts ]]; do
                echo -n "Create desktop shortcut? (y/n): " >&2

                local response
                if read -t 10 -r response; then
                    case "${response,,}" in
                        y|yes)
                            create_shortcut=true
                            break
                            ;;
                        n|no)
                            create_shortcut=false
                            break
                            ;;
                        *)
                            attempts=$((attempts + 1))
                            if [[ $attempts -lt $max_attempts ]]; then
                                echo "Invalid response. Please enter 'y' for yes or 'n' for no." >&2
                            fi
                            ;;
                    esac
                else
                    log "INFO" "No response received within 10 seconds. Creating desktop shortcut by default."
                    create_shortcut=true
                    break
                fi
            done

            if [[ $attempts -ge $max_attempts ]]; then
                log "INFO" "Maximum attempts reached. Creating desktop shortcut by default."
                create_shortcut=true
            fi
        fi

        if [[ "$create_shortcut" == true ]]; then
            # Create main shortcut
            local main_created=false
            local once_created=false

            if create_desktop_shortcut "$final_executable_path" "" "Deadlock API Ingest" "Monitors Steam cache for Deadlock match replays"; then
                main_created=true
            fi

            # Create "once" shortcut for initial cache ingest only
            if create_desktop_shortcut "$final_executable_path" "--once" "Deadlock API Ingest (Once)" "Scan existing Steam cache once and exit"; then
                once_created=true
            fi

            if [[ "$main_created" == true && "$once_created" == true ]]; then
                log "INFO" "Desktop shortcuts created:"
                log "INFO" "  - Deadlock API Ingest: Start the application with monitoring"
                log "INFO" "  - Deadlock API Ingest (Once): Run once to ingest existing cache files only"
                log "INFO" "You can also start manually: $final_executable_path"
            elif [[ "$main_created" == true ]]; then
                log "INFO" "Main desktop shortcut created. You can also run: $final_executable_path"
            else
                log "INFO" "You can start the application by running: $final_executable_path"
            fi
        else
            log "INFO" "You can manually start the application by running: $final_executable_path"
            log "INFO" "To run once (ingest existing cache only): $final_executable_path --once"
        fi
    fi

    log "SUCCESS" "🚀 Deadlock API Ingest has been installed successfully!"

    # The final messages should also be sent to stderr to not interfere with any potential scripting.
    {
        echo
        echo -e "${GREEN}Installation complete.${NC}"
        echo

        if systemctl --user is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
            echo -e "${GREEN}[+] Auto-start is enabled${NC} - The service will start automatically on user login."
        else
            echo -e "${YELLOW}[-] Auto-start is disabled${NC} - The service will not start automatically on user login."
            echo -e "  To enable auto-start: ${YELLOW}systemctl --user enable $SERVICE_NAME${NC}"
        fi
        echo

        echo -e "You can manage the main service with the following commands:"
        echo -e "  - Check status:  ${YELLOW}systemctl --user status $SERVICE_NAME${NC}"
        echo -e "  - View logs:     ${YELLOW}journalctl --user -u $SERVICE_NAME -f${NC}"
        echo -e "  - Stop service:  ${YELLOW}systemctl --user stop $SERVICE_NAME${NC}"
        echo -e "  - Start service: ${YELLOW}systemctl --user start $SERVICE_NAME${NC}"
        if ! systemctl --user is-enabled --quiet "$SERVICE_NAME" 2>/dev/null; then
            echo -e "  - Enable auto-start: ${YELLOW}systemctl --user enable $SERVICE_NAME${NC}"
        fi
        echo
        echo -e "To uninstall, run: ${YELLOW}$INSTALL_DIR/uninstall-linux.sh${NC}"
        echo
    } >&2
}

# Graceful error handling
trap 'log "ERROR" "An unexpected error occurred at line $LINENO. Installation failed."' ERR

# Run the main function
main "$@"
