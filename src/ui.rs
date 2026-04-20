use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph, Sparkline, Wrap},
};

use crate::app::{App, ButtonId};

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
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
        .constraints([Constraint::Length(42), Constraint::Min(48)])
        .spacing(2)
        .areas(inner);

    draw_cover_column(frame, left, app);
    draw_player(frame, right, app);
}

fn draw_cover_column(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let theme = app.theme.clone();
    let (image_w, image_h) = app.cover_dimensions().unwrap_or((1, 1));
    let desired_cover_height = ((area.width.saturating_sub(2) as u32 * image_h)
        / (image_w.saturating_mul(2).max(1))) as u16;
    let cover_height = desired_cover_height
        .max(8)
        .min(area.height.saturating_sub(6).max(8));
    let info_height = area.height.saturating_sub(cover_height + 1);

    let [cover_area, info_area] = if info_height >= 5 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(cover_height), Constraint::Min(5)])
            .spacing(1)
            .areas(area)
    } else {
        [area, Rect::new(area.x, area.y + area.height, area.width, 0)]
    };

    let cover_block = Block::default()
        .title(" Cover ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(theme.panel())
        .border_style(Style::default().fg(app.accent_glow()));
    let cover_inner = cover_block.inner(cover_area);
    app.set_cover_rect(cover_inner);
    frame.render_widget(cover_block, cover_area);

    let cover_lines = app.render_cover(cover_inner.width, cover_inner.height);
    frame.render_widget(
        Paragraph::new(Text::from(cover_lines)).style(theme.panel()),
        cover_inner,
    );

    if info_area.height == 0 {
        return;
    }

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
    let info_inner = info_block.inner(info_area);
    frame.render_widget(info_block, info_area);
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(Span::styled(
                app.album.title.clone(),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                app.album.artist.clone(),
                Style::default().fg(theme.muted),
            )),
            Line::from(""),
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
                Span::styled("mode", Style::default().fg(theme.muted)),
                Span::raw("  "),
                Span::styled("single-album", Style::default().fg(theme.success)),
            ]),
        ]))
        .wrap(Wrap { trim: true })
        .style(theme.panel()),
        info_inner,
    );
}

fn draw_player(frame: &mut Frame<'_>, area: Rect, app: &mut App) {
    let theme = app.theme.clone();
    let playback = app.playback();
    let track = app.current_track().clone();
    let progress = app.progress_ratio();

    let [hero, controls, tracks, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(6),
        ])
        .spacing(1)
        .areas(area);

    let hero_block = Block::default()
        .title(" Now Playing ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.accent_glow()))
        .border_type(BorderType::Rounded)
        .style(theme.panel());
    let hero_inner = hero_block.inner(hero);
    frame.render_widget(hero_block, hero);

    let elapsed = format_duration(playback.position);
    let total = format_duration(track.duration);
    let hero_text = Text::from(vec![
        Line::from(Span::styled(
            track.title,
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            app.album.title.clone(),
            Style::default().fg(theme.muted),
        )),
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
        hero_inner,
    );

    let [gauge_area, visualizer_area] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .spacing(1)
        .areas(footer);
    let progress_percent = format!("{:.0}%", progress * 100.0);

    let gauge_block = Block::default()
        .title(Line::from(vec![
            Span::raw(" Position "),
            Span::styled("•", Style::default().fg(theme.muted)),
            Span::raw(" "),
            Span::styled(progress_percent, Style::default().fg(app.accent_glow())),
            Span::raw(" "),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(theme.panel());
    let gauge_inner = gauge_block.inner(gauge_area);
    frame.render_widget(gauge_block, gauge_area);
    app.set_progress_rect(gauge_inner);
    frame.render_widget(
        Gauge::default()
            .gauge_style(
                Style::default()
                    .fg(app.accent_glow())
                    .bg(theme.surface)
                    .add_modifier(Modifier::BOLD),
            )
            .style(theme.panel())
            .use_unicode(true)
            .label("")
            .ratio(progress),
        gauge_inner,
    );

    let vis_block = Block::default()
        .title(" Glow ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(theme.panel());
    let vis_inner = vis_block.inner(visualizer_area);
    frame.render_widget(vis_block, visualizer_area);
    frame.render_widget(
        Sparkline::default()
            .data(&app.visualizer)
            .max(12)
            .bar_set(symbols::bar::NINE_LEVELS)
            .style(Style::default().fg(app.accent_glow()).bg(theme.surface)),
        vis_inner,
    );

    draw_controls(frame, controls, app, playback.paused);
    draw_tracks(frame, tracks, app);
}

fn draw_controls(frame: &mut Frame<'_>, area: Rect, app: &mut App, paused: bool) {
    let theme = app.theme.clone();
    let buttons = [
        (ButtonId::Previous, " << "),
        (
            ButtonId::PlayPause,
            if paused { " Play " } else { " Pause " },
        ),
        (ButtonId::Stop, " Stop "),
        (ButtonId::Next, " >> "),
        (
            ButtonId::Repeat,
            if app.repeat {
                " Repeat On "
            } else {
                " Repeat Off "
            },
        ),
    ];

    let button_constraints = buttons
        .iter()
        .map(|(_, label)| Constraint::Length(label.len() as u16 + 4))
        .collect::<Vec<_>>();
    let button_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(button_constraints)
        .spacing(1)
        .split(area);

    for ((button, label), rect) in buttons.into_iter().zip(button_areas.iter().copied()) {
        app.set_button_rect(button, rect);
        let is_primary = matches!(button, ButtonId::PlayPause) && !paused;
        let is_toggle = matches!(button, ButtonId::Repeat) && app.repeat;
        let base_fg = if is_primary || is_toggle {
            theme.surface
        } else {
            theme.text
        };
        let base_bg = if is_primary {
            app.accent_glow()
        } else if is_toggle {
            theme.accent_alt
        } else {
            theme.elevated
        };
        let border = if is_primary || is_toggle {
            base_bg
        } else {
            theme.border
        };

        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(Style::default().fg(base_fg).bg(base_bg))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .style(Style::default().fg(base_fg).bg(base_bg))
                        .border_style(Style::default().fg(border).bg(base_bg)),
                ),
            rect,
        );
    }
}

fn draw_tracks(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let theme = app.theme.clone();
    let block = Block::default()
        .title(" Album Queue ")
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
