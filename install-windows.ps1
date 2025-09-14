# Deadlock API Ingest - Windows Installation Script
# This script downloads and installs the deadlock-api-ingest application as a Windows Service.

# --- Configuration ---
$AppName = "deadlock-api-ingest"
$GithubRepo = "deadlock-api/deadlock-api-ingest"
$AssetKeyword = "windows-latest.exe" # Keyword to find in the release asset filename

# Installation Paths
$InstallDir = "$env:ProgramFiles\$AppName"
$FinalExecutableName = "$AppName.exe"
$LogFile = "$env:TEMP\${AppName}-install.log"

# --- Script Setup ---
# Stop on any error
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

    $ColorMap = @{
        'INFO'    = 'Cyan'
        'WARN'    = 'Yellow'
        'ERROR'   = 'Red'
        'SUCCESS' = 'Green'
    }
    $Color = $ColorMap[$Level]
    $Timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $LogMessage = "[$Timestamp] [$Level] $Message"

    # Write to console
    Write-Host $LogMessage -ForegroundColor $Color

    # Write to log file
    Add-Content -Path $LogFile -Value $LogMessage
}

# Function to check for Administrator privileges and re-launch if needed
function Test-IsAdmin {
    $currentUser = [Security.Principal.WindowsIdentity]::GetCurrent()
    $isAdmin = (New-Object Security.Principal.WindowsPrincipal $currentUser).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

    if (-not $isAdmin) {
        Write-Log -Level 'ERROR' "This script requires Administrator privileges. Please re-run as Administrator."
        # Optional: Attempt to re-launch automatically
        # Write-Log -Level 'INFO' "Attempting to re-launch with elevated privileges..."
        # Start-Process powershell -Verb RunAs -ArgumentList "-NoProfile -File `"$($MyInvocation.MyCommand.Path)`""
        exit 1
    }
    Write-Log -Level 'INFO' "Running with Administrator privileges."
}

# Function to check for dependencies
function Test-Dependencies {
    Write-Log -Level 'INFO' "Checking for Npcap dependency (required for libpcap)..."
    $npcapService = Get-Service -Name "npcap" -ErrorAction SilentlyContinue
    $npcapDriver = Get-Item -Path "$env:SystemRoot\System32\drivers\npcap.sys" -ErrorAction SilentlyContinue

    if ($npcapService -and $npcapDriver) {
        Write-Log -Level 'SUCCESS' "Npcap is already installed."
    }
    else {
        Write-Log -Level 'ERROR' "Npcap is not installed. This is required for network capture."
        Write-Log -Level 'INFO' "Please download and install Npcap from: https://npcap.com"
        exit 1
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

# Function to manage the Windows Service
function Manage-Service {
    param(
        [Parameter(Mandatory = $true)]
        [ValidateSet('Remove', 'Create', 'Start')]
        [string]$Action,
        [string]$ExecutablePath
    )

    $service = Get-Service -Name $AppName -ErrorAction SilentlyContinue

    switch ($Action) {
        'Remove' {
            if ($service) {
                Write-Log -Level 'INFO' "Stopping and removing existing '$AppName' service..."
                try {
                    Stop-Service -Name $AppName -Force
                } catch {}
                sc.exe delete $AppName | Out-Null
                Write-Log -Level 'SUCCESS' "Existing service removed."
                Start-Sleep -Seconds 2 # Allow time for service to be fully removed
            }
        }
        'Create' {
            Write-Log -Level 'INFO' "Creating new Windows Service for '$AppName'..."
            New-Service -Name $AppName -BinaryPathName "`"$ExecutablePath`"" -DisplayName "Deadlock API Ingest" -StartupType Automatic
            sc.exe description $AppName "Captures and forwards network data to the Deadlock API." | Out-Null
            Write-Log -Level 'SUCCESS' "Service created successfully."
        }
        'Start' {
            Write-Log -Level 'INFO' "Starting the '$AppName' service..."
            Start-Service -Name $AppName
            Start-Sleep -Seconds 3
            $service = Get-Service -Name $AppName
            if ($service.Status -eq 'Running') {
                Write-Log -Level 'SUCCESS' "Service started successfully and is now running."
            }
            else {
                Write-Log -Level 'ERROR' "Service failed to start. Current status: $($service.Status)"
                exit 1
            }
        }
    }
}

# --- Main Installation Logic ---

# Clear previous log file for a clean run
Clear-Content -Path $LogFile -ErrorAction SilentlyContinue

Write-Log -Level 'INFO' "Starting Deadlock API Ingest installation..."
Write-Log -Level 'INFO' "Log file is available at: $LogFile"

Test-IsAdmin
Test-Dependencies

$release = Get-LatestRelease

Manage-Service -Action 'Remove'

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

# Unblock file downloaded from the internet
Unblock-File -Path $downloadPath

Manage-Service -Action 'Create' -ExecutablePath $downloadPath
Manage-Service -Action 'Start'

Write-Host " "
Write-Log -Level 'SUCCESS' "🚀 Deadlock API Ingest ($($release.Version)) has been installed successfully!"
Write-Host " "
Write-Host "You can manage the service with the following PowerShell commands:" -ForegroundColor White
Write-Host "  - Check status:  Get-Service -Name $AppName" -ForegroundColor Yellow
Write-Host "  - Stop service:  Stop-Service -Name $AppName" -ForegroundColor Yellow
Write-Host "  - Start service: Start-Service -Name $AppName" -ForegroundColor Yellow
Write-Host " "
