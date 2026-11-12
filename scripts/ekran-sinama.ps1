# Ekran sınaması: Openify'ı 2. monitörde, sessiz (ses 0, sonra eski ayar geri
# yazılır) ve odak çalmadan açar; exe'nin kopyasını çalıştırır (derlemeyi
# kilitlemez). -Hover ile eylemler: "x,y" üzerine gel, "c:x,y" tıkla,
# "t:yazı", "k:vk,scan[,ext]" tuş, "w:x,y,çentik" tekerlek, "s:saniye" bekle (pencereye göre
# koordinatlar; görüntü koordinatından ~8 px sola, 1 px yukarı). Her eylemden
# sonra PrintWindow ile görüntü alır: %TEMP%\openify-ekran\<Out>-hoverN.png.
# Tek ekranlı makinede pencere ana ekranda açılır (odak yine çalınmaz).
#
# -Size mantıksal px (gpui px) cinsindendir. -Scale 1.25 / 1.5 / 2 uygulamanın
# ekran ölçeğini zorlar (OPENIFY_TEST_SCALE, vendor/gpui OPENIFY.md madde 8): tek
# %100'lük makinede %125-%200 ekranı taklit eder; pencere fiziksel olarak
# Size x Scale px olur. Pencere (fiziksel boyutuyla) ikinci ekrana sığmıyorsa
# (2560x1440, 1920x1080 @ %200 ...) hiçbir ekrana değmeyen yere (sanal
# masaüstünün sağ kenarının ötesi) konur; görüntü yine PrintWindow ile tam
# alınır, kullanıcının ekranlarında hiçbir şey görünmez. Ölçek verilmeyip Size
# ikinci ekrandan büyükse ölçek 1 sayılır. Görüntü ve -Hover koordinatları
# FİZİKSEL pikseldir (görüntüdeki gibi).
param(
    [string]$Size = "1180,760",
    [double]$Scale = 0,
    [int]$Wait = 12,
    [int]$PlayAfter = 0,
    [string]$Out = "shot",
    [int[]]$ShotsAt = @(),
    [string[]]$Hover = @(),
    # Sınanacak exe (varsayılan: target\release\openify.exe); kopyası çalıştırılır.
    [string]$Exe = "",
    # Bu koşunun geçici ayar dosyası (ses 0 kalsın; ör. son çalınanlarla:
    # '{ "volume": 0.0, "recent": ["liked"] }').
    [string]$SettingsJson = '{ "volume": 0.0 }'
)
$ErrorActionPreference = "Stop"
$scratch = Join-Path $env:TEMP 'openify-ekran'
New-Item -ItemType Directory -Force $scratch | Out-Null
$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")  # bu çalıştırmaya özel; gerçek settings.json'a dokunulmaz

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr dc);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
  [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr dc, IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr o);
  [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
  // PrintWindow düz GDI bitmap'ine; System.Drawing'e sonra `FromHbitmap` ile
  // geçilir. (`Graphics.GetHdc()` KULLANILMAZ: GDI+ o yolda #0D0B0C rengini
  // kendi saydamlık anahtarı sayıp (0,0,0,0) yapıyor; panelin gölgesinde bu
  // renge düşen pikseller "şeffaf delik" gibi görünüyordu, 7 Kasım.)
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
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[W]::SetProcessDPIAware() | Out-Null

function Shot($proc, $name) {
    $proc.Refresh()
    if ($proc.HasExited) { return "no shot $name (exited)" }
    $h = $proc.MainWindowHandle
    $r = New-Object W+RECT
    [W]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $hh = $r.B - $r.T
    # Pencerenin kendisini çiz (üstünde başka pencere olsa da).
    $hbmp = [W]::Grab($h, $w, $hh)
    $bmp = [System.Drawing.Image]::FromHbitmap($hbmp)
    [W]::DeleteObject($hbmp) | Out-Null
    $bmp.Save("$scratch\$name.png")
    $bmp.Dispose()
    "shot $name at $($r.L),$($r.T) ${w}x$hh"
}

try {
    [IO.File]::WriteAllText($settings, $SettingsJson, (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    $fg = [W]::GetForegroundWindow()
    $env:OPENIFY_TEST_DISPLAY = $Size
    $sz = $Size.Split(',') | ForEach-Object { [double]$_ }
    $scr = [System.Windows.Forms.Screen]::AllScreens | Where-Object { -not $_.Primary } | Select-Object -First 1
    if (-not $scr) { $scr = [System.Windows.Forms.Screen]::PrimaryScreen }
    if ($Scale -le 0 -and ($sz[0] -gt $scr.Bounds.Width -or $sz[1] -gt $scr.Bounds.Height)) { $Scale = 1 }
    if ($Scale -gt 0) { $env:OPENIFY_TEST_SCALE = "$Scale" } else { Remove-Item Env:OPENIFY_TEST_SCALE -ErrorAction SilentlyContinue }
    if ($PlayAfter -gt 0) { $env:OPENIFY_TEST_PLAY_AFTER = "$PlayAfter" } else { Remove-Item Env:OPENIFY_TEST_PLAY_AFTER -ErrorAction SilentlyContinue }
    # Önbellek ve günlük her zaman geçici klasörde (gerçek %LOCALAPPDATA%\Openify'a dokunulmaz).
    if (-not $env:OPENIFY_TEST_CACHE_DIR) { $env:OPENIFY_TEST_CACHE_DIR = Join-Path $env:TEMP 'openify-sandbox\cache' }
    if (-not $env:OPENIFY_LOG_DIR) { $env:OPENIFY_LOG_DIR = Join-Path $env:TEMP 'openify-sandbox\log' }
    New-Item -ItemType Directory -Force $env:OPENIFY_TEST_CACHE_DIR, $env:OPENIFY_LOG_DIR | Out-Null
    $logPath = Join-Path $env:OPENIFY_LOG_DIR 'openify.log'
    $logBefore = if (Test-Path $logPath) { (Get-Item $logPath).Length } else { 0 }
    # Derlemeyi kilitlememek için exe'nin kopyası çalıştırılır.
    $copyDir = Join-Path $env:TEMP ('openify-test-' + [guid]::NewGuid().ToString('N').Substring(0,8)); New-Item -ItemType Directory -Force $copyDir | Out-Null
    $srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
    Copy-Item $srcExe (Join-Path $copyDir 'openify.exe')
    $p = Start-Process (Join-Path $copyDir 'openify.exe') -PassThru
    # Pencere görünene kadar bekle, sonra odağı kullanıcının penceresine geri ver.
    for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    Start-Sleep -Milliseconds 300
    "focus restored: $([W]::Restore($fg))"
    $elapsed = 0
    foreach ($t in $ShotsAt) {
        Start-Sleep -Seconds ($t - $elapsed); $elapsed = $t
        Shot $p "$Out-$t"
    }
    if ($Wait -gt $elapsed) { Start-Sleep -Seconds ($Wait - $elapsed) }
    Shot $p $Out
    # Gerçek imleci oynatmadan pencereye "fare burada" mesajı gönder.
    $n = 0
    foreach ($pt in $Hover) {
        # "x,y" = üzerine gel, "c:x,y" = tıkla, "t:yazı" = klavyeyle yaz
        $h = $p.MainWindowHandle
        if ($pt.StartsWith("s:")) {
            # "s:saniye" = hiçbir şey yapmadan bekle (gecikmeli durumları görmek için)
            Start-Sleep -Seconds ([int]$pt.Substring(2))
        } elseif ($pt.StartsWith("w:")) {
            # "w:x,y,çentik" = (pencereye göre x,y) fare tekerleği, eksi = aşağı
            $x, $y, $notch = $pt.Substring(2).Split(',') | ForEach-Object { [int]$_ }
            $r = New-Object W+RECT; [W]::GetWindowRect($h, [ref]$r) | Out-Null
            $sx = $r.L + 8 + $x; $sy = $r.T + 1 + $y
            $lp = [IntPtr]((($sy -band 0xFFFF) -shl 16) -bor ($sx -band 0xFFFF))
            $wp = [IntPtr][long]((($notch * 120) -band 0xFFFF) -shl 16)
            [W]::PostMessage($h, 0x020A, $wp, $lp) | Out-Null
        } elseif ($pt.StartsWith("k:")) {
            # "k:vk,scan[,ext]"
            $parts = $pt.Substring(2).Split(',') | ForEach-Object { [int]$_ }
            $vk = $parts[0]; $scan = $parts[1]; $ext = if ($parts.Count -gt 2) { 1 -shl 24 } else { 0 }
            $down = 1 -bor ($scan -shl 16) -bor $ext
            [W]::PostMessage($h, 0x0100, [IntPtr]$vk, [IntPtr]$down) | Out-Null
            Start-Sleep -Milliseconds 60
            [W]::PostMessage($h, 0x0101, [IntPtr]$vk, [IntPtr]([long]$down -bor 0xC0000000L)) | Out-Null
        } elseif ($pt.StartsWith("r:")) {
            # "r:x,y" = sağ tık (bağlam menüsü)
            $x, $y = $pt.Substring(2).Split(',') | ForEach-Object { [int]$_ }
            $lp = [IntPtr](($y -shl 16) -bor $x)
            [W]::PostMessage($h, 0x0200, [IntPtr]0, $lp) | Out-Null; Start-Sleep -Milliseconds 60
            [W]::PostMessage($h, 0x0204, [IntPtr]2, $lp) | Out-Null; Start-Sleep -Milliseconds 60
            [W]::PostMessage($h, 0x0205, [IntPtr]0, $lp) | Out-Null
        } elseif ($pt.StartsWith("d:")) {
            # "d:x1,y1,x2,y2" = sol tuşla sürükle bırak (8 adımda)
            $x1, $y1, $x2, $y2 = $pt.Substring(2).Split(',') | ForEach-Object { [int]$_ }
            $lp = [IntPtr](($y1 -shl 16) -bor $x1)
            [W]::PostMessage($h, 0x0200, [IntPtr]0, $lp) | Out-Null; Start-Sleep -Milliseconds 60
            [W]::PostMessage($h, 0x0201, [IntPtr]1, $lp) | Out-Null; Start-Sleep -Milliseconds 60
            for ($s = 1; $s -le 8; $s++) {
                $x = [int]($x1 + ($x2 - $x1) * $s / 8); $y = [int]($y1 + ($y2 - $y1) * $s / 8)
                [W]::PostMessage($h, 0x0200, [IntPtr]1, [IntPtr](($y -shl 16) -bor $x)) | Out-Null; Start-Sleep -Milliseconds 40
            }
            [W]::PostMessage($h, 0x0202, [IntPtr]0, [IntPtr](($y2 -shl 16) -bor $x2)) | Out-Null
        } elseif ($pt.StartsWith("xb:")) {
            # "xb:1,x,y" = farenin geri (1) / ileri (2) yan tuşu
            $b, $x, $y = $pt.Substring(3).Split(',') | ForEach-Object { [int]$_ }
            $lp = [IntPtr](($y -shl 16) -bor $x)
            [W]::PostMessage($h, 0x020B, [IntPtr](($b -shl 16) -bor (0x20 * $b)), $lp) | Out-Null; Start-Sleep -Milliseconds 60
            [W]::PostMessage($h, 0x020C, [IntPtr]($b -shl 16), $lp) | Out-Null
        } elseif ($pt.StartsWith("t:")) {
            foreach ($ch in $pt.Substring(2).ToCharArray()) { [W]::PostMessage($h, 0x0102, [IntPtr][int]$ch, [IntPtr]1) | Out-Null; Start-Sleep -Milliseconds 40 }
        } else {
            $click = $pt.StartsWith("c:")
            $x, $y = $pt.TrimStart('c', ':').Split(',') | ForEach-Object { [int]$_ }
            $lp = [IntPtr](($y -shl 16) -bor $x)
            [W]::PostMessage($h, 0x0200, [IntPtr]0, $lp) | Out-Null
            if ($click) { Start-Sleep -Milliseconds 60; [W]::PostMessage($h, 0x0201, [IntPtr]1, $lp) | Out-Null; Start-Sleep -Milliseconds 60; [W]::PostMessage($h, 0x0202, [IntPtr]0, $lp) | Out-Null }
        }
        Start-Sleep -Milliseconds 450
        Shot $p "$Out-hover$n"
        $n++
    }
    "focus still with user: $([W]::GetForegroundWindow() -eq $fg)"
    if (!$p.HasExited) { Stop-Process -Id $p.Id } else { "EXITED $($p.ExitCode)" }
    $fs = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
    $fs.Seek($logBefore, 'Begin') | Out-Null
    (New-Object System.IO.StreamReader($fs)).ReadToEnd()
    $fs.Close()
} finally {
    Start-Sleep -Milliseconds 500
    Remove-Item Env:OPENIFY_SETTINGS_PATH -ErrorAction SilentlyContinue
    Remove-Item Env:OPENIFY_TEST_SCALE -ErrorAction SilentlyContinue
    Remove-Item $settings -ErrorAction SilentlyContinue
    if ($copyDir) { Remove-Item $copyDir -Recurse -Force -ErrorAction SilentlyContinue }
}
