//! TUI rendering ([§13]) — every screen, plus overlays for help, confirmation
//! and the tag editor.

use super::app::{App, Screen, StatusKind, TagMode};
use crate::grade::CHECK_DEFS;
use crate::model::{CheckStatus, FindingKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Gauge, List, ListItem, Paragraph, Row, Table, Tabs, Wrap,
};
use ratatui::Frame;

const PASS: Color = Color::Green;
const FAIL: Color = Color::Red;
const SKIP: Color = Color::DarkGray;
const WARN: Color = Color::Yellow;
const ACCENT: Color = Color::Cyan;

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tabs
            Constraint::Length(2), // status summary + progress
            Constraint::Min(5),    // body
            Constraint::Length(1), // footer keys
        ])
        .split(area);

    render_tabs(f, app, chunks[0]);
    render_status_header(f, app, chunks[1]);
    render_body(f, app, chunks[2]);
    render_footer(f, app, chunks[3]);

    if app.help {
        render_help(f, area);
    }
    if app.confirm.is_some() {
        render_confirm(f, app, area);
    }
    if app.tag_editor.is_some() {
        render_tag_editor(f, app, area);
    }
    if app.input_mode {
        render_input(f, app, area);
    }
}

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = Screen::TABS
        .iter()
        .map(|s| Line::from(format!(" {} ", s.title())))
        .collect();
    let idx = Screen::TABS.iter().position(|s| *s == app.screen).unwrap_or(0);
    let tabs = Tabs::new(titles)
        .select(idx)
        .highlight_style(Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD))
        .divider(Span::raw("│"));
    f.render_widget(tabs, area);
}

fn render_status_header(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let v = &app.model.verdict;
    let dot = |pass: bool| if pass { Span::styled("●", Style::default().fg(PASS)) } else { Span::styled("●", Style::default().fg(FAIL)) };
    let mut line = vec![
        Span::styled("mlo", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        dot(v.overall_pass()),
        Span::raw(format!(" {} albums  ", v.albums_total)),
        Span::raw(format!("{} PASS / {} FAIL  ", v.albums_passed, v.albums_failed)),
        Span::raw(format!("{} artists  ", v.artists_total)),
        Span::raw(format!("{} tracks  ", v.tracks_total)),
    ];
    if v.albums_audit_failed > 0 {
        line.push(Span::styled(format!("{} audit-failed  ", v.albums_audit_failed), Style::default().fg(WARN)));
    }
    let findings = app.model.all_findings().len();
    if findings > 0 {
        line.push(Span::styled(format!("{findings} layout findings"), Style::default().fg(WARN)));
    }
    f.render_widget(Paragraph::new(Line::from(line)), chunks[0]);

    // progress / status line
    if let Some(p) = &app.progress {
        let ratio = p.fraction();
        let label = match p.total {
            Some(t) => format!("{}  {}/{}", p.label, p.current, t),
            None => p.label.clone(),
        };
        f.render_widget(Gauge::default().ratio(ratio).label(label).gauge_style(Style::default().fg(ACCENT)), chunks[1]);
    } else {
        let style = match app.status.kind {
            StatusKind::Info => Style::default().fg(Color::Gray),
            StatusKind::Working => Style::default().fg(WARN),
            StatusKind::Error => Style::default().fg(FAIL).add_modifier(Modifier::BOLD),
        };
        f.render_widget(Paragraph::new(Span::styled(app.status.message.clone(), style)), chunks[1]);
    }
}

fn render_body(f: &mut Frame, app: &mut App, area: Rect) {
    match app.screen {
        Screen::Dashboard => render_dashboard(f, app, area),
        Screen::Library => render_library(f, app, area),
        Screen::Album => render_album(f, app, area),
        Screen::Artist => render_artist(f, app, area),
        Screen::Grade => render_grade(f, app, area),
        Screen::Layout => render_layout(f, app, area),
        Screen::Scripts => render_scripts(f, app, area),
        Screen::Import => render_import(f, app, area),
        Screen::Tools => render_tools(f, app, area),
        Screen::Player => render_player(f, app, area),
        Screen::Log => render_log(f, app, area),
        Screen::Settings => render_settings(f, app, area),
        Screen::Trash => render_trash(f, app, area),
    }
}

fn panel(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).title(format!(" {title} ")).border_style(Style::default().fg(Color::DarkGray))
}

fn focused_panel(title: &str, focused: bool) -> Block<'_> {
    let b = panel(title);
    if focused {
        b.border_style(Style::default().fg(ACCENT))
    } else {
        b
    }
}

fn grade_dot(pass: bool) -> Span<'static> {
    if pass {
        Span::styled(" ● ", Style::default().fg(PASS))
    } else {
        Span::styled(" ● ", Style::default().fg(FAIL))
    }
}

fn render_library(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(30), Constraint::Percentage(40)])
        .split(area);

    // artists
    let items: Vec<ListItem> = app
        .model
        .artists
        .iter()
        .map(|a| {
            let dot = if a.passed() { Span::styled("● ", Style::default().fg(PASS)) } else { Span::styled("● ", Style::default().fg(FAIL)) };
            ListItem::new(Line::from(vec![dot, Span::raw(format!("{} ({} alb)", a.name, a.albums.len()))]))
        })
        .collect();
    let list = List::new(items)
        .block(focused_panel("Artists", app.pane == 0))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    app.artist_state.select(Some(app.artist_sel));
    f.render_stateful_widget(list, cols[0], &mut app.artist_state);

    // albums
    let items: Vec<ListItem> = app
        .current_artist()
        .map(|a| {
            a.albums
                .iter()
                .map(|al| {
                    let dot = if al.passed() { Span::styled("● ", Style::default().fg(PASS)) } else { Span::styled("● ", Style::default().fg(FAIL)) };
                    ListItem::new(Line::from(vec![dot, Span::raw(format!("{} ({})", al.title(), al.tracks.len()))]))
                })
                .collect()
        })
        .unwrap_or_default();
    let list = List::new(items)
        .block(focused_panel("Albums", app.pane == 1))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    app.album_state.select(Some(app.album_sel));
    f.render_stateful_widget(list, cols[1], &mut app.album_state);

    render_track_table(f, app, cols[2], true);
}

fn render_track_table(f: &mut Frame, app: &mut App, area: Rect, titled: bool) {
    let rows: Vec<Row> = app
        .current_album()
        .map(|al| {
            al.tracks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let pass = t.grade.as_ref().map(|g| g.passed()).unwrap_or(true);
                    let pct = t.grade.as_ref().map(|g| format!("{:.0}%", g.pct())).unwrap_or_default();
                    let num = t.track_no().map(|n| n.to_string()).unwrap_or_else(|| (i + 1).to_string());
                    Row::new(vec![
                        Cell::from(grade_dot(pass)),
                        Cell::from(num),
                        Cell::from(t.title()),
                        Cell::from(t.container.as_str().to_string()),
                        Cell::from(pct),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let title = app.current_album().map(|a| a.title()).unwrap_or_else(|| "Tracks".into());
    let header = Row::new(vec!["", "#", "Title", "Fmt", "%"]).style(Style::default().add_modifier(Modifier::BOLD));
    let table = Table::new(
        rows,
        [Constraint::Length(3), Constraint::Length(4), Constraint::Min(20), Constraint::Length(5), Constraint::Length(5)],
    )
    .header(header)
    .block(if titled { panel(&title) } else { panel("Tracks") })
    .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    app.track_state.select(Some(app.track_sel));
    f.render_stateful_widget(table, area, &mut app.track_state);
}

fn render_album(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);
    render_track_table(f, app, cols[0], true);

    // tag panel for the selected track
    let lines = app
        .current_track()
        .map(|t| tag_lines(&t.tags))
        .unwrap_or_else(|| vec![Line::from("no track selected")]);
    let block = focused_panel("Tags (e edit · a add · d delete)", true);
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, cols[1]);
}

fn render_artist(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let rows: Vec<Row> = app
        .current_artist()
        .map(|a| {
            a.albums
                .iter()
                .map(|al| {
                    let pass = al.passed();
                    Row::new(vec![
                        Cell::from(grade_dot(pass)),
                        Cell::from(al.title()),
                        Cell::from(al.tracks.len().to_string()),
                        Cell::from(format!("{:.0}%", al.pct())),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    let table = Table::new(rows, [Constraint::Length(3), Constraint::Min(20), Constraint::Length(4), Constraint::Length(5)])
        .header(Row::new(vec!["", "Album", "Trk", "%"]).style(Style::default().add_modifier(Modifier::BOLD)))
        .block(panel("Albums"))
        .row_highlight_style(Style::default().bg(Color::DarkGray));
    app.table_state.select(Some(app.album_sel));
    f.render_stateful_widget(table, cols[0], &mut app.table_state);

    let info = app
        .current_artist()
        .map(|a| {
            let mut v = vec![
                Line::from(vec![Span::styled("Artist: ", Style::default().fg(Color::DarkGray)), Span::raw(a.name.clone())]),
                Line::from(format!("Image: {}", if a.has_image { "present" } else { "MISSING" })),
                Line::from(format!("Description: {}", if a.has_description { "present" } else { "MISSING" })),
                Line::from(format!("Albums: {}", a.albums.len())),
            ];
            if let Some(g) = &a.grade {
                v.push(Line::from(format!("Grade: {}/{} checks, {:.0}%", g.pass_count(), g.total_checks, g.pct())));
                for (c, i) in g.issues() {
                    v.push(Line::from(Span::styled(format!("  [{}] {}", crate::grade::display_key(&c.key), i.code), Style::default().fg(FAIL))));
                }
            }
            v
        })
        .unwrap_or_else(|| vec![Line::from("no artist")]);
    f.render_widget(Paragraph::new(info).block(panel("Artist")).wrap(Wrap { trim: true }), cols[1]);
}

fn tag_lines(tags: &crate::model::TagMap) -> Vec<Line<'static>> {
    if tags.is_empty() {
        return vec![Line::from("(no tags)")];
    }
    let mut out = Vec::new();
    for (k, vs) in tags {
        let family = crate::tagkey::family_of(k);
        let color = match family {
            crate::tagkey::TagFamily::Identity => Color::Cyan,
            crate::tagkey::TagFamily::Release => Color::Blue,
            crate::tagkey::TagFamily::Audio => Color::Magenta,
            crate::tagkey::TagFamily::Lyrics => Color::Green,
            crate::tagkey::TagFamily::Provenance => Color::Yellow,
            crate::tagkey::TagFamily::Foreign => Color::Red,
        };
        for v in vs {
            out.push(Line::from(vec![
                Span::styled(format!("{k:<24} "), Style::default().fg(color)),
                Span::raw(v.clone()),
            ]));
        }
    }
    out
}

fn render_dashboard(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let v = &app.model.verdict;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled("Library verdict", Style::default().add_modifier(Modifier::BOLD))));
    lines.push(Line::from(""));
    lines.push(Line::from(format!("Albums      {} total · {} pass · {} fail", v.albums_total, v.albums_passed, v.albums_failed)));
    lines.push(Line::from(format!("Artists     {} total · {} pass · {} fail", v.artists_total, v.artists_passed, v.artists_failed)));
    lines.push(Line::from(format!("Tracks      {}", v.tracks_total)));
    if v.albums_audit_failed > 0 {
        lines.push(Line::from(Span::styled(format!("Audit fail  {}", v.albums_audit_failed), Style::default().fg(WARN))));
    }
    lines.push(Line::from(""));
    if let Some(row) = &v.library_row {
        lines.push(Line::from(format!("Library row {}/{} checks pass", row.pass_count(), row.total_checks)));
        for (c, i) in row.issues() {
            lines.push(Line::from(Span::styled(format!("  FAIL [{}] {} — {}", crate::grade::display_key(&c.key), i.code, i.message), Style::default().fg(FAIL))));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Worst albums", Style::default().add_modifier(Modifier::BOLD))));
    let mut albums: Vec<_> = app.model.albums_flat();
    albums.sort_by(|a, b| a.pct().partial_cmp(&b.pct()).unwrap_or(std::cmp::Ordering::Equal));
    for a in albums.iter().take(8) {
        lines.push(Line::from(vec![
            grade_dot(a.passed()),
            Span::raw(format!("{:.0}%  {}", a.pct(), a.path.display())),
        ]));
    }
    f.render_widget(Paragraph::new(lines).block(panel("Library status")).wrap(Wrap { trim: true }), cols[0]);

    // findings by kind
    let owned: Vec<crate::model::Finding> = app.model.all_findings().into_iter().cloned().collect();
    let counts = crate::layout::group_by_kind(&owned);
    let mut f_lines: Vec<Line> = Vec::new();
    if counts.is_empty() {
        f_lines.push(Line::from(Span::styled("no layout findings", Style::default().fg(PASS))));
    } else {
        for (kind, n, fixable) in counts {
            f_lines.push(Line::from(vec![
                Span::styled(format!("{n:>4}  "), Style::default().fg(WARN)),
                Span::raw(format!("{:<24} ", kind.code())),
                Span::styled(format!("{fixable} fixable"), Style::default().fg(Color::DarkGray)),
            ]));
        }
    }
    f.render_widget(Paragraph::new(f_lines).block(panel("Layout findings")).wrap(Wrap { trim: true }), cols[1]);
}

fn render_grade(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // albums list with grade
    let rows: Vec<Row> = app
        .model
        .albums_flat()
        .iter()
        .map(|a| {
            Row::new(vec![
                Cell::from(grade_dot(a.passed())),
                Cell::from(a.path.display().to_string()),
                Cell::from(format!("{:.0}%", a.pct())),
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(3), Constraint::Min(20), Constraint::Length(5)])
        .header(Row::new(vec!["", "Album", "%"]).style(Style::default().add_modifier(Modifier::BOLD)))
        .block(panel("Albums · grade"))
        .row_highlight_style(Style::default().bg(Color::DarkGray));
    app.table_state.select(Some(app.album_sel.min(app.model.albums_flat().len().saturating_sub(1))));
    f.render_stateful_widget(table, cols[0], &mut app.table_state);

    // checks of the selected track (per-row detail [§13])
    let checks = app
        .current_track()
        .and_then(|t| t.grade.as_ref().map(|g| (t.title(), g.clone())))
        .or_else(|| app.current_album().and_then(|a| a.grade.as_ref().map(|g| (a.title(), g.clone()))));
    let lines = match checks {
        Some((title, g)) => {
            let mut v = vec![
                Line::from(vec![
                    Span::styled(if g.passed() { "PASS " } else { "FAIL " }, Style::default().fg(if g.passed() { PASS } else { FAIL }).add_modifier(Modifier::BOLD)),
                    Span::raw(format!("{title}  {}/{} checks ({:.1}%)", g.pass_count(), g.total_checks, g.pct())),
                ]),
                Line::from(""),
            ];
            for c in &g.checks {
                let key = crate::grade::display_key(&c.key);
                match &c.status {
                    CheckStatus::Pass => v.push(Line::from(vec![
                        Span::styled("✔ ", Style::default().fg(PASS)),
                        Span::raw(format!("{:<34} {}", key, c.label)),
                    ])),
                    CheckStatus::Fail(i) => v.push(Line::from(vec![
                        Span::styled("✘ ", Style::default().fg(FAIL)),
                        Span::raw(format!("{key:<34} ")),
                        Span::styled(format!("{} — {}", i.code, i.message), Style::default().fg(FAIL)),
                    ])),
                    CheckStatus::Skipped => v.push(Line::from(Span::styled(format!("–  {key:<34} {}", c.label), Style::default().fg(SKIP)))),
                    CheckStatus::CouldNotEvaluate(m) => v.push(Line::from(Span::styled(format!("!  {key:<34} could not be evaluated: {m}"), Style::default().fg(WARN)))),
                }
            }
            v
        }
        None => vec![Line::from("no grade available — press g to grade the library")],
    };
    f.render_widget(Paragraph::new(lines).block(panel("Checks")).wrap(Wrap { trim: false }), cols[1]);
}

fn render_layout(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    let rows: Vec<Row> = app
        .layout_findings
        .iter()
        .map(|f| {
            let selected = app.layout_selected.contains(&f.id);
            Row::new(vec![
                Cell::from(if selected { "[x]" } else { "[ ]" }),
                Cell::from(f.kind.code().to_string()),
                Cell::from(f.path.display().to_string()),
                Cell::from(if f.fixable() { f.fix.label() } else { "— report only".into() }),
            ])
        })
        .collect();
    let mode = if app.layout_dry_run { "dry-run" } else { "APPLY" };
    let title = format!("Layout findings — {mode} (space toggle · A apply · r dry-run)");
    let table = Table::new(
        rows,
        [Constraint::Length(4), Constraint::Length(22), Constraint::Min(20), Constraint::Length(24)],
    )
    .header(Row::new(vec!["sel", "kind", "path", "fix"]).style(Style::default().add_modifier(Modifier::BOLD)))
    .block(panel(&title))
    .row_highlight_style(Style::default().bg(Color::DarkGray));
    app.layout_state.select(Some(app.layout_state.selected().unwrap_or(0).min(app.layout_findings.len().saturating_sub(1))));
    f.render_stateful_widget(table, cols[0], &mut app.layout_state);

    let counts = crate::layout::group_by_kind(&app.layout_findings);
    let mut lines = vec![Line::from(Span::styled(
        format!("{} findings", app.layout_findings.len()),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for (kind, n, fixable) in counts {
        lines.push(Line::from(format!("{n:>4}  {:<24} {} fixable", kind.code(), fixable)));
    }
    if app.layout_dry_run {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("dry-run: nothing will move", Style::default().fg(WARN))));
    }
    f.render_widget(Paragraph::new(lines).block(panel("Summary")).wrap(Wrap { trim: true }), cols[1]);
}

fn render_scripts(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let items: Vec<ListItem> = crate::scripts::SCRIPTS
        .iter()
        .map(|s| {
            let switch = match s.switch {
                Some(sw) => {
                    let on = crate::grade::default_check_enabled(sw) && switch_is_on(sw, app);
                    Span::styled(format!("  [{sw}: {}]", if on { "on" } else { "off" }), Style::default().fg(if on { PASS } else { SKIP }))
                }
                None => Span::raw(""),
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{:>2}  {:<24} ", s.id, s.name)),
                Span::styled(s.description.to_string(), Style::default().fg(Color::DarkGray)),
                switch,
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(panel("Scripts (Enter runs on the current scope)"))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    app.script_state.select(Some(app.script_sel));
    f.render_stateful_widget(list, cols[0], &mut app.script_state);

    let mut lines = vec![Line::from(format!("scope: {}", app.current_scope().label()))];
    if let Some(out) = &app.last_script {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(out.summary(), Style::default().add_modifier(Modifier::BOLD))));
        for r in out.results.iter().take(200) {
            let style = match r.outcome {
                crate::scripts::Outcome::Ok => Style::default().fg(PASS),
                crate::scripts::Outcome::Skipped => Style::default().fg(SKIP),
                crate::scripts::Outcome::Failed => Style::default().fg(FAIL),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", r.outcome.label()), style),
                Span::raw(crate::model::file_name(&r.path)),
                Span::styled(if r.note.is_empty() { String::new() } else { format!("  — {}", r.note) }, Style::default().fg(Color::DarkGray)),
            ]));
        }
        for n in &out.notes {
            lines.push(Line::from(Span::styled(format!("note: {n}"), Style::default().fg(WARN))));
        }
    } else {
        lines.push(Line::from(""));
        lines.push(Line::from("Select a script and press Enter. Scripts that need a"));
        lines.push(Line::from("network or an external tool report the reason instead of failing."));
    }
    f.render_widget(Paragraph::new(lines).block(panel("Last run")).wrap(Wrap { trim: true }), cols[1]);
}

fn switch_is_on(sw: &str, app: &App) -> bool {
    match sw {
        "dr_replaygain_enabled" => app.cfg.dr_replaygain_enabled,
        "audiometa_enabled" => app.cfg.audiometa_enabled,
        "mood_enabled" => app.cfg.mood_enabled,
        "lyrics_xlit_enabled" => app.cfg.lyrics_xlit_enabled,
        "acoustid_enabled" => app.cfg.acoustid_enabled,
        "fingerprint_submit_enabled" => app.cfg.fingerprint_submit_enabled,
        "strip_unknown_tags" => app.cfg.strip_unknown_tags,
        "web_ratings_enabled" => app.cfg.web_ratings_enabled,
        "video_remux_enabled" => app.cfg.video_remux_enabled,
        "artist_image_enabled" => app.cfg.artist_image_enabled,
        _ => true,
    }
}

fn render_import(f: &mut Frame, app: &mut App, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("Import", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from("Press i to type a path (folder, file or archive), then Enter."),
        Line::from(format!("Path: {}", if app.import_path.is_empty() { "(none)" } else { &app.import_path })),
        Line::from(""),
        Line::from("The CLI runs the full pipeline:"),
        Line::from("  mlo import <path>"),
        Line::from("Steps: acquire → detect shape → identify (MusicBrainz) → write tags"),
        Line::from("→ place by the naming template → run the import chain (layout last,"),
        Line::from("grade last of all) → report."),
        Line::from(""),
        Line::from(Span::styled("Offline is a normal state: identify reports unavailable and the local tags are kept.", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("Import wizard")).wrap(Wrap { trim: true }), area);
}

fn render_tools(f: &mut Frame, app: &mut App, area: Rect) {
    let rows: Vec<Row> = app
        .tools
        .iter()
        .map(|t| {
            let found = t.found != "not found";
            Row::new(vec![
                Cell::from(t.name.clone()),
                Cell::from(t.expected.clone()),
                Cell::from(Span::styled(t.found.clone(), Style::default().fg(if found { PASS } else { SKIP }))),
                Cell::from(t.kind.clone()),
                Cell::from(t.path.clone()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Length(16), Constraint::Length(10), Constraint::Length(12), Constraint::Length(18), Constraint::Min(20)],
    )
    .header(Row::new(vec!["tool", "expected", "found", "install", "path"]).style(Style::default().add_modifier(Modifier::BOLD)))
    .block(panel("Tools — mlo tools doctor (r to refresh)"));
    f.render_widget(table, area);
}

fn render_player(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let now = app
        .now_playing
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(nothing playing)".into());
    let mut lines = vec![
        Line::from(Span::styled(if app.playing { "▶ playing" } else { "⏸ paused / stopped" }, Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(now),
        Line::from(""),
        Line::from("space play/pause · q que/queue album · s stop · +/- volume"),
    ];
    if let Some(p) = &app.player {
        lines.push(Line::from(format!("volume {:.0}%  position {:.0}s", p.volume() * 100.0, p.position_ms() as f64 / 1000.0)));
    }
    f.render_widget(Paragraph::new(lines).block(panel("Now playing")).wrap(Wrap { trim: true }), cols[0]);

    let items: Vec<ListItem> = app
        .queue
        .iter()
        .enumerate()
        .map(|(i, p)| ListItem::new(format!("{:>3}. {}", i + 1, crate::model::file_name(p))))
        .collect();
    f.render_widget(List::new(items).block(panel("Queue (q to queue the album)")), cols[1]);
}

fn render_log(f: &mut Frame, app: &mut App, area: Rect) {
    let visible: Vec<Line> = app.log.iter().map(|l| Line::from(l.clone())).collect();
    let para = Paragraph::new(visible)
        .block(panel("Log (↑/↓ scroll)"))
        .wrap(Wrap { trim: false })
        .scroll((app.log_scroll as u16, 0));
    f.render_widget(para, area);
}

fn render_settings(f: &mut Frame, app: &mut App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();
    for def in CHECK_DEFS {
        let on = app.cfg.check_enabled(def.key);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(if on { " [x] " } else { " [ ] " }, Style::default().fg(if on { PASS } else { SKIP })),
            Span::raw(format!("{:<24} ", def.key)),
            Span::styled(format!("{:<10} ", def.family.label()), Style::default().fg(Color::DarkGray)),
            Span::raw(def.label.to_string()),
        ])));
    }
    for kind in FindingKind::ALL {
        let key = kind.check_key();
        let on = app.cfg.check_enabled(&key);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(if on { " [x] " } else { " [ ] " }, Style::default().fg(if on { PASS } else { SKIP })),
            Span::raw(format!("{key:<24} ")),
            Span::styled("Layout     ", Style::default().fg(Color::DarkGray)),
            Span::raw(kind.label().to_string()),
        ])));
    }
    let list = List::new(items)
        .block(panel("Grading checks (space toggle · s save)"))
        .highlight_style(Style::default().bg(Color::DarkGray));
    let idx = app.settings_sel.min(CHECK_DEFS.len() + FindingKind::ALL.len() - 1);
    app.settings_state.select(Some(idx));
    f.render_stateful_widget(list, area, &mut app.settings_state);
}

fn render_trash(f: &mut Frame, app: &mut App, area: Rect) {
    let rows: Vec<Row> = app
        .trash
        .iter()
        .map(|t| {
            Row::new(vec![
                Cell::from(t.stamp.clone()),
                Cell::from(t.user.clone()),
                Cell::from(t.entries.to_string()),
                Cell::from(t.created_at.clone()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [Constraint::Length(22), Constraint::Length(14), Constraint::Length(8), Constraint::Min(20)],
    )
    .header(Row::new(vec!["stamp", "user", "items", "created"]).style(Style::default().add_modifier(Modifier::BOLD)))
    .block(panel("Trash — Enter restores the selected manifest"))
    .row_highlight_style(Style::default().bg(Color::DarkGray));
    let sel = app.table_state.selected().unwrap_or(0).min(app.trash.len().saturating_sub(1));
    app.table_state.select(Some(sel));
    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let hint = match app.screen {
        Screen::Library => "←→/Tab pane  ↑↓ move  Enter open  e tags  s scan  g grade  L layout  S scripts  T tools  P player  c settings  r trash  ? help  q quit",
        Screen::Layout => "[space] toggle  A apply (asks)  r dry-run  s rescan  ? help  q quit",
        Screen::Scripts => "↑↓ select  Enter run on scope  S run-all order  ? help  q quit",
        Screen::Settings => "[space] toggle  S/B/R presets  s save  ? help  q quit",
        _ => "Tab switch  s scan  g grade  L layout  S scripts  T tools  P player  l log  c settings  r trash  ? help  q quit",
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))),
        area,
    );
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v[1])[1]
}

fn render_help(f: &mut Frame, area: Rect) {
    let rect = centered(area, 70, 76);
    f.render_widget(Clear, rect);
    let text = vec![
        Line::from(Span::styled("mlo — keys", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from("q            quit (asks once)"),
        Line::from("?            this help"),
        Line::from("Esc          cancel the running job / close overlay"),
        Line::from("Tab / ← →    switch screen or pane"),
        Line::from("↑ ↓          move in the focused list"),
        Line::from("Enter        open album / run script / restore trash"),
        Line::from("s            scan and index the library"),
        Line::from("g            grade the library"),
        Line::from("L            layout findings (space toggle, A apply, r dry-run)"),
        Line::from("S            scripts menu"),
        Line::from("T            tools doctor"),
        Line::from("P            player (space play/pause, q queue album)"),
        Line::from("l            log"),
        Line::from("c            settings (grading checks)"),
        Line::from("r            trash (Enter restores exactly)"),
        Line::from("e a d        edit / add / delete a tag on the selected track"),
        Line::from(""),
        Line::from(Span::styled("Nothing is ever deleted: every removal goes to .mlo/trash with a manifest.", Style::default().fg(PASS))),
    ];
    f.render_widget(Paragraph::new(text).block(panel("Help")).wrap(Wrap { trim: false }), rect);
}

fn render_confirm(f: &mut Frame, app: &App, area: Rect) {
    let Some(c) = &app.confirm else { return };
    let rect = centered(area, 60, 20);
    f.render_widget(Clear, rect);
    let text = vec![
        Line::from(c.prompt.clone()),
        Line::from(""),
        Line::from(Span::styled("y confirm · n / Esc cancel", Style::default().fg(WARN))),
    ];
    f.render_widget(Paragraph::new(text).block(panel("Confirm")).wrap(Wrap { trim: true }), rect);
}

fn render_tag_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(editor) = app.tag_editor.as_ref() else { return };
    let rect = centered(area, 76, 80);
    f.render_widget(Clear, rect);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(3), Constraint::Length(1)])
        .split(rect);

    let items: Vec<ListItem> = editor
        .tags
        .iter()
        .map(|(k, vs)| {
            let value = vs.join("; ");
            let family = crate::tagkey::family_of(k);
            let color = match family {
                crate::tagkey::TagFamily::Foreign => FAIL,
                crate::tagkey::TagFamily::Provenance => WARN,
                _ => Color::Gray,
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{k:<26} "), Style::default().fg(color)),
                Span::raw(value),
            ]))
        })
        .collect();
    let title = format!("Tags — {} {} ", editor.title, if editor.dirty { "(unsaved)" } else { "" });
    let list = List::new(items)
        .block(panel(&title))
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    app.artist_state.select(Some(0)); // keep list state alive
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(editor.sel));
    f.render_stateful_widget(list, chunks[0], &mut state);

    let input_line = match &editor.mode {
        TagMode::Browse => Line::from(Span::styled("a add · d delete · Enter edit value · Ctrl-S save · Esc close", Style::default().fg(Color::DarkGray))),
        TagMode::Editing { key, buffer } => Line::from(vec![Span::raw(format!("{key} = ")), Span::styled(buffer.clone(), Style::default().fg(ACCENT)), Span::raw("▏")]),
        TagMode::AddingKey { buffer } => Line::from(vec![Span::raw("new key: "), Span::styled(buffer.clone(), Style::default().fg(ACCENT)), Span::raw("▏")]),
        TagMode::AddingValue { key, buffer } => Line::from(vec![Span::raw(format!("{key} = ")), Span::styled(buffer.clone(), Style::default().fg(ACCENT)), Span::raw("▏")]),
    };
    f.render_widget(Paragraph::new(input_line).block(panel("Edit")), chunks[1]);
    f.render_widget(
        Paragraph::new(Span::styled(
            "Normalization (spacing, closed value sets) is applied on save.",
            Style::default().fg(Color::DarkGray),
        )),
        chunks[2],
    );
}

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let rect = centered(area, 70, 12);
    f.render_widget(Clear, rect);
    let label = match app.screen {
        Screen::Import => "Import path",
        _ => "Input",
    };
    let text = vec![Line::from(vec![
        Span::raw(format!("{label}: ")),
        Span::styled(app.input_buffer.clone(), Style::default().fg(ACCENT)),
        Span::raw("▏"),
    ])];
    f.render_widget(Paragraph::new(text).block(panel("Enter to confirm · Esc to cancel")), rect);
}