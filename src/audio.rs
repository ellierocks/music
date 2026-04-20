use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use walkdir::WalkDir;

#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub duration: Duration,
}

#[derive(Clone, Debug)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub cover_path: Option<PathBuf>,
    pub tracks: Vec<Track>,
}

pub struct AudioEngine {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Sink,
    started_at: Option<Instant>,
    paused_at: Option<Instant>,
    paused_position: Duration,
}

#[derive(Clone, Debug)]
pub struct PlaybackSnapshot {
    pub playing: bool,
    pub paused: bool,
    pub position: Duration,
}

impl AudioEngine {
    pub fn new() -> anyhow::Result<Self> {
        let (_stream, handle) =
            OutputStream::try_default().context("failed to open audio output stream")?;
        let sink = Sink::try_new(&handle).context("failed to create audio sink")?;

        Ok(Self {
            _stream,
            handle,
            sink,
            started_at: None,
            paused_at: None,
            paused_position: Duration::ZERO,
        })
    }

    pub fn play_track(&mut self, track: &Track) -> anyhow::Result<()> {
        self.sink.stop();
        self.sink = Sink::try_new(&self.handle).context("failed to reset sink")?;

        let file = File::open(&track.path)
            .with_context(|| format!("failed to open track {}", track.path.display()))?;
        let source = Decoder::new(BufReader::new(file))
            .with_context(|| format!("failed to decode {}", track.path.display()))?;

        self.sink.append(source);
        self.sink.play();
        self.started_at = Some(Instant::now());
        self.paused_at = None;
        self.paused_position = Duration::ZERO;
        Ok(())
    }

    pub fn toggle_pause(&mut self) {
        if self.sink.is_paused() {
            self.sink.play();
            if let Some(paused_at) = self.paused_at.take() {
                self.started_at = self.started_at.map(|started| started + paused_at.elapsed());
            }
        } else {
            self.sink.pause();
            self.paused_at = Some(Instant::now());
            self.paused_position = self.position();
        }
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.started_at = None;
        self.paused_at = None;
        self.paused_position = Duration::ZERO;
    }

    pub fn seek_to(&mut self, target: Duration, track: &Track) -> anyhow::Result<()> {
        self.play_track(track)?;
        let file = File::open(&track.path)
            .with_context(|| format!("failed to open track {}", track.path.display()))?;
        let source = Decoder::new(BufReader::new(file))
            .with_context(|| format!("failed to decode {}", track.path.display()))?
            .skip_duration(target);
        self.sink.stop();
        self.sink = Sink::try_new(&self.handle).context("failed to recreate sink")?;
        self.sink.append(source);
        self.sink.play();
        self.started_at = Some(Instant::now() - target);
        self.paused_at = None;
        self.paused_position = Duration::ZERO;
        Ok(())
    }

    pub fn position(&self) -> Duration {
        if self.sink.empty() {
            return Duration::ZERO;
        }

        if self.sink.is_paused() {
            return self.paused_position;
        }

        self.started_at
            .map(|started| started.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            playing: !self.sink.empty(),
            paused: self.sink.is_paused(),
            position: self.position(),
        }
    }

    pub fn finished(&self, current_duration: Duration) -> bool {
        let snapshot = self.snapshot();
        snapshot.playing && !snapshot.paused && snapshot.position >= current_duration
    }
}

pub fn load_album(root: &Path) -> anyhow::Result<Album> {
    let mut tracks = Vec::new();
    let mut cover_path = None;

    for entry in WalkDir::new(root).min_depth(1).max_depth(2) {
        let entry = entry.context("failed to read album directory")?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if is_cover(path) && cover_path.is_none() {
            cover_path = Some(path.to_path_buf());
            continue;
        }

        if !is_supported_audio(path) {
            continue;
        }

        let duration = probe_duration(path)?;
        tracks.push(Track {
            path: path.to_path_buf(),
            title: path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("Unknown Track")
                .replace('_', " "),
            duration,
        });
    }

    tracks.sort_by(|left, right| left.path.cmp(&right.path));

    if tracks.is_empty() {
        return Err(anyhow!(
            "no supported audio files found in {}",
            root.display()
        ));
    }

    let title = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Album")
        .replace('_', " ");

    Ok(Album {
        title,
        artist: "Local Files".to_string(),
        cover_path,
        tracks,
    })
}

fn probe_duration(path: &Path) -> anyhow::Result<Duration> {
    let file = File::open(path)
        .with_context(|| format!("failed to open audio file {}", path.display()))?;
    let decoder = Decoder::new(BufReader::new(file))
        .with_context(|| format!("failed to decode {}", path.display()))?;
    decoder
        .total_duration()
        .ok_or_else(|| anyhow!("failed to determine duration for {}", path.display()))
}

fn is_supported_audio(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "mp3" | "flac" | "wav" | "ogg" | "m4a")
    )
}

fn is_cover(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.to_ascii_lowercase());

    matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg" | "webp"))
        && matches!(
            stem.as_deref(),
            Some("cover" | "folder" | "front" | "album")
        )
}
