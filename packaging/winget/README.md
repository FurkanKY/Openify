# winget manifests

The `FurkanKY.Openify/0.1.1` directory contains the three manifests for the
0.1.1 release, with its x64 hash already filled in. 0.1.1 has no ARM64 installer;
from 0.1.2 on, copy the directory to the new version and add an `arm64` entry for
`Openify-win-arm64-Setup.exe`. The manifests use Velopack's per-user `Setup.exe`
with the `--silent` switch.

After publishing a GitHub release, run from the repository root:

```powershell
./scripts/winget-manifest.ps1 -Version <version>
```

The script downloads `SHA256SUMS.txt` with `gh` and fills the hash of every
setup file the installer manifest lists. Review the resulting YAML,
then validate it with:

```powershell
winget validate --manifest packaging/winget/FurkanKY.Openify/0.1.1
```

To prepare a submission, install wingetcreate (`winget install Microsoft.WingetCreate`)
and submit the manifest directory with `wingetcreate submit packaging/winget/FurkanKY.Openify/0.1.1`.
Review the generated pull request before merging. This repository does not
submit manifests automatically.
