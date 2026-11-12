param(
    # Değişkenler: "ad:K=V;K=V" biçiminde, virgülle ayrılmış (ortam değerinde virgül olmaz).
    # Ad'dan sonraki kısım boş olabilir ("base:"). Varsayılan: base ve virtual.
    [string[]]$Variants = @("base:", "virtual:OPENIFY_PROF_HIDE=virtual"),
    [int]$Rounds = 3,
    [ValidateSet("wheel", "play", "idle")] [string]$Scenario = "wheel",
    # 700 liste, 140 kenar çubuğu.
    [int]$WheelX = 700,
    [int]$Notches = 24,
    [int]$NotchMs = 70,
    [int]$Seconds = 10,
    [double]$Warmup = 5,
    # Özet için bütçe yenileme hızı.
    [double]$Hz = 180,
    # Senaryo başladıktan sonra atılacak ilk saniyeler (açılış kuyruğu).
    [double]$Skip = 1.0,
    # Kuyruğu ayrıca otomatik bul ve at (kare_ozeti.py --auto-tail).
    [switch]$AutoTail,
    # Kayıt adlarının öneki: %TEMP%\openify-kare\<Prefix>-r<tur>-<ad>.csv
    [string]$Prefix = "ab",
    [string]$Exe = "",
    # Pencere boyu "en,boy" (kare-olcumu.ps1 -Size).
    [string]$Size = "1180,760"
)
# A/B kare ölçümü: değişkenleri (ad + ortam değişkenleri) tur tur sırayla
# (iç içe) çalıştırır, her biri için scripts/kare-olcumu.ps1 ile kayıt alır,
# scripts/kare_ozeti.py ile özetler ve karşılaştırma tablosu basar.
#
# İşlemci hız durumu (kare ≈10 ms ↔ ≈20 ms) kullanıcı çalışırken değişir; bu
# yüzden her satırda kurma ms (build p50) "makine hızı" göstergesi olarak,
# norm = çizim ms / kurma ms ise hıza göre düzeltilmiş süre olarak verilir.
# Karşılaştırmada norm sütununa bakın. Tur başına değişken sırası döndürülür
# (A B, B A, A B ...) ki sıra etkisi dengelensin.
#
# Örnek (tek satır):
#   pwsh -NoProfile -File scripts\kare-ab.ps1 -Rounds 3 -Scenario wheel -Variants "base:","virtual:OPENIFY_PROF_HIDE=virtual"
# Üç değişken: -Variants "a:","b:X=1","c:X=2;Y=3"
# Önce/sonra (iki exe): -Variants "once:EXE=target-olcum\eski.exe","sonra:"
# Önce: cargo build --release (exe geçici klasöre kopyalanır, derleme kilitlenmez).
$ErrorActionPreference = "Stop"
$inv = [Globalization.CultureInfo]::InvariantCulture
$outDir = Join-Path $env:TEMP 'openify-kare'
New-Item -ItemType Directory -Force $outDir | Out-Null
$olcum = Join-Path $PSScriptRoot 'kare-olcumu.ps1'
$ozet = Join-Path $PSScriptRoot 'kare_ozeti.py'
$py = if (Get-Command python -ErrorAction SilentlyContinue) { 'python' } elseif (Get-Command py -ErrorAction SilentlyContinue) { 'py' } else { throw "python bulunamadı" }

# Değişken listesini ayrıştır.
$vars = @()
foreach ($v in (($Variants -join ',') -split ',')) {
    if (-not $v.Trim()) { continue }
    $i = $v.IndexOf(':')
    if ($i -lt 1) { throw "değişken biçimi ad:K=V;K=V olmalı: '$v'" }
    $vars += [pscustomobject]@{ Ad = $v.Substring(0, $i).Trim(); Env = $v.Substring($i + 1).Trim() }
}
if ($vars.Count -lt 2) { throw "en az iki değişken gerekli" }
if (($vars | Select-Object -ExpandProperty Ad -Unique).Count -ne $vars.Count) { throw "değişken adları benzersiz olmalı" }

function Median([double[]]$a) {
    if (-not $a -or $a.Count -eq 0) { return [double]::NaN }
    $s = $a | Sort-Object
    $n = $s.Count
    if ($n % 2) { return [double]$s[($n - 1) / 2] }
    return ([double]$s[$n / 2 - 1] + [double]$s[$n / 2]) / 2
}
function F2($x) { if ($null -eq $x -or [double]::IsNaN([double]$x)) { '-' } else { ([double]$x).ToString('F2', $inv) } }

$start = $Warmup + 0.5
$sonuc = @()
"A/B: $($vars.Count) değişken x $Rounds tur, senaryo $Scenario (WheelX $WheelX), ısınma $Warmup sn, başlangıç $start sn + atlanan $Skip sn"
for ($r = 1; $r -le $Rounds; $r++) {
    # Tur başına sırayı döndür.
    $sira = @(); for ($k = 0; $k -lt $vars.Count; $k++) { $sira += $vars[(($r - 1) + $k) % $vars.Count] }
    foreach ($v in $sira) {
        $name = "$Prefix-r$r-$($v.Ad)"
        "tur ${r}/${Rounds}: $($v.Ad) ..."
        $a = @{ Scenario = $Scenario; Out = $name; WheelX = $WheelX; Notches = $Notches; NotchMs = $NotchMs
                Seconds = $Seconds; Warmup = $Warmup; Sessiz = $true; Size = $Size }
        # "EXE=yol" ortam değişkeni değil: o değişkeni başka bir exe ile ölç
        # (önce/sonra karşılaştırması: "once:EXE=eski.exe","sonra:").
        $envs = @(($v.Env -split ';') | Where-Object { $_.Trim() -and $_ -notmatch '^\s*EXE=' })
        $vExe = ($v.Env -split ';') | Where-Object { $_ -match '^\s*EXE=' } | ForEach-Object { $_.Substring($_.IndexOf('=') + 1) } | Select-Object -First 1
        if ($envs.Count) { $a.Env = $envs -join ';' }
        if ($vExe) { $a.Exe = $vExe } elseif ($Exe) { $a.Exe = $Exe }
        $olcumCikti = & $olcum @a
        $cpuSatir = $olcumCikti | Where-Object { $_ -like 'senaryo*' } | Select-Object -Last 1
        # Sürecin işlemci payı (tek çekirdeğe göre %), kare-olcumu.ps1'in satırından.
        $islemci = if ("$cpuSatir" -match '%([0-9]+(?:[.,][0-9]+)?)') { [double]::Parse($Matches[1].Replace(',', '.'), $inv) } else { [double]::NaN }
        $csv = Join-Path $outDir "$name.csv"
        if (-not (Test-Path $csv)) { "  UYARI: kayıt yok ($csv), atlandı"; continue }
        $oa = @($ozet, $csv, '--start', $start.ToString($inv), '--hz', $Hz.ToString($inv), '--skip', $Skip.ToString($inv), '--json')
        if ($AutoTail) { $oa += '--auto-tail' }
        $j = (& $py -I @oa) | Select-Object -Last 1 | ConvertFrom-Json
        if (-not $j -or $j.kare -lt 5 -or $null -eq $j.cpu_p50) { "  UYARI: yetersiz kare ($($j.kare)), atlandı"; continue }
        $sonuc += [pscustomobject]@{
            Tur = $r; Ad = $v.Ad; Kare = $j.kare; Asan = $j.asan
            Cpu50 = $j.cpu_p50; Cpu95 = $j.cpu_p95; Cpu99 = $j.cpu_p99
            Build50 = $j.build_p50; Norm50 = $j.norm_p50; Norm95 = $j.norm_p95; Yazi50 = $j.text_p50
            Surec = $cpuSatir; Islemci = $islemci; KareSn = $j.kare_sn
        }
    }
}
if ($sonuc.Count -eq 0) { throw "hiç geçerli ölçüm yok" }

""
"--- tur tur (build50 = makine hızı göstergesi; norm = çizim ms / kurma ms) ---"
$sonuc | Sort-Object Tur, Ad | ForEach-Object {
    [pscustomobject][ordered]@{
        tur = $_.Tur; değişken = $_.Ad; kare = $_.Kare; 'kare/sn' = F2 $_.KareSn; 'süreç %' = F2 $_.Islemci
        'cpu p50' = F2 $_.Cpu50; 'cpu p95' = F2 $_.Cpu95; 'cpu p99' = F2 $_.Cpu99
        'build p50' = F2 $_.Build50; 'norm p50' = F2 $_.Norm50; 'norm p95' = F2 $_.Norm95
        'yazı p50' = F2 $_.Yazi50; aşan = "$($_.Asan)/$($_.Kare)"
    }
} | Format-Table -AutoSize | Out-String -Width 200 | Write-Host

"--- tur medyanı (değişken başına) ---"
$ozetler = foreach ($v in $vars) {
    $s = @($sonuc | Where-Object { $_.Ad -eq $v.Ad })
    if ($s.Count -eq 0) { continue }
    [pscustomobject]@{
        Ad = $v.Ad; N = $s.Count; Islemci = Median ($s.Islemci); KareSn = Median ($s.KareSn)
        Cpu50 = Median ($s.Cpu50); Cpu95 = Median ($s.Cpu95); Cpu99 = Median ($s.Cpu99)
        Build50 = Median ($s.Build50); Norm50 = Median ($s.Norm50); Norm95 = Median ($s.Norm95)
    }
}
$ref = $ozetler | Select-Object -First 1
$ozetler | ForEach-Object {
    $d50 = if ($_.Ad -eq $ref.Ad) { '-' } else { ('{0:+0.0;-0.0}%' -f (100 * ($_.Norm50 / $ref.Norm50 - 1))) }
    $d95 = if ($_.Ad -eq $ref.Ad) { '-' } else { ('{0:+0.0;-0.0}%' -f (100 * ($_.Norm95 / $ref.Norm95 - 1))) }
    [pscustomobject][ordered]@{
        değişken = $_.Ad; tur = $_.N; 'kare/sn' = F2 $_.KareSn; 'süreç %' = F2 $_.Islemci
        'cpu p50' = F2 $_.Cpu50; 'cpu p95' = F2 $_.Cpu95; 'cpu p99' = F2 $_.Cpu99
        'build p50' = F2 $_.Build50; 'norm p50' = F2 $_.Norm50; 'norm p95' = F2 $_.Norm95
        "norm p50 fark ($($ref.Ad)'e göre)" = $d50; 'norm p95 fark' = $d95
    }
} | Format-Table -AutoSize | Out-String -Width 200 | Write-Host

# Son satır: tek satırda her değişkenin tur medyanı.
"medyan: " + (($ozetler | ForEach-Object { "$($_.Ad): süreç %$(F2 $_.Islemci), $(F2 $_.KareSn) kare/sn, cpu p50 $(F2 $_.Cpu50) / p95 $(F2 $_.Cpu95) ms, norm p50 $(F2 $_.Norm50) / p95 $(F2 $_.Norm95), build $(F2 $_.Build50) ms" }) -join '  |  ')
$jsonYol = Join-Path $outDir "$Prefix.json"
$sonuc | Select-Object Tur, Ad, Kare, KareSn, Islemci, Asan, Cpu50, Cpu95, Cpu99, Build50, Norm50, Norm95, Yazi50 | ConvertTo-Json | Set-Content $jsonYol -Encoding UTF8
"ayrıntı: $jsonYol"
