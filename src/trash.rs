//! The one trash implementation ([§1.2]): every removal moves the file/folder
//! into `<music>/.mlo/trash/<user>/` with an origin manifest that can put it
//! back. Nothing is ever deleted.
//!
//! The bin layout and manifest follow la-musica (`mlo/paths.py` `trash_path`):
//! entries sit directly in the bin, recorded in `.mlo_manifest.json` as
//! `{"version": 1, "entries": {"<name>": {"origin": "<abs>", "at": "<ts>"}}}`.
//! The older timestamped layout (`<user>/<stamp>/manifest.json`) is still
//! listed and restorable.

use crate::atomic;
use crate::error::{IoResultExt, MloError, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashEntry {
    pub original_path: PathBuf,
    pub trash_path: PathBuf,
    pub reason: String,
    pub is_dir: bool,
    /// True when the move was a copy+remove (cross-device fallback).
    pub copied: bool,
}

/// One manifest as read back for the Trash page.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub user: String,
    pub stamp: String,
    pub created_at: String,
    pub entries: Vec<TrashEntry>,
}

#[derive(Debug, Clone)]
pub struct ManifestSummary {
    pub user: String,
    pub stamp: String,
    pub created_at: String,
    pub entries: usize,
    pub path: PathBuf,
}

/// la-musica `.mlo_manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BinManifest {
    version: u32,
    entries: BTreeMap<String, BinEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BinEntry {
    origin: String,
    at: String,
}

/// The legacy timestamped manifest (`<user>/<stamp>/manifest.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyManifest {
    user: String,
    stamp: String,
    created_at: String,
    entries: Vec<TrashEntry>,
}

pub struct Trash {
    root: PathBuf,
    music_root: PathBuf,
    user: String,
}

/// A batch shares one bin and one manifest, so one operation undoes atomically.
pub struct TrashBatch<'a> {
    trash: &'a Trash,
    bin: PathBuf,
    entries: Vec<TrashEntry>,
    existing: BTreeMap<String, BinEntry>,
}

impl Trash {
    pub fn new(music_root: impl AsRef<Path>) -> Self {
        let music_root = music_root.as_ref().to_path_buf();
        let root = music_root.join(".mlo").join("trash");
        Self { root, music_root, user: current_user() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    /// `<user>`, or the literal `default` for an unclaimed install.
    fn user_segment(&self) -> String {
        let u = sanitize_user(&self.user);
        if u.is_empty() {
            "default".to_string()
        } else {
            u
        }
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root.join(self.user_segment())
    }

    /// Refuse anything that is not library content ([§4.2] rule 1).
    fn guard(&self, path: &Path) -> Result<()> {
        if !atomic::path_is_within(path, &self.music_root) {
            return Err(MloError::Invalid(format!(
                "refusing to trash {} — outside the music folder",
                path.display()
            )));
        }
        let norm = atomic::normalize_path(path);
        let music_norm = atomic::normalize_path(&self.music_root);
        if norm == music_norm {
            return Err(MloError::Invalid("refusing to trash the music folder itself".into()));
        }
        let state_prefix = format!("{music_norm}/.mlo");
        if norm == state_prefix || norm.starts_with(&format!("{state_prefix}/")) {
            return Err(MloError::Invalid("refusing to trash app state (.mlo)".into()));
        }
        if crate::model::file_name(path).eq_ignore_ascii_case("Artists")
            && atomic::normalize_path(path.parent().unwrap_or(path)) == music_norm
        {
            return Err(MloError::Invalid("refusing to trash the Artists/ folder".into()));
        }
        Ok(())
    }

    pub fn begin(&self, _reason: &str) -> Result<TrashBatch<'_>> {
        let bin = self.bin_dir();
        fs::create_dir_all(&bin).at(&bin)?;
        let existing = read_bin_entries(&bin);
        Ok(TrashBatch { trash: self, bin, entries: Vec::new(), existing })
    }

    /// Every manifest in the trash: the current bin manifest plus any legacy
    /// timestamped manifests still present.
    pub fn list(&self) -> Result<Vec<ManifestSummary>> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for user_dir in fs::read_dir(&self.root).at(&self.root)?.flatten() {
            if !user_dir.path().is_dir() {
                continue;
            }
            let user = user_dir.file_name().to_string_lossy().into_owned();
            // current layout: <user>/.mlo_manifest.json
            let rows = read_bin_entries(&user_dir.path());
            if !rows.is_empty() {
                let created = rows.values().map(|e| e.at.clone()).max().unwrap_or_default();
                out.push(ManifestSummary {
                    user: user.clone(),
                    stamp: user.clone(),
                    created_at: created,
                    entries: rows.len(),
                    path: user_dir.path(),
                });
            }
            // legacy layout: <user>/<stamp>/manifest.json
            for stamp_dir in fs::read_dir(user_dir.path()).at(user_dir.path())?.flatten() {
                let mpath = stamp_dir.path().join("manifest.json");
                if !mpath.exists() {
                    continue;
                }
                if let Ok(text) = fs::read_to_string(&mpath) {
                    if let Ok(m) = serde_json::from_str::<LegacyManifest>(&text) {
                        out.push(ManifestSummary {
                            user: m.user,
                            stamp: m.stamp,
                            created_at: m.created_at,
                            entries: m.entries.len(),
                            path: stamp_dir.path(),
                        });
                    }
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    pub fn read_manifest(dir: &Path) -> Result<Manifest> {
        // current layout first
        let entries = read_bin_entries(dir);
        if !entries.is_empty() {
            let created = entries.values().map(|e| e.at.clone()).max().unwrap_or_default();
            let user = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            return Ok(Manifest {
                user: user.clone(),
                stamp: user,
                created_at: created,
                entries: entries
                    .into_iter()
                    .map(|(name, e)| TrashEntry {
                        trash_path: dir.join(&name),
                        original_path: native_path(&e.origin),
                        reason: String::new(),
                        is_dir: false,
                        copied: false,
                    })
                    .collect(),
            });
        }
        // legacy
        let mpath = dir.join("manifest.json");
        let text = fs::read_to_string(&mpath).at(&mpath)?;
        let m: LegacyManifest = serde_json::from_str(&text)
            .map_err(|e| MloError::Other(format!("manifest parse: {e}")))?;
        Ok(Manifest { user: m.user, stamp: m.stamp, created_at: m.created_at, entries: m.entries })
    }

    /// Restore every entry in a manifest to its exact original path.
    pub fn restore(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        let manifest = Self::read_manifest(dir)?;
        let mut restored = Vec::new();
        for entry in &manifest.entries {
            restore_one(&entry.trash_path, &entry.original_path)?;
            restored.push(entry.original_path.clone());
        }
        if !restored.is_empty() {
            let _ = remove_from_bin_manifest(dir, &manifest);
        }
        Ok(restored)
    }

    pub fn restore_entry(&self, dir: &Path, original: &Path) -> Result<PathBuf> {
        let manifest = Self::read_manifest(dir)?;
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.original_path == original)
            .ok_or_else(|| MloError::NotFound(format!("no trash entry for {}", original.display())))?;
        restore_one(&entry.trash_path, &entry.original_path)?;
        let one = Manifest {
            user: manifest.user.clone(),
            stamp: manifest.stamp.clone(),
            created_at: manifest.created_at.clone(),
            entries: vec![entry.clone()],
        };
        let _ = remove_from_bin_manifest(dir, &one);
        Ok(entry.original_path.clone())
    }
}

impl TrashBatch<'_> {
    /// Move one path into the bin and record it in the manifest.
    pub fn move_path(&mut self, path: &Path, reason: &str) -> Result<TrashEntry> {
        self.trash.guard(path)?;
        if !path.exists() {
            return Err(MloError::NotFound(format!("{} does not exist", path.display())));
        }
        fs::create_dir_all(&self.bin).at(&self.bin)?;
        let is_dir = path.is_dir();
        let name = crate::model::file_name(path);
        let (stem, ext) = if is_dir {
            (name.clone(), String::new())
        } else {
            match name.rsplit_once('.') {
                Some((s, e)) => (s.to_string(), format!(".{e}")),
                None => (name.clone(), String::new()),
            }
        };
        let mut dest = self.bin.join(&name);
        let mut n = 2;
        while dest.exists() {
            dest = self.bin.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        let copied = move_path(path, &dest)?;
        let entry = TrashEntry {
            original_path: path.to_path_buf(),
            trash_path: dest.clone(),
            reason: reason.to_string(),
            is_dir,
            copied,
        };
        let key = crate::model::file_name(&dest);
        self.existing.insert(
            key,
            BinEntry {
                origin: path.to_string_lossy().replace('\\', "/"),
                at: Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            },
        );
        self.entries.push(entry.clone());
        self.flush()?;
        Ok(entry)
    }

    pub fn entries(&self) -> &[TrashEntry] {
        &self.entries
    }

    fn flush(&self) -> Result<()> {
        let manifest = BinManifest { version: 1, entries: self.existing.clone() };
        atomic::write_atomic_json(self.bin.join(".mlo_manifest.json"), &manifest)
    }

    /// Finish the batch. Writes the manifest; returns the bin directory.
    pub fn finish(self) -> Result<PathBuf> {
        self.flush()?;
        Ok(self.bin)
    }
}

fn read_bin_entries(bin: &Path) -> BTreeMap<String, BinEntry> {
    let path = bin.join(".mlo_manifest.json");
    let Ok(text) = fs::read_to_string(&path) else { return BTreeMap::new() };
    match serde_json::from_str::<BinManifest>(&text) {
        Ok(m) if m.version == 1 => m.entries,
        _ => BTreeMap::new(),
    }
}

fn remove_from_bin_manifest(dir: &Path, restored: &Manifest) -> Result<()> {
    let mut entries = read_bin_entries(dir);
    if entries.is_empty() {
        return Ok(());
    }
    for e in &restored.entries {
        let name = crate::model::file_name(&e.trash_path);
        entries.remove(&name);
    }
    let path = dir.join(".mlo_manifest.json");
    if entries.is_empty() {
        let _ = fs::remove_file(&path);
        Ok(())
    } else {
        atomic::write_atomic_json(&path, &BinManifest { version: 1, entries })
    }
}



/// Move a file or directory; `rename` first, copy+remove when it crosses a
/// device. Returns `true` when a copy was needed.
fn move_path(src: &Path, dest: &Path) -> Result<bool> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(false),
        Err(_) => {
            copy_recursive(src, dest)?;
            if src.is_dir() {
                fs::remove_dir_all(src).at(src)?;
            } else {
                fs::remove_file(src).at(src)?;
            }
            Ok(true)
        }
    }
}

fn copy_recursive(src: &Path, dest: &Path) -> Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dest).at(dest)?;
        for entry in fs::read_dir(src).at(src)?.flatten() {
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).at(parent)?;
        }
        fs::copy(src, dest).at(dest)?;
    }
    Ok(())
}

fn restore_one(trash_path: &Path, original: &Path) -> Result<()> {
    if !trash_path.exists() {
        return Err(MloError::NotFound(format!(
            "trash payload missing: {}",
            trash_path.display()
        )));
    }
    if original.exists() {
        return Err(MloError::Invalid(format!(
            "cannot restore {} — target already exists",
            original.display()
        )));
    }
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent).at(parent)?;
    }
    fs::rename(trash_path, original).or_else(|_| -> Result<()> {
        copy_recursive(trash_path, original)?;
        if trash_path.is_dir() {
            fs::remove_dir_all(trash_path).at(trash_path)?;
        } else {
            fs::remove_file(trash_path).at(trash_path)?;
        }
        Ok(())
    })?;
    Ok(())
}

pub fn current_user() -> String {
    for var in ["MLO_USER", "USERNAME", "USER", "LOGNAME"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return sanitize_user(&v);
            }
        }
    }
    String::new()
}

fn sanitize_user(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_and_restore_roundtrip_bin_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        let album = music.join("Artists/A/Album");
        fs::create_dir_all(&album).unwrap();
        let file = album.join("stray.txt");
        fs::write(&file, b"x").unwrap();

        let trash = Trash::new(&music);
        let mut batch = trash.begin("test").unwrap();
        let e = batch.move_path(&file, "stray").unwrap();
        let dir = batch.finish().unwrap();
        assert!(!file.exists());
        assert!(e.trash_path.exists());
        // la-musica layout: flat in <user>/, manifest is .mlo_manifest.json
        assert!(dir.join(".mlo_manifest.json").exists());
        assert!(e.trash_path.parent() == Some(dir.as_path()));

        let restored = trash.restore(&dir).unwrap();
        assert_eq!(restored, vec![file.clone()]);
        assert!(file.exists());
    }

    #[test]
    fn collision_uses_numbered_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        fs::create_dir_all(&music).unwrap();
        let a = music.join("a.txt");
        fs::write(&a, b"1").unwrap();
        let trash = Trash::new(&music);
        let mut batch = trash.begin("x").unwrap();
        let e1 = batch.move_path(&a, "t").unwrap();
        fs::write(&a, b"2").unwrap();
        let e2 = batch.move_path(&a, "t").unwrap();
        batch.finish().unwrap();
        assert!(e1.trash_path.file_name().unwrap().to_string_lossy().starts_with("a"));
        assert!(e2.trash_path.to_string_lossy().contains("(2)"));
    }

    #[test]
    fn refuses_music_root_and_state() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        fs::create_dir_all(music.join(".mlo/data")).unwrap();
        let trash = Trash::new(&music);
        assert!(trash.begin("x").unwrap().move_path(&music, "no").is_err());
        assert!(trash.begin("x").unwrap().move_path(&music.join(".mlo/data"), "no").is_err());
    }
}
/// la-musica stores `origin` with `/`; convert back to the platform separator.
fn native_path(s: &str) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(s.replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(s)
    }
}
