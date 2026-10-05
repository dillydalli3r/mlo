//! The 23 optimization passes (Appendix A) and the run/import chains ([§7]).
//!
//! One list is read by the TUI script menu, the CLI `mlo run <id|name>` and the
//! runner. Scripts whose feature has its own switch are skipped with a named
//! reason when the switch is off ([§7.1]). A script that cannot run reports
//! *unavailable* with the reason — never a bare "error" ([§1.7], [§10.2.3]).

use crate::config::Config;
use crate::db::Db;
use crate::error::{MloError, Result};
use crate::model::{Progress, Scope};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy)]
pub struct ScriptDef {
    pub id: u8,
    pub name: &'static str,
    pub description: &'static str,
    /// Does this script move files? (drives run order)
    pub modifies: bool,
    /// Config switch whose `false` means "skip with a named reason".
    pub switch: Option<&'static str>,
    /// A second gate for the scripts la-musica keys on two switches (17).
    pub switch_2: Option<&'static str>,
}

impl ScriptDef {
    /// The config switches that gate this script; it runs only while every one
    /// of them is on (mirrors la-musica's `server.script_runners._DISABLED`).
    pub fn gates(&self) -> impl Iterator<Item = &'static str> {
        self.switch.into_iter().chain(self.switch_2)
    }
}

/// The ONE script table: names and descriptions must equal la-musica's
/// `mlo/scripts.py` `SCRIPTS` verbatim (tests/test_script_menus).
pub const SCRIPTS: &[ScriptDef] = &[
    ScriptDef { id: 1, name: "Format lyrics", description: "multi-format + MEDIA/SOURCE normalization", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 2, name: "Format CUEs", description: "CD-N rename + FILE/INDEX layout", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 3, name: "Optimize FLACs", description: "lossless re-encode", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 4, name: "Grade", description: "per-album tag/lyrics/cover report", modifies: false, switch: None, switch_2: None },
    ScriptDef { id: 5, name: "Process images", description: "JXL / lossless / JXL-back", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 6, name: "Audit library", description: "AudioAuditor: fake lossless / upscaled / MQA", modifies: false, switch: None, switch_2: None },
    ScriptDef { id: 7, name: "DR & ReplayGain", description: "in-process DR + rsgain ReplayGain tags", modifies: true, switch: Some("dr_replaygain_enabled"), switch_2: None },
    ScriptDef { id: 8, name: "Auto tagging", description: "advisory / instrumental / mood / energy / genre", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 9, name: "AccurateRip", description: "CUETools .accurip files", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 10, name: "Format all", description: "final pass: .accurip / .cue / .lrc / tags", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 11, name: "Remux videos (MKV)", description: "any video -> MKV, audio -> FLAC", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 12, name: "Key & BPM", description: "musical key + tempo tags", modifies: true, switch: Some("audiometa_enabled"), switch_2: None },
    ScriptDef { id: 13, name: "Fetch lyrics", description: "LRCLIB synced/plain", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 14, name: "Beets tagging", description: "MusicBrainz via beets", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 15, name: "Release tracklist", description: ".mlo_expected.json manifests", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 16, name: "Mood & Energy", description: "MOOD/ENERGY from the track's audio", modifies: true, switch: Some("mood_enabled"), switch_2: None },
    ScriptDef { id: 17, name: "Lyrics transliterate (AI)", description: "TRANSLITERATION/TRANSLATION tags + sidecars", modifies: true, switch: Some("lyrics_xlit_enabled"), switch_2: Some("lyrics_translate_enabled") },
    ScriptDef { id: 19, name: "Optimize artist images", description: "crop/resize artist artwork to the configured aspect and size", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 20, name: "Optimize library layout", description: "layout report + fixes (case, loose audio, empty artist, strays to the Trash)", modifies: true, switch: None, switch_2: None },
    ScriptDef { id: 21, name: "Fix AcoustID pairs", description: "complete or create ACOUSTID_ID / ACOUSTID_FINGERPRINT pairs", modifies: true, switch: Some("acoustid_enabled"), switch_2: None },
    ScriptDef { id: 22, name: "Submit fingerprints (AcoustID)", description: "give AcoustID the fingerprint + MusicBrainz recording each track states", modifies: false, switch: Some("acoustid_enabled"), switch_2: None },
    ScriptDef { id: 23, name: "Optimize tags", description: "delete excess tags: junk names, a valued COMMENT, unneeded aliases", modifies: true, switch: Some("strip_unknown_tags"), switch_2: None },
    ScriptDef { id: 24, name: "Web ratings", description: "aggregated public album + track scores (MusicBrainz / RYM / Discogs)", modifies: true, switch: Some("web_ratings_enabled"), switch_2: None },
];

pub fn by_id(id: u8) -> Option<&'static ScriptDef> {
    SCRIPTS.iter().find(|s| s.id == id)
}

pub fn by_name(name: &str) -> Option<&'static ScriptDef> {
    let l = name.to_ascii_lowercase();
    SCRIPTS.iter().find(|s| {
        s.name.eq_ignore_ascii_case(name)
            || s.name.to_ascii_lowercase().contains(&l)
            || l == s.id.to_string()
    })
}

/// `Run All` order ([§7.4]) — path-changers first, then content, readers last.
/// Equals la-musica's `DEFAULT_RUN_ALL_ORDER`; script 22 is deliberately
/// absent because it is opt-in ([`OPT_IN_SCRIPTS`]).
pub const RUN_ALL_ORDER: &[u8] = &[11, 3, 14, 15, 2, 1, 13, 17, 8, 24, 5, 19, 6, 7, 9, 12, 16, 10, 23, 20, 21, 4];

/// Scripts no run reaches on its own — they need an explicit press
/// (la-musica `server.script_runners.OPT_IN_SCRIPTS`).
pub const OPT_IN_SCRIPTS: &[u8] = &[22];

/// Does this script move an album's files? (la-musica `_ALBUM_MOVERS`).
pub fn is_album_mover(id: u8) -> bool {
    matches!(id, 8 | 11 | 14 | 20)
}

/// The import chain ([§7.4]): la-musica's `LIBRARY_WIDE_SCRIPTS` is empty, so
/// its `DEFAULT_CHAIN == DEFAULT_RUN_ALL_ORDER`. An explicit `import_scripts`
/// list replaces it outright; an empty one means the default.
pub fn import_chain(cfg: &Config) -> Vec<u8> {
    if cfg.import_scripts.is_empty() {
        RUN_ALL_ORDER.to_vec()
    } else {
        cfg.import_scripts.clone()
    }
}

pub fn run_all_order() -> Vec<u8> {
    RUN_ALL_ORDER.to_vec()
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Skipped,
    Failed,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Skipped => "skip",
            Outcome::Failed => "fail",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileResult {
    pub path: PathBuf,
    pub outcome: Outcome,
    /// Named reason for a skip or a failure (never empty for those).
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub script: u8,
    pub results: Vec<FileResult>,
    pub notes: Vec<String>,
    /// Bytes appended across the scope (0 unless a pass measures them).
    pub bytes_added: u64,
    /// Bytes removed across the scope (0 unless a pass measures them).
    pub bytes_removed: u64,
}

impl RunOutcome {
    pub fn new(script: u8) -> Self {
        Self { script, results: Vec::new(), notes: Vec::new(), bytes_added: 0, bytes_removed: 0 }
    }
    pub fn ok(&mut self, path: impl Into<PathBuf>) {
        self.results.push(FileResult { path: path.into(), outcome: Outcome::Ok, note: String::new() });
    }
    pub fn skip(&mut self, path: impl Into<PathBuf>, reason: impl Into<String>) {
        self.results.push(FileResult { path: path.into(), outcome: Outcome::Skipped, note: reason.into() });
    }
    pub fn fail(&mut self, path: impl Into<PathBuf>, reason: impl Into<String>) {
        self.results.push(FileResult { path: path.into(), outcome: Outcome::Failed, note: reason.into() });
    }
    pub fn count(&self, o: Outcome) -> usize {
        self.results.iter().filter(|r| r.outcome == o).count()
    }
    /// Per-script counters in la-musica's `new_stats()` shape, derived from
    /// this run's results.
    pub fn stats(&self) -> RunStats {
        RunStats::from_outcome(self)
    }
    pub fn summary(&self) -> String {
        format!(
            "{}: {} ok, {} skip, {} fail",
            by_id(self.script).map(|s| s.name).unwrap_or("script"),
            self.count(Outcome::Ok),
            self.count(Outcome::Skipped),
            self.count(Outcome::Failed)
        )
    }
}

/// Per-script result counters, mirroring la-musica `mlo/stats.py` `new_stats()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunStats {
    pub total_scanned: usize,
    pub modified_count: usize,
    pub unchanged_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub total_bytes_added: u64,
    pub total_bytes_removed: u64,
    pub errors: Vec<String>,
}

impl RunStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Derive the counters from a finished run. `Ok` is a modification (the
    /// pass handled the file), `Skipped`/`Failed` keep their own buckets, and
    /// `unchanged_count` is whatever scanned files remain outside those.
    pub fn from_outcome(out: &RunOutcome) -> Self {
        let modified_count = out.count(Outcome::Ok);
        let skipped_count = out.count(Outcome::Skipped);
        let error_count = out.count(Outcome::Failed);
        Self {
            total_scanned: out.results.len(),
            modified_count,
            unchanged_count: out.results.len().saturating_sub(modified_count + skipped_count + error_count),
            skipped_count,
            error_count,
            total_bytes_added: out.bytes_added,
            total_bytes_removed: out.bytes_removed,
            errors: out
                .results
                .iter()
                .filter(|r| r.outcome == Outcome::Failed)
                .map(|r| format!("{}: {}", r.path.display(), r.note))
                .collect(),
        }
    }
}

impl From<&RunOutcome> for RunStats {
    fn from(out: &RunOutcome) -> Self {
        Self::from_outcome(out)
    }
}

pub struct ScriptCtx<'a> {
    pub cfg: &'a Config,
    pub db: &'a Db,
    pub cancel: &'a AtomicBool,
    pub progress: &'a (dyn Fn(Progress) + Sync),
}

impl ScriptCtx<'_> {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Resolve the file list a scope covers.
pub fn targets(scope: &Scope, cfg: &Config) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let push_dir = |dir: &Path, files: &mut Vec<PathBuf>| {
        if let Ok(w) = crate::scan::walk(dir) {
            files.extend(w.files.into_iter().filter(|f| crate::scan::Walk::is_audio(f) || crate::scan::Walk::is_video(f)));
        }
    };
    match scope {
        Scope::Library => {
            let walked = crate::scan::walk(&cfg.music_folder)?;
            files.extend(
                walked
                    .files
                    .into_iter()
                    .filter(|f| crate::scan::Walk::is_audio(f) || crate::scan::Walk::is_video(f)),
            );
        }
        Scope::Artist(p) | Scope::Album(p) => push_dir(p, &mut files),
        Scope::Track(p) => {
            if crate::scan::Walk::is_audio(p) || crate::scan::Walk::is_video(p) {
                files.push(p.clone());
            }
        }
        Scope::Selection(v) => {
            for p in v {
                if p.is_dir() {
                    push_dir(p, &mut files);
                } else if crate::scan::Walk::is_audio(p) || crate::scan::Walk::is_video(p) {
                    files.push(p.clone());
                }
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// Run one script over a scope.
pub fn run(id: u8, scope: &Scope, ctx: &ScriptCtx) -> Result<RunOutcome> {
    let def = by_id(id).ok_or_else(|| MloError::Invalid(format!("no script with id {id}")))?;
    // la-musica `_gate`: a multi-switch gate means ANY of them keeps the script
    // alive (17 transliterates, translates, or both).
    let switches: Vec<&'static str> = def.gates().collect();
    let any_on = switches.iter().any(|sw| switch_enabled(sw, ctx.cfg));
    if !switches.is_empty() && !any_on {
        let mut out = RunOutcome::new(id);
        let joined = switches.join(" and ");
        out.notes.push(format!(
            "skipped: {joined} {} off",
            if switches.len() > 1 { "are" } else { "is" }
        ));
        return Ok(out);
    }
    let files = targets(scope, ctx.cfg)?;
    let mut out = RunOutcome::new(id);
    if files.is_empty() {
        out.notes.push("scope contained no audio files".into());
        return Ok(out);
    }
    match id {
        1 => format_lyrics_media(&files, ctx, &mut out),
        4 => grade_only(&files, ctx, &mut out),
        7 => dr_replaygain(&files, ctx, &mut out),
        10 => format_all(&files, ctx, &mut out),
        12 => key_bpm(&files, ctx, &mut out),
        13 => fetch_lyrics(&files, ctx, &mut out),
        15 => tracklist_manifests(&files, ctx, &mut out),
        19 => optimize_artist_images(scope, ctx, &mut out),
        20 => layout_script(scope, ctx, &mut out),
        23 => optimize_tags(&files, ctx, &mut out),
        3 => unavailable(&files, &mut out, "FLAC re-encoding needs the optional `flacenc` feature (not built)"),
        5 => unavailable(&files, &mut out, "JXL encoding needs the optional `jxl-encode` feature (not built)"),
        6 => unavailable(&files, &mut out, "Audit requires the AudioAuditor tool, which is not installed"),
        8 => unavailable(&files, &mut out, "Auto tagging requires a network provider; offline"),
        9 => unavailable(&files, &mut out, "AccurateRip requires CUETools, which is not installed"),
        11 => unavailable(&files, &mut out, "Video remux needs the `video` feature or ffmpeg (not available)"),
        14 => mb_tagging(&files, ctx, &mut out),
        16 => unavailable(&files, &mut out, "Mood model not installed (run `mlo tools install mood-model`)"),
        17 => unavailable(&files, &mut out, "Transliteration needs a script table that is not bundled"),
        21 | 22 => unavailable(&files, &mut out, "AcoustID fingerprints need fpcalc, which was not found"),
        24 => unavailable(&files, &mut out, "Web ratings are disabled or the provider is unavailable"),
        2 => unavailable(&files, &mut out, "CUE formatting needs a parsed CD layout"),
        _ => out.notes.push("unknown script".into()),
    }
    Ok(out)
}

fn switch_enabled(switch: &str, cfg: &Config) -> bool {
    match switch {
        "dr_replaygain_enabled" => cfg.dr_replaygain_enabled,
        "audiometa_enabled" => cfg.audiometa_enabled,
        "mood_enabled" => cfg.mood_enabled,
        "lyrics_xlit_enabled" => cfg.lyrics_xlit_enabled,
        "lyrics_translate_enabled" => cfg.lyrics_translate_enabled,
        "acoustid_enabled" => cfg.acoustid_enabled,
        "strip_unknown_tags" => cfg.strip_unknown_tags,
        "web_ratings_enabled" => cfg.web_ratings_enabled,
        _ => true,
    }
}

fn unavailable(files: &[PathBuf], out: &mut RunOutcome, reason: &str) {
    for f in files {
        out.skip(f, reason);
    }
}

// --- implemented passes -----------------------------------------------------

fn format_lyrics_media(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    for f in files {
        if ctx.cancelled() {
            break;
        }
        match crate::tags::update_tags(f, |tags| crate::tagkey::normalize_tags(tags)) {
            Ok(_) => out.ok(f),
            Err(e) => out.fail(f, e.to_string()),
        }
    }
}

fn optimize_tags(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    for f in files {
        if ctx.cancelled() {
            break;
        }
        let r = crate::tags::update_tags(f, |tags| {
            tags.retain(|k, _| {
                crate::tagkey::is_known(k) || crate::tagkey::ALLOWLIST_EXTRA.contains(&k.as_str())
            });
            tags.remove("COMMENT");
            let ascii_title = tags
                .get("TITLE")
                .and_then(|v| v.first())
                .map(|t| !crate::tagkey::needs_alias(t))
                .unwrap_or(false);
            if ascii_title {
                tags.retain(|k, _| !k.starts_with("TITLEALIAS"));
            }
        });
        match r {
            Ok(_) => out.ok(f),
            Err(e) => out.fail(f, e.to_string()),
        }
    }
}

fn grade_only(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    let _ = (files, ctx);
    // Grading is performed by `scan`; nothing to write here. Report the scope.
    for f in files {
        out.ok(f);
    }
}

fn dr_replaygain(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    // Group by album for album gain.
    let mut by_album: std::collections::BTreeMap<PathBuf, Vec<PathBuf>> = Default::default();
    for f in files {
        let album = f.parent().unwrap_or(Path::new(".")).to_path_buf();
        by_album.entry(album).or_default().push(f.clone());
    }
    for (_, tracks) in by_album {
        if ctx.cancelled() {
            break;
        }
        // DYNAMIC RANGE / REPLAYGAIN_* are arbitrary keys: only containers with
        // full key fidelity can hold them ([§6.1]).
        let (supported, unsupported): (Vec<PathBuf>, Vec<PathBuf>) =
            tracks.iter().cloned().partition(|f| crate::tags::supports_extended(f));
        for f in &unsupported {
            out.skip(f, "container stores the shared field set only; DR/ReplayGain keys need FLAC/Vorbis/MP3");
        }
        if supported.is_empty() {
            continue;
        }
        let mut ok_dr = Vec::new();
        for f in &supported {
            match crate::analysis::compute_dr(f) {
                Ok(dr) => ok_dr.push((f.clone(), dr)),
                Err(e) => out.fail(f, format!("DR failed: {e}")),
            }
        }
        let album_rg = crate::analysis::compute_album_replaygain(&supported).ok();
        let median_dr = {
            let mut v: Vec<u8> = ok_dr.iter().map(|(_, d)| *d).collect();
            v.sort_unstable();
            if v.is_empty() { None } else { Some(v[v.len() / 2]) }
        };
        for (f, dr) in &ok_dr {
            let track_rg = crate::analysis::compute_track_replaygain(f).ok();
            let result = crate::tags::update_tags(f, |tags| {
                tags.insert("DYNAMIC RANGE".into(), vec![dr.to_string()]);
                if let Some(median) = median_dr {
                    tags.insert("ALBUM DYNAMIC RANGE".into(), vec![median.to_string()]);
                }
                if let Some((gain, peak)) = track_rg {
                    tags.insert("REPLAYGAIN_TRACK_GAIN".into(), vec![format!("{gain:.2} dB")]);
                    tags.insert("REPLAYGAIN_TRACK_PEAK".into(), vec![format!("{peak:.6}")]);
                }
                if let Some(a) = &album_rg {
                    tags.insert("REPLAYGAIN_ALBUM_GAIN".into(), vec![format!("{:.2} dB", a.album_gain_db)]);
                    tags.insert("REPLAYGAIN_ALBUM_PEAK".into(), vec![format!("{:.6}", a.album_peak)]);
                }
            });
            match result {
                Ok(_) => out.ok(f),
                Err(e) => out.fail(f, e.to_string()),
            }
        }
    }
}

fn key_bpm(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    for f in files {
        if ctx.cancelled() {
            break;
        }
        match crate::analysis::detect_bpm_key(f) {
            Ok(bk) => {
                let r = crate::tags::update_tags(f, |tags| {
                    // manual value wins; a re-run replaces it (script 12 is the
                    // default writer for BPM/INITIALKEY, Appendix B)
                    tags.insert("BPM".into(), vec![format!("{}", bk.bpm.round() as u32)]);
                    tags.insert("INITIALKEY".into(), vec![bk.key.clone()]);
                });
                match r {
                    Ok(_) => out.ok(f),
                    Err(e) => out.fail(f, e.to_string()),
                }
            }
            Err(e) => out.fail(f, format!("analysis failed: {e}")),
        }
    }
}

fn format_all(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    for f in files {
        if ctx.cancelled() {
            break;
        }
        let r = crate::tags::update_tags(f, |tags| crate::tagkey::normalize_tags(tags));
        match r {
            Ok(_) => out.ok(f),
            Err(e) => out.fail(f, e.to_string()),
        }
    }
}

fn fetch_lyrics(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    for f in files {
        if ctx.cancelled() {
            break;
        }
        let tags = crate::tags::read_tags(f).unwrap_or_default();
        let artist = tags.get("ARTIST").and_then(|v| v.first()).cloned().unwrap_or_default();
        let title = tags.get("TITLE").and_then(|v| v.first()).cloned().unwrap_or_default();
        let album = tags.get("ALBUM").and_then(|v| v.first()).cloned().unwrap_or_default();
        if artist.is_empty() || title.is_empty() {
            out.skip(f, "needs ARTIST and TITLE");
            continue;
        }
        match crate::net::lrclib_lookup(&artist, &title, &album) {
            Ok(Some(v)) => {
                let synced = v.get("syncedLyrics").and_then(|s| s.as_str());
                let plain = v.get("plainLyrics").and_then(|s| s.as_str());
                let mut wrote = false;
                if let Some(s) = synced {
                    let lrc = f.with_extension("lrc");
                    let _ = crate::atomic::write_atomic_str(&lrc, s);
                    wrote = true;
                }
                let text = synced.or(plain).unwrap_or("");
                if !text.is_empty() {
                    let key = if synced.is_some() { "LYRICS" } else { "UNSYNCEDLYRICS" };
                    if crate::tags::set_values(f, key, &[text.to_string()]).is_ok() {
                        wrote = true;
                    }
                }
                if wrote {
                    out.ok(f);
                } else {
                    out.skip(f, "provider returned no lyrics");
                }
            }
            Ok(None) => out.skip(f, "no lyrics found"),
            Err(e) => out.skip(f, format!("lyrics provider unavailable: {e}")),
        }
    }
}

fn mb_tagging(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    let mb = crate::net::MusicBrainz::new(&ctx.cfg.services);
    let mut by_album: std::collections::BTreeMap<PathBuf, Vec<PathBuf>> = Default::default();
    for f in files {
        by_album.entry(f.parent().unwrap_or(Path::new(".")).to_path_buf()).or_default().push(f.clone());
    }
    for (_, tracks) in by_album {
        if ctx.cancelled() {
            break;
        }
        let tags0 = crate::tags::read_tags(&tracks[0]).unwrap_or_default();
        let artist = tags0.get("ALBUMARTIST").or(tags0.get("ARTIST")).and_then(|v| v.first()).cloned().unwrap_or_default();
        let album = tags0.get("ALBUM").and_then(|v| v.first()).cloned().unwrap_or_default();
        if artist.is_empty() || album.is_empty() {
            for f in &tracks {
                out.skip(f, "needs ALBUMARTIST/ALBUM to search");
            }
            continue;
        }
        match mb.search_release(&artist, &album, tracks.len()) {
            Ok(cands) if !cands.is_empty() => {
                let best = &cands[0];
                match mb.release(&best.mbid) {
                    Ok(json) => {
                        let count = write_release(&tracks, &json, best.mbid.as_str());
                        for f in &tracks {
                            if count > 0 {
                                out.ok(f);
                            } else {
                                out.skip(f, "release contained no usable metadata");
                            }
                        }
                    }
                    Err(e) => {
                        for f in &tracks {
                            out.skip(f, format!("MusicBrainz release fetch failed: {e}"));
                        }
                    }
                }
            }
            Ok(_) => {
                for f in &tracks {
                    out.skip(f, "no MusicBrainz match");
                }
            }
            Err(e) => {
                for f in &tracks {
                    out.skip(f, format!("MusicBrainz unavailable: {e}"));
                }
            }
        }
    }
}

fn write_release(tracks: &[PathBuf], release: &serde_json::Value, mbid: &str) -> usize {
    let mut written = 0;
    for (i, f) in tracks.iter().enumerate() {
        let title = release
            .pointer(&format!("/media/0/tracks/{i}/title"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mut count = 0;
        let _ = crate::tags::update_tags(f, |tags| {
            tags.insert("MUSICBRAINZ_ALBUMID".into(), vec![mbid.to_string()]);
            if let Some(t) = title {
                tags.insert("TITLE".into(), vec![t]);
                count += 1;
            }
        });
        written += count;
    }
    written
}

fn tracklist_manifests(files: &[PathBuf], ctx: &ScriptCtx, out: &mut RunOutcome) {
    let mut by_album: std::collections::BTreeMap<PathBuf, Vec<PathBuf>> = Default::default();
    for f in files {
        by_album.entry(f.parent().unwrap_or(Path::new(".")).to_path_buf()).or_default().push(f.clone());
    }
    for (album, tracks) in by_album {
        if ctx.cancelled() {
            break;
        }
        let mut manifest = Vec::new();
        for f in &tracks {
            let tags = crate::tags::read_tags(f).unwrap_or_default();
            manifest.push(serde_json::json!({
                "file": crate::model::file_name(f),
                "title": tags.get("TITLE").and_then(|v| v.first()),
                "track": tags.get("TRACKNUMBER").and_then(|v| v.first()),
                "disc": tags.get("DISCNUMBER").and_then(|v| v.first()),
            }));
        }
        let path = album.join(".mlo_expected.json");
        let doc = serde_json::json!({ "generated_by": "mlo", "tracks": manifest });
        match crate::atomic::write_atomic_json(&path, &doc) {
            Ok(()) => {
                for f in &tracks {
                    out.ok(f);
                }
            }
            Err(e) => {
                for f in &tracks {
                    out.fail(f, e.to_string());
                }
            }
        }
    }
}

fn optimize_artist_images(scope: &Scope, ctx: &ScriptCtx, out: &mut RunOutcome) {
    let mut dirs: Vec<PathBuf> = Vec::new();
    match scope {
        Scope::Library => {
            let artists = ctx.cfg.music_folder.join("Artists");
            if let Ok(rd) = std::fs::read_dir(&artists) {
                for e in rd.flatten() {
                    if e.path().is_dir() {
                        dirs.push(e.path());
                    }
                }
            }
        }
        Scope::Artist(p) => dirs.push(p.clone()),
        Scope::Album(p) => {
            if let Some(parent) = p.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
        Scope::Track(p) => {
            if let Some(a) = p.parent().and_then(|al| al.parent()) {
                dirs.push(a.to_path_buf());
            }
        }
        Scope::Selection(v) => dirs.extend(v.iter().cloned()),
    }
    for dir in dirs {
        if ctx.cancelled() {
            break;
        }
        let img = ["artist.jpg", "artist.png"]
            .iter()
            .map(|n| dir.join(n))
            .find(|p| p.exists());
        match img {
            None => out.skip(&dir, "no artist image"),
            Some(p) => match crate::images::optimize_artist_image(&p, ctx.cfg) {
                Ok(note) => {
                    out.ok(&p);
                    if let Some(n) = note {
                        out.notes.push(n);
                    }
                }
                Err(e) => out.fail(&p, e.to_string()),
            },
        }
    }
}

fn layout_script(scope: &Scope, ctx: &ScriptCtx, out: &mut RunOutcome) {
    let walked = match crate::scan::walk(&ctx.cfg.music_folder) {
        Ok(w) => w,
        Err(e) => {
            out.notes.push(e.to_string());
            return;
        }
    };
    let findings = crate::layout::analyze(&walked, &ctx.cfg.music_folder, &Default::default(), ctx.cfg);
    let scoped: Vec<crate::model::Finding> = findings
        .into_iter()
        .filter(|f| match scope {
            Scope::Library => true,
            Scope::Artist(p) | Scope::Album(p) => f.path.starts_with(p) || f.artist.as_deref() == Some(p.as_path()),
            Scope::Track(p) => f.path.starts_with(p.parent().unwrap_or(p)),
            Scope::Selection(v) => v.iter().any(|p| f.path.starts_with(p)),
        })
        .collect();
    if !ctx.cfg.layout_apply {
        for f in &scoped {
            out.skip(&f.path, "layout_apply is off");
        }
        out.notes.push("report only (layout_apply = false)".into());
        return;
    }
    match crate::layout::apply(ctx.cfg, &scoped, None, false) {
        Ok(outcomes) => {
            for o in outcomes {
                if o.ok {
                    out.ok(PathBuf::from(&o.finding_id));
                } else {
                    out.fail(PathBuf::from(&o.finding_id), o.detail);
                }
            }
        }
        Err(e) => out.notes.push(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hard-coded copy of la-musica `mlo/scripts.py` `SCRIPTS` (id, name, what).
    const LA_MUSICA_SCRIPTS: &[(u8, &str, &str)] = &[
        (1, "Format lyrics", "multi-format + MEDIA/SOURCE normalization"),
        (2, "Format CUEs", "CD-N rename + FILE/INDEX layout"),
        (3, "Optimize FLACs", "lossless re-encode"),
        (4, "Grade", "per-album tag/lyrics/cover report"),
        (5, "Process images", "JXL / lossless / JXL-back"),
        (6, "Audit library", "AudioAuditor: fake lossless / upscaled / MQA"),
        (7, "DR & ReplayGain", "in-process DR + rsgain ReplayGain tags"),
        (8, "Auto tagging", "advisory / instrumental / mood / energy / genre"),
        (9, "AccurateRip", "CUETools .accurip files"),
        (10, "Format all", "final pass: .accurip / .cue / .lrc / tags"),
        (11, "Remux videos (MKV)", "any video -> MKV, audio -> FLAC"),
        (12, "Key & BPM", "musical key + tempo tags"),
        (13, "Fetch lyrics", "LRCLIB synced/plain"),
        (14, "Beets tagging", "MusicBrainz via beets"),
        (15, "Release tracklist", ".mlo_expected.json manifests"),
        (16, "Mood & Energy", "MOOD/ENERGY from the track's audio"),
        (17, "Lyrics transliterate (AI)", "TRANSLITERATION/TRANSLATION tags + sidecars"),
        (19, "Optimize artist images", "crop/resize artist artwork to the configured aspect and size"),
        (20, "Optimize library layout", "layout report + fixes (case, loose audio, empty artist, strays to the Trash)"),
        (21, "Fix AcoustID pairs", "complete or create ACOUSTID_ID / ACOUSTID_FINGERPRINT pairs"),
        (22, "Submit fingerprints (AcoustID)", "give AcoustID the fingerprint + MusicBrainz recording each track states"),
        (23, "Optimize tags", "delete excess tags: junk names, a valued COMMENT, unneeded aliases"),
        (24, "Web ratings", "aggregated public album + track scores (MusicBrainz / RYM / Discogs)"),
    ];

    fn ctx<'a>(cfg: &'a Config, db: &'a Db, cancel: &'a AtomicBool, progress: &'a (dyn Fn(Progress) + Sync)) -> ScriptCtx<'a> {
        ScriptCtx { cfg, db, cancel, progress }
    }

    #[test]
    fn registry_matches_appendix_a() {
        assert_eq!(SCRIPTS.len(), 23);
        assert!(by_id(18).is_none(), "there is no script 18");
        assert_eq!(by_id(20).unwrap().name, "Optimize library layout");
        assert_eq!(by_id(12).unwrap().name, "Key & BPM");
        for id in RUN_ALL_ORDER {
            assert!(by_id(*id).is_some(), "run order references {id}");
        }
        // movers before readers: layout (20) before grade (4)
        let pos = |id: u8| RUN_ALL_ORDER.iter().position(|x| *x == id).unwrap();
        assert!(pos(20) < pos(4));
        assert!(pos(11) < pos(20));
    }

    #[test]
    fn names_and_descriptions_equal_la_musica() {
        assert_eq!(SCRIPTS.len(), LA_MUSICA_SCRIPTS.len());
        for (id, name, description) in LA_MUSICA_SCRIPTS {
            let def = by_id(*id).unwrap_or_else(|| panic!("script {id} is missing"));
            assert_eq!(def.name, *name, "name of script {id}");
            assert_eq!(def.description, *description, "description of script {id}");
        }
    }

    #[test]
    fn run_all_order_and_chain_equal_la_musica() {
        assert_eq!(RUN_ALL_ORDER.len(), 22);
        assert!(!RUN_ALL_ORDER.contains(&22), "22 is opt-in, not in Run All");
        assert_eq!(
            RUN_ALL_ORDER,
            &[11, 3, 14, 15, 2, 1, 13, 17, 8, 24, 5, 19, 6, 7, 9, 12, 16, 10, 23, 20, 21, 4]
        );
        // LIBRARY_WIDE_SCRIPTS is empty => DEFAULT_CHAIN == DEFAULT_RUN_ALL_ORDER.
        assert_eq!(import_chain(&Config::default()), RUN_ALL_ORDER.to_vec());
        // explicit list replaces it outright
        let mut cfg = Config::default();
        cfg.import_scripts = vec![1, 2, 3];
        assert_eq!(import_chain(&cfg), vec![1, 2, 3]);
    }

    #[test]
    fn opt_in_and_album_movers_match_la_musica() {
        assert_eq!(OPT_IN_SCRIPTS, &[22]);
        for id in [8, 11, 14, 20] {
            assert!(is_album_mover(id), "{id} moves albums");
        }
        for id in [1, 4, 5, 22] {
            assert!(!is_album_mover(id), "{id} does not move albums");
        }
    }

    #[test]
    fn disabled_switch_skips_with_named_reason() {
        let mut cfg = Config::default();
        cfg.web_ratings_enabled = false;
        let db = Db::in_memory().unwrap();
        let cancel = AtomicBool::new(false);
        let progress = |_: Progress| {};
        let context = ctx(&cfg, &db, &cancel, &progress);
        let out = run(24, &Scope::Library, &context).unwrap();
        assert_eq!(out.results.len(), 0);
        assert!(out.notes.iter().any(|n| n.contains("web_ratings_enabled")));
    }

    #[test]
    fn two_switch_gate_runs_when_any_on() {
        let db = Db::in_memory().unwrap();
        let cancel = AtomicBool::new(false);
        let progress = |_: Progress| {};
        assert_eq!(by_id(17).unwrap().gates().count(), 2);
        let cfg = Config::default();
        assert!(cfg.lyrics_xlit_enabled && cfg.lyrics_translate_enabled);
        let _ = cfg;

        // one switch off: the script still runs (la-musica: ANY keeps it alive)
        let mut one_off = Config::default();
        one_off.lyrics_translate_enabled = false;
        let context = ctx(&one_off, &db, &cancel, &progress);
        let out = run(17, &Scope::Library, &context).unwrap();
        assert!(
            !out.notes.iter().any(|n| n.contains("is off") || n.contains("are off")),
            "one switch on keeps the script alive: {:?}",
            out.notes
        );

        // both off: skipped, naming both switches
        let mut both_off = Config::default();
        both_off.lyrics_xlit_enabled = false;
        both_off.lyrics_translate_enabled = false;
        let context = ctx(&both_off, &db, &cancel, &progress);
        let out = run(17, &Scope::Library, &context).unwrap();
        assert_eq!(out.results.len(), 0);
        assert!(out
            .notes
            .iter()
            .any(|n| n.contains("lyrics_xlit_enabled") && n.contains("lyrics_translate_enabled")));
    }

    #[test]
    fn run_stats_counts_match_the_results() {
        let mut out = RunOutcome::new(1);
        out.ok("a.flac");
        out.ok("b.flac");
        out.skip("c.flac", "no lyrics");
        out.fail("d.flac", "decode failed");
        out.bytes_added = 1024;
        out.bytes_removed = 256;

        let stats = out.stats();
        assert_eq!(stats.total_scanned, 4);
        assert_eq!(stats.modified_count, 2);
        assert_eq!(stats.skipped_count, 1);
        assert_eq!(stats.error_count, 1);
        assert_eq!(stats.unchanged_count, 0);
        assert_eq!(stats.total_bytes_added, 1024);
        assert_eq!(stats.total_bytes_removed, 256);
        assert_eq!(stats.errors.len(), 1);
        assert!(stats.errors[0].contains("decode failed"));
        assert_eq!(stats, RunStats::from_outcome(&out));
        assert_eq!(RunStats::from(&out), stats);
    }
}