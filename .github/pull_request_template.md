## What and why

<!-- What does this change, and why? Link the issue it closes, if any. -->

## How I checked it

<!-- Tests you ran, what you tried by hand. UI change: add a screenshot.
     Render loop, audio path or memory: include a measurement (docs/TESTING.md). -->

## Checklist

- [ ] `cargo build --release` and `cargo test --release` pass
- [ ] New user-visible text is in `src/i18n/table.rs` (English and Turkish)
- [ ] Tests do not talk to Spotify or touch real settings (fixtures, `OPENIFY_SETTINGS_PATH`)
- [ ] I agree my work is licensed under GPL-3.0-or-later
