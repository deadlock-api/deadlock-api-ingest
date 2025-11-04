# Deadlock API Ingest - Windows Installation Script
# This script downloads and installs the application to run automatically on user login via Task Scheduler.

# Suppress PSScriptAnalyzer warnings for Write-Host in this interactive installation script
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '', Justification='Interactive installation script requires colored console output for user experience')]
param()

# --- Configuration ---
$AppName = "deadlock-api-ingest"
$GithubRepo = "deadlock-api/deadlock-api-ingest"
$AssetKeyword = "windows-latest.exe"

# Installation Paths
$InstallDir = "$env:LOCALAPPDATA\$AppName"
$FinalExecutableName = "$AppName.exe"
$LogFile = "$env:TEMP\${AppName}-install.log"



# --- Script Setup ---
$ErrorActionPreference = 'Stop'

# Global error tracking
$script:HasErrors = $false
$script:ErrorDetails = @()

$script:SkipPressAnyKey = $false
if ($env:CI -eq 'true' -or $env:GITHUB_ACTIONS -eq 'true' -or $env:TF_BUILD -eq 'True') {
    $script:SkipPressAnyKey = $true
}

# Function to handle errors and keep window open
function Invoke-FatalError {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ErrorMessage,
        [string]$DetailedError = "",
        [int]$ExitCode = 1
    )

    $script:HasErrors = $true
    $script:ErrorDetails += $ErrorMessage

    Write-Host ""
    Write-Host "========================================" -ForegroundColor Red
    Write-Host "           INSTALLATION FAILED          " -ForegroundColor Red
    Write-Host "========================================" -ForegroundColor Red
    Write-Host ""

    Write-InstallLog -Level 'ERROR' $ErrorMessage

    if ($DetailedError) {
        Write-Host "Error Details:" -ForegroundColor Yellow
        Write-Host $DetailedError -ForegroundColor White
        Write-Host ""
        Add-Content -Path $LogFile -Value "Detailed Error: $DetailedError"
    }

    # Show log file location
    Write-Host "Complete installation log available at:" -ForegroundColor Cyan
    Write-Host $LogFile -ForegroundColor White
    Write-Host ""

    # Wait for user acknowledgment (skip in CI environments)
    if (-not $script:SkipPressAnyKey) {
        Write-Host "Press any key to close this window..." -ForegroundColor Green
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
    }

    exit $ExitCode
}

# Function to show final status and wait for user input
function Show-FinalStatus {
    if ($script:HasErrors) {
        Write-Host ""
        Write-Host "Installation completed with errors. Please review the messages above." -ForegroundColor Yellow
        Write-Host "Log file: $LogFile" -ForegroundColor Cyan
        Write-Host ""
    } else {
        Write-Host ""
        Write-Host "Installation completed successfully!" -ForegroundColor Green
        Write-Host ""
    }

    # Wait for user acknowledgment (skip in CI environments)
    if (-not $script:SkipPressAnyKey) {
        Write-Host "Press any key to close this window..." -ForegroundColor Green
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
    }
}

# --- Helper Functions ---

# Function to write to log and console with color
function Write-InstallLog {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('INFO', 'WARN', 'ERROR', 'SUCCESS')]
        [string]$Level,
        [Parameter(Mandatory = $true)]
        [string]$Message
    )
    $ColorMap = @{ 'INFO' = 'Cyan'; 'WARN' = 'Yellow'; 'ERROR' = 'Red'; 'SUCCESS' = 'Green' }
    $Color = $ColorMap[$Level]
    $Timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $LogMessage = "[$Timestamp] [$Level] $Message"
    Write-Host $LogMessage -ForegroundColor $Color
    Add-Content -Path $LogFile -Value $LogMessage
}

# Function to execute commands quietly while logging details
function Invoke-Quietly {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Description,
        [Parameter(Mandatory = $true)]
        [scriptblock]$ScriptBlock,
        [switch]$ContinueOnError
    )

    Write-InstallLog -Level 'INFO' $Description

    try {
        # Capture output and redirect to log file
        $output = & $ScriptBlock 2>&1
        Add-Content -Path $LogFile -Value $output
        return $true
    }
    catch {
        $errorMsg = "Command failed: $($_.Exception.Message)"
        Write-InstallLog -Level 'ERROR' $errorMsg
        Add-Content -Path $LogFile -Value "Error: $($_.Exception.Message)"
        Add-Content -Path $LogFile -Value "Stack Trace: $($_.ScriptStackTrace)"

        if (-not $ContinueOnError) {
            Invoke-FatalError -ErrorMessage $errorMsg -DetailedError $_.Exception.Message
        }
        return $false
    }
}

# Function to check for Administrator privileges
function Test-IsAdmin {
    try {
        $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent()
        $isAdmin = (New-Object Security.Principal.WindowsPrincipal $currentUser).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if (-not $isAdmin) {
            Invoke-FatalError -ErrorMessage "Administrator privileges are required to create scheduled tasks." -DetailedError "Please run this script as an Administrator."
        }
    }
    catch {
        Invoke-FatalError -ErrorMessage "Failed to check Administrator privileges." -DetailedError $_.Exception.Message
    }
}

# Function to get the latest release from GitHub
function Get-LatestRelease {
    Write-InstallLog -Level 'INFO' "Fetching latest release from repository: $GithubRepo"
    $ApiUrl = "https://api.github.com/repos/$GithubRepo/releases/latest"
    try {
        $releaseInfo = Invoke-RestMethod -Uri $ApiUrl -UseBasicParsing
    }
    catch {
        $errorMsg = "Failed to fetch release information from GitHub API."
        $detailedError = "URL: $ApiUrl`nError: $($_.Exception.Message)`n`nPossible causes:`n- No internet connection`n- GitHub API rate limit exceeded`n- Repository not found or private`n- Firewall blocking the request"
        Invoke-FatalError -ErrorMessage $errorMsg -DetailedError $detailedError
    }

    try {
        $asset = $releaseInfo.assets | Where-Object { $_.name -like "*$AssetKeyword*" } | Select-Object -First 1
        if (-not $asset) {
            $availableAssets = if ($releaseInfo.assets) { $releaseInfo.assets.name -join ', ' } else { "None" }
            $errorMsg = "Could not find a release asset containing the keyword: '$AssetKeyword'"
            $detailedError = "Available assets are: $availableAssets`n`nThe release may not have been built for Windows, or the asset naming convention has changed."
            Invoke-FatalError -ErrorMessage $errorMsg -DetailedError $detailedError
        }
        return [PSCustomObject]@{
            Version      = $releaseInfo.tag_name
            DownloadUrl  = $asset.browser_download_url
            Size         = $asset.size
        }
    }
    catch {
        Invoke-FatalError -ErrorMessage "Failed to process release information." -DetailedError $_.Exception.Message
    }
}

# Function to create desktop shortcut
function New-DesktopShortcut {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ExecutablePath,
        [Parameter(Mandatory = $false)]
        [string]$Arguments = "",
        [Parameter(Mandatory = $false)]
        [string]$ShortcutName = $AppName,
        [Parameter(Mandatory = $false)]
        [string]$Description = "Deadlock API Ingest - Monitors Steam cache for Deadlock match replays"
    )

    Write-InstallLog -Level 'INFO' "Creating desktop shortcut: $ShortcutName..."

    try {
        $DesktopPath = [Environment]::GetFolderPath('Desktop')
        if (-not (Test-Path $DesktopPath)) {
            throw "Desktop folder not found at: $DesktopPath"
        }

        $WshShell = New-Object -ComObject WScript.Shell
        $ShortcutPath = Join-Path -Path $DesktopPath -ChildPath "$ShortcutName.lnk"

        $Shortcut = $WshShell.CreateShortcut($ShortcutPath)
        $Shortcut.TargetPath = $ExecutablePath
        $Shortcut.Arguments = $Arguments
        $Shortcut.WorkingDirectory = $InstallDir
        $Shortcut.Description = $Description
        $Shortcut.IconLocation = $ExecutablePath
        $Shortcut.Save()

        Write-InstallLog -Level 'SUCCESS' "Desktop shortcut created at: $ShortcutPath"
        return $true
    }
    catch {
        Write-InstallLog -Level 'ERROR' "Failed to create desktop shortcut: $ShortcutName"
        Write-InstallLog -Level 'WARN' "Continuing installation without desktop shortcut."
        Add-Content -Path $LogFile -Value "Error details: $($_.Exception.Message)"
        $script:HasErrors = $true
        $script:ErrorDetails += "Desktop shortcut creation failed (non-critical)"
        return $false
    }
}

# Function to manage the Scheduled Task for autostart
function Set-StartupTask {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('Remove', 'Create')]
        [string]$Action,
        [string]$ExecutablePath
    )

    switch ($Action) {
        'Remove' {
            Invoke-Quietly "Removing existing scheduled task..." {
                Unregister-ScheduledTask -TaskName $AppName -Confirm:$false -ErrorAction SilentlyContinue
            } | Out-Null
        }
        'Create' {
            Write-InstallLog -Level 'INFO' "Creating startup task..."

            try {
                # Create a VBS wrapper script to run the executable hidden
                $vbsWrapperPath = Join-Path -Path $InstallDir -ChildPath "run-hidden.vbs"
                $vbsContent = @"
Set WshShell = CreateObject("WScript.Shell")
WshShell.Run """$ExecutablePath""", 0, False
"@
                Set-Content -Path $vbsWrapperPath -Value $vbsContent -Force

                # Define the action (run the VBS wrapper with wscript.exe to hide the window)
                $taskAction = New-ScheduledTaskAction -Execute "wscript.exe" -Argument "`"$vbsWrapperPath`"" -WorkingDirectory $InstallDir

                # Define the trigger (when to run it - at user logon)
                $taskTrigger = New-ScheduledTaskTrigger -AtLogOn

                # Define the user and permissions (run as current user with standard privileges)
                $taskPrincipal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType S4U -RunLevel Limited

                # Define settings (allow it to run indefinitely, prevent multiple instances)
                $taskSettings = New-ScheduledTaskSettingsSet `
                    -AllowStartIfOnBatteries `
                    -DontStopIfGoingOnBatteries `
                    -ExecutionTimeLimit 0 `
                    -StartWhenAvailable `
                    -MultipleInstances IgnoreNew `
                    -Hidden

                # Register the task with the system
                Register-ScheduledTask -TaskName $AppName -Action $taskAction -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Description "Monitors Steam cache for Deadlock match replays and submits metadata to the Deadlock API." | Out-Null

                Write-InstallLog -Level 'SUCCESS' "Startup task created successfully."
            }
            catch {
                Invoke-FatalError -ErrorMessage "Failed to create startup task." -DetailedError "Error: $($_.Exception.Message)`n`nThis could be due to:`n- Task Scheduler service not running`n- Conflicting task name`n- System policy restrictions"
            }
        }
    }
}




# --- Main Installation Logic ---

# Wrap the entire installation in a try-catch block
try {
    Clear-Content -Path $LogFile -ErrorAction SilentlyContinue
    Write-InstallLog -Level 'INFO' "Starting Deadlock API Ingest installation..."
    Write-InstallLog -Level 'INFO' "Log file is available at: $LogFile"

    $release = Get-LatestRelease

    # Try to run uninstall script if it exists (clean uninstall before fresh install)
    Write-Host "Removing scheduled tasks..." -ForegroundColor Cyan
    $tasksToRemove = @(
        "$AppName",
        "$AppName-Watchdog",
        "$AppName-updater"
    )

    foreach ($taskName in $tasksToRemove) {
        $task = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        if ($task) {
            Write-Host "  - Removing task: $taskName" -ForegroundColor Gray
            Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
        }
    }

    # Stop any running process
    Write-Host "Stopping running processes..." -ForegroundColor Cyan
    $processes = Get-Process -Name $AppName -ErrorAction SilentlyContinue
    if ($processes) {
        Write-Host "  - Stopping $($processes.Count) process(es)" -ForegroundColor Gray
        Stop-Process -Name $AppName -Force -ErrorAction SilentlyContinue
    }

    Write-InstallLog -Level 'INFO' "Preparing installation environment..."

    try {
        Stop-Process -Name $AppName -Force -ErrorAction SilentlyContinue
    }
    catch {
        # This is expected if the process isn't running
        Write-InstallLog -Level 'INFO' "No existing process to stop."
    }

    try {
        New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null
    }
    catch {
        Invoke-FatalError -ErrorMessage "Failed to create installation directory." -DetailedError "Error: $($_.Exception.Message)`n`nInstall Directory: $InstallDir"
    }

    $downloadPath = Join-Path -Path $InstallDir -ChildPath $FinalExecutableName
    Write-InstallLog -Level 'INFO' "Downloading application binary..."
    # Log detailed URL to file only
    Add-Content -Path $LogFile -Value "Downloading from: $($release.DownloadUrl)"

    try {
        Invoke-WebRequest -Uri $release.DownloadUrl -OutFile $downloadPath -UseBasicParsing
    }
    catch {
        Invoke-FatalError -ErrorMessage "Failed to download application binary." -DetailedError "URL: $($release.DownloadUrl)`nDestination: $downloadPath`nError: $($_.Exception.Message)`n`nPossible causes:`n- No internet connection`n- Insufficient disk space`n- Permission denied to write to installation directory"
    }

    # Verify file size
    try {
        $actualSize = (Get-Item -Path $downloadPath).Length
        if ($actualSize -ne $release.Size) {
            Invoke-FatalError -ErrorMessage "File size mismatch! Download may be corrupted." -DetailedError "Expected: $($release.Size) bytes`nActual: $actualSize bytes`n`nPlease try running the installation again."
        }
        Write-InstallLog -Level 'SUCCESS' "Download complete and verified."
    }
    catch {
        Invoke-FatalError -ErrorMessage "Failed to verify downloaded file." -DetailedError $_.Exception.Message
    }

    try {
        Unblock-File -Path $downloadPath
    }
    catch {
        Write-InstallLog -Level 'WARN' "Failed to unblock downloaded file, but continuing installation."
        $script:HasErrors = $true
        $script:ErrorDetails += "File unblock failed (non-critical)"
    }

    # Download uninstall script
    Write-InstallLog -Level 'INFO' "Downloading uninstall script..."
    $uninstallScriptPath = Join-Path -Path $InstallDir -ChildPath "uninstall-windows.ps1"
    $uninstallScriptUrl = "https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/uninstall-windows.ps1"

    try {
        Invoke-WebRequest -Uri $uninstallScriptUrl -OutFile $uninstallScriptPath -UseBasicParsing
        Write-InstallLog -Level 'SUCCESS' "Uninstall script downloaded to: $uninstallScriptPath"
    }
    catch {
        Write-InstallLog -Level 'WARN' "Failed to download uninstall script, but continuing installation."
        Write-InstallLog -Level 'INFO' "You can manually download it from: $uninstallScriptUrl"
        $script:HasErrors = $true
        $script:ErrorDetails += "Uninstall script download failed (non-critical)"
    }

    Write-InstallLog -Level 'INFO' "Installing application..."

    # Ask user if they want auto-start
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "      AUTO-START SETUP (OPTIONAL)      " -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Auto-start will automatically start the application when the system boots." -ForegroundColor White
    Write-Host "This ensures the application is always running in the background." -ForegroundColor White
    Write-Host "CAREFUL: This requires Administrator privileges to create a scheduled task." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Enable auto-start on system boot? (Y/N): " -ForegroundColor Yellow -NoNewline

    $enableAutoStart = $false
    $autoStartAttempts = 0
    $maxAutoStartAttempts = 2

    # Check if running in interactive mode
    $isInteractive = [Environment]::UserInteractive -and -not [Console]::IsInputRedirected

    if (-not $isInteractive) {
        Write-Host "Y (default in non-interactive mode)" -ForegroundColor Cyan
        Write-InstallLog -Level 'INFO' "Non-interactive mode detected. Enabling auto-start by default."
        $enableAutoStart = $true
    } else {
        # Try to read with timeout
        $timeoutSeconds = 10
        $startTime = Get-Date
        $keyPressed = $false

        while ($autoStartAttempts -lt $maxAutoStartAttempts -and -not $keyPressed) {
            if ([Console]::KeyAvailable) {
                $response = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
                $key = $response.Character.ToString().ToUpper()

                if ($key -eq "Y" -or $key -eq "N") {
                    Write-Host $key -ForegroundColor Cyan
                    if ($key -eq "Y") {
                        $enableAutoStart = $true
                    }
                    $keyPressed = $true
                    break
                } else {
                    $autoStartAttempts++
                    if ($autoStartAttempts -lt $maxAutoStartAttempts) {
                        Write-Host ""
                        Write-Host "Invalid response. Please enter 'Y' for yes or 'N' for no." -ForegroundColor Yellow
                        Write-Host "Enable auto-start on system boot? (Y/N): " -ForegroundColor Yellow -NoNewline
                    }
                }
            }

            # Check timeout
            if (((Get-Date) - $startTime).TotalSeconds -ge $timeoutSeconds -and -not $keyPressed) {
                Write-Host "Y (timeout - defaulting to yes)" -ForegroundColor Cyan
                Write-InstallLog -Level 'INFO' "No response received within $timeoutSeconds seconds. Enabling auto-start by default."
                $enableAutoStart = $true
                $keyPressed = $true
                break
            }

            Start-Sleep -Milliseconds 100
        }

        if (-not $keyPressed) {
            Write-Host "Y (max attempts reached - defaulting to yes)" -ForegroundColor Cyan
            Write-InstallLog -Level 'INFO' "Maximum attempts reached. Enabling auto-start by default."
            $enableAutoStart = $true
        }
    }

    Write-Host ""

    if ($enableAutoStart) {
        Write-InstallLog -Level 'INFO' "User chose to enable auto-start."
        # Create the main scheduled task
        Test-IsAdmin
        Set-StartupTask -Action 'Create' -ExecutablePath $downloadPath

        # Start the main task
        try {
            Start-ScheduledTask -TaskName $AppName
            Write-InstallLog -Level 'SUCCESS' "Application started successfully with auto-start enabled."
        }
        catch {
            Write-InstallLog -Level 'ERROR' "Failed to start the application task."
            Write-InstallLog -Level 'WARN' "Continuing installation - you can start the task manually later."
            Add-Content -Path $LogFile -Value "Error: $($_.Exception.Message)"
            Add-Content -Path $LogFile -Value "The application was installed but failed to start automatically. You can start it manually using: Start-ScheduledTask -TaskName $AppName"
            $script:HasErrors = $true
            $script:ErrorDetails += "Task start failed (non-critical)"
        }
    } else {
        Write-InstallLog -Level 'INFO' "User chose to skip auto-start."
        Write-Host "Auto-start will not be enabled." -ForegroundColor Yellow
        Write-Host ""

        # Offer to create a desktop shortcut instead
        Write-Host "Would you like to create a desktop shortcut instead? (Y/N): " -ForegroundColor Yellow -NoNewline

        $createShortcut = $false
        $shortcutAttempts = 0
        $maxShortcutAttempts = 2

        if (-not $isInteractive) {
            Write-Host "Y (default in non-interactive mode)" -ForegroundColor Cyan
            Write-InstallLog -Level 'INFO' "Non-interactive mode detected. Creating desktop shortcut by default."
            $createShortcut = $true
        } else {
            # Try to read with timeout
            $startTime = Get-Date
            $keyPressed = $false

            while ($shortcutAttempts -lt $maxShortcutAttempts -and -not $keyPressed) {
                if ([Console]::KeyAvailable) {
                    $response = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
                    $key = $response.Character.ToString().ToUpper()

                    if ($key -eq "Y" -or $key -eq "N") {
                        Write-Host $key -ForegroundColor Cyan
                        if ($key -eq "Y") {
                            $createShortcut = $true
                        }
                        $keyPressed = $true
                        break
                    } else {
                        $shortcutAttempts++
                        if ($shortcutAttempts -lt $maxShortcutAttempts) {
                            Write-Host ""
                            Write-Host "Invalid response. Please enter 'Y' for yes or 'N' for no." -ForegroundColor Yellow
                            Write-Host "Would you like to create a desktop shortcut instead? (Y/N): " -ForegroundColor Yellow -NoNewline
                        }
                    }
                }

                # Check timeout
                if (((Get-Date) - $startTime).TotalSeconds -ge $timeoutSeconds -and -not $keyPressed) {
                    Write-Host "Y (timeout - defaulting to yes)" -ForegroundColor Cyan
                    Write-InstallLog -Level 'INFO' "No response received within $timeoutSeconds seconds. Creating desktop shortcut by default."
                    $createShortcut = $true
                    $keyPressed = $true
                    break
                }

                Start-Sleep -Milliseconds 100
            }

            if (-not $keyPressed) {
                Write-Host "Y (max attempts reached - defaulting to yes)" -ForegroundColor Cyan
                Write-InstallLog -Level 'INFO' "Maximum attempts reached. Creating desktop shortcut by default."
                $createShortcut = $true
            }
        }

        Write-Host ""

        if ($createShortcut) {
            # Create main shortcut
            New-DesktopShortcut -ExecutablePath $downloadPath

            # Create "once" shortcut for initial cache ingest only
            New-DesktopShortcut -ExecutablePath $downloadPath `
                -Arguments "--once" `
                -ShortcutName "$AppName (Once)" `
                -Description "Deadlock API Ingest - Scan existing Steam cache once and exit"

            Write-Host "Desktop shortcuts created:" -ForegroundColor White
        } else {
            Write-Host "You can manually start the application by running: $downloadPath" -ForegroundColor White
            Write-Host "To run once (ingest existing cache only): $downloadPath --once" -ForegroundColor White
        }

        Write-Host "To enable auto-start later, re-run this installer." -ForegroundColor White
        Write-Host ""
    }


}
catch {
    Invoke-FatalError -ErrorMessage "Unexpected error during installation." -DetailedError $_.Exception.Message
}

# Display final status
if (-not $script:HasErrors) {
    Write-Host " "
    Write-InstallLog -Level 'SUCCESS' "Deadlock API Ingest has been installed successfully!"

    # Check if auto-start is enabled
    $autoStartEnabled = $false
    try {
        $task = Get-ScheduledTask -TaskName $AppName -ErrorAction SilentlyContinue
        if ($task) {
            $autoStartEnabled = $true
            Write-Host "[+] Auto-start is enabled" -ForegroundColor Green
            Write-Host "    The application will start automatically every time you log in." -ForegroundColor White
        } else {
            Write-Host "[-] Auto-start is disabled" -ForegroundColor Yellow
            Write-Host "    The application will not start automatically on user login." -ForegroundColor White
            Write-Host "    To enable auto-start later, re-run this installer." -ForegroundColor White
        }
    } catch {
        Write-Host "[?] Auto-start status unknown" -ForegroundColor Yellow
    }

    Write-Host " "

    if ($autoStartEnabled) {
        Write-Host "You can manage the main task via the Task Scheduler (taskschd.msc) or PowerShell:" -ForegroundColor White
        Write-Host "  - Check status:  Get-ScheduledTask -TaskName $AppName | Get-ScheduledTaskInfo" -ForegroundColor Yellow
        Write-Host "  - Run manually:  Start-ScheduledTask -TaskName $AppName" -ForegroundColor Yellow
        Write-Host "  - Stop it:       Stop-ScheduledTask -TaskName $AppName" -ForegroundColor Yellow
        Write-Host "  - Disable auto-start: Unregister-ScheduledTask -TaskName $AppName" -ForegroundColor Yellow
        Write-Host " "
    }

    Write-Host "To uninstall, run: $InstallDir\uninstall-windows.ps1" -ForegroundColor White
    Write-Host " "

    # --- User-friendly usage explanation ---
    Write-Host "How to use Deadlock API Ingest:" -ForegroundColor Green
    Write-Host "1. Restart your game after installation." -ForegroundColor Cyan
    Write-Host "2. Once in the game, go to your match history and click on 'matches'." -ForegroundColor Cyan
    Write-Host "3. The tool will now work in the background to enhance your experience!" -ForegroundColor Cyan
    Write-Host "If you have any questions, check the README or reach out for support.`n" -ForegroundColor Yellow
} else {
    Write-Host " "
    Write-Host "========================================" -ForegroundColor Yellow
    Write-Host "    INSTALLATION COMPLETED WITH ISSUES  " -ForegroundColor Yellow
    Write-Host "========================================" -ForegroundColor Yellow
    Write-Host " "
    Write-InstallLog -Level 'WARN' "Installation completed but some non-critical components failed:"
    foreach ($errorDetail in $script:ErrorDetails) {
        Write-Host "  - $errorDetail" -ForegroundColor Yellow
    }
    Write-Host " "
    Write-InstallLog -Level 'INFO' "The main application should still work. Check the log file for details: $LogFile"
    Write-Host " "
}

# Always show final status and wait for user input
Show-FinalStatus