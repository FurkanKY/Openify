# Openify gerçek çalma doğrulama oturumu (tek koşu).
#
# Uygulamayı (exe'nin %TEMP% kopyasını) 2. monitörde, odak çalmadan açar; bir
# Beğenilenler satırını GERÇEKTEN çaldırır ve sırayla: 15 sn ısınma, 5x
# duraklat/devam (Boşluk), 6 ses düzeyi adımı (Ctrl+Aşağı x2, Ctrl+Yukarı x2,
# Ctrl+Shift+Aşağı x2 = sessize al / aç), 5x geri sarma (Shift+Sol), N dk soak
# (isteğe bağlı işlemci yüküyle), sonra şarkı sonuna ~20 sn kalana dek ileri
# sarma (Shift+Sağ) ve kendiliğinden sıradaki şarkıya geçiş + 15 sn. Hepsini
# pencereye gönderilmiş klavye mesajlarıyla yapar (gerçek klavye / fare yok).
#
# Ses düzeyi 0 DEĞİL: ses düzeyi / kazanç değişimlerini sınamak için duyulur
# çalınır. Gerçek settings.json'a DOKUNULMAZ: bu koşuya özel geçici bir ayar
# dosyası (%TEMP%\openify-ayar-*.json, içinde -Volume düzeyi) yazılır ve
# OPENIFY_SETTINGS_PATH ile uygulamaya gösterilir; oturum bitince silinir.
# Çıktılar:
#   %TEMP%\openify-ses\ses-<damga>.csv        OPENIFY_SINK_TRACE izi
#   %TEMP%\openify-ses\ses-<damga>.log        oturumun log dilimi
#   %TEMP%\openify-ses\ses-<damga>.marks.csv  faz zamanları (soak penceresi vb.)
# Bitince scripts\ses_oturumu_ozet.py kendiliğinden çalıştırılır (python varsa).
#
# Spotify'ı boğma: 30 dk soak ~8-10 şarkıya denk gelir (şarkı başına bir anahtar
# isteği); bu oturumu gereksiz yere yineleme.
#
#   powershell -File scripts\ses-oturumu.ps1 -Row 12
#   powershell -File scripts\ses-oturumu.ps1 -Row 12 -SoakMinutes 30 -Load
#   powershell -File scripts\ses-oturumu.ps1 -Silent -SoakMinutes 1 -TransitionStallMs 3000
param(
    # Beğenilenler listesinin çalınacak satırı (0'dan).
    [int]$Row = 0,
    [double]$SoakMinutes = 30,
    # Soak boyunca her mantıksal çekirdek için bir meşgul PowerShell işi.
    [switch]$Load,
    [string]$Exe = "$PSScriptRoot\..\target\release\openify.exe",
    # Eylemler arası en az bekleme (sn).
    [double]$StepGap = 2.5,
    # Açılıştan ("loaded") sonra ilk eyleme kadar (sn).
    [int]$Warmup = 15,
    # Yük işlerinin öncelik sınıfı: kullanıcının 1. monitördeki işi donmasın diye
    # varsayılan BelowNormal; gerçek çekişme için Normal.
    [ValidateSet("Idle", "BelowNormal", "Normal")] [string]$LoadPriority = "BelowNormal",
    [string]$OutDir = (Join-Path $env:TEMP 'openify-ses'),
    # Geçici ayar dosyasına yazılan ses düzeyi (duyulur olmalı: >= 0.12; altında
    # ses adımları sınırda kırpılır ve 'duyulur' ölçümü anlamsız olur).
    [ValidateRange(0.12, 1.0)] [double]$Volume = 0.5,
    # Sessiz koşu: ses düzeyi 0, ses düzeyi evresi atlanır (kullanıcı dinlemek
    # zorunda kalmasın; duraklat / sarma / konum / boşluksuz geçiş / soak
    # ölçümleri ses düzeyinden bağımsız: "sessiz" / "duyulur" anları zarftan
    # gelir, örnek enerjisinden değil). Ses düzeyi ölçütü "ölçülmedi" kalır.
    [switch]$Silent,
    # Her yeni şarkının ilk paketinden önce çalar bu kadar uyur
    # (OPENIFY_TEST_TRANSITION_STALL_MS; diskte takılan çözücü taklidi). Şarkı
    # sonu payı 4 sn: 3000 ms'de geçişte 0 xrun beklenir. 0: kapalı.
    [ValidateRange(0, 10000)] [int]$TransitionStallMs = 0
)
$ErrorActionPreference = "Stop"

# ---------------------------------------------------------------- yollar
$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")
$logPath = "$env:LOCALAPPDATA\Openify\openify.log"
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
New-Item -ItemType Directory -Force $OutDir | Out-Null
$csv = Join-Path $OutDir "ses-$stamp.csv"
$logOut = Join-Path $OutDir "ses-$stamp.log"
$marksOut = Join-Path $OutDir "ses-$stamp.marks.csv"
$analyzer = Join-Path $PSScriptRoot 'ses_oturumu_ozet.py'
if (-not (Test-Path $Exe)) { throw "exe bulunamadı: $Exe (önce cargo build --release)" }
$srcExe = (Resolve-Path $Exe).Path
if (-not (Test-Path $logPath)) { throw "log dosyası yok: $logPath (Openify hiç çalıştırılmamış?)" }

# ---------------------------------------------------------------- Win32
Add-Type -AssemblyName System.Windows.Forms
if (-not ('SesOturumu' -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class SesOturumu {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll")] public static extern bool GetKeyboardState(byte[] s);
  [DllImport("user32.dll")] public static extern bool SetKeyboardState(byte[] s);
  [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint f);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
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
  // Pencereye WM_KEYDOWN / WM_KEYUP gönderir (sanal tuş + tarama kodu). gpui
  // Windows'ta Ctrl / Shift durumunu GetKeyState ile okur; gönderilmiş mesajlar
  // bu durumu değiştirmez. Bu yüzden değiştirici gerekiyorsa: uygulamanın pencere
  // iş parçacığının girdi durumuna bağlanılır (AttachThreadInput), onun klavye
  // durumu tablosunda Ctrl / Shift işaretlenir (SetKeyboardState), tuş mesajı
  // gönderilir, işlendikten sonra tablo eski haline getirilip bağ çözülür.
  // Kullanıcının gerçek klavye durumuna dokunmaz (başka iş parçacığı kuyruğu).
  // Dönüş: bağlanma başarılı mı (değiştirici istenmediyse hep true).
  public static bool Key(IntPtr h, ushort vk, ushort scan, bool ext, bool ctrl, bool shift, int holdMs) {
    uint tid = GetWindowThreadProcessId(h, IntPtr.Zero);
    uint me = GetCurrentThreadId();
    bool mods = ctrl || shift;
    bool attached = false;
    byte[] orig = null;
    if (mods) {
      attached = AttachThreadInput(me, tid, true);
      orig = new byte[256];
      GetKeyboardState(orig);
      byte[] ks = (byte[])orig.Clone();
      if (ctrl)  { ks[0x11] |= 0x80; ks[0xA2] |= 0x80; }
      if (shift) { ks[0x10] |= 0x80; ks[0xA0] |= 0x80; }
      SetKeyboardState(ks);
    }
    long lp = 1L | ((long)scan << 16) | (ext ? (1L << 24) : 0L);
    PostMessage(h, 0x0100, (IntPtr)vk, (IntPtr)lp);
    Thread.Sleep(holdMs);
    PostMessage(h, 0x0101, (IntPtr)vk, (IntPtr)(lp | 0xC0000000L));
    Thread.Sleep(holdMs);
    if (mods) {
      SetKeyboardState(orig);
      if (attached) AttachThreadInput(me, tid, false);
    }
    return attached || !mods;
  }
}
"@
}
[SesOturumu]::SetProcessDPIAware() | Out-Null

# ---------------------------------------------------------------- yardımcılar
$script:p = $null
$script:hwnd = [IntPtr]::Zero
$script:userFg = [IntPtr]::Zero
$script:focusFixes = 0
$script:rows = New-Object 'System.Collections.Generic.List[object]'
$script:traceOff = [long]0
$script:lastTick = $null
$script:missed = 0
$script:seekNetMs = 0
$script:phases = [ordered]@{}
$script:jobs = @()

function NowMs { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
function Say([string]$m) { Write-Host ("{0:HH:mm:ss.fff}  {1}" -f (Get-Date), $m) }
function Phase-Start([string]$n) { $script:phases[$n] = @((NowMs), $null); Say "== faz: $n" }
function Phase-End([string]$n) { if ($script:phases.Contains($n)) { $script:phases[$n][1] = NowMs } }

function Read-Chunk([string]$path, [long]$offset) {
    $fs = [IO.File]::Open($path, 'Open', 'Read', 'ReadWrite')
    try {
        $len = $fs.Length
        if ($len -le $offset) { return [pscustomobject]@{ Buf = (New-Object byte[] 0); N = 0 } }
        $fs.Seek($offset, 'Begin') | Out-Null
        $buf = New-Object byte[] ($len - $offset)
        $n = $fs.Read($buf, 0, $buf.Length)
        return [pscustomobject]@{ Buf = $buf; N = $n }
    } finally { $fs.Close() }
}

function Get-LogSlice {
    $c = Read-Chunk $logPath $logBefore
    if ($c.N -eq 0) { return '' }
    return [Text.Encoding]::UTF8.GetString($c.Buf, 0, $c.N)
}

# İzi kaldığı yerden okur (yalnız tam satırlar); tick olmayan satırları $script:rows'a,
# son tick'i $script:lastTick'e yazar.
function Update-Trace {
    if (-not (Test-Path $csv)) { return }
    $c = Read-Chunk $csv $script:traceOff
    if ($c.N -eq 0) { return }
    $last = [Array]::LastIndexOf($c.Buf, [byte]10, $c.N - 1)
    if ($last -lt 0) { return }
    $text = [Text.Encoding]::UTF8.GetString($c.Buf, 0, $last + 1)
    $script:traceOff += ($last + 1)
    $inv = [Globalization.CultureInfo]::InvariantCulture
    foreach ($line in $text.Split("`n")) {
        $line = $line.TrimEnd("`r")
        if (-not $line) { continue }
        $f = $line.Split(',')
        if ($f.Count -lt 7 -or $f[0] -eq 't_unix_ms') { continue }
        $t = [double]::Parse($f[0], $inv)
        if ($f[1] -eq 'tick') {
            $script:lastTick = [pscustomobject]@{
                t = $t; q = [double]::Parse($f[2], $inv); xr = [int]$f[3]
                target = [double]::Parse($f[5], $inv); under = [double]::Parse($f[6], $inv)
            }
        } else {
            $script:rows.Add([pscustomobject]@{ t = $t; ev = $f[1] })
        }
    }
}

function Find-Ev([string]$ev, [double]$since) {
    foreach ($r in $script:rows) { if ($r.ev -eq $ev -and $r.t -ge $since) { return $r } }
    return $null
}
function Count-Ev([string]$ev, [double]$since) {
    $n = 0
    foreach ($r in $script:rows) { if ($r.ev -eq $ev -and $r.t -ge $since) { $n++ } }
    return $n
}
function Wait-Trace([string]$ev, [double]$since, [double]$timeoutSec) {
    $end = (Get-Date).AddSeconds($timeoutSec)
    do {
        Update-Trace
        if (Find-Ev $ev $since) { return $true }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $end)
    return $false
}

function Assert-Alive {
    $script:p.Refresh()
    if ($script:p.HasExited) { throw "Openify beklenmedik biçimde kapandı (çıkış kodu $($script:p.ExitCode))" }
}

# Uygulama etkinleşip odağı almışsa (kullanıcının penceresi önde değilse) geri ver.
function Keep-Focus {
    if ($script:hwnd -eq [IntPtr]::Zero -or $script:userFg -eq [IntPtr]::Zero) { return }
    $cur = [SesOturumu]::GetForegroundWindow()
    if ($cur -eq $script:hwnd -and $script:userFg -ne $script:hwnd) {
        [SesOturumu]::Restore($script:userFg) | Out-Null
        $script:focusFixes++
    }
}

# Bekle: yarım saniyelik dilimlerle; odak + canlılık denetimi, ~2 sn'de bir iz okuma.
function Wait-Sec([double]$sec) {
    $end = (Get-Date).AddSeconds($sec)
    $nextRead = Get-Date
    while ((Get-Date) -lt $end) {
        Assert-Alive
        Keep-Focus
        if ((Get-Date) -ge $nextRead) { Update-Trace; $nextRead = (Get-Date).AddSeconds(2) }
        $left = ($end - (Get-Date)).TotalMilliseconds
        if ($left -gt 0) { Start-Sleep -Milliseconds ([int][math]::Min(500, $left)) }
    }
}

# Bir tuş eylemi gönderir ve izde beklenen komut satırını (pause/play/seek/volume)
# bekler: tuşun uygulamaya ulaşıp ulaşmadığının kanıtı. Dönüş: görüldü mü.
function Send-Step([string]$name, [int]$vk, [int]$scan, [bool]$ext, [bool]$ctrl, [bool]$shift, [string]$expect) {
    Update-Trace
    $t0 = (NowMs) - 20
    $attached = [SesOturumu]::Key($script:hwnd, [uint16]$vk, [uint16]$scan, $ext, $ctrl, $shift, 80)
    $seen = Wait-Trace $expect $t0 4
    if (-not $seen) { $script:missed++ }
    $msg = if ($seen) { "izde '$expect' görüldü" } else { "İZDE '$expect' GÖRÜLMEDİ (tuş ulaşmamış olabilir)" }
    if (-not $attached) { $msg += " [AttachThreadInput başarısız]" }
    Say "${name}: $msg"
    return $seen
}

# Şarkı sonuna yaklaşmak için çalan şarkının süresi ve konum tahmini (ya da $null).
function Get-PlayEstimate {
    Update-Trace
    $slice = Get-LogSlice
    $loads = @([regex]::Matches($slice, '\((\d+) ms\) loaded') | ForEach-Object { [int]$_.Groups[1].Value })
    $autos = @($script:rows | Where-Object { $_.ev -eq 'auto_next' })
    $k = $autos.Count
    if ($loads.Count -le $k) { return $null }
    $dur = $loads[$k]
    if ($k -gt 0) {
        # Soak sırasında en az bir kendiliğinden geçiş oldu: son geçişte şarkı başı.
        return [pscustomobject]@{ Dur = $dur; Pos0 = 0.0; Anchor = [double]$autos[$autos.Count - 1].t; Net0 = 0.0; Track = $k; How = "son auto_next'ten beri" }
    }
    $plays = @($script:rows | Where-Object { $_.ev -eq 'play' })
    $dv = [regex]::Matches($slice, 'Devam: \S+ (\d+) ms, duyulan (\d+) ms')
    if ($plays.Count -eq 0 -or $dv.Count -eq 0) { return $null }
    $pos0 = [double]$dv[$dv.Count - 1].Groups[2].Value
    return [pscustomobject]@{ Dur = $dur; Pos0 = $pos0; Anchor = [double]$plays[$plays.Count - 1].t; Net0 = [double]$script:seekNetMs; Track = 0; How = "son 'Devam duyulan'dan beri" }
}

# ---------------------------------------------------------------- ortam
$envWanted = [ordered]@{
    RUST_LOG                 = 'openify=debug,librespot_playback::player=info,librespot_core::audio_key=info'
    OPENIFY_SINK_TRACE       = $csv
    OPENIFY_TEST_DISPLAY     = '1180,760'
    # Sınama kopyası varsayılan 15 dk'da kendini kapatır; oturum daha uzun sürer.
    OPENIFY_TEST_EXIT_AFTER  = "$([int]($SoakMinutes * 60) + 1200)"
    OPENIFY_TEST_PLAY_AFTER  = '3'
    OPENIFY_TEST_PLAY_ROW    = "$Row"
    OPENIFY_TEST_FAKE_PLAY   = $null    # gerçek çalma
    OPENIFY_SINK_MS          = $null    # varsayılan hedef (120)
    OPENIFY_SETTINGS_PATH    = $settings   # gerçek settings.json yerine geçici dosya
    OPENIFY_TEST_TRANSITION_STALL_MS = $(if ($TransitionStallMs -gt 0) { "$TransitionStallMs" } else { $null })
}
$envSaved = @{}
$copyDir = $null
$analyzerExit = $null
$started = $false

try {
    foreach ($k in $envWanted.Keys) {
        $envSaved[$k] = [Environment]::GetEnvironmentVariable($k, 'Process')
        [Environment]::SetEnvironmentVariable($k, $envWanted[$k], 'Process')
    }
    # Geçici ayar dosyası: ses düzeyi -Volume (UTF-8, BOM'suz).
    $volText = if ($Silent) { '0' } else { $Volume.ToString('0.##', [Globalization.CultureInfo]::InvariantCulture) }
    [IO.File]::WriteAllText($settings, ('{ "volume": ' + $volText + ' }'), (New-Object System.Text.UTF8Encoding($false)))
    # Sistemin uykuya dalmasını engelle (soak sırasında); ekran kapanabilir.
    [SesOturumu]::SetThreadExecutionState([uint32]2147483649) | Out-Null   # ES_CONTINUOUS | ES_SYSTEM_REQUIRED

    $others = @(Get-Process openify -ErrorAction SilentlyContinue)
    if ($others.Count -gt 0) {
        Say "UYARI: Çalışan başka Openify var (PID $($others.Id -join ',')); kapatmıyorum. Log ortak: o örnekte çalma / duraklatma yapılırsa log dilimi karışır."
    }
    Say ("ayar: ses düzeyi $volText (geçici ayar dosyası $settings; oturum sonunda silinir)")

    $logBefore = [long](Get-Item $logPath).Length
    $copyDir = Join-Path $env:TEMP ('openify-ses-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force $copyDir | Out-Null
    $runExe = Join-Path $copyDir 'openify.exe'
    Copy-Item $srcExe $runExe

    # ------------------------------------------------------------ açılış
    Phase-Start 'launch'
    $script:userFg = [SesOturumu]::GetForegroundWindow()
    $script:p = Start-Process $runExe -PassThru
    $started = $true
    for ($i = 0; $i -lt 100 -and $script:p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $script:p.Refresh() }
    if ($script:p.MainWindowHandle -eq 0) { throw "Openify penceresi 10 sn içinde açılmadı" }
    $script:hwnd = $script:p.MainWindowHandle
    Start-Sleep -Milliseconds 300
    $okFocus = [SesOturumu]::Restore($script:userFg)
    Say "odak kullanıcıya geri verildi: $okFocus"

    # 2. monitörde mi?
    $r = New-Object SesOturumu+RECT
    [SesOturumu]::GetWindowRect($script:hwnd, [ref]$r) | Out-Null
    $cx = ($r.L + $r.R) / 2; $cy = ($r.T + $r.B) / 2
    $screens = [Windows.Forms.Screen]::AllScreens
    $scr = $screens | Where-Object { $_.Bounds.Contains([int]$cx, [int]$cy) } | Select-Object -First 1
    if ($screens.Count -lt 2) { Say "NOT: tek ekran var; pencere ana ekranda açıldı (odak yine çalınmaz)." }
    elseif ($scr -and $scr.Primary) { Say "UYARI: pencere 1. (birincil) ekranda ($($r.L),$($r.T)); 2. monitör bekleniyordu." }
    else { Say "pencere ikincil ekranda: $($r.L),$($r.T) ($($scr.DeviceName))" }

    # "loaded" (şarkı gerçekten yüklendi) bekle.
    $deadline = (Get-Date).AddSeconds(120)
    $loaded = $false
    while ((Get-Date) -lt $deadline) {
        Assert-Alive
        Keep-Focus
        if ((Get-LogSlice) -match 'ms\) loaded') { $loaded = $true; break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $loaded) { throw "120 sn içinde logda 'loaded' görülmedi (satır $Row çalmadı: Spotify / ağ / satır sayısı?)" }
    Update-Trace
    $sl = Get-LogSlice
    $open = ($sl -split "`n" | Where-Object { $_ -match 'gecikmesi \(tahmin\)' } | Select-Object -Last 1)
    if ($open) { Say ("ses çıkışı: " + $open.Trim()) } else { Say "UYARI: logda 'Ses çıkışı açıldı' satırı yok (RUST_LOG / log dilimi?)" }
    Phase-End 'launch'

    Phase-Start 'warmup'
    Wait-Sec $Warmup
    Phase-End 'warmup'

    # ------------------------------------------------------------ 5x duraklat / devam
    Phase-Start 'pause'
    for ($i = 1; $i -le 5; $i++) {
        $a = Send-Step "Duraklat $i/5 (Boşluk)" 0x20 0x39 $false $false $false 'pause'
        if ($i -eq 1 -and -not $a) { throw "Boşluk tuşu uygulamaya ulaşmadı (izde 'pause' yok); oturum durduruldu." }
        Wait-Sec $StepGap
        Send-Step "Devam $i/5 (Boşluk)" 0x20 0x39 $false $false $false 'play' | Out-Null
        Wait-Sec $StepGap
    }
    Phase-End 'pause'

    # ------------------------------------------------------------ ses düzeyi adımları
    # Ctrl+Aşağı x2, Ctrl+Yukarı x2 (net sıfır), Ctrl+Shift+Aşağı x2 (sessize al, sesi aç).
    Phase-Start 'volume'
    $volSteps = @(
        @{ n = 'Ses -  (Ctrl+Aşağı) 1/2'; vk = 0x28; sc = 0x50; ctrl = $true; shift = $false },
        @{ n = 'Ses -  (Ctrl+Aşağı) 2/2'; vk = 0x28; sc = 0x50; ctrl = $true; shift = $false },
        @{ n = 'Ses +  (Ctrl+Yukarı) 1/2'; vk = 0x26; sc = 0x48; ctrl = $true; shift = $false },
        @{ n = 'Ses +  (Ctrl+Yukarı) 2/2'; vk = 0x26; sc = 0x48; ctrl = $true; shift = $false },
        @{ n = 'Sessize al (Ctrl+Shift+Aşağı)'; vk = 0x28; sc = 0x50; ctrl = $true; shift = $true },
        @{ n = 'Sesi aç (Ctrl+Shift+Aşağı)'; vk = 0x28; sc = 0x50; ctrl = $true; shift = $true }
    )
    $first = $true
    if ($Silent) { Say "sessiz koşu: ses düzeyi evresi atlandı"; $volSteps = @() }
    foreach ($s in $volSteps) {
        $ok = Send-Step $s.n $s.vk $s.sc $true $s.ctrl $s.shift 'volume'
        if ($first -and -not $ok) {
            Say "UYARI: Ctrl+Aşağı uygulamaya ulaşmadı (değiştirici durumu iletilemedi?); ses düzeyi evresi atlandı."
            break
        }
        $first = $false
        Wait-Sec $StepGap
    }
    Phase-End 'volume'

    # ------------------------------------------------------------ 5x geri sarma
    Phase-Start 'seek_back'
    $shiftOk = $true
    for ($i = 1; $i -le 5; $i++) {
        $ok = Send-Step "Geri sar $i/5 (Shift+Sol)" 0x25 0x4B $true $false $true 'seek'
        if ($ok) { $script:seekNetMs -= 5000 }
        elseif ($i -eq 1) {
            $shiftOk = $false
            Say "UYARI: Shift+Sol uygulamaya ulaşmadı; sarma evreleri atlandı."
            break
        }
        Wait-Sec $StepGap
    }
    Phase-End 'seek_back'

    # ------------------------------------------------------------ soak
    Phase-Start 'soak'
    if ($Load) {
        $cores = [Environment]::ProcessorCount
        Say "yük: $cores meşgul iş ($LoadPriority öncelik)"
        $script:jobs = @(1..$cores | ForEach-Object {
            Start-Job -ArgumentList $LoadPriority -ScriptBlock {
                param($prio)
                try { (Get-Process -Id $PID).PriorityClass = $prio } catch { }
                $x = 1.0
                while ($true) { $x = [math]::Sqrt($x + 12345.6789) * [math]::Sin($x) + 1.0 }
            }
        })
    }
    $soakEnd = (Get-Date).AddMinutes($SoakMinutes)
    $nextReport = (Get-Date).AddSeconds(60)
    $lastXr = 0
    Say "soak başladı: $SoakMinutes dk (bitiş $($soakEnd.ToString('HH:mm:ss')))"
    while ((Get-Date) -lt $soakEnd) {
        Wait-Sec 2
        if ((Get-Date) -ge $nextReport) {
            $nextReport = (Get-Date).AddSeconds(60)
            $lt = $script:lastTick
            if ($lt) {
                $left = [math]::Max(0, ($soakEnd - (Get-Date)).TotalMinutes)
                $flag = if ($lt.xr -gt $lastXr) { '  <-- XRUN ARTTI' } else { '' }
                $lastXr = $lt.xr
                Say ("soak: kalan {0:N1} dk, xrun={1}, underrun={2:N1} ms, kuyruk={3:N0} ms, hedef={4:N0} ms, auto_next={5}{6}" -f `
                    $left, $lt.xr, $lt.under, $lt.q, $lt.target, (Count-Ev 'auto_next' 0), $flag)
            }
        }
    }
    Phase-End 'soak'
    if ($script:jobs.Count -gt 0) {
        $script:jobs | Stop-Job -ErrorAction SilentlyContinue
        $script:jobs | Remove-Job -Force -ErrorAction SilentlyContinue
        $script:jobs = @()
        Say "yük işleri durduruldu"
    }

    # ------------------------------------------------------------ şarkı sonuna sar, kendiliğinden geçişi bekle
    Phase-Start 'seek_end'
    $est = if ($shiftOk) { Get-PlayEstimate } else { $null }
    $endStart = [double](NowMs)
    $waitAuto = $false
    if (-not $shiftOk) {
        Say "NOT: Shift+Sağ çalışmadığı için sona sarma atlandı; yalnız soak sırasındaki kendiliğinden geçişler çözümlenecek."
    } elseif (-not $est -or $est.Dur -lt 60000) {
        Say "NOT: şarkı süresi / konum logdan çıkarılamadı (ya da süre < 60 sn); sona sarma atlandı."
    } else {
        $target = $est.Dur - 22000
        Say ("sona sarma: şarkı #{0}, süre {1:N1} sn, konum tahmini {2:N1} sn ({3}); hedef {4:N1} sn" -f `
            $est.Track, ($est.Dur / 1000), (($est.Pos0 + ((NowMs) - $est.Anchor) + $est.Net0) / 1000), $est.How, ($target / 1000))
        $fwd = 0.0
        $presses = 0
        while ($presses -lt 150) {
            $pos = $est.Pos0 + ((NowMs) - $est.Anchor) + $est.Net0 + $fwd
            if ($pos -ge $target) { break }
            $ok = Send-Step ("İleri sar {0} (Shift+Sağ), tahmini konum {1:N1} sn" -f ($presses + 1), ($pos / 1000)) 0x27 0x4D $true $false $true 'seek'
            if ($ok) { $fwd += 5000 }
            $presses++
            Wait-Sec 0.6
        }
        $posNow = $est.Pos0 + ((NowMs) - $est.Anchor) + $est.Net0 + $fwd
        Say ("sarma bitti ({0} basış); tahmini konum {1:N1} / {2:N1} sn" -f $presses, ($posNow / 1000), ($est.Dur / 1000))
        $waitAuto = $true
    }
    Phase-End 'seek_end'

    Phase-Start 'post_auto_next'
    if ($waitAuto) {
        $remain = [math]::Max(5, ($est.Dur - ($est.Pos0 + ((NowMs) - $est.Anchor) + $est.Net0 + $fwd)) / 1000)
        $limit = (Get-Date).AddSeconds($remain + 60)
        $got = $null
        while ((Get-Date) -lt $limit) {
            Wait-Sec 1
            $got = Find-Ev 'auto_next' $endStart
            if ($got) { break }
        }
        if ($got) { Say "kendiliğinden geçiş görüldü (auto_next); 15 sn daha bekleniyor" } else { Say "UYARI: sona sarmadan sonra auto_next görülmedi (zaman aşımı)" }
    }
    Wait-Sec 15
    Phase-End 'post_auto_next'

    # Son iz satırları dosyaya yazılsın (iz 1 sn'de bir boşaltılır), sonra durdur.
    Wait-Sec 1.5
    Update-Trace
    if (-not $script:p.HasExited) { Stop-Process -Id $script:p.Id -ErrorAction SilentlyContinue }
    Say ("özet: Openify odağı öne alıp geri verilen kez: {0}; izde görülmeyen (ulaşmamış) eylem: {1}" -f $script:focusFixes, $script:missed)
} finally {
    # ---------------------------------------------------------------- her şeyi eski haline getir
    try {
        if ($script:jobs.Count -gt 0) {
            $script:jobs | Stop-Job -ErrorAction SilentlyContinue
            $script:jobs | Remove-Job -Force -ErrorAction SilentlyContinue
        }
    } catch { }
    try { if ($script:p -and -not $script:p.HasExited) { Stop-Process -Id $script:p.Id -Force -ErrorAction SilentlyContinue } } catch { }
    try { if ($script:p) { $script:p.WaitForExit(3000) | Out-Null } } catch { }
    Start-Sleep -Milliseconds 500
    try { Remove-Item $settings -ErrorAction SilentlyContinue } catch { Write-Host "UYARI: geçici ayar dosyası silinemedi: $_" }
    foreach ($k in $envSaved.Keys) { [Environment]::SetEnvironmentVariable($k, $envSaved[$k], 'Process') }
    try { [SesOturumu]::SetThreadExecutionState([uint32]2147483648) | Out-Null } catch { }   # ES_CONTINUOUS (uykuyu yeniden serbest bırak)
    if ($copyDir) {
        for ($i = 0; $i -lt 6 -and (Test-Path $copyDir); $i++) {
            Remove-Item -LiteralPath $copyDir -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path $copyDir) { Start-Sleep -Milliseconds 500 }
        }
    }
    # Faz dosyası ve log dilimi (iptal / hata olsa da yazılır).
    try {
        $lines = @('phase,start_unix_ms,end_unix_ms')
        $now = NowMs
        foreach ($k in $script:phases.Keys) {
            $v = $script:phases[$k]
            $end = if ($null -ne $v[1]) { $v[1] } else { $now }
            $lines += ('{0},{1},{2}' -f $k, $v[0], $end)
        }
        [IO.File]::WriteAllLines($marksOut, $lines)
    } catch { Write-Host "UYARI: faz dosyası yazılamadı: $_" }
    try {
        if ($started) {
            $c = Read-Chunk $logPath $logBefore
            $fs2 = [IO.File]::Create($logOut); $fs2.Write($c.Buf, 0, $c.N); $fs2.Close()
        }
    } catch { Write-Host "UYARI: log dilimi yazılamadı: $_" }

    if ($started -and (Test-Path $csv)) {
        Write-Host ""
        Write-Host "iz:    $csv"
        Write-Host "log:   $logOut"
        Write-Host "fazlar: $marksOut"
        $py = Get-Command python -ErrorAction SilentlyContinue
        if (-not $py) { $py = Get-Command py -ErrorAction SilentlyContinue }
        if ($py -and (Test-Path $analyzer)) {
            $minSoak = [math]::Round($SoakMinutes * 0.95, 2)
            $oldEnc = [Console]::OutputEncoding
            try {
                [Console]::OutputEncoding = [Text.Encoding]::UTF8
                # Dizi açıkça kurulur: `if` ifadesi tek öğeli diziyi dizeye açar,
                # @extra da dizeyi harf harf bölerdi (7 Kasım sessiz koşu).
                [string[]]$extra = @()
                if ($Silent) { $extra += '--skip-volume' }
                & $py.Source -I $analyzer $csv $logOut --marks $marksOut --min-soak-minutes $minSoak @extra
                $analyzerExit = $LASTEXITCODE
            } catch { Write-Host "UYARI: çözümleyici çalışmadı: $_" } finally { [Console]::OutputEncoding = $oldEnc }
        } else {
            Write-Host "python bulunamadı; elle: python -I scripts\ses_oturumu_ozet.py `"$csv`" `"$logOut`" --marks `"$marksOut`" --min-soak-minutes $([math]::Round($SoakMinutes * 0.95, 2))"
        }
    }
}
if ($null -ne $analyzerExit) { exit $analyzerExit }
