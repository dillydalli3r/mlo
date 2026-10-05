//! Scan and index ([§4.1]) — one walk builds the index, with per-file
//! mtime+size reuse from the last walk. The same walk feeds the layout audit
//! ([§4.2]) and the grader ([§9]).

use crate::config::Config;
use crate::db::{Db, FileRow};
use crate::error::{IoResultExt, MloError, Result};
use crate::grade::{AlbumGrade, AlbumView, ArtistView, SidecarInfo, TrackView};
use crate::layout;
use crate::model::{
    ContainerKind, Finding, GradeReport, LibraryVerdict, Progress, Scope, TagMap,
};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const AUDIO_EXTS: &[&str] = &[
    "flac", "mp3", "m4a", "m4b", "m4p", "alac", "aac", "ogg", "oga", "opus", "wav", "wave", "aiff",
    "aif", "aifc", "wv", "ape", "dsf", "dff",
];
pub const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "m4v", "mov", "avi", "webm", "mpg", "mpeg", "ts", "vob"];
pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif", "jxl"];

/// Canonical album cover names (la-musica `grader.COVER_NAMES`).
pub const COVER_NAMES: &[&str] = &["cover.jpg", "cover.jpeg", "cover.png", "cover.jxl"];
/// Stems that name the album's cover art whatever the image extension
/// (la-musica `format_all._COVER_STEMS`).
pub const COVER_STEMS: &[&str] = &["cover", "front", "folder"];
/// Sidecar extensions inside an album (la-musica `_ALBUM_SIDECARS`).
pub const ALBUM_SIDECARS: &[&str] = &["lrc", "cue", "log", "accurip"];

#[derive(Debug, Clone)]
pub struct TrackEntry {
    pub path: PathBuf,
    pub container: ContainerKind,
    pub tags: TagMap,
    pub size: u64,
    pub mtime: i64,
    pub quick_hash: Option<String>,
    pub has_lyrics_sidecar: bool,
    pub lyrics_sidecar_synced: bool,
    pub grade: Option<GradeReport>,
}

impl TrackEntry {
    pub fn title(&self) -> String {
        self.tags
            .get("TITLE")
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_else(|| crate::model::file_name(&self.path))
    }
    pub fn track_no(&self) -> Option<u32> {
        self.tags
            .get("TRACKNUMBER")
            .and_then(|v| v.first())
            .and_then(|s| s.split('/').next())
            .and_then(|s| s.trim().parse().ok())
    }
}

#[derive(Debug, Clone)]
pub struct AlbumEntry {
    pub name: String,
    pub path: PathBuf,
    pub artist: String,
    pub tracks: Vec<TrackEntry>,
    pub side: SidecarInfo,
    pub findings: Vec<Finding>,
    pub is_cd: bool,
    pub audio_file_count: usize,
    pub grade: Option<GradeReport>,
}

impl AlbumEntry {
    pub fn title(&self) -> String {
        self.tracks
            .iter()
            .find_map(|t| t.tags.get("ALBUM").and_then(|v| v.first()).cloned())
            .unwrap_or_else(|| self.name.clone())
    }
    pub fn passed(&self) -> bool {
        self.grade.as_ref().map(|g| g.passed()).unwrap_or(true)
    }
    pub fn pct(&self) -> f64 {
        self.grade.as_ref().map(|g| g.pct()).unwrap_or(100.0)
    }
    pub fn duration_ms(&self) -> Option<u64> {
        let _ = &self.tracks;
        None
    }
}

#[derive(Debug, Clone)]
pub struct ArtistEntry {
    pub name: String,
    pub path: PathBuf,
    pub has_image: bool,
    pub has_description: bool,
    pub image_ok: Option<bool>,
    pub image_upscaled: bool,
    pub findings: Vec<Finding>,
    pub albums: Vec<AlbumEntry>,
    pub grade: Option<GradeReport>,
}

impl ArtistEntry {
    pub fn has_albums(&self) -> bool {
        !self.albums.is_empty()
    }
    pub fn passed(&self) -> bool {
        self.grade.as_ref().map(|g| g.passed()).unwrap_or(true)
    }
    pub fn track_count(&self) -> usize {
        self.albums.iter().map(|a| a.tracks.len()).sum()
    }
}

/// The whole library, in memory — what the TUI browses.
#[derive(Debug, Clone, Default)]
pub struct LibraryModel {
    pub music: PathBuf,
    pub artists: Vec<ArtistEntry>,
    pub library_findings: Vec<Finding>,
    pub verdict: LibraryVerdict,
    pub scanned_at: String,
}

impl LibraryModel {
    pub fn album_count(&self) -> usize {
        self.artists.iter().map(|a| a.albums.len()).sum()
    }
    pub fn track_count(&self) -> usize {
        self.artists.iter().map(|a| a.track_count()).sum()
    }
    pub fn find_album(&self, path: &Path) -> Option<&AlbumEntry> {
        self.artists.iter().flat_map(|a| a.albums.iter()).find(|al| al.path == path)
    }
    pub fn find_album_mut(&mut self, path: &Path) -> Option<&mut AlbumEntry> {
        self.artists
            .iter_mut()
            .flat_map(|a| a.albums.iter_mut())
            .find(|al| al.path == path)
    }
    pub fn find_track(&self, path: &Path) -> Option<(&ArtistEntry, &AlbumEntry, &TrackEntry)> {
        for a in &self.artists {
            for al in &a.albums {
                if let Some(t) = al.tracks.iter().find(|t| t.path == path) {
                    return Some((a, al, t));
                }
            }
        }
        None
    }
    pub fn all_findings(&self) -> Vec<&Finding> {
        let mut seen = std::collections::HashSet::new();
        let mut v: Vec<&Finding> = Vec::new();
        for f in self.library_findings.iter() {
            if seen.insert(f.id.clone()) {
                v.push(f);
            }
        }
        for a in &self.artists {
            for f in a.findings.iter() {
                if seen.insert(f.id.clone()) {
                    v.push(f);
                }
            }
            for al in &a.albums {
                for f in al.findings.iter() {
                    if seen.insert(f.id.clone()) {
                        v.push(f);
                    }
                }
            }
        }
        v
    }
    pub fn albums_flat(&self) -> Vec<&AlbumEntry> {
        self.artists.iter().flat_map(|a| a.albums.iter()).collect()
    }
}

/// The raw filesystem walk (dirs and files), excluding `.mlo`.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    pub music: PathBuf,
    pub dirs: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
}

impl Walk {
    pub fn is_audio(path: &Path) -> bool {
        ext_in(path, AUDIO_EXTS)
    }
    pub fn is_video(path: &Path) -> bool {
        ext_in(path, VIDEO_EXTS)
    }
    pub fn is_image(path: &Path) -> bool {
        ext_in(path, IMAGE_EXTS)
    }
}

pub fn ext_in(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| exts.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Walk `/Artists` and the root, skipping `.mlo`.
pub fn walk(music: &Path) -> Result<Walk> {
    let mut out = Walk { music: music.to_path_buf(), ..Default::default() };
    if !music.exists() {
        return Err(MloError::NotFound(format!("music folder {} does not exist", music.display())));
    }
    for entry in walkdir::WalkDir::new(music)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_state_dir(e.path()))
    {
        let entry = entry.map_err(|e| MloError::io(music, std::io::Error::other(e.to_string())))?;
        let p = entry.path().to_path_buf();
        if p == music {
            continue;
        }
        if entry.file_type().is_dir() {
            out.dirs.push(p);
        } else if entry.file_type().is_file() {
            out.files.push(p);
        }
    }
    out.dirs.sort();
    out.files.sort();
    Ok(out)
}

fn is_state_dir(path: &Path) -> bool {
    path.file_name().map(|n| n == ".mlo").unwrap_or(false)
}

pub fn quick_hash(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let meta = f.metadata().ok()?;
    let h = xxhash_rust::xxh3::xxh3_64(&buf);
    Some(format!("{h:016x}:{}", meta.len()))
}

fn mtime_secs(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Read a track entry (tags, sizes, sidecars) — the unit of pooled work.
/// `cached` carries tag maps already read from the index for unchanged files.
pub fn read_track(path: &Path, album: &Path, cached: &HashMap<PathBuf, TagMap>) -> Result<TrackEntry> {
    let mtime = mtime_secs(path);
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let container = crate::tags::detect_container(path);

    let eligible = matches!(
        container,
        ContainerKind::Flac
            | ContainerKind::OggVorbis
            | ContainerKind::Opus
            | ContainerKind::Mp3
            | ContainerKind::Mp4
            | ContainerKind::Wav
            | ContainerKind::Aiff
            | ContainerKind::Mkv
            | ContainerKind::Mp4Video
    );
    let (tags, quick) = if !eligible {
        (TagMap::new(), None)
    } else if let Some(t) = cached.get(path) {
        (t.clone(), None)
    } else {
        (crate::tags::read_tags(path).unwrap_or_default(), quick_hash(path))
    };

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let lrc = album.join(format!("{stem}.lrc"));
    let has_lyrics_sidecar = lrc.exists();
    let lyrics_sidecar_synced = if has_lyrics_sidecar {
        std::fs::read_to_string(&lrc)
            .map(|t| t.lines().any(|l| l.trim_start().starts_with('[') && l.contains(':')))
            .unwrap_or(false)
    } else {
        false
    };

    Ok(TrackEntry {
        path: path.to_path_buf(),
        container,
        tags,
        size,
        mtime,
        quick_hash: quick,
        has_lyrics_sidecar,
        lyrics_sidecar_synced,
        grade: None,
    })
}

/// Which files sit in an album folder and what sidecars exist.
fn sidecars_for(_album: &Path, files: &[PathBuf]) -> SidecarInfo {
    let mut side = SidecarInfo::default();
    let mut images: Vec<PathBuf> = Vec::new();
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let lower = name.to_ascii_lowercase();
        if ext_in(f, crate::scan::IMAGE_EXTS) {
            if is_cover_name(name) && side.cover.is_none() {
                side.cover = Some(f.clone());
                side.cover_dims = image_dims(f);
            } else {
                images.push(f.clone());
            }
        } else if lower == "description.txt" {
            side.description = std::fs::read_to_string(f).ok();
        } else if lower == ".mlo_expected.json" {
            side.expected_tracks = parse_expected_tracks(f);
        } else if lower.ends_with(".cue") {
            side.cue = Some(f.clone());
        } else if lower.ends_with(".log") {
            side.log = Some(f.clone());
        } else if lower.ends_with(".accurip") {
            side.accurip = Some(f.clone());
        }
    }
    if side.cover.is_none() {
        if let Some(first) = images.first() {
            side.cover = Some(first.clone());
            side.cover_dims = image_dims(first);
        }
    }
    side.extra_images = images
        .iter()
        .filter(|p| Some(*p) != side.cover.as_ref())
        .count() as u32;
    if let Some((w, h)) = side.cover_dims {
        side.cover_aspect_ok = Some(aspect_ok(w, h, 1.0, 0.02));
    }
    side
}

fn parse_expected_tracks(path: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(arr) = v.as_array() {
        return Some(arr.len());
    }
    if let Some(arr) = v.get("tracks").and_then(|t| t.as_array()) {
        return Some(arr.len());
    }
    None
}

fn image_dims(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

pub fn aspect_ok(w: u32, h: u32, target: f32, tol: f32) -> bool {
    if h == 0 {
        return false;
    }
    let ratio = w as f32 / h as f32;
    (ratio - target).abs() <= tol
}

/// Is a directory a disc folder (CD1 / Disc 2 / cd_1 ...)?
pub fn is_disc_folder(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    let l = l.trim();
    if let Some(rest) = l.strip_prefix("disc") {
        return rest.trim_start_matches([' ', '_', '-']).chars().all(|c| c.is_ascii_digit());
    }
    if let Some(rest) = l.strip_prefix("cd") {
        let rest = rest.trim_start_matches([' ', '_', '-']);
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    false
}

/// Known sidecar extensions inside an album (a bare `.txt` is not a sidecar —
/// only `description.txt` is, checked separately).
pub fn is_sidecar_ext(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()) {
        Some(ext) => {
            ALBUM_SIDECARS.contains(&ext.as_str()) || IMAGE_EXTS.contains(&ext.as_str())
        }
        None => false,
    }
}

/// Whether `name` is the album's cover art: a canonical cover name
/// (`cover.jpg|jpeg|png|jxl`), or a `cover`/`front`/`folder` stem with any
/// image extension (la-musica `COVER_NAMES` + `_COVER_STEMS`).
pub fn is_cover_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if COVER_NAMES.contains(&lower.as_str()) {
        return true;
    }
    match lower.rsplit_once('.') {
        Some((stem, ext)) => COVER_STEMS.contains(&stem) && IMAGE_EXTS.contains(&ext),
        None => false,
    }
}

/// A numbered copy of a sidecar, e.g. `description (2).txt`.
pub fn is_numbered_sidecar(name: &str) -> bool {
    let stem = match name.rsplit_once('.') {
        Some((s, _)) => s,
        None => name,
    };
    let Some(idx) = stem.rfind('(') else { return false };
    stem[idx..].starts_with('(') && stem.ends_with(')') && stem[idx + 1..stem.len() - 1].trim().chars().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Scan driver
// ---------------------------------------------------------------------------

pub struct ScanOptions {
    pub scope: Scope,
    pub store_report: bool,
}

/// Full scan: walk, index, layout audit, grade. Writes the layout report only
/// for a library-wide scan ([§4.2] rule 3).
pub fn scan(
    cfg: &Config,
    db: &Db,
    scope: &Scope,
    cancel: &AtomicBool,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<LibraryModel> {
    let music = cfg.music_folder.clone();
    progress(Progress { label: "walking library".into(), current: 0, total: None, note: None });
    let walked = walk(&music)?;

    // Group files by their containing directory.
    let mut by_dir: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for f in &walked.files {
        if let Some(parent) = f.parent() {
            by_dir.entry(parent.to_path_buf()).or_default().push(f.clone());
        }
    }

    // Album folders: any dir under Artists/<artist>/ whose subtree holds audio.
    let artists_root = music.join("Artists");
    let mut artist_dirs: Vec<PathBuf> = Vec::new();
    if artists_root.is_dir() {
        for d in std::fs::read_dir(&artists_root).at(&artists_root)?.flatten() {
            if d.path().is_dir() {
                artist_dirs.push(d.path());
            }
        }
    }
    artist_dirs.sort();

    // Collect album dirs = any dir whose files include audio, directly under an
    // artist dir (plus disc subfolders merged into the album).
    let mut album_dirs: Vec<(PathBuf, PathBuf)> = Vec::new(); // (artist_dir, album_dir)
    for artist in &artist_dirs {
        let mut subs: Vec<PathBuf> = std::fs::read_dir(artist)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        subs.sort();
        for sub in subs {
            album_dirs.push((artist.clone(), sub));
        }
    }

    // Scoped runs confine the model to the named targets.
    let in_scope = |p: &Path| scope_contains(scope, p);

    // Read tracks in parallel across albums.
    progress(Progress {
        label: "reading tags".into(),
        current: 0,
        total: Some(album_dirs.len() as u64),
        note: None,
    });
    let counter = std::sync::atomic::AtomicU64::new(0);
    let album_work: Vec<(PathBuf, PathBuf, Vec<PathBuf>)> = album_dirs
        .iter()
        .filter(|(_, album)| in_scope(album))
        .map(|(artist, album)| {
            let mut files = collect_album_files(album, &walked);
            files.sort();
            (artist.clone(), album.clone(), files)
        })
        .collect();

    // Reuse index tags for files whose mtime+size are unchanged ([§4.1]); this
    // pre-pass is serial because the SQLite connection is not `Sync`.
    let mut cached: HashMap<PathBuf, TagMap> = HashMap::new();
    for (_, _, files) in &album_work {
        for f in files.iter().filter(|f| Walk::is_audio(f)) {
            let mtime = mtime_secs(f);
            let size = std::fs::metadata(f).map(|m| m.len() as i64).unwrap_or(0);
            if db.cached_unchanged(f, mtime, size).unwrap_or(false) {
                if let Ok(t) = db.get_tags(f) {
                    cached.insert(f.clone(), t);
                }
            }
        }
    }

    let track_results: Vec<(PathBuf, PathBuf, Vec<PathBuf>, Vec<Result<TrackEntry>>)> = album_work
        .par_iter()
        .map(|(artist, album, files)| {
            let entries: Vec<Result<TrackEntry>> = files
                .iter()
                .filter(|f| Walk::is_audio(f) || Walk::is_video(f))
                .map(|f| read_track(f, album, &cached))
                .collect();
            let done = counter.fetch_add(1, Ordering::Relaxed) + 1;
            progress(Progress {
                label: "reading tags".into(),
                current: done,
                total: Some(album_work.len() as u64),
                note: Some(crate::model::file_name(album)),
            });
            (artist.clone(), album.clone(), files.clone(), entries)
        })
        .collect();

    if cancel.load(Ordering::Relaxed) {
        return Err(MloError::Other("scan cancelled".into()));
    }

    // Expected canonical paths per album (for wrong_case + naming).
    let mut expected_by_album: HashMap<PathBuf, String> = HashMap::new();
    let mut album_views: Vec<(PathBuf, AlbumView, Vec<TrackEntry>)> = Vec::new();
    for (artist, album, files, entries) in &track_results {
        let mut tracks: Vec<TrackEntry> = Vec::new();
        let mut errors = Vec::new();
        for e in entries {
            match e {
                Ok(t) => tracks.push(t.clone()),
                Err(err) => errors.push(err.to_string()),
            }
        }
        let side = sidecars_for(album, files);
        let is_cd = side.cue.is_some() || side.log.is_some() || files.iter().any(|f| f.parent().map(|p| p != album).unwrap_or(false));
        let album_tags = merged_album_tags(&tracks);
        let opts = crate::naming::NamingOptions { short_folder_names: cfg.short_folder_names };
        let relative = album
            .strip_prefix(&music)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let _ = relative;
        if let Ok(res) = crate::naming::evaluate(&cfg.naming_script, &album_tags, opts) {
            // `path` is the DIRECTORY relative path (the template's last
            // segment is the file name), so wrong_case compares folder to folder.
            let expected_rel = {
                let mut comps: Vec<String> =
                    res.path.split('/').map(|s| s.to_string()).collect();
                if comps.first().map(|s| s.eq_ignore_ascii_case("Artists")).unwrap_or(false) {
                    comps.remove(0);
                }
                comps.join("/")
            };
            expected_by_album.insert(album.clone(), expected_rel);
        }
        let audio_file_count = files.iter().filter(|f| Walk::is_audio(f)).count();
        let view = AlbumView {
            path: album.clone(),
            artist: artist.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            title: album_tags.get("ALBUM").and_then(|v| v.first()).cloned().unwrap_or_else(|| crate::model::file_name(album)),
            tracks: tracks.iter().map(to_track_view).collect(),
            side: side.clone(),
            findings: Vec::new(),
            is_cd,
            audio_file_count,
        };
        album_views.push((artist.clone(), view, tracks));
    }

    // Layout findings from the raw walk.
    progress(Progress { label: "auditing layout".into(), current: 0, total: None, note: None });
    let findings = layout::analyze(&walked, &music, &expected_by_album, cfg);
    let findings_by_album = group_findings(&findings);
    let library_findings: Vec<Finding> = findings.iter().filter(|f| f.library_wide).cloned().collect();
    let findings_by_artist = group_findings_artist(&findings);

    // Follow the layout report only for a library-wide scan ([§4.2] rule 3).
    // A scoped run stores nothing: a partial scan is not "the last scan".
    if matches!(scope, Scope::Library) {
        let audio_files = walked.files.iter().filter(|f| Walk::is_audio(f)).count() as u64;
        let artists = artist_dirs
            .iter()
            .filter(|d| !crate::model::file_name(d).starts_with('.'))
            .count() as u32;
        let body = layout::layout_report_body(
            &findings,
            &music,
            album_views.len() as u32,
            artists,
            audio_files,
        );
        let doc = crate::model::LayoutReportDoc {
            scanned_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            music_folder: music.to_string_lossy().replace('\\', "/"),
            report: body,
        };
        let _ = crate::atomic::write_atomic_json(cfg.layout_report(), &doc);
    }

    // Build artist entries + grade.
    let mut artists: Vec<ArtistEntry> = Vec::new();
    let mut album_reports: Vec<(PathBuf, GradeReport)> = Vec::new();
    let mut artist_reports: Vec<(String, GradeReport)> = Vec::new();

    for artist_dir in &artist_dirs {
        if !in_scope(artist_dir) {
            continue;
        }
        let name = crate::model::file_name(artist_dir);
        let (has_image, image_ok, has_description) = artist_artifacts(artist_dir);
        let mut entry = ArtistEntry {
            name: name.clone(),
            path: artist_dir.clone(),
            has_image,
            has_description,
            image_ok,
            image_upscaled: false,
            findings: findings_by_artist.get(artist_dir).cloned().unwrap_or_default(),
            albums: Vec::new(),
            grade: None,
        };

        for (_a_artist, view_ref, tracks) in album_views.iter().filter(|(a, _, _)| a == artist_dir) {
            let mut view = view_ref.clone();
            view.findings = findings_by_album.get(&view.path).cloned().unwrap_or_default();
            let AlbumGrade { report, track_reports } = crate::grade::grade_album(&view, cfg);
            let mut track_map: HashMap<PathBuf, GradeReport> = track_reports.into_iter().collect();
            let mut track_entries: Vec<TrackEntry> = tracks.clone();
            for t in track_entries.iter_mut() {
                t.grade = track_map.remove(&t.path);
                if let Err(e) = index_track(db, t, &view.artist, &view.path) {
                    tracing::warn!(error = %e, "index write failed");
                }
            }
            let name = crate::model::file_name(&view.path);
            let _ = db.upsert_album(
                &format!("{}::{}", view.artist, name),
                &view.artist,
                &view.title,
                &view.path,
                Some(report.pct()),
                Some(report.passed()),
            );
            album_reports.push((view.path.clone(), report.clone()));
            entry.albums.push(AlbumEntry {
                name,
                path: view.path.clone(),
                artist: view.artist.clone(),
                tracks: track_entries,
                side: view.side.clone(),
                findings: view.findings.clone(),
                is_cd: view.is_cd,
                audio_file_count: view.audio_file_count,
                grade: Some(report),
            });
        }

        // Artist grade.
        let av = ArtistView {
            name: name.clone(),
            path: artist_dir.clone(),
            has_albums: !entry.albums.is_empty(),
            has_image: entry.has_image,
            image_ok: entry.image_ok,
            image_upscaled: entry.image_upscaled,
            has_description: entry.has_description,
            findings: entry.findings.clone(),
        };
        let areport = crate::grade::grade_artist(&av, cfg);
        let _ = db.upsert_artist(
            &name,
            artist_dir,
            Some(areport.passed()),
            entry.has_image,
            entry.has_description,
        );
        artist_reports.push((name.clone(), areport.clone()));
        entry.grade = Some(areport);
        artists.push(entry);
    }

    // Library row: library-wide findings.
    let library_row = crate::grade::grade_library(&findings, cfg);
    let audit_failed = artists
        .iter()
        .flat_map(|a| a.albums.iter())
        .filter(|al| {
            al.grade
                .as_ref()
                .map(|g| g.issues().any(|(_, i)| i.code == "AUDIT_FAKE"))
                .unwrap_or(false)
        })
        .count() as u32;
    let tracks_total = artists.iter().map(|a| a.track_count()).sum::<usize>() as u32;
    let verdict = crate::grade::library_verdict(
        &album_reports,
        &artist_reports,
        tracks_total,
        audit_failed,
        Some(library_row.clone()),
    );

    Ok(LibraryModel {
        music,
        artists,
        library_findings,
        verdict,
        scanned_at: chrono::Local::now().to_rfc3339(),
    })
}

/// The stored layout report, or `None` when no scan ran yet or the file is
/// unreadable (la-musica `load_report`; a warning must never appear for a
/// scan that did not happen).
pub fn load_report(cfg: &Config) -> Option<crate::model::LayoutReportDoc> {
    let text = std::fs::read_to_string(cfg.layout_report()).ok()?;
    serde_json::from_str(&text).ok()
}

fn index_track(db: &Db, t: &TrackEntry, artist: &str, album: &Path) -> Result<()> {
    let row = FileRow {
        path: t.path.clone(),
        kind: if Walk::is_video(&t.path) { "video".into() } else { "audio".into() },
        artist: Some(artist.to_string()),
        album: t.tags.get("ALBUM").and_then(|v| v.first()).cloned().or_else(|| {
            album.file_name().map(|s| s.to_string_lossy().into_owned())
        }),
        track: t.tags.get("TRACKNUMBER").and_then(|v| v.first()).cloned(),
        disc: t.tags.get("DISCNUMBER").and_then(|v| v.first()).cloned(),
        container: Some(t.container),
        duration_ms: None,
        mtime: t.mtime,
        size: t.size as i64,
        quick_hash: t.quick_hash.clone(),
        has_cover: false,
        has_lyrics: t.has_lyrics_sidecar,
        has_cue: false,
        has_log: false,
        has_accurip: false,
        grade_pct: t.grade.as_ref().map(|g| g.pct()),
        grade_pass: t.grade.as_ref().map(|g| g.passed()),
    };
    db.upsert_file(&row)?;
    db.set_tags(&t.path, &t.tags)?;
    Ok(())
}

/// Merge album-level tags from tracks (first non-empty value wins).
pub fn merged_album_tags(tracks: &[TrackEntry]) -> TagMap {
    let mut out = TagMap::new();
    for t in tracks {
        for (k, v) in &t.tags {
            if k == "TITLE" || k == "TRACKNUMBER" {
                continue;
            }
            out.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    out
}

fn to_track_view(t: &TrackEntry) -> TrackView {
    TrackView {
        path: t.path.clone(),
        tags: t.tags.clone(),
        is_video: Walk::is_video(&t.path),
        decoded_ok: true,
        has_lyrics_sidecar: t.has_lyrics_sidecar,
        lyrics_sidecar_synced: t.lyrics_sidecar_synced,
        flac_md5_ok: None,
    }
}

/// All files beneath an album directory (including disc subfolders).
pub fn collect_album_files(album: &Path, walked: &Walk) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = walked
        .files
        .iter()
        .filter(|f| f.parent() == Some(album))
        .cloned()
        .collect();
    for d in walked.dirs.iter().filter(|d| d.parent() == Some(album)) {
        out.extend(walked.files.iter().filter(|f| f.parent() == Some(d.as_path())).cloned());
    }
    out
}

pub fn artist_artifacts(artist: &Path) -> (bool, Option<bool>, bool) {
    let mut has_image = false;
    let mut image_ok = None;
    let mut has_description = false;
    if let Ok(rd) = std::fs::read_dir(artist) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "artist.jpg" || name == "artist.png" {
                has_image = true;
                if let Ok((w, h)) = image::image_dimensions(e.path()) {
                    image_ok = Some(aspect_ok(w, h, 1.0, 0.02) && w.max(h) <= crate::naming::ARTIST_IMAGE_CEILING);
                } else {
                    image_ok = Some(false);
                }
            } else if name == "description.txt" {
                has_description = std::fs::read_to_string(e.path())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false);
            }
        }
    }
    (has_image, image_ok, has_description)
}

fn scope_contains(scope: &Scope, dir: &Path) -> bool {
    match scope {
        Scope::Library => true,
        // an artist dir is in scope when it is the target, contains it, or is
        // contained by it (a scoped album pass must still find its artist)
        Scope::Artist(p) | Scope::Album(p) => dir == p || dir.starts_with(p) || p.starts_with(dir),
        Scope::Track(p) => dir == p.parent().unwrap_or(dir) || p.starts_with(dir),
        Scope::Selection(v) => v.iter().any(|p| dir == p || dir.starts_with(p) || p.starts_with(dir)),
    }
}

fn group_findings(findings: &[Finding]) -> HashMap<PathBuf, Vec<Finding>> {
    let mut map: HashMap<PathBuf, Vec<Finding>> = HashMap::new();
    for f in findings {
        if let Some(album) = &f.album {
            map.entry(album.clone()).or_default().push(f.clone());
        }
    }
    map
}

fn group_findings_artist(findings: &[Finding]) -> HashMap<PathBuf, Vec<Finding>> {
    let mut map: HashMap<PathBuf, Vec<Finding>> = HashMap::new();
    for f in findings {
        if let Some(artist) = &f.artist {
            map.entry(artist.clone()).or_default().push(f.clone());
        }
    }
    map
}

/// Consumers that need the sidecar vocabulary.
pub fn sidecar_kind(name: &str) -> Option<&'static str> {
    let l = name.to_ascii_lowercase();
    if l == "description.txt" {
        Some("description")
    } else if l == ".mlo_expected.json" {
        Some("expected")
    } else if l.ends_with(".lrc") {
        Some("lyrics")
    } else if l.ends_with(".cue") {
        Some("cue")
    } else if l.ends_with(".log") {
        Some("log")
    } else if l.ends_with(".accurip") {
        Some("accurip")
    } else if l.starts_with("cover.") || l.starts_with("folder.") {
        Some("cover")
    } else {
        None
    }
}

/// Sorted, deduplicated list of extensions the library contains (for Tools/status).
pub fn library_extensions(walked: &Walk) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();
    for f in &walked.files {
        if let Some(e) = f.extension().and_then(|e| e.to_str()) {
            set.insert(e.to_ascii_lowercase());
        }
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

/// A stable map used by the TUI for filtering.
pub fn tag_values(model: &LibraryModel, key: &str) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for a in &model.artists {
        for al in &a.albums {
            for t in &al.tracks {
                if let Some(vals) = t.tags.get(key) {
                    for v in vals {
                        *counts.entry(v.clone()).or_default() += 1;
                    }
                }
            }
        }
    }
    counts
}
/// Load a browsable model straight from the index — instant startup, no walk.
/// Grades come from the stored columns; findings and fresh grades require a scan.
pub fn load_from_db(cfg: &Config, db: &Db) -> Result<LibraryModel> {
    use std::collections::BTreeMap;
    let rows = db.all_files()?;
    let mut artists: BTreeMap<String, ArtistEntry> = BTreeMap::new();

    for row in rows.iter().filter(|r| r.kind == "audio" || r.kind == "video") {
        let Some(album_path) = row.path.parent().map(|p| p.to_path_buf()) else { continue };
        let artist_path = album_path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| cfg.music_folder.clone());
        let artist_name = crate::model::file_name(&artist_path);
        let album_name = crate::model::file_name(&album_path);
        let tags = db.get_tags(&row.path).unwrap_or_default();

        let artist = artists.entry(artist_name.clone()).or_insert_with(|| ArtistEntry {
            name: artist_name.clone(),
            path: artist_path.clone(),
            has_image: false,
            has_description: false,
            image_ok: None,
            image_upscaled: false,
            findings: Vec::new(),
            albums: Vec::new(),
            grade: None,
        });
        let album = match artist.albums.iter_mut().find(|a| a.path == album_path) {
            Some(a) => a,
            None => {
                artist.albums.push(AlbumEntry {
                    name: album_name.clone(),
                    path: album_path.clone(),
                    artist: artist_name.clone(),
                    tracks: Vec::new(),
                    side: SidecarInfo::default(),
                    findings: Vec::new(),
                    is_cd: false,
                    audio_file_count: 0,
                    grade: None,
                });
                artist.albums.last_mut().unwrap()
            }
        };
        album.tracks.push(TrackEntry {
            path: row.path.clone(),
            container: row.container.unwrap_or(ContainerKind::Unknown),
            tags,
            size: row.size as u64,
            mtime: row.mtime,
            quick_hash: row.quick_hash.clone(),
            has_lyrics_sidecar: row.has_lyrics,
            lyrics_sidecar_synced: false,
            grade: row.grade_pass.map(|p| cached_grade(p, row.grade_pct.unwrap_or(100.0))),
        });
        album.audio_file_count += 1;
        if let Some(p) = row.grade_pass {
            album.grade = Some(cached_grade(p, row.grade_pct.unwrap_or(100.0)));
        }
    }

    let mut list: Vec<ArtistEntry> = artists.into_values().collect();
    for a in &mut list {
        a.albums.sort_by(|x, y| x.path.cmp(&y.path));
        for al in &mut a.albums {
            al.tracks.sort_by(|x, y| {
                x.track_no().cmp(&y.track_no()).then_with(|| x.path.cmp(&y.path))
            });
        }
    }
    let album_reports: Vec<(PathBuf, GradeReport)> = list
        .iter()
        .flat_map(|a| a.albums.iter())
        .filter_map(|al| al.grade.clone().map(|g| (al.path.clone(), g)))
        .collect();
    let artists_reports: Vec<(String, GradeReport)> = list
        .iter()
        .filter_map(|a| a.grade.clone().map(|g| (a.name.clone(), g)))
        .collect();
    let tracks_total = list.iter().map(|a| a.track_count()).sum::<usize>() as u32;
    let verdict = crate::grade::library_verdict(&album_reports, &artists_reports, tracks_total, 0, None);

    Ok(LibraryModel {
        music: cfg.music_folder.clone(),
        artists: list,
        library_findings: Vec::new(),
        verdict,
        scanned_at: chrono::Local::now().to_rfc3339(),
    })
}

/// A one-check placeholder report so an album dot reflects the stored grade.
fn cached_grade(pass: bool, pct: f64) -> GradeReport {
    use crate::model::{CheckResult, CheckStatus, Issue};
    let label = format!("cached grade ({pct:.0}%) — press g to re-grade");
    let status = if pass { CheckStatus::Pass } else { CheckStatus::Fail(Issue { code: "CACHED_FAIL".into(), message: label.clone() }) };
    GradeReport::new("cached", vec![CheckResult { key: "cached".into(), label, status }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn library_scan_writes_report_document_and_scoped_stores_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        let album = music.join("Artists").join("A").join("Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("01.flac"), b"x").unwrap();
        std::fs::write(album.join("stray.nfo"), b"x").unwrap();

        let mut cfg = Config::default();
        cfg.music_folder = music.clone();
        cfg.ensure_dirs().unwrap();
        let db = Db::open(&cfg.index_db()).unwrap();
        let cancel = AtomicBool::new(false);
        let progress = |_p: Progress| {};

        scan(&cfg, &db, &Scope::Album(album.clone()), &cancel, &progress).unwrap();
        assert!(!cfg.layout_report().exists(), "scoped run stores nothing");

        scan(&cfg, &db, &Scope::Library, &cancel, &progress).unwrap();
        let doc = load_report(&cfg).expect("library scan wrote a report");
        assert_eq!(doc.music_folder, music.to_string_lossy().replace('\\', "/"));
        assert!(doc.scanned_at.ends_with('Z'), "UTC timestamp: {}", doc.scanned_at);

        let text = std::fs::read_to_string(cfg.layout_report()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 3, "top-level keys are exactly three: {obj:?}");
        for k in ["scanned_at", "music_folder", "report"] {
            assert!(obj.contains_key(k), "missing {k}");
        }
        let rep = obj["report"].as_object().unwrap();
        assert_eq!(rep.len(), 9, "report keys are exactly nine: {rep:?}");
        for k in [
            "folder", "artists_dir", "exists", "issues", "counts", "total", "albums", "artists",
            "audio_files",
        ] {
            assert!(rep.contains_key(k), "missing report key {k}");
        }
    }
}
