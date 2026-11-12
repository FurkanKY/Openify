# Openify çalma güvenilirliği ölçümü.
#
# Uygulamayı ikinci ekranda, sessiz ve odak almadan açar; açılıştan
# $PlayAfter sn sonra listeden bir şarkı çaldırır (her denemede farklı satır)
# ve kayıttan şarkının yüklenip yüklenmediğini, Spotify'ın anahtar isteğini
# geciktirip geciktirmediğini ya da reddedip reddetmediğini sayar.
#
# Spotify anahtar isteklerini sınırlıyor: denemeler arasında $Gap sn bırakın,
# aynı şarkıyı art arda istemeyin. Ses düzeyi deneme boyunca 0, sonra eski
# değerine döner.
#
#   powershell -File scripts\oynatma-olcumu.ps1 -Trials 20 -Gap 60
param(
    [int]$Trials = 20,
    [int]$Gap = 60,
    [int]$PlayAfter = 2,
    [int]$Wait = 12,
    [int]$RowStart = 40,
    [string]$Exe = "$PSScriptRoot\..\target\release\openify.exe"
)
$ErrorActionPreference = "Stop"
# Gerçek settings.json'a dokunulmaz: bu çalıştırmaya özel geçici ayar dosyası kullanılır.
$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class OO {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, IntPtr p);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint a, uint b, bool attach);
  public static void Restore(IntPtr target) {
    IntPtr cur = GetForegroundWindow(); if (cur == target) return;
    uint me = GetCurrentThreadId(); uint them = GetWindowThreadProcessId(cur, IntPtr.Zero);
    AttachThreadInput(me, them, true); SetForegroundWindow(target); AttachThreadInput(me, them, false);
  }
}
"@
$env:OPENIFY_TEST_DISPLAY = "1180,760"
$env:OPENIFY_TEST_PLAY_AFTER = "$PlayAfter"
$env:RUST_LOG = "openify=info,librespot=warn,librespot_core::audio_key=info,librespot_playback::player=info"
$log = "$env:LOCALAPPDATA\Openify\openify.log"
$played = 0; $clean = 0; $retried = 0; $gaveUp = 0
try {
    [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    for ($t = 1; $t -le $Trials; $t++) {
        $row = $RowStart + $t - 1
        $env:OPENIFY_TEST_PLAY_ROW = "$row"
        $fg = [OO]::GetForegroundWindow()
        $before = (Get-Item $log).Length
        $p = Start-Process $Exe -PassThru
        for ($i = 0; $i -lt 300 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 10; $p.Refresh() }
        [OO]::Restore($fg)
        Start-Sleep $Wait
        if (!$p.HasExited) { Stop-Process -Id $p.Id }
        Start-Sleep -Milliseconds 400
        $fs = [IO.File]::Open($log, 'Open', 'Read', 'ReadWrite'); $fs.Seek($before, 'Begin') | Out-Null
        $text = (New-Object IO.StreamReader($fs)).ReadToEnd(); $fs.Close()
        $ok = $text.Contains('ms) loaded')
        $issues = ([regex]::Matches($text, 'Audio key response timeout|error audio key')).Count
        $failed = $text.Contains('failed after')
        if ($ok) { $played++ }
        if ($ok -and $issues -eq 0) { $clean++ }
        if ($ok -and $issues -gt 0) { $retried++ }
        if ($failed) { $gaveUp++ }
        "{0:HH:mm:ss} deneme {1} (satır {2}): çaldı={3} anahtar sorunu={4} vazgeçildi={5}" -f (Get-Date), $t, $row, $ok, $issues, $failed
        if ($t -lt $Trials) { Start-Sleep $Gap }
    }
    ""
    "çaldı: $played / $Trials (sorunsuz $clean, yeniden denemeyle $retried, anahtardan vazgeçilen $gaveUp)"
} finally {
    Start-Sleep -Milliseconds 500
    $env:OPENIFY_SETTINGS_PATH = $null
    Remove-Item $settings -ErrorAction SilentlyContinue
    $env:RUST_LOG = $null; $env:OPENIFY_TEST_PLAY_AFTER = $null; $env:OPENIFY_TEST_PLAY_ROW = $null; $env:OPENIFY_TEST_DISPLAY = $null
}
