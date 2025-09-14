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

# --- NEW FUNCTION: Replaces Manage-Service ---
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
            Write-Log -Level 'INFO' "Removing any existing scheduled task named '$AppName'..."
            Unregister-ScheduledTask -TaskName $AppName -Confirm:$false -ErrorAction SilentlyContinue
            Write-Log -Level 'SUCCESS' "Scheduled task cleanup complete."
        }
        'Create' {
            Write-Log -Level 'INFO' "Creating a new scheduled task to run on startup..."

            # Define the action (what program to run and its working directory)
            $taskAction = New-ScheduledTaskAction -Execute $ExecutablePath -WorkingDirectory $InstallDir

            # Define the trigger (when to run it)
            $taskTrigger = New-ScheduledTaskTrigger -AtStartup

            # Define the user and permissions (run as SYSTEM with highest privileges)
            $taskPrincipal = New-ScheduledTaskPrincipal -UserId "NT AUTHORITY\SYSTEM" -LogonType ServiceAccount -RunLevel Highest

            # Define settings (allow it to run indefinitely)
            $taskSettings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit 0

            # Register the task with the system
            Register-ScheduledTask -TaskName $AppName -Action $taskAction -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Description "Runs the Deadlock API Ingest application on system startup."

            Write-Log -Level 'SUCCESS' "Scheduled task created successfully."
        }
    }
}

# --- Main Installation Logic ---

Clear-Content -Path $LogFile -ErrorAction SilentlyContinue
Write-Log -Level 'INFO' "Starting Deadlock API Ingest installation..."
Write-Log -Level 'INFO' "Log file is available at: $LogFile"

Test-IsAdmin
$release = Get-LatestRelease

# Remove any old scheduled task
Manage-StartupTask -Action 'Remove'

Write-Log -Level 'INFO' "Creating installation directory: $InstallDir"
New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null

$downloadPath = Join-Path -Path $InstallDir -ChildPath $FinalExecutableName
Write-Log -Level 'INFO' "Downloading $($release.DownloadUrl)..."
Invoke-WebRequest -Uri $release.DownloadUrl -OutFile $downloadPath -UseBasicParsing

# Verify file size
$actualSize = (Get-Item -Path $downloadPath).Length
if ($actualSize -ne $release.Size) {
    Write-Log -Level 'ERROR' "File size mismatch! Expected: $($release.Size) bytes, Got: $actualSize bytes."
    exit 1
}
Write-Log -Level 'SUCCESS' "File integrity verified."

Unblock-File -Path $downloadPath

# Create the new scheduled task
Manage-StartupTask -Action 'Create' -ExecutablePath $downloadPath

Write-Host " "
Write-Log -Level 'SUCCESS' "Deadlock API Ingest ($($release.Version)) has been installed successfully!"
Write-Log -Level 'INFO' "The application will now start automatically every time the computer boots up."
Write-Host " "
Write-Host "You can manage the task via the Task Scheduler (taskschd.msc) or PowerShell:" -ForegroundColor White
Write-Host "  - Check status:  Get-ScheduledTask -TaskName $AppName | Get-ScheduledTaskInfo" -ForegroundColor Yellow
Write-Host "  - Run manually:  Start-ScheduledTask -TaskName $AppName" -ForegroundColor Yellow
Write-Host "  - Stop it:       Stop-ScheduledTask -TaskName $AppName" -ForegroundColor Yellow
Write-Host " "