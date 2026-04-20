use std::{env, path::PathBuf, time::Duration};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackSnapshot {
    pub title: String,
    pub duration_millis: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlbumSnapshot {
    pub title: String,
    pub artist: String,
    pub path: String,
    pub cover_path: Option<String>,
    pub tracks: Vec<TrackSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaybackSnapshot {
    pub playing: bool,
    pub paused: bool,
    pub position_millis: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSnapshot {
    pub album: AlbumSnapshot,
    pub current_track: usize,
    pub pulse: f32,
    pub accent_phase: f32,
    pub playback: PlaybackSnapshot,
    pub visualizer: Vec<u64>,
    pub cover_dimensions: Option<(u32, u32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RemoteAction {
    TogglePause,
    NextTrack,
    PreviousTrack,
    Stop,
    SeekByMillis(u64),
    SeekBackByMillis(u64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    Ping,
    Snapshot,
    Action(RemoteAction),
    OpenAlbum { album_dir: String },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    Pong,
    Ok,
    Snapshot(AppSnapshot),
    Error(String),
}

pub fn socket_path() -> PathBuf {
    if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime_dir).join("music.sock");
    }

    let user = env::var("USER").unwrap_or_else(|_| "user".to_string());
    PathBuf::from(format!("/tmp/music-{user}.sock"))
}

pub async fn send_request(request: &Request) -> anyhow::Result<Response> {
    let mut stream = UnixStream::connect(socket_path())
        .await
        .context("failed to connect to music daemon")?;
    let payload = serde_json::to_vec(request).context("failed to encode daemon request")?;
    stream
        .write_all(&payload)
        .await
        .context("failed to write daemon request")?;
    AsyncWriteExt::shutdown(&mut stream)
        .await
        .context("failed to close daemon request")?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .context("failed to read daemon response")?;
    serde_json::from_slice(&response).context("failed to decode daemon response")
}

pub async fn expect_ok(request: &Request) -> anyhow::Result<()> {
    match send_request(request).await? {
        Response::Ok | Response::Pong => Ok(()),
        Response::Error(message) => Err(anyhow!(message)),
        Response::Snapshot(_) => Err(anyhow!("unexpected snapshot response")),
    }
}

pub fn duration_to_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

pub fn duration_from_millis(millis: u64) -> Duration {
    Duration::from_millis(millis)
}
