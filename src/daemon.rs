use std::path::PathBuf;

use anyhow::{Context, anyhow};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::mpsc,
    time::{Duration, interval},
};

use crate::{
    app::{Action, App},
    ipc::{Request, Response, socket_path},
    tray::{self, TrayCommand},
};

pub async fn run(album_dir: PathBuf, current_exe: PathBuf) -> anyhow::Result<()> {
    let socket = socket_path();
    if socket.exists() {
        if UnixStream::connect(&socket).await.is_ok() {
            return Err(anyhow!("music daemon already running"));
        }
        std::fs::remove_file(&socket).ok();
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("failed to bind daemon socket {}", socket.display()))?;
    let (tray_tx, mut tray_rx) = mpsc::unbounded_channel();
    let mut _tray_handle = tray::spawn(current_exe, tray_tx).await.ok();
    let mut app = App::new(album_dir).await?;
    let mut ticker = interval(Duration::from_millis(66));

    let result = async {
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    app.update();
                }
                Some(command) = tray_rx.recv() => {
                    if handle_tray_command(&mut app, command).await? {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted.context("failed to accept daemon connection")?;
                    if handle_connection(&mut app, &mut stream).await? {
                        break;
                    }
                }
            }
        }

        Ok(())
    }
    .await;

    std::fs::remove_file(&socket).ok();
    result
}

async fn handle_connection(app: &mut App, stream: &mut UnixStream) -> anyhow::Result<bool> {
    let mut payload = Vec::new();
    stream
        .read_to_end(&mut payload)
        .await
        .context("failed to read daemon request")?;
    let request: Request = serde_json::from_slice(&payload).context("failed to parse daemon request")?;

    let response = match request {
        Request::Ping => Response::Pong,
        Request::Snapshot => Response::Snapshot(app.snapshot()),
        Request::Action(action) => {
            apply_remote_action(app, action).await?;
            Response::Ok
        }
        Request::OpenAlbum { album_dir } => {
            app.open_album(PathBuf::from(album_dir)).await?;
            Response::Ok
        }
        Request::Shutdown => {
            write_response(stream, &Response::Ok).await?;
            return Ok(true);
        }
    };

    write_response(stream, &response).await?;
    Ok(false)
}

async fn write_response(stream: &mut UnixStream, response: &Response) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(response).context("failed to encode daemon response")?;
    stream
        .write_all(&bytes)
        .await
        .context("failed to write daemon response")?;
    Ok(())
}

async fn handle_tray_command(app: &mut App, command: TrayCommand) -> anyhow::Result<bool> {
    match command {
        TrayCommand::TogglePause => app.handle_action(Action::TogglePause).await?,
        TrayCommand::Next => app.handle_action(Action::NextTrack).await?,
        TrayCommand::Previous => app.handle_action(Action::PreviousTrack).await?,
        TrayCommand::Stop => app.handle_action(Action::Stop).await?,
        TrayCommand::Quit => return Ok(true),
    }
    app.update();
    Ok(false)
}

async fn apply_remote_action(app: &mut App, action: crate::ipc::RemoteAction) -> anyhow::Result<()> {
    match action {
        crate::ipc::RemoteAction::TogglePause => app.handle_action(Action::TogglePause).await?,
        crate::ipc::RemoteAction::NextTrack => app.handle_action(Action::NextTrack).await?,
        crate::ipc::RemoteAction::PreviousTrack => app.handle_action(Action::PreviousTrack).await?,
        crate::ipc::RemoteAction::Stop => app.handle_action(Action::Stop).await?,
        crate::ipc::RemoteAction::SeekByMillis(millis) => {
            app.handle_action(Action::SeekBy(Duration::from_millis(millis))).await?
        }
        crate::ipc::RemoteAction::SeekBackByMillis(millis) => {
            app.handle_action(Action::SeekBackBy(Duration::from_millis(millis))).await?
        }
    }
    app.update();
    Ok(())
}
