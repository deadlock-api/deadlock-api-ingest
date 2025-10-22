# Deadlock API Ingest - Update Checker Script
# This script checks for new releases and updates the application automatically

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
$StartupShortcutPath = "$InstallDir\${AppName}-startup.lnk"
$RegistryRunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"

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
            # Ignore if we can't create event source
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
        # Ignore if we can't write to event log
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

function Stop-Application {
    Write-UpdateLog -Level 'INFO' "Stopping application..."

    try {
        # Try to stop the process by name
        $processes = Get-Process -Name $AppName -ErrorAction SilentlyContinue
        if ($processes) {
            $processes | Stop-Process -Force
            Write-UpdateLog -Level 'INFO' "Application process stopped."
        } else {
            Write-UpdateLog -Level 'INFO' "No running application process found."
        }

        # Also try to stop old scheduled task if it exists (for migration)
        Stop-ScheduledTask -TaskName $AppName -ErrorAction SilentlyContinue

        return $true
    } catch {
        Write-UpdateLog -Level 'WARN' "Failed to stop application: $($_.Exception.Message)"
        return $false
    }
}

function Start-Application {
    Write-UpdateLog -Level 'INFO' "Starting application..."

    try {
        $ExecutablePath = Join-Path -Path $InstallDir -ChildPath $FinalExecutableName

        # Check if auto-start is enabled via registry
        $regValue = Get-ItemProperty -Path $RegistryRunKey -Name $AppName -ErrorAction SilentlyContinue

        if ($regValue) {
            # Auto-start is enabled, start in background with admin privileges
            $startInfo = New-Object System.Diagnostics.ProcessStartInfo
            $startInfo.FileName = $ExecutablePath
            $startInfo.WorkingDirectory = $InstallDir
            $startInfo.Verb = "runas"  # Run as administrator
            $startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
            $startInfo.CreateNoWindow = $true

            $process = New-Object System.Diagnostics.Process
            $process.StartInfo = $startInfo
            $process.Start() | Out-Null

            Write-UpdateLog -Level 'INFO' "Application started in background with admin privileges."
        } else {
            # Check for old scheduled task (for migration)
            $task = Get-ScheduledTask -TaskName $AppName -ErrorAction SilentlyContinue
            if ($task) {
                Start-ScheduledTask -TaskName $AppName
                Write-UpdateLog -Level 'INFO' "Application started via scheduled task (legacy)."
            } else {
                # No auto-start configured, start in background anyway
                $startInfo = New-Object System.Diagnostics.ProcessStartInfo
                $startInfo.FileName = $ExecutablePath
                $startInfo.WorkingDirectory = $InstallDir
                $startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
                $startInfo.CreateNoWindow = $true

                $process = New-Object System.Diagnostics.Process
                $process.StartInfo = $startInfo
                $process.Start() | Out-Null

                Write-UpdateLog -Level 'INFO' "Application started in background (no auto-start configured)."
            }
        }

        return $true
    } catch {
        Write-UpdateLog -Level 'ERROR' "Failed to start application: $($_.Exception.Message)"
        return $false
    }
}

function Test-NewVersion {
    Write-UpdateLog -Level 'INFO' "Testing new version by restarting application..."

    try {
        # Stop the application
        Stop-Application

        # Wait a moment
        Start-Sleep -Seconds 2

        # Start the application
        if (Start-Application) {
            # Wait a moment and check if it's running
            Start-Sleep -Seconds 5

            $process = Get-Process -Name $AppName -ErrorAction SilentlyContinue
            if ($process) {
                Write-UpdateLog -Level 'SUCCESS' "New version is running successfully."
                return $true
            } else {
                Write-UpdateLog -Level 'ERROR' "New version failed to start - process not found."
                return $false
            }
        } else {
            Write-UpdateLog -Level 'ERROR' "Failed to start new version."
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

# Stop the application
Stop-Application

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
            Start-Application
            Write-UpdateLog -Level 'ERROR' "Update failed, but rollback was successful."
            exit 1
        } else {
            Write-UpdateLog -Level 'ERROR' "Update failed and rollback also failed. Manual intervention required."
            exit 1
        }
    }
} else {
    # Restart old version on download failure
    Write-UpdateLog -Level 'ERROR' "Failed to download new version. Restarting application..."
    Start-Application
    exit 1
}
