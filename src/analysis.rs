//! Native Rust audio analysis — DR, ReplayGain, BPM/key and FLAC integrity.
//!
//! No external binaries and no audio device are involved: every function opens the
//! file with symphonia, decodes it in-process and derives its measurement from the
//! decoded PCM. Every failure is reported as [`MloError::Decode`] with a concrete
//! reason (unsupported codec, no audio track, truncated stream, ...) — the module
//! never panics on malformed input.

use std::io::Read;
use std::path::{Path, PathBuf};

use byteorder::{BigEndian, ByteOrder};
use ebur128::{EbuR128, Mode};
use md5::{Digest, Md5};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use tracing::{debug, warn};

use crate::error::{MloError, Result};

/// ReplayGain 2.0 reference loudness (LUFS) used to derive a track/album gain.
const RG_TARGET_LUFS: f64 = -18.0;

/// Result of a dynamic-range measurement ([§8.1]).
#[derive(Debug, Clone, PartialEq)]
pub struct DrResult {
    pub dr: Option<u8>,
    pub track_gain_db: Option<f32>,
    pub track_peak: Option<f32>,
}

/// Album ReplayGain ([§8.2]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlbumRg {
    pub album_gain_db: f32,
    pub album_peak: f32,
}

/// Detected tempo and musical key ([§8.4]).
#[derive(Debug, Clone, PartialEq)]
pub struct BpmKey {
    pub bpm: f64,
    pub key: String,
}

fn dec_err(path: &Path, reason: impl Into<String>) -> MloError {
    MloError::Decode { path: path.to_path_buf(), reason: reason.into() }
}

/// Planar decoded audio: one `Vec<f32>` per channel, all of equal length.
struct Decoded {
    sample_rate: u32,
    planes: Vec<Vec<f32>>,
}

/// Open `path`, probe the container and hand every decoded audio buffer to `on_audio`.
///
/// Corrupt packets that a decoder can skip are logged and ignored; anything that
/// makes further decoding impossible is surfaced as a named decode error.
fn decode_stream<F>(path: &Path, mut on_audio: F) -> Result<()>
where
    F: FnMut(&GenericAudioBufferRef<'_>) -> Result<()>,
{
    let file = std::fs::File::open(path).map_err(|e| MloError::io(path, e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| dec_err(path, format!("unsupported or unrecognised container: {e}")))?;

    let track = format
        .default_track(TrackType::Audio)
        .or_else(|| format.first_track(TrackType::Audio))
        .ok_or_else(|| dec_err(path, "no audio track"))?;
    let track_id = track.id;

    let params = match &track.codec_params {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err(dec_err(path, "audio track has no codec parameters")),
    };

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| dec_err(path, format!("unsupported audio codec: {e}")))?;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => {
                return Err(dec_err(path, "decoder reset required (stream parameters changed)"));
            }
            Err(e) => return Err(dec_err(path, format!("demux error: {e}"))),
        };

        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(buf) => {
                if buf.frames() == 0 {
                    continue;
                }
                on_audio(&buf)?;
            }
            Err(SymphoniaError::DecodeError(msg)) => {
                warn!(path = %path.display(), reason = msg, "skipping corrupt packet");
            }
            Err(e) => {
                if let SymphoniaError::IoError(io) = &e {
                    if io.kind() == std::io::ErrorKind::UnexpectedEof {
                        debug!(path = %path.display(), "stream truncated at packet boundary");
                        break;
                    }
                }
                return Err(dec_err(path, format!("decode error: {e}")));
            }
        }
    }

    Ok(())
}

/// Decode a whole file to planar `f32` channel buffers.
fn decode_planar(path: &Path) -> Result<Decoded> {
    let mut planes: Vec<Vec<f32>> = Vec::new();
    let mut sample_rate: u32 = 0;

    decode_stream(path, |buf| {
        sample_rate = buf.spec().rate();
        let mut chunk: Vec<Vec<f32>> = Vec::new();
        buf.copy_to_vecs_planar::<f32>(&mut chunk);

        if planes.is_empty() {
            planes = chunk;
        } else if planes.len() == chunk.len() {
            for (dst, src) in planes.iter_mut().zip(chunk.into_iter()) {
                dst.extend_from_slice(&src);
            }
        } else {
            return Err(dec_err(path, "channel layout changed mid-stream"));
        }
        Ok(())
    })?;

    if planes.is_empty() || planes.iter().all(|p| p.is_empty()) {
        return Err(dec_err(path, "no audio samples decoded"));
    }
    if sample_rate == 0 {
        return Err(dec_err(path, "unknown sample rate"));
    }

    Ok(Decoded { sample_rate, planes })
}

/// Mix planar channels down to a single mono channel of the shortest channel length.
fn mixdown_mono(planes: &[Vec<f32>]) -> Vec<f32> {
    if planes.is_empty() {
        return Vec::new();
    }
    let frames = planes.iter().map(|p| p.len()).min().unwrap_or(0);
    let inv = 1.0f32 / planes.len() as f32;
    let mut mono = Vec::with_capacity(frames);
    for i in 0..frames {
        let mut acc = 0.0f32;
        for p in planes {
            acc += p[i];
        }
        mono.push(acc * inv);
    }
    mono
}

/// Linear-interpolation resampler (adequate for onset/chroma analysis).
fn resample_linear(input: &[f32], src_hz: u32, dst_hz: u32) -> Vec<f32> {
    if input.is_empty() || src_hz == 0 || dst_hz == 0 || src_hz == dst_hz {
        return input.to_vec();
    }
    let step = src_hz as f64 / dst_hz as f64;
    let out_len = ((input.len() as f64) / step).round().max(1.0) as usize;
    let last = *input.last().unwrap_or(&0.0);

    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * step;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(last);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Decode `path` and return its mono mixdown resampled to `target_hz`.
///
/// Use for resample-dependent analysis (tempo, chroma). The returned rate is
/// always `target_hz`.
pub fn decode_to_mono_f32(path: &Path, target_hz: u32) -> Result<(Vec<f32>, u32)> {
    if target_hz == 0 {
        return Err(MloError::Invalid("target sample rate must be greater than zero".into()));
    }
    let decoded = decode_planar(path)?;
    let mono = mixdown_mono(&decoded.planes);
    if mono.is_empty() {
        return Err(dec_err(path, "no audio samples decoded"));
    }
    let out = if decoded.sample_rate == target_hz {
        mono
    } else {
        resample_linear(&mono, decoded.sample_rate, target_hz)
    };
    Ok((out, target_hz))
}

// ---------------------------------------------------------------------------
// Dynamic range (TT-DR compatible, §8.1)
// ---------------------------------------------------------------------------

fn block_metrics(slice: &[f32], peaks: &mut Vec<f64>, rms_scaled: &mut Vec<f64>) {
    let mut peak = 0.0f64;
    let mut sum_sq = 0.0f64;
    for &s in slice {
        let v = s as f64;
        let a = v.abs();
        if a > peak {
            peak = a;
        }
        sum_sq += v * v;
    }
    peaks.push(peak);
    // TT DR scales RMS by sqrt(2): a full-scale sine yields RMS == peak.
    let rms = (sum_sq / slice.len().max(1) as f64).sqrt() * std::f64::consts::SQRT_2;
    rms_scaled.push(rms);
}

fn channel_dr(plane: &[f32], block_frames: usize) -> f64 {
    let mut peaks: Vec<f64> = Vec::new();
    let mut rms_scaled: Vec<f64> = Vec::new();

    // Full 3-second blocks, as the TT-DR meter does. A track shorter than one
    // block is measured as a single block so the value is still well defined.
    if block_frames > 0 && plane.len() >= block_frames {
        let mut start = 0usize;
        while start + block_frames <= plane.len() {
            block_metrics(&plane[start..start + block_frames], &mut peaks, &mut rms_scaled);
            start += block_frames;
        }
    }
    if peaks.is_empty() {
        block_metrics(plane, &mut peaks, &mut rms_scaled);
    }

    peaks.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    rms_scaled.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Second-highest block peak guards against a single clipping glitch.
    let peak = if peaks.len() >= 2 { peaks[peaks.len() - 2] } else { peaks[peaks.len() - 1] };

    // RMS of the loudest 20% of blocks ("top 20 average RMS measurements").
    let count = rms_scaled.len();
    let n_top = (((count as f64) * 0.2).floor() as usize).max(1).min(count);
    let top = &rms_scaled[count - n_top..];
    let rms = (top.iter().map(|v| v * v).sum::<f64>() / n_top as f64).sqrt();

    if peak <= 0.0 || rms <= 0.0 {
        return 0.0;
    }
    (20.0 * (peak / rms).log10()).max(0.0)
}

/// Compute the TT-DR dynamic-range value for a track, clamped to `0..=40`.
pub fn compute_dr(path: &Path) -> Result<u8> {
    let decoded = decode_planar(path)?;
    let block_frames = decoded.sample_rate as usize * 3;

    let mut channel_drs = Vec::with_capacity(decoded.planes.len());
    for plane in &decoded.planes {
        if plane.is_empty() {
            continue;
        }
        channel_drs.push(channel_dr(plane, block_frames));
    }
    if channel_drs.is_empty() {
        return Err(dec_err(path, "no audio samples for dynamic-range measurement"));
    }

    let mean = channel_drs.iter().sum::<f64>() / channel_drs.len() as f64;
    let clamped = mean.round().clamp(0.0, 40.0);
    debug!(path = %path.display(), dr = clamped, "computed dynamic range");
    Ok(clamped as u8)
}

// ---------------------------------------------------------------------------
// ReplayGain (EBU R128, §8.2)
// ---------------------------------------------------------------------------

/// Feed planar channels to an R128 analyser as interleaved frames.
fn feed_planes(eb: &mut EbuR128, planes: &[Vec<f32>], source: &Path) -> Result<()> {
    let channels = planes.len();
    if channels == 0 {
        return Ok(());
    }
    let frames = planes.iter().map(|p| p.len()).min().unwrap_or(0);
    const CHUNK_FRAMES: usize = 16_384;

    let mut interleaved = Vec::with_capacity(CHUNK_FRAMES * channels);
    let mut start = 0usize;
    while start < frames {
        let end = (start + CHUNK_FRAMES).min(frames);
        interleaved.clear();
        for i in start..end {
            for plane in planes {
                interleaved.push(plane[i]);
            }
        }
        eb.add_frames_f32(&interleaved)
            .map_err(|e| dec_err(source, format!("R128 frame ingestion failed: {e}")))?;
        start = end;
    }
    Ok(())
}

/// Maximum true peak (linear) across all channels of an analyser.
fn max_true_peak(eb: &EbuR128, channels: u32, source: &Path) -> Result<f64> {
    let mut peak = 0.0f64;
    for ch in 0..channels {
        let p = eb
            .true_peak(ch)
            .map_err(|e| dec_err(source, format!("R128 true-peak query failed: {e}")))?;
        if p > peak {
            peak = p;
        }
    }
    Ok(peak)
}

/// Compute per-track ReplayGain: `(gain_db, true_peak)`.
///
/// `gain_db` targets -18 LUFS; `true_peak` is the linear (non-dB) oversampled peak.
pub fn compute_track_replaygain(path: &Path) -> Result<(f32, f32)> {
    let decoded = decode_planar(path)?;
    let channels = decoded.planes.len() as u32;

    let mut eb = EbuR128::new(channels, decoded.sample_rate, Mode::I | Mode::TRUE_PEAK)
        .map_err(|e| dec_err(path, format!("R128 analyser init failed: {e}")))?;

    feed_planes(&mut eb, &decoded.planes, path)?;

    let integrated = eb
        .loudness_global()
        .map_err(|e| dec_err(path, format!("R128 loudness measurement failed: {e}")))?;

    let gain = (RG_TARGET_LUFS - integrated) as f32;
    let peak = max_true_peak(&eb, channels, path)?;

    debug!(path = %path.display(), integrated_lufs = integrated, gain_db = gain, "computed track replaygain");
    Ok((gain, peak as f32))
}

/// Reshape a decoded track to a target channel count / rate for album concatenation.
fn adapt_planes(planes: &[Vec<f32>], src_hz: u32, dst_hz: u32, target_channels: usize) -> Vec<Vec<f32>> {
    let resampled: Vec<Vec<f32>> = planes
        .iter()
        .map(|p| if src_hz == dst_hz { p.clone() } else { resample_linear(p, src_hz, dst_hz) })
        .collect();

    match (resampled.len(), target_channels) {
        (0, _) => Vec::new(),
        (n, m) if n == m => resampled,
        (1, m) => vec![resampled[0].clone(); m],
        (n, 1) => {
            let frames = resampled.iter().map(|p| p.len()).min().unwrap_or(0);
            let mut mono = Vec::with_capacity(frames);
            for i in 0..frames {
                let mut acc = 0.0f32;
                for p in &resampled {
                    acc += p[i];
                }
                mono.push(acc / n as f32);
            }
            vec![mono]
        }
        (_, m) => {
            let last = resampled.last().cloned().unwrap_or_default();
            (0..m).map(|i| resampled.get(i).cloned().unwrap_or_else(|| last.clone())).collect()
        }
    }
}

/// Compute album ReplayGain over the concatenated programme of `paths`.
pub fn compute_album_replaygain(paths: &[PathBuf]) -> Result<AlbumRg> {
    if paths.is_empty() {
        return Err(MloError::Invalid("album replaygain needs at least one track".into()));
    }

    let first = decode_planar(&paths[0])?;
    let channels = first.planes.len() as u32;
    let rate = first.sample_rate;

    let mut eb = EbuR128::new(channels, rate, Mode::I | Mode::TRUE_PEAK)
        .map_err(|e| dec_err(&paths[0], format!("R128 analyser init failed: {e}")))?;
    feed_planes(&mut eb, &first.planes, &paths[0])?;
    drop(first);

    for path in &paths[1..] {
        let decoded = decode_planar(path)?;
        if decoded.sample_rate == rate && decoded.planes.len() as u32 == channels {
            feed_planes(&mut eb, &decoded.planes, path)?;
        } else {
            let adapted =
                adapt_planes(&decoded.planes, decoded.sample_rate, rate, channels as usize);
            feed_planes(&mut eb, &adapted, path)?;
        }
    }

    let integrated = eb
        .loudness_global()
        .map_err(|e| dec_err(&paths[0], format!("R128 album loudness measurement failed: {e}")))?;
    let peak = max_true_peak(&eb, channels, &paths[0])?;

    debug!(tracks = paths.len(), integrated_lufs = integrated, "computed album replaygain");
    Ok(AlbumRg {
        album_gain_db: (RG_TARGET_LUFS - integrated) as f32,
        album_peak: peak as f32,
    })
}

// ---------------------------------------------------------------------------
// Tempo + key (§8.4)
// ---------------------------------------------------------------------------

/// Detect tempo from an onset-strength envelope by autocorrelation.
fn detect_bpm(mono: &[f32], rate: u32) -> f64 {
    const FRAME: usize = 1024;
    const HOP: usize = 441; // 100 frames/second at 44.1 kHz

    if mono.len() < FRAME || rate == 0 {
        return 120.0;
    }
    let fps = rate as f64 / HOP as f64;

    // Half-wave rectified energy flux gives a positive onset at each attack.
    let mut envelope: Vec<f64> = Vec::with_capacity(mono.len() / HOP + 1);
    let mut prev = 0.0f64;
    let mut i = 0usize;
    while i + FRAME <= mono.len() {
        let mut sum = 0.0f64;
        for &s in &mono[i..i + FRAME] {
            let v = s as f64;
            sum += v * v;
        }
        envelope.push((sum - prev).max(0.0));
        prev = sum;
        i += HOP;
    }
    if envelope.len() < 4 {
        return 120.0;
    }

    let lag_min = ((60.0 * fps / 200.0).round() as usize).max(1); // 200 BPM
    let lag_max = ((60.0 * fps / 60.0).round() as usize).min(envelope.len() - 1); // 60 BPM
    if lag_max <= lag_min {
        return 120.0;
    }

    let mut ac = vec![0.0f64; lag_max + 1];
    let mut best_lag = 0usize;
    let mut best_val = 0.0f64;
    for lag in lag_min..=lag_max {
        let mut sum = 0.0f64;
        let mut count = 0usize;
        for t in 0..envelope.len() - lag {
            sum += envelope[t] * envelope[t + lag];
            count += 1;
        }
        if count == 0 {
            continue;
        }
        let value = sum / count as f64;
        ac[lag] = value;
        if value > best_val {
            best_val = value;
            best_lag = lag;
        }
    }
    if best_lag == 0 || best_val <= 0.0 || !best_val.is_finite() {
        return 120.0;
    }

    // If a subdivision of the winning period is nearly as strong, prefer the
    // higher tempo (avoids the common half-time/octave error).
    let mut lag = best_lag;
    while lag / 2 >= lag_min && ac[lag / 2] >= 0.92 * ac[lag] {
        lag /= 2;
    }

    // Parabolic interpolation for sub-frame precision.
    let refined = if lag > lag_min && lag < lag_max {
        let y0 = ac[lag - 1];
        let y1 = ac[lag];
        let y2 = ac[lag + 1];
        let denom = y0 - 2.0 * y1 + y2;
        if denom.abs() > 1e-12 {
            lag as f64 + (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5)
        } else {
            lag as f64
        }
    } else {
        lag as f64
    };

    let bpm = 60.0 * fps / refined;
    ((bpm.clamp(40.0, 240.0)) * 10.0).round() / 10.0
}

const KRUMHANSL_MAJOR: [f64; 12] =
    [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const KRUMHANSL_MINOR: [f64; 12] =
    [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const KEY_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let ma = a.iter().sum::<f64>() / 12.0;
    let mb = b.iter().sum::<f64>() / 12.0;
    let (mut num, mut da, mut db) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..12 {
        let x = a[i] - ma;
        let y = b[i] - mb;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    if da <= 0.0 || db <= 0.0 {
        0.0
    } else {
        num / (da.sqrt() * db.sqrt())
    }
}

fn best_key(chroma: &[f64; 12]) -> String {
    let mut best = (f64::MIN, 0usize, false);
    for root in 0..12 {
        let mut major = [0.0f64; 12];
        let mut minor = [0.0f64; 12];
        for pc in 0..12 {
            let idx = (pc + 12 - root) % 12;
            major[pc] = KRUMHANSL_MAJOR[idx];
            minor[pc] = KRUMHANSL_MINOR[idx];
        }
        let score_major = pearson(chroma, &major);
        if score_major > best.0 {
            best = (score_major, root, false);
        }
        let score_minor = pearson(chroma, &minor);
        if score_minor > best.0 {
            best = (score_minor, root, true);
        }
    }
    format!("{}{}", KEY_NAMES[best.1], if best.2 { "m" } else { "" })
}

/// Build a 12-bin pitch-class profile from an FFT chromagram.
fn detect_key(mono: &[f32], rate: u32) -> String {
    const FRAME: usize = 4096;
    const HOP: usize = 2048;
    const MIN_HZ: f64 = 55.0;
    const MAX_HZ: f64 = 2000.0;

    if mono.is_empty() || rate == 0 {
        return "C".to_string();
    }

    let mut chroma = [0.0f64; 12];
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME);

    let mut window = vec![0.0f32; FRAME];
    for (i, w) in window.iter_mut().enumerate() {
        let t = i as f32 / (FRAME as f32 - 1.0);
        *w = 0.5 - 0.5 * (std::f32::consts::TAU * t).cos();
    }

    let mut buf = vec![Complex::<f32>::new(0.0, 0.0); FRAME];
    let mut start = 0usize;
    loop {
        let end = start + FRAME;
        let segment: &[f32] = if end <= mono.len() {
            &mono[start..end]
        } else if start < mono.len() {
            &mono[start..]
        } else {
            break;
        };

        for (k, slot) in buf.iter_mut().enumerate() {
            let s = segment.get(k).copied().unwrap_or(0.0);
            *slot = Complex::new(s * window[k], 0.0);
        }
        fft.process(&mut buf);

        for k in 1..FRAME / 2 {
            let re = buf[k].re as f64;
            let im = buf[k].im as f64;
            let mag = (re * re + im * im).sqrt();
            if mag <= 0.0 {
                continue;
            }
            let freq = k as f64 * rate as f64 / FRAME as f64;
            if !(MIN_HZ..=MAX_HZ).contains(&freq) {
                continue;
            }
            let midi = 69.0 + 12.0 * (freq / 440.0).log2();
            let pc = midi.round().rem_euclid(12.0) as usize;
            chroma[pc] += mag;
        }

        if end >= mono.len() {
            break;
        }
        start += HOP;
    }

    best_key(&chroma)
}

/// Detect BPM and key for `path` using a mono 44.1 kHz mixdown.
pub fn detect_bpm_key(path: &Path) -> Result<BpmKey> {
    let (mono, rate) = decode_to_mono_f32(path, 44_100)?;
    if mono.is_empty() {
        return Err(dec_err(path, "no audio samples for tempo/key detection"));
    }
    let bpm = detect_bpm(&mono, rate);
    let key = detect_key(&mono, rate);
    debug!(path = %path.display(), bpm, key = %key, "detected tempo and key");
    Ok(BpmKey { bpm, key })
}

// ---------------------------------------------------------------------------
// FLAC STREAMINFO MD5 (§8.5)
// ---------------------------------------------------------------------------

/// Verify the FLAC STREAMINFO MD5 against the MD5 of the decoded raw PCM.
///
/// Returns `Ok(None)` when `path` is not a FLAC stream, `Ok(Some(true))` when the
/// stored checksum matches and `Ok(Some(false))` when it does not.
pub fn flac_streaminfo_md5_ok(path: &Path) -> Result<Option<bool>> {
    let mut file = std::fs::File::open(path).map_err(|e| MloError::io(path, e))?;

    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() || &magic != b"fLaC" {
        return Ok(None);
    }

    let mut header = [0u8; 4];
    file.read_exact(&mut header)
        .map_err(|e| dec_err(path, format!("truncated FLAC metadata header: {e}")))?;
    let block_type = header[0] & 0x7f;
    if block_type != 0 {
        return Err(dec_err(path, "first FLAC metadata block is not STREAMINFO"));
    }
    let length =
        ((header[1] as usize) << 16) | ((header[2] as usize) << 8) | header[3] as usize;
    if length < 34 {
        return Err(dec_err(path, "invalid FLAC STREAMINFO length"));
    }

    let mut body = vec![0u8; length];
    file.read_exact(&mut body)
        .map_err(|e| dec_err(path, format!("truncated FLAC STREAMINFO: {e}")))?;

    let expected = &body[18..34];
    let packed = BigEndian::read_u64(&body[10..18]);
    let bits_per_sample = ((packed >> 36) & 0x1f) as u32 + 1;

    let raw = decode_flac_raw_pcm(path, bits_per_sample)?;

    let mut hasher = Md5::new();
    hasher.update(&raw);
    let got = hasher.finalize();

    Ok(Some(got.as_slice() == expected))
}

/// Decode a FLAC stream and re-pack the PCM into the byte layout FLAC hashes
/// (signed, little-endian, `ceil(bps/8)` bytes per sample, channel-interleaved).
fn decode_flac_raw_pcm(path: &Path, bits_per_sample: u32) -> Result<Vec<u8>> {
    let bps = bits_per_sample.clamp(1, 32);
    let bytes_per_sample = ((bps + 7) / 8) as usize;

    let mut raw: Vec<u8> = Vec::new();
    decode_stream(path, |buf| {
        let mut interleaved: Vec<i32> = Vec::new();
        buf.copy_to_vec_interleaved::<i32>(&mut interleaved);

        for &sample in &interleaved {
            let value = sample >> (32 - bps);
            let le = value.to_le_bytes();
            raw.extend_from_slice(&le[..bytes_per_sample]);
        }
        Ok(())
    })?;

    Ok(raw)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;
    use std::path::PathBuf;

    fn write_wav(path: &Path, samples: &[i16], rate: u32, channels: u16) {
        let data_len = (samples.len() * 2) as u32;
        let mut buf = Vec::with_capacity(44 + samples.len() * 2);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&rate.to_le_bytes());
        let byte_rate = rate * channels as u32 * 2;
        buf.extend_from_slice(&byte_rate.to_le_bytes());
        buf.extend_from_slice(&(channels * 2).to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, buf).expect("write wav");
    }

    fn sine(rate: u32, hz: f32, seconds: f32, amp: f32) -> Vec<i16> {
        let n = (rate as f32 * seconds) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (amp * (TAU * hz * t).sin() * i16::MAX as f32) as i16
            })
            .collect()
    }

    fn click_track(rate: u32, seconds: f32, bpm: f64) -> Vec<i16> {
        let n = (rate as f32 * seconds) as usize;
        let mut buf = vec![0i16; n];
        let interval = (rate as f64 * 60.0 / bpm).round() as usize;
        let burst = 64usize;
        let mut start = 0usize;
        while start + burst <= n {
            for j in 0..burst {
                let t = j as f32 / rate as f32;
                buf[start + j] = (0.8 * (TAU * 1000.0 * t).sin() * i16::MAX as f32) as i16;
            }
            start += interval;
        }
        buf
    }

    fn tmp_wav(dir: &Path, name: &str, samples: &[i16]) -> PathBuf {
        let p = dir.join(name);
        write_wav(&p, samples, 44_100, 1);
        p
    }

    #[test]
    fn decodes_wav_to_mono() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_wav(dir.path(), "tone.wav", &sine(44_100, 440.0, 1.0, 0.5));
        let (mono, rate) = decode_to_mono_f32(&p, 44_100).unwrap();
        assert_eq!(rate, 44_100);
        assert!(!mono.is_empty());
        assert!(mono.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn resamples_to_target_rate() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_wav(dir.path(), "tone.wav", &sine(44_100, 440.0, 1.0, 0.5));
        let (mono, rate) = decode_to_mono_f32(&p, 22_050).unwrap();
        assert_eq!(rate, 22_050);
        assert!((mono.len() as i64 - 22_050).abs() < 128);
    }

    #[test]
    fn dr_of_sine_is_small() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_wav(dir.path(), "sine.wav", &sine(44_100, 1000.0, 4.0, 0.5));
        let dr = compute_dr(&p).unwrap();
        assert!(dr <= 10, "unexpected DR {dr}");
    }

    #[test]
    fn bpm_key_valid_for_sine() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_wav(dir.path(), "sine.wav", &sine(44_100, 440.0, 3.0, 0.5));
        let bk = detect_bpm_key(&p).unwrap();
        assert!((40.0..=240.0).contains(&bk.bpm), "bpm out of range: {}", bk.bpm);
        assert!(valid_key(&bk.key), "unexpected key: {}", bk.key);
    }

    #[test]
    fn detects_120_bpm_click_track() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_wav(dir.path(), "click.wav", &click_track(44_100, 10.0, 120.0));
        let bk = detect_bpm_key(&p).unwrap();
        assert!((bk.bpm - 120.0).abs() <= 3.0, "expected ~120 BPM, got {}", bk.bpm);
    }

    #[test]
    fn track_replaygain_is_finite() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_wav(dir.path(), "sine.wav", &sine(44_100, 1000.0, 4.0, 0.5));
        let (gain, peak) = compute_track_replaygain(&p).unwrap();
        assert!(gain.is_finite());
        assert!(peak.is_finite());
        assert!(peak > 0.0);
    }

    #[test]
    fn album_replaygain_is_finite() {
        let dir = tempfile::tempdir().unwrap();
        let a = tmp_wav(dir.path(), "a.wav", &sine(44_100, 440.0, 2.0, 0.5));
        let b = tmp_wav(dir.path(), "b.wav", &sine(44_100, 880.0, 2.0, 0.4));
        let rg = compute_album_replaygain(&[a, b]).unwrap();
        assert!(rg.album_gain_db.is_finite());
        assert!(rg.album_peak.is_finite());
    }

    #[test]
    fn flac_md5_none_for_non_flac() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_wav(dir.path(), "tone.wav", &sine(44_100, 440.0, 0.5, 0.5));
        assert_eq!(flac_streaminfo_md5_ok(&p).unwrap(), None);
    }

    fn valid_key(key: &str) -> bool {
        const MAJOR: [&str; 12] =
            ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        if let Some(root) = key.strip_suffix('m') {
            MAJOR.contains(&root)
        } else {
            MAJOR.contains(&key)
        }
    }
}