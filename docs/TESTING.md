# Openify: testing and development hooks

This page lists the checks a change must pass, the environment variables that
drive test runs, and the gpui 0.2.2 pitfalls we ran into.

A **test run** is a run of the app with `OPENIFY_TEST_DISPLAY` or
`OPENIFY_SETTINGS_PATH` set. A test run never touches your real settings, cache
or Spotify account (it starts offline, with its own temporary folders) unless
you ask for it with `OPENIFY_TEST_ONLINE=1`. Hooks marked "test runs only" are
ignored outside a test run.

## Checks

- `cargo build --release` and `cargo test --release` must pass without warnings
  (the only accepted warnings are old ones from `vendor/librespot-core` and
  `proc-macro-error2`).
- `src/ui/design_rules.rs`: spacing, radius, font size, shadow and color values
  may only come from the variables in `src/ui/theme.rs`, and every
  `icon_button("...")` must be followed by `.tooltip(`.
- `src/i18n/tests.rs` (`no_raw_turkish_text_in_sources`): string literals in
  code must not contain Turkish letters; user-visible text lives in
  `src/i18n/table.rs`. Comments and log lines are exempt.
- Tests must not talk to Spotify. Use the fixtures in `src/engine/fixtures`, or
  `OPENIFY_TEST_FAKE_PLAY=1` when you run the app.

## Live tests (real account)

These are `#[ignore]`d, never run automatically, and only the account owner
should run them on purpose. They are read-only unless noted, and they print
counts and which fields are present, never names or titles.

| Command | What it does |
|---|---|
| `cargo test --release live_buddylist -- --ignored --nocapture` | Friend activity: one `buddylist` request. `Denied` means the endpoint does not accept the session token. |
| `cargo test --release live_follow -- --ignored --nocapture` | Following an artist. |
| `cargo test --release live_write -- --ignored --nocapture` | Library writes (touches the real account). |
| `cargo test --release live_credits -- --ignored --nocapture` | Track credits: one request. |
| `cargo test --release live_lyrics_translation -- --ignored --nocapture` | Lyrics translation: at most three requests per song; prints field names, language tags and counts. |
| `cargo test --release live_radio -- --ignored --nocapture` | Radio: one station request and one continuation. `OPENIFY_LIVE_SEED=spotify:artist:...` picks another seed. `Denied` means the `radio-apollo` endpoint rejects the token, `Unavailable` that it does not know the seed. |

## Environment variables

### Window and system

- `OPENIFY_TEST_DISPLAY="w,h"`: open the window on a non-primary monitor
  without taking focus (on a single monitor it opens on the main one). Turns the
  run into a test run: single-instance lock, media controls, taskbar, tray and
  global hotkey are off, and focus goes back to the user after launch. The
  window closes itself after 15 minutes.
- `OPENIFY_TEST_EXIT_AFTER=<seconds>`: other lifetime for the test window; `0`
  means never.
- `OPENIFY_TEST_SCALE=<0.5-4>`: with `OPENIFY_TEST_DISPLAY` only. Forces the
  gpui window scale (1.25 / 1.5 / 2 simulate 125-200% displays). The
  `OPENIFY_TEST_DISPLAY` size is in logical pixels, so the physical window is
  `size x scale`. A window that does not fit any screen (2560x1440, 4K) is
  placed beyond the right edge of the virtual desktop; `PrintWindow` still
  captures it completely. See `vendor/gpui/OPENIFY.md`, item 8.
- `OPENIFY_TEST_SYSTEM="smtc,taskbar,tray,hotkey"` or `all`: switch these system
  integrations on in a test run.
- `OPENIFY_INSTANCE_ID=<name>`: the single-instance lock and the message window
  use this name, so you can test single-instance behavior while your own
  Openify is open.
- `OPENIFY_TEST_LANG=en|tr`: force the interface language, overriding the
  setting and the Windows language (`src/i18n.rs`).
- `OPENIFY_TEST_PANIC_ENGINE=1`: the engine panics after 2 seconds, to see the
  "Restart" state.
- `OPENIFY_TEST_NOTICE="working|info|problem:text"`: show a notice strip.

### Fake data and fixtures

- `OPENIFY_TEST_FAKE_PLAY=1`: pretend to play without sending anything to
  Spotify.
- `OPENIFY_TEST_PLAY_AFTER=<s>` / `OPENIFY_TEST_PLAY_ROW=<n>`: start **real**
  playback after a delay. Use rarely; Spotify rate-limits audio keys.
- `OPENIFY_TEST_CONNECT=remote|spirc`: show a fixed Spotify Connect state without
  touching Spotify (`remote`: a "playing on another device" strip above the
  player with a "Transfer here" button).
- `OPENIFY_TEST_LOOK_ONLINE=1`: in an offline copy, the now-playing pill shows
  the artist and progress instead of "Offline" (used for README screenshots).
- `OPENIFY_TEST_HIRES_FILE=<jpg>`: with fake playback, the full-screen big cover
  comes from this file (no network). `OPENIFY_TEST_NO_HIRES=1` turns the big
  cover off.
- `OPENIFY_TEST_NO_RESUME=1`: do not restore where you left off, so the
  now-playing panel shows its "Recently played" state when nothing is loaded.
- `OPENIFY_TEST_USER=<name>`: show the profile button and menu as signed in as
  that account, without contacting Spotify.
- `OPENIFY_TEST_PRIVATE=1`: start in a private session (icon in the title bar).
- `OPENIFY_TEST_SEARCH_JSON=<file>`: with `OPENIFY_TEST_VIEW=search:<query>`, full
  search is filled from a recorded response instead of Spotify, for example
  `src/engine/fixtures/search_desktop.json` (images stay empty). The search
  surface (Ctrl+K / the title-bar pill) reads the same file once you type (images
  are kept, so cached covers show; the offline demo library in
  `src/engine/seed_readme.rs` uses `https://demo.invalid/covers/...` URLs). Instead
  of a file, `!loading` never answers (skeleton placeholders stay on screen) and
  `!error` fails as if Spotify were unreachable. `OPENIFY_TEST_VIEW=palette` opens
  the surface after `OPENIFY_TEST_VIEW_AFTER` seconds (default 3) and
  `OPENIFY_TEST_VIEW=palette:<query>` opens it with the query typed: the reliable
  way for screenshots. Typing with posted key presses (`ty:letters` or
  `k:vk,scan` in `scripts/ekran-sinama.ps1`; `WM_CHAR` from `t:` never reaches it)
  works for keyboard checks but now and then drops a letter.
- `OPENIFY_TEST_FRIENDS_JSON=<file>`: friend activity is filled from a fixture and
  never touches the network. The "Show friend activity" setting is switched on in
  memory only and the title-bar button appears. `ageSeconds` in the file means
  "seconds ago". Ready-made: `src/engine/fixtures/friend_activity_demo.json`. In
  place of a file, `!error`, `!busy` and `!denied` show the error states,
  `!loading` the loading state, and `{"friends":[]}` the empty state. Open the
  panel with `OPENIFY_TEST_VIEW=friends`. Test runs only.
- `OPENIFY_TEST_CREDITS_JSON=<file>`: the track credits window reads a fixture,
  no network. Ready-made: `src/engine/fixtures/track_credits.json`; states
  `!error`, `!busy`, `!denied`, `!loading`, `!empty`. Open with
  `OPENIFY_TEST_VIEW=credits`. Test runs only.
- `OPENIFY_TEST_LYRICS_JSON=<file>`: with fake playback, lyrics come from a
  Spotify lyrics response. `src/engine/fixtures/lyrics_translated.json` shows the
  translation button and translated lines, `lyrics_plain.json` has no translation
  (no button), `!none` means no lyrics. No network. Test runs only.
- `OPENIFY_TEST_RADIO_JSON=<file>`: the engine reads the radio response from a
  file, and the UI shows a radio built from the songs in it as playing (the queue
  panel says "Up next: ... radio"). Ready-made:
  `src/engine/fixtures/radio_apollo.json`; open the queue panel with
  `OPENIFY_TEST_VIEW=queue`. Test runs only.
- `OPENIFY_TEST_OPRA_FILE=<file>`: the OPRA "download" is fed from a local
  `database_v1.jsonl` and written to the cache (`<cache>\eq\`), no network;
  `!error` simulates a failed download. Test runs only.
- `OPENIFY_TEST_BACKUP_PATH=<file>`: where the library backup is written in a test
  run (no file dialog opens); without it the backup goes under
  `%TEMP%\openify-sandbox\backup\`.
- `OPENIFY_TEST_TRANSITION_STALL_MS=<ms>`: the playback thread sleeps this long
  before the first packet of every new song, simulating a decoder stuck on disk.
  Used to test the end-of-song margin (for example 3000 should give 0 xruns).
- Demo libraries (ignored tests that write a cache folder; point
  `OPENIFY_SEED_DIR` at an empty folder, then run with
  `OPENIFY_TEST_CACHE_DIR` set to the same folder):
  - `cargo test --release seed_demo_library -- --ignored`: duplicates,
    versions, 120-row playlists and about 180 plays of local history (used by
    the library tools, private session and offline views).
  - `$env:OPENIFY_SEED_DIR = $env:OPENIFY_TEST_CACHE_DIR = "<empty folder>"; cargo test --release seed_readme_library -- --ignored`:
    an English demo library with invented names and generated covers
    (`src/engine/seed_readme.rs`; `library.db`, `pages.db` and `covers\`), so
    Explore, the Home feed and artist / album pages are filled offline. Use it
    with `OPENIFY_TEST_LANG=en`.
  - Give each run a copy of the seed folder rather than reusing one folder.

### Views

- `OPENIFY_TEST_VIEW=now|home|settings|recent|help|friends|credits|queue|tools|search:<query>|artist:<uri>|album:<uri>`
  opens that page or panel `OPENIFY_TEST_VIEW_AFTER` seconds after launch
  (default 3; `0` shows the page's error state). `help` opens the shortcut
  overlay, `friends` the friend activity panel (use with
  `OPENIFY_TEST_FRIENDS_JSON`), `recent` the Recently played page, `settings` the
  Settings page, `now` the now-playing panel (also in a narrow window; with fake
  playback it has a cover), `tools` the library tools page.
- `OPENIFY_TEST_TOOLS="step;step;..."`: with `OPENIFY_TEST_VIEW=tools`, drives the
  tools page the same way clicks would (`src/ui/tools.rs`, `tools_script_step`):
  `tool=dupes|merge|split|sort|backup|stats`, `mode=split`, `pick=0,2` (indexes in
  the selection box), `wait` (until the selected lists have loaded),
  `by=artist|album|year|name|added|duration`, `dir=desc`,
  `dedupe=off|same|versions`, `review=1`, `confirm=dupes|merge|split|sort` (show
  the bulk-action confirmation card), `apply=...` (press the button and confirm
  the card if one appears), `spare=n`, `cursor=n`, `range=4w|6m|all`, `more=1`,
  `backup=1`. Test runs only.
- `OPENIFY_TEST_EQ="search=hd 6;pick=0"`: when Settings opens, runs the headphone
  search with that query and picks the nth row once the catalog arrives.
  Test runs only.
- `OPENIFY_TEST_LINK=needs_login|needs_premium|stall`: welcome, Premium-required
  and "taking long" states (`src/ui/welcome.rs`). `stall` never finishes
  connecting. No browser is opened.
- `OPENIFY_TEST_LOGIN_MS=<ms>`: fake sign-in wait that ends in "sign-in failed".
- `OPENIFY_TEST_GATE_MS="skeleton,long"`: shorten the skeleton delay and the
  "taking long" threshold, in milliseconds.
- `OPENIFY_GALLERY=1`: component gallery.

### Network, offline and isolation

- `OPENIFY_TEST_OFFLINE=1`: start offline. With an empty cache folder, Home,
  Explore and Library show the full-page "You're offline" state.
- `OPENIFY_TEST_ONLINE=1`: a test run is offline by default and does not connect
  to the real account; pass this only for a live try-out by the account owner.
- `OPENIFY_SETTINGS_PATH=<file>`: use this settings file instead of
  `%APPDATA%\Openify\settings.json`. Anything that touches settings must use it
  with a temporary file.
- `OPENIFY_TEST_CACHE_DIR=<folder>`: the whole cache folder (`library.db`,
  `pages.db`, covers, audio, and the size and clear page in Settings) lives
  there. Without it a test run uses `%TEMP%\openify-sandbox\cache` and writes its
  log to `%TEMP%\openify-sandbox\log`; the real `%LOCALAPPDATA%\Openify` is never
  touched.
- `OPENIFY_LOG_DIR=<folder>`: log and crash report folder.

### Measurement

- `RUST_LOG=openify=debug`: startup phase timings; audio device, format and
  queue target lines.
- `OPENIFY_FRAME_STATS=<csv>`: frame time recording.
- `OPENIFY_PROF_HIDE="sidebar,list,bar,titlebar,glow,images,equalizer,virtual,dock"`:
  turn regions off for A/B timing. `virtual` is the sidebar's old "draw
  everything" behavior; `dock` never lays out (or auto-opens) the now-playing
  panel.
- `OPENIFY_TRIM_SKIP="popups,gpui,heap"`: when the window is hidden, skip parts of
  the memory release (destroying popups, gpui `Window::release_memory`, heap
  compaction), to measure what each one gains (`src/ui/integration.rs`).
- `OPENIFY_EQ_FPS=30|60|...`: force the equalizer's frame rate. `0` follows the
  display; the default is 30 in front and 20 behind; it cannot exceed the
  display rate.
- `OPENIFY_EQ_DEVICE_MS=<ms>`: delay from the audio callback to the speaker
  (default 30), used for the bars' timing. `OPENIFY_EQ_TRACE=<csv>` records the
  bar inputs.
- `OPENIFY_SINK_MS=20..1000`: queue target of our own audio output (default 120;
  `src/engine/output/`).
- `OPENIFY_SINK_TRACE=<csv>`: output trace, one `tick` line every 100 ms plus a
  line per command or event, with the columns
  `t_unix_ms,event,queued_ms,xruns,gain,target_ms,underrun_ms,arg_ms`.
  - Commands and events: `pause play seek load auto_next duck unduck flush
    flush_ack gate_open start stop volume`.
  - End-of-song margin and handover: `tail_on tail_off handover first_write`
    (first packet of the new song), crossfade `xfade` (`arg_ms` is the overlap)
    and ring growth `ring_grow`.
  - `producer_gap`: while playing, the decoder was silent for more than 200 ms
    between two packets (`arg_ms` is the duration).
  - Device: `device_lost reopen reroute`.
  - Audio thread moments: `silent audible gain xrun`.
  - `target_ms` is the base target that grows with xruns, not counting the
    end-of-song margin.
- `OPENIFY_MEM_STATS=<csv>`: only in a build made with
  `cargo build --release --features mem-stats --target-dir target/memstats`.
  Every 500 ms it writes `t_s,rust_live_mb,rust_peak_mb,rust_allocs,sqlite_mb,sqlite_peak_mb`,
  to tell the Rust heap, SQLite and the rest apart.

When you run measurements, run a copy of the exe (a running exe locks the build
directory), do not talk to the real account, and report the measured value next
to the bar it had to meet.

## gpui 0.2.2 pitfalls

- **Wheel capture handlers (`on_mouse_event` in the Capture phase) do no hit
  testing.** A handler that uses `bounds.contains(position)` also takes the wheel
  meant for a layer on top of it (queue or lyrics panel) and scrolls the list
  behind. Use `smooth_scroll::wheel_zone` (hit box plus
  `Hitbox::should_handle_scroll`). A layer that covers content uses `.occlude()`
  (which also blocks the wheel); a layer that should let the wheel through (the
  player pill, the notice strip) uses `.block_mouse_except_scroll()`. States that
  do not depend on position (palette, menu / name box, full screen) go in
  `Shell::wheel_gate`.
- `truncate()` caches its first measurement, and `line_clamp` crashes on
  multi-byte characters: use our own `Line` element (`src/ui/text.rs`).
- `track_focus` on the root breaks the window buttons and edge resizing: use a
  zero-size focus anchor.
- `track_focus(handle)` ignores the element's `tab_index`; the order lives on the
  handle itself (`tab_stop` / `tab_index`).
- Releasing Enter or Space on a focused element makes gpui send a click, so after
  a mouse click and when the palette opens, focus returns to the anchor.
- A shadow with zero blur is not drawn, and a shadow on a transparent element
  paints its inside: use a ring border / outer ring.
- The `uniform_list` top offset can be "-0": compare with `> px(-0.5)`.
- Cached views (`AnyView::cached`, `src/ui/views.rs`): only a notified view is
  rebuilt, and a notify on the root does not update its children (they `observe`
  the Shell). Do not nest cached views. `window_control_area` hit areas are not
  replayed from the cache, so the title bar must stay outside the cache.
  `request_animation_frame` only wakes the view being drawn right now.
- A single-line text (`line` / `line_with_tip`) inside a block box in a flex row is
  measured at its narrowest size (0) and draws "..." even in a wide area: make the
  box `.flex()` (`line_with_tip(..).flex().min_w_0()`). A percentage upper bound
  (`max_w(relative(..))`) resolves to 0 in a container whose width is not known
  yet, so do not use it either.
- An `.absolute()` element without a corner (`top_0().left_0()`) is placed where
  it would be in the flow (below the previous sibling); in measurement canvases
  this clipped the equalizer completely.
- When an image (`img`) first loads, only the first view that asked is notified,
  so there is a bulk refresh when covers arrive.
- On Windows the wheel is always `ScrollDelta::Lines` (notches times the system
  line setting); touchpads arrive as fractional lines.
