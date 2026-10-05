//! Library layout audit and fixes ([§4.2], [§4.3]).
//!
//! One scan answers the TUI panel, the CLI and the stored report. Every removal
//! re-derives its reason at the moment of the move ([§4.2] rule 1).

use crate::config::Config;
use crate::error::{IoResultExt, MloError, Result};
use crate::model::{Finding, FindingKind, FixKind, LayoutFixRow, LayoutIssueRow, LayoutReportBody};
use crate::scan::{self, Walk};
use crate::trash::Trash;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Analyze a walk and produce every finding.
pub fn analyze(
    walked: &Walk,
    music: &Path,
    expected_by_album: &HashMap<PathBuf, String>,
    cfg: &Config,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let artists_root = music.join("Artists");

    // Root-level entries.
    let root_dirs: Vec<&PathBuf> = walked
        .dirs
        .iter()
        .filter(|d| d.parent() == Some(music))
        .collect();
    for d in &root_dirs {
        let name = crate::model::file_name(d);
        if name == ".mlo" || name.starts_with('.') {
            continue; // .mlo_data and other dot-dirs are handled elsewhere
        }
        if name.eq_ignore_ascii_case("Artists") {
            continue;
        }
        findings.push(root_folder_finding(d, walked));
    }
    for f in walked.files.iter().filter(|f| f.parent() == Some(music)) {
        if Walk::is_audio(f) {
            let mut finding = Finding::new(FindingKind::AudioAtRoot, f, "audio file directly in the library root")
                .library_wide()
                .with_hint(
                    "move it into Artists/<Artist>/<Album>/ (or import it) so grading and the organizer can see it",
                );
            match album_dir_for(f, cfg, music) {
                Some(target) => finding.fix = FixKind::Move { target },
                None => {
                    finding.detail = "audio file directly in the library root — Missing tag for the album path".into();
                    finding.fix = FixKind::None;
                }
            }
            findings.push(finding);
        }
    }

    // Artists/ level.
    if artists_root.is_dir() {
        for f in walked.files.iter().filter(|f| f.parent() == Some(artists_root.as_path())) {
            let kind = if Walk::is_audio(f) { FindingKind::AudioInArtists } else { FindingKind::StrayInArtists };
            let hint = if Walk::is_audio(f) {
                "move it into Artists/<Artist>/<Album>/"
            } else {
                "nothing here can name the album it belongs to — Apply fixes moves it to the Trash, which lists it and can put it back"
            };
            findings.push(Finding::new(kind, f, "file directly in Artists/")
                .library_wide()
                .with_hint(hint)
                .with_fix(FixKind::Trash {
                    reason: kind.code().to_string(),
                }));
        }
    }

    // Artist dirs.
    let artist_dirs: Vec<PathBuf> = walked
        .dirs
        .iter()
        .filter(|d| d.parent() == Some(artists_root.as_path()))
        .cloned()
        .collect();

    for artist in &artist_dirs {
        let name = crate::model::file_name(artist);
        if name.starts_with('.') {
            findings.push(Finding::new(FindingKind::HiddenFolder, artist, "hidden folder inside Artists/")
                .with_hint("hidden folders are not library content — move or delete it"));
        }
        let album_dirs: Vec<&PathBuf> = walked.dirs.iter().filter(|d| d.parent() == Some(artist.as_path())).collect();
        for f in walked.files.iter().filter(|f| f.parent() == Some(artist.as_path())) {
            if Walk::is_audio(f) {
                let mut finding = Finding::new(FindingKind::AudioInArtist, f, "audio directly in the artist folder")
                    .on_artist(artist)
                    .with_hint("give it an album folder: Artists/<Artist>/<Album>/");
                match album_dir_for(f, cfg, music) {
                    Some(target) => finding.fix = FixKind::Move { target },
                    None => {
                        finding.detail = "audio directly in the artist folder — Missing tag for the album path".into();
                    }
                }
                findings.push(finding);
            }
        }
        if album_dirs.is_empty() && !name.starts_with('.') {
            // empty_artist does NOT fire when a sibling names the same artist
            let has_sibling = artist_dirs.iter().any(|o| o != artist && same_artist(&name, &crate::model::file_name(o)));
            if !has_sibling {
                findings.push(
                    Finding::new(FindingKind::EmptyArtist, artist, "artist folder with no album folders")
                        .library_wide()
                        .with_hint(
                            "remove it to the Trash (Optimize → Library layout → remove), or put one of the artist's albums inside it",
                        )
                        .with_fix(FixKind::Trash { reason: "empty_artist".into() }),
                );
            }
        }
    }

    // split_artist
    for i in 0..artist_dirs.len() {
        for j in (i + 1)..artist_dirs.len() {
            let a = &artist_dirs[i];
            let b = &artist_dirs[j];
            let na = crate::model::file_name(a);
            let nb = crate::model::file_name(b);
            if !same_artist(&na, &nb) {
                continue;
            }
            let a_has = holds_albums(a, walked);
            let b_has = holds_albums(b, walked);
            let mut f = Finding::new(
                FindingKind::SplitArtist,
                a,
                format!("two folders name one artist: '{}' and '{}'", na, nb),
            )
            .on_artist(a.clone())
            .library_wide();
            if a_has && b_has {
                // both hold albums: report only
                f.fix = FixKind::None;
                f.hint = "both folders hold albums — which one is the real artist is the user's call".into();
            } else {
                let into = if a_has { a.clone() } else if b_has { b.clone() } else { mbid_named(a, b) };
                let from = if into == *a { b.clone() } else { a.clone() };
                f.fix = FixKind::MergeArtist { into: into.clone() };
                f.path = from;
                f.abs = std::path::absolute(&f.path).unwrap_or_else(|_| f.path.clone());
                f.detail = format!("merge '{}' into '{}'", crate::model::file_name(&f.path), crate::model::file_name(&into));
                f.hint = format!("two folders name one artist — Apply fixes merges '{}' into '{}'", crate::model::file_name(&f.path), crate::model::file_name(&into));
            }
            findings.push(f);
        }
    }

    // Album dirs.
    for artist in &artist_dirs {
        for album in walked.dirs.iter().filter(|d| d.parent() == Some(artist.as_path())) {
            let album_name = crate::model::file_name(album);
            if album_name.starts_with('.') {
                continue;
            }
            let audio_beneath = audio_anywhere(album, walked);
            if !audio_beneath {
                findings.push(
                    Finding::new(FindingKind::EmptyAlbum, album, "album folder holds no audio")
                        .on_album(album.clone()).on_artist(artist.clone())
                        .with_hint(
                            "an empty album grades as an error — Apply fixes moves the folder to the Trash, so put the album in it first if it is one you are still filling",
                        )
                        .with_fix(FixKind::Trash { reason: "empty_album".into() }),
                );
                continue;
            }

            // files directly in the album
            for f in walked.files.iter().filter(|f| f.parent() == Some(album.as_path())) {
                let name = crate::model::file_name(f);
                if scan::is_numbered_sidecar(&name) {
                    let canonical = canonical_sidecar_name(&name);
                    let canonical_path = album.join(&canonical);
                    let duplicate = canonical_path.exists();
                    let fix = if duplicate {
                        FixKind::Trash { reason: "sidecar_copy".into() }
                    } else {
                        FixKind::Rename { target: canonical_path }
                    };
                    let hint = if duplicate {
                        format!("a duplicate of the sidecar the app already reads — nothing reads '{name}', and Apply fixes moves it to the Trash")
                    } else {
                        format!("rename it to '{canonical}' — the app reads either name, and Apply fixes renames without touching the text")
                    };
                    findings.push(
                        Finding::new(FindingKind::SidecarCopy, f, format!("numbered sidecar copy: {name}"))
                            .on_album(album.clone()).on_artist(artist.clone())
                            .with_hint(hint)
                            .with_fix(fix),
                    );
                } else if !Walk::is_audio(f)
                    && !Walk::is_video(f)
                    && !scan::is_sidecar_ext(f)
                    && !name.starts_with('.')
                    && !name.eq_ignore_ascii_case("description.txt")
                {
                    findings.push(
                        Finding::new(FindingKind::StrayFile, f, format!("stray file in album: {name}"))
                            .on_album(album.clone()).on_artist(artist.clone())
                            .with_hint(
                                "dead weight in the library (an nfo, a db, a stray text file) — Apply fixes moves it to the Trash, which lists it and can put it back",
                            )
                            .with_fix(FixKind::Trash { reason: "stray_file".into() }),
                    );
                }
            }

            // unexpected subfolders
            for sub in walked.dirs.iter().filter(|d| d.parent() == Some(album.as_path())) {
                let sn = crate::model::file_name(sub);
                if sn == "VIDEO_TS" || sn == "BDMV" || sn.starts_with('.') {
                    continue;
                }
                if scan::is_disc_folder(&sn) {
                    continue;
                }
                if !audio_anywhere(sub, walked) {
                    findings.push(
                        Finding::new(FindingKind::UnexpectedSubfolder, sub, format!("unexpected subfolder: {sn}"))
                            .on_album(album.clone()).on_artist(artist.clone())
                            .with_hint("only disc folders (CD1, Disc 2, …) belong inside an album — Apply fixes moves it to the Trash")
                            .with_fix(FixKind::Trash { reason: "unexpected_subfolder".into() }),
                    );
                }
            }

            // wrong case vs the naming template
            if let Some(expected) = expected_by_album.get(album) {
                let actual = album
                    .strip_prefix(music)
                    .map(|p| strip_artists(&p.to_string_lossy().replace('\\', "/")))
                    .unwrap_or_default();
                if actual != *expected && actual.eq_ignore_ascii_case(expected) {
                    findings.push(
                        Finding::new(FindingKind::WrongCase, album, format!("'{actual}' should be '{expected}'"))
                            .on_album(album.clone()).on_artist(artist.clone())
                            .with_hint("run Organize — it rewrites this to the script's exact casing; Apply fixes renames the name itself")
                            .with_fix(FixKind::Rename { target: music.join("Artists").join(expected) }),
                    );
                }
            }
        }
    }

    // legacy .mlo_data leftovers
    for d in &walked.dirs {
        if crate::model::file_name(d) == ".mlo_data" {
            findings.push(
                Finding::new(FindingKind::LegacyStateFile, d, "leftover of the old .mlo_data layout")
                    .library_wide()
                    .with_hint("the app does not read it any more — Apply fixes moves it to the Trash, which can put it back")
                    .with_fix(FixKind::Trash { reason: "legacy_state_file".into() }),
            );
        }
    }
    for f in &walked.files {
        if crate::model::file_name(f) == ".mlo_data" {
            findings.push(
                Finding::new(FindingKind::LegacyStateFile, f, "leftover of the old .mlo_data layout")
                    .library_wide()
                    .with_hint("the app does not read it any more — Apply fixes moves it to the Trash, which can put it back")
                    .with_fix(FixKind::Trash { reason: "legacy_state_file".into() }),
            );
        }
    }

    let _ = cfg;
    findings
}

/// The canonical album directory a loose file belongs in, derived from its tags
/// via the naming template. `None` when a tag needed for the path is missing —
/// the path is never invented ([§3.2] rule 5).
fn album_dir_for(file: &Path, cfg: &Config, music: &Path) -> Option<PathBuf> {
    let tags = crate::tags::read_tags(file).ok()?;
    let opts = crate::naming::NamingOptions { short_folder_names: cfg.short_folder_names };
    let res = crate::naming::evaluate(&cfg.naming_script, &tags, opts).ok()?;
    if res.has_missing() || res.path.is_empty() {
        return None;
    }
    Some(music.join("Artists").join(res.path))
}

fn root_folder_finding(d: &Path, walked: &Walk) -> Finding {
    let holds_audio = audio_anywhere(d, walked);
    let mut f = Finding::new(
        FindingKind::UnexpectedFolder,
        d,
        if holds_audio { "foreign folder at the library root (holds audio: report only)" } else { "foreign folder at the library root" },
    )
    .library_wide()
    .with_hint(if holds_audio {
        "a foreign folder holding audio — where its contents belong is the user's call"
    } else {
        "the library lives in Artists/ — move anything real into Artists/<Artist>/<Album>/; Apply fixes moves the folder itself to the Trash"
    });
    f.fix = if holds_audio { FixKind::None } else { FixKind::Trash { reason: "unexpected_folder".into() } };
    f
}

pub fn audio_anywhere(dir: &Path, walked: &Walk) -> bool {
    walked.files.iter().any(|f| f.starts_with(dir) && Walk::is_audio(f))
}

fn holds_albums(artist: &Path, walked: &Walk) -> bool {
    walked
        .dirs
        .iter()
        .filter(|d| d.parent() == Some(artist))
        .any(|d| audio_anywhere(d, walked))
}

/// `Name [mbid]` vs `name` name the same artist.
pub fn same_artist(a: &str, b: &str) -> bool {
    base_artist(a).eq_ignore_ascii_case(&base_artist(b))
}

fn base_artist(name: &str) -> String {
    let without_mbid = match name.rfind('[') {
        Some(i) if name.trim_end().ends_with(']') => name[..i].trim().to_string(),
        _ => name.trim().to_string(),
    };
    without_mbid
}

fn mbid_named(a: &Path, b: &Path) -> PathBuf {
    let na = crate::model::file_name(a);
    if na.contains('[') {
        a.to_path_buf()
    } else {
        b.to_path_buf()
    }
}

fn strip_artists(rel: &str) -> String {
    match rel.split_once('/') {
        Some((first, rest)) if first.eq_ignore_ascii_case("Artists") => rest.to_string(),
        _ => rel.to_string(),
    }
}

fn canonical_sidecar_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) => {
            let base = stem.split(" (").next().unwrap_or(stem);
            format!("{base}.{ext}")
        }
        None => name.to_string(),
    }
}

// ---------------------------------------------------------------------------
// The stored report ([§3.4])
// ---------------------------------------------------------------------------

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// One report row (la-musica `_issue`): display-relative `path`, absolute
/// `abs`, and a `fix` only when the scan could settle one.
pub fn issue_row(f: &Finding, music: &Path) -> LayoutIssueRow {
    let rel = f.abs.strip_prefix(music).unwrap_or(&f.abs);
    LayoutIssueRow {
        kind: f.kind.code().to_string(),
        path: slash(rel),
        abs: slash(&f.abs),
        detail: f.detail.clone(),
        hint: f.hint.clone(),
        fix: fix_row(&f.fix),
    }
}

fn fix_row(fix: &FixKind) -> Option<LayoutFixRow> {
    match fix {
        FixKind::None => None,
        // la-musica's vocabulary is rename | move | trash; a merge is a move
        // of the artist's artefacts into the surviving folder.
        FixKind::Trash { .. } => Some(LayoutFixRow { action: "trash".into(), to: None }),
        FixKind::Rename { target } => Some(LayoutFixRow { action: "rename".into(), to: Some(slash(target)) }),
        FixKind::Move { target } => Some(LayoutFixRow { action: "move".into(), to: Some(slash(target)) }),
        FixKind::MergeArtist { into } => Some(LayoutFixRow { action: "move".into(), to: Some(slash(into)) }),
    }
}

/// The report body for a library-wide scan: the rows, their counts and the
/// walk's totals (la-musica `scan_library`'s return dict).
pub fn layout_report_body(
    findings: &[Finding],
    music: &Path,
    albums: u32,
    artists: u32,
    audio_files: u64,
) -> LayoutReportBody {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut issues = Vec::with_capacity(findings.len());
    for f in findings {
        *counts.entry(f.kind.code().to_string()).or_insert(0) += 1;
        issues.push(issue_row(f, music));
    }
    LayoutReportBody {
        folder: slash(music),
        artists_dir: slash(&music.join("Artists")),
        exists: music.is_dir(),
        total: issues.len() as u32,
        issues,
        counts,
        albums,
        artists,
        audio_files,
    }
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ApplyOutcome {
    pub finding_id: String,
    pub action: String,
    pub ok: bool,
    pub detail: String,
}

/// Apply the fixes for the given findings (all fixable ones when `ids` is None).
/// `dry_run` reports exactly which rows would be acted on ([§13.4]).
pub fn apply(
    cfg: &Config,
    findings: &[Finding],
    ids: Option<&[String]>,
    dry_run: bool,
) -> Result<Vec<ApplyOutcome>> {
    let trash = Trash::new(&cfg.music_folder);
    let mut batch = trash.begin("layout_apply")?;
    let mut outcomes = Vec::new();

    for f in findings {
        if let Some(ids) = ids {
            if !ids.iter().any(|id| id == &f.id) {
                continue;
            }
        }
        if !f.fixable() {
            continue;
        }
        let action = f.fix.label();
        if dry_run {
            outcomes.push(ApplyOutcome {
                finding_id: f.id.clone(),
                action,
                ok: true,
                detail: "dry run — no change".into(),
            });
            continue;
        }
        let result = apply_one(f, cfg, &mut batch);
        outcomes.push(match result {
            Ok(detail) => ApplyOutcome { finding_id: f.id.clone(), action, ok: true, detail },
            Err(e) => ApplyOutcome { finding_id: f.id.clone(), action, ok: false, detail: e.to_string() },
        });
    }
    if !dry_run {
        batch.finish()?;
    }
    Ok(outcomes)
}

fn apply_one(f: &Finding, cfg: &Config, batch: &mut crate::trash::TrashBatch<'_>) -> Result<String> {
    // la-musica `_within`: every source must still sit inside the music
    // folder, whatever the row said.
    if !crate::atomic::path_is_within(&f.path, &cfg.music_folder) {
        return Err(MloError::Invalid(format!(
            "refused: {} is outside the music folder",
            f.path.display()
        )));
    }
    let artists_root = cfg.music_folder.join("Artists");
    match &f.fix {
        FixKind::Trash { reason } => {
            // re-derive the premise of the removal at the move ([§4.2] rule 1)
            if let Some(why) = removal_refusal(f) {
                return Err(MloError::Invalid(format!("refused: {} — {why}", f.path.display())));
            }
            let e = batch.move_path(&f.path, reason)?;
            Ok(format!("trashed → {}", e.trash_path.display()))
        }
        FixKind::Rename { target } => {
            if !crate::atomic::path_is_within(target, &cfg.music_folder) {
                return Err(MloError::Invalid(format!(
                    "refused: {} would land outside the music folder",
                    target.display()
                )));
            }
            rename_case_safe(&f.path, target)?;
            Ok(format!("renamed → {}", target.display()))
        }
        FixKind::Move { target } => {
            if !crate::atomic::path_is_within(target, &artists_root) {
                return Err(MloError::Invalid(format!(
                    "refused: {} would land outside Artists/",
                    target.display()
                )));
            }
            let dest = if target.is_dir() && !Walk::is_audio(target) {
                target.join(crate::model::file_name(&f.path))
            } else {
                target.clone()
            };
            move_path(&f.path, &dest)?;
            Ok(format!("moved → {}", dest.display()))
        }
        FixKind::MergeArtist { into } => {
            if !crate::atomic::path_is_within(into, &artists_root) {
                return Err(MloError::Invalid(format!(
                    "refused: {} would land outside Artists/",
                    into.display()
                )));
            }
            // Never trash an album: a merge only moves the artist's own files.
            if artist_holds_albums_now(&f.path) {
                return Err(MloError::Invalid(format!(
                    "refused: {} holds albums again — left where it is",
                    f.path.display()
                )));
            }
            merge_artist(&f.path, into, batch)?;
            Ok(format!("merged into {}", into.display()))
        }
        FixKind::None => Ok("no fix".into()),
    }
}

/// Re-prove the premise of a removal at the move (la-musica `_UNFIXABLE` /
/// `_may_trash`): a folder that gained audio, a stray that became real content,
/// a leftover that is no longer the app's own — all fall back to "left where it
/// is" instead of being trashed.
fn removal_refusal(f: &Finding) -> Option<String> {
    match f.kind {
        FindingKind::UnexpectedFolder => {
            if audio_anywhere_now(&f.path) {
                Some("it gained audio since the scan".into())
            } else {
                None
            }
        }
        FindingKind::EmptyAlbum => {
            if audio_anywhere_now(&f.path) {
                Some("the album holds audio again".into())
            } else {
                None
            }
        }
        FindingKind::EmptyArtist => {
            if artist_holds_albums_now(&f.path) {
                Some("the artist folder holds an album again".into())
            } else {
                None
            }
        }
        FindingKind::StrayFile | FindingKind::StrayInArtists => {
            if is_stray_now(&f.path) {
                None
            } else {
                Some("it is audio, artwork or a sidecar now".into())
            }
        }
        FindingKind::HiddenFolder => {
            if crate::model::file_name(&f.path).starts_with('.') {
                None
            } else {
                Some("it is no longer a hidden folder".into())
            }
        }
        FindingKind::LegacyStateFile => {
            if crate::model::file_name(&f.path) == ".mlo_data" {
                None
            } else {
                Some("it is no longer the app's own leftover".into())
            }
        }
        FindingKind::SidecarCopy => {
            let canonical = canonical_sidecar_name(&crate::model::file_name(&f.path));
            match f.path.parent() {
                Some(p) if p.join(&canonical).exists() => None,
                _ => Some("the canonical sidecar is gone — nothing to replace".into()),
            }
        }
        _ => None,
    }
}

/// Whether a file is still the stray the scan reported: not audio, not a
/// video, not a cover/sidecar image, not `description.txt`, not a dotfile.
fn is_stray_now(path: &Path) -> bool {
    let name = crate::model::file_name(path);
    !path.is_dir()
        && !Walk::is_audio(path)
        && !Walk::is_video(path)
        && !scan::is_sidecar_ext(path)
        && !name.starts_with('.')
        && !name.eq_ignore_ascii_case("description.txt")
}

/// Whether an artist folder holds any album (a subfolder with audio beneath)
/// right now.
fn artist_holds_albums_now(artist: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(artist) else { return false };
    rd.flatten().any(|e| {
        let p = e.path();
        p.is_dir() && audio_anywhere_now(&p)
    })
}

fn audio_anywhere_now(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .flatten()
        .any(|e| e.file_type().is_file() && Walk::is_audio(e.path()))
}

/// Merge artist-level artefacts into `into`, then trash the emptied folder
/// ([§4.3] step 3): artefacts first, then the folder.
fn merge_artist(from: &Path, into: &Path, batch: &mut crate::trash::TrashBatch<'_>) -> Result<()> {
    std::fs::create_dir_all(into).at(into)?;
    if from.is_dir() {
        for e in std::fs::read_dir(from).at(from)?.flatten() {
            let p = e.path();
            if p.is_file() {
                let dest = into.join(e.file_name());
                if !dest.exists() {
                    move_path(&p, &dest)?;
                }
            }
        }
    }
    if from.exists() {
        batch.move_path(from, "split_artist_merge")?;
    }
    Ok(())
}

/// Case-only renames need a two-step on Windows.
pub fn rename_case_safe(from: &Path, to: &Path) -> Result<()> {
    if from == to {
        return Ok(());
    }
    if from.parent() == to.parent() && only_case_differs(from, to) {
        let tmp = from.with_file_name(format!(
            ".{}.mlo-case-{}",
            crate::model::file_name(from),
            std::process::id()
        ));
        std::fs::rename(from, &tmp).at(from)?;
        std::fs::rename(&tmp, to).at(to)?;
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    if to.exists() {
        return Err(MloError::Invalid(format!("target {} already exists", to.display())));
    }
    std::fs::rename(from, to).at(from)?;
    Ok(())
}

fn only_case_differs(a: &Path, b: &Path) -> bool {
    a != b
        && a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// Move a file or directory with a cross-device fallback.
pub fn move_path(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    if dest.exists() {
        return Err(MloError::Invalid(format!("target {} already exists", dest.display())));
    }
    match std::fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_recursive(src, dest)?;
            if src.is_dir() {
                std::fs::remove_dir_all(src).at(src)?;
            } else {
                std::fs::remove_file(src).at(src)?;
            }
            Ok(())
        }
    }
}

fn copy_recursive(src: &Path, dest: &Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest).at(dest)?;
        for e in std::fs::read_dir(src).at(src)?.flatten() {
            copy_recursive(&e.path(), &dest.join(e.file_name()))?;
        }
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        std::fs::copy(src, dest).at(dest)?;
    }
    Ok(())
}

/// Group findings by kind with counts, for the layout panel ([§13.4]).
pub fn group_by_kind(findings: &[Finding]) -> Vec<(FindingKind, usize, usize)> {
    let mut out = Vec::new();
    for kind in FindingKind::ALL {
        let matching: Vec<&Finding> = findings.iter().filter(|f| f.kind == kind).collect();
        if matching.is_empty() {
            continue;
        }
        let fixable = matching.iter().filter(|f| f.fixable()).count();
        out.push((kind, matching.len(), fixable));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cfg_for(music: &Path) -> Config {
        let mut c = Config::default();
        c.music_folder = music.to_path_buf();
        c
    }

    #[test]
    fn detects_fixture_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        let artists = music.join("Artists");
        fs::create_dir_all(artists.join("A").join("Album One")).unwrap();
        fs::write(artists.join("A").join("Album One").join("01 a.flac"), b"x").unwrap();
        // wrong case
        fs::create_dir_all(artists.join("B").join("album two")).unwrap();
        fs::write(artists.join("B").join("album two").join("01 b.flac"), b"x").unwrap();
        fs::write(artists.join("B").join("album two").join("cover.jpg"), b"x").unwrap();
        // stray + numbered sidecar
        fs::write(artists.join("A").join("Album One").join("stray.txt"), b"x").unwrap();
        fs::write(artists.join("A").join("Album One").join("description (2).txt"), b"x").unwrap();
        // empty album + artist with no albums + .mlo_data
        fs::create_dir_all(artists.join("C").join("empty")).unwrap();
        fs::create_dir_all(artists.join("D")).unwrap();
        fs::create_dir_all(music.join(".mlo_data")).unwrap();
        // loose audio at root
        fs::write(music.join("loose.flac"), b"x").unwrap();

        let walked = scan::walk(&music).unwrap();
        let findings = analyze(&walked, &music, &HashMap::new(), &cfg_for(&music));
        let kinds: Vec<FindingKind> = findings.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&FindingKind::StrayFile));
        assert!(kinds.contains(&FindingKind::SidecarCopy));
        assert!(kinds.contains(&FindingKind::EmptyAlbum));
        assert!(kinds.contains(&FindingKind::EmptyArtist));
        assert!(kinds.contains(&FindingKind::LegacyStateFile));
        assert!(kinds.contains(&FindingKind::AudioAtRoot));
        // a second scan of an unchanged tree reports the same findings
        let again = analyze(&walked, &music, &HashMap::new(), &cfg_for(&music));
        assert_eq!(findings.len(), again.len());
    }

    #[test]
    fn split_artist_not_empty_and_fix_is_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        let artists = music.join("Artists");
        let plain = artists.join("Radiohead");
        let with_id = artists.join("Radiohead [a74b1b7f-71a5-4011-9441-d0b5e4122711]");
        fs::create_dir_all(plain.join("Kid A")).unwrap();
        fs::write(plain.join("Kid A").join("01.flac"), b"x").unwrap();
        fs::create_dir_all(&with_id).unwrap();
        fs::write(with_id.join("artist.jpg"), b"x").unwrap();

        let walked = scan::walk(&music).unwrap();
        let findings = analyze(&walked, &music, &HashMap::new(), &cfg_for(&music));
        let split: Vec<&Finding> = findings.iter().filter(|f| f.kind == FindingKind::SplitArtist).collect();
        assert_eq!(split.len(), 1, "one split finding");
        assert!(matches!(split[0].fix, FixKind::MergeArtist { .. }));
        // empty_artist must NOT fire for the album-less half
        assert!(!findings.iter().any(|f| f.kind == FindingKind::EmptyArtist));
    }

    #[test]
    fn nothing_deleted_merge_moves_artifacts_then_trashes() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        let artists = music.join("Artists");
        let plain = artists.join("A");
        let with_id = artists.join("A [11111111-1111-1111-1111-111111111111]");
        fs::create_dir_all(plain.join("Alb")).unwrap();
        fs::write(plain.join("Alb").join("01.flac"), b"x").unwrap();
        fs::create_dir_all(&with_id).unwrap();
        fs::write(with_id.join("artist.jpg"), b"img").unwrap();
        fs::write(with_id.join("description.txt"), b"desc").unwrap();

        let walked = scan::walk(&music).unwrap();
        let cfg = cfg_for(&music);
        let findings = analyze(&walked, &music, &HashMap::new(), &cfg);
        let out = apply(&cfg, &findings, None, false).unwrap();
        assert!(out.iter().all(|o| o.ok), "all fixes ok: {out:?}");
        assert!(plain.join("artist.jpg").exists());
        assert!(plain.join("description.txt").exists());
        assert!(!with_id.exists());
        // the merged artefacts satisfy the artist grade
        let (has_image, _, has_desc) = crate::scan::artist_artifacts(&plain);
        assert!(has_image && has_desc);
    }

    #[test]
    fn report_document_shape_matches_la_musica() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("Music");
        let artists = music.join("Artists");
        fs::create_dir_all(artists.join("A").join("Album One")).unwrap();
        fs::write(artists.join("A").join("Album One").join("01 a.flac"), b"x").unwrap();
        fs::write(artists.join("A").join("Album One").join("stray.nfo"), b"x").unwrap();
        fs::create_dir_all(artists.join("B")).unwrap(); // empty artist → trash fix

        let walked = scan::walk(&music).unwrap();
        let findings = analyze(&walked, &music, &HashMap::new(), &cfg_for(&music));
        let body = layout_report_body(&findings, &music, 1, 2, 1);
        let doc = crate::model::LayoutReportDoc {
            scanned_at: "2026-01-01T00:00:00Z".into(),
            music_folder: music.to_string_lossy().replace('\\', "/"),
            report: body,
        };
        let json = serde_json::to_value(&doc).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 3, "top-level keys: {obj:?}");
        for k in ["scanned_at", "music_folder", "report"] {
            assert!(obj.contains_key(k), "missing top-level {k}");
        }
        let rep = obj["report"].as_object().unwrap();
        assert_eq!(rep.len(), 9, "report keys: {rep:?}");
        for k in [
            "folder", "artists_dir", "exists", "issues", "counts", "total", "albums", "artists",
            "audio_files",
        ] {
            assert!(rep.contains_key(k), "missing report key {k}");
        }
        let rows = rep["issues"].as_array().unwrap();
        assert!(!rows.is_empty());
        for r in rows {
            let r = r.as_object().unwrap();
            for k in ["kind", "path", "abs", "detail", "hint"] {
                assert!(r.contains_key(k), "row missing {k}: {r:?}");
            }
            for k in r.keys() {
                assert!(
                    ["kind", "path", "abs", "detail", "hint", "fix"].contains(&k.as_str()),
                    "unexpected row key {k}"
                );
            }
        }
        // a fixable row carries {action, to}; a trash row has no `to`
        let stray = rows
            .iter()
            .find(|r| r["kind"] == "stray_file")
            .and_then(|r| r.get("fix"))
            .and_then(|f| f.as_object())
            .expect("stray_file carries a fix");
        assert_eq!(stray["action"], "trash");
        assert!(!stray.contains_key("to"));
        // every fix that is present has an action; `to` only for moves/renames
        for r in rows {
            if let Some(fx) = r.get("fix") {
                let fx = fx.as_object().unwrap();
                assert!(fx.contains_key("action"), "fix missing action: {fx:?}");
                assert!(fx.len() == 1 || fx.contains_key("to"), "unexpected fix keys: {fx:?}");
            }
        }
    }
}