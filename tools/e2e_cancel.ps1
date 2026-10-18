# Verifies the stop path and the environment pane.
param([string]$Audio = "$env:TEMP\tstest\lecture-10min.wav", [string]$Theme = "light")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$shots = "$env:TEMP\ts_cancel"
New-Item -ItemType Directory -Force -Path $shots | Out-Null

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class C {
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
[C]::SetProcessDPIAware() | Out-Null
Add-Type -AssemblyName System.Drawing, System.Windows.Forms

Get-Process transcribe-studio -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500
Push-Location $root; $build = & cargo build 2>&1; Pop-Location
if ($LASTEXITCODE -ne 0) { $build | Select-Object -Last 25; exit 1 }

$cfg = "$env:LOCALAPPDATA\TranscribeStudio\settings.json"
$json = Get-Content $cfg -Raw | ConvertFrom-Json
$json.theme = $Theme
$json | ConvertTo-Json -Depth 5 | Set-Content $cfg -Encoding UTF8

$proc = Start-Process -FilePath "$root\target\debug\transcribe-studio.exe" -PassThru
Start-Sleep -Seconds 7
$h = (Get-Process -Id $proc.Id).MainWindowHandle
[C]::ShowWindow($h, 9) | Out-Null
[C]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0003) | Out-Null
[C]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 900

$r = New-Object C+Rect
[C]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
$scale = ($r.R - $r.L) / 1180.0
$titleBar = 38

function Shot($name) {
  $rr = New-Object C+Rect
  [C]::DwmGetWindowAttribute($h, 9, [ref]$rr, 16) | Out-Null
  $w = $rr.R - $rr.L; $ht = $rr.B - $rr.T
  $bmp = New-Object System.Drawing.Bitmap $w, $ht
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rr.L, $rr.T, 0, 0, $bmp.Size)
  $bmp.Save("$shots\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  "  shot: $name"
}
function Click($cssX, $cssY) {
  [C]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 250
  [C]::SetCursorPos([int]($r.L + $cssX * $scale), [int]($r.T + $titleBar + $cssY * $scale)) | Out-Null
  Start-Sleep -Milliseconds 220
  [C]::mouse_event(0x02, 0, 0, 0, [IntPtr]::Zero)
  [C]::mouse_event(0x04, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 450
}
function Keys($s) { [System.Windows.Forms.SendKeys]::SendWait($s); Start-Sleep -Milliseconds 500 }

Keys "{ESC}"
Click 422 370
Start-Sleep -Seconds 2
Keys $Audio
Keys "{ENTER}"
Start-Sleep -Seconds 2
[C]::SetForegroundWindow($h) | Out-Null

Click 1056 762         # start
Start-Sleep -Seconds 10
Shot "01-running"

Click 1056 762         # the same slot now holds "Остановить"
Start-Sleep -Seconds 4
Shot "02-cancelled"

# Environment pane.
Click 1150 26
Start-Sleep -Milliseconds 800
Click 218 220          # "Окружение" in the sheet nav
Start-Sleep -Milliseconds 900
Shot "03-env"

[C]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x0003) | Out-Null
"shots in $shots"
