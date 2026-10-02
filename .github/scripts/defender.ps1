# Defender on/off for the GH #623 A/B. OFF = the hosted-runner image state
# (actions/runner-images Configure-WindowsDefender.ps1); ON = shipped-Windows defaults
# for everything that touches file I/O.
$ErrorActionPreference = 'Continue'
$atp = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows Advanced Threat Protection'

function Enable-DefenderForBench {
  if (Test-Path $atp) {
    Remove-ItemProperty -Path $atp -Name ForceDefenderPassiveMode -ErrorAction SilentlyContinue
  }
  foreach ($k in 'HKLM:\SOFTWARE\Policies\Microsoft\Windows Defender','HKLM:\SOFTWARE\Policies\Microsoft\Windows Defender\Real-Time Protection') {
    foreach ($n in 'DisableAntiSpyware','DisableAntiVirus','DisableRealtimeMonitoring','DisableBehaviorMonitoring','DisableIOAVProtection','DisableOnAccessProtection','DisableScriptScanning') {
      if (Test-Path $k) { Remove-ItemProperty -Path $k -Name $n -ErrorAction SilentlyContinue }
    }
  }
  Set-Service WinDefend -StartupType Automatic -ErrorAction SilentlyContinue
  Start-Service WinDefend -ErrorAction SilentlyContinue
  Remove-MpPreference -ExclusionPath 'C:\','D:\' -ErrorAction Continue
  $on = @(
    @{DisableRealtimeMonitoring = $false}
    @{DisableIOAVProtection = $false}
    @{DisableBehaviorMonitoring = $false}
    @{DisableScriptScanning = $false}
    @{DisableArchiveScanning = $false}
    @{DisableAutoExclusions = $false}
    @{DisableIntrusionPreventionSystem = $false}
    @{DisableScanningNetworkFiles = $false}
    @{DisableBlockAtFirstSeen = $false}
    @{DisablePrivacyMode = $false}
    @{MAPSReporting = 2}
    @{ScanAvgCPULoadFactor = 50}
  )
  foreach ($p in $on) { try { Set-MpPreference @p -ErrorAction Stop } catch { Write-Host "Set-MpPreference $($p.Keys) failed: $_" } }
  Start-Sleep -Seconds 5
}

function Disable-DefenderForBench {
  $off = @(
    @{DisableArchiveScanning = $true}
    @{DisableAutoExclusions = $true}
    @{DisableBehaviorMonitoring = $true}
    @{DisableIntrusionPreventionSystem = $true}
    @{DisableIOAVProtection = $true}
    @{DisableScanningNetworkFiles = $true}
    @{DisableScriptScanning = $true}
    @{MAPSReporting = 0}
    @{DisableBlockAtFirstSeen = $true}
    @{ScanAvgCPULoadFactor = 5; ExclusionPath = @('D:\','C:\')}
    @{DisableRealtimeMonitoring = $true}
  )
  foreach ($p in $off) { try { Set-MpPreference @p -ErrorAction Stop } catch { Write-Host "Set-MpPreference $($p.Keys) failed: $_" } }
  if (Test-Path $atp) { Set-ItemProperty -Path $atp -Name ForceDefenderPassiveMode -Value 1 -Type DWord }
  Start-Sleep -Seconds 5
}

function Show-DefenderProof([string]$Label = '') {
  Write-Host "=== Defender proof $Label ==="
  Get-Service WinDefend -ErrorAction SilentlyContinue | Format-Table Name,Status,StartType | Out-String | Write-Host
  $s = Get-MpComputerStatus
  $p = Get-MpPreference
  $o = [ordered]@{
    AMRunningMode = $s.AMRunningMode
    RealTimeProtectionEnabled = $s.RealTimeProtectionEnabled
    OnAccessProtectionEnabled = $s.OnAccessProtectionEnabled
    IoavProtectionEnabled = $s.IoavProtectionEnabled
    BehaviorMonitorEnabled = $s.BehaviorMonitorEnabled
    AntivirusEnabled = $s.AntivirusEnabled
    AMServiceEnabled = $s.AMServiceEnabled
    IsTamperProtected = $s.IsTamperProtected
    AntivirusSignatureVersion = $s.AntivirusSignatureVersion
    DisableRealtimeMonitoring = $p.DisableRealtimeMonitoring
    DisableIOAVProtection = $p.DisableIOAVProtection
    DisableBehaviorMonitoring = $p.DisableBehaviorMonitoring
    DisableScriptScanning = $p.DisableScriptScanning
    DisableArchiveScanning = $p.DisableArchiveScanning
    MAPSReporting = $p.MAPSReporting
    ExclusionPath = ($p.ExclusionPath -join ';')
    ExclusionProcess = ($p.ExclusionProcess -join ';')
    ExclusionExtension = ($p.ExclusionExtension -join ';')
    ForceDefenderPassiveMode = (Get-ItemProperty $atp -ErrorAction SilentlyContinue).ForceDefenderPassiveMode
  }
  $o.GetEnumerator() | ForEach-Object { Write-Host ("{0,-28} {1}" -f $_.Key, $_.Value) }
  # Behavioural proof: EICAR must be blocked/removed by real-time protection (written from
  # pieces so this script file is not itself flagged). Harmless 68-byte test string.
  $eicar = 'X5O!P%@AP[4\PZX54(P^)7CC)7}$' + 'EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*'
  $f = Join-Path $env:TEMP "eicar-probe.txt"
  Remove-Item $f -Force -ErrorAction SilentlyContinue
  try { [IO.File]::WriteAllText($f, $eicar) } catch { Write-Host "EICAR write blocked: $($_.Exception.Message)" }
  Start-Sleep -Seconds 4
  $present = Test-Path $f
  $readable = $false
  if ($present) { try { $null = [IO.File]::ReadAllText($f); $readable = $true } catch { Write-Host "EICAR read blocked: $($_.Exception.Message)" } }
  Write-Host "EICAR probe: fileStillPresent=$present readable=$readable  (ON expected: absent or unreadable)"
  Remove-Item $f -Force -ErrorAction SilentlyContinue
  $o['EicarBlocked'] = (-not $present) -or (-not $readable)
  return $o
}
