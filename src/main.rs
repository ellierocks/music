mod app;
mod audio;
mod daemon;
mod graphics;
mod ipc;
mod remote;
mod tray;
mod ui;

use std::{
    env, io,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, anyhow};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::time::{interval, sleep};

use crate::{
    graphics::GraphicsRenderer,
    ipc::{RemoteAction, Request, Response},
    remote::RemoteApp,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let client_only = args.iter().any(|arg| arg == "--client");
    let minimal = args.iter().any(|arg| arg == "--minimal");
    let album_arg = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .map(PathBuf::from);
    let album_dir = album_arg
        .clone()
        .unwrap_or(env::current_dir().context("failed to determine current directory")?);
    let current_exe = env::current_exe().context("failed to determine current executable")?;

    if args.iter().any(|arg| arg == "--daemon") {
        return daemon::run(album_dir, current_exe).await;
    }

    let daemon_running = matches!(ipc::send_request(&Request::Ping).await, Ok(Response::Pong));
    if daemon_running {
        if album_arg.is_some() && !client_only {
            ipc::expect_ok(&Request::OpenAlbum {
                album_dir: album_dir.display().to_string(),
            })
            .await?;
        }
    } else if client_only {
        return Err(anyhow!("music daemon is not running"));
    } else {
        launch_daemon(&current_exe, &album_dir)?;
        wait_for_daemon().await?;
    }

    run_client(minimal).await
}

fn launch_daemon(current_exe: &PathBuf, album_dir: &PathBuf) -> anyhow::Result<()> {
    let mut daemon_args = vec![current_exe.display().to_string(), "--daemon".to_string()];
    daemon_args.push(album_dir.display().to_string());

    let mut command = if command_exists("setsid") {
        let mut cmd = Command::new("setsid");
        cmd.args(&daemon_args);
        cmd
    } else {
        let mut cmd = Command::new(current_exe);
        cmd.arg("--daemon");
        cmd.arg(album_dir);
        cmd
    };

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to launch music daemon")?;
    Ok(())
}

async fn wait_for_daemon() -> anyhow::Result<()> {
    for _ in 0..50 {
        if matches!(ipc::send_request(&Request::Ping).await, Ok(Response::Pong)) {
            return Ok(());
        }
        sleep(Duration::from_millis(100)).await;
    }

    Err(anyhow!("music daemon did not become ready"))
}

async fn run_client(minimal: bool) -> anyhow::Result<()> {
    let Some(_client_lock) = ipc::acquire_client_lock()? else {
        return Ok(());
    };

    let mut app = match ipc::send_request(&Request::Snapshot).await? {
        Response::Snapshot(snapshot) => RemoteApp::new(snapshot, minimal).await?,
        Response::Error(message) => return Err(anyhow!(message)),
        _ => return Err(anyhow!("unexpected daemon response")),
    };

    enable_raw_mode().context("failed to enable raw mode")?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
    terminal.clear().context("failed to clear terminal")?;

    let mut graphics = GraphicsRenderer::new();
    let result = run_loop(&mut terminal, &mut app, &mut graphics).await;

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
    app: &mut RemoteApp,
    graphics: &mut GraphicsRenderer,
) -> anyhow::Result<()> {
    let mut reader = EventStream::new();
    let mut ticker = interval(Duration::from_millis(66));
    let mut needs_redraw = true;

    loop {
        if needs_redraw {
            terminal
                .draw(|frame| ui::draw(frame, app, graphics.is_active()))
                .context("failed to draw frame")?;
            graphics
                .sync(terminal.backend_mut(), app)
                .context("failed to render album art")?;
            needs_redraw = false;
        }

        tokio::select! {
            _ = ticker.tick() => {
                refresh_snapshot(app).await?;
                needs_redraw = true;
            }
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if matches!(event, Event::Resize(_, _)) {
                            graphics.clear(terminal.backend_mut()).ok();
                            graphics.invalidate();
                            terminal.clear().ok();
                            refresh_snapshot(app).await?;
                            needs_redraw = true;
                        }
                        match translate_event(event, app)? {
                            Some(ClientEvent::Quit) => break,
                            Some(ClientEvent::Remote(action)) => {
                                ipc::expect_ok(&Request::Action(action)).await?;
                                refresh_snapshot(app).await?;
                                needs_redraw = true;
                            }
                            Some(ClientEvent::RefreshLayout) => {
                                refresh_snapshot(app).await?;
                                needs_redraw = true;
                            }
                            None => {}
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

async fn refresh_snapshot(app: &mut RemoteApp) -> anyhow::Result<()> {
    match ipc::send_request(&Request::Snapshot).await? {
        Response::Snapshot(snapshot) => app.apply_snapshot(snapshot).await,
        Response::Error(message) => Err(anyhow!(message)),
        _ => Err(anyhow!("unexpected daemon snapshot response")),
    }
}

enum ClientEvent {
    Quit,
    Remote(RemoteAction),
    RefreshLayout,
}

fn translate_event(event: Event, app: &RemoteApp) -> anyhow::Result<Option<ClientEvent>> {
    let action = match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Some(ClientEvent::Quit),
            KeyCode::Char(' ') => Some(ClientEvent::Remote(RemoteAction::TogglePause)),
            KeyCode::Char('n') | KeyCode::Right => Some(ClientEvent::Remote(RemoteAction::NextTrack)),
            KeyCode::Char('p') | KeyCode::Left => Some(ClientEvent::Remote(RemoteAction::PreviousTrack)),
            KeyCode::Char('s') => Some(ClientEvent::Remote(RemoteAction::Stop)),
            KeyCode::Char(']') => Some(ClientEvent::Remote(RemoteAction::SeekByMillis(5000))),
            KeyCode::Char('[') => Some(ClientEvent::Remote(RemoteAction::SeekBackByMillis(5000))),
            _ => None,
        },
        Event::Mouse(mouse) => app.action_from_mouse(mouse).map(ClientEvent::Remote),
        Event::Resize(_, _) => Some(ClientEvent::RefreshLayout),
        _ => None,
    };

    Ok(action)
}

fn command_exists(program: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| path.join(program).exists())
}
