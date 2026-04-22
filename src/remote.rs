use std::{
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
    playback: PlaybackSnapshot,
    cover_art: Option<DynamicImage>,
    cover_png_data: Option<Vec<u8>>,
    cover_cache: Option<CoverCache>,
}

impl RemoteApp {
    pub async fn new(snapshot: AppSnapshot, minimal: bool) -> anyhow::Result<Self> {
        let theme = Theme::from_env();
        let (cover_art, cover_png_data) = load_cover(snapshot.album.cover_path.as_deref()).await?;

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
            playback: snapshot.playback,
            cover_art,
            cover_png_data,
            cover_cache: None,
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
        }

        Ok(())
    }

    pub fn action_from_mouse(&self, mouse: MouseEvent) -> Option<RemoteAction> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(progress_rect) = self.progress_rect {
                    let point = Rect::new(mouse.column, mouse.row, 1, 1);
                    if progress_rect.intersects(point) && progress_rect.width > 0 {
                        let playback_position = self.playback.position();
                        let relative = mouse.column.saturating_sub(progress_rect.x) as f32
                            / progress_rect.width.max(1) as f32;
                        let target = self
                            .current_track()
                            .duration()
                            .mul_f32(relative.clamp(0.0, 1.0));
                        return Some(if target > playback_position {
                            RemoteAction::SeekByMillis(ipc::duration_to_millis(
                                target - playback_position,
                            ))
                        } else {
                            RemoteAction::SeekBackByMillis(ipc::duration_to_millis(
                                playback_position - target,
                            ))
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

    pub fn glow_color(&self, intensity: f32) -> Color {
        blend(self.theme.border, self.accent_glow(), intensity.clamp(0.0, 1.0))
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
            let png_data = fs::read(&path_buf)
                .with_context(|| format!("failed to read cover art {path}"))?;
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
