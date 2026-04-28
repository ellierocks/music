use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::app::Theme;
use crate::ipc::PlaybackSnapshot;
use crate::remote::RemoteApp;

pub fn draw(frame: &mut Frame<'_>, app: &mut RemoteApp, graphics_active: bool) {
    let area = frame.area();
    let theme = app.theme.clone();

    let [header, body] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .areas(area);
    let note = if ((app.pulse * 2.2).sin() + 1.0) * 0.5 > 0.55 {
        "♪"
    } else {
        "♬"
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {note} "),
                Style::default().fg(app.glow_color(0.55)).bg(theme.surface),
            ),
            Span::styled(
                " MUSIC ",
                Style::default()
                    .fg(theme.surface)
                    .bg(app.accent_glow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " • ",
                Style::default().fg(app.glow_color(0.42)).bg(theme.surface),
            ),
            Span::styled(
                " one album at a time ",
                Style::default()
                    .fg(theme.text)
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .style(theme.panel()),
        header,
    );

    let inner = body.inner(Margin {
        vertical: 0,
        horizontal: 1,
    });

    if app.minimal {
        draw_right_column(frame, inner, app, graphics_active);
        return;
    }

    let [left, right] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(36),
            Constraint::Min(42),
        ])
        .spacing(2)
        .areas(inner);

    draw_library_column(frame, left, app);
    draw_right_column(frame, right, app, graphics_active);
}

fn draw_library_column(frame: &mut Frame<'_>, area: Rect, app: &RemoteApp) {
    let [queue_area, sleeve_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(11)])
        .spacing(1)
        .areas(area);

    draw_tracks(frame, queue_area, app);
    draw_sleeve(frame, sleeve_area, app);
}

fn draw_right_column(frame: &mut Frame<'_>, area: Rect, app: &mut RemoteApp, graphics_active: bool) {
    let theme = app.theme.clone();
    let playback = app.playback();
    let track = app.current_track().clone();
    let progress = app.progress_ratio();
    let (image_w, image_h) = app.cover_dimensions().unwrap_or((1, 1));

    app.clear_progress_rect();
    app.clear_cover_rect();

    let min_cover_inner_height = 8;
    let min_cover_inner_width = 16;
    let now_playing_height = 6;
    let max_cover_height = area.height.saturating_sub(now_playing_height + 5 + 2);
    let max_cover_width = area.width;
    let cover_cell_aspect_num = 9_u32;
    let cover_cell_aspect_den = 4_u32;
    let desired_cover_height = ((max_cover_width as u32 * image_h
        * cover_cell_aspect_den)
        / image_w.saturating_mul(cover_cell_aspect_num).max(1)) as u16;
    let cover_height = desired_cover_height.min(max_cover_height);
    let cover_width = ((cover_height as u32
        * image_w.saturating_mul(cover_cell_aspect_num))
        / image_h.max(1)
        / cover_cell_aspect_den.max(1)) as u16;
    let show_cover = cover_height >= min_cover_inner_height
        && cover_width >= min_cover_inner_width;

    if show_cover {
        let [now_playing, cover_slot, glow_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(now_playing_height),
                Constraint::Length(cover_height),
                Constraint::Min(5),
            ])
            .spacing(1)
            .areas(area);

        draw_now_playing(frame, now_playing, app, playback, track, progress, &theme);
        draw_cover_box(
            frame,
            center_in_area(cover_slot, cover_width, cover_height),
            app,
            graphics_active,
            &theme,
        );
        draw_glow_box(frame, glow_area, app, &theme);
    } else {
        let [now_playing, glow_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(now_playing_height),
                Constraint::Min(5),
            ])
            .spacing(1)
            .areas(area);

        draw_now_playing(frame, now_playing, app, playback, track, progress, &theme);
        draw_glow_box(frame, glow_area, app, &theme);
    }
}

fn draw_now_playing(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut RemoteApp,
    playback: PlaybackSnapshot,
    track: crate::ipc::TrackSnapshot,
    progress: f64,
    theme: &Theme,
) {
    let hero_block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Now Playing ",
            Style::default()
                .fg(app.glow_color(0.72))
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.glow_color(0.35)))
        .border_type(BorderType::Rounded)
        .style(theme.panel());
    let hero_inner = hero_block.inner(area);
    frame.render_widget(hero_block, area);

    let elapsed = format_duration(playback.position());
    let total = format_duration(track.duration());
    let [hero_text_area, progress_bar] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .areas(hero_inner);
    let hero_text = Text::from(vec![
        Line::from(Span::styled(
            display_track_title(&track.title),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(app.album.title.clone(), Style::default().fg(theme.muted)),
            Span::styled(
                format!("  •  {:02}/{:02}", app.current_track + 1, app.album.tracks.len()),
                Style::default().fg(app.accent_glow()).add_modifier(Modifier::BOLD),
            ),
            paused_or_stopped_badge(playback, theme),
        ]),
        Line::from(vec![
            Span::styled(elapsed.clone(), Style::default().fg(theme.text)),
            Span::styled(" / ", Style::default().fg(theme.muted)),
            Span::styled(total.clone(), Style::default().fg(theme.text)),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(hero_text)
            .wrap(Wrap { trim: true })
            .style(theme.panel()),
        hero_text_area,
    );
    app.set_progress_rect(progress_bar);
    frame.render_widget(
        Paragraph::new(render_progress_bar(app, theme, progress, progress_bar.width))
            .style(theme.panel()),
        progress_bar,
    );
}

fn draw_cover_box(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut RemoteApp,
    graphics_active: bool,
    theme: &Theme,
) {
    app.set_cover_rect(area);

    if !graphics_active {
        let cover_lines = app.render_cover(area.width, area.height);
        frame.render_widget(
            Paragraph::new(Text::from(cover_lines)).style(theme.panel()),
            area,
        );
    }
}

fn center_in_area(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(height) / 2);
    Rect::new(x, y, width, height)
}

fn draw_glow_box(frame: &mut Frame<'_>, area: Rect, app: &RemoteApp, theme: &Theme) {
    let vis_block = Block::default()
        .title(Span::styled(
            " Glow ",
            Style::default()
                .fg(app.glow_color(0.7))
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(app.glow_color(0.35)))
        .style(theme.panel());
    let vis_inner = vis_block.inner(area);
    frame.render_widget(vis_block, area);
    let vis_data = app.resize_visualizer(vis_inner.width.saturating_mul(2));
    frame.render_widget(
        Paragraph::new(render_glow_lines(app, theme, &vis_data, vis_inner.width, vis_inner.height))
            .style(theme.panel()),
        vis_inner,
    );
}

fn render_glow_lines(
    app: &RemoteApp,
    theme: &Theme,
    data: &[u64],
    width: u16,
    height: u16,
) -> Text<'static> {
    if width == 0 || height == 0 {
        return Text::default();
    }

    let rows = height as usize;
    let cols = width as usize;
    let total_dots = rows * 4;
    let mut lines = Vec::with_capacity(rows);

    for row in 0..rows {
        let mut spans = Vec::with_capacity(cols);
        for col in 0..cols {
            let left = bar_mask(data.get(col * 2).copied().unwrap_or(0), row, total_dots, false);
            let right = bar_mask(data.get(col * 2 + 1).copied().unwrap_or(0), row, total_dots, true);
            let cell = char::from_u32(0x2800 + left + right).unwrap_or(' ');
            let brightness = ((row + 1) as f32 / rows as f32).powf(1.35);
            let sweep = (((app.pulse * 3.4) + col as f32 * 0.22 + row as f32 * 0.12).sin() + 1.0)
                * 0.5;
            let intensity = (0.16 + brightness * 0.62 + sweep * 0.22).clamp(0.0, 1.0);
            spans.push(Span::styled(
                cell.to_string(),
                Style::default()
                    .fg(app.glow_color(intensity))
                    .bg(theme.surface),
            ));
        }
        lines.push(Line::from(spans));
    }

    Text::from(lines)
}

fn render_progress_bar(app: &RemoteApp, theme: &Theme, progress: f64, width: u16) -> Text<'static> {
    if width == 0 {
        return Text::default();
    }

    let filled = (progress.clamp(0.0, 1.0) * width as f64).round() as usize;
    let mut rail = Vec::with_capacity(width as usize);
    let mut glow = Vec::with_capacity(width as usize);
    let pulse = ((app.pulse * 2.6).sin() + 1.0) * 0.5;

    for index in 0..width as usize {
        let is_filled = index < filled;
        let head = filled.saturating_sub(1);
        let position = if width <= 1 {
            0.0
        } else {
            index as f32 / (width as f32 - 1.0)
        };
        let shimmer = (((app.pulse * 3.3) + position * 5.7).sin() + 1.0) * 0.5;

        if is_filled {
            let fill_position = if filled <= 1 {
                1.0
            } else {
                index as f32 / (filled - 1) as f32
            };
            let taper = fill_position.powf(0.65);
            let intensity = (0.50 + taper * 0.28 + pulse as f32 * 0.12 + shimmer * 0.1)
                .clamp(0.0, 1.0);
            let color = app.glow_color(intensity);
            rail.push(Span::styled(" ", Style::default().bg(color)));

            let glow_color = if index == head {
                app.accent_glow()
            } else {
                app.glow_color((0.36 + shimmer * 0.22).clamp(0.0, 1.0))
            };
            glow.push(Span::styled(
                if index == head { "◆" } else { "─" },
                Style::default()
                    .fg(glow_color)
                    .add_modifier(if index == head {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ));
        } else {
            rail.push(Span::styled(" ", Style::default().bg(theme.border)));
            glow.push(Span::styled(
                if pulse > 0.66 && position > progress as f32 {
                    "·"
                } else {
                    "─"
                },
                Style::default().fg(theme.border),
            ));
        }
    }

    Text::from(vec![Line::from(rail), Line::from(glow)])
}

fn bar_mask(value: u64, row: usize, total_dots: usize, right_column: bool) -> u32 {
    let filled = ((value.min(12) as usize) * total_dots + 11) / 12;
    let visible_from = total_dots.saturating_sub(filled);
    let dot_bits = if right_column { [3_u32, 4, 5, 7] } else { [0_u32, 1, 2, 6] };

    let mut mask = 0;
    for (dot_row, bit) in dot_bits.into_iter().enumerate() {
        let global_dot = row * 4 + dot_row;
        if global_dot >= visible_from {
            mask |= 1_u32 << bit;
        }
    }
    mask
}

fn draw_sleeve(frame: &mut Frame<'_>, area: Rect, app: &RemoteApp) {
    let theme = app.theme.clone();
    let total_duration = app
        .album
        .tracks
        .iter()
        .fold(std::time::Duration::ZERO, |sum, track| sum + track.duration());
    let detail_border = app.glow_color(0.34 + (((app.pulse * 1.8).sin() + 1.0) * 0.5) as f32 * 0.14);
    let dims = app
        .cover_dimensions()
        .map(|(w, h)| format!("{w}x{h} px"))
        .unwrap_or_else(|| "generated cover".to_string());
    let sample_rate = format_sample_rate(app.current_track().sample_rate);
    let info_block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Sleeve ",
            Style::default()
                .fg(app.glow_color(0.72))
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(theme.panel())
        .border_style(Style::default().fg(detail_border));
    let info_inner = info_block.inner(area);
    frame.render_widget(info_block, area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled("● ", Style::default().fg(app.glow_color(0.8))),
                Span::styled(
                    app.album.title.clone(),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                app.album.path.clone(),
                Style::default().fg(theme.muted),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("artist", Style::default().fg(theme.muted).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(app.album.artist.clone(), Style::default().fg(theme.text)),
            ]),
            Line::from(vec![
                Span::styled("art", Style::default().fg(theme.muted).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(dims, Style::default().fg(app.glow_color(0.74))),
            ]),
            Line::from(vec![
                Span::styled("tracks", Style::default().fg(theme.muted).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(
                    app.album.tracks.len().to_string(),
                    Style::default().fg(theme.text),
                ),
            ]),
            Line::from(vec![
                Span::styled("length", Style::default().fg(theme.muted).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(format_duration(total_duration), Style::default().fg(theme.success)),
            ]),
            Line::from(vec![
                Span::styled("rate", Style::default().fg(theme.muted).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(sample_rate, Style::default().fg(app.glow_color(0.74))),
            ]),
        ]))
        .wrap(Wrap { trim: true })
        .style(theme.panel()),
        info_inner,
    );
}

fn format_sample_rate(sample_rate: u32) -> String {
    if sample_rate >= 1000 {
        format!("{:.1} kHz", sample_rate as f32 / 1000.0)
    } else {
        format!("{sample_rate} Hz")
    }
}


fn draw_tracks(frame: &mut Frame<'_>, area: Rect, app: &RemoteApp) {
    let theme = app.theme.clone();
    let block = Block::default()
        .title(Line::from(vec![
            Span::raw(" Album Queue "),
            Span::styled(
                format!(" {} tracks ", app.album.tracks.len()),
                Style::default().fg(theme.muted),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(theme.panel());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = app
        .album
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let active = index == app.current_track;
            let title_style = if active {
                Style::default().fg(app.accent_glow()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            let duration_style = if active {
                Style::default().fg(app.accent_glow()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.muted)
            };
            let title = display_track_title(&track.title);
            let duration = format_duration(track.duration());
            let gap = inner
                .width
                .saturating_sub(title.chars().count() as u16 + duration.chars().count() as u16)
                .max(3) as usize;

            ListItem::new(Line::from(vec![
                Span::styled(title, title_style),
                Span::raw(" ".repeat(gap)),
                Span::styled(duration, duration_style),
            ]))
        })
        .collect::<Vec<_>>();

    frame.render_widget(List::new(items).style(theme.panel()), inner);
}

fn format_duration(duration: std::time::Duration) -> String {
    let total = duration.as_secs();
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn display_track_title(title: &str) -> String {
    let stripped = title.trim_start_matches(|ch: char| ch.is_ascii_digit());
    let stripped = stripped.trim_start_matches(|ch: char| matches!(ch, ' ' | '-' | '.' | '_' | '(' | ')'));

    if stripped.is_empty() {
        title.to_string()
    } else {
        stripped.to_string()
    }
}

fn paused_or_stopped_badge(playback: PlaybackSnapshot, theme: &Theme) -> Span<'static> {
    if !playback.playing {
        return Span::styled(
            "  •  STOPPED",
            Style::default().fg(theme.muted).add_modifier(Modifier::BOLD),
        );
    }

    if playback.paused {
        return Span::styled(
            "  •  PAUSED",
            Style::default().fg(theme.success).add_modifier(Modifier::BOLD),
        );
    }

    Span::raw("")
}
