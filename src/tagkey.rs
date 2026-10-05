//! Tag families, allowlist and normalization ([§6.2], [§6.3]).
//!
//! The writer's normalization and the grader's `tag_case`/`tag_spaces` checks
//! call the *same* functions here, so an import can never produce a value the
//! grade fails ([§6.3] last bullet).

use crate::model::TagMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagFamily {
    Identity,
    Release,
    Audio,
    Lyrics,
    Provenance,
    /// Anything outside the five families (fails excess-tags unless allowlisted).
    Foreign,
}

impl TagFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            TagFamily::Identity => "identity",
            TagFamily::Release => "release",
            TagFamily::Audio => "audio",
            TagFamily::Lyrics => "lyrics",
            TagFamily::Provenance => "provenance",
            TagFamily::Foreign => "foreign",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TagFamily::Identity => "Identity",
            TagFamily::Release => "Release",
            TagFamily::Audio => "Audio",
            TagFamily::Lyrics => "Lyrics",
            TagFamily::Provenance => "Provenance",
            TagFamily::Foreign => "Foreign",
        }
    }
    pub const ALL: [TagFamily; 5] = [
        TagFamily::Identity,
        TagFamily::Release,
        TagFamily::Audio,
        TagFamily::Lyrics,
        TagFamily::Provenance,
    ];
}

const IDENTITY: &[&str] = &[
    "TITLE",
    "ARTIST",
    "ALBUM",
    "ALBUMARTIST",
    "TRACKNUMBER",
    "DISCNUMBER",
    "DATE",
    "TITLEALIAS",
    "ARTISTALIAS",
    "GENRE",
    "ITUNESADVISORY",
    "INSTRUMENTAL",
    "COMMENT",
];

const RELEASE_EXACT: &[&str] = &[
    "ALBUMALIAS",
    "MEDIA",
    "SOURCE",
    "ALBUMITUNESADVISORY",
    "RELEASETYPE",
    "LABEL",
    "CATALOGNUMBER",
    "BARCODE",
    "ASIN",
    "LANGUAGE",
    "DISCSUBTITLE",
    "LICENSE",
    "ENCODEDBY",
    "ISRC",
    "WORK",
    "MOVEMENT",
    "PERFORMER",
    "PRODUCER",
    "ENGINEER",
    "MIXER",
    "ARRANGER",
    "DJMIXER",
    "CONDUCTOR",
    "WRITER",
    "DIRECTOR",
    "COMPOSERSORT",
];

const AUDIO: &[&str] = &[
    "MOOD",
    "ENERGY",
    "BPM",
    "INITIALKEY",
    "DYNAMIC RANGE",
    "ALBUM DYNAMIC RANGE",
    "REPLAYGAIN_TRACK_GAIN",
    "REPLAYGAIN_TRACK_PEAK",
    "REPLAYGAIN_ALBUM_GAIN",
    "REPLAYGAIN_ALBUM_PEAK",
];

const LYRICS: &[&str] = &["LYRICS", "UNSYNCEDLYRICS", "TRANSLITERATION", "TRANSLATION"];

const PROVENANCE: &[&str] = &[
    "AUDIT",
    "LOG_GRADE",
    "LOG_CRC",
    "INTEGRITY",
    "AUDIO_MD5",
    "AUDIOAUDITOR_OVERRIDE",
    "ACOUSTID_ID",
    "ACOUSTID_FINGERPRINT",
    "ENCODER_PROGRAM",
    "ENCODER_QUALITY",
    "ENCODER_VERSION",
];

/// Whether a key is inside the five families (aliases with a locale suffix
/// count as their bare key, [§6.2]).
pub fn family_of(key: &str) -> TagFamily {
    let k = key.to_ascii_uppercase();
    let base = strip_alias_suffix(&k);
    if IDENTITY.contains(&base) {
        TagFamily::Identity
    } else if RELEASE_EXACT.contains(&base)
        || k.starts_with("MUSICBRAINZ_")
        || k.starts_with("RATEYOURMUSIC_")
    {
        TagFamily::Release
    } else if AUDIO.contains(&k.as_str()) {
        TagFamily::Audio
    } else if LYRICS.contains(&base) {
        TagFamily::Lyrics
    } else if PROVENANCE.contains(&k.as_str()) {
        TagFamily::Provenance
    } else {
        TagFamily::Foreign
    }
}

/// `TITLEALIAS_JA` -> `TITLEALIAS`, `TITLEALIAS` -> `TITLEALIAS`.
pub fn strip_alias_suffix(key: &str) -> &str {
    for base in ["TITLEALIAS", "ARTISTALIAS", "ALBUMALIAS"] {
        if key == base {
            return key;
        }
        if let Some(rest) = key.strip_prefix(base) {
            if rest.starts_with('_') {
                return base;
            }
        }
    }
    key
}

/// Known = inside a family, or an allowlisted extra. Foreign keys are reported
/// as excess and removed by script 23.
pub fn is_known(key: &str) -> bool {
    family_of(key) != TagFamily::Foreign
}

/// Keys that are foreign per the families but explicitly allowed.
pub const ALLOWLIST_EXTRA: &[&str] = &[
    "CUESHEET",
    "METADATA_BLOCK_PICTURE",
    "ENCODER",
    "DESCRIPTION",
    "ORIGINALDATE",
    "RELEASECOUNTRY",
    "RELEASESTATUS",
    "SCRIPT",
    "COMPILATION",
    "BPM",
    "DISCTOTAL",
    "TRACKTOTAL",
    "TOTALTRACKS",
    "TOTALDISCS",
    "LYRICIST",
    "SUBJECT",
    "RATING",
    "MUSICBRAINZ_RELEASEGROUPID",
];

// ---------------------------------------------------------------------------
// Closed value sets — the exact vocabularies of la-musica `mlo/tagtext.py`
// (`CANONICAL_VALUES` / `CANONICAL_CASE`). A value outside a vocabulary is
// returned unchanged (never coerced into a wrong answer), which is what makes
// the rule idempotent.
// ---------------------------------------------------------------------------

/// MusicBrainz medium formats, in the spelling this app stores.
pub const MEDIA_VALUES: &[&str] = &[
    "12\" Vinyl", "10\" Vinyl", "7\" Vinyl", "8-Track", "Blu-ray", "Blu-spec CD", "Cassette",
    "CD", "CD-R", "Digital Media", "DVD", "DVD-Audio", "DVD-Video", "HDCD", "LaserDisc",
    "Minidisc", "SACD", "SHM-CD", "VHS", "Vinyl", "Web",
];

/// The media values that name a CD-DA disc.
pub const CD_MEDIA_VALUES: &[&str] = &["CD", "HDCD"];

/// MusicBrainz release types (primary + secondary) in MusicBrainz's casing.
pub const RELEASE_TYPE_VALUES: &[&str] = &[
    "Album", "EP", "Single", "Broadcast", "Other", "Compilation", "Soundtrack", "Spokenword",
    "Interview", "Audiobook", "Live", "Remix", "DJ-mix", "Mixtape/Street", "Demo", "Audio drama",
    "Field recording", "Podcast",
];

/// MusicBrainz release statuses.
pub const RELEASE_STATUS_VALUES: &[&str] = &[
    "Official", "Promotion", "Bootleg", "Pseudo-Release", "Withdrawn", "Expired", "Cancelled",
];

/// The mood words the classifier derives, capitalised as a tag stores them.
pub const MOOD_VALUES: &[&str] = &[
    "Happy", "Energetic", "Aggressive", "Sad", "Calm", "Dreamy", "Dark", "Party",
];

/// AudioAuditor's verdicts.
pub const AUDIT_VALUES: &[&str] = &["REAL", "FAKE", "MIX"];

/// Where a rip came from — the values this app writes. Anything else is the
/// user's own word and is left as it stands.
pub const SOURCE_VALUES: &[&str] = &["Soulseek", "Digital", "YouTube"];

fn closed_set(tag: &str) -> Option<&'static [&'static str]> {
    match tag {
        "MEDIA" => Some(MEDIA_VALUES),
        "SOURCE" => Some(SOURCE_VALUES),
        "RELEASETYPE" => Some(RELEASE_TYPE_VALUES),
        "RELEASESTATUS" => Some(RELEASE_STATUS_VALUES),
        "AUDIT" => Some(AUDIT_VALUES),
        "MOOD" => Some(MOOD_VALUES),
        _ => None,
    }
}

/// True when a MEDIA value names a CD-DA disc (CD, or a CD variant).
pub fn is_cd_media(value: &str) -> bool {
    let f = value.trim().to_lowercase();
    CD_MEDIA_VALUES.iter().any(|v| v.to_lowercase() == f)
}

/// An ISO 3166-1 alpha-2 code, upper-cased; anything else unchanged.
fn upper_code(part: &str) -> String {
    if part.len() == 2 && part.is_ascii() && part.chars().all(|c| c.is_ascii_alphabetic()) {
        part.to_ascii_uppercase()
    } else {
        part.to_string()
    }
}

/// An ISO 15924 script code in its own casing ("latn" -> "Latn").
fn title_code(part: &str) -> String {
    if part.len() == 4 && part.is_ascii() && part.chars().all(|c| c.is_ascii_alphabetic()) {
        let mut c = part.to_ascii_lowercase();
        c[..1].make_ascii_uppercase();
        c
    } else {
        part.to_string()
    }
}

/// The canonical spelling of one value for `tag` (la-musica `canonical_value`).
/// A `"; "`-joined value is canonicalised part by part and rejoined.
pub fn canonical_value(tag: &str, value: &str) -> String {
    let t = tag.to_ascii_uppercase();
    let rule = |part: &str| -> String {
        if let Some(set) = closed_set(&t) {
            let lower = part.to_lowercase();
            if let Some(c) = set.iter().find(|c| c.to_lowercase() == lower) {
                return (*c).to_string();
            }
            part.to_string()
        } else {
            match t.as_str() {
                "RELEASECOUNTRY" => upper_code(part),
                "SCRIPT" => title_code(part),
                _ => part.to_string(),
            }
        }
    };
    if value.contains(MULTI_JOINER) {
        value
            .split(MULTI_JOINER)
            .map(|p| rule(p.trim()))
            .collect::<Vec<_>>()
            .join(MULTI_JOINER)
    } else {
        rule(value)
    }
}

/// Values whose whitespace is content and that no spacing/case rule may touch.
const MULTILINE_TAGS: &[&str] = &["LYRICS", "UNSYNCEDLYRICS", "SYNCLYRICS"];

pub fn is_multiline(tag: &str, text: &str) -> bool {
    text.contains('\n')
        || text.contains('\r')
        || MULTILINE_TAGS.contains(&tag.to_ascii_uppercase().as_str())
}

// ---------------------------------------------------------------------------
// Normalization ([§6.3], la-musica `mlo/tagtext.py`)
// ---------------------------------------------------------------------------

/// Spacing made canonical: outer spaces/tabs trimmed, a run of internal spaces
/// collapsed to one. A multi-line value is returned untouched.
pub fn collapse_spacing(value: &str) -> String {
    if value.contains('\n') || value.contains('\r') {
        return value.to_string();
    }
    let trimmed = value.trim_matches(|c| c == ' ' || c == '\t');
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_space = false;
    for ch in trimmed.chars() {
        if ch == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

/// Why `value`'s whitespace is wrong for `tag`, or `""` when it is fine.
pub fn spacing_problem(tag: &str, value: &str) -> String {
    if value.is_empty() || is_multiline(tag, value) {
        return String::new();
    }
    if value != value.trim_matches(|c| c == ' ' || c == '\t') {
        return "has leading/trailing spaces".into();
    }
    if value.contains("  ") {
        return "has a run of 2+ internal spaces".into();
    }
    String::new()
}

/// Both halves at once: the canonical spelling AND collapsed spacing — what a
/// writer stores for one single-line value.
pub fn canonical_text(tag: &str, value: &str) -> String {
    if is_multiline(tag, value) {
        return value.to_string();
    }
    collapse_spacing(&canonical_value(tag, value.trim_matches(|c| c == ' ' || c == '\t')))
}

/// True when a writer would change this value's spelling (the `tag_case` check).
pub fn needs_case_fix(tag: &str, value: &str) -> bool {
    if is_multiline(tag, value) {
        return false;
    }
    canonical_value(tag, value) != value
}

/// Normalize one value: spacing, then closed-vocabulary spelling. `None` means
/// "blank" — a blank-only value is never stored.
pub fn normalize_value(key: &str, value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    Some(canonical_text(key, value))
}

/// Normalize a whole map in place: uppercase keys, canonical values, drop
/// blank values, complete multi-values without truncating.
pub fn normalize_tags(tags: &mut TagMap) {
    let mut normalized: TagMap = TagMap::new();
    for (key, values) in tags.iter() {
        let k = key.to_ascii_uppercase();
        let entry = normalized.entry(k.clone()).or_default();
        for v in values {
            if let Some(nv) = normalize_value(&k, v) {
                if !entry.iter().any(|e| e == &nv) {
                    entry.push(nv);
                }
            }
        }
    }
    normalized.retain(|_, v| !v.is_empty());
    *tags = normalized;
}

/// Join semantics for containers that cannot repeat a field ([§6.3]).
pub const MULTI_JOINER: &str = "; ";

/// Join a list the one way a writer joins it (la-musica `join_list`).
pub fn join_list(values: &[String]) -> String {
    values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(MULTI_JOINER)
}

/// Split a `"; "`-joined value back into parts (la-musica `split_list`).
pub fn split_multi(value: &str) -> Vec<String> {
    value
        .split(MULTI_JOINER)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Which script/import owns a tag by default (Appendix B).
pub fn default_writer(key: &str) -> &'static str {
    let k = key.to_ascii_uppercase();
    match k.as_str() {
        "MEDIA" | "SOURCE" => "script 1 (Format lyrics)",
        "GENRE" => "script 8 (Auto tagging)",
        "ITUNESADVISORY" | "INSTRUMENTAL" | "ALBUMITUNESADVISORY" => "script 8 (Auto tagging)",
        "MOOD" | "ENERGY" => "script 16 (Mood & Energy)",
        "BPM" | "INITIALKEY" => "script 12 (Key & BPM)",
        "DYNAMIC RANGE" | "ALBUM DYNAMIC RANGE" => "script 7 (DR & ReplayGain)",
        k if k.starts_with("REPLAYGAIN_") => "script 7 (DR & ReplayGain)",
        "AUDIT" | "LOG_GRADE" | "LOG_CRC" | "INTEGRITY" => "script 6 (Audit library)",
        k if k.starts_with("ACOUSTID_") => "import wizard / script 21",
        k if k.starts_with("ENCODER_") => "script 3 / script 5",
        "LYRICS" | "UNSYNCEDLYRICS" => "script 13 (Fetch lyrics) / editor",
        "TRANSLITERATION" | "TRANSLATION" => "script 17 (Lyrics transliterate)",
        _ => "import (MusicBrainz)",
    }
}

/// True when a title is written in a non-Latin script and therefore needs a
/// `TITLEALIAS` (romanised) tag ([§6.2] alias rules).
pub fn needs_alias(title: &str) -> bool {
    title.chars().any(|c| {
        matches!(c as u32,
            0x3040..=0x30FF |   // Hiragana / Katakana
            0x3400..=0x4DBF |   // CJK ext A
            0x4E00..=0x9FFF |   // CJK unified
            0xAC00..=0xD7AF |   // Hangul syllables
            0x0400..=0x04FF |   // Cyrillic
            0x0600..=0x06FF |   // Arabic
            0x0590..=0x05FF     // Hebrew
        )
    })
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn detects_non_latin_titles() {
        assert!(needs_alias("夜に駆ける"));
        assert!(needs_alias("Кино"));
        assert!(!needs_alias("Kid A"));
        assert!(!needs_alias("Café Tacvba")); // Latin + diacritics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_cover_aliases_and_wildcards() {
        assert_eq!(family_of("TITLE"), TagFamily::Identity);
        assert_eq!(family_of("titlealias_ja"), TagFamily::Identity);
        assert_eq!(family_of("MUSICBRAINZ_TRACKID"), TagFamily::Release);
        assert_eq!(family_of("RATEYOURMUSIC_ALBUM"), TagFamily::Release);
        assert_eq!(family_of("DYNAMIC RANGE"), TagFamily::Audio);
        assert_eq!(family_of("UNSYNCEDLYRICS"), TagFamily::Lyrics);
        assert_eq!(family_of("ENCODER_PROGRAM"), TagFamily::Provenance);
        assert_eq!(family_of("WEIRD_KEY"), TagFamily::Foreign);
    }

    #[test]
    fn spacing_is_idempotent_and_drops_blanks() {
        assert_eq!(collapse_spacing("  a   b  "), "a b");
        assert_eq!(spacing_problem("TITLE", "a b"), "");
        assert_eq!(spacing_problem("TITLE", " a b"), "has leading/trailing spaces");
        assert_eq!(spacing_problem("TITLE", "a  b"), "has a run of 2+ internal spaces");
        // a multi-line value is never judged
        assert_eq!(spacing_problem("LYRICS", " a\n  b  "), "");
        assert_eq!(normalize_value("TITLE", "   "), None);
    }

    #[test]
    fn closed_vocabularies_canonicalise_and_keep_unknowns() {
        assert_eq!(normalize_value("MEDIA", "cd"), Some("CD".into()));
        assert_eq!(normalize_value("MEDIA", "digital media"), Some("Digital Media".into()));
        assert_eq!(normalize_value("SOURCE", "soulseek"), Some("Soulseek".into()));
        assert_eq!(normalize_value("releasetype", "dj-mix"), Some("DJ-mix".into()));
        assert_eq!(normalize_value("MOOD", "calm"), Some("Calm".into()));
        assert_eq!(normalize_value("AUDIT", "real"), Some("REAL".into()));
        // unknown value stays as stated
        assert_eq!(normalize_value("MEDIA", "Cyberdisc"), Some("Cyberdisc".into()));
        // code-shaped rules
        assert_eq!(canonical_value("RELEASECOUNTRY", "us"), "US");
        assert_eq!(canonical_value("SCRIPT", "latn"), "Latn");
        // free text is untouched
        assert_eq!(canonical_value("TITLE", "AC/DC"), "AC/DC");
    }

    #[test]
    fn multiline_values_are_never_collapsed() {
        assert_eq!(canonical_text("LYRICS", "  line one\n   line two  "), "  line one\n   line two  ");
    }

    #[test]
    fn list_join_and_split_use_one_separator() {
        assert_eq!(join_list(&["a".into(), "b".into()]), "a; b");
        assert_eq!(split_multi("a; b ; c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn multi_values_complete_not_truncate() {
        let mut t = TagMap::new();
        t.insert("genre".into(), vec!["Rock".into(), "  Rock ".into(), "Jazz".into()]);
        normalize_tags(&mut t);
        assert_eq!(t["GENRE"], vec!["Rock".to_string(), "Jazz".to_string()]);
    }
}