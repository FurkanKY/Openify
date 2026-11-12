param(
    # Tekerleğin her turdaki çentik sayısı ve çentikler arası bekleme (ms).
    [int]$Notches = 40,
    [int]$NotchMs = 50,
    # Kaç tur: her turda liste aşağı kaydırılır, sonra kenar çubuğu aşağı-yukarı.
    [int]$Rounds = 4,
    # Açılıştan sonra ilk ölçümden önce beklenecek süre (sn).
    [double]$Warmup = 8,
    # Uygulamaya verilecek ek ortam değişkenleri: @{K="V"} ya da "K=V;K=V"
    # (ör. -Env "OPENIFY_PROF_HIDE=images").
    $Env = $null,
    # Ölçülecek exe (varsayılan: target\release\openify.exe). Geçici bir
    # klasöre kopyalanır, böylece derleme çıktısı kilitlenmez.
    [string]$Exe = "",
    [string]$Label = "",
    # Kaç kez tekrarlanıp ortalaması alınacak.
    [int]$Repeat = 1
)
# Bellek ölçümü: Openify'ı 2. monitörde, sessiz ve odak çalmadan açar
# (kare-olcumu.ps1 ile aynı düzen: OPENIFY_TEST_DISPLAY, ses 0, sahte çalma),
# listeyi ve kenar çubuğunu kaydırarak kapakları yükletir ve işlemin özel
# belleğini (PrivateMemorySize64) ve çalışma kümesini (WorkingSet64) birkaç
# noktada okur. Gerçek şarkı ÇALMAZ (OPENIFY_TEST_FAKE_PLAY).
#   powershell -File scripts\bellek-olcumu.ps1 -Label temel
#   powershell -File scripts\bellek-olcumu.ps1 -Label resimsiz -Env "OPENIFY_PROF_HIDE=images"
# Önce: cargo build --release
$ErrorActionPreference = "Stop"
$srcExe = if ($Exe) { $Exe } else { Join-Path $PSScriptRoot '..\target\release\openify.exe' }
if (-not (Test-Path $srcExe)) { throw "exe bulunamadı: $srcExe (önce cargo build --release)" }

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
$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")  # bu çalıştırmaya özel; gerçek settings.json'a dokunulmaz
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class M {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
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
}
"@
[M]::SetProcessDPIAware() | Out-Null

function Wheel($h, $r, $x, $dir, $n, $ms) {
    $sx = $r.L + 8 + $x; $sy = $r.T + 1 + 400
    $lp = [IntPtr]((($sy -band 0xFFFF) -shl 16) -bor ($sx -band 0xFFFF))
    $wp = [IntPtr][long]((((120 * $dir) -band 0xFFFF)) -shl 16)
    for ($i = 0; $i -lt $n; $i++) {
        [M]::PostMessage($h, 0x020A, $wp, $lp) | Out-Null
        Start-Sleep -Milliseconds $ms
    }
}
function Sample($p, $name) {
    $p.Refresh()
    [pscustomobject]@{
        nokta   = $name
        ozelMB  = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        calismaMB = [math]::Round($p.WorkingSet64 / 1MB, 1)
    }
}

$all = @()
for ($rep = 0; $rep -lt $Repeat; $rep++) {
    $binDir = Join-Path $env:TEMP ("openify-bellek\" + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force $binDir | Out-Null
    $runExe = Join-Path $binDir 'openify.exe'
    Copy-Item $srcExe $runExe
    $savedEnv = @{}
    $rows = @()
    try {
        [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
        $env:OPENIFY_SETTINGS_PATH = $settings
        $fg = [M]::GetForegroundWindow()
        $env:OPENIFY_TEST_DISPLAY = "1180,760"
        $env:OPENIFY_TEST_FAKE_PLAY = "1"
        foreach ($k in $extraEnv.Keys) {
            $savedEnv[$k] = [Environment]::GetEnvironmentVariable($k)
            [Environment]::SetEnvironmentVariable($k, $extraEnv[$k])
        }
        $p = Start-Process $runExe -PassThru
        for ($i = 0; $i -lt 50 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 100; $p.Refresh() }
        Start-Sleep -Milliseconds 300
        [M]::Restore($fg) | Out-Null
        Start-Sleep -Milliseconds ([int]($Warmup * 1000))
        $h = $p.MainWindowHandle
        $r = New-Object M+RECT; [M]::GetWindowRect($h, [ref]$r) | Out-Null
        $rows += Sample $p "acilis"
        for ($round = 1; $round -le $Rounds; $round++) {
            Wheel $h $r 700 -1 $Notches $NotchMs      # liste aşağı
            Start-Sleep -Milliseconds 800
            Wheel $h $r 140 -1 ($Notches / 2) $NotchMs # kenar çubuğu aşağı
            Start-Sleep -Milliseconds 800
            Wheel $h $r 140 1 ($Notches / 2) $NotchMs  # kenar çubuğu yukarı
            Start-Sleep -Milliseconds 1500
            $rows += Sample $p "tur$round"
        }
        Start-Sleep -Seconds 4
        $rows += Sample $p "son"
        $rows | ForEach-Object { $_ | Add-Member etiket $Label -PassThru } | Out-Null
        if (!$p.HasExited) { Stop-Process -Id $p.Id }
    } finally {
        Remove-Item Env:OPENIFY_TEST_FAKE_PLAY -ErrorAction SilentlyContinue
        foreach ($k in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($k, $savedEnv[$k]) }
        Start-Sleep -Milliseconds 500
        Remove-Item Env:OPENIFY_SETTINGS_PATH -ErrorAction SilentlyContinue
        Remove-Item $settings -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $binDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    "--- $Label (deneme $($rep + 1)/$Repeat)"
    $rows | Format-Table nokta, ozelMB, calismaMB -AutoSize | Out-String | Write-Host
    $all += , $rows
}
if ($Repeat -gt 1) {
    $last = $all | ForEach-Object { $_[-1] }
    "ortalama son: ozel $([math]::Round(($last | Measure-Object ozelMB -Average).Average, 1)) MB, calisma $([math]::Round(($last | Measure-Object calismaMB -Average).Average, 1)) MB"
}
