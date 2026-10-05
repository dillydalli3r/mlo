---
layout: default
title: Installing mlo
---

# Installing mlo

`mlo` is a single native binary. There are three ways to get it: the install
script (prebuilt release), a direct download of the archive for your platform,
or a from-source build with Cargo. All three give the same program.

The current version is **0.1.0**. Every release publishes these archives, named
`mlo-<version>-<target>` with the `.tar.gz` extension everywhere except
Windows, and a `SHA256SUMS` file covering all of them:

| platform | target triple | archive |
|---|---|---|
| Windows x86_64 | `x86_64-pc-windows-msvc` | `mlo-0.1.0-x86_64-pc-windows-msvc.zip` |
| macOS Intel | `x86_64-apple-darwin` | `mlo-0.1.0-x86_64-apple-darwin.tar.gz` |
| macOS Apple silicon | `aarch64-apple-darwin` | `mlo-0.1.0-aarch64-apple-darwin.tar.gz` |
| Linux x86_64 | `x86_64-unknown-linux-gnu` | `mlo-0.1.0-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `aarch64-unknown-linux-gnu` | `mlo-0.1.0-aarch64-unknown-linux-gnu.tar.gz` |
| Linux x86_64, no audio | `x86_64-unknown-linux-gnu` | `mlo-0.1.0-x86_64-unknown-linux-gnu-min.tar.gz` |
| Linux arm64, no audio | `aarch64-unknown-linux-gnu` | `mlo-0.1.0-aarch64-unknown-linux-gnu-min.tar.gz` |

Each archive contains the `mlo` binary (`mlo.exe` on Windows), `README.md`,
`LICENSE`, `CHANGELOG.md`, `install.sh` and `install.ps1`.

## Install script (Linux and macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.sh | sh
```

The script detects the OS and architecture, downloads the matching archive,
verifies it against `SHA256SUMS`, and installs the binary into `~/.local/bin`.
It prints a `PATH` line to add if that directory is not already on your `PATH`.

Options (pass them after `sh -s --` when piping):

| option | effect |
|---|---|
| `--no-audio` | install the Linux `-min` build (no playback, no archives) |
| `--system` | install into `/usr/local/bin` (uses `sudo` when needed) |
| `--version 0.1.0` | install a specific version |
| `--dir "$HOME/bin"` | install somewhere else |

Examples:

```sh
# Linux container/CI: no audio, no archive support, no system libraries
curl -fsSL https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.sh | sh -s -- --no-audio

# system-wide
curl -fsSL https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.sh | sh -s -- --system
```

The installer is POSIX `sh`; on a platform it does not publish a build for it
exits with `UNSUPPORTED_PLATFORM` rather than guessing.

## Install script (Windows)

From PowerShell:

```powershell
irm https://raw.githubusercontent.com/dillydalli3r/mlo/main/install.ps1 | iex
```

It downloads the `x86_64-pc-windows-msvc` archive, verifies the checksum,
extracts it to `%LOCALAPPDATA%\Programs\mlo`, and adds that directory to your
**user** `PATH` (no administrator rights). Open a new terminal afterwards.

Options: `-Version 0.1.0`, `-InstallDir "$env:USERPROFILE\bin"`,
`-Repo owner/name`. Windows on ARM64 is refused with `UNSUPPORTED_PLATFORM` —
the published Windows binary is x86_64; build the `aarch64-pc-windows-msvc`
target from source instead.

## Download by hand

Grab the archive for your platform from the
[releases page](https://github.com/dillydalli3r/mlo/releases/latest), then verify
and extract. On Linux/macOS:

```sh
curl -fLO https://github.com/dillydalli3r/mlo/releases/download/v0.1.0/mlo-0.1.0-x86_64-unknown-linux-gnu.tar.gz
curl -fLO https://github.com/dillydalli3r/mlo/releases/download/v0.1.0/SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS          # macOS: shasum -a 256 -c --ignore-missing SHA256SUMS
tar -xzf mlo-0.1.0-x86_64-unknown-linux-gnu.tar.gz
install -m 755 mlo-0.1.0-x86_64-unknown-linux-gnu/mlo ~/.local/bin/mlo
```

On Windows, `Expand-Archive mlo-0.1.0-x86_64-pc-windows-msvc.zip` and run
`mlo.exe`; verify the hash with
`Get-FileHash -Algorithm SHA256 .\mlo-0.1.0-x86_64-pc-windows-msvc.zip` against
the matching line in `SHA256SUMS`.

## From source

Requires Rust **1.85 or newer** (the crate is edition 2024). No other build
toolchain is needed: SQLite is bundled and statically linked, and the terminal
backend is pure Rust.

```sh
git clone https://github.com/dillydalli3r/mlo
cd <repo>
cargo install --path .            # installs the `mlo` binary
```

Or straight from git without a checkout:

```sh
cargo install --git https://github.com/dillydalli3r/mlo
```

### Linux and the `audio` feature

The default features are `default = ["audio", "archives"]`. The `audio`
feature enables in-process playback (`rodio` → `cpal`), which on Linux links
**ALSA at link time**, so a source build needs the development headers and
`pkg-config`:

```sh
# Debian / Ubuntu
sudo apt install libasound2-dev pkg-config
cargo install --path .
```

At runtime, playback needs an ALSA-visible device or a compatibility layer
(`pipewire-alsa`, PulseAudio's ALSA plugin). The analysis, tagging, layout and
grading features do not need it.

To build without any system library requirement — containers, headless CI, a
machine with no audio stack:

```sh
cargo install --path . --no-default-features
```

`--no-default-features` drops **both** default features: the `audio` feature
(playback) **and** the `archives` feature (`.zip`/`.7z`/`.tar.*` extraction).
The player then reports `playback unavailable: built without the audio feature`
and archive import reports a named refusal, instead of the binary failing to
link. If you want archives but not playback, ask for the feature explicitly:

```sh
cargo install --path . --no-default-features --features archives
```

macOS uses CoreAudio and Windows uses WASAPI, so neither needs an extra
package for audio; the same `--no-default-features` flag works there if you
want playback and archive extraction compiled out.

## Verify the install

```sh
mlo doctor
```

`mlo doctor` reports the environment, the external tools it found (and their
paths), container support and network health, each with a reason when
something is unavailable. `mlo paths` prints where the config, state and index
live. A non-zero exit and a stable reason code (for example
`UNSUPPORTED_CONTAINER`, `NETWORK_UNAVAILABLE`, `TOOL_UNAVAILABLE`) means the
same thing on every command; `mlo` never prints a bare "error".

Then add the file-manager integration if you want it:

```sh
mlo shell install
```

## Uninstall

- **Install script (Linux/macOS):** `rm ~/.local/bin/mlo` (or
  `/usr/local/bin/mlo` for a `--system` install).
- **Install script (Windows):** remove
  `%LOCALAPPDATA%\Programs\mlo`, and drop that folder from your user `PATH`
  (Settings → Environment Variables). The installer never edits the machine
  `PATH`.
- **Cargo install:** `cargo uninstall mlo-tui` (the binary is `mlo`; the
  package is `mlo-tui`).
- **Shell integration, on any platform:** `mlo shell uninstall` removes the
  registry key / Finder Service / `.desktop` file
  (`mlo shell status` shows what is currently registered).

Your library is untouched: the app's own state lives in `<music>/.mlo/`, and
trashed files are under `<music>/.mlo/trash/`. Remove that folder only if you
do not want the trash manifests.