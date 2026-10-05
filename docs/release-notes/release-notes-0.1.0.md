# mlo 0.1.0

The first release. A native Rust music library manager — TUI and scriptable CLI
from one binary — that owns a library end to end: **acquire, identify, tag,
organize, analyse, grade, optimize, play**. No Python, no Node, no server, no
browser.

## Highlights

- **Library & layout**: one walk builds the SQLite index (per-file mtime+size
  reuse), a layout audit of 14 finding kinds with re-derived, Trash-backed
  fixes, split-artist merge, and one stored library-wide report.
- **Tagging**: FLAC, Ogg Vorbis, Opus and MP3 with full key fidelity (foreign
  keys preserved); MP4/WAV/AIFF map the shared field set and report other keys
  as unplaced. Atomic writes; the writer and the grader share one
  normalization.
- **Grading**: binary, counted verdicts from a single check registry;
  `Not applicable` counts on neither side; any check that raises counts as
  failed; every displayed layout finding is charged on its row.
- **Analysis**: TT-DR dynamic range, EBU R128 ReplayGain with true peak, BPM
  and Krumhansl–Schmuckler key detection, and FLAC STREAMINFO md5 — all in
  process.
- **Scripts**: the 23 passes with a run-all order and a derived import chain
  ending layout → grade; unavailable passes report the reason.
- **Import**: acquire (archives included) → identify (MusicBrainz) → tag →
  place by the naming template → chain → grade.
- **Trash**: one implementation, manifests, exact restore.
- **TUI**: Library / Album / Artist / Grade / Layout / Scripts / Import /
  Tools / Player / Log / Settings / Trash, plus a Library-status dashboard; a
  tag view and edit per track; cancellable progress; one status line.
- **Shell integration**: a right-click entry on Windows, macOS and Linux via
  `mlo shell install`.
- **Tests**: 67 unit tests and 12 acceptance tests, headless, no network, no
  audio device.

## Install

Prebuilt binaries for Windows x86_64, macOS (Intel and Apple silicon) and
Linux (x86_64 and arm64) are attached to this release, each with a `SHA256SUMS`
checksum file. See [Installing mlo](https://github.com/dillydalli3r/mlo/blob/main/docs/INSTALL.md),
or the short form:

```sh
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.sh | sh
```

```powershell
# Windows
irm https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.ps1 | iex
```

From source (Rust 1.85+):

```sh
cargo install --path .
```

Linux playback needs ALSA at link time for a source build
(`libasound2-dev pkg-config`); add `--no-default-features` for a build with no
system dependencies, which also drops archive extraction.

## Notes

- Default features are `audio` and `archives`; `--no-default-features` removes
  both, so the `-min` Linux archives have no playback and no archive
  extraction.
- `mlo doctor` reports the environment, tools, container support and network
  health before you rely on any of it.
- Nothing is ever deleted: removals move to `<music>/.mlo/trash/` with a
  manifest that restores them exactly.