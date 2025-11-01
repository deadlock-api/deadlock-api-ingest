#!/bin/bash

# Deadlock API Ingest - Update Checker Script
# This script checks for new releases and updates the application automatically

set -euo pipefail

# --- Configuration ---
APP_NAME="deadlock-api-ingest"
GITHUB_REPO="deadlock-api/deadlock-api-ingest"
ASSET_KEYWORD="ubuntu-latest"
INSTALL_DIR="/opt/$APP_NAME"
SERVICE_NAME=$APP_NAME
VERSION_FILE="$INSTALL_DIR/version.txt"
CONFIG_FILE="$INSTALL_DIR/config.conf"
BACKUP_DIR="$INSTALL_DIR/backup"
UPDATE_LOG_FILE="/var/log/${APP_NAME}-updater.log"

# --- Helper Functions ---

# Logging function for updater
update_log() {
    local level="$1"
    local message="$2"
    local timestamp
    timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    local log_message="[$timestamp] [$level] $message"
    
    # Ensure log directory exists
    mkdir -p "$(dirname "$UPDATE_LOG_FILE")"
    
    echo "$log_message" | tee -a "$UPDATE_LOG_FILE"
    
    # Also log to system journal
    case "$level" in
        "ERROR") logger -p daemon.err -t "${APP_NAME}-updater" "$message" ;;
        "WARN")  logger -p daemon.warning -t "${APP_NAME}-updater" "$message" ;;
        *)       logger -p daemon.info -t "${APP_NAME}-updater" "$message" ;;
    esac
}

# Check if automatic updates are enabled
check_update_enabled() {
    if [[ -f "$CONFIG_FILE" ]]; then
        local auto_update
        auto_update=$(grep -E "^AUTO_UPDATE=" "$CONFIG_FILE" 2>/dev/null | cut -d'=' -f2 | tr -d '"' || echo "true")
        if [[ "$auto_update" == "false" ]]; then
            update_log "INFO" "Automatic updates are disabled. Exiting."
            exit 0
        fi
    fi
    update_log "INFO" "Automatic updates are enabled."
}

# Get current installed version
get_current_version() {
    if [[ -f "$VERSION_FILE" ]]; then
        cat "$VERSION_FILE"
    else
        echo "unknown"
    fi
}

# Get latest release version from GitHub
get_latest_version() {
    local api_url="https://api.github.com/repos/$GITHUB_REPO/releases/latest"
    local response
    
    response=$(curl -s -f --user-agent "Update-Checker" "$api_url") || {
        update_log "ERROR" "Failed to fetch release information from GitHub API."
        return 1
    }
    
    echo "$response" | jq -r '.tag_name'
}

# Compare versions (returns 0 if update needed, 1 if not)
version_compare() {
    local current="$1"
    local latest="$2"
    
    if [[ "$current" == "unknown" ]] || [[ "$current" != "$latest" ]]; then
        return 0  # Update needed
    else
        return 1  # No update needed
    fi
}

# Create backup of current executable
create_backup() {
    local executable_path="$INSTALL_DIR/$APP_NAME"
    local backup_path
    backup_path="$BACKUP_DIR/$(basename "$executable_path").$(date +%Y%m%d_%H%M%S)"
    
    mkdir -p "$BACKUP_DIR"
    
    if [[ -f "$executable_path" ]]; then
        cp "$executable_path" "$backup_path"
        update_log "INFO" "Created backup: $backup_path"
        echo "$backup_path"
    else
        update_log "WARN" "No existing executable found to backup."
        echo ""
    fi
}

# Rollback to previous version
rollback() {
    local backup_path="$1"
    local executable_path="$INSTALL_DIR/$APP_NAME"
    
    if [[ -n "$backup_path" ]] && [[ -f "$backup_path" ]]; then
        update_log "INFO" "Rolling back to previous version..."
        cp "$backup_path" "$executable_path"
        chmod +x "$executable_path"
        update_log "SUCCESS" "Rollback completed."
        return 0
    else
        update_log "ERROR" "No backup available for rollback."
        return 1
    fi
}

# Download and install new version
download_and_install() {
    local version="$1"
    local api_url="https://api.github.com/repos/$GITHUB_REPO/releases/latest"
    local response
    
    response=$(curl -s -f --user-agent "Update-Checker" "$api_url") || {
        update_log "ERROR" "Failed to fetch release information for download."
        return 1
    }
    
    local asset_info
    asset_info=$(echo "$response" | jq --arg keyword "$ASSET_KEYWORD" '([.assets[] | select(.name | contains($keyword))])[0]')
    
    if [[ -z "$asset_info" || "$asset_info" == "null" ]]; then
        update_log "ERROR" "Could not find a release asset containing the keyword: '$ASSET_KEYWORD'"
        return 1
    fi
    
    local download_url size
    download_url=$(echo "$asset_info" | jq -r '.browser_download_url')
    size=$(echo "$asset_info" | jq -r '.size')
    
    local temp_download_path="/tmp/${APP_NAME}-update-${version}"
    local executable_path="$INSTALL_DIR/$APP_NAME"
    
    update_log "INFO" "Downloading new version from: $download_url"
    
    if ! wget --progress=dot:mega --user-agent="Update-Checker" -O "$temp_download_path" "$download_url" 2>&1 | grep -o '[0-9]*%' | tail -1; then
        update_log "ERROR" "Download failed."
        rm -f "$temp_download_path"
        return 1
    fi

    # Verify file size
    local actual_size
    actual_size=$(stat -c%s "$temp_download_path")
    if [[ "$actual_size" != "$size" ]]; then
        update_log "ERROR" "File size mismatch! Expected: $size bytes, Got: $actual_size bytes."
        rm -f "$temp_download_path"
        return 1
    fi
    
    # Install new version
    mv "$temp_download_path" "$executable_path"
    chmod +x "$executable_path"
    
    # Update version file
    echo "$version" > "$VERSION_FILE"
    
    update_log "SUCCESS" "New version installed successfully."
    return 0
}

# Test if new version starts correctly
test_new_version() {
    local timeout=30
    local service_name="$SERVICE_NAME"
    
    update_log "INFO" "Testing new version by restarting service..."
    
    # Restart the service
    systemctl restart "$service_name" || {
        update_log "ERROR" "Failed to restart service."
        return 1
    }
    
    # Wait and check if service is running
    sleep 5
    
    local count=0
    while [[ $count -lt $timeout ]]; do
        if systemctl is-active --quiet "$service_name"; then
            update_log "SUCCESS" "New version is running successfully."
            return 0
        fi
        sleep 1
        ((count++))
    done
    
    update_log "ERROR" "New version failed to start within $timeout seconds."
    return 1
}

# --- Main Update Logic ---
main() {
    update_log "INFO" "Starting automatic update check..."
    
    # Ensure log directory exists
    mkdir -p "$(dirname "$UPDATE_LOG_FILE")"
    
    # Check if updates are enabled
    check_update_enabled

    # Get current and latest versions
    local current_version
    current_version=$(get_current_version)
    local latest_version
    latest_version=$(get_latest_version)
    
    if [[ -z "$latest_version" ]] || [[ "$latest_version" == "null" ]]; then
        update_log "ERROR" "Failed to get latest version information."
        exit 1
    fi
    
    update_log "INFO" "Current version: $current_version"
    update_log "INFO" "Latest version: $latest_version"
    
    # Check if update is needed
    if ! version_compare "$current_version" "$latest_version"; then
        update_log "INFO" "No update needed. Current version is up to date."
        exit 0
    fi
    
    update_log "INFO" "Update available. Starting update process..."

    # Create backup
    local backup_path
    backup_path=$(create_backup)
    
    # Stop the main service
    update_log "INFO" "Stopping main service for update..."
    systemctl stop "$SERVICE_NAME" || {
        update_log "WARN" "Failed to stop service, continuing anyway..."
    }
    
    # Download and install new version
    if download_and_install "$latest_version"; then
        # Test new version
        if test_new_version; then
            update_log "SUCCESS" "Update completed successfully to version $latest_version"
            
            # Clean up old backups (keep last 5)
            find "$BACKUP_DIR" -name "${APP_NAME}.*" -type f | sort -r | tail -n +6 | xargs rm -f 2>/dev/null || true
        else
            # Rollback on failure
            update_log "ERROR" "New version failed to start. Attempting rollback..."
            if rollback "$backup_path"; then
                systemctl start "$SERVICE_NAME"
                update_log "ERROR" "Update failed, but rollback was successful."
                exit 1
            else
                update_log "ERROR" "Update failed and rollback also failed. Manual intervention required."
                exit 1
            fi
        fi
    else
        # Restart old version on download failure
        update_log "ERROR" "Failed to download new version. Restarting existing service..."
        systemctl start "$SERVICE_NAME"
        exit 1
    fi
}

# Run main function
main "$@"
