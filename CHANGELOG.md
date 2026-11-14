# Changelog

All notable changes to Openify are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.5] - 2026-11-14

### Added

- Stats page in the top navigation: plays, minutes, distinct songs and
  artists, listening by hour and by weekday (local time), and the top 10
  songs, artists and albums for the last 4 weeks, 6 months or all time.
- Import your Spotify listening history (the "Extended streaming history"
  export, JSON or ZIP; the older account-data files too). Re-importing never
  double-counts, plays Openify already logged are skipped, podcasts and
  private-session plays are left out. Nothing is sent to Spotify.

### Changed

- Spotify Connect: "Playing on <device>" and "Play here" now sit in a tab
  attached to the top edge of the now-playing pill instead of a separate box.
- "Liked, never played" no longer shows a Plays column (every row was 0).
- Mini player: the close button sits on the title row instead of its own row,
  and the title doubles as the drag handle.
- README and site: memory figure re-measured in real use (~100 MB in Task
  Manager after browsing and playing).
- CI: a weekly dependency advisory check runs separately and never blocks a
  release.

## [0.1.4] - 2026-11-14

### Added

- Smart lists: three self-updating lists in your library, built only from
  what you play in Openify on this computer. "On repeat · 30 days" shows your
  most played songs of the last month, "Liked, never played" the liked songs
  you have not played yet, and "Long time no play" old favourites you have not
  heard for two months. They can be played and shuffled like any list, are
  never written to Spotify, and can be hidden in Settings.

### Changed

- Now-playing pill redesign: one centred axis with the play button in the
  middle, times on both sides of the controls, progress as a thin line along
  the bottom edge, and a quieter set of tool buttons.

## [0.1.3] - 2026-11-13

### Added

- Mini player: a small always-on-top window with cover, title, controls,
  progress and like. Open it from the now-playing pill, the Ctrl+K palette or
  its keyboard shortcut; its position and open state are remembered.
- Play counts: artist pages show monthly listeners, top listening cities and
  a play count on every popular song; album pages show a play count per song.
  The numbers load quietly in the background and are cached for a day.
- Playlist editing: edit the description of your own playlists, turn
  collaborative on or off, and create, rename and delete folders and move
  playlists in and out of them. Every edit can be undone and waits safely
  while offline.

### Changed

- Song lists on album, artist and search pages have the same column headers
  as the library, and an album shows its artists either on every row or on
  none.

## [0.1.2] - 2026-11-13

### Added

- Light theme with warm off-white surfaces, readable cover accents and softer
  shadows. Settings offers Dark (the existing default), Light and System;
  System follows Windows app mode immediately, including the window title bar.
- Windows on ARM: a native ARM64 installer (`Openify-win-arm64-Setup.exe`) on
  its own update channel.
- winget manifests and a script that fills their hashes from a release.
- Optional Discord Rich Presence (song, artist, cover and progress). It stays
  hidden until a Discord application ID is configured at build time.
- `OPENIFY_TEST_THEME=light|dark` for isolated visual test runs.

### Changed

- Smoother motion: cover colours blend in a perceptual colour space instead of
  turning muddy halfway, the current lyric line fades in instead of jumping,
  and switches slide.
- Settings and Tools center their column in wide windows instead of leaving
  most of the page empty on the right.

### Fixed

- Searching while Spotify is unreachable no longer shows a loading skeleton
  forever; after 10 seconds it says Spotify could not be reached.

## [0.1.1] - 2026-11-13

### Changed

- Search redesigned. The search box in the title bar is wider and tinted by the
  cover that is playing. Before you type, search shows your recent searches,
  things to listen to again and genre shortcuts. Results show a top result, songs,
  artists, and albums and playlists, and you can move through them with the arrow
  keys. Commands only appear when your search clearly matches one.
- The progress bar in the floating player has its own row and is much wider, so
  seeking is easier. Lists now end above the floating player, and a soft fade
  sits behind it.

### Fixed

- Home, Explore and similar-artist requests now use the interface language
  (English or Turkish) instead of always asking Spotify for Turkish content. The
  Explore page cache is kept per language.
- Home now uses your computer's time zone instead of always Europe/Istanbul.

### Other

- Documentation corrected and brought up to date (measured memory figures,
  privacy notes about play reporting and Spotify Connect, known limitations).
- Release builds are made by GitHub Actions and published with SHA-256 checksums
  (`SHA256SUMS.txt`).

## [0.1.0] - 2026-11-12

First public release.

### Added

- Native Windows Spotify client for Premium accounts, written in Rust: gpui
  interface, librespot playback, local SQLite cache.
- Sign-in through the browser, session kept in the Windows Credential Manager.
- Home, Explore, Library, artist, album and search pages; library folders, pins
  and sorting.
- Command palette (`Ctrl+K`), keyboard shortcuts, media keys, tray icon, Windows
  media overlay and an optional global search hotkey.
- Gapless playback, optional crossfade, volume normalization, quality up to
  320 kbps, recovery after an output device change.
- 10-band equalizer with presets and a preamp, and headphone profile search
  backed by the OPRA database.
- Library tools: duplicate and version finder, merge and split playlists,
  permanent sorting, JSON backup and local listening statistics. Library edits
  can be undone and edits made offline are queued.
- Synced lyrics (side panel and full screen) with LRCLIB as a fallback and an
  optional translation line.
- Smarter shuffle and queue rules (one version per song, skip explicit, never
  play an artist, hide songs), radio from a song, artist, album or playlist, and
  a private session.
- Spotify Connect device (on by default, can be turned off in Settings).
- Track credits, similar artists, artist following.
- Friend activity panel (off by default, experimental).
- English and Turkish interface.
- Installer with automatic updates, portable zip, and rollback of a new version
  that fails to start twice.

### Known limitations

- Screen readers are not supported yet.
- Dark theme only, Windows only.
- Plays made in Openify are not reported to Spotify.
- Friend activity and artist following have had only limited testing against a
  live account.

[Unreleased]: https://github.com/FurkanKY/Openify/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/FurkanKY/Openify/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/FurkanKY/Openify/releases/tag/v0.1.0
