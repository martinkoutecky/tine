# Deterministic raster resizing from the existing icon; no new artwork.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$source = [System.Drawing.Image]::FromFile((Resolve-Path src-tauri/icons/icon.png))
try {
    foreach ($arch in @('x64', 'arm64')) {
        $assets = "msix-stage/$arch/Assets"
        $variants = @{}
        foreach ($entry in @(@('Square44x44Logo', 44), @('Square150x150Logo', 150), @('StoreLogo', 50))) {
            foreach ($scale in @(100, 125, 150, 200, 400)) {
                $variants["$($entry[0]).scale-$scale.png"] = [int][Math]::Round($entry[1] * $scale / 100, [MidpointRounding]::AwayFromZero)
            }
        }
        foreach ($size in @(16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256)) {
            foreach ($form in @('', '_altform-unplated', '_altform-lightunplated')) {
                $variants["Square44x44Logo.targetsize-$size$form.png"] = $size
            }
        }
        foreach ($name in $variants.Keys) {
            $size = $variants[$name]
            $bitmap = [System.Drawing.Bitmap]::new($size, $size)
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.Clear([System.Drawing.Color]::Transparent)
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.DrawImage($source, 0, 0, $size, $size)
                $bitmap.Save((Join-Path (Resolve-Path $assets) $name), [System.Drawing.Imaging.ImageFormat]::Png)
            } finally {
                $graphics.Dispose()
                $bitmap.Dispose()
            }
        }
        Write-Output "Generated $($variants.Count) icon variants for $arch"
    }
} finally { $source.Dispose() }
