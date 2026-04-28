use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Context;
use catppuccin::PALETTE;
use image::GenericImageView;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use ratatui::style::{Color, Style};
use tokio::task;

use crate::audio::{Album, AudioEngine, PlaybackSnapshot, load_album};
use crate::ipc::{
    self, AlbumSnapshot, AppSnapshot, PlaybackSnapshot as RemotePlaybackSnapshot, TrackSnapshot,
};

#[derive(Clone, Debug)]
pub enum Action {
    TogglePause,
    NextTrack,
    PreviousTrack,
    Stop,
    SeekTo(Duration),
    SeekBy(Duration),
    SeekBackBy(Duration),
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub accent: Color,
    pub accent_alt: Color,
    pub accent_warm: Color,
    pub accent_cool: Color,
    pub highlight: Color,
    pub success: Color,
    pub warning: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub surface: Color,
    pub dim_surface: Color,
}

#[derive(Clone, Copy, Debug)]
enum ThemeFlavor {
    Latte,
    Frappe,
    Macchiato,
    Mocha,
}

pub struct App {
    pub album: Album,
    pub engine: AudioEngine,
    pub current_track: usize,
    pub pulse: f32,
    pub started: Instant,
    pub visualizer: Vec<u64>,
    pub accent_phase: f32,
    cover_dimensions: Option<(u32, u32)>,
    queued_track: Option<usize>,
    rng: SmallRng,
}

const GAPLESS_PRELOAD_WINDOW: Duration = Duration::from_secs(45);

impl App {
    pub async fn new(album_dir: PathBuf) -> anyhow::Result<Self> {
        let (album, cover_dimensions) = load_album_assets(album_dir).await?;
        let mut engine = AudioEngine::new()?;
        engine
            .play_track(&album.tracks[0])
            .context("failed to start first track")?;

        Ok(Self {
            album,
            engine,
            current_track: 0,
            pulse: 0.0,
            started: Instant::now(),
            visualizer: vec![3; 96],
            accent_phase: 0.0,
            cover_dimensions,
            queued_track: None,
            rng: SmallRng::from_entropy(),
        })
    }

    pub fn update(&mut self) {
        self.pulse = self.started.elapsed().as_secs_f32();
        self.accent_phase = (self.pulse * 0.7).sin();
        self.tick_visualizer();

        self.sync_gapless_transition();
        let _ = self.queue_next_for_gapless();

        let current_duration = self.current_track().duration;
        if self.queued_track.is_none() && self.engine.finished(current_duration) {
            let _ = self.advance_track(true);
        }
    }

    pub fn handle_action(&mut self, action: Action) -> anyhow::Result<()> {
        match action {
            Action::TogglePause => self.engine.toggle_pause(),
            Action::NextTrack => self.advance_track(false)?,
            Action::PreviousTrack => self.rewind_or_previous()?,
            Action::Stop => {
                self.queued_track = None;
                self.engine.stop();
            }
            Action::SeekTo(target) => self.seek_to(target)?,
            Action::SeekBy(delta) => self.seek_by(delta)?,
            Action::SeekBackBy(delta) => self.seek_back_by(delta)?,
        }

        Ok(())
    }

    pub async fn open_album(&mut self, album_dir: PathBuf) -> anyhow::Result<()> {
        *self = Self::new(album_dir).await?;
        Ok(())
    }

    pub fn playback(&self) -> PlaybackSnapshot {
        self.engine.snapshot()
    }

    pub fn current_track(&self) -> &crate::audio::Track {
        &self.album.tracks[self.current_track]
    }

    pub fn cover_dimensions(&self) -> Option<(u32, u32)> {
        self.cover_dimensions
    }

    pub fn snapshot(&self) -> AppSnapshot {
        let playback = self.playback();
        AppSnapshot {
            album: AlbumSnapshot {
                title: self.album.title.clone(),
                artist: self.album.artist.clone(),
                path: self.album.path.display().to_string(),
                cover_path: self
                    .album
                    .cover_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                tracks: self
                    .album
                    .tracks
                    .iter()
                    .map(|track| TrackSnapshot {
                        title: track.title.clone(),
                        duration_millis: ipc::duration_to_millis(track.duration),
                        sample_rate: track.sample_rate,
                        channels: track.channels,
                    })
                    .collect(),
            },
            current_track: self.current_track,
            pulse: self.pulse,
            accent_phase: self.accent_phase,
            playback: RemotePlaybackSnapshot {
                playing: playback.playing,
                paused: playback.paused,
                position_millis: ipc::duration_to_millis(playback.position),
            },
            visualizer: self.visualizer.clone(),
            cover_dimensions: self.cover_dimensions(),
        }
    }

    fn tick_visualizer(&mut self) {
        let playback = self.playback();
        let playing = playback.playing && !playback.paused;
        let len = self.visualizer.len().max(1) as f32;
        let low_center = ((self.pulse * 0.95).sin() + 1.0) * 0.5 * (len - 1.0);
        let mid_center = (((self.pulse * 1.65) + 1.2).sin() + 1.0) * 0.5 * (len - 1.0);
        let high_center = (((self.pulse * 2.45) + 2.4).sin() + 1.0) * 0.5 * (len - 1.0);

        for (index, value) in self.visualizer.iter_mut().enumerate() {
            let band = index as f32 / len;
            let low_phase = self.pulse * if playing { 4.6 } else { 1.5 } + index as f32 * 0.12;
            let mid_phase = self.pulse * if playing { 7.9 } else { 2.1 } + index as f32 * 0.31;
            let high_phase = self.pulse * if playing { 12.8 } else { 3.6 } + index as f32 * 0.56;

            let low = ((low_phase.sin() + 1.0) * 0.5).powf(0.82) * 4.8;
            let mid = ((((mid_phase * 1.1) + 0.6).cos() + 1.0) * 0.5).powf(0.76) * 4.1;
            let high = ((((high_phase * 1.7) + 1.3).sin() + 1.0) * 0.5).powf(1.5) * 2.8;

            let low_focus = (1.0 - (((index as f32 - low_center).abs() / len).min(0.5) * 2.0))
                .max(0.0)
                .powf(0.65)
                * 2.6;
            let mid_focus = (1.0 - (((index as f32 - mid_center).abs() / len).min(0.5) * 2.0))
                .max(0.0)
                .powf(0.55)
                * 2.9;
            let high_focus = (1.0 - (((index as f32 - high_center).abs() / len).min(0.5) * 2.0))
                .max(0.0)
                .powf(0.45)
                * 2.2;

            let band_weight = 1.0
                + ((1.0 - (band - 0.18).abs() * 3.0).max(0.0) * 0.3)
                + ((1.0 - (band - 0.52).abs() * 3.5).max(0.0) * 0.45)
                + ((1.0 - (band - 0.82).abs() * 4.0).max(0.0) * 0.25);
            let jitter = self.rng.gen_range(0.0..=2.8);
            let target = if playing {
                ((low + mid + high + low_focus + mid_focus + high_focus) * band_weight * 0.52
                    + jitter)
                    .min(12.0)
            } else {
                0.45 + ((low_phase * 0.6).sin() + 1.0) * 0.5
                    + ((mid_phase * 0.45).cos() + 1.0) * 0.35
                    + jitter * 0.05
            };
            let smoothing = if playing {
                if target > *value as f32 { 0.58 } else { 0.26 }
            } else {
                0.16
            };
            let next = *value as f32 * (1.0 - smoothing) + target * smoothing;
            *value = next.clamp(0.0, 12.0).round() as u64;
        }
    }

    fn advance_track(&mut self, automatic: bool) -> anyhow::Result<()> {
        if self.current_track + 1 < self.album.tracks.len() {
            self.current_track += 1;
        } else {
            self.engine.stop();
            self.queued_track = None;
            if automatic {
                self.current_track = self.album.tracks.len() - 1;
            }
            return Ok(());
        }

        if automatic && self.queued_track == Some(self.current_track) {
            self.engine.skip_one();
            self.queued_track = None;
            return Ok(());
        }

        self.play_current()
    }

    fn rewind_or_previous(&mut self) -> anyhow::Result<()> {
        if self.playback().position > Duration::from_secs(3) {
            return self.seek_to(Duration::ZERO);
        }

        if self.current_track > 0 {
            self.current_track -= 1;
        }

        self.play_current()
    }

    fn play_current(&mut self) -> anyhow::Result<()> {
        let track = self.current_track().clone();
        self.queued_track = None;
        self.engine.play_track(&track)
    }

    fn total_duration(&self) -> Duration {
        self.current_track().duration
    }

    fn seek_by(&mut self, delta: Duration) -> anyhow::Result<()> {
        let position = self.playback().position;
        let target = (position + delta).min(self.total_duration());
        self.seek_to(target)
    }

    fn seek_back_by(&mut self, delta: Duration) -> anyhow::Result<()> {
        let position = self.playback().position;
        let target = position.saturating_sub(delta);
        self.seek_to(target)
    }

    fn seek_to(&mut self, target: Duration) -> anyhow::Result<()> {
        let track = self.current_track().clone();
        let target = target.min(track.duration);
        self.queued_track = None;
        self.engine.seek_to(target, &track)
    }

    fn sync_gapless_transition(&mut self) {
        if let Some(queued_track) = self.queued_track {
            if self.engine.queued_source_count() <= 1 && !self.engine.snapshot().playing {
                self.queued_track = None;
                return;
            }

            if self.engine.queued_source_count() <= 1 {
                self.current_track = queued_track;
                self.queued_track = None;
                self.engine.reset_position_offset();
            }
        }
    }

    fn queue_next_for_gapless(&mut self) -> anyhow::Result<()> {
        if self.queued_track.is_some() || self.current_track + 1 >= self.album.tracks.len() {
            return Ok(());
        }

        let playback = self.playback();
        if !playback.playing {
            return Ok(());
        }

        let remaining = self
            .current_track()
            .duration
            .saturating_sub(playback.position);
        if remaining > GAPLESS_PRELOAD_WINDOW {
            return Ok(());
        }

        let next_track = self.current_track + 1;
        let track = self.album.tracks[next_track].clone();
        self.engine.queue_track(&track)?;
        self.queued_track = Some(next_track);
        Ok(())
    }
}

async fn load_album_assets(album_dir: PathBuf) -> anyhow::Result<(Album, Option<(u32, u32)>)> {
    task::spawn_blocking(move || {
        let album = load_album(&album_dir)?;
        let cover_dimensions = album
            .cover_path
            .as_ref()
            .map(|path| {
                image::open(path)
                    .with_context(|| format!("failed to open cover art {}", path.display()))
                    .map(|image| image.dimensions())
            })
            .transpose()?;

        Ok((album, cover_dimensions))
    })
    .await
    .context("album loading task failed")?
}

impl Theme {
    pub fn from_env() -> Self {
        let flavor = ThemeFlavor::from_env();
        let palette = flavor.palette();
        let mut accent = catppuccin_color(palette.colors.blue);
        let mut accent_alt = catppuccin_color(palette.colors.mauve);
        let mut success = catppuccin_color(palette.colors.green);
        let mut border = catppuccin_color(palette.colors.surface1);
        let mut text = catppuccin_color(palette.colors.text);
        let mut muted = catppuccin_color(palette.colors.subtext0);
        let mut surface = catppuccin_color(palette.colors.base);

        accent = env_color("MUSIC_ACCENT").unwrap_or(accent);
        accent_alt = env_color("MUSIC_ACCENT_ALT").unwrap_or(accent_alt);
        success = env_color("MUSIC_SUCCESS").unwrap_or(success);
        border = env_color("MUSIC_BORDER").unwrap_or(border);
        text = env_color("MUSIC_TEXT").unwrap_or(text);
        muted = env_color("MUSIC_MUTED").unwrap_or(muted);
        surface = env_color("MUSIC_SURFACE").unwrap_or(surface);

        Self {
            accent,
            accent_alt,
            accent_warm: blend_color(accent_alt, catppuccin_color(palette.colors.peach), 0.55),
            accent_cool: blend_color(accent, catppuccin_color(palette.colors.teal), 0.45),
            highlight: blend_color(text, accent, 0.28),
            success,
            warning: catppuccin_color(palette.colors.yellow),
            border,
            text,
            muted,
            surface,
            dim_surface: blend_color(surface, border, 0.45),
        }
    }

    pub fn panel(&self) -> Style {
        Style::default().bg(self.surface).fg(self.text)
    }
}

fn catppuccin_color(color: catppuccin::Color) -> Color {
    Color::Rgb(color.rgb.r, color.rgb.g, color.rgb.b)
}

fn env_color(key: &str) -> Option<Color> {
    env::var(key).ok().and_then(|value| parse_color(value.trim()))
}

fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }

    match value.to_ascii_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" | "purple" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" | "lightpurple" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

fn blend_color(left: Color, right: Color, t: f32) -> Color {
    match (rgb_components(left), rgb_components(right)) {
        (Some((lr, lg, lb)), Some((rr, rg, rb))) => {
            let mix = |a: u8, b: u8| -> u8 { (a as f32 + (b as f32 - a as f32) * t) as u8 };
            Color::Rgb(mix(lr, rr), mix(lg, rg), mix(lb, rb))
        }
        _ => left,
    }
}

fn rgb_components(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((205, 49, 49)),
        Color::Green => Some((13, 188, 121)),
        Color::Yellow => Some((229, 229, 16)),
        Color::Blue => Some((36, 114, 200)),
        Color::Magenta => Some((188, 63, 188)),
        Color::Cyan => Some((17, 168, 205)),
        Color::Gray => Some((229, 229, 229)),
        Color::DarkGray => Some((102, 102, 102)),
        Color::LightRed => Some((241, 76, 76)),
        Color::LightGreen => Some((35, 209, 139)),
        Color::LightYellow => Some((245, 245, 67)),
        Color::LightBlue => Some((59, 142, 234)),
        Color::LightMagenta => Some((214, 112, 214)),
        Color::LightCyan => Some((41, 184, 219)),
        Color::White => Some((255, 255, 255)),
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

impl ThemeFlavor {
    fn from_env() -> Self {
        match env::var("MUSIC_THEME")
            .ok()
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("latte") => Self::Latte,
            Some("frappe") | Some("frappé") => Self::Frappe,
            Some("macchiato") => Self::Macchiato,
            _ => Self::Mocha,
        }
    }

    fn palette(self) -> &'static catppuccin::Flavor {
        match self {
            Self::Latte => &PALETTE.latte,
            Self::Frappe => &PALETTE.frappe,
            Self::Macchiato => &PALETTE.macchiato,
            Self::Mocha => &PALETTE.mocha,
        }
    }
}
