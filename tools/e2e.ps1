# End-to-end smoke test: drives the real window with synthetic input and
# screenshots each stage. Coordinates are given in CSS pixels and scaled by the
# window's actual size, so the script survives a different DPI.
param([string]$Audio = "$env:TEMP\tstest\sample.wav")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$shots = "$env:TEMP\ts_e2e"
New-Item -ItemType Directory -Force -Path $shots | Out-Null

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class U {
  // Without this the shell is DPI-virtualised and cursor coordinates no
  // longer match the physical pixels DWM reports for the window.
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out Rect r, int s);
}
"@ -ErrorAction SilentlyContinue
[U]::SetProcessDPIAware() | Out-Null
Add-Type -AssemblyName System.Drawing, System.Windows.Forms

Get-Process transcribe-studio -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

# The UI is embedded into the binary at compile time, so a stale exe would
# silently test yesterday's markup.
Push-Location $root
$build = & cargo build 2>&1
Pop-Location
if ($LASTEXITCODE -ne 0) { $build | Select-Object -Last 25; exit 1 }

$proc = Start-Process -FilePath "$root\target\debug\transcribe-studio.exe" -PassThru
Start-Sleep -Seconds 7
$h = (Get-Process -Id $proc.Id).MainWindowHandle
if ($h -eq 0) { "no window"; exit 1 }
[U]::ShowWindow($h, 9) | Out-Null
[U]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800

$r = New-Object U+Rect
# Pin the window above the shell that launched this script, otherwise the
# synthetic clicks land on whatever stole focus.
[U]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0003) | Out-Null
Start-Sleep -Milliseconds 400
[U]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
# The page is laid out for a 1180 CSS-px wide window (see tauri.conf.json).
$scale = ($r.R - $r.L) / 1180.0
$titleBar = 38   # physical px of native chrome above the web view

function Shot($name) {
  $rr = New-Object U+Rect
  [U]::DwmGetWindowAttribute($h, 9, [ref]$rr, 16) | Out-Null
  $w = $rr.R - $rr.L; $ht = $rr.B - $rr.T
  $bmp = New-Object System.Drawing.Bitmap $w, $ht
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rr.L, $rr.T, 0, 0, $bmp.Size)
  $bmp.Save("$shots\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  "  shot: $name"
}

function Click($cssX, $cssY) {
  [U]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 300
  $x = [int]($r.L + $cssX * $scale)
  $y = [int]($r.T + $titleBar + $cssY * $scale)
  [U]::SetCursorPos($x, $y) | Out-Null
  Start-Sleep -Milliseconds 250
  [U]::mouse_event(0x02, 0, 0, 0, [IntPtr]::Zero)   # LEFTDOWN
  [U]::mouse_event(0x04, 0, 0, 0, [IntPtr]::Zero)   # LEFTUP
  Start-Sleep -Milliseconds 450
}

function Keys($s) { [System.Windows.Forms.SendKeys]::SendWait($s); Start-Sleep -Milliseconds 500 }

"scale=$scale origin=$($r.L),$($r.T)"

# Dismiss the first-run toast / sheet if one opened.
Keys "{ESC}"
Shot "01-start"

# Open the file picker from the drop zone, then type the path into the dialog.
"clicking dropzone..."
Click 422 370
Start-Sleep -Seconds 2
Keys $Audio
Keys "{ENTER}"
Start-Sleep -Seconds 2
[U]::SetForegroundWindow($h) | Out-Null
Shot "02-queued"

# Run it.
"starting transcription..."
Click 1056 762
Start-Sleep -Seconds 4
Shot "03-running"

Start-Sleep -Seconds 12
Shot "04-progress"

# Wait for completion, then capture the result panel.
for ($i = 0; $i -lt 30; $i++) {
  Start-Sleep -Seconds 2
}
Shot "05-done"

# Leave the window in a normal z-order for the screenshots that follow.
[U]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x0003) | Out-Null
"shots in $shots"
