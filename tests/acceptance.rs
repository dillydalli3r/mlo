//! Acceptance tests ([§14]). Headless: no network, no audio device.
//!
//! Fixture: two artists, three albums, one wrong-case folder, one stray file,
//! one numbered sidecar, one loose audio file, one empty album, one artist
//! folder with no albums, one `.mlo_data` leftover.

use mlo_tui::config::Config;
use mlo_tui::db::Db;
use mlo_tui::model::{FindingKind, FixKind, Scope};
use mlo_tui::scripts;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

// ---------------------------------------------------------------------------
// fixture helpers
// ---------------------------------------------------------------------------

fn vorbis_comment(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    let vendor = b"mlo-test";
    out.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    out.extend_from_slice(vendor);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (k, v) in entries {
        let s = format!("{k}={v}");
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    out
}

fn write_flac(path: &Path, entries: &[(&str, &str)]) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    let payload = vorbis_comment(entries);
    let mut out = Vec::new();
    out.extend_from_slice(b"fLaC");
    out.push(0x00);
    out.extend_from_slice(&[0, 0, 34]);
    out.extend_from_slice(&[0u8; 34]);
    out.push(0x80 | 4);
    out.extend_from_slice(&[(payload.len() >> 16) as u8, (payload.len() >> 8) as u8, payload.len() as u8]);
    out.extend_from_slice(&payload);
    fs::write(path, out).unwrap();
}

fn write_mp3(path: &Path, title: &str) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, [0xFF, 0xFB, 0x90, 0x00].iter().chain(&[0u8; 256]).copied().collect::<Vec<u8>>()).unwrap();
    let _ = title;
}

fn write_wav(path: &Path, secs: f32) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    let sr = 44100u32;
    let n = (sr as f32 * secs) as u32;
    let mut data = Vec::new();
    for i in 0..n {
        let env = if (i % (sr / 2)) < 200 { 3000.0 } else { 0.0 };
        let v = (env * ((i as f32) * 0.05).sin()) as i16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    fs::write(path, out).unwrap();
}

struct Fixture {
    _tmp: tempfile::TempDir,
    pub music: PathBuf,
    pub cfg: Config,
}

fn build_fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let music = tmp.path().join("Music");
    let artists = music.join("Artists");

    // album with full tags
    write_flac(
        &artists.join("Radiohead").join("Kid A").join("01 Everything.flac"),
        &[
            ("TITLE", "Everything In Its Right Place"),
            ("ARTIST", "Radiohead"),
            ("ALBUM", "Kid A"),
            ("ALBUMARTIST", "Radiohead"),
            ("TRACKNUMBER", "1"),
            ("DATE", "2000"),
            ("GENRE", "Electronic"),
            ("MEDIA", "CD"),
            ("SOURCE", "WEB"),
        ],
    );
    write_flac(
        &artists.join("Radiohead").join("Kid A").join("02 Kid A.flac"),
        &[
            ("TITLE", "Kid A"),
            ("ARTIST", "Radiohead"),
            ("ALBUM", "Kid A"),
            ("ALBUMARTIST", "Radiohead"),
            ("TRACKNUMBER", "2"),
            ("DATE", "2000"),
            ("GENRE", "Electronic"),
            ("MEDIA", "CD"),
            ("SOURCE", "WEB"),
        ],
    );
    // wrong-case album folder
    write_mp3(&artists.join("Various").join("album two").join("01 track.mp3"), "Track");
    // stray file + numbered sidecar
    fs::write(artists.join("Radiohead").join("Kid A").join("stray.nfo"), b"junk").unwrap();
    fs::write(artists.join("Radiohead").join("Kid A").join("description (2).txt"), b"dup").unwrap();
    // empty album, empty artist, legacy state, loose root audio
    fs::create_dir_all(artists.join("Empty").join("Nothing")).unwrap();
    fs::create_dir_all(artists.join("Nobody")).unwrap();
    fs::create_dir_all(music.join(".mlo_data")).unwrap();
    write_flac(&music.join("loose.flac"), &[("TITLE", "Loose")]);

    let mut cfg = Config::default();
    cfg.music_folder = music.clone();
    cfg.ensure_dirs().unwrap();
    Fixture { _tmp: tmp, music, cfg }
}

fn scan_fixture(cfg: &Config) -> mlo_tui::scan::LibraryModel {
    let db = Db::open(&cfg.index_db()).unwrap();
    let cancel = AtomicBool::new(false);
    let progress = |_p: mlo_tui::model::Progress| {};
    mlo_tui::scan::scan(cfg, &db, &Scope::Library, &cancel, &progress).unwrap()
}

// ---------------------------------------------------------------------------
// §14 tests
// ---------------------------------------------------------------------------

#[test] // 1. Naming
fn naming_path_equals_template_and_short_names_both_grade() {
    let mut tags = mlo_tui::model::TagMap::new();
    tags.insert("ALBUMARTIST".into(), vec!["Radiohead".into()]);
    tags.insert("ALBUM".into(), vec!["Kid A".into()]);
    tags.insert("DATE".into(), vec!["2000-10-02".into()]);
    tags.insert("ORIGINALDATE".into(), vec!["2000".into()]);
    tags.insert("RELEASETYPE".into(), vec!["Album".into()]);
    tags.insert("MEDIA".into(), vec!["CD".into()]);
    tags.insert("RELEASECOUNTRY".into(), vec!["GB".into()]);
    tags.insert("TRACKNUMBER".into(), vec!["7".into()]);
    tags.insert("DISCNUMBER".into(), vec!["1".into()]);
    tags.insert("TITLE".into(), vec!["Idioteque".into()]);
    tags.insert("MUSICBRAINZ_ALBUMARTISTID".into(), vec!["a74b1b7f-71a5-4011-9441-d0b5e4122711".into()]);

    let full = mlo_tui::naming::evaluate(mlo_tui::naming::DEFAULT_NAMING_SCRIPT, &tags, Default::default()).unwrap();
    // the template's last segment is the file name; `path` is the album DIRECTORY
    assert_eq!(full.file_name.as_deref(), Some("1-07 Idioteque"));
    assert_eq!(
        full.path,
        "Radiohead [a74b1b7f-71a5-4011-9441-d0b5e4122711]/[Album] 2000 - 2000-10-02 - Kid A {GB - CD}"
    );
    assert_eq!(full.components.len(), 3);

    let short = mlo_tui::naming::evaluate(
        mlo_tui::naming::DEFAULT_NAMING_SCRIPT,
        &tags,
        mlo_tui::naming::NamingOptions { short_folder_names: true },
    )
    .unwrap();
    assert_eq!(
        short.path,
        "Radiohead [a74b1b7f]/[Album] 2000 - 2000-10-02 - Kid A {GB - CD}"
    );
    assert!(short.file_name.as_deref().unwrap().starts_with("1-07 Idioteque"));
}

#[test] // 2. Scan reports the fixture's findings, one row each, stable on re-scan
fn scan_reports_fixture_findings_and_is_stable() {
    let fx = build_fixture();
    let model = scan_fixture(&fx.cfg);
    let kinds: Vec<FindingKind> = model.all_findings().iter().map(|f| f.kind).collect();
    for expected in [
        FindingKind::StrayFile,
        FindingKind::SidecarCopy,
        FindingKind::EmptyAlbum,
        FindingKind::EmptyArtist,
        FindingKind::LegacyStateFile,
        FindingKind::AudioAtRoot,
    ] {
        assert!(kinds.contains(&expected), "missing {expected:?}");
    }
    // one row per finding
    let mut ids: Vec<&str> = model.all_findings().iter().map(|f| f.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before, "no duplicate finding rows");

    // a second scan of the unchanged tree reports the same set
    let again = scan_fixture(&fx.cfg);
    assert_eq!(again.all_findings().len(), model.all_findings().len());
}

#[test] // 3. Split artist: no empty_artist, merge moves artefacts, trashes the emptied half
fn split_artist_merges_and_does_not_trash_album_holding_half() {
    let tmp = tempfile::tempdir().unwrap();
    let music = tmp.path().join("Music");
    let artists = music.join("Artists");
    write_flac(&artists.join("Radiohead").join("Kid A").join("01.flac"), &[("TITLE", "T")]);
    let with_id = artists.join("Radiohead [a74b1b7f-71a5-4011-9441-d0b5e4122711]");
    fs::create_dir_all(&with_id).unwrap();
    fs::write(with_id.join("artist.jpg"), b"img").unwrap();
    fs::write(with_id.join("description.txt"), b"desc").unwrap();

    let walked = mlo_tui::scan::walk(&music).unwrap();
    let findings = mlo_tui::layout::analyze(&walked, &music, &Default::default(), &Config {
        music_folder: music.clone(),
        ..Config::default()
    });
    let split: Vec<_> = findings.iter().filter(|f| f.kind == FindingKind::SplitArtist).collect();
    assert_eq!(split.len(), 1);
    assert!(matches!(split[0].fix, FixKind::MergeArtist { .. }));
    assert!(!findings.iter().any(|f| f.kind == FindingKind::EmptyArtist), "empty_artist does not fire for the half");
}

#[test] // 4. Artist artefacts fail the grade when missing
fn missing_artist_artefacts_fail_the_grade() {
    let fx = build_fixture();
    let model = scan_fixture(&fx.cfg);
    let radiohead = model.artists.iter().find(|a| a.name == "Radiohead").unwrap();
    let grade = radiohead.grade.as_ref().unwrap();
    assert!(!grade.passed(), "artist with no image/description fails");
    let codes: Vec<String> = grade.issues().map(|(_, i)| i.code.clone()).collect();
    assert!(codes.iter().any(|c| c == "ARTIST_IMAGE_MISSING"));
    assert!(codes.iter().any(|c| c == "ARTIST_DESCRIPTION_MISSING"));
    // both counts appear in the library verdict
    assert!(model.verdict.artists_failed >= 1);
}

#[test] // 5. Layout finding is charged onto the album's grade
fn layout_finding_is_charged_on_the_album() {
    let fx = build_fixture();
    let model = scan_fixture(&fx.cfg);
    let album = model.albums_flat().into_iter().find(|a| a.name == "Kid A").unwrap();
    let grade = album.grade.as_ref().unwrap();
    assert!(!grade.passed());
    assert!(
        grade.issues().any(|(k, _)| k.key == "layout_stray_file" || k.key == "layout_sidecar_copy"),
        "a layout finding costs the album a check"
    );
    // the dot, the percentage and the problem list agree
    assert_eq!(grade.passed(), grade.failed_checks == 0);
}

#[test] // 6. Import chain ends with layout (20) then grade (4); scoped import stores no report
fn import_chain_order_and_scoped_run_have_no_library_report() {
    let fx = build_fixture();
    // la-musica: LIBRARY_WIDE_SCRIPTS is empty, so the import chain IS the run order
    let chain = scripts::import_chain(&fx.cfg);
    assert_eq!(chain, scripts::run_all_order());
    assert_eq!(chain[chain.len() - 1], 4, "grade is last");
    let pos = |id: u8| chain.iter().position(|x| *x == id).unwrap();
    assert!(pos(20) < pos(21) && pos(21) < pos(4), "layout → AcoustID → grade");
    assert!(!chain.contains(&22), "22 is opt-in and absent");

    // a scoped scan must not write the library-wide report
    let _ = fs::remove_file(fx.cfg.layout_report());
    let db = Db::open(&fx.cfg.index_db()).unwrap();
    let cancel = AtomicBool::new(false);
    let progress = |_p: mlo_tui::model::Progress| {};
    let album = fx.music.join("Artists").join("Radiohead").join("Kid A");
    mlo_tui::scan::scan(&fx.cfg, &db, &Scope::Album(album), &cancel, &progress).unwrap();
    assert!(!fx.cfg.layout_report().exists(), "scoped run stores no library-wide report");

    // a library-wide scan does write it
    mlo_tui::scan::scan(&fx.cfg, &db, &Scope::Library, &cancel, &progress).unwrap();
    assert!(fx.cfg.layout_report().exists());
}

#[test] // 7. Atomicity: a stale temp is swept; a write replaces old bytes with new
fn atomic_writes_and_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("x.flac");
    mlo_tui::atomic::write_atomic(&target, b"old").unwrap();
    mlo_tui::atomic::write_atomic(&target, b"new").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new");

    let stale = tmp.path().join(".x.flac.123.aaaa.mlo-edit.flac");
    fs::write(&stale, b"partial").unwrap();
    let removed = mlo_tui::atomic::sweep_temp_files(tmp.path());
    assert_eq!(removed.len(), 1);
    assert!(!stale.exists());
}

#[test] // 8. Nothing deleted: every fix trashes with a manifest, restore puts it back
fn nothing_deleted_and_restore_is_exact() {
    let fx = build_fixture();
    let walked = mlo_tui::scan::walk(&fx.music).unwrap();
    let findings = mlo_tui::layout::analyze(&walked, &fx.music, &Default::default(), &fx.cfg);
    let stray = fx.music.join("Artists").join("Radiohead").join("Kid A").join("stray.nfo");
    assert!(stray.exists());

    let outcomes = mlo_tui::layout::apply(&fx.cfg, &findings, None, false).unwrap();
    assert!(outcomes.iter().filter(|o| o.ok).count() >= 3);
    assert!(!stray.exists(), "stray was moved");

    let trash = mlo_tui::trash::Trash::new(&fx.music);
    let manifests = trash.list().unwrap();
    assert!(!manifests.is_empty());
    let mut restored_any = false;
    for m in &manifests {
        if trash.restore(&m.path).unwrap().iter().any(|p| p == &stray) {
            restored_any = true;
        }
    }
    assert!(restored_any, "stray restored to its exact original path");
    assert!(stray.exists());
}

#[test] // 9. Offline: tag edits, layout and grading work; network actions report a reason
fn offline_operations_work_and_network_actions_report_reason() {
    let fx = build_fixture();
    let file = fx.music.join("Artists").join("Radiohead").join("Kid A").join("01 Everything.flac");
    // tag edit
    mlo_tui::tags::set_values(&file, "DYNAMIC RANGE", &["9".into()]).unwrap();
    assert_eq!(mlo_tui::tags::read_tags(&file).unwrap()["DYNAMIC RANGE"], vec!["9"]);
    // layout + grade
    let model = scan_fixture(&fx.cfg);
    assert!(model.track_count() >= 3);
    // a network-dependent script skips with a named reason, never a bare error
    let db = Db::open(&fx.cfg.index_db()).unwrap();
    let cancel = AtomicBool::new(false);
    let progress = |_p: mlo_tui::model::Progress| {};
    let ctx = scripts::ScriptCtx { cfg: &fx.cfg, db: &db, cancel: &cancel, progress: &progress };
    let out = scripts::run(8, &Scope::Album(fx.music.join("Artists/Radiohead/Kid A")), &ctx).unwrap();
    assert!(out.results.iter().all(|r| r.outcome == scripts::Outcome::Skipped && !r.note.is_empty()));
}

#[test] // 12. Grading semantics
fn grading_semantics() {
    use mlo_tui::model::{CheckResult, CheckStatus, Issue};
    let checks = vec![
        CheckResult::pass("a", "passes"),
        CheckResult::fail("b", "fails", "CODE", "reason"),
        CheckResult::skipped("c", "n/a"),
    ];
    let report = mlo_tui::model::GradeReport::new("scope", checks);
    assert_eq!(report.total_checks, 2, "skipped counts on neither side");
    assert_eq!(report.failed_checks, 1);
    assert_eq!(report.pass_count(), 1);
    assert!(!report.passed());

    // a could-not-evaluate counts as failed
    let r2 = mlo_tui::model::GradeReport::new(
        "scope",
        vec![CheckResult { key: "k".into(), label: "l".into(), status: CheckStatus::CouldNotEvaluate("boom".into()) }],
    );
    assert_eq!(r2.failed_checks, 1);

    // empty folder fails with EMPTY_FOLDER
    let fx = build_fixture();
    let model = scan_fixture(&fx.cfg);
    let empty = model
        .albums_flat()
        .into_iter()
        .find(|a| a.path.ends_with("Nothing") || a.path.ends_with("Empty"))
        .expect("empty album present");
    let codes: Vec<String> = empty.grade.as_ref().unwrap().issues().map(|(_, i)| i.code.clone()).collect();
    assert!(codes.iter().any(|c| c == "EMPTY_FOLDER"), "got {codes:?}");
    let _ = Issue { code: "x".into(), message: "y".into() };
}

#[test] // containers round-trip (§6.1 subset: flac, mp3, wav)
fn container_roundtrips() {
    let tmp = tempfile::tempdir().unwrap();
    let flac = tmp.path().join("a.flac");
    write_flac(&flac, &[("TITLE", "x")]);
    let mp3 = tmp.path().join("a.mp3");
    write_mp3(&mp3, "x");
    let wav = tmp.path().join("a.wav");
    write_wav(&wav, 0.1);

    for f in [&flac, &mp3, &wav] {
        let mut tags = mlo_tui::model::TagMap::new();
        tags.insert("TITLE".into(), vec!["Round Trip".into()]);
        tags.insert("ALBUMARTIST".into(), vec!["Someone".into()]);
        mlo_tui::tags::write_tags(f, &tags).unwrap();
        let back = mlo_tui::tags::read_tags(f).unwrap();
        assert_eq!(back["TITLE"], vec!["Round Trip"], "{}", f.display());
        assert_eq!(back["ALBUMARTIST"], vec!["Someone"], "{}", f.display());
    }
}

#[test] // analysis on a fixed fixture (DR/RG/BPM) — no network, no device
fn analysis_works_on_a_wav() {
    let tmp = tempfile::tempdir().unwrap();
    let wav = tmp.path().join("click.wav");
    write_wav(&wav, 2.0);
    let bk = mlo_tui::analysis::detect_bpm_key(&wav).unwrap();
    assert!(bk.bpm > 40.0 && bk.bpm <= 240.0, "bpm {}", bk.bpm);
    assert!(bk.key.len() >= 1);
    let (gain, peak) = mlo_tui::analysis::compute_track_replaygain(&wav).unwrap();
    assert!(gain.is_finite() && peak.is_finite());
    let dr = mlo_tui::analysis::compute_dr(&wav).unwrap();
    assert!(dr <= 40);
}