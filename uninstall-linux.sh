#!/bin/bash
# Deadlock API Ingest - Uninstall Script
# This script removes the application and all related components


APP_NAME="deadlock-api-ingest"
SERVICE_NAME="$APP_NAME"
INSTALL_DIR="$HOME/.local/share/$APP_NAME"
BIN_SYMLINK="$HOME/.local/bin/$APP_NAME"

# Color codes
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m' # No Color

# Parse arguments
SILENT=false
if [[ "$1" == "--silent" ]]; then
    SILENT=true
fi

if [[ "$SILENT" == false ]]; then
    echo ""
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  Deadlock API Ingest - Uninstaller${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""
    echo -e "${YELLOW}This will remove:${NC}"
    echo -e "  ${NC}- User-level systemd service${NC}"
    echo -e "  ${NC}- Old system-level services (if found)${NC}"
    echo -e "  ${NC}- Desktop shortcuts${NC}"
    echo -e "  ${NC}- Installation directory: $INSTALL_DIR${NC}"
    echo -e "  ${NC}- Binary symlink: $BIN_SYMLINK${NC}"
    echo ""
    
    read -p "Continue with uninstallation? (Y/N): " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo -e "${YELLOW}Uninstallation cancelled.${NC}"
        exit 0
    fi
    echo ""
fi

echo -e "${YELLOW}Uninstalling Deadlock API Ingest...${NC}"
echo ""

# Stop and disable user service
echo -e "${CYAN}Removing systemd user service...${NC}"
if systemctl --user is-active "$SERVICE_NAME" &>/dev/null; then
    echo -e "  ${GRAY}- Stopping service${NC}"
fi
systemctl --user stop "$SERVICE_NAME" 2>/dev/null || true
systemctl --user disable "$SERVICE_NAME" 2>/dev/null || true

# Remove systemd user unit files
echo -e "${CYAN}Removing service files...${NC}"
if [[ -f "$HOME/.config/systemd/user/$SERVICE_NAME.service" ]]; then
    echo -e "  ${GRAY}- Removing user service file${NC}"
    rm -f "$HOME/.config/systemd/user/$SERVICE_NAME.service"
fi

# Reload systemd user state
systemctl --user daemon-reload 2>/dev/null || true
systemctl --user reset-failed 2>/dev/null || true

# Clean up old system-level installations (if they exist)
echo -e "${CYAN}Checking for old system-level installations...${NC}"
if systemctl list-unit-files "$SERVICE_NAME.service" 2>/dev/null | grep -q "$SERVICE_NAME.service" || \
   [[ -f /etc/systemd/system/"$SERVICE_NAME".service ]] || \
   [[ -f /etc/systemd/system/"$SERVICE_NAME"-updater.service ]] || \
   [[ -f /etc/systemd/system/"$SERVICE_NAME"-updater.timer ]]; then
    echo ""
    echo -e "${YELLOW}Old system-level services detected. Attempting to remove...${NC}"
    echo -e "${YELLOW}You may be prompted for your password (sudo).${NC}"
    echo ""
    
    sudo systemctl stop "$SERVICE_NAME" 2>/dev/null || true
    sudo systemctl disable "$SERVICE_NAME" 2>/dev/null || true
    
    sudo systemctl stop "$SERVICE_NAME"-updater.timer 2>/dev/null || true
    sudo systemctl disable "$SERVICE_NAME"-updater.timer 2>/dev/null || true
    sudo systemctl stop "$SERVICE_NAME"-updater.service 2>/dev/null || true
    sudo systemctl disable "$SERVICE_NAME"-updater.service 2>/dev/null || true
    
    sudo rm -f /etc/systemd/system/"$SERVICE_NAME".service 2>/dev/null || true
    sudo rm -f /etc/systemd/system/"$SERVICE_NAME"-updater.service 2>/dev/null || true
    sudo rm -f /etc/systemd/system/"$SERVICE_NAME"-updater.timer 2>/dev/null || true
    
    sudo systemctl daemon-reload 2>/dev/null || true
    sudo systemctl reset-failed 2>/dev/null || true
    
    echo -e "  ${GRAY}- Old system-level services removed${NC}"
else
    echo -e "  ${GRAY}- No old system-level services found${NC}"
fi

# Remove desktop shortcuts
echo -e "${CYAN}Removing desktop shortcuts...${NC}"
if [[ -f "$HOME/.local/share/applications/$APP_NAME.desktop" ]] || \
   [[ -f "$HOME/.local/share/applications/$APP_NAME-once.desktop" ]]; then
    echo -e "  ${GRAY}- Removing desktop entries${NC}"
fi
rm -f "$HOME/.local/share/applications/$APP_NAME.desktop"
rm -f "$HOME/.local/share/applications/$APP_NAME-once.desktop"

# Remove installation and symlink
echo -e "${CYAN}Removing installation files...${NC}"
if [[ -L "$BIN_SYMLINK" ]] || [[ -f "$BIN_SYMLINK" ]]; then
    echo -e "  ${GRAY}- Removing binary symlink${NC}"
    rm -f "$BIN_SYMLINK"
fi
if [[ -d "$INSTALL_DIR" ]]; then
    echo -e "  ${GRAY}- Removing installation directory${NC}"
    rm -rf "$INSTALL_DIR"
fi

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  Uninstallation Complete!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""

if [[ "$SILENT" == false ]]; then
    echo "Press Enter to exit..."
    read -r
fi
