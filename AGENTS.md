# AGENTS.md

## Verify Fast
- `cargo build` is the main verification step.
- `cargo test` currently runs, but there are `0` tests; do not treat it as meaningful coverage.
- Run the app with `cargo run -- /path/to/album`. If no path is passed, it uses the current working directory.

## Entrypoints
- `src/main.rs` owns terminal setup/teardown, the async event loop, resize handling, and keybindings.
- `src/app.rs` is the state machine: actions, playback state, mouse seek handling, theme/env parsing, cover rendering fallback, and visualizer data.
- `src/audio.rs` loads a single album directory and drives `rodio` playback.
- `src/graphics.rs` is the Kitty/WezTerm/Ghostty graphics path; terminal image rendering is separate from the text fallback in `app.rs`/`ui.rs`.
- `src/ui.rs` is pure layout/rendering. Keep layout math there unless state must be persisted in `App`.

## Behavior That Is Easy To Guess Wrong
- This app is single-album only. `load_album()` scans just the provided root at depth `1..=2`; do not assume a library/multi-album model.
- Album title comes from the directory name. Artist is always hardcoded to `Local Files`.
- Supported audio extensions are exactly: `mp3`, `flac`, `wav`, `ogg`, `m4a`.
- Recognized cover stems are exactly: `cover`, `folder`, `front`, `album` with extensions `png`, `jpg`, `jpeg`, `webp`.
- Mouse support is currently only click-to-seek on the progress bar. README still mentions transport button clicks, but that is stale.

## Graphics / UI Quirks
- Full-resolution cover art only renders when `GraphicsRenderer::detect_graphics_support()` passes; otherwise the app uses the text-mode cover renderer.
- Graphics support can be overridden with `MUSIC_FORCE_GRAPHICS=1` or `MUSIC_DISABLE_GRAPHICS=1`.
- On terminal resize, `main.rs` explicitly clears the graphics layer, invalidates the cached image rect, and clears the terminal before the next draw. Preserve that flow if changing resize behavior.
- `App.cover_rect` and `App.progress_rect` are render-time hitboxes used by graphics sync and mouse seeking; update them whenever those widgets move.

## Current Maintenance Notes
- `Cargo.toml` still lists `nerd-font-symbols` and `throbber-widgets-tui`, but current code no longer uses them. Re-check before adding new UI work that assumes those crates are active.
