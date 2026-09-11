# Recover salts for your own Deadlock matches via the Steam Game Coordinator.
# Usage: irm https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/fetch-own-matches.ps1 | iex
$ErrorActionPreference = 'Stop'
# The progress bar makes Invoke-WebRequest extremely slow on Windows PowerShell 5.1.
$ProgressPreference = 'SilentlyContinue'

$bin = Join-Path $env:TEMP 'deadlock-api-ingest-own-matches.exe'
Invoke-WebRequest -UseBasicParsing -OutFile $bin -Uri 'https://github.com/deadlock-api/deadlock-api-ingest/releases/latest/download/deadlock-api-ingest-windows-latest.exe'
$prevRustLog = $env:RUST_LOG
$env:RUST_LOG = 'info'
try {
    & $bin --own-matches
} finally {
    $env:RUST_LOG = $prevRustLog
    Remove-Item $bin -ErrorAction SilentlyContinue
}
