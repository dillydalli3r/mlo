#!/bin/sh
# mlo installer for Linux and macOS.
#
# Downloads the prebuilt release archive for this machine's OS and
# architecture, verifies it against the release's SHA256SUMS, and installs the
# `mlo` binary into ~/.local/bin (or /usr/local/bin with --system).
#
# Usage:
#   sh install.sh              # latest release, into ~/.local/bin
#   sh install.sh --no-audio   # Linux `-min` build (no audio, no archives)
#   sh install.sh --system     # /usr/local/bin (uses sudo if needed)
#   sh install.sh --version 0.1.0
#   sh install.sh --dir "$HOME/bin"
#
# Environment overrides: MLO_REPO (owner/name), MLO_VERSION, MLO_INSTALL_DIR.
#
# Every refusal below carries a stable reason code, so a failure can be acted
# on without parsing prose.

set -eu

REPO="${MLO_REPO:-dillydalli3r/mlo}"
VERSION="${MLO_VERSION:-}"
INSTALL_DIR="${MLO_INSTALL_DIR:-}"
WANT_MIN=0
USE_SYSTEM=0

fail() {
    printf 'mlo-install: error: %s\n' "$1" >&2
    exit 1
}

note() {
    printf 'mlo-install: %s\n' "$1"
}

usage() {
    cat <<'EOF'
mlo installer — Linux and macOS

Options:
  --no-audio        install the Linux `-min` build (no playback, no archives)
  --system          install into /usr/local/bin (sudo when required)
  --version VER     install a specific version (default: the latest release)
  --dir PATH        install into PATH instead of ~/.local/bin
  -h, --help        show this help

Environment:
  MLO_REPO          repository owner/name (default: dillydalli3r/mlo)
  MLO_VERSION       same as --version
  MLO_INSTALL_DIR   same as --dir
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-audio) WANT_MIN=1 ;;
        --system) USE_SYSTEM=1 ;;
        --version)
            [ "$#" -ge 2 ] || fail "MISSING_VALUE: --version needs a version"
            shift; VERSION="$1" ;;
        --version=*) VERSION="${1#*=}" ;;
        --dir)
            [ "$#" -ge 2 ] || fail "MISSING_VALUE: --dir needs a path"
            shift; INSTALL_DIR="$1" ;;
        --dir=*) INSTALL_DIR="${1#*=}" ;;
        -h|--help) usage; exit 0 ;;
        *) fail "UNKNOWN_OPTION: $1" ;;
    esac
    shift
done

# --- platform -------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
    Linux) os_part="unknown-linux-gnu" ;;
    Darwin) os_part="apple-darwin" ;;
    *) fail "UNSUPPORTED_PLATFORM: operating system '$os' (mlo ships for Linux and macOS; Windows uses install.ps1)" ;;
esac

case "$arch" in
    x86_64|amd64) arch_part="x86_64" ;;
    arm64|aarch64) arch_part="aarch64" ;;
    *) fail "UNSUPPORTED_PLATFORM: architecture '$arch' on '$os'" ;;
esac

target="${arch_part}-${os_part}"

# Only the Linux jobs build a --no-default-features archive, so --no-audio is
# meaningful there. On macOS the default build's playback uses CoreAudio (no
# system package), but a caller who asked for no audio still gets told the
# variant does not exist rather than silently handed playback.
suffix=""
if [ "$WANT_MIN" = 1 ]; then
    if [ "$os" = "Linux" ]; then
        suffix="-min"
    else
        note "WARNING: no -min build is published for ${target}; installing the default build"
    fi
fi

# --- version --------------------------------------------------------------
if [ -z "$VERSION" ]; then
    latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")" \
        || fail "NETWORK: could not reach https://github.com/${REPO}/releases/latest"
    tag="${latest_url##*/}"
    VERSION="${tag#v}"
    [ -n "$VERSION" ] || fail "NO_RELEASE: could not read a version from '${latest_url}'"
fi

asset="mlo-${VERSION}-${target}${suffix}.tar.gz"
base="https://github.com/${REPO}/releases/download/v${VERSION}"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/mlo-install.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT INT TERM

# --- download -------------------------------------------------------------
note "downloading ${base}/${asset}"
curl -fsSL "${base}/${asset}" -o "${tmp}/${asset}" \
    || fail "DOWNLOAD_FAILED: ${base}/${asset} (is v${VERSION} released for ${target}${suffix}?)"
curl -fsSL "${base}/SHA256SUMS" -o "${tmp}/SHA256SUMS" \
    || fail "DOWNLOAD_FAILED: ${base}/SHA256SUMS"

# --- verify ---------------------------------------------------------------
expected="$(awk -v a="$asset" '$2 == a { print $1 }' "${tmp}/SHA256SUMS")"
[ -n "$expected" ] || fail "CHECKSUM_MISSING: no SHA256SUMS entry for ${asset}"

if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${tmp}/${asset}" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "${tmp}/${asset}" | awk '{ print $1 }')"
else
    fail "NO_SHA256_TOOL: neither sha256sum nor shasum is installed"
fi

[ "$actual" = "$expected" ] \
    || fail "CHECKSUM_MISMATCH: ${asset} is ${actual}, expected ${expected}"

# --- extract --------------------------------------------------------------
tar -xzf "${tmp}/${asset}" -C "$tmp" || fail "EXTRACT_FAILED: ${asset}"
binary="${tmp}/${asset%.tar.gz}/mlo"
[ -f "$binary" ] || fail "ARCHIVE_LAYOUT: ${asset} does not contain ${asset%.tar.gz}/mlo"

# --- install --------------------------------------------------------------
if [ -z "$INSTALL_DIR" ]; then
    if [ "$USE_SYSTEM" = 1 ]; then
        INSTALL_DIR="/usr/local/bin"
    else
        INSTALL_DIR="${HOME}/.local/bin"
    fi
fi

SUDO=""
if [ ! -d "$INSTALL_DIR" ]; then
    if mkdir -p "$INSTALL_DIR" 2>/dev/null; then
        :
    elif command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
        $SUDO mkdir -p "$INSTALL_DIR" || fail "NOT_WRITABLE: could not create ${INSTALL_DIR}"
    else
        fail "NOT_WRITABLE: could not create ${INSTALL_DIR} and sudo is unavailable"
    fi
fi
if [ ! -w "$INSTALL_DIR" ]; then
    if command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
    else
        fail "NOT_WRITABLE: ${INSTALL_DIR} is not writable and sudo is unavailable"
    fi
fi

# $SUDO is intentionally unquoted: it is either empty or the word `sudo`.
# shellcheck disable=SC2086
$SUDO cp "$binary" "${INSTALL_DIR}/mlo" || fail "INSTALL_FAILED: could not write ${INSTALL_DIR}/mlo"
# shellcheck disable=SC2086
$SUDO chmod 755 "${INSTALL_DIR}/mlo"

note "installed mlo ${VERSION} -> ${INSTALL_DIR}/mlo"

# --- PATH -----------------------------------------------------------------
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*)
        ;;
    *)
        note "${INSTALL_DIR} is not on your PATH. Add it:"
        printf '\n  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
        printf '\nAppend that line to ~/.profile (or ~/.zshrc) to make it permanent.\n\n'
        ;;
esac

note "next: mlo doctor   (then: mlo shell install, to add the right-click menu)"