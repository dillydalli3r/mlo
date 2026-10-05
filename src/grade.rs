//! Grading ([§9]) — binary, counted, and every displayed issue is charged.
//!
//! One registry (Appendix C) is read by the TUI settings screen, the CLI and the
//! grader. `pass_count = total_checks - failed_checks`; a check that raises is
//! counted as failed; `Not applicable` is counted on neither side.

use crate::config::Config;
use crate::model::{CheckResult, Finding, Issue, LibraryVerdict, TagMap};
use crate::naming::{self, EvalResult, NamingOptions};
use crate::tagkey::{self};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckFamily {
    Identity,
    Release,
    Audio,
    Lyrics,
    Covers,
    Acoustid,
    Description,
    Artist,
    Layout,
    Include,
}

impl CheckFamily {
    pub fn label(self) -> &'static str {
        match self {
            CheckFamily::Identity => "Identity",
            CheckFamily::Release => "Release",
            CheckFamily::Audio => "Audio",
            CheckFamily::Lyrics => "Lyrics",
            CheckFamily::Covers => "Covers",
            CheckFamily::Acoustid => "AcoustID",
            CheckFamily::Description => "Description",
            CheckFamily::Artist => "Artist",
            CheckFamily::Layout => "Layout",
            CheckFamily::Include => "Files",
        }
    }
}

pub struct CheckDef {
    pub key: &'static str,
    pub label: &'static str,
    pub family: CheckFamily,
    pub default: bool,
    pub issue_codes: &'static [&'static str],
}

/// The check registry — la-musica `server/tags_registry.py` `CHECK_LABELS`
/// (keys, labels and defaults; every one ships ON, `mlo/config.py`).
/// `grade_include_*` keys are file-category participation toggles.
pub const CHECK_DEFS: &[CheckDef] = &[
    def("grade_check_unreadable", "Unreadable files", CheckFamily::Identity, true, &["UNREADABLE"]),
    def("grade_check_missing_tags", "Required tags", CheckFamily::Identity, true, &["TITLE", "ARTIST", "ALBUM", "ALBUMARTIST", "DATE", "TRACKNUMBER", "DISCNUMBER", "GENRE", "MOOD", "ENERGY", "INSTRUMENTAL", "DYNAMIC RANGE"]),
    def("grade_check_album_tags", "Album-level tags", CheckFamily::Release, true, &["ALBUMITUNESADVISORY", "ALBUM DYNAMIC RANGE"]),
    def("grade_check_mood", "Mood tag present", CheckFamily::Audio, true, &["MOOD_MISSING"]),
    def("grade_check_energy", "Energy tag present", CheckFamily::Audio, true, &["ENERGY_MISSING"]),
    def("grade_check_genre", "Genre tag present", CheckFamily::Release, true, &["GENRE_MISSING"]),
    def("grade_check_genre_count", "Genre count per track", CheckFamily::Release, true, &["GENRE_COUNT"]),
    def("grade_check_genre_order", "Genre order (family first)", CheckFamily::Release, true, &["GENRE_ORDER"]),
    def("grade_check_genre_vocab", "Genre vocabulary", CheckFamily::Release, true, &["GENRE_VOCAB"]),
    def("grade_check_replaygain", "ReplayGain tags present", CheckFamily::Audio, true, &["REPLAYGAIN_TRACK_GAIN", "REPLAYGAIN_TRACK_PEAK"]),
    def("grade_check_encoder", "Encoder identity", CheckFamily::Audio, true, &["ENCODER_PROGRAM", "ENCODER_QUALITY", "ENCODER_VERSION"]),
    def("grade_check_naming", "Naming script match", CheckFamily::Identity, true, &["PATH", "PATH_CASE"]),
    def("grade_check_filename_case", "Path capitalization", CheckFamily::Identity, true, &["PATH_CASE"]),
    def("grade_check_ext_case", "Lowercase extensions", CheckFamily::Identity, true, &["EXT_CASE"]),
    def("grade_check_key_bpm", "Key & BPM", CheckFamily::Audio, true, &["INITIALKEY", "BPM"]),
    def("grade_check_acoustid", "AcoustID tags required", CheckFamily::Acoustid, true, &["ACOUSTID_ID", "ACOUSTID_FINGERPRINT"]),
    def("grade_check_alias_needed", "Locale alias for non-Latin names", CheckFamily::Identity, true, &["TITLEALIAS", "ARTISTALIAS", "ALBUMALIAS"]),
    def("grade_check_alias_excess", "Locale alias only where needed", CheckFamily::Identity, true, &["TITLEALIAS", "ARTISTALIAS", "ALBUMALIAS"]),
    def("grade_check_excess_tags", "Excess tags", CheckFamily::Identity, true, &["TAGS", "COMMENT"]),
    def("grade_check_media", "Media type", CheckFamily::Release, true, &["MEDIA"]),
    def("grade_check_source", "Source tag", CheckFamily::Release, true, &["SOURCE"]),
    def("grade_check_instrumental", "Instrumental consistency", CheckFamily::Audio, true, &["INSTRUMENTAL"]),
    def("grade_check_disallowed", "Disallowed file types", CheckFamily::Identity, true, &["DISALLOWED_FILE"]),
    def("grade_check_extra_images", "Stray images", CheckFamily::Covers, true, &["EXTRA_IMAGES"]),
    def("grade_check_empty_folders", "Empty folders", CheckFamily::Identity, true, &["EMPTY_FOLDER"]),
    def("grade_check_expected_tracks", "Whole release present", CheckFamily::Identity, true, &["EXPECTED_TRACKS_MISSING", "EXPECTED_TRACKS_INCOMPLETE"]),
    def("grade_check_album_description", "Album description stored", CheckFamily::Description, true, &["ALBUM_DESCRIPTION"]),
    def("grade_check_raw_video", "Raw videos", CheckFamily::Audio, true, &["RAW_VIDEO"]),
    def("grade_check_lossless_source", "Lossless sources", CheckFamily::Audio, true, &["LOSSLESS_SOURCE"]),
    def("grade_check_disc_naming", "Disc rip-sheet naming", CheckFamily::Audio, true, &["DISC_NAMING"]),
    def("grade_check_cd_log", "CD — .log present", CheckFamily::Audio, true, &["CD_LOG"]),
    def("grade_check_cd_cue", "CD — .cue present", CheckFamily::Audio, true, &["CD_CUE"]),
    def("grade_check_cd_format", "CD — lossless format", CheckFamily::Audio, true, &["CD_FORMAT"]),
    def("grade_check_crc", "CRC checksums", CheckFamily::Audio, true, &["CRC", "CRC_MISMATCH"]),
    def("grade_check_artist_image", "Artist image stored", CheckFamily::Artist, true, &["ARTIST_IMAGE_MISSING", "ARTIST_IMAGE_CORRUPT", "ARTIST_IMAGE_FORMAT", "ARTIST_IMAGE_OVERSIZED", "ARTIST_IMAGE_ASPECT", "ARTIST_IMAGE_UPSCALED"]),
    def("grade_check_artist_description", "Artist description stored", CheckFamily::Artist, true, &["ARTIST_DESCRIPTION_MISSING", "ARTIST_FOLDER_MISSING", "ARTIST_EMPTY"]),
    def("grade_check_audit", "Require audit tag", CheckFamily::Audio, true, &["AUDIT"]),
    def("grade_check_flac_md5", "FLAC stream MD5 (STREAMINFO)", CheckFamily::Audio, true, &["FLAC_MD5", "FLAC_MD5_ABSENT", "FLAC_MD5_UNKNOWN"]),
    def("grade_check_log_checksum", "Log checksum valid", CheckFamily::Audio, true, &["LOG_CHECKSUM"]),
    def("grade_check_accuraterip", "AccurateRip verified (audit only)", CheckFamily::Audio, true, &["ACCURATERIP"]),
    def("grade_check_log_grade", "Log grade present & in range", CheckFamily::Audio, true, &["LOG_GRADE"]),
    def("grade_check_mb_links", "MusicBrainz release link", CheckFamily::Release, true, &["MB_LINK"]),
    def("grade_check_rym_links", "RateYourMusic release link", CheckFamily::Release, true, &["RYM_LINK"]),
    def("grade_check_cover", "Cover art", CheckFamily::Covers, true, &["COVER"]),
    def("grade_check_cover_crop", "Cover aspect ratio (squareness)", CheckFamily::Covers, true, &["COVER_CROP"]),
    def("grade_check_sidecar_cover", "Per-track sidecar covers", CheckFamily::Covers, true, &["COVER"]),
    def("grade_check_tag_spaces", "Tags — no padding", CheckFamily::Identity, true, &["TAG_SPACES"]),
    def("grade_check_tag_case", "Tags — canonical value case", CheckFamily::Identity, true, &["TAG_CASE"]),
    def("grade_check_tag_blank_lines", "Tags — no blank lines", CheckFamily::Identity, true, &["TAG_BLANK_LINES"]),
    def("grade_check_lyrics_spaces", "Lyrics — no padding", CheckFamily::Lyrics, true, &["LYRICS_SPACES"]),
    def("grade_check_lyrics_blank_lines", "Lyrics — blank line rules", CheckFamily::Lyrics, true, &["LYRICS_BLANK_LINES"]),
    def("grade_check_lyrics_zero", "Lyrics — zero timestamp rule", CheckFamily::Lyrics, true, &["LYRICS_ZERO"]),
    def("grade_check_lyrics_format", "Lyrics — canonical formatting", CheckFamily::Lyrics, true, &["LYRICS"]),
    def("grade_check_cue_spaces", "CUE — no padding", CheckFamily::Audio, true, &["CUE_SPACES"]),
    def("grade_check_cue_blank_lines", "CUE — no blank lines", CheckFamily::Audio, true, &["CUE_BLANK_LINES"]),
    def("grade_check_cue_format", "CUE — canonical formatting", CheckFamily::Audio, true, &["CUE_FORMAT"]),
    def("grade_check_accurip_format", ".accurip — canonical formatting", CheckFamily::Audio, true, &["ACCURIP_FORMAT"]),
    def("grade_check_cue_files", "CUE — referenced files exist", CheckFamily::Audio, true, &["CUE_FILES"]),
    def("grade_check_lyrics", "Lyrics present", CheckFamily::Lyrics, true, &["LYRICS"]),
    def("grade_check_lyrics_lang_tags", "Transform language tags", CheckFamily::Lyrics, true, &["LYRICS_LANG_TAGS"]),
    def("grade_check_xlit_transliteration", "Transliteration — needed, never extra", CheckFamily::Lyrics, true, &["XLIT_MISSING", "XLIT_UNNEEDED"]),
    def("grade_check_xlit_translation", "Translation — needed, never extra", CheckFamily::Lyrics, true, &["XLIT_MISSING", "XLIT_UNNEEDED"]),
    // File-category participation (grade_include_*)
    def("grade_include_music", "Audio tracks", CheckFamily::Include, true, &[]),
    def("grade_include_cover", "Cover art", CheckFamily::Include, true, &[]),
    def("grade_include_description", "Album description", CheckFamily::Include, true, &[]),
    def("grade_include_cue", "CUE sheets", CheckFamily::Include, true, &[]),
    def("grade_include_log", "Log files", CheckFamily::Include, true, &[]),
    def("grade_include_lrc", "LRC lyrics", CheckFamily::Include, true, &[]),
    def("grade_include_accurip", "AccurateRip files", CheckFamily::Include, true, &[]),
    def("grade_include_video", "Remuxed videos", CheckFamily::Include, true, &[]),
    def("grade_include_other", "Other files", CheckFamily::Include, true, &[]),
];

const fn def(
    key: &'static str,
    label: &'static str,
    family: CheckFamily,
    default: bool,
    issue_codes: &'static [&'static str],
) -> CheckDef {
    CheckDef { key, label, family, default, issue_codes }
}

/// The layout checks mlo charges (§9.2 of the Rust brief) — an extension of
/// the la-musica registry, one key per finding kind.
pub fn layout_check_keys() -> Vec<(&'static str, String)> {
    crate::model::FindingKind::ALL
        .iter()
        .map(|k| (k.code(), k.check_key()))
        .collect()
}

/// Map an internal short check name to its la-musica config key.
pub fn long_of(short: &str) -> String {
    match short {
        "cr" => "grade_check_crc".to_string(),
        s if s.starts_with("grade_check_") || s.starts_with("grade_include_") => s.to_string(),
        s if s.starts_with("layout_") || s == "library" => s.to_string(),
        s => format!("grade_check_{s}"),
    }
}

/// Show a check's la-musica key in the UI/report.
pub fn display_key(short: &str) -> String {
    long_of(short)
}

pub fn default_check_enabled(key: &str) -> bool {
    if let Some(d) = CHECK_DEFS.iter().find(|d| d.key == key) {
        return d.default;
    }
    // layout checks and unknown keys are ON (la-musica ships every check ON)
    true
}

pub fn check_label(key: &str) -> String {
    if let Some(d) = CHECK_DEFS.iter().find(|d| d.key == key) {
        return d.label.to_string();
    }
    if let Some(kind) = finding_kind_for_check(key) {
        return kind.label().to_string();
    }
    // humanise "grade_check_foo_bar" -> "Foo bar" like la-musica `_humanize`
    let tail = key
        .strip_prefix("grade_check_")
        .or_else(|| key.strip_prefix("grade_include_"))
        .unwrap_or(key);
    let mut s = tail.replace('_', " ");
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s
}

fn finding_kind_for_check(key: &str) -> Option<crate::model::FindingKind> {
    let code = key.strip_prefix("layout_")?;
    crate::model::FindingKind::ALL.iter().copied().find(|k| k.code() == code)
}

pub fn all_check_keys() -> Vec<String> {
    let mut v: Vec<String> = CHECK_DEFS.iter().map(|d| d.key.to_string()).collect();
    for k in crate::model::FindingKind::ALL {
        v.push(k.check_key());
    }
    v
}

// ---------------------------------------------------------------------------
// Presets ([§4] of the la-musica spec): Strict / Balanced / Relaxed
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Every check on — the shipped default and the identity preset.
    Strict,
    /// The pre-strict set: audit and "other files" off.
    Balanced,
    /// The 18 formatting/link/image checks off.
    Relaxed,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Preset::Strict => "Strict",
            Preset::Balanced => "Balanced",
            Preset::Relaxed => "Relaxed",
        }
    }
}

/// The 18 keys Relaxed switches off (la-musica spec R20).
pub const RELAXED_OFF: &[&str] = &[
    "grade_check_tag_spaces",
    "grade_check_tag_case",
    "grade_check_lyrics_spaces",
    "grade_check_cue_spaces",
    "grade_check_cover_crop",
    "grade_check_lyrics_zero",
    "grade_check_tag_blank_lines",
    "grade_check_lyrics_blank_lines",
    "grade_check_cue_blank_lines",
    "grade_check_filename_case",
    "grade_check_ext_case",
    "grade_check_excess_tags",
    "grade_check_mb_links",
    "grade_check_rym_links",
    "grade_check_replaygain",
    "grade_check_album_description",
    "grade_check_artist_image",
    "grade_check_artist_description",
];

/// The keys Balanced switches off.
pub const BALANCED_OFF: &[&str] = &["grade_check_audit", "grade_include_other"];

impl crate::config::Config {
    /// Apply a preset to the config's check overrides.
    pub fn apply_preset(&mut self, preset: Preset) {
        self.checks.clear();
        match preset {
            Preset::Strict => {}
            Preset::Balanced => {
                for k in BALANCED_OFF {
                    self.checks.insert((*k).to_string(), false);
                }
            }
            Preset::Relaxed => {
                for k in RELAXED_OFF {
                    self.checks.insert((*k).to_string(), false);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Views — what the grader needs, built by `scan`
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SidecarInfo {
    pub cover: Option<PathBuf>,
    pub cover_dims: Option<(u32, u32)>,
    pub cover_aspect_ok: Option<bool>,
    pub extra_images: u32,
    pub description: Option<String>,
    pub cue: Option<PathBuf>,
    pub log: Option<PathBuf>,
    pub accurip: Option<PathBuf>,
    pub expected_tracks: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct TrackView {
    pub path: PathBuf,
    pub tags: TagMap,
    pub is_video: bool,
    pub decoded_ok: bool,
    pub has_lyrics_sidecar: bool,
    pub lyrics_sidecar_synced: bool,
    /// `None` = not computed / not a FLAC.
    pub flac_md5_ok: Option<bool>,
}

impl TrackView {
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key).and_then(|v| v.first()).map(|s| s.as_str())
    }
    pub fn has(&self, key: &str) -> bool {
        self.tag(key).map(|s| !s.trim().is_empty()).unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default)]
pub struct AlbumView {
    pub path: PathBuf,
    pub artist: String,
    pub title: String,
    pub tracks: Vec<TrackView>,
    pub side: SidecarInfo,
    pub findings: Vec<Finding>,
    pub is_cd: bool,
    /// Audio files present (even if unreadable) — drives `empty_folders`.
    pub audio_file_count: usize,
}

#[derive(Debug, Clone)]
pub struct AlbumGrade {
    pub report: crate::model::GradeReport,
    pub track_reports: Vec<(PathBuf, crate::model::GradeReport)>,
}

#[derive(Debug, Clone, Default)]
pub struct ArtistView {
    pub name: String,
    pub path: PathBuf,
    pub has_albums: bool,
    pub has_image: bool,
    pub image_ok: Option<bool>,
    pub image_upscaled: bool,
    pub has_description: bool,
    pub findings: Vec<Finding>,
}

/// Required per-track tags (la-musica `PER_TRACK_TAGS`).
const REQUIRED_TAGS: &[&str] = &[
    "TITLE", "ARTIST", "ALBUM", "ALBUMARTIST", "DATE", "TRACKNUMBER", "GENRE", "MOOD", "ENERGY",
    "INSTRUMENTAL", "DYNAMIC RANGE",
];
const ALBUM_REQUIRED: &[&str] = &["ALBUMARTIST", "RELEASETYPE", "LABEL", "CATALOGNUMBER", "BARCODE", "DATE"];

fn enabled(cfg: &Config, key: &str) -> bool {
    cfg.check_enabled(&long_of(key))
}

// ---------------------------------------------------------------------------
// Track grade
// ---------------------------------------------------------------------------

pub fn grade_track(tv: &TrackView, cfg: &Config) -> crate::model::GradeReport {
    let mut checks = Vec::new();

    if enabled(cfg, "missing_tags") {
        let missing: Vec<&str> = REQUIRED_TAGS.iter().copied().filter(|k| !tv.has(k)).collect();
        checks.push(if missing.is_empty() {
            CheckResult::pass("missing_tags", "Required tags")
        } else {
            // la-musica's per-tag codes: the issue is the TAG, naming every one
            CheckResult::fail(
                "missing_tags",
                "Required tags",
                missing[0],
                format!("missing {}", missing.join(", ")),
            )
        });
    } else {
        checks.push(CheckResult::skipped("missing_tags", "Required tags"));
    }

    if enabled(cfg, "tag_case") {
        let bad: Vec<String> = tv
            .tags
            .iter()
            .flat_map(|(k, vs)| vs.iter().filter(move |v| tagkey::needs_case_fix(k, v)).map(move |v| format!("{k}={v}")))
            .collect();
        checks.push(if bad.is_empty() {
            CheckResult::pass("tag_case", "Canonical tag spelling")
        } else {
            CheckResult::fail("tag_case", "Canonical tag spelling", "TAG_CASE", bad.join(", "))
        });
    } else {
        checks.push(CheckResult::skipped("tag_case", "Canonical tag spelling"));
    }

    if enabled(cfg, "tag_spaces") {
        let bad: Vec<String> = tv
            .tags
            .iter()
            .flat_map(|(k, vs)| vs.iter().filter(move |v| !tagkey::spacing_problem(k, v).is_empty()).map(move |_| k.clone()))
            .collect();
        checks.push(if bad.is_empty() {
            CheckResult::pass("tag_spaces", "Canonical spacing")
        } else {
            CheckResult::fail("tag_spaces", "Canonical spacing", "TAG_SPACES", format!("tags: {}", uniq_join(&bad)))
        });
    } else {
        checks.push(CheckResult::skipped("tag_spaces", "Canonical spacing"));
    }

    if enabled(cfg, "tag_blank_lines") {
        let bad: Vec<String> = tv
            .tags
            .iter()
            .flat_map(|(k, vs)| {
                let is_lyrics = k == "LYRICS" || k == "UNSYNCEDLYRICS";
                let key = k.clone();
                vs.iter()
                    .filter(move |v| v.trim().is_empty() || (v.contains('\n') && !is_lyrics))
                    .map(move |_| key.clone())
            })
            .collect();
        checks.push(if bad.is_empty() {
            CheckResult::pass("tag_blank_lines", "No blank/only-whitespace values")
        } else {
            CheckResult::fail("tag_blank_lines", "No blank/only-whitespace values", "TAG_BLANK_LINES", uniq_join(&bad))
        });
    }

    if enabled(cfg, "excess_tags") {
        let foreign: Vec<String> = tv
            .tags
            .keys()
            .filter(|k| !tagkey::is_known(k) && !tagkey::ALLOWLIST_EXTRA.contains(&k.as_str()))
            .cloned()
            .collect();
        // A non-empty COMMENT fails with code COMMENT (la-musica §7.6).
        let comment = tv.tag("COMMENT").filter(|v| !v.trim().is_empty());
        checks.push(if let Some(c) = comment {
            CheckResult::fail("excess_tags", "Excess tags", "COMMENT", format!("COMMENT is not empty: {c}"))
        } else if foreign.is_empty() {
            CheckResult::pass("excess_tags", "Excess tags")
        } else {
            CheckResult::fail("excess_tags", "Excess tags", "TAGS", uniq_join(&foreign))
        });
    }

    if enabled(cfg, "alias_needed") {
        let title = tv.tag("TITLE").unwrap_or("");
        let needs_alias = tagkey::needs_alias(title);
        let has_alias = tv.tags.keys().any(|k| k == "TITLEALIAS" || k.starts_with("TITLEALIAS_"));
        checks.push(if !needs_alias || has_alias {
            CheckResult::pass("alias_needed", "Alias tag present when needed")
        } else {
            CheckResult::fail("alias_needed", "Alias tag present when needed", "ALIAS_NEEDED", "TITLEALIAS required for a non-ASCII title".to_string())
        });
    }

    if enabled(cfg, "alias_excess") {
        let title = tv.tag("TITLE").unwrap_or("");
        let has_alias = tv.tags.keys().any(|k| k == "TITLEALIAS" || k.starts_with("TITLEALIAS_"));
        let excess = has_alias && !tagkey::needs_alias(title);
        checks.push(if excess {
            CheckResult::fail("alias_excess", "No unneeded alias tags", "ALIAS_EXCESS", "TITLEALIAS present for an ASCII title".to_string())
        } else {
            CheckResult::pass("alias_excess", "No unneeded alias tags")
        });
    }

    if enabled(cfg, "ext_case") {
        let ok = naming::lowercase_ext(&tv.path);
        checks.push(if ok {
            CheckResult::pass("ext_case", "File extensions are lowercase")
        } else {
            CheckResult::fail("ext_case", "File extensions are lowercase", "EXT_CASE", "extension is not lowercase".to_string())
        });
    }

    if enabled(cfg, "unreadable") {
        checks.push(if tv.decoded_ok {
            CheckResult::pass("unreadable", "Every file is readable/decodable")
        } else {
            CheckResult::fail("unreadable", "Every file is readable/decodable", "UNREADABLE", "decode failed".to_string())
        });
    }

    if enabled(cfg, "flac_md5") {
        match tv.flac_md5_ok {
            Some(true) => checks.push(CheckResult::pass("flac_md5", "FLAC STREAMINFO md5 verified")),
            Some(false) => checks.push(CheckResult::fail("flac_md5", "FLAC STREAMINFO md5 verified", "FLAC_MD5", "md5 mismatch".to_string())),
            None => checks.push(CheckResult::skipped("flac_md5", "FLAC STREAMINFO md5 verified")),
        }
    }

    for (key, label) in [
        ("key_bpm", "BPM and key present"),
        ("mood", "MOOD present"),
        ("energy", "ENERGY present"),
        ("replaygain", "ReplayGain tags present"),
        ("encoder", "ENCODER_* provenance present"),
        ("instrumental", "INSTRUMENTAL decided"),
    ] {
        if !enabled(cfg, key) {
            checks.push(CheckResult::skipped(key, label));
            continue;
        }
        let required: &[&str] = match key {
            "key_bpm" => &["BPM", "INITIALKEY"],
            "replaygain" => &["REPLAYGAIN_TRACK_GAIN", "REPLAYGAIN_TRACK_PEAK"],
            "encoder" => &["ENCODER_PROGRAM"],
            _ => &[],
        };
        let ok = match key {
            "mood" => tv.has("MOOD"),
            "energy" => tv.has("ENERGY"),
            "instrumental" => tv.has("INSTRUMENTAL"),
            _ => required.iter().all(|k| tv.has(k)),
        };
        let opt_out = matches!(key, "key_bpm" if !cfg.audiometa_enabled)
            || matches!(key, "mood" | "energy" if !cfg.mood_enabled);
        if opt_out {
            checks.push(CheckResult::skipped(key, label));
        } else {
            checks.push(if ok {
                CheckResult::pass(key, label)
            } else {
                CheckResult::fail(key, label, key.to_ascii_uppercase(), format!("missing {}", required.join(", ")))
            });
        }
    }

    if enabled(cfg, "audit") {
        match tv.tag("AUDIT").map(str::to_ascii_uppercase).as_deref() {
            Some("PASS") => checks.push(CheckResult::pass("audit", "Audit verdict is PASS")),
            Some("FAKE") | Some("MIX") | Some("MQA") => checks.push(CheckResult::fail(
                "audit",
                "Audit verdict is PASS",
                "AUDIT",
                format!("AUDIT={}", tv.tag("AUDIT").unwrap_or("")),
            )),
            Some(other) if tv.has("AUDIOAUDITOR_OVERRIDE") => {
                // the override wins over every derived verdict
                let _ = other;
                checks.push(CheckResult::pass("audit", "Audit verdict is PASS"));
            }
            _ => checks.push(CheckResult::skipped("audit", "Audit verdict is PASS")),
        }
    }

    if enabled(cfg, "lyrics") {
        let has_tag = tv.has("LYRICS") || tv.has("UNSYNCEDLYRICS");
        let ok = has_tag || tv.has_lyrics_sidecar;
        checks.push(if ok {
            CheckResult::pass("lyrics", "Lyrics present")
        } else {
            CheckResult::fail("lyrics", "Lyrics present", "LYRICS", "no lyrics tag or sidecar".to_string())
        });
    }

    if enabled(cfg, "lyrics_format") && tv.has_lyrics_sidecar {
        checks.push(if tv.lyrics_sidecar_synced {
            CheckResult::pass("lyrics_format", "Lyrics sidecar well-formed")
        } else {
            CheckResult::fail("lyrics_format", "Lyrics sidecar well-formed", "LYRICS_FORMAT", "sidecar has no timestamps".to_string())
        });
    } else if enabled(cfg, "lyrics_format") {
        checks.push(CheckResult::skipped("lyrics_format", "Lyrics sidecar well-formed"));
    }

    if enabled(cfg, "lyrics_spaces") {
        let bad = tv
            .tags
            .iter()
            .filter(|(k, _)| *k == "LYRICS" || *k == "UNSYNCEDLYRICS")
            .flat_map(|(_, vs)| vs.iter())
            .any(|v| v.lines().any(|l| l.contains("  ") || l.starts_with(' ')));
        checks.push(if bad {
            CheckResult::fail("lyrics_spaces", "Lyrics spacing canonical", "LYRICS_SPACES", "double or leading spaces".to_string())
        } else {
            CheckResult::pass("lyrics_spaces", "Lyrics spacing canonical")
        });
    }

    if enabled(cfg, "lyrics_zero") {
        let bad = tv
            .tags
            .iter()
            .filter(|(k, _)| *k == "LYRICS" || *k == "UNSYNCEDLYRICS")
            .flat_map(|(_, vs)| vs.iter())
            .any(|v| {
                let t = v.trim();
                t.is_empty() || t == "0" || t.eq_ignore_ascii_case("no lyrics")
            });
        checks.push(if bad {
            CheckResult::fail("lyrics_zero", "No placeholder lyrics", "LYRICS_ZERO", "placeholder or empty lyrics".to_string())
        } else {
            CheckResult::pass("lyrics_zero", "No placeholder lyrics")
        });
    }

    if enabled(cfg, "instrumental") && !checks.iter().any(|c| c.key == "instrumental") {
        checks.push(CheckResult::skipped("instrumental", "INSTRUMENTAL decided"));
    }

    crate::model::GradeReport::new(format!("track {}", file_label(&tv.path)), checks)
}

// ---------------------------------------------------------------------------
// Album grade
// ---------------------------------------------------------------------------

pub fn grade_album(view: &AlbumView, cfg: &Config) -> AlbumGrade {
    let track_reports: Vec<(PathBuf, crate::model::GradeReport)> = view
        .tracks
        .iter()
        .map(|tv| (tv.path.clone(), grade_track(tv, cfg)))
        .collect();

    let mut checks: Vec<CheckResult> = Vec::new();

    // Album-level identity/release checks aggregate over tracks.
    checks.push(aggregate("missing_tags", "Required identity tags present", &track_reports, cfg));
    checks.push(aggregate("tag_case", "Canonical tag spelling", &track_reports, cfg));
    checks.push(aggregate("tag_spaces", "Canonical spacing", &track_reports, cfg));
    checks.push(aggregate("tag_blank_lines", "No blank/only-whitespace values", &track_reports, cfg));
    checks.push(aggregate("excess_tags", "No foreign tags", &track_reports, cfg));
    checks.push(aggregate("ext_case", "File extensions are lowercase", &track_reports, cfg));
    checks.push(aggregate("unreadable", "Every file is readable/decodable", &track_reports, cfg));
    checks.push(aggregate("instrumental", "INSTRUMENTAL decided", &track_reports, cfg));
    checks.push(aggregate("lyrics", "Lyrics present", &track_reports, cfg));
    checks.push(aggregate("lyrics_zero", "No placeholder lyrics", &track_reports, cfg));

    if enabled(cfg, "empty_folders") {
        checks.push(if view.audio_file_count == 0 && view.tracks.is_empty() {
            CheckResult::fail("empty_folders", "No empty album folders", "EMPTY_FOLDER", "album folder holds no audio".to_string())
        } else {
            CheckResult::pass("empty_folders", "No empty album folders")
        });
    }

    if enabled(cfg, "naming") {
        checks.push(grade_naming(view, cfg));
    }

    if enabled(cfg, "expected_tracks") {
        match view.side.expected_tracks {
            Some(n) if n != view.tracks.len() => checks.push(CheckResult::fail(
                "expected_tracks",
                "Tracklist matches .mlo_expected.json",
                "EXPECTED_TRACKS_INCOMPLETE",
                format!("expected {n}, found {}", view.tracks.len()),
            )),
            Some(_) => checks.push(CheckResult::pass("expected_tracks", "Tracklist matches .mlo_expected.json")),
            None => checks.push(CheckResult::skipped("expected_tracks", "Tracklist matches .mlo_expected.json")),
        }
    }

    // album-level release tags
    if enabled(cfg, "album_tags") {
        let mut missing: Vec<&str> = Vec::new();
        for k in ALBUM_REQUIRED {
            if !view.tracks.iter().any(|t| t.has(k)) {
                missing.push(k);
            }
        }
        checks.push(if missing.is_empty() {
            CheckResult::pass("album_tags", "Album-level release tags present")
        } else {
            CheckResult::fail("album_tags", "Album-level release tags present", "ALBUMITUNESADVISORY", format!("missing {}", missing.join(", ")))
        });
    }
    if enabled(cfg, "mb_links") {
        let ok = view.tracks.iter().any(|t| t.tags.keys().any(|k| k.starts_with("MUSICBRAINZ_")));
        checks.push(if ok {
            CheckResult::pass("mb_links", "MusicBrainz links present")
        } else {
            CheckResult::fail("mb_links", "MusicBrainz links present", "MB_LINK", "no MUSICBRAINZ_* tags".to_string())
        });
    }
    if enabled(cfg, "rym_links") {
        let ok = view.tracks.iter().any(|t| t.tags.keys().any(|k| k.starts_with("RATEYOURMUSIC_")));
        checks.push(if ok {
            CheckResult::pass("rym_links", "RateYourMusic links present")
        } else {
            CheckResult::fail("rym_links", "RateYourMusic links present", "RYM_LINK", "no RATEYOURMUSIC_* tags".to_string())
        });
    } else {
        checks.push(CheckResult::skipped("rym_links", "RateYourMusic links present"));
    }
    for (key, label, tag) in [("media", "MEDIA present and canonical", "MEDIA"), ("source", "SOURCE present and canonical", "SOURCE")] {
        if enabled(cfg, key) {
            let ok = !view.tracks.is_empty() && view.tracks.iter().all(|t| t.has(tag));
            checks.push(if ok {
                CheckResult::pass(key, label)
            } else {
                CheckResult::fail(key, label, tag, format!("{tag} missing on some tracks"))
            });
        } else {
            checks.push(CheckResult::skipped(key, label));
        }
    }
    if enabled(cfg, "genre") {
        let ok = view.tracks.iter().all(|t| t.has("GENRE")) && !view.tracks.is_empty();
        checks.push(if ok {
            CheckResult::pass("genre", "GENRE present")
        } else {
            CheckResult::fail("genre", "GENRE present", "GENRE", "some tracks have no GENRE".to_string())
        });
    }
    if enabled(cfg, "genre_count") {
        let max_genres = view.tracks.iter().map(|t| t.tags.get("GENRE").map(|v| v.len()).unwrap_or(0)).max().unwrap_or(0);
        checks.push(if max_genres <= 3 {
            CheckResult::pass("genre_count", "Genre count within policy")
        } else {
            CheckResult::fail("genre_count", "Genre count within policy", "GENRE_COUNT", format!("{max_genres} genres on one track"))
        });
    }
    if enabled(cfg, "genre_order") {
        let bad = view.tracks.iter().any(|t| {
            t.tags
                .get("GENRE")
                .map(|v| v.windows(2).any(|w| w[0].to_lowercase() > w[1].to_lowercase()))
                .unwrap_or(false)
        });
        checks.push(if bad {
            CheckResult::fail("genre_order", "Genres sorted", "GENRE_ORDER", "genres are not sorted".to_string())
        } else {
            CheckResult::pass("genre_order", "Genres sorted")
        });
    }
    if enabled(cfg, "genre_vocab") {
        // No vocabulary configured => not applicable.
        checks.push(CheckResult::skipped("genre_vocab", "Genres in the configured vocabulary"));
    }

    // Covers
    if enabled(cfg, "cover") {
        checks.push(if view.side.cover.is_some() {
            CheckResult::pass("cover", "Album cover present")
        } else {
            CheckResult::fail("cover", "Album cover present", "COVER", "no cover.* or folder.*".to_string())
        });
    }
    if enabled(cfg, "cover_crop") {
        match (view.side.cover.is_some(), view.side.cover_aspect_ok) {
            (false, _) => checks.push(CheckResult::skipped("cover_crop", "Cover aspect within policy")),
            (true, Some(true)) => checks.push(CheckResult::pass("cover_crop", "Cover aspect within policy")),
            (true, Some(false)) => checks.push(CheckResult::fail("cover_crop", "Cover aspect within policy", "COVER_CROP", "cover aspect outside tolerance".to_string())),
            (true, None) => checks.push(CheckResult::skipped("cover_crop", "Cover aspect within policy")),
        }
    }
    if enabled(cfg, "sidecar_cover") {
        checks.push(if view.side.cover.is_some() {
            CheckResult::pass("sidecar_cover", "cover.* is the canonical sidecar")
        } else {
            CheckResult::fail("sidecar_cover", "cover.* is the canonical sidecar", "COVER", "no cover sidecar".to_string())
        });
    }
    if enabled(cfg, "extra_images") {
        checks.push(if view.side.extra_images == 0 {
            CheckResult::pass("extra_images", "No extra images in the album")
        } else {
            CheckResult::fail("extra_images", "No extra images in the album", "EXTRA_IMAGES", format!("{} extra image(s)", view.side.extra_images))
        });
    }

    // Description
    if enabled(cfg, "album_description") {
        let ok = view.side.description.as_ref().map(|d| !d.trim().is_empty()).unwrap_or(false);
        checks.push(if ok {
            CheckResult::pass("album_description", "Album description present")
        } else {
            CheckResult::fail("album_description", "Album description present", "ALBUM_DESCRIPTION", "description.txt missing or blank".to_string())
        });
    }

    // CD-specific
    if view.is_cd {
        if enabled(cfg, "cd_format") {
            let nums: Vec<Option<u32>> = view
                .tracks
                .iter()
                .map(|t| t.tag("TRACKNUMBER").and_then(|s| s.parse::<u32>().ok()))
                .collect();
            let contiguous = nums.iter().enumerate().all(|(i, n)| n.map(|v| v as usize == i + 1).unwrap_or(false));
            checks.push(if contiguous {
                CheckResult::pass("cd_format", "CD track numbering contiguous")
            } else {
                CheckResult::fail("cd_format", "CD track numbering contiguous", "CD_FORMAT", "track numbers are not 1..N".to_string())
            });
        }
        if enabled(cfg, "cd_cue") {
            checks.push(if view.side.cue.is_some() {
                CheckResult::pass("cd_cue", "CUE matches the release")
            } else {
                CheckResult::fail("cd_cue", "CUE matches the release", "CD_CUE", "no .cue sidecar".to_string())
            });
        }
        if enabled(cfg, "cd_log") {
            checks.push(if view.side.log.is_some() {
                CheckResult::pass("cd_log", "Rip log present for CD rips")
            } else {
                CheckResult::fail("cd_log", "Rip log present for CD rips", "CD_LOG", "no .log sidecar".to_string())
            });
        }
        if enabled(cfg, "log_grade") {
            let ok = view.tracks.iter().any(|t| t.has("LOG_GRADE"));
            checks.push(if ok {
                CheckResult::pass("log_grade", "LOG_GRADE recorded")
            } else {
                CheckResult::fail("log_grade", "LOG_GRADE recorded", "LOG_GRADE", "LOG_GRADE missing".to_string())
            });
        }
        if enabled(cfg, "log_checksum") {
            let ok = view.tracks.iter().any(|t| t.has("LOG_CRC"));
            checks.push(if ok {
                CheckResult::pass("log_checksum", "LOG_CRC recorded")
            } else {
                CheckResult::fail("log_checksum", "LOG_CRC recorded", "LOG_CRC", "LOG_CRC missing".to_string())
            });
        }
        if enabled(cfg, "accurip_format") {
            checks.push(match view.side.accurip {
                Some(_) => CheckResult::pass("accurip_format", "AccurateRip sidecar well-formed"),
                None => CheckResult::skipped("accurip_format", "AccurateRip sidecar well-formed"),
            });
        }
    } else {
        for key in ["cd_format", "cd_cue", "cd_log", "log_grade", "log_checksum", "accurip_format"] {
            if enabled(cfg, key) {
                checks.push(CheckResult::skipped(key, check_label(key)));
            }
        }
    }
    if enabled(cfg, "accuraterip") {
        checks.push(if view.side.accurip.is_some() {
            CheckResult::pass("accuraterip", "AccurateRip verified")
        } else {
            CheckResult::fail("accuraterip", "AccurateRip verified", "ACCURATERIP", "no .accurip".to_string())
        });
    }
    if enabled(cfg, "lossless_source") {
        checks.push(CheckResult::skipped("lossless_source", "Source is lossless where claimed"));
    }
    if enabled(cfg, "raw_video") {
        let raw = view.findings.iter().any(|f| f.kind == crate::model::FindingKind::UnexpectedFolder && f.detail.contains("VIDEO_TS"));
        checks.push(if raw {
            CheckResult::fail("raw_video", "No raw VIDEO_TS/BDMV structure", "RAW_VIDEO", "raw disc structure not remuxed".to_string())
        } else {
            CheckResult::pass("raw_video", "No raw VIDEO_TS/BDMV structure")
        });
    }

    if enabled(cfg, "acoustid") {
        let ok = view.tracks.iter().any(|t| t.has("ACOUSTID_ID"));
        checks.push(if !cfg.acoustid_enabled {
            CheckResult::skipped("acoustid", "ACOUSTID_ID present")
        } else if ok {
            CheckResult::pass("acoustid", "ACOUSTID_ID present")
        } else {
            CheckResult::fail("acoustid", "ACOUSTID_ID present", "ACOUSTID_ID", "no ACOUSTID_ID".to_string())
        });
    } else {
        checks.push(CheckResult::skipped("acoustid", "ACOUSTID_ID present"));
    }

    // Layout findings charged on this album ([§9.2])
    for kind in crate::model::FindingKind::ALL {
        let key = kind.check_key();
        if !enabled(cfg, &key) {
            continue;
        }
        let matches: Vec<&Finding> = view
            .findings
            .iter()
            .filter(|f| f.kind == kind && (f.album.as_deref() == Some(view.path.as_path()) || f.album.is_none()))
            .collect();
        checks.push(if matches.is_empty() {
            CheckResult::pass(key.clone(), kind.label())
        } else {
            CheckResult::fail(
                key.clone(),
                kind.label(),
                key.to_ascii_uppercase(),
                matches.iter().map(|f| f.detail.clone()).collect::<Vec<_>>().join("; "),
            )
        });
    }

    let report = crate::model::GradeReport::new(format!("album {}", view.title), checks);
    AlbumGrade { report, track_reports }
}

fn grade_naming(view: &AlbumView, cfg: &Config) -> CheckResult {
    // Album-level tags: take the first track that has each tag.
    let mut album_tags: TagMap = TagMap::new();
    for t in &view.tracks {
        for (k, v) in &t.tags {
            album_tags.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let opts = NamingOptions { short_folder_names: cfg.short_folder_names };
    match naming::evaluate(&cfg.naming_script, &album_tags, opts) {
        Ok(EvalResult { path: expected, missing, .. }) => {
            let actual = relative_under_music(&view.path, &cfg.music_folder);
            let expected_norm = normalize_seps(&expected);
            let actual_norm = normalize_seps(&actual);
            if actual_norm == expected_norm {
                CheckResult::pass("naming", "Album path matches the naming template")
            } else if actual_norm.eq_ignore_ascii_case(&expected_norm) {
                CheckResult::fail("naming", "Album path matches the naming template", "PATH_CASE", format!("case differs from '{expected}'"))
            } else if !missing.is_empty() {
                CheckResult::fail(
                    "naming",
                    "Album path matches the naming template",
                    "MISSING_TAG_FOR_PATH",
                    format!("Missing {} tag", missing.join(", ")),
                )
            } else {
                CheckResult::fail("naming", "Album path matches the naming template", "PATH", format!("expected '{expected}', found '{actual}'"))
            }
        }
        Err(e) => CheckResult::fail("naming", "Album path matches the naming template", "PATH", e.to_string()),
    }
}

/// `Artists/<artist>/<album>` -> `<artist>/<album>` (the template's base).
fn relative_under_music(path: &Path, music: &Path) -> String {
    let rel = path.strip_prefix(music).unwrap_or(path);
    let mut comps: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if comps.first().map(|s| s.eq_ignore_ascii_case("Artists")).unwrap_or(false) {
        comps.remove(0);
    }
    comps.join("/")
}

fn normalize_seps(s: &str) -> String {
    s.replace('\\', "/").trim_matches('/').to_string()
}

// ---------------------------------------------------------------------------
// Artist / library grade
// ---------------------------------------------------------------------------

pub fn grade_artist(view: &ArtistView, cfg: &Config) -> crate::model::GradeReport {
    let mut checks = Vec::new();

    if enabled(cfg, "artist_image") {
        if !cfg.artist_image_enabled {
            checks.push(CheckResult::skipped("artist_image", "Artist image present and within policy"));
        } else if !view.has_image {
            checks.push(CheckResult::fail("artist_image", "Artist image present and within policy", "ARTIST_IMAGE_MISSING", "no artist.jpg/artist.png".to_string()));
        } else if view.image_upscaled {
            checks.push(CheckResult::fail("artist_image", "Artist image present and within policy", "ARTIST_IMAGE_UPSCALED", "image larger than what this app wrote".to_string()));
        } else if view.image_ok == Some(false) {
            checks.push(CheckResult::fail("artist_image", "Artist image present and within policy", "ARTIST_IMAGE_UNREADABLE", "image unreadable".to_string()));
        } else {
            checks.push(CheckResult::pass("artist_image", "Artist image present and within policy"));
        }
    }

    if enabled(cfg, "artist_description") {
        if !cfg.artist_description_enabled {
            checks.push(CheckResult::skipped("artist_description", "Artist description present"));
        } else if !view.has_description {
            checks.push(CheckResult::fail("artist_description", "Artist description present", "ARTIST_DESCRIPTION_MISSING", "description.txt missing or blank".to_string()));
        } else {
            checks.push(CheckResult::pass("artist_description", "Artist description present"));
        }
    }

    if !view.has_albums {
        checks.push(CheckResult::fail("artist_albums", "Artist folder holds albums", "ARTIST_EMPTY", "no album folders".to_string()));
    }

    for kind in crate::model::FindingKind::ALL {
        let key = kind.check_key();
        if !enabled(cfg, &key) {
            continue;
        }
        let matches = view.findings.iter().filter(|f| f.kind == kind && f.artist.as_deref() == Some(view.path.as_path())).count();
        if matches > 0 {
            checks.push(CheckResult::fail(key.clone(), kind.label(), key.to_ascii_uppercase(), format!("{matches} finding(s)")));
        }
    }

    crate::model::GradeReport::new(format!("artist {}", view.name), checks)
}

/// Grade the explicit library row ([§9.2]): library-wide findings.
pub fn grade_library(findings: &[Finding], cfg: &Config) -> crate::model::GradeReport {
    let mut checks = Vec::new();
    for kind in crate::model::FindingKind::ALL {
        let key = kind.check_key();
        if !enabled(cfg, &key) {
            continue;
        }
        let matches = findings.iter().filter(|f| f.kind == kind && f.library_wide).count();
        checks.push(if matches == 0 {
            CheckResult::pass(key.clone(), format!("{} (library)", kind.label()))
        } else {
            CheckResult::fail(key.clone(), format!("{} (library)", kind.label()), key.to_ascii_uppercase(), format!("{matches} finding(s)"))
        });
    }
    crate::model::GradeReport::new("library", checks)
}

/// Build the library verdict from per-album/artist reports ([§9.5]).
pub fn library_verdict(
    albums: &[(PathBuf, crate::model::GradeReport)],
    artists: &[(String, crate::model::GradeReport)],
    tracks_total: u32,
    audit_failed: u32,
    library_row: Option<crate::model::GradeReport>,
) -> LibraryVerdict {
    let albums_total = albums.len() as u32;
    let albums_passed = albums.iter().filter(|(_, r)| r.passed()).count() as u32;
    let artists_total = artists.len() as u32;
    let artists_passed = artists.iter().filter(|(_, r)| r.passed()).count() as u32;
    LibraryVerdict {
        albums_total,
        albums_passed,
        albums_failed: albums_total - albums_passed,
        albums_audit_failed: audit_failed,
        artists_total,
        artists_passed,
        artists_failed: artists_total - artists_passed,
        tracks_total,
        library_row,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn aggregate(
    key: &str,
    label: &str,
    track_reports: &[(PathBuf, crate::model::GradeReport)],
    cfg: &Config,
) -> CheckResult {
    if !enabled(cfg, key) {
        return CheckResult::skipped(key, label);
    }
    let mut worst: Option<Issue> = None;
    for (_, r) in track_reports {
        if let Some(c) = r.checks.iter().find(|c| c.key == key) {
            match &c.status {
                crate::model::CheckStatus::Fail(i) => {
                    worst = Some(i.clone());
                    break;
                }
                crate::model::CheckStatus::CouldNotEvaluate(m) => {
                    worst = Some(Issue { code: "COULD_NOT_EVALUATE".to_string(), message: m.clone() });
                    break;
                }
                _ => {}
            }
        }
    }
    match worst {
        Some(i) => CheckResult { key: key.into(), label: label.into(), status: crate::model::CheckStatus::Fail(i) },
        None => {
            let any_evaluated = track_reports.iter().any(|(_, r)| r.checks.iter().any(|c| c.key == key && !matches!(c.status, crate::model::CheckStatus::Skipped)));
            if any_evaluated {
                CheckResult::pass(key, label)
            } else {
                CheckResult::skipped(key, label)
            }
        }
    }
}

fn uniq_join(v: &[String]) -> String {
    let mut out: Vec<&String> = Vec::new();
    for s in v {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
}

fn file_label(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}