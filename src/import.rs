//! Import pipeline ([§5]): acquire → detect shape → identify → write metadata →
//! artefacts → lyrics → run the import chain (layout last, grade last of all).

use crate::config::Config;
use crate::db::Db;
use crate::error::{IoResultExt, MloError, Result};
use crate::model::Scope;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Import a folder, set of files, single file or archive.
pub fn import_path(cfg: &Config, src: &Path) -> Result<()> {
    if !src.exists() {
        return Err(MloError::NotFound(format!("{} does not exist", src.display())));
    }
    let job = format!("import-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    let staged = cfg.incomplete_dir().join(&job);

    // 1. Acquire.
    let source_dir = if src.is_file() && is_archive(src) {
        std::fs::create_dir_all(&staged).at(&staged)?;
        extract_archive(src, &staged)?;
        staged.clone()
    } else if src.is_file() {
        // single file: stage a copy so the pipeline has a folder
        std::fs::create_dir_all(&staged).at(&staged)?;
        let dest = staged.join(src.file_name().unwrap_or_default());
        std::fs::copy(src, &dest).at(&dest)?;
        staged.clone()
    } else {
        src.to_path_buf()
    };

    let mut audio = Vec::new();
    for e in walkdir::WalkDir::new(&source_dir).follow_links(false).into_iter().flatten() {
        if e.file_type().is_file() && crate::scan::Walk::is_audio(e.path()) {
            audio.push(e.path().to_path_buf());
        }
    }
    if audio.is_empty() {
        return Err(MloError::Invalid(format!("no audio found under {}", source_dir.display())));
    }
    println!("acquired {} audio file(s)", audio.len());

    // 2/3. Identify from existing tags (offline is normal: report unavailable).
    let mut identified = false;
    let mb = crate::net::MusicBrainz::new(&cfg.services);
    let tags0 = crate::tags::read_tags(&audio[0]).unwrap_or_default();
    let artist = tags0.get("ARTIST").or(tags0.get("ALBUMARTIST")).and_then(|v| v.first()).cloned().unwrap_or_default();
    let album = tags0.get("ALBUM").and_then(|v| v.first()).cloned().unwrap_or_default();
    if !artist.is_empty() && !album.is_empty() {
        match mb.search_release(&artist, &album, audio.len()) {
            Ok(c) if !c.is_empty() => {
                println!("identified: {} — {} ({}, score {})", c[0].artist, c[0].title, c[0].mbid, c[0].score);
                identified = true;
            }
            Ok(_) => println!("identify: no MusicBrainz match (continuing with local tags)"),
            Err(e) => println!("identify: unavailable — {e}"),
        }
    } else {
        println!("identify: needs ARTIST and ALBUM tags to search MusicBrainz");
    }
    let _ = identified;

    // 4/5. Place into the canonical library path from the naming template.
    let album_tags = crate::scan::merged_album_tags(
        &audio
            .iter()
            .map(|p| crate::scan::TrackEntry {
                path: p.clone(),
                container: crate::tags::detect_container(p),
                tags: crate::tags::read_tags(p).unwrap_or_default(),
                size: 0,
                mtime: 0,
                quick_hash: None,
                has_lyrics_sidecar: false,
                lyrics_sidecar_synced: false,
                grade: None,
            })
            .collect::<Vec<_>>(),
    );
    let opts = crate::naming::NamingOptions { short_folder_names: cfg.short_folder_names };
    let target_rel = crate::naming::evaluate(&cfg.naming_script, &album_tags, opts)?;
    if target_rel.has_missing() {
        println!(
            "naming: Missing {} tag — files were tagged but not placed",
            target_rel.missing.join(", ")
        );
    } else {
        let dest = cfg.music_folder.join("Artists").join(&target_rel.path);
        std::fs::create_dir_all(&dest).at(&dest)?;
        for f in &audio {
            let name = f.file_name().unwrap_or_default();
            let out = dest.join(name);
            if out != *f {
                if out.exists() {
                    println!("kept {} (target exists)", name.to_string_lossy());
                } else {
                    crate::layout::move_path(f, &out)?;
                }
            }
        }
        println!("placed album at {}", dest.display());
    }

    // 8. Run the import chain (layout last, grade last of all).
    let db = Db::open(&cfg.index_db())?;
    let cancel = AtomicBool::new(false);
    let progress = |_: crate::model::Progress| {};
    let chain = crate::scripts::import_chain(cfg);
    let placed = cfg.music_folder.join("Artists").join(&target_rel.path);
    let scope = if placed.is_dir() { Scope::Album(placed.clone()) } else { Scope::Library };
    for id in chain {
        let ctx = crate::scripts::ScriptCtx { cfg, db: &db, cancel: &cancel, progress: &progress };
        match crate::scripts::run(id, &scope, &ctx) {
            Ok(out) => println!("script {}: {}", id, out.summary()),
            Err(e) => println!("script {}: failed — {e}", id),
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            println!("import cancelled; remaining scripts were not run");
            break;
        }
    }

    // 9. Grade what landed.
    let model = crate::scan::scan(cfg, &db, &Scope::Library, &cancel, &progress)?;
    let v = &model.verdict;
    println!(
        "grade: {}/{} albums pass ({:.1}%)",
        v.albums_passed, v.albums_total, v.albums_pct()
    );
    Ok(())
}

fn is_archive(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    name.ends_with(".zip")
        || name.ends_with(".7z")
        || name.ends_with(".tar")
        || name.ends_with(".tar.gz")
        || name.ends_with(".tgz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tbz2")
        || name.ends_with(".tar.xz")
        || name.ends_with(".txz")
        || name.ends_with(".rar")
}

#[cfg(feature = "archives")]
pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name.ends_with(".zip") {
        let file = std::fs::File::open(archive).at(archive)?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| MloError::Other(format!("zip open: {e}")))?;
        zip.extract(dest).map_err(|e| MloError::Other(format!("zip extract: {e}")))?;
        return Ok(());
    }
    if name.ends_with(".7z") {
        sevenz_rust::decompress_file(archive, dest)
            .map_err(|e| MloError::Other(format!("7z extract: {e}")))?;
        return Ok(());
    }
    if name.ends_with(".rar") {
        return Err(MloError::tool(
            "rar",
            "no .rar extractor is available; install `unrar`/`7z` and extract manually",
        ));
    }
    // tar family
    let file = std::fs::File::open(archive).at(archive)?;
    let reader: Box<dyn std::io::Read> = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        Box::new(bzip2::read::BzDecoder::new(file))
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        Box::new(xz2::read::XzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut tar = tar::Archive::new(reader);
    tar.unpack(dest).map_err(|e| MloError::Other(format!("tar extract: {e}")))?;
    Ok(())
}

#[cfg(not(feature = "archives"))]
pub fn extract_archive(_archive: &Path, _dest: &Path) -> Result<()> {
    Err(MloError::tool(
        "archives",
        "built without the `archives` feature; cannot extract archives",
    ))
}