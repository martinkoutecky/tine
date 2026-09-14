$ErrorActionPreference = "Stop"
$fixtureId = "tine-smb-533-$([guid]::NewGuid().ToString('N'))"
$shareName = "tine533$([guid]::NewGuid().ToString('N').Substring(0, 10))"
$shareRoot = Join-Path $env:RUNNER_TEMP $shareName
$graphRoot = Join-Path $shareRoot $fixtureId
$drive = "Z:"
$createdShare = $false
$createdMapping = $false
New-Item -ItemType Directory -Path $graphRoot | Out-Null
$outside = Join-Path $shareRoot "outside-control"
New-Item -ItemType Directory -Path $outside | Out-Null
New-Item -ItemType Junction -Path (Join-Path $graphRoot "outside-link") -Target $outside | Out-Null
try {
    if (Get-PSDrive -Name Z -ErrorAction SilentlyContinue) { throw "Z: already occupied" }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    Write-Output "Windows: $([Environment]::OSVersion.VersionString); identity: $identity"
    New-SmbShare -Name $shareName -Path $shareRoot -FullAccess $identity | Format-List
    $createdShare = $true
    $unc = "\\localhost\$shareName"
    New-SmbMapping -LocalPath $drive -RemotePath $unc -Persistent $false | Format-List
    $createdMapping = $true
    Get-SmbConnection | Format-List ServerName,ShareName,Dialect,Encrypted,Signed
    & cargo run --locked --manifest-path scripts/probes/windows-smb-open/Cargo.toml -- $graphRoot "$unc\$fixtureId" "Z:\$fixtureId"
    if ($LASTEXITCODE -ne 0) { throw "SMB probe failed with exit $LASTEXITCODE" }
} finally {
    if ($createdMapping) { Remove-SmbMapping -LocalPath $drive -Force -UpdateProfile -ErrorAction Continue }
    if ($createdShare) { Remove-SmbShare -Name $shareName -Force -ErrorAction Continue }
    Remove-Item -LiteralPath $shareRoot -Recurse -Force -ErrorAction Continue
}
