# Dependency audit

Every crate below was chosen against alternatives for correctness, cross-platform
build simplicity and performance, under the spec's rule "choose equivalents only
with a reason" ([§12.1]). Versions are pinned in `Cargo.toml`.

## Default features

`default = ["audio", "archives"]`. `--no-default-features` yields a build with no
system library requirements at all (no ALSA, no zlib).

## Terminal UI

| crate | why | alternatives rejected |
|---|---|---|
| `ratatui` | the maintained, layout-driven TUI framework; widget model and diffing renderer | `tui-rs` (unmaintained), raw `crossterm` drawing |
| `crossterm` | pure-Rust cross-platform terminal backend (Windows Console, POSIX, macOS); ratatui's native backend | `termion` (Unix-only), `ncurses-rs` (C dep) |
| `unicode-width` | correct column math for wide glyphs; ratatui re-uses it | hand-rolled width tables |

Terminal performance: ratatui renders only changed cells (double-buffered diff),
so redraws are cheap. The event loop polls at 120 ms and does no work on the
render thread ([§12.4]) — jobs run on their own threads and stream progress over
an `mpsc` channel.

## CLI / config / errors

| crate | why | alternatives |
|---|---|---|
| `clap` (derive) | the standard, fast, well-documented argument parser | `argh`, `pico-args` |
| `serde` + `serde_json` + `toml` | the ecosystem baseline; `mlo.toml` and the JSON sidecars/reports | `figment`, custom parsers |
| `anyhow` / `thiserror` | `thiserror` for the typed `MloError` with reason codes; `anyhow` avoided in library code in favour of the typed error | `eyre` |

## Data

| crate | why | alternatives |
|---|---|---|
| `rusqlite` (**bundled**) | statically links SQLite: no system dependency on any OS; prepared statements, transactions | `sqlx` (async runtime + more deps), `sled` (no SQL/joins for the index) |
| `xxhash-rust` (xxh3) | quick-hash for change detection is ~10× faster than MD5 and not security-sensitive | `md5` for the quick hash (kept only for the FLAC STREAMINFO check) |
| `walkdir` | one traversal for the layout audit | `ignore` (gitignore semantics not wanted) |
| `rayon` | data-parallel tag reads across albums; a work-stealing pool sized by items ([§7.3]) | manual threads |
| `regex` | naming/template and sidecar pattern work | hand-rolled scanners |
| `unicode-normalization` | correct NFC handling for filenames and artist matching | — |

The parallel read is bounded by **items** (albums/tracks), not containers, so a
single-album run still uses the whole budget.

## Tagging

| crate | why | alternatives rejected |
|---|---|---|
| **own Vorbis/FLAC/Ogg engine** (`src/tags/{vorbis,flac,oggstream}.rs`) | FLAC/Ogg/Opus preserving **every** key, incl. foreign ones, which the spec's excess-tag check and `DYNAMIC RANGE`/`RATEYOURMUSIC_*` tags require | `lofty` drops unknown keys in 0.25 (`ItemKey` has no `Unknown`), which would silently lose data |
| `id3` | MP3 ID3v2.4 with arbitrary keys via `TXXX` (Picard convention), unknown frames preserved | `lofty` (same unknown-key loss), `mp3-metadata` |
| `ogg` | correct Ogg page/packet re-muxing (CRC, sequence numbers, granule positions) when rewriting comments | hand-rolled paging (error-prone) |
| `lofty` | MP4/WAV/AIFF, where only the shared field set is representable anyway | `mp4ameta` (read-mostly), `taglib` (C++ dependency) |

Writes are atomic on every container ([§1.3]): temp file beside the destination,
flush, fsync, one rename.

## Audio decode / analysis

| crate | why | alternatives |
|---|---|---|
| `symphonia` (flac/mp3/aac/alac/isomp4/ogg/vorbis/wav/pcm) | the pure-Rust decoder that covers every format the spec lists; SIMD-accelerated | `minimp3`/`claxon`/`lewton` individually (more crates, less coverage), FFmpeg (C, external) |
| `ebur128` | ITU-R BS.1770-4 / EBU R128 integrated loudness + true peak, exactly what ReplayGain needs | `rsgain` (external binary) |
| `rustfft` | chromagram for key detection and onset analysis for BPM | `realfft` wrapper, custom FFT |
| `md-5` | FLAC STREAMINFO md5 verification of the decoded PCM | `md5` (slower, older API) |

Analysis runs off the UI thread, per item, cancellable at boundaries.

## Images

| crate | why | alternatives |
|---|---|---|
| `image` (jpeg/png/webp/gif/bmp/tiff, rayon) | decode + encode for every sidecar/cover format, with SIMD | `zune-image` (younger), `libvips` (C) |
| `fast_image_resize` | SIMD (AVX2/NEON) resampling for the artist-image policy, ~5–10× faster than naive | `image`'s own resize |

JXL encode is optional and not built by default; JXL *decode* is out of scope of
the default feature set and reported as unavailability with a reason when needed.

## Network

| crate | why | alternatives rejected |
|---|---|---|
| `ureq` (rustls, json) | **blocking** HTTP: the app is synchronous and does its I/O on job threads, so an async runtime is pure overhead | `reqwest` + `tokio` (spec's suggestion, but pulls a runtime, makes the binary bigger and builds slower, and complicates cancellation in a sync TUI); `isahc` (C deps) |

One client, per-service rate limits (MusicBrainz 1 req/s, identified User-Agent),
and an on-disk SQLite response cache with TTL. Every call degrades to
*unavailable + reason*; a timeout never aborts a batch.

## Archives

`zip`, `sevenz-rust`, `flate2`, `tar`, `bzip2`, `xz2` — the standard Rust
implementations, each behind the `archives` feature. `.rar` requires an external
extractor and reports a named refusal otherwise.

## Playback (default feature, optional dependency)

| crate | why | alternatives |
|---|---|---|
| `rodio` | simplest correct `cpal`-based output with Symphonia decoding built in | `cpal` + `symphonia` wired by hand (more code for no gain), `kira` (game-audio focus) |

`audio` is a default feature but an optional dependency, so
`--no-default-features` removes `cpal` and the ALSA link requirement on Linux.

## Cross-platform / misc

| crate | why |
|---|---|
| `directories` | one API for the config/state dirs on Windows, macOS and Linux |
| `chrono` | timestamps for manifests, reports, trash |
| `tracing` + `tracing-appender` | structured logging with a rotating file appender ([§3.4]) |
| `tempfile` (dev) | isolated test directories |

## Build profile

Release: `opt-level = 3`, thin LTO, one codegen unit, symbols stripped.
Dev: `opt-level = 1` with dependencies at `opt-level = 3`, because audio
decode/analysis is unusable at `-O0`. `panic = "unwind"` is deliberate — a
panicking worker thread must be joinable and reported, never abort the TUI.