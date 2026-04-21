use std::{env, path::PathBuf, time::{Duration, Instant}};

use anyhow::Context;
use catppuccin::PALETTE;
use image::GenericImageView;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use ratatui::style::{Color, Style};

use crate::audio::{Album, AudioEngine, PlaybackSnapshot, load_album};
use crate::ipc::{self, AlbumSnapshot, AppSnapshot, PlaybackSnapshot as RemotePlaybackSnapshot, TrackSnapshot};

#[derive(Clone, Debug)]
pub enum Action {
    TogglePause,
    NextTrack,
    PreviousTrack,
    Stop,
    SeekBy(Duration),
    SeekBackBy(Duration),
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub accent: Color,
    pub accent_alt: Color,
    pub success: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub surface: Color,
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
    rng: SmallRng,
}

impl App {
    pub async fn new(album_dir: PathBuf) -> anyhow::Result<Self> {
        let album = load_album(&album_dir)?;
        let mut engine = AudioEngine::new()?;
        engine
            .play_track(&album.tracks[0])
            .context("failed to start first track")?;

        let cover_dimensions = album
            .cover_path
            .as_ref()
            .map(|path| {
                image::open(path)
                    .with_context(|| format!("failed to open cover art {}", path.display()))
                    .map(|image| image.dimensions())
            })
            .transpose()?;

        Ok(Self {
            album,
            engine,
            current_track: 0,
            pulse: 0.0,
            started: Instant::now(),
            visualizer: vec![3; 96],
            accent_phase: 0.0,
            cover_dimensions,
            rng: SmallRng::from_entropy(),
        })
    }

    pub fn update(&mut self) {
        self.pulse = self.started.elapsed().as_secs_f32();
        self.accent_phase = (self.pulse * 0.7).sin();
        self.tick_visualizer();

        let current_duration = self.current_track().duration;
        if self.engine.finished(current_duration) {
            let _ = self.advance_track(true);
        }
    }

    pub async fn handle_action(&mut self, action: Action) -> anyhow::Result<()> {
        match action {
            Action::TogglePause => self.engine.toggle_pause(),
            Action::NextTrack => self.advance_track(false)?,
            Action::PreviousTrack => self.rewind_or_previous()?,
            Action::Stop => self.engine.stop(),
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
                cover_path: self.album.cover_path.as_ref().map(|path| path.display().to_string()),
                tracks: self
                    .album
                    .tracks
                    .iter()
                    .map(|track| TrackSnapshot {
                        title: track.title.clone(),
                        duration_millis: ipc::duration_to_millis(track.duration),
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
        let playing = self.playback().playing && !self.playback().paused;
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
                0.45
                    + ((low_phase * 0.6).sin() + 1.0) * 0.5
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
            if automatic {
                self.current_track = self.album.tracks.len() - 1;
            }
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
        self.engine.play_track(&track)
    }

    fn total_duration(&self) -> Duration {
        self.current_track().duration
    }

    fn seek_by(&mut self, delta: Duration) -> anyhow::Result<()> {
        let target = (self.playback().position + delta).min(self.total_duration());
        self.seek_to(target)
    }

    fn seek_back_by(&mut self, delta: Duration) -> anyhow::Result<()> {
        let target = self.playback().position.saturating_sub(delta);
        self.seek_to(target)
    }

    fn seek_to(&mut self, target: Duration) -> anyhow::Result<()> {
        let track = self.current_track().clone();
        self.engine.seek_to(target, &track)
    }
}

impl Theme {
    pub fn from_env() -> Self {
        let flavor = ThemeFlavor::from_env();
        let palette = flavor.palette();

        Self {
            accent: catppuccin_color(palette.colors.blue),
            accent_alt: catppuccin_color(palette.colors.mauve),
            success: catppuccin_color(palette.colors.green),
            border: catppuccin_color(palette.colors.surface1),
            text: catppuccin_color(palette.colors.text),
            muted: catppuccin_color(palette.colors.subtext0),
            surface: catppuccin_color(palette.colors.base),
        }
    }

    pub fn panel(&self) -> Style {
        Style::default().bg(self.surface).fg(self.text)
    }
}

fn catppuccin_color(color: catppuccin::Color) -> Color {
    Color::Rgb(color.rgb.r, color.rgb.g, color.rgb.b)
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
