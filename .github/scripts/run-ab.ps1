# GH #623 Defender A/B driver. Phases OFF1 -> ON -> OFF2, 3 reps each. Per rep:
#   cold  (fresh unseen graph copy, no checkpoint; writes the checkpoint)
#   warm  (same copy, from the checkpoint)
#   prims (second fresh unseen copy: raw list/stat/open/read primitives, first then second touch)
param([int]$Reps = 3)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/defender.ps1"
$exe = (Resolve-Path "target/release/examples/defender_ab.exe").Path
$base = Join-Path $env:USERPROFILE 'ellis'     # C:\Users\runneradmin\ellis (where a user's graph lives)
$master = Join-Path $base 'master'
$results = Join-Path $PWD 'results.jsonl'
New-Item -ItemType Directory -Force "$base\ckpt" | Out-Null
Remove-Item $results -ErrorAction SilentlyContinue

function Log($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }

# 0. State as shipped + pre-copy all graphs while Defender is still OFF so the ON pass reads files
#    Defender has never seen (verdicts are cached per file).
Log "Defender as shipped"
$proof0 = Show-DefenderProof 'as shipped'
Log "Copying graphs (Defender as shipped = off)"
$phases = 'OFF1','ON1','OFF2','ON2'
foreach ($ph in $phases) { foreach ($r in 1..$Reps) { foreach ($k in 'g','p') {
  $dst = Join-Path $base "$k-$ph-$r"
  robocopy $master $dst /E /MT:16 /NFL /NDL /NJH /NJS /NP | Out-Null
  if ($LASTEXITCODE -ge 8) { throw "robocopy failed $LASTEXITCODE" }
}}}
$fileCount = (Get-ChildItem -Recurse -File (Join-Path $base 'g-OFF1-1') | Measure-Object).Count
Log "copies done; files per copy: $fileCount; drive: $((Get-Item $base).PSDrive.Name):"

function Run-Tool([string]$phase, [int]$rep, [string]$mode, [string]$graph, [string]$ckpt) {
  $av = Get-Process MsMpEng -ErrorAction SilentlyContinue; $cpu0 = if ($av) { $av.TotalProcessorTime.TotalMilliseconds } else { 0 }
  $t = Get-Date
  $raw = if ($mode -eq 'prims') { & $exe prims $graph } else { & $exe $mode $graph $ckpt }
  $wall = ((Get-Date) - $t).TotalMilliseconds
  $av = Get-Process MsMpEng -ErrorAction SilentlyContinue; $cpu1 = if ($av) { $av.TotalProcessorTime.TotalMilliseconds } else { 0 }
  $json = ($raw | Where-Object { $_ -like '{*' } | Select-Object -Last 1)
  if (-not $json) { throw "no JSON from $mode : $raw" }
  $line = '{"phase":"' + $phase + '","rep":' + $rep + ',"mode":"' + $mode + '","processWallMs":' + [math]::Round($wall,1) + ',"result":' + $json + '}'
  Add-Content -Path $results -Value $line -Encoding utf8
  Log "$phase rep$rep $mode done (process wall $([math]::Round($wall)) ms)"
}

foreach ($ph in $phases) {
  if ($ph -like 'ON*') { Enable-DefenderForBench } else { Disable-DefenderForBench }
  Start-Sleep -Seconds 60   # settle: lingering scan backlog after a toggle polluted OFF2 in run 1
  $proof = Show-DefenderProof $ph
  Add-Content -Path $results -Value (('{"phase":"' + $ph + '","mode":"proof","proof":') + ($proof | ConvertTo-Json -Compress) + '}') -Encoding utf8
  foreach ($r in 1..$Reps) {
    $g = Join-Path $base "g-$ph-$r"; $p = Join-Path $base "p-$ph-$r"; $c = "$base\ckpt\$ph-$r.bin"
    $pre = Get-Process MsMpEng -ErrorAction SilentlyContinue
    Log "pre-rep MsMpEng cpu s: $([math]::Round($pre.TotalProcessorTime.TotalSeconds,1)); system cpu%: $((Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average)"
    Run-Tool $ph $r 'cold' $g $c
    # second cold launch on the SAME copy (checkpoint removed): files already scanned, i.e. a user's repeat launch
    Remove-Item $c -Force -ErrorAction SilentlyContinue
    Run-Tool $ph $r 'cold2' $g $c
    Run-Tool $ph $r 'warm' $g $c
    Run-Tool $ph $r 'prims' $p ''
  }
}
Get-Content $results | Select-Object -First 3 | ForEach-Object { $_.Substring(0, [math]::Min(400, $_.Length)) }
