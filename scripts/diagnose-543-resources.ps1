param([string]$OutputFile, [string]$CacheRoot, [int]$ProcessId = 0)
# Diagnostic resource sampler for the Windows measurement probes: one JSON line
# per second with the Tine host process's I/O transfer counts, CPU times and
# working set. With -ProcessId it follows that one process and stops when it
# exits; without it, every tine.exe is sampled.
$ErrorActionPreference = 'Continue'
while ($true) {
    if ($ProcessId -gt 0) {
        $processes = @(Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" | Select-Object ProcessId, KernelModeTime, UserModeTime, ReadTransferCount, WriteTransferCount, WorkingSetSize, PeakWorkingSetSize, PageFileUsage, PeakPageFileUsage)
        if ($processes.Count -eq 0) { break }
    } else {
        $processes = @(Get-CimInstance Win32_Process -Filter "name='tine.exe'" | Select-Object ProcessId, KernelModeTime, UserModeTime, ReadTransferCount, WriteTransferCount, WorkingSetSize, PeakWorkingSetSize, PageFileUsage, PeakPageFileUsage)
    }
    @{ time = [DateTime]::UtcNow.ToString('o'); processes = $processes } | ConvertTo-Json -Depth 5 -Compress | Add-Content -LiteralPath $OutputFile
    Start-Sleep -Seconds 1
}
