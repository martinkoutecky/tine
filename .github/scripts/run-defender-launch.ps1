# GH #623: cold/warm launch of an Ellis-shaped graph with Windows Defender ON (shipped-Windows
# defaults), measured on a hosted runner. Hosted images ship Defender in passive mode, which is why
# an ordinary CI run says nothing about a user's launch. Derived from the A/B lane
# (branch diagnose/gh623-defender); the OFF control phases were dropped. Per rep:
#   cold  (fresh graph copy Defender has never scanned, no checkpoint; writes the checkpoint)
#   cold2 (same copy, checkpoint removed: files already scanned, a user's repeat launch)
#   warm  (same copy, from the checkpoint)
#   prims (second fresh copy: raw list/stat/open/read primitives, first then second touch)
# No budget is enforced: the numbers are reported (results.jsonl, summary), and the job's only
# assertion is that Defender really is ON before measuring.
param([int]$Reps = 3)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/defender.ps1"
$exe = (Resolve-Path "target/release/examples/defender_ab.exe").Path
$base = Join-Path $env:USERPROFILE 'ellis'
$master = Join-Path $base 'master'
$results = Join-Path $PWD 'results.jsonl'
New-Item -ItemType Directory -Force "$base\ckpt" | Out-Null
Remove-Item $results -ErrorAction SilentlyContinue

function Log($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }

# Copy every graph while Defender is still as shipped (passive), so the ON pass reads files Defender
# has never scanned (a verdict is cached per file; a user's first launch after a sync is unseen).
Log "Copying graphs (Defender as shipped)"
foreach ($r in 1..$Reps) { foreach ($k in 'g','p') {
  $dst = Join-Path $base "$k-ON-$r"
  robocopy $master $dst /E /MT:16 /NFL /NDL /NJH /NJS /NP | Out-Null
  if ($LASTEXITCODE -ge 8) { throw "robocopy failed $LASTEXITCODE" }
}}
$fileCount = (Get-ChildItem -Recurse -File (Join-Path $base 'g-ON-1') | Measure-Object).Count
Log "copies done; files per copy: $fileCount"

Enable-DefenderForBench
Start-Sleep -Seconds 60   # settle: a scan backlog after the toggle polluted the first measurement
$proof = Show-DefenderProof 'ON'
Add-Content -Path $results -Value (('{"phase":"ON","mode":"proof","proof":') + ($proof | ConvertTo-Json -Compress) + '}') -Encoding utf8
# The one assertion: a measurement with Defender not really on would be a silent lie.
$on = ($proof['AMRunningMode'] -eq 'Normal') -and $proof['RealTimeProtectionEnabled'] -and $proof['OnAccessProtectionEnabled'] -and $proof['EicarBlocked']
if (-not $on) {
  Write-Host "::error::Defender is not ON (AMRunningMode=$($proof['AMRunningMode']) RealTime=$($proof['RealTimeProtectionEnabled']) OnAccess=$($proof['OnAccessProtectionEnabled']) EicarBlocked=$($proof['EicarBlocked'])); the numbers below were NOT measured with Defender on"
  exit 1
}

function Run-Tool([int]$rep, [string]$label, [string]$mode, [string]$graph, [string]$ckpt) {
  $av = Get-Process MsMpEng -ErrorAction SilentlyContinue; $cpu0 = if ($av) { $av.TotalProcessorTime.TotalMilliseconds } else { 0 }
  $t = Get-Date
  $raw = if ($mode -eq 'prims') { & $exe prims $graph } else { & $exe $mode $graph $ckpt }
  $wall = ((Get-Date) - $t).TotalMilliseconds
  $av = Get-Process MsMpEng -ErrorAction SilentlyContinue; $cpu1 = if ($av) { $av.TotalProcessorTime.TotalMilliseconds } else { 0 }
  $json = ($raw | Where-Object { $_ -like '{*' } | Select-Object -Last 1)
  if (-not $json) { throw "no JSON from $mode : $raw" }
  $line = '{"phase":"ON","rep":' + $rep + ',"mode":"' + $label + '","processWallMs":' + [math]::Round($wall,1) + ',"msMpEngCpuMs":' + [math]::Round($cpu1 - $cpu0,1) + ',"result":' + $json + '}'
  Add-Content -Path $results -Value $line -Encoding utf8
  Log "rep$rep $label done (process wall $([math]::Round($wall)) ms)"
}

foreach ($r in 1..$Reps) {
  $g = Join-Path $base "g-ON-$r"; $p = Join-Path $base "p-ON-$r"; $c = "$base\ckpt\ON-$r.bin"
  Run-Tool $r 'cold' 'cold' $g $c
  Remove-Item $c -Force -ErrorAction SilentlyContinue
  Run-Tool $r 'cold2' 'cold' $g $c
  Run-Tool $r 'warm' 'warm' $g $c
  Run-Tool $r 'prims' 'prims' $p ''
}
