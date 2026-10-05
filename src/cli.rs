//! CLI surface ([§0]) — the same engine as the TUI, scriptable.

use crate::config::Config;
use crate::db::Db;
use crate::error::{MloError, Result};
use crate::model::{Scope, TagMap};
use crate::tui::app::ToolRow;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "mlo",
    version,
    about = "mlo — native music library manager (TUI + CLI)",
    long_about = "Owns a music library end to end: acquire, identify, tag, organize, analyse, grade, optimize, play."
)]
pub struct Cli {
    /// Force the TUI even when stdout is not a terminal.
    #[arg(long, global = true)]
    pub tui: bool,
    /// Force the CLI even on a terminal.
    #[arg(long, global = true)]
    pub cli: bool,
    /// Config file (default: platform config dir / mlo.toml).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Open the TUI focused on a folder/album/file (used by the shell menu).
    #[arg(long, global = true)]
    pub open: Option<PathBuf>,
    /// Run a music folder for this invocation (overrides the config).
    #[arg(long, global = true)]
    pub music: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Scan, index, audit layout and grade.
    Scan {
        #[arg(long)]
        artist: Option<PathBuf>,
        #[arg(long)]
        album: Option<PathBuf>,
    },
    /// Print the layout report; `--apply` performs the fixes.
    Layout {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// Print the grade report.
    Grade {
        #[arg(long)]
        album: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Show or edit the tags of one file.
    Tags {
        path: PathBuf,
        #[arg(long, value_name = "KEY=VALUE")]
        set: Vec<String>,
        #[arg(long)]
        remove: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run an optimization script over a scope.
    Run {
        #[arg(value_name = "ID|NAME")]
        script: String,
        #[arg(long)]
        artist: Option<PathBuf>,
        #[arg(long)]
        album: Option<PathBuf>,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Import a path (folder, files or archive).
    Import { path: PathBuf },
    /// Trash: list manifests and restore them.
    Trash {
        #[command(subcommand)]
        action: TrashAction,
    },
    /// External tools: detect, install, doctor.
    Tools {
        #[command(subcommand)]
        action: ToolsAction,
    },
    /// Config: show, get, set.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Shell context-menu integration (Windows / macOS / Linux).
    Shell {
        #[command(subcommand)]
        action: ShellAction,
    },
    /// External service health.
    Sources,
    /// Environment and tool diagnostics.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// List the optimization scripts.
    Scripts,
    /// Print the resolved config and paths.
    Paths,
}

#[derive(Subcommand, Debug)]
pub enum TrashAction {
    List,
    Restore {
        /// Manifest stamp (see `mlo trash list`).
        stamp: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ToolsAction {
    Doctor,
    Install { name: String },
    List,
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    Show,
    Get { key: String },
    Set { key: String, value: String },
}

#[derive(Subcommand, Debug)]
pub enum ShellAction {
    Install,
    Uninstall,
    Status,
}

/// Entry point used by `main`.
pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.clone().unwrap_or_else(Config::default_path);
    let mut cfg = Config::load(&config_path)?;
    if let Some(music) = &cli.music {
        cfg.music_folder = music.clone();
    }
    cfg.ensure_dirs()?;

    // Interrupt recovery ([§3.5]): sweep the app's own temp files and report
    // abandoned jobs by name — never guess.
    for dir in [cfg.data_dir(), cfg.music_folder.clone()] {
        let removed = crate::atomic::sweep_temp_files(&dir);
        if !removed.is_empty() {
            tracing::info!(count = removed.len(), dir = %dir.display(), "swept stale temp files");
        }
    }
    if let Ok(db) = Db::open(&cfg.index_db()) {
        for job in db.abandoned_jobs().unwrap_or_default() {
            tracing::warn!(kind = %job.kind, scope = %job.scope, "abandoned job from a previous run");
        }
    }

    let _guard = crate::logging::init(&cfg, false);

    let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let want_tui = cli.command.is_none() && !cli.cli && (cli.tui || is_tty || cli.open.is_some());

    if want_tui {
        return crate::tui::run(cfg, cli.open.clone(), cli.config.clone());
    }

    let Some(cmd) = cli.command else {
        // no subcommand, not a terminal: print help-ish summary
        println!("mlo {} — not a terminal and no command given.", crate::version());
        println!("Run `mlo --help` for commands, or `mlo <command>` on a pipe.");
        return Ok(());
    };

    match cmd {
        Command::Scan { artist, album } => cmd_scan(&cfg, artist, album),
        Command::Layout { apply, dry_run, json } => cmd_layout(&cfg, apply, dry_run, json),
        Command::Grade { album, json } => cmd_grade(&cfg, album, json),
        Command::Tags { path, set, remove, json } => cmd_tags(&cfg, path, set, remove, json),
        Command::Run { script, artist, album, path } => cmd_run(&cfg, script, artist, album, path),
        Command::Import { path } => cmd_import(&cfg, path),
        Command::Trash { action } => cmd_trash(&cfg, action),
        Command::Tools { action } => cmd_tools(&cfg, action),
        Command::Config { action } => cmd_config(&cfg, &config_path, action),
        Command::Shell { action } => cmd_shell(action),
        Command::Sources => cmd_sources(&cfg),
        Command::Doctor { json } => cmd_doctor(&cfg, json),
        Command::Scripts => {
            for s in crate::scripts::SCRIPTS {
                let switch = s.switch.map(|x| format!("  [{x}]")).unwrap_or_default();
                println!("{:>2}  {:<24} {}{}", s.id, s.name, s.description, switch);
            }
            Ok(())
        }
        Command::Paths => {
            println!("config          {}", config_path.display());
            println!("music_folder    {}", cfg.music_folder.display());
            println!("state_dir       {}", cfg.state_dir().display());
            println!("index_db        {}", cfg.index_db().display());
            println!("layout_report   {}", cfg.layout_report().display());
            println!("tools_dir       {}", cfg.tools_dir().display());
            println!("trash_dir       {}", cfg.trash_dir().display());
            println!("logs_dir        {}", cfg.logs_dir().display());
            Ok(())
        }
    }
}

fn scope_from(artist: Option<PathBuf>, album: Option<PathBuf>) -> Scope {
    if let Some(a) = album {
        Scope::Album(a)
    } else if let Some(a) = artist {
        Scope::Artist(a)
    } else {
        Scope::Library
    }
}

fn cmd_scan(cfg: &Config, artist: Option<PathBuf>, album: Option<PathBuf>) -> Result<()> {
    let db = Db::open(&cfg.index_db())?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let progress = |p: crate::model::Progress| {
        if let Some(t) = p.total {
            eprint!("\r{} {}/{}   ", p.label, p.current, t);
        }
    };
    let model = crate::scan::scan(cfg, &db, &scope_from(artist, album), &cancel, &progress)?;
    eprintln!();
    println!("scanned {} artists, {} albums, {} tracks", model.artists.len(), model.album_count(), model.track_count());
    println!("findings: {}", model.all_findings().len());
    print_verdict(&model.verdict);
    Ok(())
}

fn print_verdict(v: &crate::model::LibraryVerdict) {
    println!(
        "grade: {}/{} albums pass ({:.1}%), {}/{} artists pass{}",
        v.albums_passed,
        v.albums_total,
        v.albums_pct(),
        v.artists_passed,
        v.artists_total,
        if v.albums_audit_failed > 0 { format!(", {} audit-failed", v.albums_audit_failed) } else { String::new() }
    );
    if let Some(row) = &v.library_row {
        println!("library row: {}/{} checks pass", row.pass_count(), row.total_checks);
        for (c, i) in row.issues() {
            println!("  FAIL [{}] {} — {}", crate::grade::display_key(&c.key), i.code, i.message);
        }
    }
}

fn cmd_layout(cfg: &Config, apply: bool, dry_run: bool, json: bool) -> Result<()> {
    // ONE scan answers the CLI, the TUI panel and the stored report ([§4.2]).
    let db = Db::open(&cfg.index_db())?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let progress = |p: crate::model::Progress| {
        if let Some(t) = p.total {
            eprint!("\r{} {}/{}   ", p.label, p.current, t);
        }
    };
    let model = crate::scan::scan(cfg, &db, &Scope::Library, &cancel, &progress)?;
    eprintln!();
    let findings: Vec<crate::model::Finding> = model.all_findings().into_iter().cloned().collect();
    if json {
        let s = serde_json::to_string_pretty(&findings).unwrap_or_default();
        println!("{s}");
        return Ok(());
    }
    for (kind, count, fixable) in crate::layout::group_by_kind(&findings) {
        println!("{:>3}  {:<24} {} fixable   {}", count, kind.code(), fixable, kind.label());
    }
    println!("total: {}", findings.len());
    if apply && !dry_run {
        let outcomes = crate::layout::apply(cfg, &findings, None, false)?;
        for o in &outcomes {
            println!("{} {}: {}", if o.ok { "ok  " } else { "FAIL" }, o.action, o.detail);
        }
    } else if apply {
        for o in crate::layout::apply(cfg, &findings, None, true)? {
            println!("would {} : {}", o.action, o.detail);
        }
    }
    Ok(())
}

fn cmd_grade(cfg: &Config, album: Option<PathBuf>, json: bool) -> Result<()> {
    let db = Db::open(&cfg.index_db())?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let progress = |_: crate::model::Progress| {};
    let scope = album.map(Scope::Album).unwrap_or(Scope::Library);
    let model = crate::scan::scan(cfg, &db, &scope, &cancel, &progress)?;
    if json {
        let out: Vec<_> = model
            .albums_flat()
            .iter()
            .map(|a| {
                serde_json::json!({
                    "path": a.path,
                    "title": a.title(),
                    "pass": a.grade.as_ref().map(|g| g.passed()),
                    "pct": a.pct(),
                    "checks": a.grade.as_ref().map(|g| g.total_checks),
                    "failed": a.grade.as_ref().map(|g| g.failed_checks),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return Ok(());
    }
    for a in model.albums_flat() {
        let Some(g) = &a.grade else { continue };
        println!(
            "{} {}  {}/{}  {:.1}%",
            if g.passed() { "PASS" } else { "FAIL" },
            a.path.display(),
            g.pass_count(),
            g.total_checks,
            g.pct()
        );
        for (c, i) in g.issues() {
            println!("      [{}] {} — {}", crate::grade::display_key(&c.key), i.code, i.message);
        }
    }
    print_verdict(&model.verdict);
    Ok(())
}

fn cmd_tags(cfg: &Config, path: PathBuf, set: Vec<String>, remove: Vec<String>, json: bool) -> Result<()> {
    let _ = cfg;
    if !set.is_empty() || !remove.is_empty() {
        for s in &set {
            let (k, v) = s
                .split_once('=')
                .ok_or_else(|| MloError::Invalid(format!("expected KEY=VALUE, got '{s}'")))?;
            let existing = crate::tags::read_tags(&path)?
                .get(&k.to_ascii_uppercase())
                .cloned()
                .unwrap_or_default();
            let mut values = existing;
            let v = v.to_string();
            if !values.iter().any(|x| x == &v) {
                values.push(v);
            }
            crate::tags::set_values(&path, k, &values)?;
        }
        for k in &remove {
            crate::tags::remove_key(&path, k)?;
        }
    }
    let tags = crate::tags::read_tags(&path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&tags).unwrap_or_default());
    } else {
        println!("{}  ({})", path.display(), crate::tags::detect_container(&path).as_str());
        for (k, vs) in &tags {
            for v in vs {
                println!("{k} = {v}");
            }
        }
    }
    Ok(())
}

fn cmd_run(cfg: &Config, script: String, artist: Option<PathBuf>, album: Option<PathBuf>, path: Option<PathBuf>) -> Result<()> {
    let def = crate::scripts::by_id(script.parse().unwrap_or(0))
        .or_else(|| crate::scripts::by_name(&script))
        .ok_or_else(|| MloError::Invalid(format!("no script matches '{script}'")))?;
    let scope = if let Some(p) = path {
        if p.is_dir() {
            Scope::Album(p)
        } else {
            Scope::Track(p)
        }
    } else {
        scope_from(artist, album)
    };
    let db = Db::open(&cfg.index_db())?;
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let progress = |p: crate::model::Progress| {
        if let Some(t) = p.total {
            eprint!("\r{} {}/{}   ", p.label, p.current, t);
        }
    };
    let ctx = crate::scripts::ScriptCtx { cfg, db: &db, cancel: &cancel, progress: &progress };
    let out = crate::scripts::run(def.id, &scope, &ctx)?;
    eprintln!();
    for r in &out.results {
        println!("{} {} {}", r.outcome.label(), r.path.display(), r.note);
    }
    for n in &out.notes {
        println!("note: {n}");
    }
    println!("{}", out.summary());
    Ok(())
}

fn cmd_import(cfg: &Config, path: PathBuf) -> Result<()> {
    crate::import::import_path(cfg, &path)
}

fn cmd_trash(cfg: &Config, action: TrashAction) -> Result<()> {
    let trash = crate::trash::Trash::new(&cfg.music_folder);
    match action {
        TrashAction::List => {
            for m in trash.list()? {
                println!("{}  {}  {} entries  {}", m.stamp, m.user, m.entries, m.path.display());
            }
        }
        TrashAction::Restore { stamp } => {
            let manifests = trash.list()?;
            let target = match stamp {
                Some(s) => manifests
                    .into_iter()
                    .find(|m| m.stamp == s)
                    .ok_or_else(|| MloError::NotFound(format!("no trash manifest '{s}'")))?,
                None => manifests
                    .into_iter()
                    .next()
                    .ok_or_else(|| MloError::NotFound("trash is empty".into()))?,
            };
            let restored = trash.restore(&target.path)?;
            for p in &restored {
                println!("restored {}", p.display());
            }
            println!("{} item(s) restored", restored.len());
        }
    }
    Ok(())
}

/// The tools doctor table ([§10.2.4]).
pub fn tools_doctor_rows(cfg: &Config) -> Vec<ToolRow> {
    let expected: &[(&str, &str, &str)] = &[
        ("flac", "1.4+", "download"),
        ("libjxl", "0.10+", "system package"),
        ("oxipng", "9+", "download"),
        ("ffmpeg", "6+", "system package"),
        ("fpcalc", "1.5+", "download"),
        ("yt-dlp", "2024+", "download"),
        ("rsgain", "3+", "system package"),
        ("AudioAuditor", "2+", "unsupported here"),
        ("Logchecker", "0.9+", "system package"),
        ("CUETools", "2+", "unsupported here"),
    ];
    let mut rows = Vec::new();
    for (name, want, kind) in expected {
        let found = find_executable(name);
        rows.push(ToolRow {
            name: (*name).to_string(),
            expected: (*want).to_string(),
            found: found
                .as_ref()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "not found".into()),
            kind: (*kind).to_string(),
            path: found.map(|p| p.display().to_string()).unwrap_or_default(),
            last_failure: String::new(),
        });
    }
    // Native rows (implemented in-process, listed for compatibility).
    let _ = cfg;
    rows
}

pub fn find_executable(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|s| s.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        if cfg!(windows) {
            for ext in &exts {
                let cand = dir.join(format!("{name}{ext}"));
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn cmd_tools(cfg: &Config, action: ToolsAction) -> Result<()> {
    match action {
        ToolsAction::Doctor | ToolsAction::List => {
            println!("{:<16} {:<10} {:<16} {:<16} {}", "tool", "expected", "found", "install", "path");
            for r in tools_doctor_rows(cfg) {
                println!("{:<16} {:<10} {:<16} {:<16} {}", r.name, r.expected, r.found, r.kind, r.path);
                if !r.last_failure.is_empty() {
                    println!("    last failure: {}", r.last_failure);
                }
            }
            Ok(())
        }
        ToolsAction::Install { name } => Err(MloError::tool(
            name,
            "automatic installation is not available in this build; install the tool and re-run `mlo tools doctor`",
        )),
    }
}

fn cmd_config(cfg: &Config, path: &std::path::Path, action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Show => {
            let text = toml::to_string_pretty(cfg).map_err(|e| MloError::Config(e.to_string()))?;
            println!("{text}");
            println!("# file: {}", path.display());
        }
        ConfigAction::Get { key } => {
            let text = toml::to_string_pretty(cfg).map_err(|e| MloError::Config(e.to_string()))?;
            let value: toml::Value = toml::from_str(&text).map_err(|e| MloError::Config(e.to_string()))?;
            let v = value
                .get(&key)
                .ok_or_else(|| MloError::NotFound(format!("no config key '{key}'")))?;
            println!("{v}");
        }
        ConfigAction::Set { key, value } => {
            let mut text = toml::to_string_pretty(cfg).map_err(|e| MloError::Config(e.to_string()))?;
            let mut doc: toml::Value = toml::from_str(&text).map_err(|e| MloError::Config(e.to_string()))?;
            let new_val: toml::Value = if let Ok(b) = value.parse::<bool>() {
                toml::Value::Boolean(b)
            } else if let Ok(i) = value.parse::<i64>() {
                toml::Value::Integer(i)
            } else {
                toml::Value::String(value.clone())
            };
            let table = doc.as_table_mut().ok_or_else(|| MloError::Config("config root is not a table".into()))?;
            table.insert(key.clone(), new_val);
            text = toml::to_string_pretty(&doc).map_err(|e| MloError::Config(e.to_string()))?;
            crate::atomic::write_atomic_str(path, &text)?;
            println!("set {key} = {value}");
        }
    }
    Ok(())
}

fn cmd_shell(action: ShellAction) -> Result<()> {
    let exe = std::env::current_exe().map_err(|e| MloError::Other(format!("cannot find own path: {e}")))?;
    match action {
        ShellAction::Status => {
            for item in crate::shellmenu::status(&exe) {
                println!("[{}] {} — {} ({})", if item.installed { "x" } else { " " }, item.name, item.detail, item.os);
            }
            Ok(())
        }
        ShellAction::Install => {
            for line in crate::shellmenu::install(&exe)? {
                println!("{line}");
            }
            Ok(())
        }
        ShellAction::Uninstall => {
            for line in crate::shellmenu::uninstall()? {
                println!("{line}");
            }
            Ok(())
        }
    }
}

fn cmd_sources(cfg: &Config) -> Result<()> {
    println!("MusicBrainz user-agent: {}", cfg.services.musicbrainz_user_agent);
    let ok = crate::net::check_connectivity();
    println!("MusicBrainz: {}", if ok { "reachable" } else { "unavailable (offline is a normal state)" });
    println!("AcoustID key: {}", if cfg.services.acoustid_api_key.is_empty() { "not configured" } else { "configured" });
    println!("Discogs token: {}", if cfg.services.discogs_token.is_empty() { "not configured" } else { "configured" });
    Ok(())
}

fn cmd_doctor(cfg: &Config, json: bool) -> Result<()> {
    let tools = tools_doctor_rows(cfg);
    let db_ok = Db::open(&cfg.index_db()).map(|d| d.file_count().unwrap_or(0)).ok();
    let containers = ["flac", "ogg", "opus", "mp3", "m4a", "wav", "aiff", "mkv", "mp4"];
    let info = serde_json::json!({
        "version": crate::version(),
        "config": Config::default_path(),
        "music_folder": cfg.music_folder,
        "music_exists": cfg.music_folder.exists(),
        "index_db": cfg.index_db(),
        "indexed_files": db_ok,
        "containers": containers,
        "full_fidelity_containers": ["flac", "ogg", "opus", "mp3"],
        "features": {
            "audio": cfg!(feature = "audio"),
            "archives": cfg!(feature = "archives"),
        },
        "tools": tools.iter().map(|t| serde_json::json!({
            "name": t.name, "expected": t.expected, "found": t.found, "install": t.kind, "path": t.path,
        })).collect::<Vec<_>>(),
        "services": { "musicbrainz": crate::net::check_connectivity() },
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&info).unwrap_or_default());
        return Ok(());
    }
    println!("mlo {} (audio={}, archives={})", crate::version(), cfg!(feature = "audio"), cfg!(feature = "archives"));
    println!("music folder : {} ({})", cfg.music_folder.display(), if cfg.music_folder.exists() { "ok" } else { "MISSING" });
    println!("index db     : {} ({} files)", cfg.index_db().display(), db_ok.unwrap_or(0));
    println!("tag fidelity : flac, ogg, opus, mp3 (full key preservation); m4a/wav/aiff (shared fields)");
    println!("network      : {}", if crate::net::check_connectivity() { "reachable" } else { "offline" });
    println!("\ntools:");
    for t in &tools {
        println!("  {:<14} {:<16} {}", t.name, t.found, if t.path.is_empty() { t.kind.clone() } else { t.path.clone() });
    }
    Ok(())
}

/// Convert a map to a `Vec` for display purposes.
pub fn tag_rows(tags: &TagMap) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = tags
        .iter()
        .flat_map(|(k, vs)| vs.iter().map(move |val| (k.clone(), val.clone())))
        .collect();
    v.sort();
    v
}