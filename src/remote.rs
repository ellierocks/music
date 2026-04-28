use std::{
    env,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::Context;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use image::{DynamicImage, GenericImageView, ImageFormat};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
};
use tokio::task;

use crate::{
    app::Theme,
    ipc::{self, AlbumSnapshot, AppSnapshot, PlaybackSnapshot, RemoteAction, TrackSnapshot},
};

#[derive(Clone)]
struct CoverCache {
    width: u16,
    height: u16,
    lines: Vec<Line<'static>>,
}

#[derive(Clone, Copy)]
pub enum MotionLevel {
    Off,
    Low,
    Full,
}

#[derive(Clone, Copy)]
struct CoverPalette {
    low: Color,
    mid: Color,
    high: Color,
}

pub struct RemoteApp {
    pub album: AlbumSnapshot,
    pub current_track: usize,
    pub pulse: f32,
    pub minimal: bool,
    pub progress_rect: Option<Rect>,
    pub cover_rect: Option<Rect>,
    pub visualizer: Vec<u64>,
    pub theme: Theme,
    pub accent_phase: f32,
    pub motion: MotionLevel,
    playback: PlaybackSnapshot,
    cover_art: Option<DynamicImage>,
    cover_png_data: Option<Vec<u8>>,
    cover_cache: Option<CoverCache>,
    cover_palette: Option<CoverPalette>,
}

impl RemoteApp {
    pub async fn new(snapshot: AppSnapshot, minimal: bool) -> anyhow::Result<Self> {
        let theme = Theme::from_env();
        let (cover_art, cover_png_data) = load_cover(snapshot.album.cover_path.as_deref()).await?;
        let cover_palette = extract_cover_palette(cover_art.as_ref(), &theme);

        Ok(Self {
            album: snapshot.album,
            current_track: snapshot.current_track,
            pulse: snapshot.pulse,
            minimal,
            progress_rect: None,
            cover_rect: None,
            visualizer: snapshot.visualizer,
            theme,
            accent_phase: snapshot.accent_phase,
            motion: MotionLevel::from_env(),
            playback: snapshot.playback,
            cover_art,
            cover_png_data,
            cover_cache: None,
            cover_palette,
        })
    }

    pub async fn apply_snapshot(&mut self, snapshot: AppSnapshot) -> anyhow::Result<()> {
        let cover_changed = self.album.cover_path != snapshot.album.cover_path;
        self.album = snapshot.album;
        self.current_track = snapshot.current_track;
        self.pulse = snapshot.pulse;
        self.accent_phase = snapshot.accent_phase;
        self.playback = snapshot.playback;
        self.visualizer = snapshot.visualizer;

        if cover_changed {
            let (cover_art, cover_png_data) = load_cover(self.album.cover_path.as_deref()).await?;
            self.cover_art = cover_art;
            self.cover_png_data = cover_png_data;
            self.cover_cache = None;
            self.cover_palette = extract_cover_palette(self.cover_art.as_ref(), &self.theme);
        }

        Ok(())
    }

    pub fn action_from_mouse(&self, mouse: MouseEvent) -> Option<RemoteAction> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(progress_rect) = self.progress_rect {
                    let point = Rect::new(mouse.column, mouse.row, 1, 1);
                    if progress_rect.intersects(point) && progress_rect.width > 0 {
                        let relative = mouse.column.saturating_sub(progress_rect.x) as f32
                            / progress_rect.width.saturating_sub(1).max(1) as f32;
                        let target = self
                            .current_track()
                            .duration()
                            .mul_f32(relative.clamp(0.0, 1.0));
                        return Some(RemoteAction::SeekToMillis(ipc::duration_to_millis(target)));
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
        self.playback.clone()
    }

    pub fn current_track(&self) -> &TrackSnapshot {
        &self.album.tracks[self.current_track]
    }

    pub fn total_duration(&self) -> std::time::Duration {
        self.current_track().duration()
    }

    pub fn progress_ratio(&self) -> f64 {
        let total = self.total_duration().as_secs_f64();
        if total == 0.0 {
            0.0
        } else {
            let position = self.playback.position().as_secs_f64();
            (position / total).clamp(0.0, 1.0)
        }
    }

    pub fn accent_glow(&self) -> Color {
        blend(
            self.theme.accent,
            self.theme.accent_alt,
            (self.accent_phase + 1.0) * 0.5,
        )
    }

    pub fn motion_pulse(&self) -> f32 {
        match self.motion {
            MotionLevel::Off => 0.0,
            MotionLevel::Low => self.pulse * 0.35,
            MotionLevel::Full => self.pulse,
        }
    }

    pub fn spectrum_color(&self, t: f32, shimmer: f32) -> Color {
        let t = (t + shimmer * 0.03).clamp(0.0, 1.0);
        let (low, mid, high) = self
            .cover_palette
            .map(|palette| (palette.low, palette.mid, palette.high))
            .unwrap_or((self.theme.accent_cool, self.theme.accent_alt, self.theme.accent_warm));

        let color = if t < 0.5 {
            blend(low, mid, t * 2.0)
        } else {
            blend(mid, high, (t - 0.5) * 2.0)
        };

        blend(self.theme.border, color, (0.42 + shimmer * 0.58).clamp(0.0, 1.0))
    }

    pub fn glow_color(&self, intensity: f32) -> Color {
        blend(
            self.theme.border,
            self.accent_glow(),
            intensity.clamp(0.0, 1.0),
        )
    }

    pub fn cover_dimensions(&self) -> Option<(u32, u32)> {
        self.cover_art.as_ref().map(DynamicImage::dimensions)
    }

    pub fn cover_path(&self) -> Option<&Path> {
        self.album.cover_path.as_deref().map(Path::new)
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

    pub fn resize_visualizer(&self, width: u16) -> Vec<u64> {
        let count = width as usize;
        let len = self.visualizer.len();
        if len == 0 {
            return vec![0; count];
        }
        if count == 0 {
            return Vec::new();
        }
        if count == len {
            return self.visualizer.clone();
        }
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let start = i * len / count;
            let end = ((i + 1) * len).div_ceil(count).min(len);
            if end <= start {
                result.push(self.visualizer[start.min(len - 1)]);
                continue;
            }

            let slice = &self.visualizer[start..end];
            let average = slice.iter().copied().sum::<u64>() as f32 / slice.len() as f32;
            result.push(average.round() as u64);
        }
        result
    }
}

impl MotionLevel {
    fn from_env() -> Self {
        match env::var("MUSIC_MOTION")
            .ok()
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("off") | Some("0") | Some("false") => Self::Off,
            Some("low") => Self::Low,
            _ => Self::Full,
        }
    }
}

impl TrackSnapshot {
    pub fn duration(&self) -> std::time::Duration {
        ipc::duration_from_millis(self.duration_millis)
    }
}

impl PlaybackSnapshot {
    pub fn position(&self) -> std::time::Duration {
        ipc::duration_from_millis(self.position_millis)
    }
}

async fn load_cover(
    cover_path: Option<&str>,
) -> anyhow::Result<(Option<DynamicImage>, Option<Vec<u8>>)> {
    let cover_path = cover_path.map(str::to_owned);
    task::spawn_blocking(move || {
        let Some(path) = cover_path.as_deref() else {
            return Ok((None, None));
        };

        let path_buf = PathBuf::from(path);
        let is_png = path_buf
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));

        let (cover_art, cover_png_data) = if is_png {
            let png_data =
                fs::read(&path_buf).with_context(|| format!("failed to read cover art {path}"))?;
            let image = image::load_from_memory_with_format(&png_data, ImageFormat::Png)
                .with_context(|| format!("failed to decode cover art {path}"))?;
            (Some(image), Some(png_data))
        } else {
            let image = image::open(&path_buf)
                .with_context(|| format!("failed to open cover art {path}"))?;
            let png_data = encode_cover_png(&image)?;
            (Some(image), Some(png_data))
        };

        Ok((cover_art, cover_png_data))
    })
    .await
    .context("cover loading task failed")?
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

fn extract_cover_palette(image: Option<&DynamicImage>, theme: &Theme) -> Option<CoverPalette> {
    if !env_flag("MUSIC_COVER_COLORS", true) {
        return None;
    }

    let image = image?;
    let rgb = image.thumbnail(32, 32).to_rgb8();
    let mut warm = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut cool = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut bright = (0_u64, 0_u64, 0_u64, 0_u64);

    for pixel in rgb.pixels() {
        let [r, g, b] = pixel.0;
        let brightness = r as u16 + g as u16 + b as u16;
        if r >= b {
            add_rgb(&mut warm, r, g, b);
        } else {
            add_rgb(&mut cool, r, g, b);
        }
        if brightness > 220 {
            add_rgb(&mut bright, r, g, b);
        }
    }

    let low = average_rgb(cool).unwrap_or(theme.accent_cool);
    let mid = average_rgb(bright).unwrap_or(theme.highlight);
    let high = average_rgb(warm).unwrap_or(theme.accent_warm);
    Some(CoverPalette {
        low: blend(theme.accent_cool, low, 0.32),
        mid: blend(theme.accent_alt, mid, 0.28),
        high: blend(theme.accent_warm, high, 0.32),
    })
}

fn add_rgb(bucket: &mut (u64, u64, u64, u64), r: u8, g: u8, b: u8) {
    bucket.0 += r as u64;
    bucket.1 += g as u64;
    bucket.2 += b as u64;
    bucket.3 += 1;
}

fn average_rgb(bucket: (u64, u64, u64, u64)) -> Option<Color> {
    let count = bucket.3;
    if count == 0 {
        return None;
    }
    Some(Color::Rgb(
        (bucket.0 / count) as u8,
        (bucket.1 / count) as u8,
        (bucket.2 / count) as u8,
    ))
}

fn env_flag(key: &str, default: bool) -> bool {
    env::var(key)
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        })
        .unwrap_or(default)
}

fn blank_line(width: usize) -> Line<'static> {
    Line::from(Span::raw(" ".repeat(width)))
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
