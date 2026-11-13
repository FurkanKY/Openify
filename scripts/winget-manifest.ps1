[CmdletBinding()]
param(
    [string]$Version = "0.1.1",
    [string]$Owner = "FurkanKY",
    [string]$Repository = "Openify"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$manifestDir = Join-Path $root "packaging/winget/$Owner.$Repository/$Version"
if (-not (Test-Path $manifestDir)) { throw "Manifest directory not found: $manifestDir" }
$tag = "v$Version"
$temp = Join-Path ([IO.Path]::GetTempPath()) ("openify-winget-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    gh release download $tag --repo "$Owner/$Repository" --pattern SHA256SUMS.txt --dir $temp
    if ($LASTEXITCODE -ne 0) { throw "Could not download SHA256SUMS.txt for $tag" }
    $sumPath = Join-Path $temp "SHA256SUMS.txt"
    $hashes = @{}
    foreach ($line in Get-Content $sumPath) {
        if ($line -match '^([0-9a-fA-F]{64})\s+\*?(.+)$') { $hashes[$Matches[2].Trim()] = $Matches[1].ToUpperInvariant() }
    }
    $assets = @{
        "Openify-win-Setup.exe" = "x64"
        "Openify-win-arm64-Setup.exe" = "arm64"
    }
    $installerPath = Join-Path $manifestDir "$Owner.$Repository.installer.yaml"
    $yaml = Get-Content -Raw $installerPath
    $yaml = [regex]::Replace($yaml, '(?m)^PackageVersion: .*$', "PackageVersion: $Version")
    # Önceki sürümün klasöründen kopyalanınca indirme adresleri de bu sürüme.
    $yaml = [regex]::Replace($yaml, '/releases/download/v[^/]+/', "/releases/download/$tag/")
    foreach ($asset in $assets.Keys) {
        # Releases before 0.1.2 have no ARM64 installer; only fill entries the manifest lists.
        if ($yaml -notmatch [regex]::Escape("/$asset")) { continue }
        if (-not $hashes.ContainsKey($asset)) { throw "Hash for $asset was not found in SHA256SUMS.txt" }
        $pattern = "(?s)(Architecture: $($assets[$asset]).*?InstallerUrl: [^\r\n]*/$([regex]::Escape($asset))\s+InstallerSha256:)\s*[^\r\n]+"
        $replacement = '$1 ' + $hashes[$asset]
        $yaml = [regex]::Replace($yaml, $pattern, $replacement)
        if ($yaml -notmatch [regex]::Escape($hashes[$asset])) { throw "Could not update hash for $asset" }
    }
    Get-ChildItem -Path $manifestDir -Filter "*.yaml" | Where-Object FullName -ne $installerPath | ForEach-Object {
        $content = Get-Content -Raw $_.FullName
        $content = [regex]::Replace($content, '(?m)^PackageVersion: .*$', "PackageVersion: $Version")
        $content = [regex]::Replace($content, '/releases/tag/v[^\s]+', "/releases/tag/$tag")
        Set-Content -Path $_.FullName -Value $content -Encoding utf8
    }
    Set-Content -Path $installerPath -Value $yaml -Encoding utf8
    Write-Host "Updated winget hashes for $tag in $manifestDir"
}
finally {
    Remove-Item -LiteralPath $temp -Recurse -Force
}
