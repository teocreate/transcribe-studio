# Smoke test for the model manager: open settings, download a small model,
# watch the progress bar, and confirm the card flips to "installed".
param([string]$Theme = "")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$shots = "$env:TEMP\ts_model"
New-Item -ItemType Directory -Force -Path $shots | Out-Null

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class K {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint fl);
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out Rect r, int s);
}
"@ -ErrorAction SilentlyContinue
[K]::SetProcessDPIAware() | Out-Null
Add-Type -AssemblyName System.Drawing, System.Windows.Forms

Get-Process transcribe-studio -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

Push-Location $root
$build = & cargo build 2>&1
Pop-Location
if ($LASTEXITCODE -ne 0) { $build | Select-Object -Last 25; exit 1 }

if ($Theme) {
  $cfg = "$env:LOCALAPPDATA\TranscribeStudio\settings.json"
  $json = Get-Content $cfg -Raw | ConvertFrom-Json
  $json.theme = $Theme
  $json | ConvertTo-Json -Depth 5 | Set-Content $cfg -Encoding UTF8
}

$proc = Start-Process -FilePath "$root\target\debug\transcribe-studio.exe" -PassThru
Start-Sleep -Seconds 7
$h = (Get-Process -Id $proc.Id).MainWindowHandle
[K]::ShowWindow($h, 9) | Out-Null
[K]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0003) | Out-Null
[K]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 900

$r = New-Object K+Rect
[K]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
$scale = ($r.R - $r.L) / 1180.0
$titleBar = 38

function Shot($name) {
  $rr = New-Object K+Rect
  [K]::DwmGetWindowAttribute($h, 9, [ref]$rr, 16) | Out-Null
  $w = $rr.R - $rr.L; $ht = $rr.B - $rr.T
  $bmp = New-Object System.Drawing.Bitmap $w, $ht
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rr.L, $rr.T, 0, 0, $bmp.Size)
  $bmp.Save("$shots\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  "  shot: $name"
}
function Click($cssX, $cssY) {
  [K]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 250
  [K]::SetCursorPos([int]($r.L + $cssX * $scale), [int]($r.T + $titleBar + $cssY * $scale)) | Out-Null
  Start-Sleep -Milliseconds 220
  [K]::mouse_event(0x02, 0, 0, 0, [IntPtr]::Zero)
  [K]::mouse_event(0x04, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 450
}

[System.Windows.Forms.SendKeys]::SendWait("{ESC}")
Start-Sleep -Milliseconds 600
Click 1150 26          # gear -> settings sheet
Start-Sleep -Seconds 1
Shot "01-models"

# "Скачать" on the Base card (second column, first row).
Click 937 338
Start-Sleep -Seconds 3
Shot "02-downloading"
Start-Sleep -Seconds 6
Shot "03-downloading"

# Give the transfer time to finish, then confirm the card flipped.
for ($i = 0; $i -lt 40; $i++) {
  Start-Sleep -Seconds 2
  if (Test-Path (Join-Path $env:LOCALAPPDATA "TranscribeStudio/models/base/model.bin")) { break }
}
Start-Sleep -Seconds 3
Shot "04-installed"

[K]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x0003) | Out-Null
"shots in $shots"
