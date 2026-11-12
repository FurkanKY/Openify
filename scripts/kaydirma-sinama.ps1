# Kaydırma eşzamanlılığı sınaması (raf + liste). Openify'ı -Runs kez açar
# (2. monitörde, sessiz: geçici ayar dosyasıyla ses 0, gerçek ayar dosyasına
# dokunulmaz; odak çalmadan, gerçek fare / klavye oynamadan, pencereye mesaj
# gönderilir; exe'nin kopyası), açılıştan -Wait sn sonra senaryoyu oynatır,
# hareket otursun diye -Settle sn bekler, görüntü alır ve kapatır.
#
# Senaryolar (-Scenario):
#   wheel  listenin üstüne tek tekerlek olayı (-Notches çentik, eksi = aşağı)
#   keys   raf açıkken -Keys kez ↓ (satır seçimi listeyi kaydırır), sonra tekerlek
#   short  tekerlek, sonra pencere kısa boya (< 640 px) ve geri (raf
#          kısa pencerede kapanır, uzunda açılırdı; liste kaymışken açılmamalı)
#   bar    raf açıkken kaydırma çubuğunun yoluna tıkla (bir sayfa aşağı),
#          sonra tekerlek (çizimdeki onarımı sınar)
#   all    dördü sırayla, her biri -Runs kez
#
# Her koşu OPENIFY_SCROLL_TRACE iziyle (bkz. src/ui/library.rs →
# scroll_trace) denetlenir. Değişmez: raf/başlık tam kapanmamışken
# (collapse < en çok) liste en üstte (offset = 0). İlk eylemden (tekerlek,
# satır, pencere boyu) sonraki herhangi bir satırda offset > 0,5 px iken
# collapse < en çok - 0,5 px ise koşu BOZUK. Eylem izde yoksa "işlenmedi"
# (sınama geçersiz; ayrıca yazılır).
#
# Çıktı: %TEMP%\openify-kaydirma\<Out>-<senaryo>-NN.csv (iz) ve .png (görüntü),
# sonda senaryo başına "bozuk x/N". Bozuk koşu varsa çıkış kodu 1.
# Örnek: & .\scripts\kaydirma-sinama.ps1 -Runs 50
#        & .\scripts\kaydirma-sinama.ps1 -Scenario all -Runs 10 -NoShots
# Önce: cargo build --release
param(
    [ValidateSet("wheel", "keys", "short", "bar", "all")] [string]$Scenario = "wheel",
    [int]$Runs = 50,
    [string]$Size = "1180,760",
    [double]$Wait = 5,
    [int]$Notches = -3,
    # Tekerleğin çevrileceği yer (pencereye göre): listenin üstü.
    [int]$WheelX = 600,
    [int]$WheelY = 450,
    # keys: kaç kez ↓.
    [int]$Keys = 12,
    # short: geçici pencere yüksekliği (px, < 640).
    [int]$ShortHeight = 600,
    [double]$Settle = 1.5,
    [string]$Out = "run",
    # Her koşuda görüntü al (varsayılan açık; -NoShots ile kapanır).
    [switch]$NoShots,
    # Sınanacak exe (varsayılan: target\release\openify.exe); kopyası çalıştırılır.
    [string]$Exe = "",
    # Uygulamayı açmadan, önceki koşuların izlerini (aynı -Out / -Scenario /
    # -Runs) yeniden denetle.
    [switch]$CheckOnly
)
$ErrorActionPreference = "Stop"
$outDir = Join-Path $env:TEMP 'openify-kaydirma'
New-Item -ItemType Directory -Force $outDir | Out-Null
$binDir = $null
if (-not $CheckOnly) {
    $srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
    if (-not (Test-Path $srcExe)) { throw "exe bulunamadı: $srcExe (önce cargo build --release)" }
    $binDir = Join-Path $outDir ("bin\" + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force $binDir | Out-Null
    $runExe = Join-Path $binDir 'openify.exe'
    Copy-Item $srcExe $runExe
}

# Kullanıcının gerçek ayar dosyasına (%APPDATA%\Openify\settings.json) hiç
# dokunulmaz: uygulama OPENIFY_SETTINGS_PATH ile geçici bir dosyayı (ses 0)
# okur/yazar. (Aynı anda koşan sınamalar gerçek dosyayı "kaydet, 0 yaz, geri
# yaz" yarışıyla bozabiliyordu.)
$settings = Join-Path $outDir 'settings.json'

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class KS {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
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
}
"@
[KS]::SetProcessDPIAware() | Out-Null

function Shot($proc, $path) {
    $proc.Refresh()
    if ($proc.HasExited) { return }
    $h = $proc.MainWindowHandle
    $r = New-Object KS+RECT
    [KS]::GetWindowRect($h, [ref]$r) | Out-Null
    $hbmp = [KS]::Grab($h, ($r.R - $r.L), ($r.B - $r.T))
    $bmp = [System.Drawing.Image]::FromHbitmap($hbmp)
    [KS]::DeleteObject($hbmp) | Out-Null
    $bmp.Save($path)
    $bmp.Dispose()
}

# Gerçek imleci oynatmadan pencereye tekerlek mesajı (ekran koordinatı).
function Wheel($h) {
    $r = New-Object KS+RECT; [KS]::GetWindowRect($h, [ref]$r) | Out-Null
    $sx = $r.L + 8 + $WheelX; $sy = $r.T + 1 + $WheelY
    $lp = [IntPtr]((($sy -band 0xFFFF) -shl 16) -bor ($sx -band 0xFFFF))
    $wp = [IntPtr][long]((($Notches * 120) -band 0xFFFF) -shl 16)
    [KS]::PostMessage($h, 0x020A, $wp, $lp) | Out-Null
}

# ↓ tuşu (VK_DOWN, tarama kodu 0x50, genişletilmiş tuş).
function KeyDown($h) {
    $down = 1 -bor (0x50 -shl 16) -bor (1 -shl 24)
    [KS]::PostMessage($h, 0x0100, [IntPtr]0x28, [IntPtr]$down) | Out-Null
    Start-Sleep -Milliseconds 30
    [KS]::PostMessage($h, 0x0101, [IntPtr]0x28, [IntPtr]([long]$down -bor 0xC0000000L)) | Out-Null
    Start-Sleep -Milliseconds 50
}

# Pencerenin boyunu değiştir (yerinden oynatmadan, öne getirmeden).
function Resize($h, $height) {
    $r = New-Object KS+RECT; [KS]::GetWindowRect($h, [ref]$r) | Out-Null
    # SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
    [KS]::SetWindowPos($h, [IntPtr]::Zero, 0, 0, $r.R - $r.L, $height, 0x0002 -bor 0x0004 -bor 0x0010) | Out-Null
}

# İzi denetle: (durum, açıklama). Satır: t_ms,event,collapse,offset,detail
function Check($csv, $sc) {
    if (-not (Test-Path $csv)) { return @('iz-yok', 'iz dosyası yok') }
    $max = $null; $acted = $false; $bad = $null; $last = $null; $steps = 0; $heals = 0; $rows = 0; $flips = 0
    # Süreç kapanırken dosya hâlâ açık olabilir: paylaşımlı oku.
    $fs = [System.IO.File]::Open($csv, 'Open', 'Read', 'ReadWrite')
    $text = (New-Object System.IO.StreamReader($fs)).ReadToEnd()
    $fs.Close()
    $inv = [Globalization.CultureInfo]::InvariantCulture
    foreach ($line in ($text -split "`r?`n")) {
        $f = $line.Split(',', 5)
        if ($f.Count -lt 4 -or $f[0] -eq 't_ms') { continue }
        $ev = $f[1]
        $c = [double]::Parse($f[2], $inv)
        $o = [double]::Parse($f[3], $inv)
        if ($ev -eq 'start') { $max = [double]::Parse(($f[4] -replace 'collapse_max=', ''), $inv); continue }
        if ($ev -in @('wheel', 'row', 'short', 'bar')) { $acted = $true }
        if (-not $acted) { continue }
        if ($ev -eq 'step') { $steps++ }
        if ($ev -eq 'heal') { $heals++ }
        if ($ev -eq 'row') { $rows++ }
        if ($ev -eq 'short') { $flips++ }
        $last = "collapse=$c offset=$o"
        if ($null -eq $bad -and $o -gt 0.5 -and $c -lt $max - 0.5) { $bad = "$ev t=$($f[0]) ms collapse=$c offset=$o" }
    }
    # Senaryonun eylemi gerçekten işlendi mi (tekerlek adım attı, ↓ satır
    # seçti, pencere kısa boya geçti)? Yoksa sınama geçersiz.
    $missing = switch ($sc) {
        'keys' { if ($rows -eq 0) { 'satır seçimi izde yok' } }
        'short' { if ($flips -eq 0) { 'kısa pencere geçişi izde yok' } }
    }
    if (-not $acted) { $missing = 'eylem izde yok' }
    elseif ($steps -eq 0) { $missing = 'tekerlek adım atmadı' }
    if ($missing) { return @('islenmedi', $missing) }
    if ($bad) { return @('bozuk', "ilk bozuk satır: $bad; son: $last") }
    return @('iyi', "adım $steps, onarım $heals; son: $last")
}

$scenarios = if ($Scenario -eq 'all') { @('wheel', 'keys', 'short', 'bar') } else { @($Scenario) }
$summary = @()
$anyBad = $false
try {
    Set-Content $settings '{ "volume": 0.0 }' -NoNewline
    $env:OPENIFY_SETTINGS_PATH = $settings
    $fg = [KS]::GetForegroundWindow()
    $env:OPENIFY_TEST_DISPLAY = $Size
    Remove-Item Env:OPENIFY_TEST_PLAY_AFTER, Env:OPENIFY_TEST_FAKE_PLAY -ErrorAction SilentlyContinue
    foreach ($sc in $scenarios) {
        $counts = [ordered]@{ iyi = 0; bozuk = 0; islenmedi = 0; 'iz-yok' = 0 }
        $badRuns = @()
        for ($run = 1; $run -le $Runs; $run++) {
            $name = '{0}-{1}-{2:D2}' -f $Out, $sc, $run
            $csv = Join-Path $outDir "$name.csv"
            if ($CheckOnly) {
                $state, $why = Check $csv $sc
                $counts[$state]++
                if ($state -ne 'iyi') { $badRuns += "$name ${state}: $why" }
                continue
            }
            Remove-Item $csv -ErrorAction SilentlyContinue
            $env:OPENIFY_SCROLL_TRACE = $csv
            $p = Start-Process $runExe -PassThru
            for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
            Start-Sleep -Milliseconds 300
            [KS]::Restore($fg) | Out-Null
            Start-Sleep -Milliseconds ([int]($Wait * 1000))
            $h = $p.MainWindowHandle
            switch ($sc) {
                'wheel' { Wheel $h }
                'keys' {
                    for ($k = 0; $k -lt $Keys; $k++) { KeyDown $h }
                    Start-Sleep -Milliseconds 600
                    Wheel $h
                }
                'bar' {
                    # Kaydırma çubuğunun yoluna, tutamacın altına tıkla (bir
                    # sayfa aşağı; pencereye göre istemci koordinatı).
                    $r = New-Object KS+RECT; [KS]::GetWindowRect($h, [ref]$r) | Out-Null
                    $cx = ($r.R - $r.L) - 16 - 3; $cy = $WheelY + 150
                    $lp = [IntPtr](($cy -shl 16) -bor $cx)
                    [KS]::PostMessage($h, 0x0200, [IntPtr]0, $lp) | Out-Null
                    Start-Sleep -Milliseconds 60
                    [KS]::PostMessage($h, 0x0201, [IntPtr]1, $lp) | Out-Null
                    Start-Sleep -Milliseconds 60
                    [KS]::PostMessage($h, 0x0202, [IntPtr]0, $lp) | Out-Null
                    Start-Sleep -Milliseconds 600
                    Wheel $h
                }
                'short' {
                    Wheel $h
                    Start-Sleep -Milliseconds ([int]($Settle * 1000))
                    $r = New-Object KS+RECT; [KS]::GetWindowRect($h, [ref]$r) | Out-Null
                    $tall = $r.B - $r.T
                    Resize $h $ShortHeight
                    Start-Sleep -Milliseconds 600
                    Resize $h $tall
                }
            }
            Start-Sleep -Milliseconds ([int]($Settle * 1000))
            if (-not $NoShots) { Shot $p (Join-Path $outDir "$name.png") }
            if (!$p.HasExited) { Stop-Process -Id $p.Id; $p.WaitForExit(3000) | Out-Null }
            $state, $why = Check $csv $sc
            $counts[$state]++
            if ($state -ne 'iyi') { $badRuns += "$name ${state}: $why" }
            "{0} {1,-9} {2}" -f $name, $state, $why
        }
        if ($counts['bozuk'] -gt 0) { $anyBad = $true }
        $summary += "${sc}: bozuk $($counts['bozuk'])/$Runs, iyi $($counts['iyi']), işlenmedi $($counts['islenmedi']), iz yok $($counts['iz-yok'])"
        $summary += $badRuns
    }
    "odak kullanıcıda: $([KS]::GetForegroundWindow() -eq $fg)"
} finally {
    Remove-Item Env:OPENIFY_SCROLL_TRACE, Env:OPENIFY_TEST_DISPLAY, Env:OPENIFY_SETTINGS_PATH -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    if ($binDir) { Remove-Item -LiteralPath $binDir -Recurse -Force -ErrorAction SilentlyContinue }
}
""
"özet:"
$summary
"izler ve görüntüler: $outDir"
if ($anyBad) { exit 1 }
