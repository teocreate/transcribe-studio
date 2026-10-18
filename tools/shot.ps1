# Rebuilds, relaunches and screenshots the app window — the inner dev loop.
# Usage: pwsh tools/shot.ps1 [-NoBuild] [-Keys "{ESC}"] [-Wait 6] [-Out shot.png]
param(
  [switch]$NoBuild,
  [string]$Keys = "",
  [int]$Wait = 6,
  [string]$Out = "$env:TEMP\ts_shot.png"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

Get-Process transcribe-studio -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400

if (-not $NoBuild) {
  Push-Location $root
  $build = & cargo build 2>&1
  Pop-Location
  if ($LASTEXITCODE -ne 0) { $build | Select-Object -Last 30; exit 1 }
}

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win {
  // Without this the shell is DPI-virtualised and cursor coordinates no
  // longer match the physical pixels DWM reports for the window.
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out Rect r, int s);
}
"@ -ErrorAction SilentlyContinue

$proc = Start-Process -FilePath "$root\target\debug\transcribe-studio.exe" -PassThru
Start-Sleep -Seconds $Wait

$proc = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
if (-not $proc -or $proc.MainWindowHandle -eq 0) { "no window"; exit 1 }

$h = $proc.MainWindowHandle
[Win]::ShowWindow($h, 9) | Out-Null
[Win]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 900

if ($Keys) {
  Add-Type -AssemblyName System.Windows.Forms
  [System.Windows.Forms.SendKeys]::SendWait($Keys)
  Start-Sleep -Milliseconds 900
}

$r = New-Object Win+Rect
# DWMWA_EXTENDED_FRAME_BOUNDS (9) excludes the invisible resize border.
[Win]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) | Out-Null
$w = $r.R - $r.L; $ht = $r.B - $r.T

[Win]::SetProcessDPIAware() | Out-Null
Add-Type -AssemblyName System.Drawing
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"saved $Out (${w}x${ht})"
