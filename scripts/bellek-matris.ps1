param(
    # Her yapılandırma kaç kez ölçülsün (medyan alınır).
    [int]$Repeat = 3,
    # Yalnız şu etiketleri çalıştır (virgülle): -Only "temel,cevrimdisi"
    [string]$Only = "",
    # Zaman çizelgesi noktaları (sn, işlemin gerçek başlangıcından).
    [string]$At = "1;1.5;2;3;4;6;8;12",
    [string]$Csv = "",
    # Son noktadan sonra listeyi ve kenar çubuğunu kaydır (kapaklar yüklenir);
    # döküm kaydırmadan sonra alınır (bellek-olcumu.ps1'in "son" durumu).
    [switch]$Kaydir,
    # Ölçülecek exe (boşsa target\release\openify.exe).
    [string]$Exe = "",
    # Her koşunun tam dökümü bu klasöre yazılır (varsayılan %TEMP%\openify-bellek-matris).
    [string]$LogDir = ""
)
# Bellek eleme matrisi: bellek-dokum.ps1'i birkaç yapılandırmayla, sırayla
# (her turda hepsi, böylece sürüklenme dağılır) çalıştırır ve etiket başına
# medyanları yazar: zaman çizelgesi (özel MB) + son dökümden heap / yığıt /
# diğer (VirtualAlloc) / yazılabilir görüntü. Gerçek şarkı çalmaz.
#   powershell -File scripts\bellek-matris.ps1 -Repeat 3
$ErrorActionPreference = "Stop"
[System.Threading.Thread]::CurrentThread.CurrentCulture = [System.Globalization.CultureInfo]::InvariantCulture
$all = "sidebar,list,bar,titlebar,glow,images,equalizer"
$configs = [ordered]@{
    temel       = ""
    cevrimdisi  = "OPENIFY_TEST_OFFLINE=1"
    resimsiz    = "OPENIFY_PROF_HIDE=images"
    hepsiGizli  = "OPENIFY_PROF_HIDE=$all"
    cdHepsiGizli = "OPENIFY_TEST_OFFLINE=1;OPENIFY_PROF_HIDE=$all"
    vitrin      = "OPENIFY_GALLERY=1"
    kucukPencere = "OPENIFY_TEST_DISPLAY=590,380"
    cdKucukHepsiGizli = "OPENIFY_TEST_OFFLINE=1;OPENIFY_TEST_DISPLAY=590,380;OPENIFY_PROF_HIDE=$all"
    # gpui'nin kendi anahtarı: DirectComposition yerine doğrudan HWND takas zinciri.
    dcKapali    = "GPUI_DISABLE_DIRECT_COMPOSITION=1"
}
if ($Only) {
    $keep = $Only -split ','
    foreach ($k in $keep) { if (-not $configs.Contains($k)) { throw "bilinmeyen yapılandırma: $k (var olanlar: $($configs.Keys -join ', '))" } }
    foreach ($k in @($configs.Keys)) { if ($keep -notcontains $k) { $configs.Remove($k) } }
}

$csv = if ($Csv) { $Csv } else { Join-Path $env:TEMP "openify-bellek-matris.csv" }
Remove-Item $csv -ErrorAction SilentlyContinue
$logDir = if ($LogDir) { $LogDir } else { Join-Path $env:TEMP "openify-bellek-matris" }
New-Item -ItemType Directory -Force $logDir | Out-Null
$dokum = Join-Path $PSScriptRoot 'bellek-dokum.ps1'
for ($r = 1; $r -le $Repeat; $r++) {
    foreach ($name in $configs.Keys) {
        "[$r/$Repeat] $name ..." | Write-Host
        # deneme numarasını betik kendisi 1 yazar; tur numarasını etikete katmadan
        # ayrı CSV dosyasına ekleyip tur bilgisini sonradan ekliyoruz.
        $tmp = Join-Path $env:TEMP "openify-bellek-tek.csv"
        Remove-Item $tmp -ErrorAction SilentlyContinue
        $args2 = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $dokum, '-Label', $name, '-Detay', '-At', $At, '-Csv', $tmp)
        if ($Kaydir) { $args2 += '-Kaydir' }
        if ($Exe) { $args2 += @('-Exe', $Exe) }
        if ($configs[$name]) { $args2 += @('-Env', $configs[$name]) }
        # Çıktı (tam döküm, hatalar) her koşu için ayrı dosyaya: önce sessizce yutuluyordu.
        $log = Join-Path $logDir "$name-$r.txt"
        & powershell @args2 *> $log
        if ($LASTEXITCODE) { "  UYARI: $name tur $r çıkış kodu $LASTEXITCODE (bkz. $log)" | Write-Host }
        if (Test-Path $tmp) { Get-Content $tmp | ForEach-Object { ($_ -replace '^([^,]+),\d+,', "`$1,$r,") } | Add-Content $csv }
        else { "  UYARI: $name tur $r CSV üretmedi (bkz. $log)" | Write-Host }
        Start-Sleep -Seconds 2
    }
}

function Med($v) {
    $s = @($v | Sort-Object); if ($s.Count -eq 0) { return $null }
    $m = [int][math]::Floor($s.Count / 2)
    if ($s.Count % 2) { $s[$m] } else { ($s[$m - 1] + $s[$m]) / 2 }
}
$rows = Get-Content $csv | ForEach-Object {
    $f = $_ -split ','
    [pscustomobject]@{ etiket = $f[0]; tur = [int]$f[1]; t = $f[2]; ozel = $f[3]; calisma = $f[4] }
}
$ts = @(($At -split ';') | Where-Object { $_ })
if ($Kaydir) { $ts += 'kaydir' }
"`n=== ZAMAN ÇİZELGESİ: özel bayt medyanı (MB), n=$Repeat"
$tbl = foreach ($name in $configs.Keys) {
    $o = [ordered]@{ yapilandirma = $name }
    foreach ($t in $ts) {
        $v = $rows | Where-Object { $_.etiket -eq $name -and $_.t -eq $t } | ForEach-Object { [double]$_.ozel }
        $o["t$t"] = if ($v) { [math]::Round((Med $v), 1) } else { $null }
    }
    $v = $rows | Where-Object { $_.etiket -eq $name -and $_.t -eq ($ts[-1]) } | ForEach-Object { [double]$_.calisma }
    $o['calisma'] = if ($v) { [math]::Round((Med $v), 1) } else { $null }
    [pscustomobject]$o
}
$tbl | Format-Table -AutoSize | Out-String | Write-Host
"=== SON DÖKÜM: özel belleğin parçaları, medyan (MB işlenmiş; W = çalışma kümesinde)"
$tbl2 = foreach ($name in $configs.Keys) {
    $o = [ordered]@{ yapilandirma = $name }
    foreach ($k in 'sayac', 'heapKesim', 'heapBuyuk', 'gpuWC', 'gpuWCW', 'yigit', 'diger', 'yazgor') {
        $v = $rows | Where-Object { $_.etiket -eq $name -and $_.t -eq $k } | ForEach-Object { [double]$_.ozel }
        $o[$k] = if ($v) { [math]::Round((Med $v), 1) } else { $null }
    }
    [pscustomobject]$o
}
$tbl2 | Format-Table -AutoSize | Out-String | Write-Host
"(ham veri: $csv)"
