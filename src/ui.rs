use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Sparkline, Wrap},
};

use crate::app::{App, Theme};
use crate::audio::PlaybackSnapshot;

pub fn draw(frame: &mut Frame<'_>, app: &mut App, graphics_active: bool) {
    let area = frame.area();
    let theme = app.theme.clone();

    let [header, content] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .areas(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " MUSIC ",
                Style::default()
                    .fg(theme.surface)
                    .bg(app.accent_glow())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " one album at a time ",
                Style::default().fg(theme.text).bg(theme.surface),
            ),
        ]))
        .style(theme.panel()),
        header,
    );

    let outer = Block::default()
        .style(theme.panel())
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(theme.border));
    frame.render_widget(outer, content);

    let inner = content.inner(Margin {
        vertical: 1,
        horizontal: 2,
    });

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

fn draw_library_column(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let [queue_area, sleeve_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(11)])
        .spacing(1)
        .areas(area);

    draw_tracks(frame, queue_area, app);
    draw_sleeve(frame, sleeve_area, app);
}

fn draw_right_column(frame: &mut Frame<'_>, area: Rect, app: &mut App, graphics_active: bool) {
    let theme = app.theme.clone();
    let playback = app.playback();
    let track = app.current_track().clone();
    let progress = app.progress_ratio();
    let (image_w, image_h) = app.cover_dimensions().unwrap_or((1, 1));

    app.clear_progress_rect();
    app.clear_cover_rect();

    let min_cover_inner_height = 8;
    let min_cover_inner_width = 16;
    let max_cover_outer_height = area.height.saturating_sub(6 + 5 + 2);
    let max_cover_inner_width = area.width.saturating_sub(2);
    let max_cover_inner_height = max_cover_outer_height.saturating_sub(2);
    let desired_cover_inner_height = ((max_cover_inner_width as u32 * image_h)
        / image_w.saturating_mul(2).max(1)) as u16;
    let cover_inner_height = desired_cover_inner_height.min(max_cover_inner_height);
    let cover_inner_width = ((cover_inner_height as u32 * image_w.saturating_mul(2))
        / image_h.max(1)) as u16;
    let show_cover = cover_inner_height >= min_cover_inner_height
        && cover_inner_width >= min_cover_inner_width;

    if show_cover {
        let cover_outer_width = cover_inner_width.saturating_add(2).min(area.width);
        let cover_outer_height = cover_inner_height.saturating_add(2);
        let [now_playing, cover_slot, glow_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),
                Constraint::Length(cover_outer_height),
                Constraint::Min(5),
            ])
            .spacing(1)
            .areas(area);

        draw_now_playing(frame, now_playing, app, playback, track, progress, &theme);
        draw_cover_box(
            frame,
            center_horizontally(cover_slot, cover_outer_width),
            app,
            graphics_active,
            &theme,
        );
        draw_glow_box(frame, glow_area, app, &theme);
    } else {
        let [now_playing, glow_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),
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
    app: &mut App,
    playback: PlaybackSnapshot,
    track: crate::audio::Track,
    progress: f64,
    theme: &Theme,
) {
    let hero_block = Block::default()
        .title_top(draw_now_playing_title(app))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.accent_glow()))
        .border_type(BorderType::Rounded)
        .style(theme.panel());
    let hero_inner = hero_block.inner(area);
    frame.render_widget(hero_block, area);

    let elapsed = format_duration(playback.position);
    let total = format_duration(track.duration);
    let [hero_text_area, progress_bar] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .areas(hero_inner);
    let hero_text = Text::from(vec![
        Line::from(Span::styled(
            track.title,
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
            Span::styled(elapsed, Style::default().fg(theme.text)),
            Span::styled(" / ", Style::default().fg(theme.muted)),
            Span::styled(total, Style::default().fg(theme.text)),
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
        Gauge::default()
            .gauge_style(
                Style::default()
                    .fg(app.accent_glow())
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            )
            .use_unicode(true)
            .label("")
            .ratio(progress),
        progress_bar,
    );
}

fn draw_cover_box(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut App,
    graphics_active: bool,
    theme: &Theme,
) {
    let cover_block = Block::default()
        .title(" Cover ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(theme.panel())
        .border_style(Style::default().fg(app.accent_glow()));
    let cover_inner = cover_block.inner(area);
    app.set_cover_rect(cover_inner);
    frame.render_widget(cover_block, area);

    if !graphics_active {
        let cover_lines = app.render_cover(cover_inner.width, cover_inner.height);
        frame.render_widget(
            Paragraph::new(Text::from(cover_lines)).style(theme.panel()),
            cover_inner,
        );
    }
}

fn center_horizontally(area: Rect, width: u16) -> Rect {
    let width = width.min(area.width);
    let x = area.x.saturating_add(area.width.saturating_sub(width) / 2);
    Rect::new(x, area.y, width, area.height)
}

fn draw_glow_box(frame: &mut Frame<'_>, area: Rect, app: &App, theme: &Theme) {
    let vis_block = Block::default()
        .title(" Glow ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(theme.panel());
    let vis_inner = vis_block.inner(area);
    frame.render_widget(vis_block, area);
    let vis_data = app.resize_visualizer(vis_inner.width);
    frame.render_widget(
        Sparkline::default()
            .data(&vis_data)
            .max(12)
            .bar_set(symbols::bar::NINE_LEVELS)
            .style(Style::default().fg(app.accent_glow()).bg(theme.surface)),
        vis_inner,
    );
}

fn draw_sleeve(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let theme = app.theme.clone();
    let total_duration = app
        .album
        .tracks
        .iter()
        .fold(std::time::Duration::ZERO, |sum, track| sum + track.duration);
    let dims = app
        .cover_dimensions()
        .map(|(w, h)| format!("{w}x{h} px"))
        .unwrap_or_else(|| "generated cover".to_string());
    let info_block = Block::default()
        .title(" Sleeve ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(theme.panel())
        .border_style(Style::default().fg(theme.border));
    let info_inner = info_block.inner(area);
    frame.render_widget(info_block, area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                app.album.title.clone(),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                app.album.path.display().to_string(),
                Style::default().fg(theme.muted),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("artist", Style::default().fg(theme.muted)),
                Span::raw("  "),
                Span::styled(app.album.artist.clone(), Style::default().fg(theme.text)),
            ]),
            Line::from(vec![
                Span::styled("art", Style::default().fg(theme.muted)),
                Span::raw("  "),
                Span::styled(dims, Style::default().fg(theme.accent)),
            ]),
            Line::from(vec![
                Span::styled("tracks", Style::default().fg(theme.muted)),
                Span::raw("  "),
                Span::styled(
                    app.album.tracks.len().to_string(),
                    Style::default().fg(theme.text),
                ),
            ]),
            Line::from(vec![
                Span::styled("length", Style::default().fg(theme.muted)),
                Span::raw("  "),
                Span::styled(format_duration(total_duration), Style::default().fg(theme.success)),
            ]),
        ]))
        .wrap(Wrap { trim: true })
        .style(theme.panel()),
        info_inner,
    );
}


fn draw_tracks(frame: &mut Frame<'_>, area: Rect, app: &App) {
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
            let marker = if active { "▶" } else { " " };
            let line_style = if active {
                Style::default()
                    .fg(theme.text)
                    .bg(app.accent_glow())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text).bg(theme.surface)
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", marker), line_style),
                Span::styled(format!("{:02}. {}", index + 1, track.title), line_style),
                Span::styled(
                    "  ",
                    Style::default().bg(line_style.bg.unwrap_or(theme.surface)),
                ),
                Span::styled(
                    format_duration(track.duration),
                    if active {
                        Style::default().fg(theme.surface).bg(app.accent_glow())
                    } else {
                        Style::default().fg(theme.muted).bg(theme.surface)
                    },
                ),
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

fn draw_now_playing_title(app: &mut App) -> Line<'static> {
    let music_notes = ["♪", "♫", "♬", "♩"];
    let note = music_notes[((app.pulse * 2.0) as usize) % music_notes.len()];

    let eq_frames = ["▁▃▅", "▃▅▇", "▅▇▆", "▇▆▄", "▆▄▂", "▄▂▁"];
    let eq = eq_frames[((app.pulse * 3.0) as usize) % eq_frames.len()];

    Line::from(vec![
        Span::raw(" "),
        Span::styled(note, Style::default().fg(app.accent_glow())),
        Span::raw(" "),
        Span::styled("Now Playing", animated_title_style(app)),
        Span::raw(" "),
        Span::styled(eq, Style::default().fg(app.accent_glow())),
        Span::raw(" "),
    ])
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

fn animated_title_style(app: &App) -> Style {
    let flash = ((app.pulse * 2.5).sin() + 1.0) * 0.5;
    let color = if flash > 0.55 {
        app.accent_glow()
    } else {
        app.theme.text
    };

    Style::default().fg(color).add_modifier(if flash > 0.72 {
        Modifier::BOLD
    } else {
        Modifier::empty()
    })
}
