mod app;
mod audio;
mod graphics;
mod ui;

use std::{env, io, path::PathBuf, time::Duration};

use anyhow::Context;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::time::interval;

use crate::app::{Action, App};
use crate::graphics::GraphicsRenderer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let album_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(env::current_dir().context("failed to determine current directory")?);

    let mut app = App::new(album_dir).await?;
    run(&mut app).await
}

async fn run(app: &mut App) -> anyhow::Result<()> {
    enable_raw_mode().context("failed to enable raw mode")?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
    terminal.clear().context("failed to clear terminal")?;

    let mut graphics = GraphicsRenderer::new();
    let result = run_loop(&mut terminal, app, &mut graphics).await;

    graphics.clear(terminal.backend_mut()).ok();
    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .ok();
    terminal.show_cursor().ok();

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    graphics: &mut GraphicsRenderer,
) -> anyhow::Result<()> {
    let mut reader = EventStream::new();
    let mut ticker = interval(Duration::from_millis(66));

    loop {
        app.update();

        terminal
            .draw(|frame| ui::draw(frame, app))
            .context("failed to draw frame")?;
        graphics
            .sync(terminal.backend_mut(), app)
            .context("failed to render album art")?;

        tokio::select! {
            _ = ticker.tick() => {}
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if let Some(action) = translate_event(event, app)? {
                            if matches!(action, Action::Quit) {
                                break;
                            }
                            app.handle_action(action).await?;
                        }
                    }
                    Some(Err(err)) => return Err(err).context("terminal event error"),
                    None => break,
                }
            }
        }
    }

    Ok(())
}

fn translate_event(event: Event, app: &App) -> anyhow::Result<Option<Action>> {
    let action = match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
            KeyCode::Char(' ') => Some(Action::TogglePause),
            KeyCode::Char('n') | KeyCode::Right => Some(Action::NextTrack),
            KeyCode::Char('p') | KeyCode::Left => Some(Action::PreviousTrack),
            KeyCode::Char('s') => Some(Action::Stop),
            KeyCode::Char('r') => Some(Action::ToggleRepeat),
            KeyCode::Char(']') => Some(Action::SeekBy(Duration::from_secs(5))),
            KeyCode::Char('[') => Some(Action::SeekBackBy(Duration::from_secs(5))),
            _ => None,
        },
        Event::Mouse(mouse) => app.action_from_mouse(mouse),
        Event::Resize(_, _) => Some(Action::RefreshLayout),
        _ => None,
    };

    Ok(action)
}
