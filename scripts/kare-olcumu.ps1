param(
    [ValidateSet("wheel", "play", "idle")] [string]$Scenario = "wheel",
    [string]$Out = "frames",
    [int]$Notches = 24,
    [int]$NotchMs = 70,
    [int]$Seconds = 10,
    # Tekerleğin çevrileceği yer (pencereye göre x): 700 liste, 140 kenar çubuğu.
    # Kenar çubuğu senaryosu: -Scenario wheel -WheelX 140
    [int]$WheelX = 700,
    # Açılıştan sonra senaryodan önce beklenecek süre (sn).
    [double]$Warmup = 5,
    # Uygulamaya verilecek ek ortam değişkenleri: @{K="V"} ya da "K=V;K=V"
    # (ör. -Env "OPENIFY_PROF_HIDE=virtual").
    $Env = $null,
    # Ölçülecek exe (varsayılan: target\release\openify.exe). Her seferinde
    # geçici bir klasöre kopyalanır, böylece derleme çıktısı kilitlenmez.
    [string]$Exe = "",
    # Ekran listesi ve ara bilgileri yazma (kare-ab.ps1 kullanır).
    [switch]$Sessiz,
    # Pencere boyu "en,boy" (geniş pencerede "Şimdi çalıyor" paneli yerleşir: 1920,1080).
    [string]$Size = "1180,760"
)
# Kare ölçümü (bkz. src/ui/frames.rs): Openify'ı 2. monitörde, sessiz ve
# odak çalmadan açar, senaryoyu oynatır (wheel: tekerlekle kaydırma, play:
# Spotify'a dokunmadan sahte çalma, idle), kare kaydını ve işlemci süresini
# toplar. Kayıt: %TEMP%\openify-kare\<Out>.csv; özet: scripts/kare_ozeti.py.
# Önce: cargo build --release
# Ölçülen exe %TEMP%\openify-kare\bin\<rastgele>\ altına kopyalanır (varlıklar
# include_bytes ile gömülü, yanında başka dosya gerekmez); bitince silinir.
$ErrorActionPreference = "Stop"
$outDir = Join-Path $env:TEMP 'openify-kare'
New-Item -ItemType Directory -Force $outDir | Out-Null
$srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
if (-not (Test-Path $srcExe)) { throw "exe bulunamadı: $srcExe (önce cargo build --release)" }
$binDir = Join-Path $outDir ("bin\" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $binDir | Out-Null
$runExe = Join-Path $binDir 'openify.exe'
Copy-Item $srcExe $runExe

# -Env: hashtable ya da "K=V;K=V" -> ad/değer çiftleri.
$extraEnv = [ordered]@{}
if ($Env -is [System.Collections.IDictionary]) {
    foreach ($k in $Env.Keys) { $extraEnv[[string]$k] = [string]$Env[$k] }
} elseif ($Env) {
    foreach ($pair in ([string]$Env -split ';')) {
        if (-not $pair.Trim()) { continue }
        $i = $pair.IndexOf('=')
        if ($i -lt 1) { throw "-Env biçimi K=V;K=V olmalı: '$pair'" }
        $extraEnv[$pair.Substring(0, $i).Trim()] = $pair.Substring($i + 1)
    }
}
$savedEnv = @{}
$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")  # bu çalıştırmaya özel; gerçek settings.json'a dokunulmaz
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class F {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumDisplayDevices(string dev, uint i, ref DISPLAY_DEVICE dd, uint flags);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool EnumDisplaySettings(string dev, int mode, ref DEVMODE dm);
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
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct DISPLAY_DEVICE {
    public int cb; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string DeviceName;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string DeviceString; public int StateFlags;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string DeviceID;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string DeviceKey; }
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra; public int dmFields;
    public int dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName;
    public short dmLogPixels; public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight; }
}
"@
[F]::SetProcessDPIAware() | Out-Null

# Her ekranın konumu ve yenileme hızı.
for ($i = 0; $i -lt 8; $i++) {
    $dd = New-Object F+DISPLAY_DEVICE; $dd.cb = [Runtime.InteropServices.Marshal]::SizeOf($dd)
    if (-not [F]::EnumDisplayDevices($null, $i, [ref]$dd, 0)) { break }
    if (($dd.StateFlags -band 1) -eq 0) { continue }
    $dm = New-Object F+DEVMODE; $dm.dmSize = [Runtime.InteropServices.Marshal]::SizeOf($dm)
    [F]::EnumDisplaySettings($dd.DeviceName, -1, [ref]$dm) | Out-Null
    if (-not $Sessiz) { "ekran $($dd.DeviceName): $($dm.dmPelsWidth)x$($dm.dmPelsHeight) @ $($dm.dmDisplayFrequency) Hz, konum $($dm.dmPositionX),$($dm.dmPositionY), birincil=$(($dd.StateFlags -band 4) -ne 0)" }
}

$csv = Join-Path $outDir "$Out.csv"
Remove-Item $csv -ErrorAction SilentlyContinue
try {
    [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    $fg = [F]::GetForegroundWindow()
    $env:OPENIFY_TEST_DISPLAY = $Size
    $env:OPENIFY_FRAME_STATS = $csv
    if ($Scenario -eq "play") { $env:OPENIFY_TEST_FAKE_PLAY = "1" } else { Remove-Item Env:OPENIFY_TEST_FAKE_PLAY -ErrorAction SilentlyContinue }
    foreach ($k in $extraEnv.Keys) {
        $savedEnv[$k] = [Environment]::GetEnvironmentVariable($k)
        [Environment]::SetEnvironmentVariable($k, $extraEnv[$k])
    }
    if ($extraEnv.Count -gt 0 -and -not $Sessiz) { "ek ortam: " + (($extraEnv.Keys | ForEach-Object { "$_=$($extraEnv[$_])" }) -join ' ') }
    $p = Start-Process $runExe -PassThru
    for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
    Start-Sleep -Milliseconds 300
    [F]::Restore($fg) | Out-Null
    Start-Sleep -Milliseconds ([int]($Warmup * 1000))   # açılış, kapaklar, bağlantı otursun
    $h = $p.MainWindowHandle
    $r = New-Object F+RECT; [F]::GetWindowRect($h, [ref]$r) | Out-Null
    $p.Refresh(); $cpu0 = $p.TotalProcessorTime; $t0 = Get-Date
    switch ($Scenario) {
        "wheel" {
            $sx = $r.L + 8 + $WheelX; $sy = $r.T + 1 + 400
            $lp = [IntPtr]((($sy -band 0xFFFF) -shl 16) -bor ($sx -band 0xFFFF))
            foreach ($dir in @(-1, 1)) {
                $wp = [IntPtr][long]((((120 * $dir) -band 0xFFFF)) -shl 16)
                for ($n = 0; $n -lt $Notches; $n++) {
                    [F]::PostMessage($h, 0x020A, $wp, $lp) | Out-Null
                    Start-Sleep -Milliseconds $NotchMs
                }
                Start-Sleep -Milliseconds 400
            }
        }
        default { Start-Sleep -Seconds $Seconds }
    }
    $p.Refresh(); $cpu1 = $p.TotalProcessorTime; $t1 = Get-Date
    Start-Sleep -Milliseconds 1200   # son satırlar dosyaya yazılsın
    $wall = ($t1 - $t0).TotalSeconds
    $cpuPct = 100 * ($cpu1 - $cpu0).TotalSeconds / $wall
    "senaryo ${Scenario}: $([math]::Round($wall,2)) sn, işlemci tek çekirdeğin %$([math]::Round($cpuPct,1))'i"
    if (-not $Sessiz) { "pencere $($r.L),$($r.T); odak kullanıcıda: $([F]::GetForegroundWindow() -eq $fg)" }
    if (!$p.HasExited) { Stop-Process -Id $p.Id }
} finally {
    Remove-Item Env:OPENIFY_FRAME_STATS, Env:OPENIFY_TEST_FAKE_PLAY -ErrorAction SilentlyContinue
    foreach ($k in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($k, $savedEnv[$k]) }
    Start-Sleep -Milliseconds 500
    Remove-Item Env:OPENIFY_SETTINGS_PATH -ErrorAction SilentlyContinue
    Remove-Item $settings -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $binDir -Recurse -Force -ErrorAction SilentlyContinue
}
"kayıt: $csv"
