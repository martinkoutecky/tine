param([string]$OutputFile, [string]$CacheRoot)
$ErrorActionPreference = 'Continue'
while ($true) {
    $processes = @(Get-CimInstance Win32_Process -Filter "name='tine.exe'" | Select-Object ProcessId, KernelModeTime, UserModeTime, ReadTransferCount, WriteTransferCount, WorkingSetSize, PageFileUsage, PeakPageFileUsage)
    $files = @(Get-ChildItem -LiteralPath $CacheRoot -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'sqlite|projection|query' } | Select-Object Name, Length)
    @{ time = [DateTime]::UtcNow.ToString('o'); processes = $processes; files = $files } | ConvertTo-Json -Depth 5 -Compress | Add-Content -LiteralPath $OutputFile
    Start-Sleep -Seconds 2
}
