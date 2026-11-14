# Openify

A fast, quiet, native Spotify client for Windows, written in Rust. It plays through your own **Spotify Premium** account, uses about 60 MB of memory while playing with the window open (Task Manager) and is built for the keyboard.

**[Download for Windows](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-Setup.exe)** · [ARM64 installer](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-arm64-Setup.exe) · [Website](https://furkanky.github.io/Openify/) · [Release notes](CHANGELOG.md) · [Türkçe](#türkçe)

![Openify in use: search, an album, playback](docs/screenshots/demo.gif)

> **Status:** early (0.1). Friend activity and artist following are experimental and may break.

## Why Openify

The interface is drawn on the GPU by [gpui](https://gpui.rs), the UI framework of the Zed editor. Playback comes from [librespot](https://github.com/librespot-org/librespot), and your library cache lives in SQLite on your PC. There is no web view and no Electron, so it opens in about half a second and stays light.

## Features

- **Search everything with `Ctrl+K`.** One palette for songs, artists, albums and playlists from your library and Spotify, plus a few commands when you type their name. `?` shows every shortcut; media keys, the tray icon and the Windows media overlay work too.
- **Gapless albums and crossfade.** Songs that follow each other on an album play without a gap. Crossfade from 2 to 12 seconds, three levels of volume normalization, up to 320 kbps.
- **Equalizer with headphone profiles.** Ten bands, presets, a preamp, and correction curves from the open [OPRA](https://github.com/opra-project/OPRA) database.
- **Library tools.** Find duplicates and other versions of one song, merge or split playlists, sort one for good, back up to JSON and see your own statistics. `Ctrl+Z` undoes edits; edits made offline are sent when you are back.
- **Synced lyrics.** In the side panel or full screen, with an optional translation under each line ([LRCLIB](https://lrclib.net) fills in when Spotify has none).
- **Spotify Connect.** Openify shows up as a device in the Spotify app on your phone.
- **Smarter shuffle and queue rules.** One version of each song, skip explicit songs, never play a chosen artist, start a radio from anything, and a private session that records nothing.
- **Dark, Light or System theme**, English and Turkish interface.

| | |
|---|---|
| ![Search](docs/screenshots/palette.png) | ![Lyrics](docs/screenshots/lyrics.png) |
| Search (`Ctrl+K`) | Full-screen lyrics with translation |
| ![Equalizer](docs/screenshots/equalizer.png) | ![Library tools](docs/screenshots/tools.png) |
| Equalizer and headphone profile | Duplicate finder |
| ![Album](docs/screenshots/album.png) | ![Now playing](docs/screenshots/now-playing.png) |
| An album page | Now playing |

## Install

1. Download [Openify-win-Setup.exe](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-Setup.exe) (on Windows on ARM: [Openify-win-arm64-Setup.exe](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-arm64-Setup.exe)) and run it. It installs for your user only, no administrator rights needed.
2. If SmartScreen says "Windows protected your PC", click **More info**, then **Run anyway**. The installer is not code-signed yet.
3. Sign in with your Spotify Premium account in the browser window that opens.

Openify updates itself in the background and applies the update the next time you open it; if a new version fails to start twice, it goes back to the previous one. Prefer not to install? A portable zip is on the [release page](https://github.com/FurkanKY/Openify/releases/latest). Uninstall from Windows Settings > Apps.

**Requirements:** Windows 10 or 11 (x64 or ARM64), Spotify Premium, a working audio output.

## Privacy

- **Your data stays on your PC.** Cache, history and statistics are in `%LOCALAPPDATA%\Openify`, settings in `%APPDATA%\Openify`, the Spotify session in the Windows Credential Manager. There is no Openify server, no analytics and no telemetry.
- **Plays are not reported to Spotify.** Openify sends no listening reports and does not scrobble, so songs you play here may not appear in Spotify's Recently played, your Wrapped or artists' play counts. Playback you transfer to Openify through Spotify Connect goes through Spotify's own protocol and may be counted.
- **Spotify Connect is on by default.** Openify registers as a persistent device named `Openify (<computer name>)`. Turn it off or rename it in Settings.
- **Few outside calls.** Besides Spotify: LRCLIB for lyrics, Deezer for similar artists (artist names are sent), OPRA for headphone profiles, and the update server.
- **Friend activity** is off by default; when on, it asks Spotify's unofficial friend feed while the panel is open.

## Known limitations

- **No screen reader support yet.** gpui 0.2.2 exposes no UI Automation tree on Windows, so Narrator and NVDA cannot read the interface. Keyboard use is well covered, but that is not a substitute.
- Windows only, and Premium only.
- Friend activity, radio, track credits and lyrics translation use endpoints Spotify does not document and may stop working without notice.

## Build from source

You need Windows 10 or 11, [Rust](https://rustup.rs) (the pinned version in `rust-toolchain.toml` installs on the first build), the MSVC build tools ("Desktop development with C++") and Git.

```powershell
git clone https://github.com/FurkanKY/Openify.git
cd Openify
cargo build --release
.\target\release\openify.exe
cargo test --release
```

The first build compiles gpui and takes a while. No `.env` file or API key is needed. Before a non-trivial change read [ARCHITECTURE.md](docs/ARCHITECTURE.md), [DESIGN.md](docs/DESIGN.md) and [TESTING.md](docs/TESTING.md), and see [CONTRIBUTING.md](CONTRIBUTING.md) for the checks a pull request must pass.

## Disclaimer

Openify is an independent project. It is **not affiliated with, endorsed by or sponsored by Spotify AB**. "Spotify" is a trademark of Spotify AB, used here only to say what Openify works with. Openify talks to Spotify through the same unofficial interfaces as librespot, which Spotify may change or block at any time, and using a third-party client may conflict with Spotify's terms of service. It does not download or save music and only plays on Premium accounts.

## License

GNU General Public License, version 3 or later. See [LICENSE](LICENSE). Third-party components and their licenses are listed in [THIRD_PARTY_LICENSES.md](docs/THIRD_PARTY_LICENSES.md).

## Türkçe

Openify, Spotify Premium hesabınla çalışan, Windows için hızlı ve sade bir Spotify istemcisi. Rust ile yazıldı: arayüzü gpui GPU'da çizer, sesi librespot çalar. İçinde tarayıcı ya da Electron yok; pencere açıkken şarkı çalarken yaklaşık 60 MB bellek kullanır (Görev Yöneticisi).

**[Windows için indir](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-Setup.exe)** · [ARM64 kurulumu](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-arm64-Setup.exe) · [Web sitesi](https://furkanky.github.io/Openify/tr/)

**Neler var**

- `Ctrl+K` ile kütüphanende ve Spotify'da tek kutudan arama
- Albümlerde boşluksuz çalma, 2–12 saniyelik geçiş, üç düzey ses dengeleme
- On bantlı ekolayzer ve OPRA kulaklık profilleri
- Kütüphane araçları: kopya bulma, liste birleştirme ve bölme, sıralama, yedek, istatistik; `Ctrl+Z` ile geri alma
- Çeviri destekli, zamanlı şarkı sözleri
- Spotify Connect: telefondaki Spotify'da cihaz olarak görünür
- Koyu, Açık ya da Sistem teması; Türkçe ve İngilizce arayüz

**Kurulum:** kurulum dosyasını indirip çalıştır. Dosya henüz imzalı olmadığı için Windows "Bilgisayarınız korundu" derse **Ek bilgi**, sonra **Yine de çalıştır**'a bas. Ardından açılan tarayıcı penceresinde Spotify Premium hesabınla giriş yap. Güncellemeler kendiliğinden gelir.

**Gizlilik:** verilerin bilgisayarında kalır; Openify'ın sunucusu, analizi ya da telemetrisi yok. Openify çaldıklarını Spotify'a bildirmez, bu yüzden bu şarkılar Spotify'ın Son çalınanlar listesinde ve Wrapped'ında görünmeyebilir. Spotify Connect varsayılan olarak açık; Ayarlar'dan kapatılabilir.

**Bilinen sınırlar:** ekran okuyucular henüz desteklenmiyor, yalnız Windows ve yalnız Premium. Arkadaş etkinliği ve sanatçı takibi deneysel.

Openify'ın Spotify AB ile bir bağı yok; Spotify, Spotify AB'nin ticari markası. Katkı için [CONTRIBUTING.md](CONTRIBUTING.md). Lisans: GPL-3.0-or-later.
