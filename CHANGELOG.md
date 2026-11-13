# Changelog

All notable changes to Openify are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Light theme with warm off-white surfaces, readable cover accents and softer
  shadows. Settings offers Dark (the existing default), Light and System;
  System follows Windows app mode immediately, including the window title bar.
- `OPENIFY_TEST_THEME=light|dark` for isolated visual test runs and contrast
  checks for text and cover-tinted surfaces in both palettes.

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
