# Deadlock API Ingest - Windows Installation Script
# This script downloads and installs the application to run automatically on system startup via Task Scheduler.

# Suppress PSScriptAnalyzer warnings for Write-Host in this interactive installation script
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '', Justification='Interactive installation script requires colored console output for user experience')]
param()

# --- Configuration ---
$AppName = "deadlock-api-ingest"
$GithubRepo = "deadlock-api/deadlock-api-ingest"
$AssetKeyword = "windows-latest.exe"

# Installation Paths
$InstallDir = "$env:ProgramFiles\$AppName"
$FinalExecutableName = "$AppName.exe"
$LogFile = "$env:TEMP\${AppName}-install.log"

# Update functionality
$UpdateTaskName = "${AppName}-updater"
$UpdateScriptPath = "$InstallDir\update-checker.ps1"
$UpdateLogFile = "$env:ProgramData\${AppName}\updater.log"
$VersionFile = "$InstallDir\version.txt"
$ConfigFile = "$InstallDir\config.conf"
$BackupDir = "$InstallDir\backup"

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

    # Show recent log entries
    if (Test-Path $LogFile) {
        Write-Host "Recent log entries:" -ForegroundColor Yellow
        try {
            $recentLogs = Get-Content $LogFile -Tail 10 -ErrorAction SilentlyContinue
            if ($recentLogs) {
                $recentLogs | ForEach-Object { Write-Host "  $_" -ForegroundColor Gray }
            }
        } catch {
            Write-Host "  Unable to read recent log entries" -ForegroundColor Gray
        }
        Write-Host ""
    }

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
            Invoke-FatalError -ErrorMessage "This script requires Administrator privileges. Please re-run as Administrator." -DetailedError "Right-click on PowerShell and select 'Run as Administrator', then run this script again."
        }
        Write-InstallLog -Level 'INFO' "Running with Administrator privileges."
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
        Write-InstallLog -Level 'SUCCESS' "Found version: $($releaseInfo.tag_name)"
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

# Function to download the update checker script
function Get-UpdateChecker {
    Write-InstallLog -Level 'INFO' "Downloading update checker script..."

    $UpdateScriptUrl = "https://raw.githubusercontent.com/$GithubRepo/master/update-checker.ps1"

    try {
        Invoke-WebRequest -Uri $UpdateScriptUrl -OutFile $UpdateScriptPath -UseBasicParsing
        Write-InstallLog -Level 'SUCCESS' "Update checker script installed."
    } catch {
        Write-InstallLog -Level 'ERROR' "Failed to download update checker script."
        Write-InstallLog -Level 'WARN' "Continuing installation without update checker script."
        Add-Content -Path $LogFile -Value "Error details: $($_.Exception.Message)"
        $script:HasErrors = $true
        $script:ErrorDetails += "Update checker download failed (non-critical)"
    }
}

# Function to get the actual user who invoked the script (not Administrator)
function Get-ActualUser {
    try {
        # Try to get the user from environment variables set by UAC elevation
        $actualUser = $env:USERNAME

        # If running elevated, try to get the original user
        if ([Security.Principal.WindowsIdentity]::GetCurrent().Name -match "Administrator") {
            # Try various methods to get the actual user

            # Method 1: Check USERPROFILE path
            if ($env:USERPROFILE -notmatch "Administrator") {
                $actualUser = Split-Path $env:USERPROFILE -Leaf
            }
            # Method 2: Get the user who owns the explorer.exe process (usually the logged-in user)
            else {
                $explorerProcess = Get-CimInstance Win32_Process -Filter "name = 'explorer.exe'" | Select-Object -First 1
                if ($explorerProcess) {
                    $owner = Invoke-CimMethod -InputObject $explorerProcess -MethodName GetOwner
                    if ($owner.User) {
                        $actualUser = $owner.User
                    }
                }
            }
        }

        Write-InstallLog -Level 'INFO' "Detected user: $actualUser"
        return $actualUser
    }
    catch {
        Write-InstallLog -Level 'WARN' "Could not determine actual user, using current user: $env:USERNAME"
        return $env:USERNAME
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
        [string]$Description = "Deadlock API Ingest - Network packet analyzer for Deadlock game replay data"
    )

    Write-InstallLog -Level 'INFO' "Creating desktop shortcut: $ShortcutName..."

    try {
        # Get the actual user (not Administrator)
        $actualUser = Get-ActualUser

        # Build the path to the user's desktop
        $userProfile = "C:\Users\$actualUser"
        if (-not (Test-Path $userProfile)) {
            # Fallback: try to find the user profile
            $userProfile = (Get-ChildItem "C:\Users" | Where-Object { $_.Name -eq $actualUser } | Select-Object -First 1).FullName
            if (-not $userProfile) {
                throw "Could not find user profile for: $actualUser"
            }
        }

        $DesktopPath = Join-Path -Path $userProfile -ChildPath "Desktop"
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
                # Define the action (what program to run and its working directory)
                $taskAction = New-ScheduledTaskAction -Execute $ExecutablePath -WorkingDirectory $InstallDir

                # Define the trigger (when to run it)
                $taskTrigger = New-ScheduledTaskTrigger -AtStartup

                # Define the user and permissions (run as SYSTEM with highest privileges)
                $taskPrincipal = New-ScheduledTaskPrincipal -UserId "NT AUTHORITY\SYSTEM" -LogonType ServiceAccount -RunLevel Highest

                # Define settings (allow it to run indefinitely, prevent multiple instances)
                $taskSettings = New-ScheduledTaskSettingsSet `
                    -AllowStartIfOnBatteries `
                    -DontStopIfGoingOnBatteries `
                    -ExecutionTimeLimit 0 `
                    -StartWhenAvailable `
                    -MultipleInstances IgnoreNew

                # Register the task with the system
                Register-ScheduledTask -TaskName $AppName -Action $taskAction -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Description "Runs the Deadlock API Ingest application on system startup." | Out-Null

                Write-InstallLog -Level 'SUCCESS' "Startup task created successfully."
            }
            catch {
                Invoke-FatalError -ErrorMessage "Failed to create startup task." -DetailedError "Error: $($_.Exception.Message)`n`nThis could be due to:`n- Insufficient permissions`n- Task Scheduler service not running`n- Conflicting task name`n- System policy restrictions"
            }
        }
    }
}

# Function to manage the Watchdog Task (ensures app is always running)
function Set-WatchdogTask {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('Remove', 'Create')]
        [string]$Action,
        [string]$ExecutablePath
    )

    $WatchdogTaskName = "$AppName-Watchdog"

    switch ($Action) {
        'Remove' {
            Invoke-Quietly "Removing existing watchdog task..." {
                Unregister-ScheduledTask -TaskName $WatchdogTaskName -Confirm:$false -ErrorAction SilentlyContinue
            } | Out-Null
        }
        'Create' {
            Write-InstallLog -Level 'INFO' "Creating watchdog task to ensure application stays running..."

            try {
                # Create a PowerShell script that checks if the process is running
                $watchdogScript = @"
`$processName = '$([System.IO.Path]::GetFileNameWithoutExtension($ExecutablePath))'
`$taskName = '$AppName'

# Check if the process is already running
`$process = Get-Process -Name `$processName -ErrorAction SilentlyContinue

if (-not `$process) {
    # Process not running, start the scheduled task
    Start-ScheduledTask -TaskName `$taskName -ErrorAction SilentlyContinue
}
"@

                $watchdogScriptPath = Join-Path $InstallDir "watchdog.ps1"
                Set-Content -Path $watchdogScriptPath -Value $watchdogScript -Force

                # Define the action (run PowerShell with the watchdog script)
                $taskAction = New-ScheduledTaskAction -Execute "PowerShell.exe" -Argument "-ExecutionPolicy Bypass -WindowStyle Hidden -File `"$watchdogScriptPath`"" -WorkingDirectory $InstallDir

                # Define the trigger (every 30 minutes)
                $taskTrigger = New-ScheduledTaskTrigger -Once -At (Get-Date) -RepetitionInterval (New-TimeSpan -Minutes 30)

                # Define the user and permissions (run as SYSTEM)
                $taskPrincipal = New-ScheduledTaskPrincipal -UserId "NT AUTHORITY\SYSTEM" -LogonType ServiceAccount -RunLevel Highest

                # Define settings
                $taskSettings = New-ScheduledTaskSettingsSet `
                    -AllowStartIfOnBatteries `
                    -DontStopIfGoingOnBatteries `
                    -ExecutionTimeLimit (New-TimeSpan -Minutes 5) `
                    -StartWhenAvailable `
                    -MultipleInstances IgnoreNew

                # Register the task with the system
                Register-ScheduledTask -TaskName $WatchdogTaskName -Action $taskAction -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Description "Monitors and restarts the Deadlock API Ingest application if it stops running." | Out-Null

                Write-InstallLog -Level 'SUCCESS' "Watchdog task created successfully (checks every 30 minutes)."
            }
            catch {
                Write-InstallLog -Level 'ERROR' "Failed to create watchdog task: $($_.Exception.Message)"
                Write-InstallLog -Level 'WARN' "Continuing installation without watchdog task."
                $script:HasErrors = $true
                $script:ErrorDetails += "Watchdog task creation failed (non-critical)"
            }
        }
    }
}

# Function to manage the update scheduled task
function Set-UpdateTask {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('Remove', 'Create')]
        [string]$Action
    )

    switch ($Action) {
        'Remove' {
            Invoke-Quietly "Removing existing update task..." {
                Unregister-ScheduledTask -TaskName $UpdateTaskName -Confirm:$false -ErrorAction SilentlyContinue
            } | Out-Null
        }
        'Create' {
            Write-InstallLog -Level 'INFO' "Creating automatic update task..."

            try {
                # Define the action (run PowerShell with the update script)
                $taskAction = New-ScheduledTaskAction -Execute "PowerShell.exe" -Argument "-ExecutionPolicy Bypass -File `"$UpdateScriptPath`"" -WorkingDirectory $InstallDir

                # Define the trigger (daily at 3 AM with random delay)
                $taskTrigger = New-ScheduledTaskTrigger -Daily -At "3:00 AM"
                $taskTrigger.RandomDelay = "PT30M"  # 30 minute random delay

                # Define the user and permissions (run as SYSTEM)
                $taskPrincipal = New-ScheduledTaskPrincipal -UserId "NT AUTHORITY\SYSTEM" -LogonType ServiceAccount -RunLevel Highest

                # Define settings
                $taskSettings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Hours 1) -StartWhenAvailable

                # Register the task with the system
                Register-ScheduledTask -TaskName $UpdateTaskName -Action $taskAction -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Description "Daily update checker for Deadlock API Ingest application." | Out-Null

                Write-InstallLog -Level 'SUCCESS' "Automatic updates enabled."
                # Log detailed schedule info to file only
                Add-Content -Path $LogFile -Value "Update task will run daily at 3:00 AM (with up to 30 minute random delay)."
            }
            catch {
                Write-InstallLog -Level 'ERROR' "Failed to create automatic update task."
                Write-InstallLog -Level 'WARN' "Continuing installation without automatic updates."
                Add-Content -Path $LogFile -Value "Error details: $($_.Exception.Message)"
                $script:HasErrors = $true
                $script:ErrorDetails += "Update task creation failed (non-critical)"
            }
        }
    }
}

# Function to create configuration file
function New-ConfigFile {
    Write-InstallLog -Level 'INFO' "Creating configuration file..."

    try {
        $ConfigContent = @"
# Deadlock API Ingest Configuration
# This file controls various settings for the application and updater

# Automatic Updates
# Set to "false" to disable automatic updates
AUTO_UPDATE="true"

# Update Check Time
# The task runs daily at 3 AM, but you can manually trigger updates with:
# Start-ScheduledTask -TaskName $UpdateTaskName

# Backup Retention
# Number of backup versions to keep (default: 5)
BACKUP_RETENTION=5

# Update Log Level
# Options: INFO, WARN, ERROR
UPDATE_LOG_LEVEL="INFO"
"@

        Set-Content -Path $ConfigFile -Value $ConfigContent
        Write-InstallLog -Level 'SUCCESS' "Configuration file created at $ConfigFile"
    }
    catch {
        Write-InstallLog -Level 'ERROR' "Failed to create configuration file."
        Write-InstallLog -Level 'WARN' "Continuing installation without configuration file."
        Add-Content -Path $LogFile -Value "Error details: $($_.Exception.Message)"
        $script:HasErrors = $true
        $script:ErrorDetails += "Configuration file creation failed (non-critical)"
    }
}

# Function to store version information
function Set-VersionInfo {
    param($Version)
    try {
        Set-Content -Path $VersionFile -Value $Version
        Write-InstallLog -Level 'INFO' "Version information stored: $Version"
    }
    catch {
        Write-InstallLog -Level 'ERROR' "Failed to store version information."
        Write-InstallLog -Level 'WARN' "Continuing installation without version file."
        Add-Content -Path $LogFile -Value "Error: $($_.Exception.Message)"
        Add-Content -Path $LogFile -Value "Path: $VersionFile"
        Add-Content -Path $LogFile -Value "This is not critical for the main application."
        $script:HasErrors = $true
        $script:ErrorDetails += "Version file creation failed (non-critical)"
    }
}

# --- Main Installation Logic ---

# Wrap the entire installation in a try-catch block
try {
    Clear-Content -Path $LogFile -ErrorAction SilentlyContinue
    Write-InstallLog -Level 'INFO' "Starting Deadlock API Ingest installation..."
    Write-InstallLog -Level 'INFO' "Log file is available at: $LogFile"

    Test-IsAdmin
    $release = Get-LatestRelease

    # Remove any old scheduled tasks (main, watchdog, and update)
    Invoke-Quietly "Removing existing scheduled tasks..." {
        Set-StartupTask -Action 'Remove'
        Set-WatchdogTask -Action 'Remove'
        Set-UpdateTask -Action 'Remove'
    } -ContinueOnError | Out-Null

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
        New-Item -Path $BackupDir -ItemType Directory -Force | Out-Null

        # Ensure log directory exists
        $UpdateLogDir = Split-Path -Parent $UpdateLogFile
        New-Item -Path $UpdateLogDir -ItemType Directory -Force | Out-Null
    }
    catch {
        Invoke-FatalError -ErrorMessage "Failed to create installation directories." -DetailedError "Error: $($_.Exception.Message)`n`nInstall Directory: $InstallDir`nBackup Directory: $BackupDir`nUpdate Log Directory: $UpdateLogDir"
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

    Write-InstallLog -Level 'INFO' "Installing application..."

    # Ask user if they want auto-start
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "      AUTO-START SETUP (OPTIONAL)      " -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Auto-start will automatically start the application when the system boots." -ForegroundColor White
    Write-Host "This ensures the application is always running in the background." -ForegroundColor White
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
        Set-StartupTask -Action 'Create' -ExecutablePath $downloadPath

        # Create the watchdog task to ensure the app stays running
        Set-WatchdogTask -Action 'Create' -ExecutablePath $downloadPath

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
                -Description "Deadlock API Ingest - Run once to ingest existing cache files only"

            Write-Host "Desktop shortcuts created:" -ForegroundColor White
        } else {
            Write-Host "You can manually start the application by running: $downloadPath" -ForegroundColor White
            Write-Host "To run once (ingest existing cache only): $downloadPath --once" -ForegroundColor White
        }

        Write-Host "To enable auto-start later, re-run this installer." -ForegroundColor White
        Write-Host ""
    }

    # Ask user if they want automatic updates
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "     AUTOMATIC UPDATES (OPTIONAL)      " -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Would you like to enable automatic updates?" -ForegroundColor Yellow
    Write-Host "This will create a scheduled task that checks for updates daily at 3 AM." -ForegroundColor White
    Write-Host ""
    Write-Host "Enable automatic updates? (Y/N): " -ForegroundColor Yellow -NoNewline

    $installUpdater = $false
    $updaterAttempts = 0
    $maxUpdaterAttempts = 2

    if (-not $isInteractive) {
        Write-Host "Y (default in non-interactive mode)" -ForegroundColor Cyan
        Write-InstallLog -Level 'INFO' "Non-interactive mode detected. Installing automatic updater by default."
        $installUpdater = $true
    } else {
        # Try to read with timeout
        $startTime = Get-Date
        $keyPressed = $false

        while ($updaterAttempts -lt $maxUpdaterAttempts -and -not $keyPressed) {
            if ([Console]::KeyAvailable) {
                $response = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
                $key = $response.Character.ToString().ToUpper()

                if ($key -eq "Y" -or $key -eq "N") {
                    Write-Host $key -ForegroundColor Cyan
                    if ($key -eq "Y") {
                        $installUpdater = $true
                    }
                    $keyPressed = $true
                    break
                } else {
                    $updaterAttempts++
                    if ($updaterAttempts -lt $maxUpdaterAttempts) {
                        Write-Host ""
                        Write-Host "Invalid response. Please enter 'Y' for yes or 'N' for no." -ForegroundColor Yellow
                        Write-Host "Enable automatic updates? (Y/N): " -ForegroundColor Yellow -NoNewline
                    }
                }
            }

            # Check timeout
            if (((Get-Date) - $startTime).TotalSeconds -ge $timeoutSeconds -and -not $keyPressed) {
                Write-Host "Y (timeout - defaulting to yes)" -ForegroundColor Cyan
                Write-InstallLog -Level 'INFO' "No response received within $timeoutSeconds seconds. Installing automatic updater by default."
                $installUpdater = $true
                $keyPressed = $true
                break
            }

            Start-Sleep -Milliseconds 100
        }

        if (-not $keyPressed) {
            Write-Host "Y (max attempts reached - defaulting to yes)" -ForegroundColor Cyan
            Write-InstallLog -Level 'INFO' "Maximum attempts reached. Installing automatic updater by default."
            $installUpdater = $true
        }
    }

    Write-Host ""

    if ($installUpdater) {
        Write-InstallLog -Level 'INFO' "User chose to enable automatic updates."
        # Store version information
        Set-VersionInfo -Version $release.Version
        # Create configuration file
        New-ConfigFile
        # Download update checker script
        Get-UpdateChecker
        # Create the update scheduled task
        Set-UpdateTask -Action 'Create'
    } else {
        Write-InstallLog -Level 'INFO' "User chose to skip automatic updates."
        Write-Host "Automatic updates will not be installed." -ForegroundColor Yellow
        Write-Host "You can manually update by downloading the latest version from GitHub or re-run the installer." -ForegroundColor White
        Write-Host ""
    }
}
catch {
    Invoke-FatalError -ErrorMessage "Unexpected error during installation." -DetailedError $_.Exception.Message
}

# Display final status
if (-not $script:HasErrors) {
    Write-Host " "
    Write-InstallLog -Level 'SUCCESS' "Deadlock API Ingest ($($release.Version)) has been installed successfully!"

    # Check if auto-start is enabled
    $autoStartEnabled = $false
    try {
        $task = Get-ScheduledTask -TaskName $AppName -ErrorAction SilentlyContinue
        if ($task) {
            $autoStartEnabled = $true
            Write-Host "[+] Auto-start is enabled" -ForegroundColor Green
            Write-Host "    The application will start automatically every time the computer boots up." -ForegroundColor White
        } else {
            Write-Host "[-] Auto-start is disabled" -ForegroundColor Yellow
            Write-Host "    The application will not start automatically on system boot." -ForegroundColor White
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

    # Check if automatic updates are enabled
    $updatesEnabled = $false
    try {
        $updateTask = Get-ScheduledTask -TaskName $UpdateTaskName -ErrorAction SilentlyContinue
        if ($updateTask) {
            $updatesEnabled = $true
        }
    } catch {
        # Silently continue if task doesn't exist
        Write-Verbose "Update task not found or error checking task status: $_"
    }

    if ($updatesEnabled) {
        Write-Host "Automatic update functionality:" -ForegroundColor White
        Write-Host "  - Update task:   Get-ScheduledTask -TaskName $UpdateTaskName | Get-ScheduledTaskInfo" -ForegroundColor Yellow
        Write-Host "  - Manual update: Start-ScheduledTask -TaskName $UpdateTaskName" -ForegroundColor Yellow
        Write-Host "  - Update logs:   Get-Content '$UpdateLogFile'" -ForegroundColor Yellow
        Write-Host "  - Disable updates: Edit '$ConfigFile' and set AUTO_UPDATE=`"false`"" -ForegroundColor Yellow
        Write-Host " "
        Write-Host "Configuration file: $ConfigFile" -ForegroundColor Cyan
        Write-Host "Version file: $VersionFile" -ForegroundColor Cyan
        Write-Host "Update logs: $UpdateLogFile" -ForegroundColor Cyan
        Write-Host " "
    }

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
    foreach ($error in $script:ErrorDetails) {
        Write-Host "  - $error" -ForegroundColor Yellow
    }
    Write-Host " "
    Write-InstallLog -Level 'INFO' "The main application should still work. Check the log file for details: $LogFile"
    Write-Host " "
}

# Always show final status and wait for user input
Show-FinalStatus