<#
.SYNOPSIS
  Shows how each vendored crate (vendor/*) differs from its crates.io release.

.DESCRIPTION
  Every deliberate change in vendor/ is marked with an `OPENIFY:` comment and
  documented in vendor/<crate>/OPENIFY.md. This script downloads the pristine
  release (the version recorded in vendor/<crate>/Cargo.toml), compares it with
  the vendored copy, and lists changed files together with the number of
  `OPENIFY:` markers in each. A changed file with no marker at all is
  undocumented drift and is counted as a problem.

  Without -Summary the full diff of every changed file is printed as well.
  The script only reports; use -Strict to exit 1 when undocumented drift exists.

.EXAMPLE
  ./scripts/vendor-diff.ps1                 # full diff per crate
  ./scripts/vendor-diff.ps1 -Summary        # changed files + marker counts
  ./scripts/vendor-diff.ps1 -Crate gpui     # one crate
#>
param(
    [string[]]$Crate = @('gpui', 'librespot-core', 'librespot-playback', 'librespot-connect'),
    [switch]$Summary,
    [switch]$Strict
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$work = Join-Path ([IO.Path]::GetTempPath()) ("openify-vendor-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
$problems = 0

try {
    foreach ($name in $Crate) {
        $dir = Join-Path $root "vendor/$name"
        $manifest = Get-Content (Join-Path $dir 'Cargo.toml') -Raw
        if ($manifest -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw "no version in $dir/Cargo.toml" }
        $version = $Matches[1]

        $archive = Join-Path $work "$name-$version.crate"
        Invoke-WebRequest -UseBasicParsing "https://static.crates.io/crates/$name/$name-$version.crate" -OutFile $archive
        $pristineRoot = Join-Path $work "pristine-$name"
        New-Item -ItemType Directory -Path $pristineRoot | Out-Null
        tar -xzf $archive -C $pristineRoot
        $pristine = Join-Path $pristineRoot "$name-$version"

        Write-Host "=== $name $version ===" -ForegroundColor Cyan
        # Files that exist only on our side by design.
        $ours = @('OPENIFY.md', 'Cargo.lock')

        $files = Get-ChildItem -Recurse -File $dir | ForEach-Object {
            $_.FullName.Substring($dir.Length + 1) -replace '\\', '/'
        }
        foreach ($rel in $files) {
            if ($ours -contains $rel) { continue }
            $a = Join-Path $pristine $rel
            $b = Join-Path $dir $rel
            if (-not (Test-Path $a)) { Write-Host "  added   $rel"; continue }
            if ((Get-FileHash $a).Hash -eq (Get-FileHash $b).Hash) { continue }
            $markers = @(Select-String -Path $b -Pattern 'OPENIFY:' -SimpleMatch).Count
            Write-Host ("  changed {0}  (OPENIFY markers: {1})" -f $rel, $markers)
            if ($markers -eq 0 -and $rel -notlike 'Cargo.toml*') { $problems++ }
            if (-not $Summary) { git diff --no-index --unified=2 -- $a $b }
        }
    }
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}

if ($problems -gt 0) {
    Write-Host "$problems changed file(s) without an OPENIFY: marker (undocumented drift)." -ForegroundColor Yellow
    if ($Strict) { exit 1 }
}
