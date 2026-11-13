# Openify architecture

This document is the map for contributors. It describes how the process is
laid out, where the boundaries are and why some things look the way they do.
For the exact behaviour of a piece, read the module header comment in the
source: most modules start with a long `//!` comment that is kept up to date
(some of those comments are in Turkish; the identifiers are English).

Openify is a native Windows Spotify client: UI in [gpui](https://gpui.rs)
(Zed's UI framework, patched copy in `vendor/gpui`), playback by
[librespot](https://github.com/librespot-org/librespot) 0.8 (patched copies in
`vendor/librespot-core`, `vendor/librespot-playback` and `vendor/librespot-connect`),
tokio for async,
SQLite (`rusqlite`, bundled) for caches and queues. Windows only.

## 1. Big picture

```text
                       ┌──────────────────────────── process ────────────────────────────┐
                       │                                                                 │
   user ──▶ gpui UI thread (src/ui)                       openify-engine thread (src/engine)
            Shell + child Entities/views                  ┌───────────────────────────────┐
                 │   ▲                                    │ tokio "openify-net" (2 workers)│
   Command ──────┘   └────── Event / PlaybackEvent        │   session, command loop        │
   (unbounded mpsc)         (async_channel)               │ tokio "openify-bg" (3 workers) │
                 ▲                                        │   library refresh, SQLite,     │
                 └────────────── EngineHandle ───────────▶│   covers, search, pathfinder   │
                                                          └──────────────┬────────────────┘
                                                                         │ librespot Player
   Windows shell (src/system): SMTC, taskbar, tray,                      ▼
   hotkey, single-instance  ◀── channel ──▶ ui/integration.rs   player thread (decode)
                                                                         │ Sink::write
                                                                         ▼
                                                  lock-free ring (engine/output/) ──▶ audio-device thread
```

Rules that hold the design together:

1. The UI never touches librespot, the network or the engine's state. It sends
   `Command`s and renders `Event`s.
2. The engine never produces user-visible text. It sends typed codes; the UI
   maps them to strings through the i18n table.
3. The audio thread never takes a lock, allocates or waits.
4. User data (the edit queue) is persisted *before* it is sent to Spotify.

## 2. Threads and runtimes

| Thread / runtime | Where | Job |
|---|---|---|
| UI thread (gpui main) | `src/ui`, `src/main.rs` (`ui::run`) | All rendering and input. Child views observe `Shell`. |
| `openify-engine` | `engine/mod.rs` (`spawn_engine`) | Owns two tokio runtimes (below). Wrapped in `catch_unwind`: a panic becomes `Event::EngineStopped`, and `EngineHandle::restart` starts a new engine on the same channels. |
| tokio `openify-net`, 2 workers | `engine/mod.rs` | The librespot `Session` connection (including audio-key answers) and the engine command loop. |
| tokio `openify-bg`, 3 workers | `engine/mod.rs` | Library refresh, track metadata, SQLite writes, covers, search. |
| librespot player thread | `vendor/librespot-playback` | Fetches, decrypts and decodes; calls our `Sink::write` (`engine/tap.rs`). |
| Audio-device thread | `engine/output/` (`RingSource`, via rodio/cpal) | Drains the ring buffer in the sound card callback. |
| `Saver` thread | `engine/resume.rs` | Writes the "resume where I left off" state; coalesces requests, flushes on exit. |
| Small named threads | `openify-audio-devices` (default-device notifications), `openify-update*` (Velopack), login helpers (`auth.rs`), `VSyncProvider` (gpui) | See each module. |

Why two runtimes: librespot waits only a short time for the answer to an
audio-key request. When heavy background work (library refresh at startup)
shared workers with the session, the answer was not read in time and tracks
failed to start. Heavy work now cannot starve the connection.

CLI mode (`openify <track link>`) skips the UI entirely and plays in a
terminal (`src/cli.rs`) with its own runtime. `openify logout` clears the stored
login.

## 3. UI and engine boundary

The whole contract is in `src/engine/mod.rs`:

- `Command` (UI to engine): `Load`, `OpenCollection`, `PlayCollection`,
  `PlayContext`, `Search { seq, query }`, `RefreshLibrary`, `Library(...)`
  (page requests and library edits, handled in `engine/wire.rs`), login
  commands (`StartLogin`, `CancelLogin`), `Flush` and so on.
  `EngineHandle::send` is non-blocking.
- `Event` (engine to UI): connection and login state, collections, search
  results, covers, notices, `Library(...)` replies, `EngineStopped`.
  `PlaybackEvent` carries queue view and the state restored at startup.
- Answers to a request echo the request's key or sequence number (`seq`), so
  the UI drops stale results.
- `EngineHandle` keeps the sender halves of the event channels, so the UI's
  listeners survive an engine restart.
- Pure decision logic lives outside the engine's mutable state so it can be
  unit-tested: `queue.rs` (play queue model), `recovery.rs` (what to do when
  the session silently dies), `login.rs` (connection/login state machine),
  `session.rs` (`SessionHost` transitions: reconnect backoff, account checks),
  `playback/handover.rs` (gapless UI handover gate).
- **Typed codes, not text.** Notices carry a `NoticeKind` and codes; failures
  carry reason codes. Do not format a user-visible sentence in the engine.

### Playback core (`engine/playback/`)

Everything that decides what the player does lives here, behind a test seam:

```text
Engine ──with_pb(Cx)──▶ Playback ──▶ Driver::Local ──▶ Deck ──▶ PlayerPort ──▶ librespot Player
   ▲                       │                            └────▶ OutputCtl (duck / flush / marks)
   └──── Effects, in order ◀┘
```

- `port.rs`: `PlayerPort` (what the engine asks of the player: load, preload,
  play, pause, stop, seek, mark, rebind, key_failures) and `PlayerSignal` (what
  comes back, translated from `PlayerEvent`). `LibrespotPort` is the real end,
  `testkit::FakePort` records the calls. `new_player` is unchanged (the CLI
  uses it).
- `deck.rs`: `Deck { port, out, mixer, volume }`. A user command touches the
  output first (duck / flush), then sends the gate mark, then the player; an
  automatic track change never flushes (gapless). The mark comes back from the
  player as `SessionClientChanged { client_id: "openify-mark", client_name:
  <64-bit number> }` (`output::mark_of`), not as `VolumeChanged`: Spirc reports
  the phone's volume through `VolumeChanged`, and a volume equal to a mark
  would have opened the gate early (gate tests in `output/tests.rs`).
- `mod.rs`: `Playback` owns the playback state (deck, position estimate,
  restored track, preload, failure counters) and never touches `Engine`. Per
  call it borrows the queue and the track metadata (`Cx`) and writes its side
  effects to `Cx::fx` in order: `Emit`, `Notice`, `Persist`, `FetchCover`,
  `QueueChanged`, `Reconnect`, `Connect`. `Engine::with_pb` (`bridge.rs`) drains that list
  after every call, so the order of events equals calling directly.
- `signal.rs`, `load.rs`, `recover.rs`: player events, loading / advancing /
  preloading, recovery (key failure: reconnect and retry the same song once,
  a second failure skips, the fourth stops; stalled load; after reconnect).
- `handover.rs`: the gapless UI handover gate. While it holds, resume-state
  writes with an estimated position are skipped (the queue already moved on,
  the UI and the audible track did not).
- `crossfade.rs`: crossfade policy (`spec_for`: which transition overlaps and by
  how much; same album, repeat-one, user moves and "off" never do) and the
  hand-off to the output (`arm_transition`, `preview_transition`); AutoMix variant
  `spec_for_auto` and the in-memory analysis book (`beats`).
- `driver.rs`, `connect.rs`: `TransportCmd` and `Driver`. "Whoever started the
  playback drives it": `Driver::Local` (our `Queue`, rules, balanced shuffle),
  `Driver::Spirc` (the phone started / transferred it: the queue and progress are
  Spotify's, we only observe and run the output), `Driver::Remote` (another device
  plays and nothing plays here: observation and "play here" only). The switching
  rules and the Spirc arm are in `playback/connect.rs`; see "Spotify Connect"
  below. `Queue` stays in `Engine` and is lent through `Cx`.
- `persist.rs`: resume-state writing and the startup restore (`impl Engine`).
- Tests: `playback/tests.rs` (scenarios against `FakePort`, a deviceless
  output and the real `Queue`), engine-level ordering tests in `engine/tests.rs`.

### Session host (`engine/session.rs`)

Everything about the Spotify session's life that is not playback. The fields moved out of `Engine` into one
`SessionHost`: the connected session (`conn`), the link / login state
(`link`, `login`, `new_session`), reconnect bookkeeping (`reconnecting`,
`last_reconnect`, `reconnect_failures`, `reconnect_wanted`) and the late
"free account" check (`premium_checks_left`).

- State transitions that need no network are methods of `SessionHost` and are
  unit-tested: `begin_reconnect` (`Started` / `Joined` / `Backoff`, the backoff
  comes from `recovery.rs`), `reconnect_failed` / `_succeeded` / `_refused`,
  `premium_step`, `facts()` (the `LinkFacts` that `Engine::with_pb` hands to
  `Playback`).
- The `impl Engine` part of the file does the network side: `start_login` /
  `cancel_login` / `on_login_done`, `reconnect` / `retry_connect` /
  `on_reconnected`, `check_health`, `on_offline` / `on_refused` /
  `needs_premium`, and the connect / login entry points with their test hooks
  (`first_connect`, `login_flow`).
- **A session enters the engine through one function:**
  `install_session(session, Install::First | Install::Replace)`. `First`: no
  connected session existed (startup, login, audio device or Premium arrived
  later): the player and the output are built (`new_player`, `Deck`), pending
  commands run, the library refreshes; it returns the player's event channel,
  which the command loop owns. `Replace`: the session died or was rejected: the
  player is kept and rebound, the old session is shut down, `after_reconnect`
  retries the song. With Connect on, `connect_on_install` starts one Spirc per
  installed session (the dealer builder is single-use per session) and
  `install_replace` drops the old one first; with Connect off it does nothing.
- Device identity is persistent (`Settings::device_id`, generated once by
  `Settings::load`, plus `connect_enabled`, `device_name`). Only while Connect is
  on does `first_connect` open sessions with that id (`apply_connect_identity`);
  off, sessions use a random id exactly as before.

### Spotify Connect (`engine/connect.rs`, `playback/connect.rs`)

**On by default** since 13 Nov (`Settings::connect_enabled`,
Settings > Spotify Connect). Off means: no Spirc is built,
sessions use a random device id, the UI sends no Connect command and receives no
`PlaybackEvent::Connect`; unit tests pin this
(`disabled_never_launches_whatever_the_session_looks_like`,
`without_spirc_everything_stays_local_and_nothing_is_published`).

- `engine/connect.rs`: the UI contract (`Command::Connect(ConnectCommand::{SetEnabled,
  SetName, TransferHere})`, `PlaybackEvent::Connect(ConnectStatus { driver, device_name,
  remote })`), `ConnectPort` (what the engine asks of Spirc: no player commands, Spirc
  commands the player itself), `SpircLink` (real end, wraps `Spirc`), `OutMixer`
  (Spirc's mixer: shares the deck's software volume and sets the output gain at
  once), and `ConnectHost` (pure lifecycle decisions: launch / stop / restart the
  session, stale launches rejected by an epoch number). The `impl Engine` part starts
  Spirc on `openify-net` (`Spirc::with_session`), pumps its events back as
  `Internal::Connect`, and restarts it for every installed session.
- `playback/connect.rs`: `Link` (port, own Connect id, last cluster view, last
  published status) and the driver switching:

  | event | Local | Spirc | Remote |
  |---|---|---|---|
  | phone transferred (in-band `RemoteAttached`) | to Spirc | - | to Spirc |
  | phone took it back (`RemoteDetached`) | - | to Local (UI shows the local track paused) | - |
  | new playback started here | - | leave Spirc, to Local | to Local |
  | another device plays (cluster) | to Remote if idle | - | name updated |
  | nothing else plays | - | - | to Local |

  `RemoteAttached` / `RemoteDetached` are the player's own `SessionConnected` /
  `SessionDisconnected` events, which Spirc emits before it commands the player, so
  they order correctly against the player events (the `Spirc` state events are the
  backup). While Spirc drives: no player command is ever sent from the engine (tests
  assert it); pause silences the output at once (`Deck::silence`), a phone-side skip
  or the attach / detach flushes the queue after the fact (`Deck::foreign_flush`;
  a phone-side seek is not flushed), our own Pause / Seek / Next are prepared on
  the output first and then forwarded; preload, tail margin, recovery and resume
  saving belong to `Local` only.
- Tests: `connect/tests.rs` (lifecycle: disabled, once, replace, stale start),
  `playback/connect_tests.rs` (switching matrix, pause latency, transfer, local
  start leaves Spirc, remote, reconnect). The real Spirc is only exercised by
  `openify connect-spike` and by hand.

On the UI side `Shell` (`src/ui/mod.rs`) is the root view and still a large
object. Cached child views (`src/ui/views.rs`, `AnyView::cached`) are rebuilt
only when notified; `Shell` is observed by its children. See "Conventions".

## 4. Audio path

```text
librespot Player ──write──▶ LevelSink (engine/tap.rs)
                               ├──▶ LevelTap  (engine/levels.rs: bass/mid/treble for the equalizer bars)
                               └──▶ Writer ──▶ ring buffer (engine/output/)
                                                   │
                                  device callback ◀┘ RingSource: envelope, volume gain
```

`engine/output/` replaces librespot's stock rodio backend. The stock backend
keeps about 0.6 s of audio queued, so pause, seek, track change and volume all
felt late. Ours:

- The ring counts stereo frames. `written` and `read` are monotonically
  increasing totals; queue length is their difference. Cells are `AtomicU32`
  (f32 bits): no `unsafe`, no locks, no allocation on the audio thread.
- `Writer::write` pushes a packet, then parks until the queue drops under a
  target (default 120 ms, `OPENIFY_SINK_MS`). The audio thread wakes it.
- `RingSource` never returns `None`. With no data it plays silence; if that
  happens mid-stream it counts an *xrun* and grows the target.
- Control is by atomics from the engine thread (`OutputCtl`): `duck` (8 ms
  fade before pause), `flush` (drop queued audio on seek / track change),
  `set_gain` (volume applied *after* the queue, about 10 ms ramp, so the
  equalizer bars do not flatten when the volume is low).
- The player is given a constant 1.0 `NoOpVolume`; the real volume lives in the
  device callback.
- Track-end margin: at the end of a track the ring is allowed to hold about 4 s
  so a stalled decoder at the next track start cannot cause an underrun. This
  makes the player run ahead of what is heard, which is why
  `playback/handover.rs` exists: it holds back `TrackChanged`, `Playing` and the cover until the first
  frame of the new track is audible, then releases them in order. The UI
  contract is unchanged (same events, same order, only at audible time).
- Device changes (default device switched or lost) are handled in `output/`
  (`device_lost`, `reopen`, `reroute` in the trace).
- **Crossfade** (`settings.crossfade_ms`, 0 = off, at most 12 s;
  `Command::SetCrossfade`). Overlap-add on the *writer* side, so the ring stays
  single-writer and the time axis linear (position, lyric clock, level timing
  are untouched; the audio thread knows nothing about it). Only for natural
  track ends between different albums: same-album neighbours, repeat-one and
  every user-driven change stay gapless (policy in `playback/crossfade.rs`,
  `spec_for`). Flow: the engine announces the wanted `TransitionSpec` (overlap,
  curve) when the track-end margin opens (`PreloadDue`) and again when the
  track ends (`OutputCtl::arm_transition`); the margin then holds `4 s +
  overlap`. The writer notes `eot = written` when it sees `EndOfTrack` (the
  player does not write between that event and the next `load`), and on the
  first packet of the new track sets `Transition { start: eot - overlap,
  overlap }` (one `AtomicU64`, 44 + 20 bits), clamped to what is still
  unplayed (`read + GUARD_FRAMES`), to half of either track, and falling back
  to gapless when a stall used the margin up. The first `overlap` frames of the
  new track are mixed into the old track's tail (`ring[start + k] = ring *
  out(t) + new * in(t)`, equal power: out² + in² = 1); the rest is appended.
  Invariant, tested: the writer only modifies frames after `read + GUARD`.
  The UI's track change moves to `Transition::start` (the title changes when
  the new song is first audible), `track_base` too. Level bars follow the
  outgoing track until the overlap ends.
- **AutoMix** (`settings.automix`, `Command::SetAutoMix`; UI: Settings > Sound >
  Crossfade, last option). Smart crossfade on top of the same writer-side
  overlap-add: it only decides WHERE and HOW LONG, with the same rules (natural
  track ends only; same-album neighbours, repeat-one and user moves stay gapless).
  Pieces: (1) **Analysis** `output/beat.rs` (`BeatTap`, lives in `tap::LevelSink`,
  i.e. on the player thread, never on the audio thread; runs only while AutoMix is
  on): per 512-frame hop two one-pole filters, three band energies, peak; onset
  strength = positive log-energy flux; leading silence, trailing silence / fade end
  (hop peak < -60 dBFS), a 20 x 1 s dBFS envelope at both ends, and per end a tempo
  (autocorrelation with harmonic enhancement, octave "halving" rule, fractional lag
  refinement), confidence and beat phase. Heavy work happens twice per song (head
  window full, song end) and costs ~1-2 ms. (2) **Cache** `engine/automix.rs`:
  `library.db` table `track_analysis` (schema v9, recomputable by playing; the
  engine keeps the newest 2000 rows in memory while AutoMix is on,
  `Playback::beats`). (3) **Decision** `output/automix.rs` (`decide`, pure): overlap
  = max(fade-out length, intro length) clamped to 2-12 s; if both tempos are
  confident (>= 0.30) and within +-6% (or x2 / x0.5) the overlap length is snapped
  so the incoming first beat lands on an outgoing beat, preferring multiples of 4
  beats; else plain equal power. (4) **Silence skipping** in the writer
  (`Writer::prepare`): while an AutoMix request is armed for the next transition,
  trailing digital silence (> 23 ms) of the outgoing song is not written (gaps in
  the middle of a song are compressed to at most 1.5 s, never cut), and the leading
  silence (<= 6 s) of the incoming song is dropped (`track_base` is moved back by
  the dropped frames). Flow: `PreloadDue` arms `TransitionSpec { auto: true }`
  with a planned overlap from cached data (fallback 5 s) plus the incoming song's
  cached head edge (`OutputCtl::arm_automix`); at the first packet of the next song
  the writer decides from the LIVE analysis of the song that just ended
  (`Writer::set_prev_analysis`), clamped like a fixed crossfade. Without a live
  analysis (e.g. joined the song in its last seconds) the requested fixed overlap
  is used. The incoming song's tempo / intro are only known if it was played from
  its start before (cache); the first time only the outgoing end drives the choice.
- **Growable ring.** The ring is sized from the setting when the output is
  built (`ring_frames`: 2 MiB off, 4 MiB up to 5 s, 8 MiB for 12 s). Turning
  crossfade on while connected calls `OutputCtl::reserve`: a second, larger slot
  is allocated on the engine thread and the writer switches to it at the next
  packet (`big_from`); the audio thread picks the slot per block. Nothing is
  copied or flushed; the audio thread never allocates or locks.
- **Equalizer** (`settings.eq_*`, `Command::SetEq { enabled, gains_db: [f32; 10],
  preamp_db }`). Ten fixed peaking biquads (31.5 Hz ... 16 kHz, Q 1.41) run on
  the audio thread in `RingSource::fill` after the envelope and volume are
  computed on the raw frames (`output/eq.rs`); coefficients are computed on the
  engine thread and published through a seqlock (`EqCtl`), the audio thread glides
  to them in 8 blocks and skips the whole branch while the equalizer is off
  (bit-identical output, golden-tested). The automatic pre-gain is the *true*
  peak of the combined response (not just the largest band), so neighbouring
  boosts cannot clip. The engine keeps the last setting and re-applies it when
  the output is rebuilt (`Engine::apply_eq`). Headphone profiles come from OPRA
  (CC BY-SA 4.0, https://github.com/opra-project/OPRA), never bundled: the first
  profile search downloads `database_v1.jsonl` (12.6 MB, 1.2 MB gzipped) to
  `<cache>/eq/`, refreshed with an ETag (`engine/opra.rs`). A profile (peak, shelf and
  pass filters) is fitted to the ten bands by least squares on a log frequency
  grid (`engine/equalizer.rs`, Gauss-Newton with the engine's own band responses,
  gains clamped to +-12 dB); the fit error is reported with the profile.
- **Karaoke** (`settings.karaoke_*`, `Command::SetKaraoke { enabled, amount }`).
  Vocal reduction without a model: mid/side (`output/karaoke.rs`). After the
  equalizer in `RingSource::shape` (before envelope and volume), only while on.
  `M = (L+R)/2` is split with two Linkwitz-Riley crossovers (150 Hz, 8 kHz); the
  band between them is subtracted from an all-passed copy of `M` (`M' = AP(M) -
  amount * mid`, the all-pass being exactly low + mid + high, so the cut does not
  suffer the poor skirts of subtracting a phase-shifted band-pass); `S` goes
  through the same all-pass, so side content and the stereo image keep their
  level. Bass (< 150 Hz) and air (> 8 kHz) stay in the middle. Mono sources
  (side energy below -40 dB of mid over ~150 ms, or sample-identical L = R for 4
  blocks) are detected and bypassed silently, otherwise the whole song would
  vanish. Enable, disable, amount and mono changes glide over 32 blocks (46 ms)
  as a wet/dry ramp. Off (or amount 0) the branch never runs: output bit-identical
  (golden test). One atomic `KaraokeCtl` between engine and audio thread.
- Deterministic tests drive the ring without a sound card (`output/tests.rs`,
  `output/xfade_tests.rs`: crossfade, growth, no allocation on the audio thread);
  `OPENIFY_SINK_TRACE=<csv>` writes a timeline of commands and events for
  measuring real runs.

## 5. Persistence

All paths are per user. Nothing here is ever required for correctness except
the edit queue and the login.

| What | Where | Notes |
|---|---|---|
| Library cache | `%LOCALAPPDATA%\Openify\library.db` (SQLite) | Library items, collection contents, track info, lyrics, and the `playback` table for resume, the library model for folders / pinning / added-at dates (schema v4-v6), the local play history `play_history` (v7, user data) and the locally hidden songs `hidden_songs` (v8, user data) and the AutoMix per-song analysis `track_analysis` (v9, recomputable). Shown instantly at startup, refreshed from Spotify in the background. Versioned migrations, see below. |
| Page cache | `%LOCALAPPDATA%\Openify\pages.db` | Artist, album, Home, Explore pages; stale entries are shown first and refreshed behind. |
| Covers, audio cache | `%LOCALAPPDATA%\Openify\covers`, `audio` | Covers capped at 200 MB, encrypted audio at 2 GB (`engine/cache.rs`). |
| Logs, crash reports | `%LOCALAPPDATA%\Openify\openify.log`, `crash-<time>.txt` | `src/crash.rs`; last 5 reports kept. `OPENIFY_LOG_DIR` overrides. |
| Settings | `%APPDATA%\Openify\settings.json` | Plain JSON with a `version` field; migration steps run on the raw JSON at load (the `.bak` / `.corrupt` recovery is unchanged; the backup keeps the pre-migration text). `OPENIFY_SETTINGS_PATH` overrides (tests must use it). |
| Edit queue | `%APPDATA%\Openify\edits.db` | User data, deliberately not in the cache folder. Migrations only add, never delete. |
| Spotify session | Windows Credential Manager (`keyring`) | Never written to disk files. |

Migrations (`engine/migrate.rs`): every SQLite file keeps its version in
`PRAGMA user_version` and lists ordered steps; each step runs in its own
`BEGIN IMMEDIATE` transaction together with the version bump, so an interrupted
step leaves no trace. A file newer than the code (app rollback) is refused and
left untouched. `library.db`: v3 is the old schema (baseline, which for the
long-gone v1-v2 drops only re-fetchable tables), v4 adds `library_items.kind` /
`parent` (folder tree), `collection_tracks.added_at` (ms, NULL = unknown) and
`library_pins` (user pins, separate so library refreshes do not touch them);
v5 adds `library_artists` (followed artists, newest follow first; re-fetchable,
own table so the folder-tree writer never touches it); v6 adds
`library_items.added_at` (when the item joined the library, ms) and
`library_items.creator` (for the "Creator" sort); v7 adds `play_history` (local
listening history for the Tools page: one row per counted play, name / artists /
album copied in so it outlives cache refreshes; never leaves the machine); v8 adds
`hidden_songs` (songs hidden locally, per context or everywhere; never sent to Spotify).
`lyrics`, `playback`, `library_pins`, `play_history` and `hidden_songs` are never dropped. `pages.db` (all
re-fetchable) is v1; `edits.db` (user data) is v2. Each has a test that builds
every older version and migrates it to the latest.

Connections (`engine/conns.rs`): `library.db` and `pages.db` use WAL with one
writer and a pool of two read-only connections; reads never wait for a writer.
`OPENIFY_TEST_CACHE_DIR` redirects the whole cache folder (tests must use it).
The UI thread does no database reads: pages, related artists and Explore data
come from the engine as events (it answers from the cache first, then
refreshes), and the remaining cache-only lookups (suggestions, Now playing
artist card, library album) run on gpui's background executor. The engine now
starts before gpui, so the first list is ready when the window is built and
`Shell::new` no longer waits for it.

### Edit queue (`engine/edits.rs`, wired in `engine/wire.rs`)

Library writes (like / unlike, add to / remove from playlist, create / rename /
delete playlist, reorder) follow one flow:

1. The UI updates its own view immediately (optimistic).
2. `EditQueue::enqueue` writes the operation to `edits.db`.
3. The engine applies the same pure patch functions (`patch_tracks`,
   `patch_library`) to its own copy and the disk cache.
4. Only then does it send the request to Spotify. On success the row is
   removed; on a retryable failure it is retried with backoff; on a permanent
   failure it moves to `failed` with a reason code for the UI to show.

Follow / unfollow artist (`EditOp::Follow` / `Unfollow`) uses the same flow:
it is the same `/collection/v2/write` call with set `artist` instead of
`collection`; the read side is `/collection/v2/paging` with set `artist`
(`Library::artists`). The artist card (name, portrait) travels with the
operation like an album's card does. Not yet verified against a live account:
`cargo test --release live_follow -- --ignored --nocapture` (owner's approval).

So a write survives quitting the app before it reached Spotify. The undo stack
(Ctrl+Z) is built on the same operations. Tests and UI test runs use an
in-memory queue with a fake backend (`OPENIFY_TEST_DISPLAY` or
`OPENIFY_TEST_EDITS`), so they cannot write to a real account.

### Library layout (folders, pins, sort)

`Library::meta` (`engine/library.rs`, `LibraryMeta`) carries what the flat
`albums` / `playlists` lists cannot: folders (`Folder`, read from the root
list's `start-group` / `end-group` markers in the same `get_rootlist` call,
`at` = how many lists precede the folder), each list's parent folder, when it
joined the library, its creator, and the pins. Folders are **read-only**: the
edit queue has no root-list operation, so Openify never creates, renames or
moves a folder. Pins are local (`library_pins`, written by
`LibraryCommand::Pin`, the UI updates optimistically; nothing goes to
Spotify). Track "date added" values are read from the cache on demand
(`LibraryCommand::AddedDates`), no network. The pure tree logic (sibling order,
pin lifting, nesting) is `ui/library_tree.rs`; the Shell glue (folder
open/closed state in `settings.json`, entering folders in the grid, pin
and sort menus, keyboard cursor) is `ui/library_layout.rs`.

### Library tools (`engine/tools/`, `ui/tools.rs`, `ui/tools_view.rs`)

One Tools page (`Route::Tools`, opened from the Library toolbar or the palette)
with five tools. Everything that writes to Spotify goes through the existing
edit queue (`EditOp::RemoveFromPlaylist`, `CreatePlaylist`,
`MovePlaylistItems`; undo with Ctrl+Z); no new live write type exists.

- **Duplicates** (`tools/dupes.rs`): same URI, plus versions of one song using
  the shuffle rules' own key (`rules::song_key` + `durations_compatible`).
  One row per group stays (plainest title); the user unticks what to keep.
  Own playlists only (Spotify refuses writes to others').
- **Merge / split** (`tools/plan.rs`): merge several lists into a new one
  (optionally dropping same-URI copies or versions), split one list by artist,
  album or year added into new lists. Sources are never touched. A preview and
  confirm step sits in the page.
- **Persistent sort** (`tools/plan.rs`): target order by name / artist / album /
  date added / length, then the minimum number of single-item
  `MovePlaylistItems` (n minus the longest increasing subsequence; tested
  against random permutations). At most `MAX_SORT_MOVES` (500) per run.
- **Backup** (`tools/backup.rs`): JSON export of the cached library (playlists
  with track URIs, albums, folders, pins, followed artists, Liked Songs) to a
  file chosen in the Windows save dialog (`system/dialog.rs`). No restore yet.
- **Local stats** (`tools/stats.rs`): `PlayCounter`, a pure state machine fed
  with playback events by `Shell::tools_note_playback` (a play counts after 30 s
  listened or half the track, whichever is first; pausing or seeking does not
  advance it). Rows go to `play_history` through `LibraryCommand::RecordPlay`;
  `PlayStats(range)` aggregates on a background thread. Nothing is written in a
  private session (`stats::set_private_session`) and nothing is sent anywhere.

Bulk rule: `BULK_CONFIRM` (50) or more changes (tracks written or removed,
lists created, moves) show a warning and ask again before anything is queued.

### Listening core: radio, hidden songs, private session (`engine/radio.rs`, `hidden.rs`, `listening.rs`, `ui/listening.rs`)

- **Radio** (`Command::StartRadio { seed, title }`, from the right-click menu of a
  song, artist, album or playlist and the palette): the station data comes from
  `GET spclient.wg.spotify.com/radio-apollo/v3/stations/{seed}?autoplay=true&count=50[&prev_tracks=…]`
  (the old desktop client's radio endpoint, also carried by librespot as
  `get_apollo_station`; the web player's `inspiredby-mix` builds a fixed playlist
  and has no "I played these, give me more", which an endless radio needs). One
  constant URL (`radio::STATION_BASE`), `Pathfinder::get_json` on its own gate
  (`pathfinder::radio`), tolerant parsing, typed failure codes, read-only. The
  returned URIs are completed with the shared batched track-info request and
  started as a context-less queue (`Queue::radio` carries seed and title, saved
  with the resume state). While playing, `persist_tick` asks for more when 10 or
  fewer context songs remain, at most one request in flight, at least 30 s apart
  (doubling after an empty / failed answer, giving up after 3), at most 50 songs
  per request, at most 1000 songs in the context (`radio::Feed`, pure and
  tested). New songs are appended through `Queue::extend_context`, so the shuffle
  rules (never play, skip explicit, versions) and hidden songs apply to radio
  like to any context. Checked once against a live account on 12 Nov (`live_radio`, owner's
  approval).
- **Hidden songs** (`hidden.rs`): "Hide this song" (per playlist / album) and
  "Never play anywhere" (global), kept in `library.db` `hidden_songs` (v8, user
  data) and mirrored to the UI as `LibraryEvent::Hidden`. `Queue::pool` drops them
  by URI before the name-based rules, so they are skipped in shuffle, Next,
  Previous and repeat laps; a song the user picks by hand still plays (the
  `restart_with` index and the user queue are never filtered). Purely local: no
  new Spotify write exists.
- **Private session** (`ui/listening.rs`, toggled from the profile menu, the
  palette or the settings row; a badge shows in the title bar): while on,
  Recently played (`Shell::note_played`) and the local play history
  (`stats::set_private_session`, checked by both the UI and the engine) are not
  written; where you left off is still saved. It ends when the app quits (not
  persisted). Playback reporting: Openify's own (local) playback sends no
  listening reports to Spotify. The vendored librespot code contains no
  event-service / playback-report call (`spclient.rs` has none; only the
  Connect-state calls `put_connect_state_request`, `delete_connect_state_request`
  and `put_connect_state_inactive`). While Connect is on (the default) the app
  does register a Connect device, but while the local player drives the device
  stays inactive and publishes no playing state (`Spirc` ignores player events
  then, patch P2). So songs played locally may not appear in Spotify's Recently
  played, in Wrapped, in artists' play counts or in friends' activity; this was
  not checked against a live account. Playback that a phone started or
  transferred (`Driver::Spirc`) goes through Spotify's own Connect protocol and
  may be counted by Spotify. The private session switch only controls Openify's
  own history and statistics; it does not change the Connect device state. The UI
  therefore says "Openify doesn't write to its own history" and no more.

### Resume (`engine/resume.rs`)

Queue (context, user queue, history, shuffle, repeat) and playback position are
saved in two rows: a small `state` row (written about every 5 s and on
pause / track change) and a large `context` row (written only when the context
changes). The `Saver` thread coalesces writes.

## 6. Talking to Spotify

- Playback, session and metadata go through librespot (`Session`, Mercury /
  spclient calls, login5 token).
- Library data and edits use the same internal endpoints the official desktop
  client uses (`/collection/v2/...`, `/playlist/v2/...`), not the public Web
  API, so no Web API scopes are needed.
- **pathfinder**: Spotify's internal GraphQL endpoint
  (`api-partner.spotify.com/pathfinder/v2/query`) is used by `search.rs`
  (full search), `explore.rs` (Home, Explore, genre pages) and `related.rs`
  (related artists). It takes *persisted queries*: a per-operation sha256 hash
  found in the official web player's JS bundle. Hashes change when Spotify
  ships a new web player, so they are discovered at runtime and cached.
  Failure is silent in the UI: stale cache if present, otherwise an empty or
  offline state.
- **Do not hammer Spotify.** Shared rules (`related.rs` `Gate`, used by
  `explore.rs` too): at most one request per 2 s, no retry on 429 and no
  requests until `Retry-After`, a circuit breaker after two consecutive 429s in
  a session, one in-flight request per key, fetch only what is on screen. Audio
  keys are rate-limited by Spotify too (see the vendor patches). Real-playback
  tests must be rare; use `OPENIFY_TEST_FAKE_PLAY=1`.
- **Track credits** (`engine/credits.rs`, read-only): the unofficial
  `GET spclient.wg.spotify.com/track-credits-view/v0/experimental/{id}/credits`
  the web player's credits dialog uses. Same pattern as friend activity:
  `Pathfinder::get_json` on its own gate (`pathfinder::credits`), tolerant
  parsing, typed failure codes, an in-memory cache only. The UI asks with
  `LibraryCommand::OpenCredits` and gets `LibraryEvent::Credits`. Checked once
  against a live account on 12 Nov (`live_credits`, owner's approval).
- Lyrics (`engine/lyrics.rs`) try several sources in order and cache results,
  including negative results.
- **Friend activity** (`engine/friends.rs`, optional, off by default): the
  unofficial `GET guc-spclient.spotify.com/presence-view/v1/buddylist` the
  desktop client uses. It goes through the same pathfinder client
  (`Pathfinder::get_json`: token, gate, 401 refresh, 5xx retry), on its own
  gate instance (`pathfinder::presence`) so a 429 there does not stop search
  or Explore. The result lives only in memory (UI side); the UI polls every
  60 s while the panel and the window are visible (`ui/friends.rs`
  `Schedule`), with back-off on errors. Not verified against a live account
  (`cargo test --release live_buddylist -- --ignored`, owner's approval).

## 7. Windows integration

### Discord Rich Presence (`src/discord.rs`)

`ui/integration.rs::sync_discord` maps playback and the opt-in
`Settings::discord_presence` preference to the latest desired activity. Private
sessions, pause, stop and disabling produce no activity. The Windows-only
`Presence` owns a watch sender; the `openify-discord` thread owns a small tokio
runtime and asynchronous named-pipe transport. It tries `discord-ipc-0` through
`discord-ipc-9`, performs opcode-0/READY handshake, then sends nonce-matched
SET_ACTIVITY frames with little-endian headers. Reads preserve partial frames,
reject bodies over 64 KiB, handle ping/pong and detect close/EOF. I/O has a
two-second deadline; failed sessions retry after 15 seconds without logging.
Updates coalesce and publish at most every five seconds; clears bypass that
limit. Quit cancels pending I/O and awaits worker cleanup asynchronously.
Clears get a 50 ms send deadline, without waiting for an acknowledgement, to
fit gpui's 100 ms quit window; closing the pipe also ends its presence session.
No audio-thread involvement.

The only client ID is `DISCORD_CLIENT_ID`, initially empty: no worker and no
Settings section. Test copies never open Discord pipes. Track cover URLs from
`i.scdn.co` are sent directly; a rejected activity retries once with static
asset `openify`, remembered for that connection. The status display selects
the track title; the artist remains in the activity state. Timestamps are Unix
seconds derived from the playback position,
so reconnecting retains the current progress without one update per second.
The generic async-stream transport is tested with in-memory duplex peers.

### Windows shell

`src/system` (compiled on Windows only) is OS-facing and does not know the UI;
`src/ui/integration.rs` bridges it to the app through a channel.

- `smtc.rs`: System Media Transport Controls (volume flyout card, lock screen,
  media keys).
- `taskbar.rs`: previous / play-pause / next thumbnail buttons (`ITaskbarList3`).
- `tray.rs`: notification-area icon and menu; closing the window hides to tray.
- `hook.rs`: messages gpui does not handle (`WM_HOTKEY`, `WM_COMMAND`, tray
  callback).
- `dialog.rs`: the "Save as" dialog for the library backup (`IFileSaveDialog`, called on its own thread).
- `instance.rs`: single instance via a named mutex and a hidden message window;
  a second launch forwards to the first. `OPENIFY_INSTANCE_ID` namespaces both
  for testing next to a running install.
- `icons.rs`: draws taskbar / tray icons from the app's own SVGs (`resvg`).
- `src/autostart.rs`: "start with Windows" via the per-user Run key.
- `src/update.rs`: Velopack installer hooks and auto-update; `scripts/surum-hazirla.ps1`
  builds releases.
  Bad updates roll back (`src/update/health.rs`: pure decision logic + unit
  tests). Every start of a version not yet proven healthy (window open and 20 s
  without a crash) is counted in `<cache>/update-health.json`; on the 3rd such
  start (2 failed trials) the app returns to `last_good`, first from a copy of
  that full package kept in `<cache>/rollback` (taken just before an update
  download, because Velopack deletes old packages; works offline), else from
  the feed in the background. The rolled-back version is blacklisted: not
  downloaded or applied again until a newer version is released. A clean quit
  undoes the count. Crashes before `main` reaches `update::startup` (loader
  failures) are not seen. For the feed fallback, keep the previous
  `*-full.nupkg` on the latest Release (the default feed is this repo's Releases).
- `ui/popups.rs`: the tray menu and the global palette are separate small gpui
  windows. When the main window is hidden they are destroyed on close and
  recreated on demand, to keep hidden memory low (`ui/integration.rs`
  `schedule_trim`).
- `system/app_theme.rs`: reads `AppsUseLightTheme` without modifying the registry;
  resolves Dark / Light / System and the isolated screenshot override. The existing
  `WM_SETTINGCHANGE` / `ImmersiveColorSet` event updates System mode live.
- `ui/windows_titlebar.rs`: applies the active palette and the DWM immersive dark
  mode flag to the reachable HWND.

## 8. Vendored patches and why

`[patch.crates-io]` in `Cargo.toml` points four crates at patched copies. Each
directory has an `OPENIFY.md` with the rationale and measurements, and every
changed line is tagged with an `OPENIFY:` comment.

| Crate | Upstream | What is patched | Why |
|---|---|---|---|
| `vendor/gpui` | gpui 0.2.2 (Zed, Apache-2.0) | Windows layer: the vsync thread sleeps when no window is visible; hidden windows release swapchain / MSAA surfaces; closed windows free surfaces immediately; `Window::release_memory`; shadow draw ordering; two tiny `taffy.rs` type fixes. | Idle CPU of about 0.5% and tens of MB when hidden in the tray; a shadow painted over later elements. |
| `vendor/librespot-core` | 0.8.0 (MIT) | Audio-key requests: 4 s wait, up to 3 retries with 1 s / 2 s backoff, key memo per session, quick failure when the session is dead. | Spotify delays or rejects audio-key requests under load; upstream gives up after 1.5 s and the track never starts. |
| `vendor/librespot-playback` | 0.8.0 (MIT) | Audio quality, volume normalization and its level (pre-gain for Quiet / Normal / Loud) can change while running. | Settings apply without restarting the player. |
| `vendor/librespot-connect` | 0.8.0 (MIT) | Four small patches (P1-P4): `Spirc::with_session` (attach to an already connected session), Spirc ignores player events while inactive or while the host drives (`set_host_drives`), `SpircEvent::{State(HostView), Cluster(ClusterView)}` observer events, `Spirc::add_to_queue`. | Unpatched Spirc reacts to *our* `EndOfTrack` / `Unavailable` / preload events and stops local playback, binds its own session and reports nothing outward. Spirc is started only while Connect is on (on by default since 13 Nov; can be turned off in Settings); `openify connect-spike` exercises it standalone. |

Check drift against crates.io with `scripts/vendor-diff.ps1` (see
CONTRIBUTING). CI runs it in summary mode. If you upgrade librespot or gpui,
re-apply the patches from the `OPENIFY:` markers and update the `OPENIFY.md`.

## 9. Module guide

`src/` top level:

| File | Purpose |
|---|---|
| `main.rs` | Entry: Velopack hooks, logging, crash hook, mode dispatch (UI, CLI play, logout). |
| `auth.rs` | Spotify login (browser OAuth to a local redirect) and persistent session in Credential Manager. |
| `settings.rs` | Persistent preferences (JSON), including queue rules (`dedupe_versions`, `skip_explicit`, `blocked_artists`), the normalization level and the Connect identity (`device_id`, generated once; `connect_enabled`, on by default; `device_name`, default "Openify (<PC name>)"). |
| `i18n.rs`, `i18n/` | Translation table and `t!` / `tf!` / `tp!` macros; tests for placeholders and plurals. |
| `brand.rs` | The app's display name, in one place. |
| `msg.rs` | Temporary user strings added during release prep; to be folded into the i18n table. |
| `crash.rs` | Panic hook, crash reports, log directory. |
| `update.rs`, `autostart.rs`, `link.rs`, `cli.rs`, `mem_stats.rs` | Updates, Windows autostart, Spotify link parsing, terminal player, memory accounting (feature `mem-stats`). |
| `engine/connect.rs` | Spotify Connect: UI contract, `ConnectPort` / `SpircLink` / `OutMixer`, Spirc lifecycle (`ConnectHost`). On by default. See section 3. |
| `connect_spike.rs` | Feature `connect-spike` only: `openify connect-spike`, the interactive checklist (scenarios S1-S7) the owner runs with a real account and phone to prove the Spirc assumptions before Connect was wired in. Not in the default build. |

`src/engine/`:

| File | Purpose |
|---|---|
| `mod.rs` | `Engine`, `Command`, `Event`, `EngineHandle`, runtimes, command loop, library glue. Playback is in `playback/`, the session's life in `session.rs`. |
| `session.rs` | `SessionHost` (connection / login / reconnect state), `install_session(First / Replace)`, login and reconnect flows, health check. See §3. |
| `playback/` | Playback core: `Playback` (state and decisions), `PlayerPort` / `PlayerSignal` (test seam), `Deck`, `Driver` / `TransportCmd`, recovery, gapless handover gate, resume-state writing, `testkit` (`FakePort`) and scenario tests. See §3. |
| `tests.rs` | Engine-level tests (restore, queue rules, effect order, saving during a handover). |
| `queue.rs` | Pure play-queue model: history, current, user queue, context order, balanced shuffle, repeat; applies the queue rules to the context pool. |
| `rules.rs`, `traits.rs` | Pure shuffle and queue rules (never-play artists, skip explicit, one version per song via title normalization) and the store of per-track explicit / popularity flags (`track_traits` table, created lazily, no schema bump). |
| `hidden.rs` | Locally hidden songs: `HiddenList` index (global / per context), `hidden_songs` table access, v8 migration step. |
| `automix.rs` | AutoMix analysis cache: `track_analysis` table access (v9 migration step), `Db::save_analysis` / `load_analyses`. |
| `output/beat.rs`, `output/automix.rs` | AutoMix: `BeatTap` (silence, envelope, tempo, beat phase from the decoded audio), `decide` (overlap length and beat alignment), `TailTrim` / `LeadSkip` (silence skipping). |
| `radio.rs`, `listening.rs` | Radio: station URL, tolerant parsing, start-song choice, follow-up timing (`Feed`), live read-only test; `listening.rs` is the `impl Engine` glue (start, extend, hide / show). |
| `recovery.rs`, `login.rs` | Pure policies / state machines. |
| `output/`, `tap.rs`, `levels.rs` | Audio output ring, sink wiring, level analysis. |
| `library.rs`, `cache.rs`, `resume.rs` | Library fetch, SQLite cache, resume state. |
| `wire.rs`, `edits.rs`, `edits/live.rs` | Pages and library writes wiring; edit queue; opt-in live test (`cargo test --release live_write -- --ignored`, touches a real account, never run in CI). |
| `pages.rs`, `explore.rs`, `search.rs`, `related.rs`, `lyrics.rs` | Page data, pathfinder clients, lyrics. |
| `tools/` | Library tools: duplicate finder, merge / split / sort planners, backup export, local listening stats (`PlayCounter`, aggregation, `play_history` queries), civil-time helpers; command handlers in `tools/handlers.rs`. Pure parts are unit-tested. |
| `friends.rs` | Friend activity: tolerant `buddylist` parsing, play plan (context + start position), typed failure codes. |
| `credits.rs` | Track credits: tolerant parsing (roles, people with optional artist URIs, sources), typed failure codes, in-memory cache. |
| `palette.rs` | Accent color from cover art. |
| `fixtures/` | Recorded JSON used by tests (no network in tests). |

`src/ui/`:

| File(s) | Purpose |
|---|---|
| `mod.rs` | `Shell` root view, window setup, shortcuts, event pump from the engine. |
| `theme.rs`, `components.rs`, `text.rs`, `tween.rs`, `smooth_scroll.rs`, `views.rs` | Design tokens, shared components, single-line truncating text, hover tweens, wheel handling, cached views. |
| `shelf.rs`, `library.rs`, `home.rs`, `pages.rs`, `explore.rs`, `settings.rs` | Cover shelf, library, home, artist / album / search pages, Explore, Settings. |
| `friends.rs` | Friend activity panel (`Panel::Friends`, same right-hand slot as the Now playing panel), title-bar button, rows, context menu, refresh rules (`Schedule`), test fixture hook. |
| `listening.rs` | Listening core, UI side: radio menu entry and start, hide / show menu entries and dimmed rows, the hidden-songs list in Settings, the private-session toggle (profile menu, palette, settings row, title-bar badge) and the test hooks. |
| `credits.rs` | Track credits dialog (overlay card, same layer pattern as the shortcut help): role rows, clickable artist names, one-line loading / empty / error states, keyboard cursor, test fixture hook. `lyrics_view.rs` also holds the lyrics translation toggle and its test hook. |
| `welcome.rs` | Welcome / sign-in and full-page "no data" states (offline, Premium required, engine stopped, slow) that replace endless skeletons on Home, Explore and Library when nothing is cached; skeleton delay and timeout; keyboard focus on the primary button. |
| `pill.rs`, `pill_glass.rs`, `player.rs`, `now_panel.rs`, `queue_panel.rs`, `lyrics_view.rs`, `fullscreen.rs`, `now_playing.rs` | Floating player pill, UI-side playback state, right-hand panel, queue, lyrics, full-screen view. |
| `shortcuts.rs` | The single table of bound shortcuts (`Shortcut`, exact modifier matching): every key handler resolves keys through it and the `?` / Ctrl+/ help overlay is drawn from it; tests catch a shortcut no handler uses. |
| `tools.rs`, `tools_view.rs` | Library tools page (`Route::Tools`): selection boxes (own scrolling, one Tab stop each), previews, bulk confirmation, name field, backup / stats states, play recorder; test script hook `OPENIFY_TEST_TOOLS`. |
| `palette.rs`, `fuzzy.rs`, `manage.rs`, `history.rs`, `popups.rs`, `integration.rs`, `windows_titlebar.rs` | Music search surface (Ctrl+K, title-bar pill: best result, songs, artists, albums and playlists, skeletons, grid keyboard navigation), fuzzy matching, library management (menus, drag reorder, undo), back/forward, popup windows, OS bridge, title bar. |
| `gallery.rs`, `frames.rs`, `design_rules.rs` | Component gallery (`OPENIFY_GALLERY=1`), frame-time stats, design-rule tests. |

## 10. Build, run, test

Requirements: Windows 10/11, Rust (the version pinned in rust-toolchain.toml), the
Windows SDK (for `rc.exe`, used by `build.rs` to embed the icon). No `.env` or
secrets: the Spotify client id used for login is a public constant in
`src/auth.rs`.

```powershell
cargo build --release
cargo run --release
cargo test --release
cargo clippy --release -- -D warnings
```

The Connect experiment is a separate, optional build (`connect-spike` feature):

```powershell
cargo build --release --features connect-spike --target-dir target/spike
```

It adds the `openify connect-spike` command (console app, real account and
phone; it prints its own steps). It is never run by automated tests.

`cargo build --release` and `cargo test --release` are expected to be
warning-free apart from known warnings in `vendor/librespot-core` and
`proc-macro-error2`. If a running Openify locks `target/`, build to a different
directory (`--target-dir`) and run a copy of the exe.

Tests are plain `cargo test` unit tests inside the modules (queue, recovery,
login, output ring, edit queue, i18n, settings, cache ...). They use fixtures and
fake backends and never touch Spotify or your real settings. Tests marked
`#[ignore]` (live write) hit a real account and are run by hand only.

### Design-rule tests (`src/ui/design_rules.rs`)

Count-based tests that must stay at zero: spacing, radius, font size, shadow
and color values may come only from `src/ui/theme.rs`; every `icon_button("...")`
must be followed by `.tooltip(`. A failing test means a raw value slipped in.

### Visual and performance checks

Screenshots, frame time, memory and playback reliability are measured with the
test hooks below (second monitor, silent, no focus steal, a copy of the exe,
`PrintWindow`). `scripts/vendor-diff.ps1` checks vendor drift.

### Test hooks (environment variables)

These exist for tests and measurements; none changes behaviour for normal users.

- Window and system: `OPENIFY_TEST_DISPLAY="w,h"` opens on a non-primary monitor
  without taking focus, skips the single-instance lock, disables SMTC / taskbar
  / tray / global hotkey, and self-closes after 15 min (`OPENIFY_TEST_EXIT_AFTER=sec`,
  `0` = never). `OPENIFY_TEST_SYSTEM="smtc,taskbar,tray,hotkey"` or `all` turns
  those parts back on. `OPENIFY_INSTANCE_ID=name` namespaces the single-instance
  lock and message window.
- Isolation: `OPENIFY_SETTINGS_PATH=<file>`, `OPENIFY_LOG_DIR=<dir>`,
  `OPENIFY_TEST_CACHE_DIR=<dir>` (Settings cache size / clearing act on this
  fake folder; clearing is disabled in test runs without it), `OPENIFY_TEST_EDITS`
  (in-memory edit queue with fake backend).
- Fake state: `OPENIFY_TEST_FAKE_PLAY=1` (playback appearance, nothing sent to
  Spotify), `OPENIFY_TEST_HIRES_FILE=<jpg>`, `OPENIFY_TEST_NO_HIRES=1`,
  `OPENIFY_TEST_NO_RESUME=1`, `OPENIFY_TEST_USER=name`,
  `OPENIFY_TEST_CREDITS_JSON=<file|!error|!busy|!denied|!loading|!empty>` (credits
  dialog from a fixture, no network, test runs only; open it with
  `OPENIFY_TEST_VIEW=credits`), `OPENIFY_TEST_LYRICS_JSON=<file|!none>` (fake
  playback shows lyrics from a recorded Spotify lyrics response, e.g.
  `engine/fixtures/lyrics_translated.json`; no network, test runs only),
  `OPENIFY_TEST_FRIENDS_JSON=<file|!error|!busy|!denied|!loading>` (friend
  activity from a fixture, no network, test runs only; open the panel with
  `OPENIFY_TEST_VIEW=friends`),
  `OPENIFY_TEST_CONNECT=remote|spirc` (test runs only: shows the Connect state
  without touching Spotify, e.g. the "playing on another device" strip and its
  "Play here" button),
  `OPENIFY_TEST_OFFLINE=1`, `OPENIFY_TEST_NOTICE="working|info|problem:text"`,
  `OPENIFY_TEST_LANG=en|tr`.
- Navigation: `OPENIFY_TEST_VIEW=now|home|settings|search:<q>|artist:<uri>|album:<uri>`
  after `OPENIFY_TEST_VIEW_AFTER` seconds (default 3; 0 shows the page's error
  state); `OPENIFY_GALLERY=1` component gallery.
- Library tools page (test runs only): `OPENIFY_TEST_VIEW=tools`;
  `OPENIFY_TEST_TOOLS="tool=dupes;pick=0;wait;…"` drives it through the same code
  paths as clicks (steps listed in `ui/tools.rs` `tools_script_step` and in
  `docs/TESTING.md`); `OPENIFY_TEST_BACKUP_PATH=<file>` is where the backup goes (no
  save dialog ever opens in a test run).
- Real playback (use sparingly): `OPENIFY_TEST_PLAY_AFTER=sec`,
  `OPENIFY_TEST_PLAY_ROW=n`.
- Audio: `OPENIFY_SINK_MS=20..1000` (queue target, default 120),
  `OPENIFY_SINK_TRACE=<csv>` (timeline of commands / events / xruns /
  producer gaps), `OPENIFY_TEST_TRANSITION_STALL_MS=<ms>` (simulate a stalled
  decoder at each new track; expect 0 xruns), `OPENIFY_EQ_FPS`,
  `OPENIFY_EQ_DEVICE_MS`, `OPENIFY_EQ_TRACE`.
- Measurement: `OPENIFY_FRAME_STATS=<csv>`, `OPENIFY_PROF_HIDE="sidebar,list,bar,titlebar,glow,images,equalizer,virtual,dock"`,
  `OPENIFY_TRIM_SKIP="popups,gpui,heap"`, `OPENIFY_MEM_STATS=<csv>` (build with
  `--features mem-stats`), `RUST_LOG=openify=debug`.

## 11. Conventions

Theme preferences are persisted as `theme: "dark" | "light" | "system"` in
settings. Missing or unknown values select Dark, preserving existing users'
appearance. `ui/theme.rs` owns `Palette::DARK` and `Palette::LIGHT`; `palette()`
resolves every surface and foreground token. Theme changes notify the Shell and
its observing child views. Cover-dependent pill and full-screen images are
rebuilt with a captured palette and a generation check, so an older worker
cannot paint a previous theme over the current one. Taskbar and tray icons
continue to follow Windows' shell mode, independently of the app preference.

- **New features are their own gpui `Entity`.** Do not add fields to `Shell`;
  it is already too big and splitting it is planned. A feature owns its state,
  observes what it needs and renders itself.
- **The engine sends typed codes, never user text.** Strings belong to the UI
  and the i18n table (`src/i18n/table.rs`: English and Turkish in the same
  macro entry, so a missing translation is a compile error).
- **Pure core, thin shell.** Put decisions in side-effect-free functions with
  unit tests (`queue.rs`, `recovery.rs`, `login.rs`); keep I/O at the edges
  behind small seams (`EditBackend`, fixtures).
- **Design tokens only.** Use `theme.rs` variables and `components.rs`; the
  design-rule tests enforce it.
- **No blocking on the render path.** No network, no long disk work, no waiting
  on a mutex held by background work.
- **Vendored code changes are marked** with `OPENIFY:` and documented.
- **Measure risks.** For performance-sensitive changes, state a metric and a
  threshold and report the measured value (frame time, idle CPU, hidden memory,
  xruns).
- **gpui 0.2.2 pitfalls** are recorded in `docs/TESTING.md` (wheel handlers do not hit
  test, `truncate()` caching, cached views and `window_control_area`, absolute
  elements need explicit corners, and more). Read them before touching layout
  or scrolling.

## 12. Known debt

From an internal architecture review:

- `Engine`, `Shell` and a few files (`output/`, `edits.rs`) are large and not
  encapsulated. `Engine` is down to 27 fields and `engine/mod.rs` to about 1800
  lines (the playback state and decisions moved to `playback/`, and
  connection, login and reconnect state moved to `session.rs`).
- The player now has a test seam (`PlayerPort` / `FakePort`, scenario tests in
  `playback/tests.rs`). Still concrete: the librespot `Session` (`SessionHost`
  holds the state and decisions are unit-tested, but the network calls are not
  behind a trait). Needs a real-audio
  check by hand after changes: gapless transitions, pause / seek / next while a
  handover is held, key-failure recovery.
- Persisted-query hash discovery and retry logic is duplicated in four places;
  search does not go through the shared rate gate.
- The engine still produces some user-facing text (i18n migration in progress).
- Vendor drift against upstream is checked by script, not yet enforced.
- Spirc (Connect) is wired and on by default since 13 Nov, after the
  owner's live check passed (phone picked Openify and played). Transfer-here, phone
  volume and on/off/on were not separately checked yet. Its assumptions (dealer listeners on a connected
  session, no interference while inactive, same `device_id` on reconnect, no idle
  traffic, the in-band attach / detach order, `Spirc::transfer(None)` for "Play
  here") are proven by reading source only until then. Known limits: the phone's
  volume reaches the output but not the UI slider; the queue panel shows our own
  queue while the phone drives; a phone-side seek leaves up to one queue length of
  old audio; turning Connect on or renaming restarts the Spotify session once.
- Folders (read-only) and pinning in the library, the single playback `Driver`,
  the overlap-add crossfade at the track-end margin and the EQ inside
  `RingSource::fill` are implemented; the sections above describe them.
