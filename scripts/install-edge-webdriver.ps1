# Installs the Edge WebDriver that matches the WebView2 runtime and puts it on the job's PATH.
# The app runs in the WebView2 runtime, which a hosted image can carry at another version than the
# Edge browser; a driver chosen by the browser's version then refuses to create a session.
$ErrorActionPreference = 'Stop'
$client = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$version = @("HKLM:\SOFTWARE\WOW6432Node\$client", "HKLM:\SOFTWARE\$client", "HKCU:\SOFTWARE\$client") |
  ForEach-Object { (Get-ItemProperty -Path $_ -Name pv -ErrorAction SilentlyContinue).pv } |
  Where-Object { $_ -and $_ -ne '0.0.0.0' } | Select-Object -First 1
if (-not $version) { throw 'WebView2 runtime not found in the EdgeUpdate registry' }
$driverDir = "$env:RUNNER_TEMP\edgedriver"
$zip = "$env:RUNNER_TEMP\edgedriver.zip"
Invoke-WebRequest "https://msedgedriver.microsoft.com/$version/edgedriver_win64.zip" -OutFile $zip
Expand-Archive $zip -DestinationPath $driverDir -Force
$driverDir | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
"WebView2 runtime $version"
& "$driverDir\msedgedriver.exe" --version
