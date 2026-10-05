---
layout: default
title: mlo — native Rust music library manager
---

# mlo

`mlo` owns a music library end to end — **acquire, identify, tag, organize,
analyse, grade, optimize, play** — from one native Rust binary. No Python, no
Node, no server, no browser: point it at a folder and run. It opens a **TUI**
by default when stdout is a terminal (`mlo`) and a scriptable **CLI** when it
is not (`mlo <command>`, or `--cli`), with the same engine, config and results
either way. Nothing is ever deleted — every removal moves to
`<music>/.mlo/trash/…` with a manifest that can restore it exactly — and
offline is a normal state: browsing, tag edits, layout, grading and local
playback all work, while every network feature reports *unavailable* with a
reason.

## Features

- **One binary, two front ends.** A full-screen TUI when you are at a terminal,
  the same engine as a CLI when you are not. Scan, grade, layout, tags, scripts,
  import, trash and config are all reachable both ways.
- **Library & layout.** A single walk builds the SQLite index with per-file
  mtime+size reuse; the layout audit reports 14 finding kinds, and fixes are
  re-derived and Trash-backed. Split-artist merge and one stored library-wide
  report.
- **Full-fidelity tagging.** FLAC, Ogg Vorbis, Opus and MP3 (ID3v2.4) keep
  **every** key, foreign ones included; MP4/WAV/AIFF map the shared field set
  and report other keys as *unplaced*. Writes are atomic, and the writer and
  the grader share one normalization.
- **Binary, counted grading.** One check registry produces PASS/FAIL verdicts;
  `Not applicable` counts on neither side, a check that raises counts as
  failed, and every displayed layout finding is charged on its row.
- **In-process analysis.** TT-DR dynamic range, EBU R128 ReplayGain with true
  peak, BPM and Krumhansl–Schmuckler key detection, and FLAC STREAMINFO md5 —
  all in the binary, no external analysers.
- **23 optimization scripts.** A defined run-all order and a derived import
  chain ending layout → grade; a pass that needs a missing tool or the network
  reports `skipped: <reason>` instead of failing.
- **Import.** Acquire (archives included) → identify (MusicBrainz) → tag →
  place by the naming template → chain → grade.
- **Trash that restores exactly.** One implementation, origin manifests, exact
  restore to the original path.
- **Screens.** Library / Album / Artist / Grade / Layout / Scripts / Import /
  Tools / Player / Log / Settings / Trash, plus a Library-status dashboard; a
  tag view and editor per track; cancellable progress and one status line.
- **Shell integration.** A right-click entry on Windows, macOS and Linux from
  `mlo shell install`.
- **Tests.** 83 unit tests and 12 acceptance tests, headless, no network, no
  audio device.

The external-binary surface is deliberately tiny — `fpcalc` fingerprinting and
a `.rar` extractor where no credible Rust crate exists — and both degrade with
a named reason.

## Compatibility with la-musica

`mlo` implements the contract of its reference implementation, **la-musica**:
the same library on disk (layout, sidecar names, naming template), the same
check registry (`grade_check_*` / `grade_include_*`, all ON, with Strict /
Balanced / Relaxed presets), the same issue codes, the same tag vocabularies and
spacing rules, the same layout-report and trash-manifest formats, and the same
script ids, order and gates. A la-musica `config.json` is read on first run.

- **[SPECIFICATION.md](SPECIFICATION.md)** — every on-disk and wire format.
- [DEPENDENCIES.md](DEPENDENCIES.md) — the crate audit.

## Install

The detailed, per-platform guide (including from-source builds, verification
and uninstall) is in **[INSTALL.md](INSTALL.md)**. The short version:

### Windows

```powershell
irm https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.ps1 | iex
```

or download `mlo-<version>-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/dillydalli3r/mlo/releases/latest) and run
`mlo.exe`. No system libraries are needed — SQLite is statically linked and the
terminal backend is pure Rust.

### macOS

```sh
curl -fsSL https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.sh | sh
```

The installer picks the `x86_64-apple-darwin` or `aarch64-apple-darwin` archive
for your machine. Playback uses CoreAudio, so there are no packages to install.

### Linux

```sh
curl -fsSL https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.sh | sh
```

Playback links ALSA at build time, which the prebuilt binaries already do; the
runtime package is `libasound2` / `pipewire-alsa` on Debian/Ubuntu. For a
container or headless host, the `-min` build has **no** audio or archive
support and no system dependencies:

```sh
curl -fsSL https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.sh | sh -s -- --no-audio
```

### From source (any platform, needs Rust 1.85+)

```sh
cargo install --path .                   # from a checkout
cargo install --git https://github.com/dillydalli3r/mlo   # from git
```

## Right-click menu

Add `mlo` to the file manager's context menu (user-level, no admin rights):

```sh
mlo shell install
mlo shell status
mlo shell uninstall
```

- **Windows**: `HKCU\Software\Classes\{Directory,*}\shell\mlo`, plus a folder
  background entry and an App Paths entry.
- **macOS**: `~/Library/Services/mlo.workflow`, a Finder Quick Action
  registered with Launch Services.
- **Linux**: `~/.local/share/applications/mlo.desktop` and a Nautilus script.

Clicking an entry runs `mlo --open <path>`, which lands the TUI on that album
or artist.

## First run

```sh
mlo doctor      # environment, tools, container support, network health
mlo paths       # where config, state and the index live
mlo             # open the TUI
```

The config file is `mlo.toml` in the platform config directory
(`%APPDATA%\mlo\mlo.toml`, `~/.config/mlo/mlo.toml`,
`~/Library/Application Support/mlo/mlo.toml`). An existing `config.json` from
the older `la musica` app is read and migrated on first run, never rewritten in
place.

Set your library:

```toml
music_folder = "F:/Media/Music"
```

The canonical layout is `<music>/Artists/<Artist>/<Album>/<files>`; the app's
own state lives in `<music>/.mlo/` and is never library content.

## Licence

MIT — see [LICENSE](https://github.com/dillydalli3r/mlo/blob/main/LICENSE).