# music

![music TUI screenshot](assets/example.png)

`music` is a minimal Rust TUI music player built with `ratatui`, `tokio`, `rodio`, and Catppuccin themes.
It is intentionally focused on one album directory at a time, with keyboard-first playback,
mouse seek support, minimal mode, and high-resolution album art rendering through terminal graphics.

## Features

- Single-album playback from a local directory
- Mouse-click seek on the progress bar
- Keyboard transport and seeking controls
- Track queue, integrated progress bar, and animated visualizer
- `--minimal` mode that hides the queue/sleeve and expands the main player view
- Terminal graphics support for album covers in compatible terminals
- Catppuccin themes: Mocha by default, plus Latte, Frappé, and Macchiato

## Requirements

- Rust stable
- A supported audio output device
- A Kitty-graphics-compatible terminal if you want full-resolution album art

In compatible terminals such as Kitty, WezTerm, and Ghostty, `music` uses terminal graphics
rendering for sharp album art. Otherwise it falls back to a text-mode cover renderer.

If the window is too small, the cover panel is hidden instead of distorting the artwork.

## Usage

```bash
cargo run -- /path/to/album
```

If no path is provided, `music` uses the current working directory.

Minimal mode:

```bash
cargo run -- --minimal /path/to/album
```

Supported audio formats:

- `mp3`
- `flac`
- `wav`
- `ogg`
- `m4a`

Recognized cover filenames:

- `cover.*`
- `folder.*`
- `front.*`
- `album.*`

## Controls

- `Space`: play / pause
- `n` or `Right`: next track
- `p` or `Left`: previous track
- `s`: stop
- `]`: seek forward 5 seconds
- `[`: seek backward 5 seconds
- `q` or `Esc`: quit

Mouse support:

- Click the progress bar to seek

## Theme Configuration

By default, the UI uses Catppuccin Mocha.
Choose a different built-in theme with `MUSIC_THEME`:

- `mocha`
- `frappe`
- `macchiato`
- `latte`

Example:

```bash
MUSIC_THEME=latte cargo run -- ~/Music/Album
```

Graphics protocol behavior can also be overridden:

- `MUSIC_FORCE_GRAPHICS=1` forces protocol image rendering
- `MUSIC_DISABLE_GRAPHICS=1` disables protocol image rendering

The UI still accepts color overrides for advanced tuning:

- `MUSIC_ACCENT`
- `MUSIC_ACCENT_ALT`
- `MUSIC_SUCCESS`
- `MUSIC_BORDER`
- `MUSIC_TEXT`
- `MUSIC_MUTED`
- `MUSIC_SURFACE`

Values can be named colors like `cyan` or hex colors like `#5fd7ff`.

Eye-candy behavior can be tuned without changing layout:

- `MUSIC_MOTION=full` uses the default animated glow
- `MUSIC_MOTION=low` slows decorative animation
- `MUSIC_MOTION=off` freezes decorative animation
- `MUSIC_COVER_COLORS=0` disables cover-derived accent colors

Example:

```bash
MUSIC_ACCENT=#ff875f MUSIC_ACCENT_ALT=#5f87ff cargo run -- ~/Music/Album
```

## License

MIT
