# Batch conversion on the Markdown tab: several files at once, including a
# scan and an empty file. Every converted row must open its own preview, and
# files with no text must fail instead of producing an empty .md.
param([string]$Dir = "$env:TEMP\tstest\multi", [int[]]$RowYs = @(), [string]$Theme = "")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$shots = "$env:TEMP\ts_md_multi"
New-Item -ItemType Directory -Force -Path $shots | Out-Null

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M {
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
[M]::SetProcessDPIAware() | Out-Null
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
if ($h -eq 0) { "no window"; exit 1 }
[M]::ShowWindow($h, 9) | Out-Null
[M]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0003) | Out-Null
[M]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 900

$r = New-Object M+Rect
[M]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
$scale = ($r.R - $r.L) / 1180.0
$titleBar = 38

function Shot($name) {
  $rr = New-Object M+Rect
  [M]::DwmGetWindowAttribute($h, 9, [ref]$rr, 16) | Out-Null
  $w = $rr.R - $rr.L; $ht = $rr.B - $rr.T
  $bmp = New-Object System.Drawing.Bitmap $w, $ht
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rr.L, $rr.T, 0, 0, $bmp.Size)
  $bmp.Save("$shots\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  "  shot: $name"
}
function Click($cssX, $cssY) {
  [M]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 250
  [M]::SetCursorPos([int]($r.L + $cssX * $scale), [int]($r.T + $titleBar + $cssY * $scale)) | Out-Null
  Start-Sleep -Milliseconds 220
  [M]::mouse_event(0x02, 0, 0, 0, [IntPtr]::Zero)
  [M]::mouse_event(0x04, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 450
}
function Keys($s) { [System.Windows.Forms.SendKeys]::SendWait($s); Start-Sleep -Milliseconds 500 }
Keys "{ESC}"
Get-ChildItem $Dir -Filter *.md | Remove-Item
Click 660 26          # the "Markdown" segment in the toolbar
Start-Sleep -Milliseconds 700
Click 422 370         # drop zone -> native picker
Start-Sleep -Seconds 2
Keys "$Dir{ENTER}"
Start-Sleep -Milliseconds 600
Keys '"note.html" "otchet.html" "scan.pdf" "empty.txt"'
Keys "{ENTER}"
Start-Sleep -Seconds 2
[M]::SetForegroundWindow($h) | Out-Null
Shot "01-queued"

Click 1056 762        # convert
Start-Sleep -Seconds 10
Shot "02-converted"

$n = 3
foreach ($y in $RowYs) {
  Click 300 $y        # a row in the queue
  Start-Sleep -Seconds 1
  Shot ("{0:00}-row-{1}" -f $n, $y)
  $n++
}
"outputs: " + ((Get-ChildItem $Dir -Filter *.md).Name -join ', ')
[M]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x0003) | Out-Null
"shots in $shots"
