# Openify

A fast, quiet, native Spotify client for Windows. It plays music through your own **Spotify Premium** account, uses a fraction of the memory of the official desktop app and is built for keyboard use.

![Library](docs/screenshots/library.png)

> **Status:** early (0.1). [Download the installer](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-Setup.exe) or [build it from source](#build-from-source).

## What it is

Openify is a desktop player written in Rust. The interface is drawn with [gpui](https://gpui.rs) (the UI framework of the Zed editor), playback is done by [librespot](https://github.com/librespot-org/librespot), and everything is stored locally in SQLite. There is no web view and no Electron. It needs a Spotify Premium account: free accounts cannot stream through third-party clients.

## Highlights

- **Light and quick.** About 95 MB of memory while playing and about 70 MB when hidden in the tray, and a cold start of about half a second, measured on the author's machine. Pages you have opened before come from a local cache and appear within a frame.
- **Keyboard first.** `Ctrl+K` opens a command palette that searches your library and Spotify and runs commands. `Space`, `Ctrl+←/→`, `Ctrl+↑/↓` and friends control playback, `?` shows every shortcut. Media keys, the tray icon, the Windows media overlay and an optional global search hotkey work too.
- **Gapless playback and crossfade.** Songs on the same album never get a gap. Optional crossfade between songs (2 to 12 seconds), volume normalization with three levels and quality up to 320 kbps. Playback recovers by itself when you unplug or switch an output device.
- **Equalizer and headphone profiles.** A 10-band equalizer with presets and a preamp, and a headphone profile search backed by the open [OPRA](https://github.com/opra-project/OPRA) database: pick your model and get a ready-made correction curve fitted to the ten bands.
- **Library tools.** Find duplicate songs and different versions of the same song (remaster, live, radio edit) in a playlist, merge or split playlists, sort a playlist permanently, back everything up to a JSON file, and see your own listening statistics. All edits can be undone, and edits made offline are queued and sent when you are back online.
- **Lyrics and translation.** Synced lyrics in the side panel or full screen (from Spotify, with [LRCLIB](https://lrclib.net) as a fallback), with an optional translation under every line when one is available.
- **Friend activity.** An optional panel (off by default) that shows what your friends are listening to. See the privacy section: it uses an unofficial endpoint.
- **Smarter shuffle and queue rules.** Play only one version of each song, skip explicit songs, never play a chosen artist, hide single songs, start a radio from a song, artist, album or playlist, and use a private session that records nothing locally.
- **Calm interface.** Pages for Home, Explore, Library, artists, albums and search, a floating player, covers that tint the player and the full-screen view, English and Turkish interface languages. The design rules are enforced by tests.

## Screenshots

The screenshots show Spotify's own public playlists and albums with a sample profile.

| | |
|---|---|
| ![Playlist](docs/screenshots/playlist.png) | ![Lyrics](docs/screenshots/lyrics.png) |
| A Spotify playlist with the floating player | Full-screen lyrics with translation |
| ![Equalizer](docs/screenshots/equalizer.png) | ![Library tools](docs/screenshots/tools.png) |
| Equalizer and headphone profile | Duplicate finder in the library tools |
| ![Album](docs/screenshots/album.png) | |
| An album page | |

## Install

1. Download [Openify-win-Setup.exe](https://github.com/FurkanKY/Openify/releases/latest/download/Openify-win-Setup.exe) from the [latest release](https://github.com/FurkanKY/Openify/releases/latest).
2. Run it. Openify installs for your user only (no administrator rights) and opens.
3. Sign in with your Spotify account in the browser window that opens.

The installer is not code-signed yet, so Windows SmartScreen may say "Windows protected your PC". Click **More info**, then **Run anyway**. A portable zip is on the same release page if you prefer not to install.

Openify updates itself in the background and applies the update the next time you open it. If a new version fails to start twice, it goes back to the previous one on its own.

To uninstall, use Windows Settings > Apps.

Requirements: Windows 10 or 11 (64-bit), a Spotify Premium account and a working audio output.

## Build from source

You need Windows 10 or 11, [Rust](https://rustup.rs) (stable, 1.85 or newer), the MSVC build tools (Visual Studio Build Tools with the "Desktop development with C++" workload) and Git.

```powershell
git clone https://github.com/FurkanKY/Openify.git
cd Openify
cargo build --release
.\target\release\openify.exe
```

The first build compiles gpui and its dependencies and takes a while. No `.env` file or API key is needed. Then run the tests:

```powershell
cargo test --release
```

Read [ARCHITECTURE.md](docs/ARCHITECTURE.md) before a non-trivial change and [DESIGN.md](docs/DESIGN.md) for the design system (some project notes are in Turkish; the identifiers are English). Test hooks and checks are in [TESTING.md](docs/TESTING.md).

## Privacy

- Your data stays on your computer. The library cache, play history and statistics live in `%LOCALAPPDATA%\Openify`, settings in `%APPDATA%\Openify`, and the Spotify session in the Windows Credential Manager. Nothing is sent to any server of ours: there is none, and there is no analytics or telemetry.
- Openify does **not report what you play to Spotify** (no play events, no scrobbles). Its own statistics are computed locally from your own play history.
- Besides Spotify itself, Openify talks to a few services only when the feature needs them: Spotify's cover image servers, [LRCLIB](https://lrclib.net) for lyrics Spotify does not have, Deezer for the "similar artists" lists (artist names are sent), the OPRA database for headphone profiles (downloaded the first time you search one), and the update server for update checks.
- Friend activity is off by default. When you turn it on, Openify asks Spotify's unofficial friend feed endpoint for the list while the panel is open.
- A private session (profile menu) stops Openify from recording recent plays and statistics locally for that session.

## Disclaimer

Openify is an independent project. It is **not affiliated with, endorsed by or sponsored by Spotify AB**. "Spotify" and the Spotify logo are trademarks of Spotify AB; they are used here only to say what Openify works with. Openify talks to Spotify through the same unofficial, reverse-engineered interfaces as librespot, and some of its features use endpoints that Spotify does not document and may change or block at any time. Using third-party clients can conflict with Spotify's terms of service; you use Openify at your own risk. Please do not use it to circumvent Spotify's licensing: it does not download or save music, and it only plays on Premium accounts.

## Contributing

Bug reports, ideas and pull requests are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) first: it explains how to report a problem (with the log file), which checks a pull request must pass and the conventions of the code base.

## License

Openify is licensed under the **GNU General Public License, version 3 or later**. See [LICENSE](LICENSE). Third-party components and their licenses (gpui, librespot, fonts, icons, the OPRA data and every Rust crate) are listed in [THIRD_PARTY_LICENSES.md](docs/THIRD_PARTY_LICENSES.md).

## Türkçe

Openify, Spotify Premium hesabınla çalışan, Windows için yerel, hızlı ve sade bir Spotify istemcisidir. Rust ile yazıldı; arayüz gpui, ses librespot ile gelir. Az bellek kullanır (çalarken yaklaşık 95 MB, tepsideyken yaklaşık 70 MB), klavyeyle her şey yapılır, şarkılar arası boşluk yoktur, isteğe bağlı yumuşak geçiş, ekolayzer ve OPRA kulaklık profilleri, kütüphane araçları (yinelenen bulucu, birleştir / böl, sıralama, yedek, istatistik), çeviri destekli sözler ve arkadaş etkinliği vardır. Arayüz İngilizce ve Türkçedir. Verilerin bilgisayarında kalır; Openify Spotify'a ne dinlediğini bildirmez. Kurulum dosyası [son sürüm sayfasında](https://github.com/FurkanKY/Openify/releases/latest); dosya henüz imzalı olmadığı için Windows "Bilgisayarınız korundu" derse **Ek bilgi**, sonra **Yine de çalıştır**'a bas. Spotify ile ilişkisi yoktur, Spotify bir Spotify AB ticari markasıdır. Katkı için [CONTRIBUTING.md](CONTRIBUTING.md), lisans GPL-3.0-or-later.
