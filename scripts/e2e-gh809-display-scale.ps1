# Hosted disposable runner only. Select Windows' real display scaling control;
# registry values or browser zoom do not establish a monitor's effective DPI.
$ErrorActionPreference = 'Stop'
if ($env:CI -ne 'true') { throw 'Run this display-changing probe on a disposable CI runner only' }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$root = [System.Windows.Automation.AutomationElement]::RootElement
$tree = [System.Windows.Automation.TreeScope]::Descendants
$comboType = [System.Windows.Automation.ControlType]::ComboBox
$comboCondition = [System.Windows.Automation.PropertyCondition]::new(
  [System.Windows.Automation.AutomationElement]::ControlTypeProperty, $comboType)
$settingsCondition = [System.Windows.Automation.PropertyCondition]::new(
  [System.Windows.Automation.AutomationElement]::NameProperty, 'Settings')
Start-Process 'ms-settings:display'
$deadline = [DateTime]::UtcNow.AddSeconds(30)
$scale = $null
while ([DateTime]::UtcNow -lt $deadline) {
  $settings = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $settingsCondition)
  if ($null -eq $settings) { Start-Sleep -Milliseconds 250; continue }
  $combos = $settings.FindAll($tree, $comboCondition)
  foreach ($combo in $combos) {
    # UIA name varies between Windows client/server builds. Only the display
    # scale selector has a percentage among these Settings combo values.
    $value = $combo.Current.Name
    Write-Host "Display combo: $value ($($combo.Current.AutomationId))"
    if ($value -match '\d+%' -or $combo.Current.AutomationId -match '(^|_)SystemSettings_Display_Scal(e|ing)') {
      $scale = $combo
      break
    }
  }
  if ($null -ne $scale) { break }
  Start-Sleep -Milliseconds 250
}
if ($null -eq $scale) { throw 'Infrastructure: Windows Settings exposes no display scale selector' }
$expand = $scale.GetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern)
$expand.Expand()
$choice = $null
$deadline = [DateTime]::UtcNow.AddSeconds(10)
while ([DateTime]::UtcNow -lt $deadline) {
  $items = $root.FindAll($tree, [System.Windows.Automation.PropertyCondition]::new(
    [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
    [System.Windows.Automation.ControlType]::ListItem))
  foreach ($item in $items) {
    if ($item.Current.Name -match '^150%($|\s)') { $choice = $item; break }
  }
  if ($null -ne $choice) { break }
  Start-Sleep -Milliseconds 100
}
if ($null -eq $choice) { Write-Host "Scale choices offered: $(($items | ForEach-Object { $_.Current.Name }) -join ', ')"; throw 'Infrastructure: this hosted display does not offer 150% scaling' }
$choice.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
Write-Host 'Selected Windows display scale 150%; the app journey must verify native scale 1.5 on both windows.'
