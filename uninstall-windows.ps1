# Deadlock API Ingest - Uninstall Script
# This script removes the application and all related components

param(
    [switch]$Silent
)

$AppName = "deadlock-api-ingest"
$InstallDir = Join-Path -Path $env:LOCALAPPDATA -ChildPath $AppName

if (-not $Silent) {
    Write-Host ""
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host "  Deadlock API Ingest - Uninstaller" -ForegroundColor Cyan
    Write-Host "========================================" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "This will remove:" -ForegroundColor Yellow
    Write-Host "  - All scheduled tasks (current and old versions)" -ForegroundColor White
    Write-Host "  - Running processes" -ForegroundColor White
    Write-Host "  - Desktop shortcuts" -ForegroundColor White
    Write-Host "  - Installation directory: $InstallDir" -ForegroundColor White
    Write-Host ""
    
    $confirmation = Read-Host "Continue with uninstallation? (Y/N)"
    if ($confirmation -ne 'Y' -and $confirmation -ne 'y') {
        Write-Host "Uninstallation cancelled." -ForegroundColor Yellow
        exit 0
    }
    Write-Host ""
}

Write-Host "Uninstalling Deadlock API Ingest..." -ForegroundColor Yellow
Write-Host ""

# Stop and remove all scheduled tasks (current and old versions)
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

# Remove desktop shortcuts
Write-Host "Removing desktop shortcuts..." -ForegroundColor Cyan
$shortcuts = Get-Item "$env:USERPROFILE\Desktop\$AppName*.lnk" -ErrorAction SilentlyContinue
$oneDriveShortcuts = Get-Item "$env:USERPROFILE\OneDrive\Desktop\$AppName*.lnk" -ErrorAction SilentlyContinue
if ($shortcuts || $oneDriveShortcuts) {
    Write-Host "  - Removing $(@($shortcuts).Count) shortcut(s)" -ForegroundColor Gray
}

if ($shortcuts) {
    Remove-Item "$env:USERPROFILE\Desktop\$AppName*.lnk" -Force -ErrorAction SilentlyContinue
}
if ($oneDriveShortcuts) {
    Remove-Item "$env:USERPROFILE\OneDrive\Desktop\$AppName*.lnk" -Force -ErrorAction SilentlyContinue
}

# Remove installation directory
Write-Host "Removing installation directory..." -ForegroundColor Cyan
if (Test-Path $InstallDir) {
    Write-Host "  - Removing: $InstallDir" -ForegroundColor Gray
    Set-Location $env:TEMP
    Start-Sleep -Seconds 1
    Remove-Item $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host "  Uninstallation Complete!" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
Write-Host ""

if (-not $Silent) {
    Write-Host "Press any key to exit..." -ForegroundColor Gray
    $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
}
