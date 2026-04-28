use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process,
    time::Duration,
};

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
    pub sample_rate: u32,
    pub channels: u16,
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
    SeekToMillis(u64),
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

fn runtime_path(name: &str) -> PathBuf {
    if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(runtime_dir).join(name);
    }

    let user = env::var("USER").unwrap_or_else(|_| "user".to_string());
    PathBuf::from(format!("/tmp/music-{user}-{name}"))
}

pub fn client_lock_path() -> PathBuf {
    runtime_path("client.lock")
}

pub fn client_is_showing() -> bool {
    active_pid_from_lock(&client_lock_path()).is_some()
}

pub fn acquire_client_lock() -> anyhow::Result<Option<ClientLock>> {
    let path = client_lock_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create runtime directory {}", parent.display()))?;
    }

    for _ in 0..2 {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let pid = process::id();
                writeln!(file, "{pid}")
                    .with_context(|| format!("failed to write client lock {}", path.display()))?;
                return Ok(Some(ClientLock { path, pid }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if client_is_showing() {
                    return Ok(None);
                }
                fs::remove_file(&path).ok();
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create client lock {}", path.display()));
            }
        }
    }

    if client_is_showing() {
        Ok(None)
    } else {
        Err(anyhow!("failed to acquire client lock"))
    }
}

fn active_pid_from_lock(path: &Path) -> Option<u32> {
    let pid = fs::read_to_string(path).ok()?.trim().parse::<u32>().ok()?;
    process_is_alive(pid).then_some(pid)
}

fn process_is_alive(pid: u32) -> bool {
    PathBuf::from("/proc").join(pid.to_string()).exists()
}

pub struct ClientLock {
    path: PathBuf,
    pid: u32,
}

impl Drop for ClientLock {
    fn drop(&mut self) {
        let owner = fs::read_to_string(&self.path)
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok());
        if owner == Some(self.pid) {
            fs::remove_file(&self.path).ok();
        }
    }
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
