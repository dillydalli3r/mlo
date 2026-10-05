# mlo — native Rust music library manager (TUI + CLI)

`mlo` owns a music library end to end — **acquire, identify, tag, organize,
analyse, grade, optimize, play** — with one binary. No Python, no Node, no
server, no browser. Point it at a folder and run.

- **TUI** by default when stdout is a terminal (`mlo`), **CLI** when it is not
  (`mlo <command>`) or with `--cli`.
- Same engine, same config, same results for both.
- Nothing is ever deleted: every removal moves to `<music>/.mlo/trash/…` with an
  origin manifest that can put it back.
- Offline is a normal state: browsing, tag edits, layout, grading and playback of
  local files all work; every network feature reports *unavailable* with a reason.

## Install

### Windows

```powershell
# from source (needs Rust 1.85+)
cargo install --path .
# or build a release binary
cargo build --release      # target\release\mlo.exe
```

No extra system libraries are needed. The SQLite engine is statically linked
(`rusqlite` bundled) and the terminal backend is pure Rust.

### macOS

```sh
cargo install --path .     # audio uses CoreAudio, no extra packages
```

### Linux

```sh
# playback needs a system audio library at link time (Debian/Ubuntu):
sudo apt install libasound2-dev pkg-config
cargo install --path .

# no-audio build (containers, headless CI) — no system deps at all:
cargo install --path . --no-default-features
```

`--no-default-features` also drops archive extraction. The player then reports
`playback unavailable: built without the audio feature` instead of failing.

### Shell / file-manager integration

Add a right-click entry that opens `mlo` on a file or folder (user-level, no
admin):

```sh
mlo shell install      # Windows registry · macOS Finder Service · Linux .desktop + Nautilus
mlo shell status
mlo shell uninstall
```

- **Windows**: `HKCU\Software\Classes\{Directory,*}\shell\mlo` (+ background menu
  and an App Paths entry).
- **macOS**: `~/Library/Services/mlo.workflow` (a Finder Quick Action), registered
  with Launch Services.
- **Linux**: `~/.local/share/applications/mlo.desktop` and a Nautilus script.

Clicking an entry runs `mlo --open <path>`, which lands the TUI on that album or
artist.

## First run

```sh
mlo doctor          # environment, tools, container support, network health
mlo paths           # where config/state/index live
mlo                 # open the TUI
```

The config file is `mlo.toml` in the platform config directory
(`%APPDATA%\mlo\mlo.toml`, `~/.config/mlo/mlo.toml`,
`~/Library/Application Support/mlo/mlo.toml`). An existing `config.json` from the
older `la musica` app is read and migrated on first run, never rewritten in place.

Set your library:

```toml
music_folder = "F:/Media/Music"
```

The canonical layout is `<music>/Artists/<Artist>/<Album>/<files>`; the app's own
state lives in `<music>/.mlo/` and is never library content.

## CLI

```
mlo scan                     # walk, index, layout audit, grade
mlo grade [--album PATH]     # per-album check report (binary PASS/FAIL)
mlo layout [--apply]         # the layout report; --apply performs the fixes
mlo tags PATH [--set K=V]…   # show/edit a file's tags (full key fidelity)
mlo run <id|name> [--scope]  # one of the 23 optimization scripts
mlo import <path|archive>    # acquire → identify → tag → place → chain → grade
mlo trash list|restore       # every removal, restorable to the exact path
mlo tools doctor             # expected vs found, install kind, path
mlo scripts                  # list the passes
mlo config show|get|set      # mlo.toml
mlo sources                  # service health
```

Exit codes are non-zero on error; every refusal carries a stable reason code
(`UNSUPPORTED_CONTAINER`, `NETWORK_UNAVAILABLE`, `TOOL_UNAVAILABLE`, …), never a
bare "error".

## TUI

| key | action |
|---|---|
| `Tab` / `←→` | switch screen or pane |
| `↑↓` | move in the focused list |
| `Enter` | open album / run script / edit tag / restore trash |
| `s` | scan and index |
| `g` | grade |
| `L` | layout findings (space toggle selection, `r` dry-run, `A` apply with confirm) |
| `S` | scripts menu (runs on the current scope) |
| `T` | tools doctor |
| `P` | player (space play/pause, `a` queue album, `s` stop, `+`/`-` volume) |
| `e` `a` `d` | edit / add / delete a tag on the selected track |
| `c` | grading checks (space toggle, `s` save) |
| `r` | trash (Enter restores exactly) |
| `?` | help · `q` quit (asks once) · `Esc` cancels the running job |

Screens: Library, Album, Artist, Grade (per-row check detail), Layout, Scripts,
Import, Tools, Player, Log, Settings, Trash, and a Library-status dashboard.

Tags are viewable and editable for every track (and the panel colours keys by
family: identity, release, audio, lyrics, provenance; foreign keys show red).
Writes are atomic and normalized with the same functions the grader uses.

## What is native, and what degrades with a reason

Native Rust: decoding (Symphonia), tag read/write (FLAC/Ogg/Opus/MP3 own engine +
`lofty` for MP4/WAV/AIFF), DR and ReplayGain (ebur128), BPM/key (rustfft),
fingerprint handling, images, archives, MusicBrainz/AcoustID/lyrics clients.

External binaries are allowed only where no credible crate exists — `fpcalc`
fingerprinting and a `.rar` extractor — and both degrade with a named reason.
Scripts that need a tool or the network report `skipped: <reason>`; a script that
raises is reported and never fatal.

Container tag fidelity:

| container | fidelity |
|---|---|
| FLAC, Ogg Vorbis, Opus | every Vorbis comment key preserved |
| MP3 (ID3v2.4) | every key preserved (unmapped keys round-trip as `TXXX`) |
| M4A/ALAC, WAV/AIFF | shared field set; other keys reported *unplaced* |

## Tests

```sh
cargo test               # unit + §14 acceptance tests, headless, no network, no device
cargo test --test acceptance
```

## Specification and compatibility

- [`docs/SPECIFICATION.md`](docs/SPECIFICATION.md) — the **data & format
  specification**: every on-disk and wire format (directory layout, config keys,
  index schema, layout report, grade report and issue codes, tag families and
  canonical vocabularies, sidecars, the naming template, the trash manifest,
  scripts/order/chains, tools). It is derived from **la-musica**, the reference
  implementation, so a library written by either app is understood identically.
- [`docs/INSTALL.md`](docs/INSTALL.md) — detailed per-platform install.
- [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md) — the crate audit.
- [`RELEASING.md`](RELEASING.md) — how to cut a release; the GitHub Pages site
  lives in `docs/`.

Compatibility with la-musica in one line: the same **library on disk** (layout,
sidecar names, naming template), the same **check registry** (`grade_check_*` /
`grade_include_*`, all ON, Strict/Balanced/Relaxed presets), the same **issue
codes**, the same **tag vocabularies and spacing rules**, the same **layout
report** and **trash manifest** formats, and the same **script ids, order and
gates**. mlo reads a la-musica `config.json` on first run and keeps the keys it
does not model in `[extra]`. What it does not implement — the server, the web
and desktop clients, multi-user features and the AI-backed passes — is listed in
the specification's *Honest limits*.

## Licence

MIT — see [LICENSE](LICENSE).