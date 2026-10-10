# Smoke test for the Windows installer, status window and console handling.
#
# Starts the exe the way Explorer does (Start-Process = ShellExecute, so a console exe gets a
# fresh console window) and checks that every windowed entry point lets go of that console,
# i.e. the console only flashes. Also checks what the installer sets up, takes a screenshot of
# the status window and runs the uninstaller. Nothing is uploaded: the fake Steam install has an
# empty cache and no logged-in account.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$OutDir
)

$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$script:Failures = @()

function Test-Check([string]$Name, [scriptblock]$Condition) {
    if (& $Condition) {
        Write-Host "PASS  $Name"
    } else {
        Write-Host "FAIL  $Name"
        $script:Failures += $Name
    }
}

function Get-ConsoleHost([int]$ProcessId) {
    @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$ProcessId" |
        Where-Object { $_.Name -in @('conhost.exe', 'OpenConsole.exe') })
}

function Get-Ingest {
    @(Get-CimInstance Win32_Process -Filter "Name='deadlock-api-ingest.exe'")
}

# Starts like a double-click and returns how long (ms) its console window stayed up, or -1
# if it was still there after 10 s. The process must still be running afterwards.
function Measure-ConsoleFlash([string]$Path, [string[]]$Arguments) {
    $params = @{ FilePath = $Path; PassThru = $true }
    if ($Arguments) { $params.ArgumentList = $Arguments }
    $proc = Start-Process @params
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $sawConsole = $false
    while ($watch.ElapsedMilliseconds -lt 10000) {
        if ((Get-ConsoleHost $proc.Id).Count -gt 0) {
            $sawConsole = $true
        } elseif ($sawConsole -or $watch.ElapsedMilliseconds -gt 3000) {
            break
        }
        Start-Sleep -Milliseconds 50
    }
    $elapsed = if ((Get-ConsoleHost $proc.Id).Count -gt 0) { -1 } else { $watch.ElapsedMilliseconds }
    return [pscustomobject]@{ Process = $proc; Ms = $elapsed; SawConsole = $sawConsole }
}

function Save-Screenshot([string]$Name) {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $bitmap.Save((Join-Path $OutDir "$Name.png"))
    $graphics.Dispose()
    $bitmap.Dispose()
}

$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\deadlock-api-ingest'
$installDir = Join-Path $env:LOCALAPPDATA 'deadlock-api-ingest'
$installed = Join-Path $installDir 'deadlock-api-ingest.exe'
$startMenuLink = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Deadlock API Ingest.lnk'
$desktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Deadlock API Ingest.lnk'

# A Steam install without matches, so the watcher keeps running instead of giving up.
$steam = Join-Path $env:RUNNER_TEMP 'Steam'
New-Item -ItemType Directory -Force (Join-Path $steam 'appcache\httpcache') | Out-Null
New-Item -Path 'HKLM:\SOFTWARE\Wow6432Node\Valve\Steam' -Force | Out-Null
Set-ItemProperty 'HKLM:\SOFTWARE\Wow6432Node\Valve\Steam' -Name InstallPath -Value $steam

# --- Install: double-click the downloaded setup exe ------------------------------------------
$setup = Join-Path $env:RUNNER_TEMP 'deadlock-api-ingest-setup.exe'
Copy-Item $Exe $setup -Force
$run = Measure-ConsoleFlash $setup @()
Write-Host "setup: console visible for $($run.Ms) ms"
Test-Check 'setup releases its console' { $run.Ms -ge 0 }
Test-Check 'setup exits successfully' { $run.Process.WaitForExit(60000) -and $run.Process.ExitCode -eq 0 }

Start-Sleep -Seconds 10
Test-Check 'exe installed to %LOCALAPPDATA%' { Test-Path $installed }
Test-Check 'Start menu shortcut' { Test-Path $startMenuLink }
Test-Check 'desktop shortcut' { Test-Path $desktopLink }
Test-Check 'Apps & Features entry' { Test-Path $uninstallKey }
Test-Check 'autostart entry' { (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).'deadlock-api-ingest' }

$procs = Get-Ingest
$procs | ForEach-Object { Write-Host "running: $($_.ProcessId) $($_.CommandLine)" }
Test-Check 'status window running' { $procs | Where-Object { $_.CommandLine -like '*--gui*' } }
Test-Check 'watcher running' { $procs | Where-Object { $_.CommandLine -notlike '*--gui*' } }
foreach ($proc in $procs) {
    Test-Check "no console for $($proc.ProcessId)" { (Get-ConsoleHost $proc.ProcessId).Count -eq 0 }
}
Save-Screenshot '1-after-install'

# --- Shortcut and sign-in autostart, started fresh -------------------------------------------
Get-Process deadlock-api-ingest -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$watcher = Measure-ConsoleFlash $installed @()
Write-Host "autostart watcher: console visible for $($watcher.Ms) ms"
Test-Check 'autostart watcher releases its console' { $watcher.Ms -ge 0 }
Test-Check 'autostart watcher keeps running' { -not $watcher.Process.HasExited }

$gui = Measure-ConsoleFlash $installed @('--gui')
Write-Host "shortcut (--gui): console visible for $($gui.Ms) ms"
Test-Check 'status window releases its console' { $gui.Ms -ge 0 }
Start-Sleep -Seconds 5
Test-Check 'status window keeps running' { -not $gui.Process.HasExited }
Save-Screenshot '2-status-window'

# --- From a terminal, the console must stay usable --------------------------------------------
$version = & $installed --version
Test-Check 'CLI output still reaches the terminal' { "$version" -like 'deadlock-api-ingest *' }

# --- Uninstall (what Apps & Features runs) ----------------------------------------------------
$uninstall = Start-Process -FilePath $installed -ArgumentList '--uninstall' -PassThru
Start-Sleep -Seconds 8
Save-Screenshot '3-uninstall'
Test-Check 'autostart entry removed' { -not (Get-ItemProperty $runKey -ErrorAction SilentlyContinue).'deadlock-api-ingest' }
Test-Check 'Apps & Features entry removed' { -not (Test-Path $uninstallKey) }
Test-Check 'shortcuts removed' { -not (Test-Path $startMenuLink) -and -not (Test-Path $desktopLink) }
Test-Check 'other instances stopped' { -not (Get-Ingest | Where-Object { $_.ProcessId -ne $uninstall.Id }) }

# Confirm the "uninstalled" message box; the install folder is removed after that.
$shell = New-Object -ComObject WScript.Shell
if ($shell.AppActivate('Deadlock API Ingest')) { $shell.SendKeys('{ENTER}') }
if ($uninstall.WaitForExit(10000)) {
    Start-Sleep -Seconds 5
    if (Test-Path $installDir) { Write-Host "::warning::install folder still present after uninstall" }
} else {
    Write-Host "::warning::could not confirm the uninstall message box"
    $uninstall | Stop-Process -Force
}

if ($script:Failures.Count -gt 0) {
    Write-Host "::error::Windows smoke test failed: $($script:Failures -join ', ')"
    exit 1
}
Write-Host 'All checks passed.'
