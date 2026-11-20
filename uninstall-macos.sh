#!/bin/bash
# Deadlock API Ingest - macOS Uninstall Script
# Removes the application, its LaunchAgent and the status window app.

APP_NAME="deadlock-api-ingest"
LABEL="com.deadlock-api.deadlock-api-ingest"
INSTALL_DIR="$HOME/Library/Application Support/$APP_NAME"
EXECUTABLE="$INSTALL_DIR/$APP_NAME"
LAUNCH_AGENT="$HOME/Library/LaunchAgents/$LABEL.plist"
APP_BUNDLE="$HOME/Applications/Deadlock API Ingest.app"
BIN_SYMLINK="$HOME/.local/bin/$APP_NAME"

# Color codes
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m' # No Color

SILENT=false
if [[ "${1:-}" == "--silent" ]]; then
    SILENT=true
fi

# Run as `curl … | bash`, stdin is this script itself (BASH_SOURCE is then empty), so the
# prompts read the terminal instead.
PROMPT_INPUT=/dev/stdin
if [[ -z "${BASH_SOURCE[0]:-}" && -r /dev/tty ]]; then
    PROMPT_INPUT=/dev/tty
fi

if [[ "$SILENT" == false ]]; then
    echo ""
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  Deadlock API Ingest - Uninstaller${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""
    echo -e "${YELLOW}This will remove:${NC}"
    echo -e "  - LaunchAgent: $LAUNCH_AGENT"
    echo -e "  - App: $APP_BUNDLE"
    echo -e "  - Installation directory, with logs: $INSTALL_DIR"
    echo -e "  - Binary symlink: $BIN_SYMLINK"
    echo ""

    read -p "Continue with uninstallation? (Y/N): " -n 1 -r < "$PROMPT_INPUT"
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${YELLOW}Uninstallation cancelled.${NC}"
        exit 0
    fi
    echo ""
fi

echo -e "${YELLOW}Uninstalling Deadlock API Ingest...${NC}"
echo ""

echo -e "${CYAN}Removing the LaunchAgent...${NC}"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
rm -f "$LAUNCH_AGENT"

echo -e "${CYAN}Stopping running copies...${NC}"
# -f: macOS truncates process names to 16 characters, which "deadlock-api-ingest" exceeds.
pkill -f "$EXECUTABLE" 2>/dev/null || true

echo -e "${CYAN}Removing installation files...${NC}"
if [[ -d "$APP_BUNDLE" ]]; then
    echo -e "  ${GRAY}- Removing the app${NC}"
    rm -rf "$APP_BUNDLE"
fi
if [[ -L "$BIN_SYMLINK" ]] || [[ -f "$BIN_SYMLINK" ]]; then
    echo -e "  ${GRAY}- Removing binary symlink${NC}"
    rm -f "$BIN_SYMLINK"
fi
# A reinstall (--silent) keeps the logs, the salt history and the Steam request quotas.
if [[ "$SILENT" == true ]]; then
    rm -f "$EXECUTABLE"
elif [[ -d "$INSTALL_DIR" ]]; then
    echo -e "  ${GRAY}- Removing installation directory${NC}"
    rm -rf "$INSTALL_DIR"
fi
# The installer's log; a reinstall (--silent) is writing to it right now.
if [[ "$SILENT" == false ]]; then
    rm -rf "$HOME/Library/Logs/$APP_NAME"
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  Uninstallation Complete!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
