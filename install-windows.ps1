# Deadlock API Ingest - Windows Installation Script
# This script downloads and installs the application to run automatically on system startup via Task Scheduler.

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

# Function to handle errors and keep window open
function Handle-FatalError {
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

    Write-Log -Level 'ERROR' $ErrorMessage

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

    # Wait for user acknowledgment
    Write-Host "Press any key to close this window..." -ForegroundColor Green
    $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")

    exit $ExitCode
}

# Function to show final status and wait for user input
function Show-FinalStatus {
    if ($script:HasErrors) {
        Write-Host ""
        Write-Host "Installation completed with errors. Please review the messages above." -ForegroundColor Yellow
        Write-Host "Log file: $LogFile" -ForegroundColor Cyan
        Write-Host ""
        Write-Host "Press any key to close this window..." -ForegroundColor Green
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
    } else {
        Write-Host ""
        Write-Host "Installation completed successfully!" -ForegroundColor Green
        Write-Host ""
        Write-Host "Press any key to close this window..." -ForegroundColor Green
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
    }
}

# --- Helper Functions ---

# Function to write to log and console with color
function Write-Log {
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

    Write-Log -Level 'INFO' $Description

    try {
        # Capture output and redirect to log file
        $output = & $ScriptBlock 2>&1
        Add-Content -Path $LogFile -Value $output
        return $true
    }
    catch {
        $errorMsg = "Command failed: $($_.Exception.Message)"
        Write-Log -Level 'ERROR' $errorMsg
        Add-Content -Path $LogFile -Value "Error: $($_.Exception.Message)"
        Add-Content -Path $LogFile -Value "Stack Trace: $($_.ScriptStackTrace)"

        if (-not $ContinueOnError) {
            Handle-FatalError -ErrorMessage $errorMsg -DetailedError $_.Exception.Message
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
            Handle-FatalError -ErrorMessage "This script requires Administrator privileges. Please re-run as Administrator." -DetailedError "Right-click on PowerShell and select 'Run as Administrator', then run this script again."
        }
        Write-Log -Level 'INFO' "Running with Administrator privileges."
    }
    catch {
        Handle-FatalError -ErrorMessage "Failed to check Administrator privileges." -DetailedError $_.Exception.Message
    }
}

# Function to get the latest release from GitHub
function Get-LatestRelease {
    Write-Log -Level 'INFO' "Fetching latest release from repository: $GithubRepo"
    $ApiUrl = "https://api.github.com/repos/$GithubRepo/releases/latest"
    try {
        $releaseInfo = Invoke-RestMethod -Uri $ApiUrl -UseBasicParsing
    }
    catch {
        $errorMsg = "Failed to fetch release information from GitHub API."
        $detailedError = "URL: $ApiUrl`nError: $($_.Exception.Message)`n`nPossible causes:`n- No internet connection`n- GitHub API rate limit exceeded`n- Repository not found or private`n- Firewall blocking the request"
        Handle-FatalError -ErrorMessage $errorMsg -DetailedError $detailedError
    }

    try {
        $asset = $releaseInfo.assets | Where-Object { $_.name -like "*$AssetKeyword*" } | Select-Object -First 1
        if (-not $asset) {
            $availableAssets = if ($releaseInfo.assets) { $releaseInfo.assets.name -join ', ' } else { "None" }
            $errorMsg = "Could not find a release asset containing the keyword: '$AssetKeyword'"
            $detailedError = "Available assets are: $availableAssets`n`nThe release may not have been built for Windows, or the asset naming convention has changed."
            Handle-FatalError -ErrorMessage $errorMsg -DetailedError $detailedError
        }
        Write-Log -Level 'SUCCESS' "Found version: $($releaseInfo.tag_name)"
        return [PSCustomObject]@{
            Version      = $releaseInfo.tag_name
            DownloadUrl  = $asset.browser_download_url
            Size         = $asset.size
        }
    }
    catch {
        Handle-FatalError -ErrorMessage "Failed to process release information." -DetailedError $_.Exception.Message
    }
}

# Function to download the update checker script
function Get-UpdateChecker {
    Write-Log -Level 'INFO' "Downloading update checker script..."

    $UpdateScriptUrl = "https://raw.githubusercontent.com/$GithubRepo/master/update-checker.ps1"

    try {
        Invoke-WebRequest -Uri $UpdateScriptUrl -OutFile $UpdateScriptPath -UseBasicParsing
        Write-Log -Level 'SUCCESS' "Update checker script installed."
    } catch {
        $errorMsg = "Failed to download update checker script."
        $detailedError = "URL: $UpdateScriptUrl`nError: $($_.Exception.Message)`n`nThis is not critical for the main installation, but automatic updates will not work."
        Write-Log -Level 'ERROR' $errorMsg
        Write-Log -Level 'WARN' "Continuing installation without update checker script."
        $script:HasErrors = $true
        $script:ErrorDetails += "Update checker download failed (non-critical)"
    }
}

# Function to manage the Scheduled Task for autostart
function Manage-StartupTask {
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
            Write-Log -Level 'INFO' "Creating startup task..."

            try {
                # Define the action (what program to run and its working directory)
                $taskAction = New-ScheduledTaskAction -Execute $ExecutablePath -WorkingDirectory $InstallDir

                # Define the trigger (when to run it)
                $taskTrigger = New-ScheduledTaskTrigger -AtStartup

                # Define the user and permissions (run as SYSTEM with highest privileges)
                $taskPrincipal = New-ScheduledTaskPrincipal -UserId "NT AUTHORITY\SYSTEM" -LogonType ServiceAccount -RunLevel Highest

                # Define settings (allow it to run indefinitely)
                $taskSettings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0

                # Register the task with the system
                Register-ScheduledTask -TaskName $AppName -Action $taskAction -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Description "Runs the Deadlock API Ingest application on system startup." | Out-Null

                Write-Log -Level 'SUCCESS' "Startup task created successfully."
            }
            catch {
                Handle-FatalError -ErrorMessage "Failed to create startup task." -DetailedError "Error: $($_.Exception.Message)`n`nThis could be due to:`n- Insufficient permissions`n- Task Scheduler service not running`n- Conflicting task name`n- System policy restrictions"
            }
        }
    }
}

# Function to manage the update scheduled task
function Manage-UpdateTask {
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
            Write-Log -Level 'INFO' "Creating automatic update task..."

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

                Write-Log -Level 'SUCCESS' "Automatic updates enabled."
                # Log detailed schedule info to file only
                Add-Content -Path $LogFile -Value "Update task will run daily at 3:00 AM (with up to 30 minute random delay)."
            }
            catch {
                $errorMsg = "Failed to create automatic update task."
                $detailedError = "Error: $($_.Exception.Message)`n`nAutomatic updates will not be available, but the main application will still work."
                Write-Log -Level 'ERROR' $errorMsg
                Write-Log -Level 'WARN' "Continuing installation without automatic updates."
                $script:HasErrors = $true
                $script:ErrorDetails += "Update task creation failed (non-critical)"
            }
        }
    }
}

# Function to create configuration file
function New-ConfigFile {
    Write-Log -Level 'INFO' "Creating configuration file..."

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
        Write-Log -Level 'SUCCESS' "Configuration file created at $ConfigFile"
    }
    catch {
        $errorMsg = "Failed to create configuration file."
        $detailedError = "Error: $($_.Exception.Message)`n`nPath: $ConfigFile`n`nThis is not critical for the main application."
        Write-Log -Level 'ERROR' $errorMsg
        Write-Log -Level 'WARN' "Continuing installation without configuration file."
        $script:HasErrors = $true
        $script:ErrorDetails += "Configuration file creation failed (non-critical)"
    }
}

# Function to store version information
function Set-VersionInfo {
    param($Version)
    try {
        Set-Content -Path $VersionFile -Value $Version
        Write-Log -Level 'INFO' "Version information stored: $Version"
    }
    catch {
        $errorMsg = "Failed to store version information."
        $detailedError = "Error: $($_.Exception.Message)`n`nPath: $VersionFile`n`nThis is not critical for the main application."
        Write-Log -Level 'ERROR' $errorMsg
        Write-Log -Level 'WARN' "Continuing installation without version file."
        $script:HasErrors = $true
        $script:ErrorDetails += "Version file creation failed (non-critical)"
    }
}

# --- Main Installation Logic ---

# Wrap the entire installation in a try-catch block
try {
    Clear-Content -Path $LogFile -ErrorAction SilentlyContinue
    Write-Log -Level 'INFO' "Starting Deadlock API Ingest installation..."
    Write-Log -Level 'INFO' "Log file is available at: $LogFile"

    Test-IsAdmin
    $release = Get-LatestRelease

    # Remove any old scheduled tasks (both main and update)
    Invoke-Quietly "Removing existing scheduled tasks..." {
        Manage-StartupTask -Action 'Remove'
        Manage-UpdateTask -Action 'Remove'
    } -ContinueOnError | Out-Null

    Write-Log -Level 'INFO' "Preparing installation environment..."

    try {
        Stop-Process -Name $AppName -Force -ErrorAction SilentlyContinue
    }
    catch {
        # This is expected if the process isn't running
        Write-Log -Level 'INFO' "No existing process to stop."
    }

    try {
        New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null
        New-Item -Path $BackupDir -ItemType Directory -Force | Out-Null

        # Ensure log directory exists
        $UpdateLogDir = Split-Path -Parent $UpdateLogFile
        New-Item -Path $UpdateLogDir -ItemType Directory -Force | Out-Null
    }
    catch {
        Handle-FatalError -ErrorMessage "Failed to create installation directories." -DetailedError "Error: $($_.Exception.Message)`n`nInstall Directory: $InstallDir`nBackup Directory: $BackupDir`nUpdate Log Directory: $UpdateLogDir"
    }

    $downloadPath = Join-Path -Path $InstallDir -ChildPath $FinalExecutableName
    Write-Log -Level 'INFO' "Downloading application binary..."
    # Log detailed URL to file only
    Add-Content -Path $LogFile -Value "Downloading from: $($release.DownloadUrl)"

    try {
        Invoke-WebRequest -Uri $release.DownloadUrl -OutFile $downloadPath -UseBasicParsing
    }
    catch {
        Handle-FatalError -ErrorMessage "Failed to download application binary." -DetailedError "URL: $($release.DownloadUrl)`nDestination: $downloadPath`nError: $($_.Exception.Message)`n`nPossible causes:`n- No internet connection`n- Insufficient disk space`n- Permission denied to write to installation directory"
    }

    # Verify file size
    try {
        $actualSize = (Get-Item -Path $downloadPath).Length
        if ($actualSize -ne $release.Size) {
            Handle-FatalError -ErrorMessage "File size mismatch! Download may be corrupted." -DetailedError "Expected: $($release.Size) bytes`nActual: $actualSize bytes`n`nPlease try running the installation again."
        }
        Write-Log -Level 'SUCCESS' "Download complete and verified."
    }
    catch {
        Handle-FatalError -ErrorMessage "Failed to verify downloaded file." -DetailedError $_.Exception.Message
    }

    try {
        Unblock-File -Path $downloadPath
    }
    catch {
        Write-Log -Level 'WARN' "Failed to unblock downloaded file, but continuing installation."
        $script:HasErrors = $true
        $script:ErrorDetails += "File unblock failed (non-critical)"
    }

    Write-Log -Level 'INFO' "Installing application..."

    # Store version information
    Set-VersionInfo -Version $release.Version

    # Create configuration file
    New-ConfigFile

    # Download update checker script
    Get-UpdateChecker

    # Create the main scheduled task
    Manage-StartupTask -Action 'Create' -ExecutablePath $downloadPath

    # Start the main task
    try {
        Start-ScheduledTask -TaskName $AppName
        Write-Log -Level 'SUCCESS' "Application started successfully."
    }
    catch {
        $errorMsg = "Failed to start the application task."
        $detailedError = "Error: $($_.Exception.Message)`n`nThe application was installed but failed to start automatically. You can start it manually using:`nStart-ScheduledTask -TaskName $AppName"
        Write-Log -Level 'ERROR' $errorMsg
        Write-Log -Level 'WARN' "Continuing installation - you can start the task manually later."
        $script:HasErrors = $true
        $script:ErrorDetails += "Task start failed (non-critical)"
    }

    # Create the update scheduled task
    Manage-UpdateTask -Action 'Create'
}
catch {
    Handle-FatalError -ErrorMessage "Unexpected error during installation." -DetailedError $_.Exception.Message
}

# Display final status
if (-not $script:HasErrors) {
    Write-Host " "
    Write-Log -Level 'SUCCESS' "Deadlock API Ingest ($($release.Version)) has been installed successfully with automatic updates!"
    Write-Log -Level 'INFO' "The application will now start automatically every time the computer boots up."
    Write-Host " "
    Write-Host "You can manage the main task via the Task Scheduler (taskschd.msc) or PowerShell:" -ForegroundColor White
    Write-Host "  - Check status:  Get-ScheduledTask -TaskName $AppName | Get-ScheduledTaskInfo" -ForegroundColor Yellow
    Write-Host "  - Run manually:  Start-ScheduledTask -TaskName $AppName" -ForegroundColor Yellow
    Write-Host "  - Stop it:       Stop-ScheduledTask -TaskName $AppName" -ForegroundColor Yellow
    Write-Host " "
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
    Write-Log -Level 'WARN' "Installation completed but some non-critical components failed:"
    foreach ($error in $script:ErrorDetails) {
        Write-Host "  - $error" -ForegroundColor Yellow
    }
    Write-Host " "
    Write-Log -Level 'INFO' "The main application should still work. Check the log file for details: $LogFile"
    Write-Host " "
}

# Always show final status and wait for user input
Show-FinalStatus