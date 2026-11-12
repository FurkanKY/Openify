# Başlık düğmeleri ve sürükleme alanı, sahte çalma sırasında (önbellekli
# karelerde) Windows'a doğru isabet sonucu veriyor mu? 2=başlık, 8=küçült,
# 9=büyüt, 20=kapat. Sürükleme noktası aramanın sağındaki boş alanda (solda profil hapı,
# gezinme ve sekmeler var); profil hapı istemci alanı (1) dönmeli.
param(
    # Sınanacak exe (varsayılan: target\release\openify.exe); kopyası çalıştırılır.
    [string]$Exe = "",
    # Uygulamaya verilecek ek ortam değişkenleri: "K=V;K=V".
    [string]$Env = ""
)
$ErrorActionPreference = "Stop"
$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")  # bu çalıştırmaya özel; gerçek settings.json'a dokunulmaz
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class C {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  public static void Restore(IntPtr target) {
    IntPtr cur = GetForegroundWindow(); if (cur == target) return;
    uint me = GetCurrentThreadId(); uint them = GetWindowThreadProcessId(cur, IntPtr.Zero);
    AttachThreadInput(me, them, true); SetForegroundWindow(target); AttachThreadInput(me, them, false);
  }
}
"@
[C]::SetProcessDPIAware() | Out-Null
try {
    [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    $fg = [C]::GetForegroundWindow()
    $env:OPENIFY_TEST_DISPLAY = "1180,760"
    $env:OPENIFY_TEST_FAKE_PLAY = "1"
    $extra = @()
    foreach ($pair in ($Env -split ';')) {
        $i = $pair.IndexOf('='); if ($i -lt 1) { continue }
        $extra += $pair.Substring(0, $i).Trim()
        [Environment]::SetEnvironmentVariable($pair.Substring(0, $i).Trim(), $pair.Substring($i + 1))
    }
    $copyDir = Join-Path $env:TEMP ('openify-isabet-' + [guid]::NewGuid().ToString('N').Substring(0,8)); New-Item -ItemType Directory -Force $copyDir | Out-Null
    $srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
    Copy-Item $srcExe (Join-Path $copyDir 'openify.exe')
    $p = Start-Process (Join-Path $copyDir 'openify.exe') -PassThru
    for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    Start-Sleep -Milliseconds 300
    [C]::Restore($fg)
    Start-Sleep -Seconds 5
    $h = $p.MainWindowHandle
    # Noktalar pencerenin iç alanında, mantıksal piksel (1180 genişlik); ekran
    # ölçeğiyle (%125 vb.) fiziksel piksele çevrilir.
    $scale = [C]::GetDpiForWindow($h) / 96.0
    $o = New-Object C+POINT; [C]::ClientToScreen($h, [ref]$o) | Out-Null
    $points = [ordered]@{ "sürükle" = @(1000, 21); "profil" = @(50, 21); "küçült" = @(1064, 21); "büyüt" = @(1110, 21); "kapat" = @(1156, 21); "arama" = @(582, 21) }
    foreach ($name in $points.Keys) {
        $cx = [int]($points[$name][0] * $scale); $cy = [int]($points[$name][1] * $scale)
        $x = $o.X + $cx; $y = $o.Y + $cy
        $lp = [IntPtr]((($y -band 0xFFFF) -shl 16) -bor ($x -band 0xFFFF))
        # gpui isabeti son fare konumundan yapar: önce fareyi oraya "getir".
        [C]::SendMessage($h, 0x0200, [IntPtr]0, [IntPtr](($cy -shl 16) -bor $cx)) | Out-Null
        Start-Sleep -Milliseconds 80
        $results = @()
        for ($k = 0; $k -lt 12; $k++) { $results += [int][C]::SendMessage($h, 0x0084, [IntPtr]0, $lp); Start-Sleep -Milliseconds 47 }
        "$name -> $(($results | Group-Object | ForEach-Object { "$($_.Name) x$($_.Count)" }) -join ', ')"
    }
    "odak kullanıcıda: $([C]::GetForegroundWindow() -eq $fg)"
    Stop-Process -Id $p.Id
} finally {
    Remove-Item Env:OPENIFY_TEST_FAKE_PLAY -ErrorAction SilentlyContinue
    foreach ($k in $extra) { [Environment]::SetEnvironmentVariable($k, $null) }
    Start-Sleep -Milliseconds 500
    Remove-Item Env:OPENIFY_SETTINGS_PATH -ErrorAction SilentlyContinue
    Remove-Item $settings -ErrorAction SilentlyContinue
    if ($copyDir) { Remove-Item $copyDir -Recurse -Force -ErrorAction SilentlyContinue }
}
