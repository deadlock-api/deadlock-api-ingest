@echo off
setlocal EnableDelayedExpansion

REM ============================================================================
REM Deadlock API Ingest - Windows Installation Wrapper
REM ============================================================================
REM This batch script automatically handles UAC elevation and downloads/executes
REM the PowerShell installation script from the GitHub repository.
REM ============================================================================

title Deadlock API Ingest Installer

REM --- Configuration ---
set "SCRIPT_NAME=%~nx0"
set "POWERSHELL_URL=https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/install-windows.ps1"
set "TEMP_LOG=%TEMP%\deadlock-api-ingest-installer.log"

REM --- Color codes for output ---
REM Note: We'll use simple echo for compatibility, but add visual separators

echo.
echo ============================================================================
echo                    Deadlock API Ingest Installer
echo ============================================================================
echo.

REM --- Check if running with administrator privileges ---
echo [INFO] Checking administrator privileges...

REM Use 'net session' command to test admin rights (works on all Windows versions)
net session >nul 2>&1
if %errorLevel% == 0 (
    echo [SUCCESS] Running with administrator privileges.
    goto :RunInstallation
) else (
    echo [WARN] Not running as administrator. Requesting elevation...
    goto :RequestElevation
)

:RequestElevation
echo.
echo [INFO] This installer requires administrator privileges to:
echo   - Install the application to Program Files
echo   - Create scheduled tasks for autostart and updates
echo   - Configure system-level services
echo.
echo [INFO] You will be prompted by Windows UAC to grant administrator access.
echo [INFO] Please click "Yes" when prompted to continue the installation.
echo.

REM Create a VBScript to show a user-friendly UAC prompt
set "UAC_SCRIPT=%TEMP%\uac_prompt.vbs"
echo Set UAC = CreateObject^("Shell.Application"^) > "%UAC_SCRIPT%"
echo UAC.ShellExecute "cmd.exe", "/c ""%~f0"" elevated", "", "runas", 1 >> "%UAC_SCRIPT%"

REM Execute the VBScript to trigger UAC
cscript //nologo "%UAC_SCRIPT%" >nul 2>&1

REM Clean up the temporary VBScript
if exist "%UAC_SCRIPT%" del "%UAC_SCRIPT%" >nul 2>&1

REM If we reach here, either UAC was cancelled or failed
echo.
echo [ERROR] Administrator privileges are required for installation.
echo [ERROR] Installation cancelled or UAC prompt was denied.
echo.
echo Press any key to exit...
pause >nul
exit /b 1

:RunInstallation
REM Clear any previous log
if exist "%TEMP_LOG%" del "%TEMP_LOG%" >nul 2>&1

echo.
echo [INFO] Starting PowerShell installation script download and execution...
echo [INFO] This may take a few moments depending on your internet connection.
echo.

REM Log the start of installation
echo [%DATE% %TIME%] Starting Deadlock API Ingest installation >> "%TEMP_LOG%"
echo [%DATE% %TIME%] PowerShell URL: %POWERSHELL_URL% >> "%TEMP_LOG%"

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

echo [INFO] PowerShell detected. Downloading and executing installation script...

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

REM Capture the exit code from PowerShell
set "PS_EXIT_CODE=%errorLevel%"

REM Log the result
echo [%DATE% %TIME%] PowerShell script exit code: %PS_EXIT_CODE% >> "%TEMP_LOG%"

if %PS_EXIT_CODE% equ 0 (
    echo.
    echo ============================================================================
    echo [SUCCESS] Installation completed successfully!
    echo ============================================================================
    echo.
    echo The Deadlock API Ingest application has been installed and configured.
    echo It will start automatically on system boot and check for updates daily.
    echo.
    echo For more information, check the installation log or refer to the
    echo application documentation.
    echo.
) else (
    echo.
    echo ============================================================================
    echo [ERROR] Installation failed!
    echo ============================================================================
    echo.
    echo The PowerShell installation script encountered an error.
    echo.
    echo Common solutions:
    echo   1. Check your internet connection
    echo   2. Ensure Windows is up to date
    echo   3. Try running the installer again
    echo   4. Check Windows Defender or antivirus settings
    echo.
    echo For detailed error information, check: %TEMP_LOG%
    echo.
    echo You can also try the manual installation method described in the
    echo project documentation at:
    echo https://github.com/deadlock-api/deadlock-api-ingest
    echo.
)

REM Show log file location for troubleshooting
echo Installation log: %TEMP_LOG%
echo.

REM Pause to show results before closing
echo Press any key to exit...
pause >nul

REM Exit with the same code as PowerShell
exit /b %PS_EXIT_CODE%
