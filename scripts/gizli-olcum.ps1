# Gizli (tepside) durum ölçümü: işlemci (süreç + iş parçacığı), özel bellek,
# isteğe göre bellek dökümü (bellek-dokum.ps1'in DokumYap'ı), sonra tepsiden
# geri gösterip karelerin yeniden başladığını (VSync uyanması) doğrular.
# 2. monitör, kopya exe, geçici ayar, sahte çalma + çevrimdışı. -Agir: en ağır
# durum (palette arama + menü kullanıldıktan sonra gizle); -AcTekrar N: gizliyken
# paleti / menüyü N kez aç (açılış ms). Görüntüler wm-pre-<Label> (gizlemeden önce)
# ve gpui-yama-<Label>-goster (gösterdikten sonra) %TEMP%/openify-ekran'a düşer.
param(
    [Parameter(Mandatory)] [string]$Exe,
    [int]$Sn = 60,
    [switch]$Acilir,
    # En ağır durum: palette arama yazılır (iki kez), tepsi menüsü açılıp kullanılır, sonra gizlenir.
    [switch]$Agir,
    # Gizliyken palet ve menüyü N kez aç: açılış süresi (ms) ve bellek (pencereler her seferinde yeniden kurulur).
    [int]$AcTekrar = 0,
    [switch]$Detay,
    [switch]$Calma,
    [string]$Label = "",
    [int]$GosterTekrar = 3,
    # Kare kaydı kapalı (kare-stats iş parçacığı 200 ms'de bir uyanır; saf
    # süreç işlemcisi için).
    [switch]$KareYok
)
$ErrorActionPreference = "Stop"
$proj = "C:\Users\furka\Desktop\Projects\Openify"
# bellek-dokum.ps1'den C# türünü ve DokumYap işlevini al.
$src = Get-Content (Join-Path $proj 'scripts\bellek-dokum.ps1') -Encoding UTF8
$a = ($src | Select-String -Pattern '^Add-Type -TypeDefinition @"' | Select-Object -First 1).LineNumber - 1
$b = ($src | Select-String -Pattern '^"@' | Where-Object { $_.LineNumber -gt $a } | Select-Object -First 1).LineNumber - 1
Invoke-Expression (($src[$a..$b]) -join "`n")
$f0 = ($src | Select-String -Pattern '^function MB\(' | Select-Object -First 1).LineNumber - 1
$f1 = ($src | Select-String -Pattern '^function DokumYap' | Select-Object -First 1).LineNumber - 1
Invoke-Expression (($src[$f0..($f0)]) -join "`n")
# DokumYap'ın sonu: ilk sütunda "}" olan satır.
$e = $f1; while ($src[$e] -ne '}') { $e++ }
Invoke-Expression (($src[$f1..$e]) -join "`n")
$Referans = $false; $Bak = $false

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class G {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(uint access, bool inherit, uint id);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] public static extern int GetThreadDescription(IntPtr h, out IntPtr desc);
  [DllImport("kernel32.dll")] public static extern IntPtr LocalFree(IntPtr p);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
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
  public static IntPtr FirstPopup(uint pid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h) && (GetWindowLong(h, -20) & 0x80) != 0) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
  public static void Key(IntPtr h, int vk, int scan) {
    long down = 1 | ((long)scan << 16);
    PostMessage(h, 0x0100, (IntPtr)vk, (IntPtr)down);
    System.Threading.Thread.Sleep(30);
    PostMessage(h, 0x0101, (IntPtr)vk, unchecked((IntPtr)(down | 0xC0000000L)));
  }
  public static bool Restore(IntPtr target) {
    IntPtr cur = GetForegroundWindow();
    if (cur == target) return true;
    uint me = GetCurrentThreadId();
    uint them = GetWindowThreadProcessId(cur, IntPtr.Zero);
    AttachThreadInput(me, them, true);
    bool ok = SetForegroundWindow(target);
    AttachThreadInput(me, them, false);
    return ok;
  }
  public static string Name(uint id) {
    IntPtr h = OpenThread(0x0800, false, id);
    if (h == IntPtr.Zero) return "?";
    IntPtr p; string s = "";
    if (GetThreadDescription(h, out p) >= 0 && p != IntPtr.Zero) { s = Marshal.PtrToStringUni(p); LocalFree(p); }
    CloseHandle(h);
    return s;
  }
}
"@
function WaitFor([scriptblock]$cond, [int]$timeoutMs = 3000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) { if (& $cond) { return [math]::Round($sw.Elapsed.TotalMilliseconds, 1) }; Start-Sleep -Milliseconds 1 }
    return -1
}
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class P {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr dc);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
  [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr dc, IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  // PrintWindow düz GDI bitmap'ine; System.Drawing'e sonra `FromHbitmap` ile
  // geçilir. (`Graphics.GetHdc()` KULLANILMAZ: GDI+ o yolda #0D0B0C rengini
  // kendi saydamlık anahtarı sayıp (0,0,0,0) yapıyor; gölgede bu renge düşen
  // pikseller "şeffaf delik" gibi görünüyordu, 7 Kasım; ekran-sinama.ps1 gibi.)
  public static IntPtr Grab(IntPtr h, int w, int hh) {
    IntPtr screen = GetDC(IntPtr.Zero); IntPtr mem = CreateCompatibleDC(screen);
    IntPtr bmp = CreateCompatibleBitmap(screen, w, hh); IntPtr old = SelectObject(mem, bmp);
    PrintWindow(h, mem, 2);
    SelectObject(mem, old); DeleteDC(mem); ReleaseDC(IntPtr.Zero, screen);
    return bmp;
  }
}
"@
# Yalnız bizim pencere (PrintWindow); ekrandan kesit alınmaz.
function Shot($h, $name) {
    $dir = Join-Path $env:TEMP 'openify-ekran'; New-Item -ItemType Directory -Force $dir | Out-Null
    $r = New-Object P+RECT; [P]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    $hbmp = [P]::Grab($h, $w, $hh)
    $bmp = [System.Drawing.Image]::FromHbitmap($hbmp)
    [P]::DeleteObject($hbmp) | Out-Null
    $bmp.Save("$dir\$name.png"); $bmp.Dispose()
    "  görüntü $dir\$name.png (${w}x$hh)"
}
function Lines($f) { if (Test-Path $f) { ([IO.File]::ReadAllLines($f)).Count } else { 0 } }

$script:shows = @()
$tag = [guid]::NewGuid().ToString('N').Substring(0, 8)
$settings = Join-Path $env:TEMP "openify-ayar-$tag.json"
$frames = Join-Path $env:TEMP "openify-kare-$tag.csv"
$copyDir = Join-Path $env:TEMP "openify-gizli-$tag"
New-Item -ItemType Directory -Force $copyDir | Out-Null
Copy-Item $Exe (Join-Path $copyDir 'openify.exe')
$fg = [G]::GetForegroundWindow()
$p = $null
try {
    [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    $env:OPENIFY_TEST_DISPLAY = "1180,760"
    if ($Calma) { $env:OPENIFY_TEST_FAKE_PLAY = "1" }
    $env:OPENIFY_TEST_OFFLINE = "1"
    $env:OPENIFY_TEST_SYSTEM = "smtc,taskbar,tray"
    $env:OPENIFY_INSTANCE_ID = "gz-$tag"
    if (-not $KareYok) { $env:OPENIFY_FRAME_STATS = $frames }
    $p = Start-Process (Join-Path $copyDir 'openify.exe') -PassThru
    for ($i = 0; $i -lt 100 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 50; $p.Refresh() }
    $main = $p.MainWindowHandle
    $ui = [G]::GetWindowThreadProcessId($main, [IntPtr]::Zero)
    Start-Sleep -Seconds 5
    $p.Refresh(); $memVis = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    if ($Acilir) {
        [G]::PostMessage($main, 0x0312, [IntPtr]0x4F50, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 600
        [G]::PostMessage($main, 0x0312, [IntPtr]0x4F50, [IntPtr]0) | Out-Null
        Start-Sleep -Milliseconds 400
        [G]::PostMessage($main, 0x8051, [IntPtr]((1030 -shl 16) -bor 3800), [IntPtr]0x007B) | Out-Null
        Start-Sleep -Milliseconds 600
        "açık açılır pencereye Esc: $([G]::EscPopups([uint32]$p.Id))"
        Start-Sleep -Milliseconds 400
    }
    if ($Agir) {
        for ($k = 0; $k -lt 2; $k++) {
            [G]::PostMessage($main, 0x0312, [IntPtr]0x4F50, [IntPtr]0) | Out-Null
            Start-Sleep -Milliseconds 700
            $pop = [G]::FirstPopup([uint32]$p.Id)
            foreach ($kk in @(@(0x44,0x20),@(0x41,0x1E),@(0x4C,0x26),@(0x45,0x12),@(0x53,0x1F))) { [G]::Key($pop, $kk[0], $kk[1]); Start-Sleep -Milliseconds 120 }
            Start-Sleep -Milliseconds 1500
            [G]::Key($pop, 0x28, 0x50); [G]::Key($pop, 0x28, 0x50); Start-Sleep -Milliseconds 300
            if ($k -eq 0) { Shot $pop "wm-agir-palet" | Out-Null }
            [G]::Key($pop, 0x1B, 1); Start-Sleep -Milliseconds 500
            [G]::PostMessage($main, 0x8051, [IntPtr]((1030 -shl 16) -bor 3800), [IntPtr]0x007B) | Out-Null
            Start-Sleep -Milliseconds 700
            $pop = [G]::FirstPopup([uint32]$p.Id)
            [G]::Key($pop, 0x28, 0x50); [G]::Key($pop, 0x28, 0x50); Start-Sleep -Milliseconds 300
            [G]::Key($pop, 0x1B, 1); Start-Sleep -Milliseconds 500
        }
        "agir: kalan açılır pencere: $([G]::EscPopups([uint32]$p.Id))"
    }
    if ([G]::GetForegroundWindow() -ne $fg) { [G]::Restore($fg) | Out-Null }
    Shot $main "wm-pre-$Label" | Out-Null
    [G]::PostMessage($main, 0x0010, [IntPtr]0, [IntPtr]0) | Out-Null
    "X → gizlendi: $(WaitFor { -not [G]::IsWindowVisible($main) }) ms"
    if ($AcTekrar -gt 0) {
        $latp = @(); $latm = @()
        for ($n = 0; $n -lt $AcTekrar; $n++) {
            Start-Sleep -Seconds 7   # bırakma bitsin (popup'lar yok edilir)
            [G]::PostMessage($main, 0x0312, [IntPtr]0x4F50, [IntPtr]0) | Out-Null
            $ms = WaitFor { [G]::FirstPopup([uint32]$p.Id) -ne [IntPtr]::Zero } 3000
            $latp += $ms
            $pop = [G]::FirstPopup([uint32]$p.Id)
            Start-Sleep -Milliseconds 500
            if ($n -eq 0 -and $pop -ne [IntPtr]::Zero) {
                foreach ($kk in @(@(0x44,0x20),@(0x41,0x1E),@(0x4C,0x26))) { [G]::Key($pop, $kk[0], $kk[1]); Start-Sleep -Milliseconds 100 }
                Start-Sleep -Milliseconds 900
                Shot $pop "wm-ac-palet" | Out-Null
            }
            [G]::Key($pop, 0x1B, 1)
            WaitFor { [G]::FirstPopup([uint32]$p.Id) -eq [IntPtr]::Zero } 3000 | Out-Null
            Start-Sleep -Seconds 7
            [G]::PostMessage($main, 0x8051, [IntPtr]((1030 -shl 16) -bor 3800), [IntPtr]0x007B) | Out-Null
            $ms = WaitFor { [G]::FirstPopup([uint32]$p.Id) -ne [IntPtr]::Zero } 3000
            $latm += $ms
            $pop = [G]::FirstPopup([uint32]$p.Id)
            Start-Sleep -Milliseconds 400
            if ($n -eq 0 -and $pop -ne [IntPtr]::Zero) { Shot $pop "wm-ac-menu" | Out-Null }
            [G]::Key($pop, 0x1B, 1)
            WaitFor { [G]::FirstPopup([uint32]$p.Id) -eq [IntPtr]::Zero } 3000 | Out-Null
        }
        Start-Sleep -Seconds 8; $p.Refresh()
        "ac: palet açılış ms: $($latp -join ', ')  | menü açılış ms: $($latm -join ', ') | özel bellek şimdi $([math]::Round($p.PrivateMemorySize64 / 1MB, 1)) MB"
    }
    # Gizlendikten sonra özel bellek zaman çizelgesi (sürücü serbest
    # bırakmayı erteleyebiliyor).
    $tl = for ($i = 1; $i -le 24; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh(); "$($i/2)s:$([math]::Round($p.PrivateMemorySize64 / 1MB, 1))" }
    "gizlendikten sonra özel MB: $($tl -join ' ')"
    $p.Refresh()
    $before = @{}
    foreach ($t in $p.Threads) { try { $before[$t.Id] = $t.TotalProcessorTime.TotalMilliseconds } catch {} }
    $total0 = $p.TotalProcessorTime.TotalMilliseconds
    $mem0 = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    $rows0 = Lines $frames
    Start-Sleep -Seconds $Sn
    $p.Refresh()
    $total = $p.TotalProcessorTime.TotalMilliseconds - $total0
    $mem1 = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    $rows1 = Lines $frames
    "[$Label] gizli $Sn sn: süreç $([math]::Round($total, 1)) ms = %$([math]::Round($total / ($Sn * 10), 3)); kare $($rows1 - $rows0); özel bellek görünürken $memVis MB, gizli başta $mem0 MB, sonda $mem1 MB; açılır: $Acilir"
    $rows = foreach ($t in $p.Threads) {
        try {
            $ms = $t.TotalProcessorTime.TotalMilliseconds - ($before[$t.Id] ?? 0)
            if ($ms -gt 0) { [pscustomobject]@{ Ms = [math]::Round($ms, 1); Ad = $(if ($t.Id -eq $ui) { "[arayüz] " } else { "" }) + [G]::Name([uint32]$t.Id) } }
        } catch {}
    }
    $rows | Sort-Object Ms -Descending | Select-Object -First 6 | Format-Table -AutoSize | Out-String -Width 120
    if ($Detay) { $Label = "gizli"; DokumYap $p }
    # Tepsiden geri göster: görünürlük süresi ve sonraki 2 sn'de çizilen kare
    # (sahte çalmada ilerleme/ekolayzer kare ister: VSync uyanmalı).
    for ($n = 0; $n -lt $GosterTekrar; $n++) {
        $r0 = Lines $frames
        [G]::PostMessage($main, 0x8051, [IntPtr]0, [IntPtr]0x0400) | Out-Null
        $ms = WaitFor { [G]::IsWindowVisible($main) }
        $script:shows += $ms
        Start-Sleep -Milliseconds 2000
        $r1 = Lines $frames
        if ([G]::GetForegroundWindow() -ne $fg) { [G]::Restore($fg) | Out-Null }
        if ($n -eq 0) { Shot $main "gpui-yama-$Label-goster" }
        $p.Refresh()
        "göster $($n+1): görünür $ms ms; 2 sn'de kare $($r1 - $r0); özel bellek $([math]::Round($p.PrivateMemorySize64 / 1MB, 1)) MB"
        [G]::PostMessage($main, 0x0010, [IntPtr]0, [IntPtr]0) | Out-Null
        WaitFor { -not [G]::IsWindowVisible($main) } | Out-Null
        Start-Sleep -Milliseconds 6000
        $p.Refresh()
        "  yeniden gizli (6 sn): özel bellek $([math]::Round($p.PrivateMemorySize64 / 1MB, 1)) MB"
    }
    $srt = @($script:shows | Sort-Object)
    if ($srt.Count) { "[$Label] göster ms: $($script:shows -join ', ')  ortanca $($srt[[int][math]::Floor(($srt.Count - 1) / 2)])  en çok $($srt[-1])" }
    if ($Detay) { Start-Sleep -Seconds 2; $Label = "gizli2"; DokumYap $p }
} finally {
    if ($p -and !$p.HasExited) { Stop-Process -Id $p.Id -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
    foreach ($k in "OPENIFY_SETTINGS_PATH", "OPENIFY_TEST_DISPLAY", "OPENIFY_TEST_FAKE_PLAY", "OPENIFY_TEST_OFFLINE", "OPENIFY_TEST_SYSTEM", "OPENIFY_INSTANCE_ID", "OPENIFY_FRAME_STATS") {
        Remove-Item "Env:$k" -ErrorAction SilentlyContinue
    }
    Remove-Item $settings, $frames -ErrorAction SilentlyContinue
    Remove-Item $copyDir -Recurse -Force -ErrorAction SilentlyContinue
}
