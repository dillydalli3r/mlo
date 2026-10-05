//! Atomic writes ([§1.3]): temp file beside the destination, flush, fsync,
//! one rename. A killed process leaves the old file or the new one.
//!
//! `std::fs::rename` maps to `MoveFileExW(.., MOVEFILE_REPLACE_EXISTING)` on
//! Windows and `renameat2`/`rename` on Unix, so the replace is atomic on every
//! supported platform.

use crate::error::{IoResultExt, MloError, Result};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Unique temp name in the destination's directory (same filesystem => rename
/// cannot cross a device boundary).
fn temp_path(path: &Path) -> Result<PathBuf> {
    let dir = path
        .parent()
        .ok_or_else(|| MloError::Invalid(format!("no parent directory for {}", path.display())))?;
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let stamp = format!(".{name}.{pid}.{nanos:08x}.mlo-tmp");
    Ok(dir.join(stamp))
}

/// Write `bytes` to `path` atomically, creating parent dirs.
pub fn write_atomic(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    write_atomic_with(path, |f| f.write_all(bytes))
}

/// Write with a closure for large/streamed payloads.
pub fn write_atomic_with<F>(path: impl AsRef<Path>, write: F) -> Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    let tmp = temp_path(path)?;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .at(&tmp)?;
        write(&mut file).at(&tmp)?;
        file.flush().at(&tmp)?;
        file.sync_all().at(&tmp)?;
        drop(file);
        std::fs::rename(&tmp, path).at(path)?;
        sync_parent(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// String convenience.
pub fn write_atomic_str(path: impl AsRef<Path>, s: &str) -> Result<()> {
    write_atomic(path, s.as_bytes())
}

/// JSON convenience (pretty, trailing newline).
pub fn write_atomic_json<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    let mut s = serde_json::to_string_pretty(value)
        .map_err(|e| MloError::Other(format!("json encode: {e}")))?;
    s.push('\n');
    write_atomic_str(path, &s)
}

/// Best-effort directory fsync on Unix; a no-op on Windows (rename durability
/// there is not exposed without an extra handle).
#[cfg(unix)]
fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) {}

/// Canonical, comparable form of a path: canonicalise when possible, strip the
/// Windows `\\?\` verbatim prefix, unify separators, and fold case on Windows.
/// Used for containment checks so a just-moved path still compares.
pub fn normalize_path(p: &Path) -> String {
    let canon = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let mut s = canon.to_string_lossy().replace('\\', "/");
    for prefix in ["//?/", "//./"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
            break;
        }
    }
    while s.ends_with('/') && s.len() > 1 {
        s.pop();
    }
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

/// Is `child` inside `root` (or equal to it)?
pub fn path_is_within(child: &Path, root: &Path) -> bool {
    let c = normalize_path(child);
    let r = normalize_path(root);
    c == r || c.starts_with(&format!("{r}/"))
}

/// Remove stale temp files left by a killed process ([§3.5]).
/// Returns the temp paths removed.
pub fn sweep_temp_files(dir: &Path) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return removed };
    for entry in rd.flatten() {
        let p = entry.path();
        let is_temp = p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".mlo-tmp") || n.contains(".mlo-edit."))
            .unwrap_or(false);
        if is_temp && std::fs::remove_file(&p).is_ok() {
            removed.push(p);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.bin");
        write_atomic(&f, b"old").unwrap();
        write_atomic(&f, b"new").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"new");
        // no temp leftovers
        assert!(sweep_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x/y/z.txt");
        write_atomic_str(&f, "hi").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hi");
    }
}