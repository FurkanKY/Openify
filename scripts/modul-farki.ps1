# İşleme yüklenen DLL'leri iki yapılandırmada karşılaştırır (örneğin
# çevrimiçi ve çevrimdışı): hangi modül yalnız birinde var, boyutları ne.
# Bellek farkının işlem dışı bir bileşenden (ses sürücüsü eklentisi, ağ /
# şifreleme katmanı) gelip gelmediğini görmek için. Gerçek şarkı çalmaz:
# OPENIFY_TEST_FAKE_PLAY, ses 0, 2. monitör, odak çalınmaz.
#   powershell -File scripts\modul-farki.ps1 -B "OPENIFY_TEST_OFFLINE=1"
param(
    [string]$A = "",
    [string]$B = "OPENIFY_TEST_OFFLINE=1",
    [double]$Wait = 8,
    [string]$Exe = ""
)
$ErrorActionPreference = "Stop"
if (-not $Exe) { $Exe = Join-Path $PSScriptRoot "..\target\release\openify.exe" }
# Gerçek settings.json'a dokunulmaz: bu çalıştırmaya özel geçici ayar dosyası kullanılır.
$settings = Join-Path $env:TEMP ("openify-ayar-" + [guid]::NewGuid().ToString('N').Substring(0,8) + ".json")

function Run-Modules([string]$envSpec) {
    $dir = Join-Path $env:TEMP ("openify-modul-" + [guid]::NewGuid().ToString("N").Substring(0, 8))
    New-Item -ItemType Directory -Force $dir | Out-Null
    Copy-Item $Exe $dir
    $saved = @{}
    $vars = @{ "OPENIFY_TEST_DISPLAY" = "1180,760"; "OPENIFY_TEST_FAKE_PLAY" = "1" }
    foreach ($kv in ($envSpec -split ';' | Where-Object { $_ })) { $k, $v = $kv -split '=', 2; $vars[$k] = $v }
    foreach ($k in $vars.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $vars[$k]) }
    try {
        $p = Start-Process (Join-Path $dir "openify.exe") -PassThru -WindowStyle Minimized
        Start-Sleep -Seconds $Wait
        $p.Refresh()
        $mods = $p.Modules | ForEach-Object { [pscustomobject]@{ Name = $_.ModuleName.ToLower(); MB = [math]::Round($_.ModuleMemorySize / 1MB, 2); Path = $_.FileName } }
        $priv = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        Stop-Process -Id $p.Id -Force
        Start-Sleep -Milliseconds 500
        return @{ Mods = $mods; Priv = $priv }
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
        Get-ChildItem $dir -Recurse | ForEach-Object { try { $_.Delete() } catch {} }
        try { [IO.Directory]::Delete($dir, $true) } catch {}
    }
}

try {
    [IO.File]::WriteAllText($settings, '{ "volume": 0.0 }', (New-Object System.Text.UTF8Encoding($false)))
    $env:OPENIFY_SETTINGS_PATH = $settings
    $ra = Run-Modules $A
    $rb = Run-Modules $B
} finally {
    $env:OPENIFY_SETTINGS_PATH = $null
    try { [IO.File]::Delete($settings) } catch {}
}
"A [$A]: $($ra.Mods.Count) modül, özel $($ra.Priv) MB"
"B [$B]: $($rb.Mods.Count) modül, özel $($rb.Priv) MB"
$bn = $rb.Mods.Name
$an = $ra.Mods.Name
"--- yalnız A'da"
$ra.Mods | Where-Object { $bn -notcontains $_.Name } | Sort-Object MB -Descending | Format-Table Name, MB, Path -AutoSize | Out-String -Width 220
"--- yalnız B'de"
$rb.Mods | Where-Object { $an -notcontains $_.Name } | Sort-Object MB -Descending | Format-Table Name, MB, Path -AutoSize | Out-String -Width 220
