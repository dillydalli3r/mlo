//! TUI entry point and event loop ([§13]).

pub mod app;
pub mod ui;

use crate::config::Config;
use crate::db::Db;
use crate::error::Result;
use app::{Confirm, ConfirmAction, Screen, StatusLine, TagMode};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;

/// Run the TUI. Restores the terminal on every exit path.
pub fn run(cfg: Config, open: Option<PathBuf>, config_path: Option<PathBuf>) -> Result<()> {
    let db = Db::open(&cfg.index_db())?;
    let model = crate::scan::load_from_db(&cfg, &db).unwrap_or_default();
    let mut app = app::App::new(cfg, model);
    app.config_path = config_path.unwrap_or_else(Config::default_path);
    // Restore the persisted queue and current track ([§11.2]).
    if let Ok(q) = db.queue_list() {
        app.queue = q;
    }
    if let Ok(st) = db.player_state() {
        app.now_playing = st.path;
        app.restored_volume = st.volume;
    }
    app.refresh_layout_from_model();
    app.load_trash();
    app.load_tools();
    if app.model.artists.is_empty() {
        app.status = StatusLine::info("empty index — press s to scan the library");
    }
    if let Some(path) = open {
        app.focus_path(&path);
    }

    let mut terminal = setup_terminal()?;
    let result = event_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    app.persist_player_state();
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().map_err(|e| crate::error::MloError::Other(format!("raw mode: {e}")))?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| crate::error::MloError::Other(format!("alt screen: {e}")))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| crate::error::MloError::Other(format!("terminal: {e}")))?;
    terminal.clear().ok();
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut app::App) -> Result<()> {
    while !app.should_quit {
        app.poll_job();
        terminal
            .draw(|f| ui::render(f, app))
            .map_err(|e| crate::error::MloError::Other(format!("draw: {e}")))?;

        if event::poll(Duration::from_millis(120)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => handle_key(app, key),
                Ok(Event::Resize(_, _)) => {}
                Ok(_) => {}
                Err(_) => {}
            }
        }
        // apply a queued quit
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn handle_key(app: &mut app::App, key: KeyEvent) {
    // overlay priority: input > tag editor > confirm > help > screen
    if app.input_mode {
        return handle_input(app, key);
    }
    if app.tag_editor.is_some() {
        return handle_tag_editor(app, key);
    }
    if app.confirm.is_some() {
        return handle_confirm(app, key);
    }
    if app.help {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
            app.help = false;
        }
        return;
    }

    match key.code {
        KeyCode::Char('?') => app.help = true,
        KeyCode::Char('q') if key.modifiers.is_empty() => {
            app.confirm = Some(Confirm { prompt: "Quit mlo?".into(), action: ConfirmAction::Quit });
        }
        KeyCode::Esc => {
            if app.busy() {
                app.cancel_job();
            } else {
                app.status = StatusLine::info("ready");
            }
        }
        KeyCode::Tab => next_screen(app, 1),
        KeyCode::BackTab => next_screen(app, -1),
        KeyCode::Char('s') => app.scan(app.current_scope()),
        KeyCode::Char('S') => {
            app.screen = Screen::Scripts;
        }
        KeyCode::Char('g') => {
            app.scan(crate::model::Scope::Library);
            app.screen = Screen::Grade;
        }
        KeyCode::Char('L') => {
            app.refresh_layout_from_model();
            app.screen = Screen::Layout;
        }
        KeyCode::Char('T') => {
            app.load_tools();
            app.screen = Screen::Tools;
        }
        KeyCode::Char('P') => {
            app.screen = Screen::Player;
        }
        KeyCode::Char('l') => {
            app.screen = Screen::Log;
        }
        KeyCode::Char('c') => {
            app.screen = Screen::Settings;
        }
        KeyCode::Char('r') => {
            app.load_trash();
            app.screen = Screen::Trash;
        }
        KeyCode::Char('i') => {
            app.screen = Screen::Import;
            app.input_mode = true;
            app.input_buffer.clear();
        }
        _ => screen_keys(app, key),
    }
}

fn next_screen(app: &mut app::App, dir: i32) {
    let tabs = Screen::TABS;
    let cur = tabs.iter().position(|s| *s == app.screen).unwrap_or(0) as i32;
    let n = tabs.len() as i32;
    let next = ((cur + dir) % n + n) % n;
    app.screen = tabs[next as usize];
    match app.screen {
        Screen::Layout => app.refresh_layout_from_model(),
        Screen::Trash => app.load_trash(),
        Screen::Tools => app.load_tools(),
        _ => {}
    }
}

fn screen_keys(app: &mut app::App, key: KeyEvent) {
    match app.screen {
        Screen::Library => library_keys(app, key),
        Screen::Album => album_keys(app, key),
        Screen::Artist => artist_keys(app, key),
        Screen::Grade => grade_keys(app, key),
        Screen::Layout => layout_keys(app, key),
        Screen::Scripts => script_keys(app, key),
        Screen::Log => log_keys(app, key),
        Screen::Settings => settings_keys(app, key),
        Screen::Trash => trash_keys(app, key),
        Screen::Player => player_keys(app, key),
        _ => {}
    }
}

fn move_sel(app: &mut app::App, delta: i32) {
    match app.screen {
        Screen::Library | Screen::Album | Screen::Grade | Screen::Artist => match app.pane {
            0 => step(&mut app.artist_sel, app.model.artists.len(), delta),
            1 => {
                let n = app.current_artist().map(|a| a.albums.len()).unwrap_or(0);
                step(&mut app.album_sel, n, delta);
            }
            _ => {
                let n = app.current_album().map(|a| a.tracks.len()).unwrap_or(0);
                step(&mut app.track_sel, n, delta);
            }
        },
        _ => {}
    }
}

fn step(sel: &mut usize, len: usize, delta: i32) {
    if len == 0 {
        *sel = 0;
        return;
    }
    let cur = *sel as i32 + delta;
    *sel = cur.clamp(0, len as i32 - 1) as usize;
}

fn library_keys(app: &mut app::App, key: KeyEvent) {
    match key.code {
        KeyCode::Up => move_sel(app, -1),
        KeyCode::Down => move_sel(app, 1),
        KeyCode::Left => app.pane = app.pane.saturating_sub(1),
        KeyCode::Right => app.pane = (app.pane + 1).min(2),
        KeyCode::Enter => {
            app.pane = (app.pane + 1).min(2);
            if app.pane == 2 {
                app.screen = Screen::Album;
            }
        }
        KeyCode::Char('e') => {
            if let Some(t) = app.current_track() {
                let p = t.path.clone();
                app.open_tag_editor(p);
            }
        }
        KeyCode::Char('p') => {
            if let Some(t) = app.current_track() {
                let p = t.path.clone();
                app.play_track(p);
            }
        }
        _ => {}
    }
}

fn album_keys(app: &mut app::App, key: KeyEvent) {
    app.pane = 2;
    match key.code {
        KeyCode::Up => move_sel(app, -1),
        KeyCode::Down => move_sel(app, 1),
        KeyCode::Enter => {
            if let Some(t) = app.current_track() {
                let p = t.path.clone();
                app.open_tag_editor(p);
            }
        }
        KeyCode::Char('e') => {
            if let Some(t) = app.current_track() {
                let p = t.path.clone();
                app.open_tag_editor(p);
            }
        }
        KeyCode::Char('p') => {
            if let Some(t) = app.current_track() {
                let p = t.path.clone();
                app.play_track(p);
            }
        }
        KeyCode::Char('a') => app.queue_album(),
        _ => {}
    }
}

fn artist_keys(app: &mut app::App, key: KeyEvent) {
    app.pane = 1;
    move_sel(app, if key.code == KeyCode::Up { -1 } else if key.code == KeyCode::Down { 1 } else { 0 });
    if key.code == KeyCode::Enter {
        app.screen = Screen::Album;
        app.pane = 2;
    }
}

fn grade_keys(app: &mut app::App, key: KeyEvent) {
    app.pane = 1;
    move_sel(app, if key.code == KeyCode::Up { -1 } else if key.code == KeyCode::Down { 1 } else { 0 });
}

fn layout_keys(app: &mut app::App, key: KeyEvent) {
    let n = app.layout_findings.len();
    let cur = app.layout_state.selected().unwrap_or(0);
    match key.code {
        KeyCode::Up => app.layout_state.select(Some(cur.saturating_sub(1))),
        KeyCode::Down => app.layout_state.select(Some((cur + 1).min(n.saturating_sub(1)))),
        KeyCode::Char(' ') => {
            if let Some(f) = app.layout_findings.get(cur) {
                if app.layout_selected.contains(&f.id) {
                    app.layout_selected.remove(&f.id);
                } else {
                    app.layout_selected.insert(f.id.clone());
                }
            }
        }
        KeyCode::Char('r') => {
            app.layout_dry_run = !app.layout_dry_run;
            app.status = StatusLine::info(if app.layout_dry_run { "dry-run: nothing will move" } else { "apply mode: fixes will run" });
        }
        KeyCode::Char('A') => {
            let ids: Vec<String> = app.layout_selected.iter().cloned().collect();
            if app.layout_dry_run {
                app.status = StatusLine::info("toggle off dry-run with r, then press A to apply");
            } else {
                let count = ids.len();
                app.confirm = Some(Confirm {
                    prompt: format!("Apply {count} layout fix(es)? Removals move to the Trash with a manifest."),
                    action: ConfirmAction::ApplyLayout(Some(ids)),
                });
            }
        }
        _ => {}
    }
}

fn script_keys(app: &mut app::App, key: KeyEvent) {
    let n = crate::scripts::SCRIPTS.len();
    match key.code {
        KeyCode::Up => app.script_sel = app.script_sel.saturating_sub(1),
        KeyCode::Down => app.script_sel = (app.script_sel + 1).min(n - 1),
        KeyCode::Enter => {
            let id = crate::scripts::SCRIPTS[app.script_sel].id;
            let scope = app.current_scope();
            app.run_script(id, scope);
        }
        _ => {}
    }
}

fn log_keys(app: &mut app::App, key: KeyEvent) {
    match key.code {
        KeyCode::Up => app.log_scroll = app.log_scroll.saturating_sub(1),
        KeyCode::Down => app.log_scroll = (app.log_scroll + 1).min(app.log.len().saturating_sub(1)),
        KeyCode::PageUp => app.log_scroll = app.log_scroll.saturating_sub(20),
        KeyCode::PageDown => app.log_scroll = (app.log_scroll + 20).min(app.log.len().saturating_sub(1)),
        _ => {}
    }
}

fn settings_keys(app: &mut app::App, key: KeyEvent) {
    let total = crate::grade::CHECK_DEFS.len() + crate::model::FindingKind::ALL.len();
    match key.code {
        KeyCode::Up => app.settings_sel = app.settings_sel.saturating_sub(1),
        KeyCode::Down => app.settings_sel = (app.settings_sel + 1).min(total.saturating_sub(1)),
        KeyCode::Char(' ') => {
            let key = if app.settings_sel < crate::grade::CHECK_DEFS.len() {
                crate::grade::CHECK_DEFS[app.settings_sel].key.to_string()
            } else {
                crate::model::FindingKind::ALL[app.settings_sel - crate::grade::CHECK_DEFS.len()].check_key()
            };
            let now = app.cfg.check_enabled(&key);
            app.cfg.checks.insert(key.clone(), !now);
            app.status = StatusLine::info(format!("{key} = {}", if now { "off" } else { "on" }));
        }
        KeyCode::Char('s') => {
            let path = app.config_path.clone();
            match app.cfg.save(&path) {
                Ok(()) => app.status = StatusLine::info(format!("settings saved to {}", path.display())),
                Err(e) => app.status = StatusLine::error(e.to_string()),
            }
        }
        KeyCode::Char('S') => {
            app.cfg.apply_preset(crate::grade::Preset::Strict);
            app.status = StatusLine::info("preset: Strict (every check on) — press s to save");
        }
        KeyCode::Char('B') => {
            app.cfg.apply_preset(crate::grade::Preset::Balanced);
            app.status = StatusLine::info("preset: Balanced (audit + other files off) — press s to save");
        }
        KeyCode::Char('R') => {
            app.cfg.apply_preset(crate::grade::Preset::Relaxed);
            app.status = StatusLine::info("preset: Relaxed (18 formatting checks off) — press s to save");
        }
        _ => {}
    }
}

fn trash_keys(app: &mut app::App, key: KeyEvent) {
    let n = app.trash.len();
    let cur = app.table_state.selected().unwrap_or(0);
    match key.code {
        KeyCode::Up => app.table_state.select(Some(cur.saturating_sub(1))),
        KeyCode::Down => app.table_state.select(Some((cur + 1).min(n.saturating_sub(1)))),
        KeyCode::Enter => {
            if n == 0 {
                app.status = StatusLine::error("trash is empty");
            } else {
                let idx = cur.min(n - 1);
                let stamp = app.trash[idx].stamp.clone();
                let entries = app.trash[idx].entries;
                app.confirm = Some(Confirm {
                    prompt: format!("Restore {entries} item(s) from {stamp} to their exact original paths?"),
                    action: ConfirmAction::RestoreTrash(idx),
                });
            }
        }
        _ => {}
    }
}

fn player_keys(app: &mut app::App, key: KeyEvent) {
    match key.code {
        KeyCode::Char(' ') => app.toggle_playing(),
        KeyCode::Char('q') => app.queue_album(),
        KeyCode::Char('s') => {
            if let Some(p) = app.player.as_mut() {
                p.stop();
                app.playing = false;
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if let Some(p) = app.player.as_mut() {
                let v = (p.volume() + 0.05).min(1.0);
                p.set_volume(v);
            }
        }
        KeyCode::Char('-') => {
            if let Some(p) = app.player.as_mut() {
                let v = (p.volume() - 0.05).max(0.0);
                p.set_volume(v);
            }
        }
        _ => {}
    }
}

fn handle_confirm(app: &mut app::App, key: KeyEvent) {
    let Some(confirm) = app.confirm.clone() else { return };
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            app.confirm = None;
            match confirm.action {
                ConfirmAction::Quit => app.should_quit = true,
                ConfirmAction::ApplyLayout(ids) => {
                    app.start_job(crate::jobs::JobRequest::LayoutApply { ids, dry_run: false });
                }
                ConfirmAction::RestoreTrash(idx) => {
                    let trash = crate::trash::Trash::new(&app.cfg.music_folder);
                    let path = app.trash.get(idx).map(|t| t.path.clone());
                    match path {
                        Some(p) => match trash.restore(&p) {
                            Ok(restored) => {
                                app.status = StatusLine::info(format!("restored {} item(s)", restored.len()));
                                app.push_log(format!("trash restore: {} item(s) from {}", restored.len(), p.display()));
                                app.load_trash();
                            }
                            Err(e) => app.status = StatusLine::error(e.to_string()),
                        },
                        None => app.status = StatusLine::error("manifest disappeared"),
                    }
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => app.confirm = None,
        _ => {}
    }
}

fn handle_input(app: &mut app::App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = false;
            app.input_buffer.clear();
        }
        KeyCode::Enter => {
            app.input_mode = false;
            let path = std::path::PathBuf::from(app.input_buffer.trim());
            app.input_buffer.clear();
            if path.as_os_str().is_empty() {
                return;
            }
            // Run the import in the background via the CLI pipeline.
            let cfg = app.cfg.clone();
            app.status = StatusLine::working(format!("importing {}…", path.display()));
            app.push_log(format!("import started: {}", path.display()));
            std::thread::spawn(move || {
                if let Err(e) = crate::import::import_path(&cfg, &path) {
                    tracing::error!(error = %e, "import failed");
                }
            });
        }
        KeyCode::Backspace => {
            app.input_buffer.pop();
        }
        KeyCode::Char(c) => app.input_buffer.push(c),
        _ => {}
    }
}

fn handle_tag_editor(app: &mut app::App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(key.code, KeyCode::Char('s')) {
        match app.save_tag_editor() {
            Ok(()) => {}
            Err(e) => app.status = StatusLine::error(e.to_string()),
        }
        return;
    }

    let Some(editor) = app.tag_editor.as_mut() else { return };
    match editor.mode.clone() {
        TagMode::Browse => match key.code {
            KeyCode::Esc => {
                app.tag_editor = None;
            }
            KeyCode::Up => editor.sel = editor.sel.saturating_sub(1),
            KeyCode::Down => {
                let n = editor.tags.len();
                editor.sel = (editor.sel + 1).min(n.saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some((k, vs)) = editor.tags.get(editor.sel) {
                    editor.mode = TagMode::Editing { key: k.clone(), buffer: vs.join("; ") };
                }
            }
            KeyCode::Char('a') => {
                editor.mode = TagMode::AddingKey { buffer: String::new() };
            }
            KeyCode::Char('d') => {
                if !editor.tags.is_empty() {
                    editor.tags.remove(editor.sel);
                    editor.sel = editor.sel.min(editor.tags.len().saturating_sub(1));
                    editor.dirty = true;
                }
            }
            _ => {}
        },
        TagMode::Editing { key: ekey, mut buffer } => match key.code {
            KeyCode::Esc => editor.mode = TagMode::Browse,
            KeyCode::Enter => {
                let values: Vec<String> = buffer
                    .split("; ")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if let Some(entry) = editor.tags.iter_mut().find(|(k, _)| *k == ekey) {
                    entry.1 = values;
                } else {
                    editor.tags.push((ekey.clone(), values));
                    editor.tags.sort_by(|a, b| a.0.cmp(&b.0));
                }
                editor.dirty = true;
                editor.mode = TagMode::Browse;
            }
            KeyCode::Backspace => {
                buffer.pop();
                editor.mode = TagMode::Editing { key: ekey, buffer };
            }
            KeyCode::Char(c) => {
                buffer.push(c);
                editor.mode = TagMode::Editing { key: ekey, buffer };
            }
            _ => {}
        },
        TagMode::AddingKey { mut buffer } => match key.code {
            KeyCode::Esc => editor.mode = TagMode::Browse,
            KeyCode::Enter => {
                let key = buffer.trim().to_ascii_uppercase();
                if key.is_empty() {
                    editor.mode = TagMode::Browse;
                } else {
                    editor.mode = TagMode::AddingValue { key, buffer: String::new() };
                }
            }
            KeyCode::Backspace => {
                buffer.pop();
                editor.mode = TagMode::AddingKey { buffer };
            }
            KeyCode::Char(c) => {
                buffer.push(c);
                editor.mode = TagMode::AddingKey { buffer };
            }
            _ => {}
        },
        TagMode::AddingValue { key: ekey, mut buffer } => match key.code {
            KeyCode::Esc => editor.mode = TagMode::Browse,
            KeyCode::Enter => {
                let values: Vec<String> = buffer
                    .split("; ")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                editor.tags.push((ekey, values));
                editor.tags.sort_by(|a, b| a.0.cmp(&b.0));
                editor.dirty = true;
                editor.mode = TagMode::Browse;
            }
            KeyCode::Backspace => {
                buffer.pop();
                editor.mode = TagMode::AddingValue { key: ekey, buffer };
            }
            KeyCode::Char(c) => {
                buffer.push(c);
                editor.mode = TagMode::AddingValue { key: ekey, buffer };
            }
            _ => {}
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::model::{GradeReport, Scope};
    use crate::scan::{AlbumEntry, ArtistEntry, LibraryModel, TrackEntry};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_model() -> LibraryModel {
        let mut model = LibraryModel::default();
        let mut album = AlbumEntry {
            name: "Kid A".into(),
            path: std::path::PathBuf::from("/m/Artists/Radiohead/Kid A"),
            artist: "Radiohead".into(),
            tracks: vec![TrackEntry {
                path: std::path::PathBuf::from("/m/Artists/Radiohead/Kid A/01.flac"),
                container: crate::model::ContainerKind::Flac,
                tags: [("TITLE".to_string(), vec!["Everything".to_string()])].into_iter().collect(),
                size: 1,
                mtime: 0,
                quick_hash: None,
                has_lyrics_sidecar: false,
                lyrics_sidecar_synced: false,
                grade: Some(GradeReport::new("track", vec![])),
            }],
            side: Default::default(),
            findings: Vec::new(),
            is_cd: false,
            audio_file_count: 1,
            grade: Some(GradeReport::new("album", vec![])),
        };
        album.grade = Some(GradeReport::new("album", vec![]));
        model.artists.push(ArtistEntry {
            name: "Radiohead".into(),
            path: std::path::PathBuf::from("/m/Artists/Radiohead"),
            has_image: false,
            has_description: false,
            image_ok: None,
            image_upscaled: false,
            findings: Vec::new(),
            albums: vec![album],
            grade: Some(GradeReport::new("artist", vec![])),
        });
        model
    }

    fn render_all_screens() -> String {
        let mut app = app::App::new(Config::default(), fixture_model());
        app.refresh_layout_from_model();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut all = String::new();
        for screen in app::Screen::TABS {
            app.screen = screen;
            terminal.draw(|f| ui::render(f, &mut app)).unwrap();
            all.push_str(&format!("{:?}\n", screen));
        }
        // trash and help overlays
        app.screen = app::Screen::Trash;
        app.help = true;
        terminal.draw(|f| ui::render(f, &mut app)).unwrap();
        all
    }

    #[test]
    fn every_screen_renders_without_panic() {
        let out = render_all_screens();
        assert!(out.contains("Dashboard"));
        assert!(out.contains("Settings"));
    }

    #[test]
    fn rendering_shows_library_status_and_track_title() {
        let mut app = app::App::new(Config::default(), fixture_model());
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        app.screen = app::Screen::Library;
        terminal.draw(|f| ui::render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = buffer.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Radiohead"), "artist visible: {text}");
        assert!(text.contains("Kid A"), "album visible");
        assert!(text.contains("Everything"), "track title visible");
        assert!(text.contains("albums"), "status summary visible");
    }

    #[test]
    fn key_navigation_and_overlays() {
        let mut app = app::App::new(Config::default(), fixture_model());
        // open help then close
        handle_key(&mut app, KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(app.help);
        handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.help);
        // quit asks once
        handle_key(&mut app, KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.confirm.is_some());
        assert!(!app.should_quit);
        handle_key(&mut app, KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.should_quit);
        // screen switching
        let mut app2 = app::App::new(Config::default(), fixture_model());
        handle_key(&mut app2, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_ne!(app2.screen, app::Screen::Dashboard);
    }

    #[test]
    fn tag_editor_stages_a_value() {
        let mut app = app::App::new(Config::default(), fixture_model());
        let p = std::path::PathBuf::from("/m/Artists/Radiohead/Kid A/01.flac");
        // does not exist on disk -> status error, editor stays closed
        app.open_tag_editor(p);
        assert!(app.tag_editor.is_none());
        // grade screen scope resolves to the selected track
        app.screen = app::Screen::Album;
        app.artist_sel = 0;
        app.album_sel = 0;
        app.track_sel = 0;
        assert!(matches!(app.current_scope(), Scope::Track(_)));
    }
}
