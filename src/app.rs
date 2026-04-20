use std::{
    env,
    io::Cursor,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Context;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use image::{DynamicImage, GenericImageView, ImageFormat};
use rand::{Rng, SeedableRng, rngs::SmallRng};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};

use crate::audio::{Album, AudioEngine, PlaybackSnapshot, load_album};

#[derive(Clone, Debug)]
pub enum Action {
    Quit,
    TogglePause,
    NextTrack,
    PreviousTrack,
    Stop,
    ToggleRepeat,
    SeekBy(Duration),
    SeekBackBy(Duration),
    RefreshLayout,
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

#[derive(Clone)]
struct CoverCache {
    width: u16,
    height: u16,
    lines: Vec<Line<'static>>,
}

pub struct App {
    pub album: Album,
    pub engine: AudioEngine,
    pub current_track: usize,
    pub repeat: bool,
    pub pulse: f32,
    pub started: Instant,
    pub progress_rect: Option<Rect>,
    pub cover_rect: Option<Rect>,
    pub visualizer: Vec<u64>,
    pub theme: Theme,
    pub accent_phase: f32,
    cover_art: Option<DynamicImage>,
    cover_png_data: Option<Vec<u8>>,
    cover_cache: Option<CoverCache>,
    rng: SmallRng,
}

impl App {
    pub async fn new(album_dir: PathBuf) -> anyhow::Result<Self> {
        let album = load_album(&album_dir)?;
        let mut engine = AudioEngine::new()?;
        engine
            .play_track(&album.tracks[0])
            .context("failed to start first track")?;

        let cover_art = album
            .cover_path
            .as_ref()
            .map(|path| {
                image::open(path)
                    .with_context(|| format!("failed to open cover art {}", path.display()))
            })
            .transpose()?;
        let cover_png_data = cover_art.as_ref().map(encode_cover_png).transpose()?;

        Ok(Self {
            album,
            engine,
            current_track: 0,
            repeat: true,
            pulse: 0.0,
            started: Instant::now(),
            progress_rect: None,
            cover_rect: None,
            visualizer: vec![3; 28],
            theme: Theme::from_env(),
            accent_phase: 0.0,
            cover_art,
            cover_png_data,
            cover_cache: None,
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
            Action::Quit | Action::RefreshLayout => {}
            Action::TogglePause => self.engine.toggle_pause(),
            Action::NextTrack => self.advance_track(false)?,
            Action::PreviousTrack => self.rewind_or_previous()?,
            Action::Stop => self.engine.stop(),
            Action::ToggleRepeat => self.repeat = !self.repeat,
            Action::SeekBy(delta) => self.seek_by(delta)?,
            Action::SeekBackBy(delta) => self.seek_back_by(delta)?,
        }

        Ok(())
    }

    pub fn action_from_mouse(&self, mouse: MouseEvent) -> Option<Action> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(progress_rect) = self.progress_rect {
                    let point = Rect::new(mouse.column, mouse.row, 1, 1);
                    if progress_rect.intersects(point) && progress_rect.width > 0 {
                        let relative = mouse.column.saturating_sub(progress_rect.x) as f32
                            / progress_rect.width.max(1) as f32;
                        let target = self
                            .current_track()
                            .duration
                            .mul_f32(relative.clamp(0.0, 1.0));
                        return Some(if target > self.playback().position {
                            Action::SeekBy(target - self.playback().position)
                        } else {
                            Action::SeekBackBy(self.playback().position - target)
                        });
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub fn set_progress_rect(&mut self, rect: Rect) {
        self.progress_rect = Some(rect);
    }

    pub fn clear_progress_rect(&mut self) {
        self.progress_rect = None;
    }

    pub fn set_cover_rect(&mut self, rect: Rect) {
        self.cover_rect = Some(rect);
    }

    pub fn clear_cover_rect(&mut self) {
        self.cover_rect = None;
    }

    pub fn playback(&self) -> PlaybackSnapshot {
        self.engine.snapshot()
    }

    pub fn current_track(&self) -> &crate::audio::Track {
        &self.album.tracks[self.current_track]
    }

    pub fn total_duration(&self) -> Duration {
        self.current_track().duration
    }

    pub fn progress_ratio(&self) -> f64 {
        let total = self.total_duration().as_secs_f64();
        if total == 0.0 {
            0.0
        } else {
            (self.playback().position.as_secs_f64() / total).clamp(0.0, 1.0)
        }
    }

    pub fn accent_glow(&self) -> Color {
        blend(
            self.theme.accent,
            self.theme.accent_alt,
            (self.accent_phase + 1.0) * 0.5,
        )
    }

    pub fn cover_dimensions(&self) -> Option<(u32, u32)> {
        self.cover_art.as_ref().map(DynamicImage::dimensions)
    }

    pub fn cover_path(&self) -> Option<&std::path::Path> {
        self.album.cover_path.as_deref()
    }

    pub fn cover_png_data(&self) -> Option<&[u8]> {
        self.cover_png_data.as_deref()
    }

    pub fn render_cover(&mut self, width: u16, height: u16) -> Vec<Line<'static>> {
        if width == 0 || height == 0 {
            return Vec::new();
        }

        if let Some(cache) = &self.cover_cache {
            if cache.width == width && cache.height == height {
                return cache.lines.clone();
            }
        }

        let lines = match &self.cover_art {
            Some(image) => render_cover_lines(image, width, height),
            None => fallback_cover(&self.album.title, width, height, &self.theme),
        };

        self.cover_cache = Some(CoverCache {
            width,
            height,
            lines: lines.clone(),
        });
        lines
    }

    fn tick_visualizer(&mut self) {
        let playing = self.playback().playing && !self.playback().paused;
        for value in &mut self.visualizer {
            let floor = if playing { 2 } else { 1 };
            let ceiling = if playing { 12 } else { 4 };
            *value = self.rng.gen_range(floor..=ceiling);
        }
    }

    pub fn resize_visualizer(&self, width: u16) -> Vec<u64> {
        let count = width as usize;
        let len = self.visualizer.len();
        if len == 0 {
            return vec![0; count];
        }
        if count == len {
            return self.visualizer.clone();
        }
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let src = i * len / count;
            result.push(self.visualizer[src.min(len - 1)]);
        }
        result
    }

    fn advance_track(&mut self, automatic: bool) -> anyhow::Result<()> {
        if self.current_track + 1 < self.album.tracks.len() {
            self.current_track += 1;
        } else if self.repeat {
            self.current_track = 0;
        } else if automatic {
            self.engine.stop();
            return Ok(());
        }

        self.play_current()
    }

    fn rewind_or_previous(&mut self) -> anyhow::Result<()> {
        if self.playback().position > Duration::from_secs(3) {
            return self.seek_to(Duration::ZERO);
        }

        if self.current_track == 0 {
            if self.repeat {
                self.current_track = self.album.tracks.len() - 1;
            }
        } else {
            self.current_track -= 1;
        }

        self.play_current()
    }

    fn play_current(&mut self) -> anyhow::Result<()> {
        let track = self.current_track().clone();
        self.engine.play_track(&track)
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
        Self {
            accent: color_from_env("MUSIC_ACCENT").unwrap_or(Color::Cyan),
            accent_alt: color_from_env("MUSIC_ACCENT_ALT").unwrap_or(Color::Blue),
            success: color_from_env("MUSIC_SUCCESS").unwrap_or(Color::Green),
            border: color_from_env("MUSIC_BORDER").unwrap_or(Color::DarkGray),
            text: color_from_env("MUSIC_TEXT").unwrap_or(Color::Reset),
            muted: color_from_env("MUSIC_MUTED").unwrap_or(Color::Gray),
            surface: color_from_env("MUSIC_SURFACE").unwrap_or(Color::Reset),
        }
    }

    pub fn panel(&self) -> Style {
        Style::default().bg(self.surface).fg(self.text)
    }
}

fn render_cover_lines(image: &DynamicImage, width: u16, height: u16) -> Vec<Line<'static>> {
    let target_w = width.max(1) as u32;
    let target_h = (height.max(1) as u32).saturating_mul(2);
    let resized = if image.width() <= target_w && image.height() <= target_h {
        image.to_rgb8()
    } else {
        image
            .resize(target_w, target_h, image::imageops::FilterType::Lanczos3)
            .to_rgb8()
    };

    let image_width = resized.width() as usize;
    let image_height = resized.height() as usize;
    let lines_needed = image_height.div_ceil(2);

    let horizontal_pad = ((width as usize).saturating_sub(image_width)) / 2;
    let vertical_pad = (height as usize).saturating_sub(lines_needed) / 2;

    let empty_line = blank_line(width as usize);
    let mut lines = Vec::new();

    for _ in 0..vertical_pad {
        lines.push(empty_line.clone());
    }

    for row in (0..image_height).step_by(2) {
        let mut spans = Vec::new();
        if horizontal_pad > 0 {
            spans.push(Span::raw(" ".repeat(horizontal_pad)));
        }

        for col in 0..image_width {
            let top = resized.get_pixel(col as u32, row as u32);
            let bottom = resized.get_pixel(col as u32, (row + 1).min(image_height - 1) as u32);
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(Color::Rgb(top[0], top[1], top[2]))
                    .bg(Color::Rgb(bottom[0], bottom[1], bottom[2])),
            ));
        }

        let right_pad = (width as usize)
            .saturating_sub(horizontal_pad)
            .saturating_sub(image_width);
        if right_pad > 0 {
            spans.push(Span::raw(" ".repeat(right_pad)));
        }

        lines.push(Line::from(spans));
    }

    while lines.len() < height as usize {
        lines.push(empty_line.clone());
    }

    lines.truncate(height as usize);
    lines
}

fn fallback_cover(title: &str, width: u16, height: u16, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::with_capacity(height as usize);
    let label = title.chars().take(width as usize).collect::<String>();

    for row in 0..height {
        let line = if row == height / 2 {
            Line::from(Span::styled(
                format!("{label:^width$}", width = width as usize),
                Style::default().fg(theme.text).bg(theme.accent_alt),
            ))
        } else {
            let mut spans = Vec::with_capacity(width as usize);
            for col in 0..width {
                let bg = if (row + col) % 2 == 0 {
                    theme.accent
                } else {
                    theme.accent_alt
                };
                spans.push(Span::styled(" ", Style::default().bg(bg)));
            }
            Line::from(spans)
        };
        lines.push(line);
    }

    lines
}

fn encode_cover_png(image: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut buffer = Cursor::new(Vec::new());
    image
        .write_to(&mut buffer, ImageFormat::Png)
        .context("failed to encode cover art as PNG")?;
    Ok(buffer.into_inner())
}

fn blank_line(width: usize) -> Line<'static> {
    Line::from(Span::raw(" ".repeat(width)))
}

fn color_from_env(key: &str) -> Option<Color> {
    let raw = env::var(key).ok()?;
    parse_color(&raw)
}

fn parse_color(raw: &str) -> Option<Color> {
    let value = raw.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }

    match value.to_ascii_lowercase().as_str() {
        "reset" | "default" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

fn blend(left: Color, right: Color, t: f32) -> Color {
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
