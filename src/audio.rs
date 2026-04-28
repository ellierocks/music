use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use anyhow::{Context, anyhow};
#[cfg(test)]
use audioadapter_buffers::direct::InterleavedSlice;
use rodio::{
    Decoder, OutputStream, OutputStreamHandle, Sink, Source,
    cpal::{
        SampleFormat, SampleRate, SupportedStreamConfig, SupportedStreamConfigRange,
        traits::{DeviceTrait, HostTrait},
    },
};
#[cfg(test)]
use rubato::{Fft, FixedSync, Resampler};
use walkdir::WalkDir;

#[cfg(test)]
const RESAMPLER_CHUNK_SIZE: usize = 4096;
#[cfg(test)]
const RESAMPLER_SUB_CHUNKS: usize = 1;

#[derive(Clone, Debug)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub duration: Duration,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Debug)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub path: PathBuf,
    pub cover_path: Option<PathBuf>,
    pub tracks: Vec<Track>,
}

pub struct AudioEngine {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Sink,
    position_offset: Duration,
    active: bool,
}

#[derive(Clone, Debug)]
pub struct PlaybackSnapshot {
    pub playing: bool,
    pub paused: bool,
    pub position: Duration,
}

impl AudioEngine {
    pub fn new() -> anyhow::Result<Self> {
        let host = rodio::cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("failed to find default audio output device"))?;
        let output_config =
            select_output_config(&device).context("failed to choose audio output stream config")?;
        let (_stream, handle) = OutputStream::try_from_device_config(&device, output_config)
            .context("failed to open audio output stream")?;
        let sink = Sink::try_new(&handle).context("failed to create audio sink")?;

        Ok(Self {
            _stream,
            handle,
            sink,
            position_offset: Duration::ZERO,
            active: false,
        })
    }

    pub fn play_track(&mut self, track: &Track) -> anyhow::Result<()> {
        self.sink.stop();
        self.sink = Sink::try_new(&self.handle).context("failed to reset sink")?;
        self.position_offset = Duration::ZERO;
        self.append_track(track)?;
        self.sink.play();
        self.active = true;
        Ok(())
    }

    pub fn queue_track(&mut self, track: &Track) -> anyhow::Result<()> {
        self.append_track(track)
    }

    fn append_track(&mut self, track: &Track) -> anyhow::Result<()> {
        let file = File::open(&track.path)
            .with_context(|| format!("failed to open track {}", track.path.display()))?;
        let source = Decoder::new(BufReader::new(file))
            .with_context(|| format!("failed to decode {}", track.path.display()))?
            .convert_samples::<f32>();
        let source = prepare_playback_source(source)?;

        self.sink.append(source);
        Ok(())
    }

    pub fn toggle_pause(&mut self) {
        if self.sink.is_paused() {
            self.sink.play();
        } else {
            self.sink.pause();
        }
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.position_offset = Duration::ZERO;
        self.active = false;
    }

    pub fn seek_to(&mut self, target: Duration, track: &Track) -> anyhow::Result<()> {
        let was_paused = self.sink.is_paused();
        let file = File::open(&track.path)
            .with_context(|| format!("failed to open track {}", track.path.display()))?;
        let source = Decoder::new(BufReader::new(file))
            .with_context(|| format!("failed to decode {}", track.path.display()))?
            .skip_duration(target)
            .convert_samples::<f32>();
        let source = prepare_playback_source(source)?;
        self.sink.stop();
        self.sink = Sink::try_new(&self.handle).context("failed to recreate sink")?;
        self.position_offset = target;
        self.sink.append(source);
        if was_paused {
            self.sink.pause();
        } else {
            self.sink.play();
        }
        self.active = true;
        Ok(())
    }

    pub fn skip_one(&mut self) {
        self.sink.skip_one();
        self.position_offset = Duration::ZERO;
    }

    pub fn reset_position_offset(&mut self) {
        self.position_offset = Duration::ZERO;
    }

    pub fn queued_source_count(&self) -> usize {
        self.sink.len()
    }

    fn position(&self) -> Duration {
        if !self.active || self.sink.empty() {
            return Duration::ZERO;
        }

        self.position_offset + self.sink.get_pos()
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            playing: self.active && !self.sink.empty(),
            paused: self.sink.is_paused(),
            position: self.position(),
        }
    }

    pub fn finished(&self, current_duration: Duration) -> bool {
        self.active
            && !self.sink.is_paused()
            && (self.sink.empty() || self.position() >= current_duration)
    }
}

fn select_output_config(device: &rodio::cpal::Device) -> anyhow::Result<SupportedStreamConfig> {
    let default_config = device
        .default_output_config()
        .context("failed to query default output stream config")?;

    let supported_configs = match device.supported_output_configs() {
        Ok(configs) => configs.collect::<Vec<_>>(),
        Err(_) => return Ok(default_config),
    };

    let default_channels = default_config.channels();
    let preferred = supported_configs
        .iter()
        .filter(|config| config.channels() == default_channels)
        .max_by(|left, right| compare_config_priority(left, right, default_config.sample_format()))
        .or_else(|| {
            supported_configs.iter().max_by(|left, right| {
                compare_config_priority(left, right, default_config.sample_format())
            })
        })
        .cloned();

    Ok(preferred
        .map(select_preferred_sample_rate)
        .unwrap_or(default_config))
}

fn compare_config_priority(
    left: &SupportedStreamConfigRange,
    right: &SupportedStreamConfigRange,
    default_sample_format: SampleFormat,
) -> std::cmp::Ordering {
    left.max_sample_rate()
        .cmp(&right.max_sample_rate())
        .then_with(|| {
            sample_format_priority(left.sample_format(), default_sample_format).cmp(
                &sample_format_priority(right.sample_format(), default_sample_format),
            )
        })
}

fn sample_format_priority(sample_format: SampleFormat, default_sample_format: SampleFormat) -> u8 {
    match sample_format {
        format if format == default_sample_format => 3,
        SampleFormat::F32 => 2,
        SampleFormat::F64
        | SampleFormat::I64
        | SampleFormat::U64
        | SampleFormat::I32
        | SampleFormat::U32 => 1,
        _ => 0,
    }
}

fn select_preferred_sample_rate(config: SupportedStreamConfigRange) -> SupportedStreamConfig {
    for preferred_rate in [192_000_u32, 176_400, 96_000, 88_200] {
        if let Some(config) = config.try_with_sample_rate(SampleRate(preferred_rate)) {
            return config;
        }
    }

    config.with_max_sample_rate()
}

fn prepare_playback_source<S>(source: S) -> anyhow::Result<Box<dyn Source<Item = f32> + Send>>
where
    S: Source<Item = f32> + Send + 'static,
{
    Ok(Box::new(source))
}

#[cfg(test)]
fn resample_interleaved_samples(
    mut samples: Vec<f32>,
    channels: u16,
    input_sample_rate: u32,
    output_sample_rate: u32,
) -> anyhow::Result<Vec<f32>> {
    if input_sample_rate == output_sample_rate {
        return Ok(samples);
    }

    let channel_count = channels as usize;
    if channel_count == 0 {
        return Ok(Vec::new());
    }

    let frames = samples.len() / channel_count;
    samples.truncate(frames * channel_count);
    if frames == 0 {
        return Ok(Vec::new());
    }

    let samples64: Vec<f64> = samples.into_iter().map(f64::from).collect();

    let input = InterleavedSlice::new(&samples64, channel_count, frames)
        .context("failed to prepare resampler input buffer")?;
    let mut resampler = Fft::<f64>::new(
        input_sample_rate as usize,
        output_sample_rate as usize,
        RESAMPLER_CHUNK_SIZE,
        RESAMPLER_SUB_CHUNKS,
        channel_count,
        FixedSync::Both,
    )
    .context("failed to create rubato resampler")?;

    let output_frames_capacity = resampler.process_all_needed_output_len(frames);
    let mut output = vec![0.0f64; output_frames_capacity * channel_count];
    let mut output_buffer =
        InterleavedSlice::new_mut(&mut output, channel_count, output_frames_capacity)
            .context("failed to prepare resampler output buffer")?;
    let (_, output_frames) = resampler
        .process_all_into_buffer(&input, &mut output_buffer, frames, None)
        .context("failed to resample audio")?;
    output.truncate(output_frames * channel_count);

    Ok(output.into_iter().map(|sample| sample as f32).collect())
}

pub fn load_album(root: &Path) -> anyhow::Result<Album> {
    let mut audio_paths = Vec::new();
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

        audio_paths.push(path.to_path_buf());
    }

    let mut tracks = probe_tracks(audio_paths)?;

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
    let artist = root
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Local Files")
        .replace('_', " ");

    Ok(Album {
        title,
        artist,
        path: root.to_path_buf(),
        cover_path,
        tracks,
    })
}

fn probe_tracks(paths: Vec<PathBuf>) -> anyhow::Result<Vec<Track>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(paths.len());

    if worker_count <= 1 {
        return paths
            .into_iter()
            .map(|path| {
                let metadata = probe_track_metadata(&path)?;
                Ok(Track {
                    title: track_title(&path),
                    path,
                    duration: metadata.duration,
                    sample_rate: metadata.sample_rate,
                    channels: metadata.channels,
                })
            })
            .collect();
    }

    let chunk_size = paths.len().div_ceil(worker_count);
    let mut handles = Vec::with_capacity(worker_count);

    for chunk in paths.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        handles.push(thread::spawn(move || -> anyhow::Result<Vec<Track>> {
            chunk
                .into_iter()
                .map(|path| {
                    let metadata = probe_track_metadata(&path)?;
                    Ok(Track {
                        title: track_title(&path),
                        path,
                        duration: metadata.duration,
                        sample_rate: metadata.sample_rate,
                        channels: metadata.channels,
                    })
                })
                .collect()
        }));
    }

    let mut tracks = Vec::with_capacity(paths.len());
    for handle in handles {
        let chunk_tracks = handle
            .join()
            .map_err(|_| anyhow!("track duration worker panicked"))??;
        tracks.extend(chunk_tracks);
    }

    Ok(tracks)
}

struct TrackMetadata {
    duration: Duration,
    sample_rate: u32,
    channels: u16,
}

fn probe_track_metadata(path: &Path) -> anyhow::Result<TrackMetadata> {
    let file = File::open(path)
        .with_context(|| format!("failed to open audio file {}", path.display()))?;
    let decoder = Decoder::new(BufReader::new(file))
        .with_context(|| format!("failed to decode {}", path.display()))?;
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels();
    let duration = decoder
        .total_duration()
        .ok_or_else(|| anyhow!("failed to determine duration for {}", path.display()))?;

    Ok(TrackMetadata {
        duration,
        sample_rate,
        channels,
    })
}

fn track_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Unknown Track")
        .replace('_', " ")
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

#[cfg(test)]
mod tests {
    use super::resample_interleaved_samples;

    #[test]
    fn resamples_44k1_stereo_to_192k() {
        assert_resample_shape(44_100, 192_000, 2, 44_100);
    }

    #[test]
    fn resamples_48k_stereo_to_192k() {
        assert_resample_shape(48_000, 192_000, 2, 48_000);
    }

    #[test]
    fn keeps_samples_when_rates_match() {
        let samples = vec![0.25, -0.25, 0.5, -0.5];
        let output = resample_interleaved_samples(samples.clone(), 2, 48_000, 48_000).unwrap();
        assert_eq!(output, samples);
    }

    fn assert_resample_shape(
        input_sample_rate: u32,
        output_sample_rate: u32,
        channels: u16,
        input_frames: usize,
    ) {
        let samples = sine_wave(input_sample_rate, input_frames, channels);
        let output =
            resample_interleaved_samples(samples, channels, input_sample_rate, output_sample_rate)
                .unwrap();

        let output_frames = output.len() / channels as usize;
        let expected_frames = ((input_frames as u128 * output_sample_rate as u128)
            / input_sample_rate as u128) as usize;
        let tolerance = 8;

        assert!(!output.is_empty());
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(output_frames.abs_diff(expected_frames) <= tolerance);
    }

    fn sine_wave(sample_rate: u32, frames: usize, channels: u16) -> Vec<f32> {
        let channel_count = channels as usize;
        let mut output = Vec::with_capacity(frames * channel_count);

        for frame in 0..frames {
            let sample =
                ((frame as f32 * 440.0 * std::f32::consts::TAU) / sample_rate as f32).sin() * 0.25;
            for _ in 0..channel_count {
                output.push(sample);
            }
        }

        output
    }
}
