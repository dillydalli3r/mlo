//! TUI application state and actions ([§13]).

use crate::config::Config;
use crate::db::Db;
use crate::error::Result;
use crate::jobs::{self, JobHandle, JobMsg, JobRequest};
use crate::model::{Finding, Progress, Scope};
use crate::scan::LibraryModel;
use ratatui::widgets::{ListState, TableState};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Library,
    Album,
    Artist,
    Grade,
    Layout,
    Scripts,
    Import,
    Tools,
    Player,
    Log,
    Settings,
    Trash,
}

impl Screen {
    pub const TABS: [Screen; 12] = [
        Screen::Dashboard,
        Screen::Library,
        Screen::Album,
        Screen::Artist,
        Screen::Grade,
        Screen::Layout,
        Screen::Scripts,
        Screen::Import,
        Screen::Tools,
        Screen::Player,
        Screen::Log,
        Screen::Settings,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Screen::Dashboard => "Library status",
            Screen::Library => "Library",
            Screen::Album => "Album",
            Screen::Artist => "Artist",
            Screen::Grade => "Grade",
            Screen::Layout => "Layout",
            Screen::Scripts => "Scripts",
            Screen::Import => "Import",
            Screen::Tools => "Tools",
            Screen::Player => "Player",
            Screen::Log => "Log",
            Screen::Settings => "Settings",
            Screen::Trash => "Trash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Working,
    Error,
}

#[derive(Debug, Clone)]
pub struct StatusLine {
    pub kind: StatusKind,
    pub message: String,
}

impl StatusLine {
    pub fn info(m: impl Into<String>) -> Self {
        Self { kind: StatusKind::Info, message: m.into() }
    }
    pub fn error(m: impl Into<String>) -> Self {
        Self { kind: StatusKind::Error, message: m.into() }
    }
    pub fn working(m: impl Into<String>) -> Self {
        Self { kind: StatusKind::Working, message: m.into() }
    }
}

#[derive(Debug, Clone)]
pub struct ToolRow {
    pub name: String,
    pub expected: String,
    pub found: String,
    pub kind: String,
    pub path: String,
    pub last_failure: String,
}

#[derive(Debug, Clone)]
pub struct TrashRow {
    pub user: String,
    pub stamp: String,
    pub created_at: String,
    pub entries: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum TagMode {
    Browse,
    Editing { key: String, buffer: String },
    AddingKey { buffer: String },
    AddingValue { key: String, buffer: String },
}

#[derive(Debug, Clone)]
pub struct TagEditor {
    pub path: PathBuf,
    pub title: String,
    pub tags: Vec<(String, Vec<String>)>,
    pub sel: usize,
    pub mode: TagMode,
    pub dirty: bool,
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub prompt: String,
    pub action: ConfirmAction,
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    Quit,
    ApplyLayout(Option<Vec<String>>),
    RestoreTrash(usize),
}

pub struct App {
    pub cfg: Config,
    pub config_path: std::path::PathBuf,
    pub model: LibraryModel,
    pub screen: Screen,
    pub dirty_status: Vec<String>,

    pub artist_sel: usize,
    pub album_sel: usize,
    pub track_sel: usize,
    pub artist_state: ListState,
    pub album_state: ListState,
    pub track_state: TableState,
    pub table_state: TableState,
    pub find_state: TableState,

    pub pane: usize, // 0 = artists, 1 = albums, 2 = tracks

    pub status: StatusLine,
    pub log: Vec<String>,
    pub log_scroll: usize,
    pub help: bool,
    pub confirm: Option<Confirm>,
    pub should_quit: bool,

    pub job: Option<JobHandle>,
    pub progress: Option<Progress>,

    pub layout_findings: Vec<Finding>,
    pub layout_selected: HashSet<String>,
    pub layout_dry_run: bool,
    pub layout_state: TableState,

    pub script_sel: usize,
    pub script_state: ListState,
    pub last_script: Option<crate::scripts::RunOutcome>,

    pub tag_editor: Option<TagEditor>,
    pub grade_scroll: usize,
    pub settings_sel: usize,
    pub settings_state: ListState,
    pub trash: Vec<TrashRow>,
    pub tools: Vec<ToolRow>,

    #[cfg(feature = "audio")]
    pub player: Option<crate::player::Player>,
    #[cfg(not(feature = "audio"))]
    pub player: Option<crate::player::Player>,
    pub queue: Vec<PathBuf>,
    pub now_playing: Option<PathBuf>,
    pub restored_volume: f32,
    pub playing: bool,

    pub import_path: String,
    pub import_mode: usize,
    pub input_mode: bool,
    pub input_buffer: String,

    pub cancel: Arc<AtomicBool>,
}

impl App {
    pub fn new(cfg: Config, model: LibraryModel) -> Self {
        let mut artist_state = ListState::default();
        artist_state.select(Some(0));
        Self {
            cfg,
            config_path: Config::default_path(),
            model,
            screen: Screen::Dashboard,
            dirty_status: Vec::new(),
            artist_sel: 0,
            album_sel: 0,
            track_sel: 0,
            artist_state,
            album_state: ListState::default(),
            track_state: TableState::default(),
            table_state: TableState::default(),
            find_state: TableState::default(),
            pane: 0,
            status: StatusLine::info("ready"),
            log: Vec::new(),
            log_scroll: 0,
            help: false,
            confirm: None,
            should_quit: false,
            job: None,
            progress: None,
            layout_findings: Vec::new(),
            layout_selected: HashSet::new(),
            layout_dry_run: true,
            layout_state: TableState::default(),
            script_sel: 0,
            script_state: ListState::default(),
            last_script: None,
            tag_editor: None,
            grade_scroll: 0,
            settings_sel: 0,
            settings_state: ListState::default(),
            trash: Vec::new(),
            tools: Vec::new(),
            player: None,
            queue: Vec::new(),
            now_playing: None,
            restored_volume: 0.8,
            playing: false,
            import_path: String::new(),
            import_mode: 0,
            input_mode: false,
            input_buffer: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > 2000 {
            self.log.drain(0..500);
        }
        self.log_scroll = self.log.len().saturating_sub(1);
    }

    pub fn busy(&self) -> bool {
        self.job.as_ref().map(|j| !j.is_done()).unwrap_or(false)
    }

    // --- navigation helpers -------------------------------------------------

    pub fn current_artist(&self) -> Option<&crate::scan::ArtistEntry> {
        self.model.artists.get(self.artist_sel)
    }

    pub fn current_album(&self) -> Option<&crate::scan::AlbumEntry> {
        self.current_artist().and_then(|a| a.albums.get(self.album_sel))
    }

    pub fn current_track(&self) -> Option<&crate::scan::TrackEntry> {
        self.current_album().and_then(|a| a.tracks.get(self.track_sel))
    }

    pub fn current_scope(&self) -> Scope {
        match self.screen {
            Screen::Artist => self
                .current_artist()
                .map(|a| Scope::Artist(a.path.clone()))
                .unwrap_or(Scope::Library),
            Screen::Album | Screen::Grade | Screen::Library => match self.current_track() {
                Some(t) => Scope::Track(t.path.clone()),
                None => self
                    .current_album()
                    .map(|a| Scope::Album(a.path.clone()))
                    .unwrap_or(Scope::Library),
            },
            _ => Scope::Library,
        }
    }

    // --- jobs ---------------------------------------------------------------

    pub fn start_job(&mut self, req: JobRequest) {
        if self.busy() {
            self.status = StatusLine::error("a job is already running (Esc cancels)");
            return;
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        match jobs::spawn(req, self.cfg.clone()) {
            Ok(handle) => {
                self.status = StatusLine::working(format!("{} …", handle.kind));
                self.push_log(format!("started {} on {}", handle.kind, handle.scope_label));
                self.job = Some(handle);
            }
            Err(e) => self.status = StatusLine::error(e.to_string()),
        }
    }

    pub fn scan(&mut self, scope: Scope) {
        self.start_job(JobRequest::Scan(scope));
    }

    pub fn run_script(&mut self, id: u8, scope: Scope) {
        self.start_job(JobRequest::Script { id, scope });
    }

    pub fn poll_job(&mut self) {
        let Some(handle) = self.job.take() else { return };
        let msgs = handle.drain();
        let mut finished = false;
        for m in msgs {
            match m {
                JobMsg::Progress(p) => self.progress = Some(p),
                JobMsg::Log(line) => self.push_log(line),
                JobMsg::ScanDone(model) => {
                    self.model = *model;
                    self.clamp_selection();
                    self.refresh_layout_from_model();
                    self.status = StatusLine::info(format!(
                        "indexed {} tracks in {} albums",
                        self.model.track_count(),
                        self.model.album_count()
                    ));
                }
                JobMsg::ScriptDone(outcome) => {
                    self.status = StatusLine::info(outcome.summary());
                    self.last_script = Some(*outcome);
                }
                JobMsg::LayoutDone(outcomes) => {
                    let ok = outcomes.iter().filter(|o| o.ok).count();
                    let fail = outcomes.len() - ok;
                    self.status = StatusLine::info(format!("layout apply: {ok} ok, {fail} failed"));
                    // re-scan so the panel reflects reality
                    self.scan(Scope::Library);
                }
                JobMsg::Error(e) => {
                    self.status = StatusLine::error(e.clone());
                    self.push_log(format!("error: {e}"));
                }
                JobMsg::Finished => {
                    finished = true;
                    self.progress = None;
                }
            }
        }
        if finished {
            self.job = None;
        } else {
            self.job = Some(handle);
        }
    }

    fn clamp_selection(&mut self) {
        if self.model.artists.is_empty() {
            self.artist_sel = 0;
            self.album_sel = 0;
            self.track_sel = 0;
            return;
        }
        self.artist_sel = self.artist_sel.min(self.model.artists.len() - 1);
        let albums = self.model.artists[self.artist_sel].albums.len();
        self.album_sel = if albums == 0 { 0 } else { self.album_sel.min(albums - 1) };
        let tracks = self.model.artists[self.artist_sel].albums.get(self.album_sel).map(|a| a.tracks.len()).unwrap_or(0);
        self.track_sel = if tracks == 0 { 0 } else { self.track_sel.min(tracks - 1) };
    }

    pub fn refresh_layout_from_model(&mut self) {
        self.layout_findings = self.model.all_findings().into_iter().cloned().collect();
        self.layout_selected = self
            .layout_findings
            .iter()
            .filter(|f| f.fixable())
            .map(|f| f.id.clone())
            .collect();
    }

    pub fn load_trash(&mut self) {
        let trash = crate::trash::Trash::new(&self.cfg.music_folder);
        match trash.list() {
            Ok(list) => {
                self.trash = list
                    .into_iter()
                    .map(|m| TrashRow {
                        user: m.user,
                        stamp: m.stamp,
                        created_at: m.created_at,
                        entries: m.entries,
                        path: m.path,
                    })
                    .collect();
            }
            Err(e) => self.status = StatusLine::error(e.to_string()),
        }
    }

    pub fn load_tools(&mut self) {
        self.tools = crate::cli::tools_doctor_rows(&self.cfg);
    }

    pub fn toggle_playing(&mut self) {
        #[cfg(feature = "audio")]
        {
            if self.player.is_none() {
                match crate::player::Player::new(self.restored_volume) {
                    Ok(p) => self.player = Some(p),
                    Err(e) => {
                        self.status = StatusLine::error(format!("playback unavailable: {e}"));
                        return;
                    }
                }
            }
            if let Some(p) = self.player.as_mut() {
                p.toggle();
                self.playing = p.is_playing();
            }
        }
        #[cfg(not(feature = "audio"))]
        {
            self.status = StatusLine::error("playback unavailable: built without the `audio` feature");
        }
    }

    pub fn play_track(&mut self, path: PathBuf) {
        #[cfg(feature = "audio")]
        {
            if self.player.is_none() {
                match crate::player::Player::new(self.restored_volume) {
                    Ok(p) => self.player = Some(p),
                    Err(e) => {
                        self.status = StatusLine::error(format!("playback unavailable: {e}"));
                        return;
                    }
                }
            }
            if let Some(p) = self.player.as_mut() {
                match p.play_file(&path) {
                    Ok(()) => {
                        self.now_playing = Some(path.clone());
                        self.playing = true;
                        self.status = StatusLine::info(format!("playing {}", crate::model::file_name(&path)));
                    }
                    Err(e) => self.status = StatusLine::error(format!("cannot play: {e}")),
                }
            }
        }
        #[cfg(not(feature = "audio"))]
        {
            let _ = path;
            self.status = StatusLine::error("playback unavailable: built without the `audio` feature");
        }
    }

    /// Queue the whole current album.
    pub fn queue_album(&mut self) {
        if let Some(album) = self.current_album() {
            self.queue = album.tracks.iter().map(|t| t.path.clone()).collect();
            self.status = StatusLine::info(format!("queued {} tracks", self.queue.len()));
        }
    }

    pub fn persist_player_state(&self) {
        if let Ok(db) = Db::open(&self.cfg.index_db()) {
            let st = crate::model::PlayerState {
                path: self.now_playing.clone(),
                position_ms: self.player.as_ref().map(|p| p.position_ms()).unwrap_or(0),
                volume: self.player.as_ref().map(|p| p.volume()).unwrap_or(0.8),
                shuffle: false,
                repeat: crate::model::Repeat::Off,
            };
            let _ = db.set_player_state(&st);
            let _ = db.queue_replace(&self.queue);
        }
    }

    pub fn cancel_job(&mut self) {
        if let Some(j) = &self.job {
            j.cancel();
            self.status = StatusLine::working("cancelling…");
        }
        self.cancel.store(true, Ordering::Relaxed);
    }

    // --- tag editing --------------------------------------------------------

    pub fn open_tag_editor(&mut self, path: PathBuf) {
        match crate::tags::read_tags(&path) {
            Ok(tags) => {
                let mut list: Vec<(String, Vec<String>)> = tags.into_iter().collect();
                list.sort_by(|a, b| a.0.cmp(&b.0));
                let title = crate::model::file_name(&path);
                self.tag_editor = Some(TagEditor { path, title, tags: list, sel: 0, mode: TagMode::Browse, dirty: false });
                self.screen = Screen::Album;
            }
            Err(e) => self.status = StatusLine::error(format!("cannot read tags: {e}")),
        }
    }

    pub fn save_tag_editor(&mut self) -> Result<()> {
        let Some(editor) = self.tag_editor.clone() else { return Ok(()) };
        let map: crate::model::TagMap = editor.tags.iter().cloned().collect();
        crate::tags::write_tags(&editor.path, &map)?;
        self.status = StatusLine::info(format!("saved tags for {}", editor.title));
        self.push_log(format!("tags written: {}", editor.path.display()));
        // refresh the in-memory model entry
        if let Some((_, _, t)) = self.model.find_track(&editor.path) {
            let _ = t;
        }
        for artist in &mut self.model.artists {
            for album in &mut artist.albums {
                for track in &mut album.tracks {
                    if track.path == editor.path {
                        track.tags = map.clone();
                    }
                }
            }
        }
        if let Some(editor) = self.tag_editor.as_mut() {
            editor.dirty = false;
        }
        Ok(())
    }

    pub fn status_summary(&self) -> String {
        let v = &self.model.verdict;
        format!(
            "{} albums ({} pass / {} fail) · {} artists ({} pass) · {} tracks{}",
            v.albums_total,
            v.albums_passed,
            v.albums_failed,
            v.artists_total,
            v.artists_passed,
            v.tracks_total,
            if v.albums_audit_failed > 0 {
                format!(" · {} audit-failed", v.albums_audit_failed)
            } else {
                String::new()
            }
        )
    }
}

pub fn finding_counts(findings: &[Finding]) -> Vec<(crate::model::FindingKind, usize, usize)> {
    crate::layout::group_by_kind(findings)
}

impl App {
    /// Focus the artist/album/track containing `path` (used by `--open`).
    pub fn focus_path(&mut self, path: &std::path::Path) {
        let target_dir = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.to_path_buf())
        };
        for (ai, artist) in self.model.artists.iter().enumerate() {
            for (bi, album) in artist.albums.iter().enumerate() {
                if album.path == target_dir || album.path.starts_with(&target_dir) || target_dir.starts_with(&album.path) {
                    self.artist_sel = ai;
                    self.album_sel = bi;
                    self.pane = 2;
                    self.screen = Screen::Album;
                    if !path.is_dir() {
                        if let Some(ti) = album.tracks.iter().position(|t| t.path == path) {
                            self.track_sel = ti;
                        }
                    }
                    self.status = StatusLine::info(format!("opened {}", album.path.display()));
                    return;
                }
            }
        }
        // fall back to artist match, then a scoped scan
        for (ai, artist) in self.model.artists.iter().enumerate() {
            if artist.path == target_dir || target_dir.starts_with(&artist.path) {
                self.artist_sel = ai;
                self.album_sel = 0;
                self.pane = 1;
                self.screen = Screen::Artist;
                self.status = StatusLine::info(format!("opened {}", artist.path.display()));
                return;
            }
        }
        self.status = StatusLine::info("path is outside the indexed library — press s to rescan");
    }
}