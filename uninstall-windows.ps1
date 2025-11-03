# Stop and remove scheduled tasks (main, watchdog, and updater)
Stop-ScheduledTask -TaskName "deadlock-api-ingest" -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName "deadlock-api-ingest" -Confirm:$false -ErrorAction SilentlyContinue

Stop-ScheduledTask -TaskName "deadlock-api-ingest-Watchdog" -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName "deadlock-api-ingest-Watchdog" -Confirm:$false -ErrorAction SilentlyContinue

Stop-ScheduledTask -TaskName "deadlock-api-ingest-updater" -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName "deadlock-api-ingest-updater" -Confirm:$false -ErrorAction SilentlyContinue

# Stop any running process (if still running)
Stop-Process -Name "deadlock-api-ingest" -Force -ErrorAction SilentlyContinue

# Remove desktop shortcuts (if created)
Remove-Item "$env:Public\Desktop\deadlock-api-ingest.lnk" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:Public\Desktop\deadlock-api-ingest (Once).lnk" -Force -ErrorAction SilentlyContinue

# Remove installation directory and related data
Remove-Item "$env:ProgramFiles\deadlock-api-ingest" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "$env:ProgramData\deadlock-api-ingest" -Recurse -Force -ErrorAction SilentlyContinue