# Sürecin iş parçacığı başına işlemci kullanımı (adlarıyla): gizliyken
# işlemciyi kimin harcadığını bulmak için (Adım 10). Openify'ı sınama
# ortamında (2. monitör, geçici ayar, sahte çalma, çevrimdışı) açar, isteğe
# göre tepsiye gizler (-Gizle), -Sn saniye ölçer.
#   pwsh -File scripts\is-parcacigi-islemci.ps1 -Sn 20 -Gizle
param(
    [int]$Sn = 20,
    [switch]$Gizle,
    [switch]$Calma,
    [switch]$Acilir,
    [string]$Exe = ""
)
$ErrorActionPreference = "Stop"
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class T {
  [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(uint access, bool inherit, uint id);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] public static extern int GetThreadDescription(IntPtr h, out IntPtr desc);
  [DllImport("kernel32.dll")] public static extern IntPtr LocalFree(IntPtr p);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  // Sürecin görünür açılır pencerelerine Esc gönder.
  public static int EscPopups(uint pid) {
    int n = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h) && (GetWindowLong(h, -20) & 0x80) != 0) {
        PostMessage(h, 0x0100, (IntPtr)0x1B, (IntPtr)0x00010001); PostMessage(h, 0x0101, (IntPtr)0x1B, unchecked((IntPtr)(int)0xC0010001)); n++;
      }
      return true;
    }, IntPtr.Zero);
    return n;
  }
  public static string Name(uint id) {
    IntPtr h = OpenThread(0x0800, false, id); // THREAD_QUERY_LIMITED_INFORMATION
    if (h == IntPtr.Zero) return "?";
    IntPtr p; string s = "";
    if (GetThreadDescription(h, out p) >= 0 && p != IntPtr.Zero) { s = Marshal.PtrToStringUni(p); LocalFree(p); }
    CloseHandle(h);
    return s;
  }
}
"@
$tag = [guid]::NewGuid().ToString('N').Substring(0, 8)
$settings = Join-Path $env:TEMP "openify-ayar-$tag.json"
$copyDir = Join-Path $env:TEMP "openify-thr-$tag"
New-Item -ItemType Directory -Force $copyDir | Out-Null
$srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
Copy-Item $srcExe (Join-Path $copyDir 'openify.exe')
try {
    [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    $env:OPENIFY_TEST_DISPLAY = "1180,760"
    if ($Calma) { $env:OPENIFY_TEST_FAKE_PLAY = "1" }
    $env:OPENIFY_TEST_OFFLINE = "1"
    $env:OPENIFY_TEST_SYSTEM = "smtc,taskbar,tray"
    $env:OPENIFY_INSTANCE_ID = "thr-$tag"
    $p = Start-Process (Join-Path $copyDir 'openify.exe') -PassThru
    for ($i = 0; $i -lt 100 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 50; $p.Refresh() }
    $main = $p.MainWindowHandle
    $ui = [T]::GetWindowThreadProcessId($main, [IntPtr]::Zero)
    Start-Sleep -Seconds 4
    if ($Acilir) {
        # Her yerden palet ve tepsi menüsü bir kez açılıp kapanır (pencereleri
        # gizli olarak kalır).
        [T]::PostMessage($main, 0x0312, [IntPtr]0x4F50, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 600
        [T]::PostMessage($main, 0x0312, [IntPtr]0x4F50, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 400
        [T]::PostMessage($main, 0x8051, [IntPtr]((1030 -shl 16) -bor 3800), [IntPtr]0x007B) | Out-Null
        Start-Sleep -Milliseconds 600
        "açık açılır pencereye Esc: $([T]::EscPopups([uint32]$p.Id))"
        Start-Sleep -Milliseconds 400
        "hâlâ açık: $([T]::EscPopups([uint32]$p.Id))"
    }
    if ($Gizle) { [T]::PostMessage($main, 0x0010, [IntPtr]0, [IntPtr]0) | Out-Null; Start-Sleep -Seconds 2 }
    $p.Refresh()
    $before = @{}
    foreach ($t in $p.Threads) { try { $before[$t.Id] = $t.TotalProcessorTime.TotalMilliseconds } catch {} }
    $total0 = $p.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Seconds $Sn
    $p.Refresh()
    $total = $p.TotalProcessorTime.TotalMilliseconds - $total0
    "süreç: $([math]::Round($total)) ms / $Sn sn = %$([math]::Round($total / ($Sn * 10), 3)) (bir çekirdek); iş parçacığı: $($p.Threads.Count); gizli: $Gizle; çalma: $Calma"
    $rows = foreach ($t in $p.Threads) {
        try {
            $ms = $t.TotalProcessorTime.TotalMilliseconds - ($before[$t.Id] ?? 0)
            [pscustomobject]@{ Id = $t.Id; Ms = [math]::Round($ms, 1); Ad = $(if ($t.Id -eq $ui) { "[arayüz] " } else { "" }) + [T]::Name([uint32]$t.Id); Bekleme = "$($t.WaitReason)" }
        } catch {}
    }
    $rows | Sort-Object Ms -Descending | Select-Object -First 15 | Format-Table -AutoSize | Out-String -Width 160
} finally {
    if ($p -and !$p.HasExited) { Stop-Process -Id $p.Id -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
    foreach ($k in "OPENIFY_SETTINGS_PATH", "OPENIFY_TEST_DISPLAY", "OPENIFY_TEST_FAKE_PLAY", "OPENIFY_TEST_OFFLINE", "OPENIFY_TEST_SYSTEM", "OPENIFY_INSTANCE_ID") {
        Remove-Item "Env:$k" -ErrorAction SilentlyContinue
    }
    Remove-Item $settings -ErrorAction SilentlyContinue
    Remove-Item $copyDir -Recurse -Force -ErrorAction SilentlyContinue
}
