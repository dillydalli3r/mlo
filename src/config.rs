//! Configuration ([§12.3]) — `mlo.toml`, tolerating the source app's
//! `config.json` on first run (read + migrated, never rewritten in place).

use crate::atomic;
use crate::error::{IoResultExt, MloError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "mlo.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // --- library / layout ---
    pub music_folder: PathBuf,
    pub naming_script: String,
    pub short_folder_names: bool,
    pub layout_apply: bool,

    // --- import ---
    pub import_auto_scripts: bool,
    /// Empty = derived default chain ([§7.4]).
    pub import_scripts: Vec<u8>,
    pub import_match_threshold: f32,

    // --- artist artefacts ---
    pub artist_image_enabled: bool,
    pub artist_image_aspect: String,
    pub artist_image_crop: bool,
    /// 0 = provider-native, capped at 2000 px hard ceiling.
    pub artist_image_target_size: u32,
    pub artist_description_enabled: bool,

    // --- feature switches for scripts ([§7.1]) ---
    pub dr_replaygain_enabled: bool,
    pub audiometa_enabled: bool,
    pub mood_enabled: bool,
    pub lyrics_xlit_enabled: bool,
    pub lyrics_translate_enabled: bool,
    pub acoustid_enabled: bool,
    pub fingerprint_submit_enabled: bool,
    pub strip_unknown_tags: bool,
    pub web_ratings_enabled: bool,
    pub video_remux_enabled: bool,
    pub analyze_pool_size: usize,

    // --- la-musica config keys that change a grade (§9 of its spec) ---
    pub grade_verbose: bool,
    pub grade_log_score_threshold: i64,
    pub grader_cover_size_tolerance_px: i64,
    pub grader_strict_square_threshold: f64,
    pub cover_crop_threshold: f64,
    pub cover_enforce_size: bool,
    pub cover_enforce_square: bool,
    pub cover_resize_enabled: bool,
    pub cover_force_exact_size: bool,
    pub cover_target_size: u32,
    pub cover_jpeg_target_size: u32,
    pub cover_png_target_size: u32,
    pub cover_jxl_target_size: u32,
    pub cover_crop_enabled: bool,
    pub cover_country: String,
    pub cover_sources: Vec<String>,
    pub reencode_images: bool,
    pub embed_covers: bool,
    pub embed_cover_jpeg_quality: u32,
    pub embed_cover_resolution: u32,
    pub album_description_enabled: bool,
    pub description_full: bool,
    pub description_sources: Vec<String>,
    pub mb_genre_count: u32,
    pub genre_autofill: bool,
    pub genre_sources: Vec<String>,
    pub mood_source: String,
    pub ai_effort: String,
    pub ai_genre_effort: String,
    pub audiometa_key_notation: String,
    pub lyrics_format: String,
    pub lyrics_allow_plain: bool,
    pub lyrics_translation_langs: String,
    pub optimize_lrc: bool,
    pub optimize_embedded_lyrics: bool,
    pub lrc_timestamp_precision: u32,
    pub lrc_strip_metadata: bool,
    pub lrc_collapse_blank_lines: bool,
    pub lrc_enhanced_enabled: bool,
    pub lrc_enhanced_word_sync: bool,
    pub lrc_sync_level: String,
    pub lrc_extended_enabled: bool,
    pub lrc_add_zero_timestamp: bool,
    pub lrc_zero_timestamp_blank: bool,
    pub lrc_zero_timestamp_target: String,
    pub append_final_newline: bool,
    pub keep_empty_cue_lines: bool,
    pub keep_other_cue_lines: bool,
    pub keep_empty_accurip_lines: bool,
    pub cue_file_type: String,
    pub discs_rename_enabled: bool,
    pub discs_rename_pattern: String,
    pub discs_rename_single_fallback: bool,
    pub cue_fix_filenames: bool,
    pub discs_toc_tolerance_s: f64,
    pub discs_toc_unique_margin_s: f64,
    pub normalize_media_source: bool,
    pub digital_media_source_value: String,
    pub fill_empty_source: bool,
    pub strip_source_on_cd: bool,
    pub write_audit_tag: bool,
    pub write_log_grade: bool,
    pub write_replaygain_tags: bool,
    pub write_dynamic_range_tags: bool,
    pub replaygain_skip_existing: bool,
    pub force_dr_replaygain: bool,
    pub audit_require_accuraterip: bool,
    pub audit_verify_log_checksum: bool,
    pub audit_check_cd_format: bool,
    pub audit_verify_cd_checksums: bool,
    pub audit_integrity: bool,
    pub audit_cd_require_both: bool,
    pub audit_log_score_threshold: i64,
    pub audit_fail_on_unscorable_log: bool,
    pub audit_thorough: bool,
    pub audit_mqa: bool,
    pub audit_ai: bool,
    pub lossless_remove_original: bool,
    pub video_remove_original: bool,
    pub library_codec: String,
    pub library_codec_quality: u32,
    pub library_codec_bitrate: u32,
    pub library_codec_args: String,
    pub library_codec_optimize: String,
    /// Empty = the derived run-all / import order.
    pub run_all_order: Vec<u8>,

    // --- grading check overrides (registry defaults in `grade`) ---
    pub checks: BTreeMap<String, bool>,

    /// Unknown la-musica keys are preserved verbatim so a round trip never
    /// drops a setting this build does not model yet.
    pub extra: BTreeMap<String, toml::Value>,

    pub services: Services,
    pub player: Player,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Services {
    pub musicbrainz_user_agent: String,
    pub acoustid_api_key: String,
    pub discogs_token: String,
    pub request_timeout_s: u64,
    pub cache_ttl_s: u64,
    /// MusicBrainz is rate-limited to 1 req/s by policy.
    pub musicbrainz_rate_per_s: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Player {
    pub cache_dir: PathBuf,
    pub resume_on_start: bool,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            musicbrainz_user_agent: "mlo-tui/0.1 ( https://github.com/dillydalli3r/mlo )".into(),
            acoustid_api_key: String::new(),
            discogs_token: String::new(),
            request_timeout_s: 20,
            cache_ttl_s: 60 * 60 * 24 * 7,
            musicbrainz_rate_per_s: 1.0,
        }
    }
}

impl Default for Player {
    fn default() -> Self {
        Self { cache_dir: PathBuf::new(), resume_on_start: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            music_folder: default_music_folder(),
            naming_script: crate::naming::DEFAULT_TEMPLATE.to_string(),
            short_folder_names: false,
            layout_apply: true,
            import_auto_scripts: true,
            import_scripts: Vec::new(),
            import_match_threshold: 0.85,
            artist_image_enabled: true,
            artist_image_aspect: "1:1".into(),
            artist_image_crop: true,
            artist_image_target_size: 0,
            artist_description_enabled: true,
            dr_replaygain_enabled: true,
            audiometa_enabled: true,
            mood_enabled: true,
            lyrics_xlit_enabled: true,
            lyrics_translate_enabled: true,
            acoustid_enabled: true,
            fingerprint_submit_enabled: false,
            strip_unknown_tags: true,
            web_ratings_enabled: false,
            video_remux_enabled: true,
            analyze_pool_size: 0,
            // la-musica grade-affecting defaults
            grade_verbose: true,
            grade_log_score_threshold: 100,
            grader_cover_size_tolerance_px: 0,
            grader_strict_square_threshold: 0.0,
            cover_crop_threshold: 0.0,
            cover_enforce_size: true,
            cover_enforce_square: true,
            cover_resize_enabled: true,
            cover_force_exact_size: true,
            cover_target_size: 1200,
            cover_jpeg_target_size: 0,
            cover_png_target_size: 0,
            cover_jxl_target_size: 0,
            cover_crop_enabled: true,
            cover_country: "us".into(),
            cover_sources: Vec::new(),
            reencode_images: true,
            embed_covers: false,
            embed_cover_jpeg_quality: 90,
            embed_cover_resolution: 1200,
            album_description_enabled: true,
            description_full: true,
            description_sources: Vec::new(),
            mb_genre_count: 2,
            genre_autofill: true,
            genre_sources: vec![
                "rateyourmusic", "musicbrainz", "listenbrainz", "itunes", "lastfm", "theaudiodb",
                "wikidata", "bandcamp", "discogs", "deezer", "spotify",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            mood_source: "hybrid".into(),
            ai_effort: "high".into(),
            ai_genre_effort: "high".into(),
            audiometa_key_notation: "musical".into(),
            lyrics_format: "EMBEDDED".into(),
            lyrics_allow_plain: false,
            lyrics_translation_langs: "en".into(),
            optimize_lrc: true,
            optimize_embedded_lyrics: true,
            lrc_timestamp_precision: 2,
            lrc_strip_metadata: true,
            lrc_collapse_blank_lines: true,
            lrc_enhanced_enabled: true,
            lrc_enhanced_word_sync: true,
            lrc_sync_level: "LINE".into(),
            lrc_extended_enabled: true,
            lrc_add_zero_timestamp: false,
            lrc_zero_timestamp_blank: false,
            lrc_zero_timestamp_target: "BOTH".into(),
            append_final_newline: false,
            keep_empty_cue_lines: false,
            keep_other_cue_lines: false,
            keep_empty_accurip_lines: false,
            cue_file_type: "WAVE".into(),
            discs_rename_enabled: true,
            discs_rename_pattern: "CD-{n}".into(),
            discs_rename_single_fallback: true,
            cue_fix_filenames: true,
            discs_toc_tolerance_s: 4.0,
            discs_toc_unique_margin_s: 4.0,
            normalize_media_source: true,
            digital_media_source_value: "Digital".into(),
            fill_empty_source: false,
            strip_source_on_cd: true,
            write_audit_tag: true,
            write_log_grade: true,
            write_replaygain_tags: true,
            write_dynamic_range_tags: true,
            replaygain_skip_existing: true,
            force_dr_replaygain: false,
            audit_require_accuraterip: true,
            audit_verify_log_checksum: true,
            audit_check_cd_format: true,
            audit_verify_cd_checksums: true,
            audit_integrity: true,
            audit_cd_require_both: true,
            audit_log_score_threshold: 100,
            audit_fail_on_unscorable_log: true,
            audit_thorough: true,
            audit_mqa: true,
            audit_ai: true,
            lossless_remove_original: true,
            video_remove_original: true,
            library_codec: "flac".into(),
            library_codec_quality: 5,
            library_codec_bitrate: 0,
            library_codec_args: String::new(),
            library_codec_optimize: "lossless_to_lossy".into(),
            run_all_order: Vec::new(),
            checks: BTreeMap::new(),
            extra: BTreeMap::new(),
            services: Services::default(),
            player: Player::default(),
        }
    }
}

fn default_music_folder() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.audio_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

impl Config {
    /// `<config>/mlo/mlo.toml` — same location on every platform via `directories`.
    pub fn default_path() -> PathBuf {
        if let Some(pd) = directories::ProjectDirs::from("dev", "mlo", "mlo") {
            pd.config_dir().join(CONFIG_FILE)
        } else {
            PathBuf::from(CONFIG_FILE)
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let text = std::fs::read_to_string(path).at(path)?;
            let cfg: Config = toml::from_str(&text)
                .map_err(|e| MloError::Config(format!("{}: {e}", path.display())))?;
            return Ok(cfg);
        }
        // First run: migrate an existing la musica config.json if present.
        if let Some(legacy) = find_legacy_config(path) {
            let cfg = Self::migrate_json(&legacy)?;
            cfg.save(path)?;
            return Ok(cfg);
        }
        let cfg = Config::default();
        cfg.save(path)?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)
            .map_err(|e| MloError::Config(format!("encode: {e}")))?;
        atomic::write_atomic_str(path, &text)
    }

    /// Look in the config dir, the CWD, and `$MLO_MUSIC_FOLDER/.mlo/data/` for a
    /// legacy la-musica `config.json`.
    fn legacy_candidates(config_path: &Path) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(dir) = config_path.parent() {
            v.push(dir.join("config.json"));
        }
        if let Some(pd) = directories::ProjectDirs::from("dev", "mlo", "mlo") {
            v.push(pd.config_dir().join("config.json"));
        }
        if let Ok(music) = std::env::var("MLO_MUSIC_FOLDER") {
            v.push(PathBuf::from(music).join(".mlo").join("data").join("config.json"));
        }
        v.push(PathBuf::from("config.json"));
        v
    }

    /// Import a la-musica `config.json`. Modelled keys are read by name;
    /// every unmapped key is preserved verbatim in `extra` so a round trip
    /// never drops a setting this build does not model yet. The original file
    /// is never rewritten in place.
    pub fn migrate_json(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).at(path)?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| MloError::Config(format!("legacy config {}: {e}", path.display())))?;
        let Some(obj) = value.as_object() else { return Ok(Config::default()) };

        // Convert the whole JSON object to TOML and read the modelled keys.
        let table = json_object_to_toml(obj);
        let toml_text = toml::to_string(&table)
            .map_err(|e| MloError::Config(format!("legacy config {}: {e}", path.display())))?;
        let mut cfg: Config = toml::from_str(&toml_text)
            .map_err(|e| MloError::Config(format!("legacy config {}: {e}", path.display())))?;

        // Preserve keys this build does not model.
        for (k, v) in obj {
            // la-musica stores the check toggles at the TOP LEVEL of the JSON;
            // mlo keeps them in `checks`. Lift them so an override survives.
            if (k.starts_with("grade_check_") || k.starts_with("grade_include_")) && v.is_boolean() {
                cfg.checks.insert(k.clone(), v.as_bool().unwrap_or(true));
                continue;
            }
            if let Some(tv) = json_to_toml(v) {
                cfg.extra.entry(k.clone()).or_insert(tv);
            }
        }

        // Legacy/alternate spellings.
        if cfg.music_folder.as_os_str().is_empty() {
            if let Some(m) = obj.get("music_dir").and_then(|v| v.as_str()) {
                cfg.music_folder = PathBuf::from(m);
            }
        }
        if let Some(ua) = obj
            .get("services")
            .and_then(|v| v.get("musicbrainz_user_agent"))
            .and_then(|v| v.as_str())
        {
            if cfg.services.musicbrainz_user_agent.is_empty() {
                cfg.services.musicbrainz_user_agent = ua.to_string();
            }
        }
        Ok(cfg)
    }

    // --- derived paths ([§3.1], [§3.4]) ---

    pub fn state_dir(&self) -> PathBuf {
        self.music_folder.join(".mlo")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.state_dir().join("data")
    }
    pub fn tools_dir(&self) -> PathBuf {
        self.state_dir().join("tools")
    }
    pub fn downloads_dir(&self) -> PathBuf {
        self.state_dir().join("downloads")
    }
    pub fn incomplete_dir(&self) -> PathBuf {
        self.state_dir().join("incomplete")
    }
    pub fn trash_dir(&self) -> PathBuf {
        self.state_dir().join("trash")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir().join("logs")
    }
    pub fn index_db(&self) -> PathBuf {
        self.data_dir().join("index.db")
    }
    pub fn layout_report(&self) -> PathBuf {
        // la-musica: <music>/.mlo/data/layout_report.json
        self.data_dir().join("layout_report.json")
    }
    pub fn cover_cache_dir(&self) -> PathBuf {
        if self.player.cache_dir.as_os_str().is_empty() {
            self.data_dir().join("player")
        } else {
            self.player.cache_dir.clone()
        }
    }

    pub fn check_enabled(&self, key: &str) -> bool {
        match self.checks.get(key) {
            Some(v) => *v,
            None => crate::grade::default_check_enabled(key),
        }
    }

    /// Effective analyze pool: 0 = auto ([§7.3]).
    pub fn pool_size(&self) -> usize {
        if self.analyze_pool_size == 0 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
        } else {
            self.analyze_pool_size
        }
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for d in [
            self.state_dir(),
            self.data_dir(),
            self.tools_dir(),
            self.downloads_dir(),
            self.incomplete_dir(),
            self.trash_dir(),
            self.logs_dir(),
            self.cover_cache_dir(),
        ] {
            std::fs::create_dir_all(&d).at(&d)?;
        }
        Ok(())
    }
}

fn find_legacy_config(config_path: &Path) -> Option<PathBuf> {
    Config::legacy_candidates(config_path)
        .into_iter()
        .find(|p| p.exists())
}
/// Convert one `serde_json` value into a `toml::Value`.
fn json_to_toml(v: &serde_json::Value) -> Option<toml::Value> {
    Some(match v {
        serde_json::Value::Null => return None,
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                return None;
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(a) => {
            toml::Value::Array(a.iter().filter_map(json_to_toml).collect())
        }
        serde_json::Value::Object(o) => json_object_to_toml(o),
    })
}

fn json_object_to_toml(o: &serde_json::Map<String, serde_json::Value>) -> toml::Value {
    let mut table = toml::value::Table::new();
    for (k, v) in o {
        if let Some(tv) = json_to_toml(v) {
            table.insert(k.clone(), tv);
        }
    }
    toml::Value::Table(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_a_la_musica_config_json() {
        let dir = tempfile::tempdir().unwrap();
        let json = dir.path().join("config.json");
        std::fs::write(
            &json,
            r#"{
              "music_folder": "F:/Media/Music",
              "naming_script": "%albumartist%/%album%",
              "short_folder_names": true,
              "mb_genre_count": 3,
              "grade_verbose": false,
              "grade_check_audit": false,
              "grade_include_other": false,
              "lyrics_format": "LRC",
              "encoder_tags": {"flac": ["ENCODER_QUALITY"]},
              "ai_effort": "medium"
            }"#,
        )
        .unwrap();

        let cfg = Config::migrate_json(&json).unwrap();
        assert_eq!(cfg.music_folder, PathBuf::from("F:/Media/Music"));
        assert_eq!(cfg.naming_script, "%albumartist%/%album%");
        assert!(cfg.short_folder_names);
        assert_eq!(cfg.mb_genre_count, 3);
        assert!(!cfg.grade_verbose);
        // top-level grade_check_*/grade_include_* lift into `checks`
        assert_eq!(cfg.checks.get("grade_check_audit"), Some(&false));
        assert_eq!(cfg.checks.get("grade_include_other"), Some(&false));
        assert!(!cfg.check_enabled("grade_check_audit"));
        assert!(cfg.check_enabled("grade_check_missing_tags"), "unlisted checks default ON");
        // unmodelled keys survive in `extra`
        assert!(cfg.extra.contains_key("encoder_tags"));
        assert_eq!(cfg.ai_effort, "medium");
    }

    #[test]
    fn presets_match_la_musica() {
        let mut cfg = Config::default();
        cfg.apply_preset(crate::grade::Preset::Relaxed);
        assert_eq!(cfg.checks.get("grade_check_tag_spaces"), Some(&false));
        assert_eq!(cfg.checks.get("grade_check_missing_tags"), None, "unlisted = default ON");
        assert_eq!(crate::grade::RELAXED_OFF.len(), 18);

        cfg.apply_preset(crate::grade::Preset::Balanced);
        assert_eq!(cfg.checks.get("grade_check_audit"), Some(&false));
        assert_eq!(cfg.checks.get("grade_include_other"), Some(&false));
        assert_eq!(cfg.checks.get("grade_check_tag_spaces"), None, "balanced re-enables");

        cfg.apply_preset(crate::grade::Preset::Strict);
        assert!(cfg.checks.is_empty(), "strict is the shipped default");
    }
}
