@echo off
setlocal EnableDelayedExpansion

title Deadlock API Ingest Installer

REM --- Configuration ---
set "POWERSHELL_URL=https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1"

:RunInstallation
REM Check if PowerShell is available
powershell.exe -Command "Get-Host" >nul 2>&1
if %errorLevel% neq 0 (
    echo [ERROR] PowerShell is not available or not working properly.
    echo [ERROR] This installer requires PowerShell to function.
    echo.
    echo Press any key to exit...
    pause >nul
    exit /b 1
)

REM Execute the PowerShell installation script with proper error handling
powershell.exe -ExecutionPolicy Bypass -Command ^
    "try { " ^
    "    Write-Host '[INFO] Downloading installation script from GitHub...' -ForegroundColor Cyan; " ^
    "    $script = Invoke-RestMethod -Uri '%POWERSHELL_URL%' -UseBasicParsing; " ^
    "    Write-Host '[INFO] Executing installation script...' -ForegroundColor Cyan; " ^
    "    Invoke-Expression $script; " ^
    "    Write-Host '[SUCCESS] Installation script completed successfully.' -ForegroundColor Green; " ^
    "    exit 0; " ^
    "} catch { " ^
    "    Write-Host '[ERROR] Installation failed: ' -ForegroundColor Red -NoNewline; " ^
    "    Write-Host $_.Exception.Message -ForegroundColor Red; " ^
    "    Write-Host '[ERROR] Please check your internet connection and try again.' -ForegroundColor Red; " ^
    "    exit 1; " ^
    "}"

REM Pause to show results before closing
echo Press any key to exit...
pause >nul
