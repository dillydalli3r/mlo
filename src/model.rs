//! Shared domain model — the vocabulary every module speaks.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Tag key -> values. Keys are canonical uppercase; values order-preserving.
pub type TagMap = BTreeMap<String, Vec<String>>;

/// Container formats the app understands ([§6.1]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerKind {
    Flac,
    OggVorbis,
    Opus,
    Mp3,
    Mp4,
    Wav,
    Aiff,
    Mkv,
    Mp4Video,
    Unknown,
}

impl ContainerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContainerKind::Flac => "flac",
            ContainerKind::OggVorbis => "ogg",
            ContainerKind::Opus => "opus",
            ContainerKind::Mp3 => "mp3",
            ContainerKind::Mp4 => "m4a",
            ContainerKind::Wav => "wav",
            ContainerKind::Aiff => "aiff",
            ContainerKind::Mkv => "mkv",
            ContainerKind::Mp4Video => "mp4",
            ContainerKind::Unknown => "unknown",
        }
    }

    /// Can this container carry tags at all? ([§6.1] last sentence.)
    pub fn taggable(self) -> bool {
        !matches!(self, ContainerKind::Unknown)
    }

    pub fn is_video(self) -> bool {
        matches!(self, ContainerKind::Mkv | ContainerKind::Mp4Video)
    }
}

/// A directory scope an action runs over ([§2] Scopes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Library,
    Artist(PathBuf),
    Album(PathBuf),
    Track(PathBuf),
    Selection(Vec<PathBuf>),
}

impl Scope {
    pub fn kind(&self) -> ScopeKind {
        match self {
            Scope::Library => ScopeKind::Library,
            Scope::Artist(_) => ScopeKind::Artist,
            Scope::Album(_) => ScopeKind::Album,
            Scope::Track(_) => ScopeKind::Track,
            Scope::Selection(_) => ScopeKind::Selection,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Scope::Library => "library".into(),
            Scope::Artist(p) => format!("artist {}", file_name(p)),
            Scope::Album(p) => format!("album {}", file_name(p)),
            Scope::Track(p) => format!("track {}", file_name(p)),
            Scope::Selection(v) => format!("selection ({} items)", v.len()),
        }
    }

    /// Roots the scope covers, used to confine a scoped scan ([§4.2] rule 3).
    pub fn roots(&self) -> Vec<PathBuf> {
        match self {
            Scope::Library => Vec::new(),
            Scope::Artist(p) | Scope::Album(p) | Scope::Track(p) => vec![p.clone()],
            Scope::Selection(v) => match v {
                // collapse selections to their common parents later; keep raw here
                _ => v.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScopeKind {
    Library,
    Artist,
    Album,
    Track,
    Selection,
}

pub fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

// ---------------------------------------------------------------------------
// Layout findings ([§4.2])
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FindingKind {
    AudioAtRoot,
    AudioInArtists,
    AudioInArtist,
    UnexpectedFolder,
    UnexpectedSubfolder,
    EmptyAlbum,
    EmptyArtist,
    SplitArtist,
    WrongCase,
    SidecarCopy,
    StrayFile,
    StrayInArtists,
    HiddenFolder,
    LegacyStateFile,
}

impl FindingKind {
    pub const ALL: [FindingKind; 14] = [
        FindingKind::AudioAtRoot,
        FindingKind::AudioInArtists,
        FindingKind::AudioInArtist,
        FindingKind::UnexpectedFolder,
        FindingKind::UnexpectedSubfolder,
        FindingKind::EmptyAlbum,
        FindingKind::EmptyArtist,
        FindingKind::SplitArtist,
        FindingKind::WrongCase,
        FindingKind::SidecarCopy,
        FindingKind::StrayFile,
        FindingKind::StrayInArtists,
        FindingKind::HiddenFolder,
        FindingKind::LegacyStateFile,
    ];

    /// snake_case code, verbatim in the CLI ([§4.2]).
    pub fn code(self) -> &'static str {
        match self {
            FindingKind::AudioAtRoot => "audio_at_root",
            FindingKind::AudioInArtists => "audio_in_artists",
            FindingKind::AudioInArtist => "audio_in_artist",
            FindingKind::UnexpectedFolder => "unexpected_folder",
            FindingKind::UnexpectedSubfolder => "unexpected_subfolder",
            FindingKind::EmptyAlbum => "empty_album",
            FindingKind::EmptyArtist => "empty_artist",
            FindingKind::SplitArtist => "split_artist",
            FindingKind::WrongCase => "wrong_case",
            FindingKind::SidecarCopy => "sidecar_copy",
            FindingKind::StrayFile => "stray_file",
            FindingKind::StrayInArtists => "stray_in_artists",
            FindingKind::HiddenFolder => "hidden_folder",
            FindingKind::LegacyStateFile => "legacy_state_file",
        }
    }

    /// Grading check key charged for this finding ([§9.2]).
    pub fn check_key(self) -> String {
        format!("layout_{}", self.code())
    }

    pub fn label(self) -> &'static str {
        match self {
            FindingKind::AudioAtRoot => "Audio file at library root",
            FindingKind::AudioInArtists => "Audio file directly in Artists/",
            FindingKind::AudioInArtist => "Audio file directly in artist folder",
            FindingKind::UnexpectedFolder => "Foreign folder in library root",
            FindingKind::UnexpectedSubfolder => "Unexpected subfolder in album",
            FindingKind::EmptyAlbum => "Album folder with no audio",
            FindingKind::EmptyArtist => "Artist folder with no albums",
            FindingKind::SplitArtist => "Two folders name one artist",
            FindingKind::WrongCase => "Folder/file name differs by case",
            FindingKind::SidecarCopy => "Duplicate/numbered sidecar",
            FindingKind::StrayFile => "Stray file inside album",
            FindingKind::StrayInArtists => "Stray file directly in Artists/",
            FindingKind::HiddenFolder => "Hidden folder inside Artists/",
            FindingKind::LegacyStateFile => "Leftover of the old .mlo_data layout",
        }
    }
}

/// Concrete fix a finding offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixKind {
    /// Move file(s)/folder into `target`.
    Move { target: PathBuf },
    /// Move to Trash with `reason`.
    Trash { reason: String },
    /// Rename in place to `target`.
    Rename { target: PathBuf },
    /// Merge artist artefacts into `into` and trash the emptied folder.
    MergeArtist { into: PathBuf },
    /// Report only — no fix.
    None,
}

impl FixKind {
    pub fn label(&self) -> String {
        match self {
            FixKind::Move { target } => format!("move → {}", target.display()),
            FixKind::Trash { reason } => format!("trash ({reason})"),
            FixKind::Rename { target } => format!("rename → {}", file_name(target)),
            FixKind::MergeArtist { into } => format!("merge → {}", file_name(into)),
            FixKind::None => "report only".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable identity: `<kind>:<path>`.
    pub id: String,
    pub kind: FindingKind,
    pub path: PathBuf,
    /// Absolute spelling of `path`, native separators — the report's `abs`
    /// column (la-musica `_issue`).
    pub abs: PathBuf,
    /// Album this finding is charged to, when album-scoped.
    pub album: Option<PathBuf>,
    /// Artist this finding is charged to, when artist-scoped.
    pub artist: Option<PathBuf>,
    /// True when the finding concerns the library as a whole (charged on the
    /// explicit `library` row, [§9.2]).
    pub library_wide: bool,
    pub detail: String,
    /// One line saying what the fix does — or why the row stays (la-musica
    /// `hint`).
    pub hint: String,
    pub fix: FixKind,
}

impl Finding {
    pub fn new(kind: FindingKind, path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        let path = path.into();
        let abs = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
        Self {
            id: format!("{}:{}", kind.code(), path.display()),
            kind,
            path,
            abs,
            album: None,
            artist: None,
            library_wide: false,
            detail: detail.into(),
            hint: String::new(),
            fix: FixKind::None,
        }
    }

    pub fn with_fix(mut self, fix: FixKind) -> Self {
        self.fix = fix;
        self
    }

    /// Override the absolute spelling (used when a fix rewrites `path`).
    pub fn with_abs(mut self, abs: impl Into<PathBuf>) -> Self {
        self.abs = abs.into();
        self
    }

    /// The one-line fix hint / reason the row stays.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    pub fn on_album(mut self, album: impl Into<PathBuf>) -> Self {
        self.album = Some(album.into());
        self
    }

    pub fn on_artist(mut self, artist: impl Into<PathBuf>) -> Self {
        self.artist = Some(artist.into());
        self
    }

    pub fn library_wide(mut self) -> Self {
        self.library_wide = true;
        self
    }

    pub fn fixable(&self) -> bool {
        !matches!(self.fix, FixKind::None)
    }

    /// The scope row this finding is charged to, for grading.
    pub fn charge_target(&self) -> ChargeTarget {
        if self.library_wide {
            ChargeTarget::Library
        } else if let Some(a) = &self.album {
            ChargeTarget::Album(a.clone())
        } else if let Some(a) = &self.artist {
            ChargeTarget::Artist(a.clone())
        } else {
            ChargeTarget::Library
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChargeTarget {
    Library,
    Album(PathBuf),
    Artist(PathBuf),
}

/// The persisted layout report document, matching la-musica's
/// `<music>/.mlo/data/layout_report.json` byte for byte ([§3.4]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutReportDoc {
    /// UTC, `%Y-%m-%dT%H:%M:%SZ`.
    pub scanned_at: String,
    /// The music folder the scan looked at, slash-normalised.
    pub music_folder: String,
    pub report: LayoutReportBody,
}

/// The scan payload (la-musica `scan_library`'s return dict).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutReportBody {
    pub folder: String,
    pub artists_dir: String,
    pub exists: bool,
    pub issues: Vec<LayoutIssueRow>,
    pub counts: BTreeMap<String, u32>,
    pub total: u32,
    pub albums: u32,
    pub artists: u32,
    pub audio_files: u64,
}

/// One issue row (la-musica `_issue`). `path` is music-folder-relative for
/// display, `abs` the absolute spelling; both slash-normalised.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutIssueRow {
    pub kind: String,
    pub path: String,
    pub abs: String,
    pub detail: String,
    pub hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<LayoutFixRow>,
}

/// What the apply will do about a row: `{"action", "to"}` (la-musica `fix`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutFixRow {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

// ---------------------------------------------------------------------------
// Grading ([§9])
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    Pass,
    Fail(Issue),
    /// Not applicable — counted on neither side ([§9.1.3]).
    Skipped,
    /// Raised while evaluating — counted as failed ([§9.1.2]).
    CouldNotEvaluate(String),
}

impl CheckStatus {
    pub fn passed(&self) -> bool {
        matches!(self, CheckStatus::Pass)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub key: String,
    pub label: String,
    pub status: CheckStatus,
}

impl CheckResult {
    pub fn pass(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into(), status: CheckStatus::Pass }
    }
    pub fn fail(
        key: impl Into<String>,
        label: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            status: CheckStatus::Fail(Issue {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
    pub fn skipped(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self { key: key.into(), label: label.into(), status: CheckStatus::Skipped }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradeReport {
    pub scope_label: String,
    pub checks: Vec<CheckResult>,
    pub total_checks: u32,
    pub failed_checks: u32,
}

impl GradeReport {
    pub fn new(scope_label: impl Into<String>, checks: Vec<CheckResult>) -> Self {
        let mut total = 0u32;
        let mut failed = 0u32;
        for c in &checks {
            match &c.status {
                CheckStatus::Pass | CheckStatus::Fail(_) | CheckStatus::CouldNotEvaluate(_) => {
                    total += 1;
                    if !c.status.passed() {
                        failed += 1;
                    }
                }
                CheckStatus::Skipped => {}
            }
        }
        Self { scope_label: scope_label.into(), checks, total_checks: total, failed_checks: failed }
    }

    pub fn pass_count(&self) -> u32 {
        self.total_checks.saturating_sub(self.failed_checks)
    }

    /// Binary verdict ([§1.6]).
    pub fn passed(&self) -> bool {
        self.failed_checks == 0
    }

    pub fn pct(&self) -> f64 {
        if self.total_checks == 0 {
            100.0
        } else {
            self.pass_count() as f64 * 100.0 / self.total_checks as f64
        }
    }

    pub fn issues(&self) -> impl Iterator<Item = (&CheckResult, &Issue)> {
        self.checks.iter().filter_map(|c| match &c.status {
            CheckStatus::Fail(i) => Some((c, i)),
            _ => None,
        })
    }
}

/// Aggregate library verdict for the status/dashboard screen ([§9.5], [§9.2]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryVerdict {
    pub albums_total: u32,
    pub albums_passed: u32,
    pub albums_failed: u32,
    pub albums_audit_failed: u32,
    pub artists_total: u32,
    pub artists_passed: u32,
    pub artists_failed: u32,
    pub tracks_total: u32,
    pub library_row: Option<GradeReport>,
}

impl LibraryVerdict {
    pub fn albums_pct(&self) -> f64 {
        if self.albums_total == 0 {
            100.0
        } else {
            self.albums_passed as f64 * 100.0 / self.albums_total as f64
        }
    }
    pub fn overall_pass(&self) -> bool {
        self.albums_failed == 0
            && self.artists_failed == 0
            && self.library_row.as_ref().map(|r| r.passed()).unwrap_or(true)
    }
}

// ---------------------------------------------------------------------------
// Jobs ([§12.4])
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: i64,
    pub kind: String,
    pub scope: String,
    pub state: JobState,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub journal: String,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub label: String,
    pub current: u64,
    pub total: Option<u64>,
    pub note: Option<String>,
}

impl Progress {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), current: 0, total: None, note: None }
    }
    pub fn fraction(&self) -> f64 {
        match self.total {
            Some(t) if t > 0 => (self.current as f64 / t as f64).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Player state ([§11.2])
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
    Off,
    All,
    One,
}

impl Default for Repeat {
    fn default() -> Self {
        Repeat::Off
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub path: Option<PathBuf>,
    pub position_ms: u64,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: Repeat,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self { path: None, position_ms: 0, volume: 0.8, shuffle: false, repeat: Repeat::Off }
    }
}