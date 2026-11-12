<#
.SYNOPSIS
  Openify sürümünü paketler (Velopack): cargo build --release, sonra vpk pack.
  Hiçbir şeyi yayımlamaz; çıktı target\velopack klasörüne yazılır.

.DESCRIPTION
  Gereken: .NET SDK (dotnet) ve vpk aracı, sürümü Cargo.toml'daki `velopack`
  crate'iyle AYNI olmalı (şu an 1.2.161). Script önce proje içi .tools\vpk\vpk.exe'ye,
  sonra PATH'teki vpk'ya bakar. Hiçbiri yoksa kurmaz, nasıl kurulacağını söyleyip çıkar:
    dotnet tool install vpk --version 1.2.161 --tool-path .tools\vpk      (proje içi, önerilen)
    dotnet tool install -g vpk --version 1.2.161                          (genel)

  Sürüm Cargo.toml'daki `version`'dan gelir (-Version ile ezilebilir; semver).
  Çıktı (target\velopack): Openify-<sürüm>-full.nupkg, önceki sürüm varsa
  Openify-<sürüm>-delta.nupkg, Openify-win-Setup.exe, Openify-win-Portable.zip,
  releases.win.json, assets.win.json. Güncelleme farkı (delta) için önceki
  sürümün paketleri çıktı klasöründe durmalı (silme).

  Yayımlama (elle, bu script yapmaz): releases.win.json + *.nupkg + Setup.exe dosyalarını
  kod deposunun (FurkanKY/Openify) Releases sayfasında yeni bir sürüme (GitHub Release) yükle.
  Uygulama varsayılan olarak https://github.com/FurkanKY/Openify/releases/latest/download
  adresinden okur (OPENIFY_UPDATE_URL ile değişir). Bozuk sürümü kaynaktan geri almak
  için (bkz. ARCHITECTURE.md, güncelleme geri alma) önceki sürümün *-full.nupkg dosyasını da
  son Release'e ekle: "latest/download" yalnız son Release'in dosyalarını sunar.

.PARAMETER PackId
  Paket kimliği; kurulum klasörü %LOCALAPPDATA%\<PackId>. Gerçek sürüm "Openify".
  Yerel kurulum denemesi için "Openify.Test" kullan (gerçek kuruluma karışmaz).
.PARAMETER Aumid
  Başlat menüsü kısayoluna yazılan AppUserModelID. Uygulamanın kendi kimliğiyle
  (src/system/mod.rs `app_id`) aynı olmalı: "Openify" (paket kimliği Openify.Test ise "Openify.Test").
.PARAMETER Shortcuts
  Kısayol konumları: StartMenuRoot (varsayılan), Desktop.
.PARAMETER SkipBuild
  Derlemeyi atla, var olan release exe'yi paketle.
#>
param(
    [string]$Version,
    [string]$PackId = "Openify",
    [string]$Aumid,
    [string]$Shortcuts = "StartMenuRoot",
    [string]$Channel = "win",
    [string]$TargetDir = "target",
    [string]$Out = "target\velopack",
    [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $Version) {
    $m = Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $m) { throw "Cargo.toml'da version bulunamadı" }
    $Version = $m.Matches[0].Groups[1].Value
}
if (-not $Aumid) { $Aumid = $PackId }

# Önkoşullar: kurmadan yalnız denetler.
$velopackCrate = (Select-String -Path "Cargo.toml" -Pattern '^velopack\s*=\s*"=?([^"]+)"').Matches[0].Groups[1].Value
$vpk = Join-Path $root ".tools\vpk\vpk.exe"
if (-not (Test-Path $vpk)) {
    $cmd = Get-Command vpk -ErrorAction SilentlyContinue
    if ($cmd) { $vpk = $cmd.Source } else { $vpk = $null }
}
if (-not $vpk) {
    Write-Host "vpk bulunamadı." -ForegroundColor Yellow
    if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
        Write-Host "Önce .NET SDK gerekli: winget install Microsoft.DotNet.SDK.10"
    }
    Write-Host "Kur (proje içi, genel araç değil):"
    Write-Host "  dotnet tool install vpk --version $velopackCrate --tool-path .tools\vpk"
    exit 2
}
$vpkVersion = (& $vpk --version 2>$null | Select-Object -First 1)
Write-Host "vpk $vpkVersion (crate $velopackCrate), paket $PackId $Version"

if (-not $SkipBuild) {
    cargo build --release --target-dir $TargetDir
    if ($LASTEXITCODE -ne 0) { throw "cargo build başarısız" }
}
$exe = Join-Path $root "$TargetDir\release\openify.exe"
if (-not (Test-Path $exe)) { throw "$exe yok" }

# vpk klasörün tamamını paketler: yalnız exe'yi temiz bir hazırlık klasörüne koy.
$stage = Join-Path $root "$TargetDir\velopack-stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item $exe $stage

$outDir = Join-Path $root $Out
New-Item -ItemType Directory -Force $outDir | Out-Null

# Yazar adı (7 Kasım kararı: Türkçe "ı" ile). Harf koddan kurulur: dosya hangi
# kodlamayla okunursa okunsun (Windows PowerShell 5.1 BOM'suz dosyayı ANSI
# okur) vpk'ya doğru Unicode gider (yerel komutlara UTF-16 ile geçer).
$authors = "Furkan Kadir Y$([char]0x0131)lmaz"

& $vpk pack `
    --packId $PackId `
    --packVersion $Version `
    --packDir $stage `
    --mainExe openify.exe `
    --packTitle "Openify" `
    --packAuthors $authors `
    --icon (Join-Path $root "assets\logo\openify.ico") `
    --aumid $Aumid `
    --shortcuts $Shortcuts `
    --channel $Channel `
    --outputDir $outDir
if ($LASTEXITCODE -ne 0) { throw "vpk pack başarısız" }

Write-Host "`nHazır: $outDir" -ForegroundColor Green
Get-ChildItem $outDir | Sort-Object LastWriteTime -Descending | Select-Object -First 8 |
    ForEach-Object { "{0,-44} {1,10:N1} MB" -f $_.Name, ($_.Length / 1MB) }
