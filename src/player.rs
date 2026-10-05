//! In-process audio playback.
//!
//! The player is a thin, single-track wrapper around `rodio`: it opens the
//! default output device once, decodes a local file on demand and reports the
//! current position and volume. It performs no network access and no metadata
//! fetching — the TUI owns the queue and metadata; this module only plays one
//! file and answers questions about it.
//!
//! rodio is an optional dependency behind the default-on `audio` feature. When
//! the feature is disabled the same public API is provided by a stub whose
//! constructors fail with a named `MloError::Tool { tool: "audio", .. }` so the
//! rest of the program keeps compiling.

use std::path::Path;

use crate::error::{MloError, Result};

// ---------------------------------------------------------------------------
// Feature: audio (rodio-backed implementation)
// ---------------------------------------------------------------------------

#[cfg(feature = "audio")]
mod audio_impl {
    use std::fs::File;
    use std::io::BufReader;

    use tracing::instrument;

    use super::{MloError, Path, Result};

    /// Clamp a requested volume into `0.0..=1.0`.
    ///
    /// Non-finite values (notably `NaN`) fall back to `0.0`; infinities
    /// saturate to their nearest bound.
    pub(crate) fn clamp_volume(v: f32) -> f32 {
        if v.is_nan() {
            0.0
        } else {
            v.clamp(0.0, 1.0)
        }
    }

    /// Open and probe a local audio file into a rodio decoder.
    ///
    /// Split out from [`Player::play_file`] so the decode path can be exercised
    /// headlessly, without touching an output device.
    pub(crate) fn open_decoder(
        path: &Path,
    ) -> Result<rodio::Decoder<BufReader<File>>> {
        let file = File::open(path).map_err(|e| MloError::io(path, e))?;
        rodio::Decoder::try_from(file).map_err(|e| MloError::Decode {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })
    }

    /// Single-track in-process player.
    pub struct Player {
        /// Kept alive for the player's lifetime: dropping it tears down the
        /// output stream and silences everything routed through it.
        _device: rodio::MixerDeviceSink,
        sink: rodio::Player,
        volume: f32,
    }

    impl Player {
        /// Open the default output device and build an idle player.
        ///
        /// Returns a named `Tool { tool: "audio", reason: .. }` error when no
        /// usable output device exists (e.g. headless CI).
        pub fn new(volume: f32) -> Result<Self> {
            let mut device = rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| {
                MloError::tool("audio", format!("cannot open default audio output: {e}"))
            })?;
            // The TUI owns lifecycle messaging; don't print on drop.
            device.log_on_drop(false);

            let sink = rodio::Player::connect_new(device.mixer());
            let volume = clamp_volume(volume);
            sink.set_volume(volume);

            Ok(Self { _device: device, sink, volume })
        }

        /// Decode `path` and start playing it, replacing anything already
        /// queued (one file at a time).
        ///
        /// A decode failure is reported as `MloError::Decode { path, reason }`
        /// and leaves the player untouched, so callers can apply skip
        /// semantics (advance to the next track) without a panic.
        #[instrument(skip(self))]
        pub fn play_file(&mut self, path: &Path) -> Result<()> {
            let decoder = open_decoder(path)?;

            self.sink.clear();
            self.sink.append(decoder);
            self.sink.set_volume(self.volume);
            self.sink.play();

            tracing::debug!(path = %path.display(), "playing file");
            Ok(())
        }

        /// Pause playback. No effect if already paused.
        pub fn pause(&mut self) {
            self.sink.pause();
        }

        /// Resume playback. No effect if not paused.
        pub fn resume(&mut self) {
            self.sink.play();
        }

        /// Toggle between paused and playing.
        pub fn toggle(&mut self) {
            if self.sink.is_paused() {
                self.sink.play();
            } else {
                self.sink.pause();
            }
        }

        /// Stop playback and empty the queue.
        pub fn stop(&mut self) {
            self.sink.stop();
        }

        /// Set the volume, clamped to `0.0..=1.0`.
        pub fn set_volume(&mut self, v: f32) {
            self.volume = clamp_volume(v);
            self.sink.set_volume(self.volume);
        }

        /// Current volume in `0.0..=1.0`.
        pub fn volume(&self) -> f32 {
            self.volume
        }

        /// Position of the currently playing source, in milliseconds.
        pub fn position_ms(&self) -> u64 {
            self.sink.get_pos().as_millis() as u64
        }

        /// Whether a source is queued and not paused.
        ///
        /// Note: after [`stop`](Self::stop) the queue drains on the audio
        /// thread, so this may report `true` for up to one periodic tick.
        pub fn is_playing(&self) -> bool {
            !self.sink.empty() && !self.sink.is_paused()
        }

        /// Whether nothing is queued.
        pub fn is_empty(&self) -> bool {
            self.sink.empty()
        }
    }
}

#[cfg(feature = "audio")]
pub use audio_impl::Player;

// ---------------------------------------------------------------------------
// Feature: no audio (stub)
// ---------------------------------------------------------------------------

#[cfg(not(feature = "audio"))]
mod stub {
    use super::{MloError, Path, Result};

    fn audio_unavailable() -> MloError {
        MloError::tool("audio", "built without the `audio` feature")
    }

    /// Stub player used when the crate is built without the `audio` feature.
    ///
    /// It cannot be constructed: [`Player::new`] always fails with a named
    /// `Tool { tool: "audio", .. }` error.
    pub struct Player {
        _private: (),
    }

    impl Player {
        /// Always fails: playback requires the `audio` feature.
        pub fn new(volume: f32) -> Result<Self> {
            let _ = volume;
            Err(audio_unavailable())
        }

        /// Always fails: playback requires the `audio` feature.
        pub fn play_file(&mut self, path: &Path) -> Result<()> {
            let _ = path;
            Err(audio_unavailable())
        }

        /// No-op: no player can exist.
        pub fn pause(&mut self) {}

        /// No-op: no player can exist.
        pub fn resume(&mut self) {}

        /// No-op: no player can exist.
        pub fn toggle(&mut self) {}

        /// No-op: no player can exist.
        pub fn stop(&mut self) {}

        /// No-op: no player can exist.
        pub fn set_volume(&mut self, v: f32) {
            let _ = v;
        }

        /// Always `0.0`: no player can exist.
        pub fn volume(&self) -> f32 {
            0.0
        }

        /// Always `0`: no player can exist.
        pub fn position_ms(&self) -> u64 {
            0
        }

        /// Always `false`: no player can exist.
        pub fn is_playing(&self) -> bool {
            false
        }

        /// Always `true`: no player can exist.
        pub fn is_empty(&self) -> bool {
            true
        }
    }
}

#[cfg(not(feature = "audio"))]
pub use stub::Player;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "audio"))]
mod tests {
    use std::path::PathBuf;

    use super::audio_impl::{clamp_volume, open_decoder};
    use super::Player;
    use crate::error::MloError;

    /// Unique, non-existent path helper (no `tempfile` dependency needed).
    fn unique_temp_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("mlo-player-{}-{tag}-{nanos}", std::process::id()))
    }

    #[test]
    fn clamp_volume_bounds() {
        assert_eq!(clamp_volume(-1.0), 0.0);
        assert_eq!(clamp_volume(0.0), 0.0);
        assert_eq!(clamp_volume(0.5), 0.5);
        assert_eq!(clamp_volume(1.0), 1.0);
        assert_eq!(clamp_volume(2.0), 1.0);
    }

    #[test]
    fn clamp_volume_handles_non_finite() {
        assert_eq!(clamp_volume(f32::NAN), 0.0);
        assert_eq!(clamp_volume(f32::INFINITY), 1.0);
        assert_eq!(clamp_volume(f32::NEG_INFINITY), 0.0);
    }

    #[test]
    fn missing_file_is_named_io_error() {
        let path = unique_temp_path("missing");
        let _ = std::fs::remove_file(&path);
        let err = match open_decoder(&path) {
            Ok(_) => panic!("expected an error for missing file {path:?}"),
            Err(e) => e,
        };
        assert!(matches!(err, MloError::Io { .. }), "unexpected: {err:?}");
    }

    #[test]
    fn garbage_file_is_named_decode_error() {
        let path = unique_temp_path("garbage");
        std::fs::write(&path, b"this is definitely not an audio stream").unwrap();

        let err = match open_decoder(&path) {
            Ok(_) => panic!("expected a decode error for garbage input"),
            Err(e) => e,
        };
        let _ = std::fs::remove_file(&path);

        match err {
            MloError::Decode { path: p, reason } => {
                assert_eq!(p, path);
                assert!(!reason.is_empty(), "decode reason must be named");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Device-dependent smoke test. On headless CI `Player::new` returns a
    /// `Tool` error; we return early rather than failing the suite.
    #[test]
    fn player_state_without_device_requirement() {
        let mut player = match Player::new(1.5) {
            Ok(p) => p,
            Err(MloError::Tool { .. }) => return,
            Err(e) => panic!("unexpected error creating player: {e:?}"),
        };

        // Volume is clamped on construction.
        assert_eq!(player.volume(), 1.0);
        assert!(player.is_empty());
        assert_eq!(player.position_ms(), 0);
        assert!(!player.is_playing());

        player.set_volume(-1.0);
        assert_eq!(player.volume(), 0.0);
        player.set_volume(f32::NAN);
        assert_eq!(player.volume(), 0.0);

        // Pause/resume/toggle/stop must not panic on an idle player.
        player.pause();
        player.resume();
        player.toggle();
        player.stop();
    }
}

#[cfg(all(test, not(feature = "audio")))]
mod stub_tests {
    use super::Player;
    use crate::error::MloError;

    #[test]
    fn new_reports_missing_feature() {
        let err = match Player::new(0.8) {
            Ok(_) => panic!("stub player must never be constructible"),
            Err(e) => e,
        };
        match err {
            MloError::Tool { tool, reason } => {
                assert_eq!(tool, "audio");
                assert_eq!(reason, "built without the `audio` feature");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn play_file_reports_missing_feature() {
        // `Player` has no constructible value, so exercise the only reachable
        // constructor path; the stub must fail with the same named tool error.
        let err = match Player::new(0.8) {
            Ok(_) => panic!("stub player must never be constructible"),
            Err(e) => e,
        };
        assert!(matches!(err, MloError::Tool { ref tool, .. } if tool == "audio"));
    }
}