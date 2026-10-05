//! Unified tag layer ([§6.1]) — one API over FLAC, Ogg Vorbis, Opus, MP3,
//! M4A/ALAC, WAV/AIFF and MKV/MP4.
//!
//! Fidelity: FLAC / Ogg Vorbis / Opus preserve every Vorbis comment key, MP3
//! preserves every key via TXXX. MP4/WAV/AIFF map the shared field set and
//! report other keys as *unplaced* (never invented, [§5.4]).
//!
//! Writes are atomic ([§1.3]); the writer's normalization is the grader's
//! normalization ([§6.3]).

pub mod flac;
pub mod id3mp3;
pub mod oggstream;
pub mod other;
pub mod vorbis;

use crate::error::{IoResultExt, MloError, Result};
use crate::model::{ContainerKind, TagMap};
use std::path::{Path, PathBuf};

/// Sniff the container by magic bytes, falling back to the extension.
pub fn detect_container(path: &Path) -> ContainerKind {
    use std::io::Read;
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    let h = &head[..n];
    if h.len() >= 4 {
        if &h[0..4] == b"fLaC" {
            return ContainerKind::Flac;
        }
        if &h[0..4] == b"OggS" {
            return detect_ogg_codec(path);
        }
        if &h[0..3] == b"ID3" {
            return ContainerKind::Mp3;
        }
        if &h[0..4] == b"RIFF" {
            return ContainerKind::Wav;
        }
        if &h[0..4] == b"FORM" {
            return ContainerKind::Aiff;
        }
        if h.len() >= 8 && &h[4..8] == b"ftyp" {
            return mp4_kind(h, path);
        }
        if h.len() >= 4 && h[0] == 0x1A && h[1] == 0x45 && h[2] == 0xDF && h[3] == 0xA3 {
            return ContainerKind::Mkv;
        }
        // MP3 without an ID3 header: frame sync 0xFF Ex
        if h[0] == 0xFF && (h[1] & 0xE0) == 0xE0 {
            return ContainerKind::Mp3;
        }
    }
    from_extension(path)
}

fn detect_ogg_codec(path: &Path) -> ContainerKind {
    use std::io::Read;
    let mut buf = vec![0u8; 8192];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    let hay = &buf[..n];
    if find_subslice(hay, b"OpusHead").is_some() {
        ContainerKind::Opus
    } else if find_subslice(hay, b"vorbis").is_some() {
        ContainerKind::OggVorbis
    } else {
        ContainerKind::OggVorbis
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn mp4_kind(head: &[u8], path: &Path) -> ContainerKind {
    let brand = &head[8..12.min(head.len())];
    if brand.starts_with(b"M4A") || brand.starts_with(b"M4B") {
        return ContainerKind::Mp4;
    }
    match path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("m4a" | "m4b" | "m4p" | "alac" | "aac") => ContainerKind::Mp4,
        _ => ContainerKind::Mp4Video,
    }
}

fn from_extension(path: &Path) -> ContainerKind {
    match path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("flac") => ContainerKind::Flac,
        Some("ogg" | "oga") => ContainerKind::OggVorbis,
        Some("opus") => ContainerKind::Opus,
        Some("mp3") => ContainerKind::Mp3,
        Some("m4a" | "m4b" | "m4p" | "alac" | "aac") => ContainerKind::Mp4,
        Some("wav" | "wave") => ContainerKind::Wav,
        Some("aiff" | "aif" | "aifc") => ContainerKind::Aiff,
        Some("mkv") => ContainerKind::Mkv,
        Some("mp4" | "m4v" | "mov") => ContainerKind::Mp4Video,
        _ => ContainerKind::Unknown,
    }
}

pub fn is_audio(path: &Path) -> bool {
    matches!(
        detect_container(path),
        ContainerKind::Flac
            | ContainerKind::OggVorbis
            | ContainerKind::Opus
            | ContainerKind::Mp3
            | ContainerKind::Mp4
            | ContainerKind::Wav
            | ContainerKind::Aiff
    )
}

pub fn is_video(path: &Path) -> bool {
    matches!(detect_container(path), ContainerKind::Mkv | ContainerKind::Mp4Video)
}

/// Read tags as stored (keys uppercased, values left raw so the grader can see
/// a spacing/case problem).
pub fn read_tags(path: &Path) -> Result<TagMap> {
    match detect_container(path) {
        ContainerKind::Flac => flac::read_tags(path),
        ContainerKind::OggVorbis | ContainerKind::Opus => oggstream::read_tags(path),
        ContainerKind::Mp3 => id3mp3::read_tags(path),
        ContainerKind::Mp4 | ContainerKind::Wav | ContainerKind::Aiff => other::read_tags(path),
        ContainerKind::Mkv | ContainerKind::Mp4Video => other::read_tags(path),
        ContainerKind::Unknown => Err(MloError::UnsupportedContainer {
            path: path.to_path_buf(),
            container: "unknown".into(),
            reason: "unrecognised container".into(),
        }),
    }
}

/// Write the full tag set (normalized) atomically.
pub fn write_tags(path: &Path, tags: &TagMap) -> Result<()> {
    let mut normalized = tags.clone();
    crate::tagkey::normalize_tags(&mut normalized);
    match detect_container(path) {
        ContainerKind::Flac => flac::write_tags(path, &normalized),
        ContainerKind::OggVorbis | ContainerKind::Opus => oggstream::write_tags(path, &normalized),
        ContainerKind::Mp3 => id3mp3::write_tags(path, &normalized),
        ContainerKind::Mp4 | ContainerKind::Wav | ContainerKind::Aiff | ContainerKind::Mkv
        | ContainerKind::Mp4Video => {
            let unplaced = other::write_tags(path, &normalized)?;
            if !unplaced.is_empty() {
                tracing::warn!(
                    path = %path.display(),
                    unplaced = %unplaced.join(", "),
                    "tags with no home in this container were not written"
                );
            }
            Ok(())
        }
        ContainerKind::Unknown => Err(MloError::UnsupportedContainer {
            path: path.to_path_buf(),
            container: "unknown".into(),
            reason: "cannot carry tags".into(),
        }),
    }
}

/// Read-modify-write with the caller's closure; returns the new tag set.
pub fn update_tags<F>(path: &Path, f: F) -> Result<TagMap>
where
    F: FnOnce(&mut TagMap),
{
    let mut tags = read_tags(path)?;
    f(&mut tags);
    write_tags(path, &tags)?;
    Ok(tags)
}

/// Set one key's values (empty slice removes the key).
pub fn set_values(path: &Path, key: &str, values: &[String]) -> Result<TagMap> {
    let key = key.to_ascii_uppercase();
    update_tags(path, |tags| {
        if values.is_empty() {
            tags.remove(&key);
        } else {
            let entry = tags.entry(key.clone()).or_default();
            for v in values {
                if !entry.iter().any(|e| e == v) {
                    entry.push(v.clone());
                }
            }
        }
    })
}

pub fn remove_key(path: &Path, key: &str) -> Result<TagMap> {
    set_values(path, key, &[])
}

/// Does this container preserve arbitrary keys?
pub fn supports_extended(path: &Path) -> bool {
    matches!(
        detect_container(path),
        ContainerKind::Flac | ContainerKind::OggVorbis | ContainerKind::Opus | ContainerKind::Mp3
    )
}

pub fn embedded_cover(path: &Path) -> Result<Option<Vec<u8>>> {
    other::embedded_cover(path)
}

// ---------------------------------------------------------------------------
// atomic helpers shared with the in-place writers
// ---------------------------------------------------------------------------

/// A unique sibling temp path in the destination directory. The original
/// extension is preserved so type-sniffing writers (lofty) still recognise it.
pub fn temp_sibling(path: &Path) -> Result<PathBuf> {
    let dir = path
        .parent()
        .ok_or_else(|| MloError::Invalid(format!("no parent for {}", path.display())))?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let name = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!(".{stem}.{pid}.{nanos:08x}.mlo-edit.{ext}"),
        _ => format!(".{stem}.{pid}.{nanos:08x}.mlo-edit"),
    };
    Ok(dir.join(name))
}

/// Copy `path` to a sibling temp, run `edit` on the temp, then rename over the
/// original ([§1.3]). The original is untouched if `edit` fails.
pub fn atomic_edit_copy<F>(path: &Path, edit: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    let tmp = temp_sibling(path)?;
    std::fs::copy(path, &tmp).at(&tmp)?;
    match edit(&tmp) {
        Ok(()) => {
            std::fs::rename(&tmp, path).at(path)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_when_no_magic() {
        assert_eq!(detect_container(Path::new("x.flac")), ContainerKind::Flac);
        assert_eq!(detect_container(Path::new("x.mp3")), ContainerKind::Mp3);
        assert_eq!(detect_container(Path::new("x.m4a")), ContainerKind::Mp4);
        assert_eq!(detect_container(Path::new("x.opus")), ContainerKind::Opus);
    }

    #[test]
    fn unsupported_container_reports_reason() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.txt");
        std::fs::write(&f, b"not music").unwrap();
        let err = read_tags(&f).unwrap_err();
        assert_eq!(err.reason_code(), "UNSUPPORTED_CONTAINER");
    }
}