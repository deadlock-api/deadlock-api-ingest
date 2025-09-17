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
        [scriptblock]$ScriptBlock
    )

    Write-Log -Level 'INFO' $Description

    try {
        # Capture output and redirect to log file
        $output = & $ScriptBlock 2>&1
        Add-Content -Path $LogFile -Value $output
        return $true
    }
    catch {
        Write-Log -Level 'ERROR' "Command failed: $($_.Exception.Message)"
        Add-Content -Path $LogFile -Value "Error: $($_.Exception.Message)"
        return $false
    }
}

# Function to check for Administrator privileges
function Test-IsAdmin {
    $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent()
    $isAdmin = (New-Object Security.Principal.WindowsPrincipal $currentUser).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $isAdmin) {
        Write-Log -Level 'ERROR' "This script requires Administrator privileges. Please re-run as Administrator."
        exit 1
    }
    Write-Log -Level 'INFO' "Running with Administrator privileges."
}

# Function to get the latest release from GitHub
function Get-LatestRelease {
    Write-Log -Level 'INFO' "Fetching latest release from repository: $GithubRepo"
    $ApiUrl = "https://api.github.com/repos/$GithubRepo/releases/latest"
    try {
        $releaseInfo = Invoke-RestMethod -Uri $ApiUrl -UseBasicParsing
    }
    catch {
        Write-Log -Level 'ERROR' "Failed to fetch release information from GitHub API."
        exit 1
    }
    $asset = $releaseInfo.assets | Where-Object { $_.name -like "*$AssetKeyword*" } | Select-Object -First 1
    if (-not $asset) {
        Write-Log -Level 'ERROR' "Could not find a release asset containing the keyword: '$AssetKeyword'"
        Write-Log -Level 'INFO' "Available assets are: $($releaseInfo.assets.name -join ', ')"
        exit 1
    }
    Write-Log -Level 'SUCCESS' "Found version: $($releaseInfo.tag_name)"
    return [PSCustomObject]@{
        Version      = $releaseInfo.tag_name
        DownloadUrl  = $asset.browser_download_url
        Size         = $asset.size
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
        Write-Log -Level 'ERROR' "Failed to download update checker script: $($_.Exception.Message)"
        exit 1
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
    }
}

# Function to create configuration file
function New-ConfigFile {
    Write-Log -Level 'INFO' "Creating configuration file..."

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

# Function to store version information
function Set-VersionInfo {
    param($Version)
    Set-Content -Path $VersionFile -Value $Version
    Write-Log -Level 'INFO' "Version information stored: $Version"
}

# --- Main Installation Logic ---

Clear-Content -Path $LogFile -ErrorAction SilentlyContinue
Write-Log -Level 'INFO' "Starting Deadlock API Ingest installation..."
Write-Log -Level 'INFO' "Log file is available at: $LogFile"

Test-IsAdmin
$release = Get-LatestRelease

# Remove any old scheduled tasks (both main and update)
Manage-StartupTask -Action 'Remove'
Manage-UpdateTask -Action 'Remove'

Write-Log -Level 'INFO' "Preparing installation environment..."
Stop-Process -Name $AppName -Force -ErrorAction SilentlyContinue

New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null
New-Item -Path $BackupDir -ItemType Directory -Force | Out-Null

# Ensure log directory exists
$UpdateLogDir = Split-Path -Parent $UpdateLogFile
New-Item -Path $UpdateLogDir -ItemType Directory -Force | Out-Null

$downloadPath = Join-Path -Path $InstallDir -ChildPath $FinalExecutableName
Write-Log -Level 'INFO' "Downloading application binary..."
# Log detailed URL to file only
Add-Content -Path $LogFile -Value "Downloading from: $($release.DownloadUrl)"
Invoke-WebRequest -Uri $release.DownloadUrl -OutFile $downloadPath -UseBasicParsing

# Verify file size
$actualSize = (Get-Item -Path $downloadPath).Length
if ($actualSize -ne $release.Size) {
    Write-Log -Level 'ERROR' "File size mismatch! Expected: $($release.Size) bytes, Got: $actualSize bytes."
    exit 1
}
Write-Log -Level 'SUCCESS' "Download complete and verified."

Unblock-File -Path $downloadPath

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
Start-ScheduledTask -TaskName $AppName

# Create the update scheduled task
Manage-UpdateTask -Action 'Create'

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