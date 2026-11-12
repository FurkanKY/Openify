# Sistemle bütünleşme sınaması (Adım 10): her yerden palet, tepsi menüsü,
# Windows medya kontrolü, tepsiye gizleme (işlemci), tek kopya.
#
# Kurallar (docs/TESTING.md): 2. monitörde, sessiz, odak çalmadan; exe'nin kopyası;
# kendi geçici ayar dosyası (OPENIFY_SETTINGS_PATH); gerçek çalma yok
# (OPENIFY_TEST_FAKE_PLAY + OPENIFY_TEST_OFFLINE; motora yalnız Duraklat gider).
# Kısayol ve tepsi tıklaması pencereye ileti olarak gönderilir (gerçek tuş /
# fare yok): bu yüzden "öne alma" sınamada izin almaz, ölçülen süre pencerenin
# görünmesine kadardır.
#
#   pwsh -File scripts\sistem-sinama.ps1 -Adimlar palet,menu,smtc,gizle,ikinci
#
# Görüntüler: %TEMP%\openify-ekran\sistem-*.png
param(
    [string[]]$Adimlar = @("palet", "menu", "smtc", "gizle", "ikinci"),
    [int]$Tekrar = 10,
    [int]$CpuSn = 60,
    # OPENIFY_TEST_SYSTEM: hangi parçalar açık (hotkey yok: genel kısayolu
    # sınama sırasında kullanıcıdan almayalım; WM_HOTKEY doğrudan gönderilir).
    [string]$Parcalar = "smtc,taskbar,tray",
    [string]$Exe = ""
)
$ErrorActionPreference = "Stop"
# "-File" ile verilen "palet,menu" tek dize gelir.
$Adimlar = @($Adimlar | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() })
$scratch = Join-Path $env:TEMP 'openify-ekran'
New-Item -ItemType Directory -Force $scratch | Out-Null
$tag = [guid]::NewGuid().ToString('N').Substring(0, 8)
$settings = Join-Path $env:TEMP "openify-ayar-$tag.json"
$frames = Join-Path $env:TEMP "openify-kare-$tag.csv"

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class S {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr dc);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
  [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr dc, IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
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
  // Sürecin bütün üst düzey pencereleri (gizliler dahil).
  public static int AllWindows(uint pid) {
    int n = 0;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) n++; return true; }, IntPtr.Zero);
    return n;
  }
  // Sürecin görünür açılır pencereleri (WS_EX_TOOLWINDOW).
  public static List<IntPtr> Popups(uint pid) {
    var list = new List<IntPtr>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h) && (GetWindowLong(h, -20) & 0x80) != 0) list.Add(h);
      return true;
    }, IntPtr.Zero);
    return list;
  }
}
"@
[S]::SetProcessDPIAware() | Out-Null

$WM_CLOSE = 0x0010; $WM_KEYDOWN = 0x0100; $WM_KEYUP = 0x0101; $WM_CHAR = 0x0102
$WM_COMMAND = 0x0111; $WM_HOTKEY = 0x0312; $WM_CONTEXTMENU = 0x007B
$WM_TRAY = 0x8051; $NIN_SELECT = 0x0400
$HOTKEY_ID = 0x4F50; $THUMB_PLAY = 0x4F02

function Key($h, $vk, $scan) {
    $down = 1 -bor ($scan -shl 16)
    [S]::PostMessage($h, $WM_KEYDOWN, [IntPtr]$vk, [IntPtr]$down) | Out-Null
    Start-Sleep -Milliseconds 30
    [S]::PostMessage($h, $WM_KEYUP, [IntPtr]$vk, [IntPtr]([long]$down -bor 0xC0000000L)) | Out-Null
}

# Yalnız bizim pencerenin görüntüsü (PrintWindow). Ekrandan kesit ALINMAZ:
# 2. monitörde kullanıcının kendi pencereleri olabilir, onlar da görüntüye
# girer (7 Kasım'da girdi, silindi). Saydam pay siyah çıkar.
function ScreenShot($h, $name, $pad = 0) {
    $r = New-Object S+RECT
    [S]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    $hbmp = [S]::Grab($h, $w, $hh)
    $bmp = [System.Drawing.Image]::FromHbitmap($hbmp)
    [S]::DeleteObject($hbmp) | Out-Null
    $bmp.Save("$scratch\$name.png")
    $bmp.Dispose()
    "  görüntü $name.png ($($r.L),$($r.T) ${w}x$hh)"
}

function WaitFor([scriptblock]$cond, [int]$timeoutMs = 3000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        if (& $cond) { return $sw.Elapsed.TotalMilliseconds }
        Start-Sleep -Milliseconds 1
    }
    return -1
}

# Öndeki pencere kimin: Openify sınama süreci odağı çalmış mı?
function FgOwner { $h = [S]::GetForegroundWindow(); $id = [uint32]0; [S]::GetWindowThreadProcessId($h, [ref]$id) | Out-Null; $name = (Get-Process -Id $id -ErrorAction SilentlyContinue).ProcessName; if ($procs | Where-Object { $_.Id -eq $id }) { "SINAMA SÜRECİ (odak çalındı!)" } else { "$name (bizim değil)" } }

# Odak yalnız sınama sürecindeyse kullanıcının penceresine geri verilir
# (kullanıcı bu arada başka pencereye geçtiyse ona dokunulmaz).
function GiveBack { if ((FgOwner) -like "SINAMA*") { "odak geri verildi (betik): $([S]::Restore($fg))" } }

function Median($xs) { $s = @($xs | Sort-Object); if ($s.Count -eq 0) { return $null }; return $s[[int][math]::Floor(($s.Count - 1) / 2)] }

$logPath = "$env:LOCALAPPDATA\Openify\openify.log"
function LogSince($offset) {
    $fs = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
    $fs.Seek($offset, 'Begin') | Out-Null
    $text = (New-Object System.IO.StreamReader($fs)).ReadToEnd()
    $fs.Close()
    $text
}
# env_logger satırının UTC zamanı.
function LogTime($line) {
    if ($line -match '^\[(\S+Z)') { return [DateTime]::Parse($Matches[1]).ToUniversalTime() }
    return $null
}

$copyDir = Join-Path $env:TEMP "openify-sistem-$tag"
New-Item -ItemType Directory -Force $copyDir | Out-Null
$srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
$exeCopy = Join-Path $copyDir 'openify.exe'
Copy-Item $srcExe $exeCopy
$procs = @()
try {
    [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    $env:OPENIFY_TEST_DISPLAY = "1180,760"
    $env:OPENIFY_TEST_FAKE_PLAY = "1"
    $env:OPENIFY_TEST_OFFLINE = "1"
    $env:OPENIFY_TEST_SYSTEM = $Parcalar
    $env:OPENIFY_INSTANCE_ID = "sinama-$tag"
    $env:OPENIFY_FRAME_STATS = $frames
    $fg = [S]::GetForegroundWindow()
    $logStart = (Get-Item $logPath).Length
    $ps5 = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
    $smtcScript = Join-Path $PSScriptRoot 'smtc-oku.ps1'
    $waitOut = Join-Path $env:TEMP "openify-smtc-$tag.txt"
    if ($Adimlar -contains "smtc") {
        # Medya kontrolünü uygulama açılmadan izlemeye başla (yayılma süresi).
        $waiter = Start-Process $ps5 -ArgumentList "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$smtcScript`"", "-Mode", "bekle", "-Kimlik", "Openify.Test" -RedirectStandardOutput $waitOut -WindowStyle Hidden -PassThru
        Start-Sleep -Milliseconds 2500
    }
    $t0 = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process $exeCopy -PassThru
    $procs += $p
    for ($i = 0; $i -lt 100 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 50; $p.Refresh() }
    $main = $p.MainWindowHandle
    Start-Sleep -Milliseconds 300
    # Uygulama sınama koşusunda odağı kendisi geri veriyor; olmadıysa betik de dener.
    "pencere $main, $([int]$t0.ElapsedMilliseconds) ms; öndeki: $(FgOwner)"
    GiveBack
    Start-Sleep -Seconds 3

    if ($Adimlar -contains "palet") {
        "── her yerden palet ($Tekrar kez) ──"
        $open = @(); $close = @()
        $p.Refresh(); $mem0 = $p.PrivateMemorySize64; $memAt = @()
        for ($n = 0; $n -lt $Tekrar; $n++) {
            $sw = [Diagnostics.Stopwatch]::StartNew()
            [S]::PostMessage($main, $WM_HOTKEY, [IntPtr]$HOTKEY_ID, [IntPtr]0) | Out-Null
            $ms = WaitFor { ([S]::Popups($p.Id)).Count -gt 0 } 3000
            $open += $ms
            $pop = ([S]::Popups($p.Id)) | Select-Object -First 1
            if (-not $pop) { "  palet penceresi açılmadı (ana pencere öndeyse palet pencerenin içinde açılır: öndeki $([S]::GetForegroundWindow()), ana $main)"; break }
            if ($n -eq 0) {
                Start-Sleep -Milliseconds 400
                ScreenShot $pop "sistem-palet" 0
                # Palet yazıyı tuş iletisinden okuyor (WM_CHAR değil): d, a, l.
                foreach ($k in @(@(0x44, 0x20), @(0x41, 0x1E), @(0x4C, 0x26))) { Key $pop $k[0] $k[1]; Start-Sleep -Milliseconds 40 }
                Start-Sleep -Milliseconds 700
                ScreenShot $pop "sistem-palet-yazi" 0
            } else { Start-Sleep -Milliseconds 250 }
            $sw = [Diagnostics.Stopwatch]::StartNew()
            Key $pop 0x1B 1   # Esc
            $close += (WaitFor { ([S]::Popups($p.Id)).Count -eq 0 } 3000)
            Start-Sleep -Milliseconds 200
            if ($n -in 0, 1, 4, 9, 19, 39, 59, 99) { $p.Refresh(); $memAt += "#$($n + 1): $([math]::Round($p.PrivateMemorySize64 / 1MB, 1))" }
        }
        "  özel bellek açılış sonrası (MB): $($memAt -join ', ')"
        "  pencere görünene kadar (ms): $($open -join ', ')  ortanca $(Median $open)"
        "  Esc → kapanana kadar (ms): $($close -join ', ')  ortanca $(Median $close)"
        Start-Sleep -Seconds 1; $p.Refresh()
        "  özel bellek: önce $([math]::Round($mem0 / 1MB, 1)) MB, $Tekrar açılıştan sonra $([math]::Round($p.PrivateMemorySize64 / 1MB, 1)) MB; sürecin üst düzey pencereleri: $([S]::AllWindows($p.Id))"
        "  öndeki: $(FgOwner)"
    }

    if ($Adimlar -contains "menu") {
        "── tepsi menüsü ($Tekrar kez, 2. monitörün sağ altında) ──"
        $p.Refresh(); $mem0 = $p.PrivateMemorySize64
        $x = 3800; $y = 1030
        $wp = [IntPtr]((($y -band 0xFFFF) -shl 16) -bor ($x -band 0xFFFF))
        $open = @()
        for ($n = 0; $n -lt $Tekrar; $n++) {
            [S]::PostMessage($main, $WM_TRAY, $wp, [IntPtr]$WM_CONTEXTMENU) | Out-Null
            $open += (WaitFor { ([S]::Popups($p.Id)).Count -gt 0 } 3000)
            $pop = ([S]::Popups($p.Id)) | Select-Object -First 1
            if (-not $pop) { "  menü açılmadı"; break }
            if ($n -eq 0) {
                Start-Sleep -Milliseconds 300
                ScreenShot $pop "sistem-menu" 0
                Key $pop 0x28 0x50  # ↓
                Key $pop 0x28 0x50  # ↓
                Start-Sleep -Milliseconds 200
                ScreenShot $pop "sistem-menu-klavye" 0
            } else { Start-Sleep -Milliseconds 150 }
            Key $pop 0x1B 1
            WaitFor { ([S]::Popups($p.Id)).Count -eq 0 } 3000 | Out-Null
            Start-Sleep -Milliseconds 150
        }
        "  menü görünene kadar (ms): $($open -join ', ')  ortanca $(Median $open)"
        Start-Sleep -Seconds 1; $p.Refresh()
        "  özel bellek: önce $([math]::Round($mem0 / 1MB, 1)) MB, $Tekrar açılıştan sonra $([math]::Round($p.PrivateMemorySize64 / 1MB, 1)) MB"
        "  öndeki: $(FgOwner)"
    }

    if ($Adimlar -contains "smtc") {
        "── Windows medya kontrolü ──"
        $waiter.WaitForExit(10000) | Out-Null
        Get-Content $waitOut -Encoding UTF8 | ForEach-Object { "  izleyici: $_" }
        & $ps5 -NoProfile -ExecutionPolicy Bypass -File $smtcScript -Mode sar -Kimlik "Openify.Test" | ForEach-Object { "  $_" }
        & $ps5 -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'smtc-oku.ps1') -Mode oku -Kimlik "Openify.Test"
        "  düğme: oynat/duraklat ($Tekrar kez, her biri yalnız Duraklat gönderir)"
        $out = & $ps5 -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'smtc-oku.ps1') -Mode dugme -Kimlik "Openify.Test" -Tekrar $Tekrar
        $out | ForEach-Object { "  $_" }
        "  görev çubuğu düğmesi (WM_COMMAND, 3 kez)"
        for ($n = 0; $n -lt 3; $n++) {
            "  thumb $([DateTime]::UtcNow.ToString('HH:mm:ss.fff'))"
            [S]::PostMessage($main, $WM_COMMAND, [IntPtr]((0x1800 -shl 16) -bor $THUMB_PLAY), [IntPtr]0) | Out-Null
            Start-Sleep -Milliseconds 300
        }
    }

    if ($Adimlar -contains "gizle") {
        "── tepsiye gizleme ──"
        $p.Refresh()
        $cpu0 = $p.TotalProcessorTime.TotalMilliseconds
        Start-Sleep -Seconds 20
        $p.Refresh()
        $vis = ($p.TotalProcessorTime.TotalMilliseconds - $cpu0)
        "  görünürken (sahte çalma) 20 sn: $([math]::Round($vis)) ms işlemci = %$([math]::Round($vis / 20000 * 100, 3)) (bir çekirdek)"
        $sw = [Diagnostics.Stopwatch]::StartNew()
        [S]::PostMessage($main, $WM_CLOSE, [IntPtr]0, [IntPtr]0) | Out-Null
        "  X → gizlendi: $(WaitFor { -not [S]::IsWindowVisible($main) } 3000) ms"
        Start-Sleep -Seconds 2
        $rows0 = (Get-Content $frames -ErrorAction SilentlyContinue | Measure-Object -Line).Lines
        $p.Refresh(); $cpu0 = $p.TotalProcessorTime.TotalMilliseconds
        Start-Sleep -Seconds $CpuSn
        $p.Refresh(); $cpu = $p.TotalProcessorTime.TotalMilliseconds - $cpu0
        $rows1 = (Get-Content $frames -ErrorAction SilentlyContinue | Measure-Object -Line).Lines
        $p.Refresh()
        "  gizliyken $CpuSn sn: $([math]::Round($cpu)) ms işlemci = %$([math]::Round($cpu / ($CpuSn * 1000) * 100, 3)) (bir çekirdek), çizilen kare: $($rows1 - $rows0), özel bellek $([math]::Round($p.PrivateMemorySize64 / 1MB, 1)) MB"
        $show = @()
        for ($n = 0; $n -lt 5; $n++) {
            $sw = [Diagnostics.Stopwatch]::StartNew()
            [S]::PostMessage($main, $WM_TRAY, [IntPtr]0, [IntPtr]$NIN_SELECT) | Out-Null
            $show += (WaitFor { [S]::IsWindowVisible($main) } 3000)
            Start-Sleep -Milliseconds 400
            if ($n -eq 0) { GiveBack; Start-Sleep -Milliseconds 200; ScreenShot $main "sistem-geri" 0 }
            [S]::PostMessage($main, $WM_CLOSE, [IntPtr]0, [IntPtr]0) | Out-Null
            WaitFor { -not [S]::IsWindowVisible($main) } 3000 | Out-Null
            Start-Sleep -Milliseconds 300
        }
        "  tepsi tıklaması → pencere görünür (ms): $($show -join ', ')  ortanca $(Median $show)"
        GiveBack
    }

    if ($Adimlar -contains "ikinci") {
        "── tek kopya: ikinci açılış (5 kez) ──"
        $front = @(); $exits = @()
        for ($n = 0; $n -lt 5; $n++) {
            if ([S]::IsWindowVisible($main)) {
                [S]::PostMessage($main, $WM_CLOSE, [IntPtr]0, [IntPtr]0) | Out-Null
                WaitFor { -not [S]::IsWindowVisible($main) } 3000 | Out-Null
                Start-Sleep -Milliseconds 300
            }
            $sw = [Diagnostics.Stopwatch]::StartNew()
            $q = Start-Process $exeCopy -PassThru
            $procs += $q
            $front += (WaitFor { [S]::IsWindowVisible($main) } 3000)
            $exits += (WaitFor { $q.HasExited } 2000) + $front[-1]
            Start-Sleep -Milliseconds 300
            GiveBack | Out-Null
        }
        Start-Sleep -Seconds 2
        $extra = @(Get-Process openify -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exeCopy -and $_.Id -ne $p.Id })
        "  ikinci açılış → var olan pencere görünür (ms): $($front -join ', ')  ortanca $(Median $front)"
        "  ikinci süreç kapandı (ms, başlatmadan): $($exits -join ', ')"
        "  2 sn sonra fazladan süreç: $($extra.Count)"
    }

    "öndeki: $(FgOwner)"
    "── kayıt (sistem satırları) ──"
    LogSince $logStart | Select-String -Pattern "sistem:|Openify zaten|kısayol|medya|tepsi|ERROR|WARN|panick" | ForEach-Object { $_.Line }
} finally {
    foreach ($q in $procs) { if (!$q.HasExited) { Stop-Process -Id $q.Id -ErrorAction SilentlyContinue } }
    Start-Sleep -Milliseconds 500
    foreach ($k in "OPENIFY_SETTINGS_PATH", "OPENIFY_TEST_DISPLAY", "OPENIFY_TEST_FAKE_PLAY", "OPENIFY_TEST_OFFLINE", "OPENIFY_TEST_SYSTEM", "OPENIFY_INSTANCE_ID", "OPENIFY_FRAME_STATS") {
        Remove-Item "Env:$k" -ErrorAction SilentlyContinue
    }
    Remove-Item $settings, $frames, $waitOut -ErrorAction SilentlyContinue
    Remove-Item $copyDir -Recurse -Force -ErrorAction SilentlyContinue
}
