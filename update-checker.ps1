# Deadlock API Ingest - Update Checker Script
# This script checks for new releases and updates the application automatically

# Suppress PSScriptAnalyzer warnings for Write-Host in this script
[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingWriteHost', '', Justification='Console output needed for interactive execution')]
param(
    [switch]$Force
)

# --- Configuration ---
$AppName = "deadlock-api-ingest"
$GithubRepo = "deadlock-api/deadlock-api-ingest"
$AssetKeyword = "windows-latest.exe"
$InstallDir = "$env:ProgramFiles\$AppName"
$FinalExecutableName = "$AppName.exe"
$VersionFile = "$InstallDir\version.txt"
$ConfigFile = "$InstallDir\config.conf"
$BackupDir = "$InstallDir\backup"
$UpdateLogFile = "$env:ProgramData\$AppName\updater.log"

# --- Helper Functions ---

function Write-UpdateLog {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('INFO', 'WARN', 'ERROR', 'SUCCESS')]
        [string]$Level,
        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    $Timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $LogMessage = "[$Timestamp] [$Level] $Message"

    # Ensure log directory exists
    $LogDir = Split-Path -Parent $UpdateLogFile
    if (-not (Test-Path $LogDir)) {
        New-Item -Path $LogDir -ItemType Directory -Force | Out-Null
    }

    # Write to log file
    Add-Content -Path $UpdateLogFile -Value $LogMessage

    # Also write to event log
    $EventSource = "$AppName-Updater"
    if (-not [System.Diagnostics.EventLog]::SourceExists($EventSource)) {
        try {
            New-EventLog -LogName Application -Source $EventSource
        } catch {
            # Silently continue if we can't create event source (requires admin privileges)
            Write-Verbose "Unable to create event log source: $_"
        }
    }

    $EventType = switch ($Level) {
        'ERROR' { 'Error' }
        'WARN' { 'Warning' }
        default { 'Information' }
    }

    try {
        Write-EventLog -LogName Application -Source $EventSource -EntryType $EventType -EventId 1000 -Message $Message
    } catch {
        # Silently continue if we can't write to event log
        Write-Verbose "Unable to write to event log: $_"
    }

    # Also output to console if running interactively
    if ($Host.Name -eq "ConsoleHost") {
        $ColorMap = @{ 'INFO' = 'Cyan'; 'WARN' = 'Yellow'; 'ERROR' = 'Red'; 'SUCCESS' = 'Green' }
        $Color = $ColorMap[$Level]
        Write-Host $LogMessage -ForegroundColor $Color
    }
}

function Test-UpdateEnabled {
    if (Test-Path $ConfigFile) {
        $config = Get-Content $ConfigFile | Where-Object { $_ -match '^AUTO_UPDATE=' }
        if ($config) {
            $autoUpdate = ($config -split '=')[1] -replace '"', ''
            if ($autoUpdate -eq 'false') {
                Write-UpdateLog -Level 'INFO' "Automatic updates are disabled. Exiting."
                return $false
            }
        }
    }
    Write-UpdateLog -Level 'INFO' "Automatic updates are enabled."
    return $true
}

function Get-CurrentVersion {
    if (Test-Path $VersionFile) {
        return Get-Content $VersionFile -Raw | ForEach-Object { $_.Trim() }
    } else {
        return "unknown"
    }
}

function Get-LatestVersion {
    try {
        $ApiUrl = "https://api.github.com/repos/$GithubRepo/releases/latest"
        $releaseInfo = Invoke-RestMethod -Uri $ApiUrl -UseBasicParsing
        return $releaseInfo.tag_name
    } catch {
        Write-UpdateLog -Level 'ERROR' "Failed to fetch release information from GitHub API: $($_.Exception.Message)"
        return $null
    }
}

function Test-UpdateNeeded {
    param($Current, $Latest)

    if ($Current -eq "unknown" -or $Current -ne $Latest) {
        return $true
    } else {
        return $false
    }
}

function New-Backup {
    $ExecutablePath = Join-Path $InstallDir $FinalExecutableName
    $BackupPath = Join-Path $BackupDir "$FinalExecutableName.$(Get-Date -Format 'yyyyMMdd_HHmmss')"

    if (-not (Test-Path $BackupDir)) {
        New-Item -Path $BackupDir -ItemType Directory -Force | Out-Null
    }

    if (Test-Path $ExecutablePath) {
        Copy-Item $ExecutablePath $BackupPath
        Write-UpdateLog -Level 'INFO' "Created backup: $BackupPath"
        return $BackupPath
    } else {
        Write-UpdateLog -Level 'WARN' "No existing executable found to backup."
        return $null
    }
}

function Restore-Backup {
    param($BackupPath)

    $ExecutablePath = Join-Path $InstallDir $FinalExecutableName

    if ($BackupPath -and (Test-Path $BackupPath)) {
        Write-UpdateLog -Level 'INFO' "Rolling back to previous version..."
        Copy-Item $BackupPath $ExecutablePath -Force
        Write-UpdateLog -Level 'SUCCESS' "Rollback completed."
        return $true
    } else {
        Write-UpdateLog -Level 'ERROR' "No backup available for rollback."
        return $false
    }
}

function Install-NewVersion {
    param($Version)

    try {
        $ApiUrl = "https://api.github.com/repos/$GithubRepo/releases/latest"
        $releaseInfo = Invoke-RestMethod -Uri $ApiUrl -UseBasicParsing

        $asset = $releaseInfo.assets | Where-Object { $_.name -like "*$AssetKeyword*" } | Select-Object -First 1
        if (-not $asset) {
            Write-UpdateLog -Level 'ERROR' "Could not find a release asset containing the keyword: '$AssetKeyword'"
            return $false
        }

        $TempDownloadPath = "$env:TEMP\$AppName-update-$Version.exe"
        $ExecutablePath = Join-Path $InstallDir $FinalExecutableName

        Write-UpdateLog -Level 'INFO' "Downloading new version from: $($asset.browser_download_url)"

        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $TempDownloadPath -UseBasicParsing

        # Verify file size
        $ActualSize = (Get-Item $TempDownloadPath).Length
        if ($ActualSize -ne $asset.size) {
            Write-UpdateLog -Level 'ERROR' "File size mismatch! Expected: $($asset.size) bytes, Got: $ActualSize bytes."
            Remove-Item $TempDownloadPath -Force
            return $false
        }

        # Unblock the file
        Unblock-File -Path $TempDownloadPath

        # Install new version
        Move-Item $TempDownloadPath $ExecutablePath -Force

        # Update version file
        Set-Content -Path $VersionFile -Value $Version

        Write-UpdateLog -Level 'SUCCESS' "New version installed successfully."
        return $true

    } catch {
        Write-UpdateLog -Level 'ERROR' "Failed to download and install new version: $($_.Exception.Message)"
        if (Test-Path $TempDownloadPath) {
            Remove-Item $TempDownloadPath -Force
        }
        return $false
    }
}

function Test-NewVersion {
    $TaskName = $AppName

    Write-UpdateLog -Level 'INFO' "Testing new version by restarting scheduled task..."

    try {
        # Stop the task if running
        Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue

        # Start the task
        Start-ScheduledTask -TaskName $TaskName

        # Wait a moment and check if it's running
        Start-Sleep -Seconds 5

        $task = Get-ScheduledTask -TaskName $TaskName
        $taskInfo = Get-ScheduledTaskInfo -TaskName $TaskName

        if ($taskInfo.LastTaskResult -eq 0 -or $task.State -eq 'Running') {
            Write-UpdateLog -Level 'SUCCESS' "New version is running successfully."
            return $true
        } else {
            Write-UpdateLog -Level 'ERROR' "New version failed to start. Last result: $($taskInfo.LastTaskResult)"
            return $false
        }
    } catch {
        Write-UpdateLog -Level 'ERROR' "Failed to test new version: $($_.Exception.Message)"
        return $false
    }
}

# --- Main Update Logic ---

Write-UpdateLog -Level 'INFO' "Starting automatic update check..."

# Check if updates are enabled (unless forced)
if (-not $Force -and -not (Test-UpdateEnabled)) {
    exit 0
}

# Get current and latest versions
$CurrentVersion = Get-CurrentVersion
$LatestVersion = Get-LatestVersion

if (-not $LatestVersion) {
    Write-UpdateLog -Level 'ERROR' "Failed to get latest version information."
    exit 1
}

Write-UpdateLog -Level 'INFO' "Current version: $CurrentVersion"
Write-UpdateLog -Level 'INFO' "Latest version: $LatestVersion"

# Check if update is needed
if (-not (Test-UpdateNeeded $CurrentVersion $LatestVersion)) {
    Write-UpdateLog -Level 'INFO' "No update needed. Current version is up to date."
    exit 0
}

Write-UpdateLog -Level 'INFO' "Update available. Starting update process..."

# Create backup
$BackupPath = New-Backup

# Stop the main task
Write-UpdateLog -Level 'INFO' "Stopping main task for update..."
try {
    Stop-ScheduledTask -TaskName $AppName -ErrorAction SilentlyContinue
} catch {
    Write-UpdateLog -Level 'WARN' "Failed to stop task, continuing anyway..."
}

# Download and install new version
if (Install-NewVersion $LatestVersion) {
    # Test new version
    if (Test-NewVersion) {
        Write-UpdateLog -Level 'SUCCESS' "Update completed successfully to version $LatestVersion"

        # Clean up old backups (keep last 5)
        $OldBackups = Get-ChildItem $BackupDir -Filter "$FinalExecutableName.*" | Sort-Object CreationTime -Descending | Select-Object -Skip 5
        $OldBackups | Remove-Item -Force
    } else {
        # Rollback on failure
        Write-UpdateLog -Level 'ERROR' "New version failed to start. Attempting rollback..."
        if (Restore-Backup $BackupPath) {
            Start-ScheduledTask -TaskName $AppName
            Write-UpdateLog -Level 'ERROR' "Update failed, but rollback was successful."
            exit 1
        } else {
            Write-UpdateLog -Level 'ERROR' "Update failed and rollback also failed. Manual intervention required."
            exit 1
        }
    }
} else {
    # Restart old version on download failure
    Write-UpdateLog -Level 'ERROR' "Failed to download new version. Restarting existing task..."
    Start-ScheduledTask -TaskName $AppName
    exit 1
}
