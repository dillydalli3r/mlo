//! SQLite index ([§12.2]) via `rusqlite` with the bundled SQLite (no system dep).

use crate::error::{IoResultExt, Result};
use crate::model::{CheckStatus, ContainerKind, JobRecord, JobState, PlayerState, Repeat, TagMap};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// One indexed file (`library` table; a superset of the §12.2 minimum so the
/// index carries durations, sidecar presence and grade results, [§4.1]).
#[derive(Debug, Clone, Default)]
pub struct FileRow {
    pub path: PathBuf,
    pub kind: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<String>,
    pub disc: Option<String>,
    pub container: Option<ContainerKind>,
    pub duration_ms: Option<u64>,
    pub mtime: i64,
    pub size: i64,
    pub quick_hash: Option<String>,
    pub has_cover: bool,
    pub has_lyrics: bool,
    pub has_cue: bool,
    pub has_log: bool,
    pub has_accurip: bool,
    pub grade_pct: Option<f64>,
    pub grade_pass: Option<bool>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).at(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        // WAL is faster for a TUI that reads while a job writes; fall back
        // silently on filesystems that reject it.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(None, "foreign_keys", "ON");
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS library(
                path TEXT PRIMARY KEY,
                kind TEXT NOT NULL DEFAULT 'audio',
                artist TEXT, album TEXT, track TEXT, disc TEXT,
                container TEXT,
                duration_ms INTEGER,
                mtime INTEGER NOT NULL DEFAULT 0,
                size INTEGER NOT NULL DEFAULT 0,
                quick_hash TEXT,
                indexed_at TEXT NOT NULL DEFAULT '',
                has_cover INTEGER NOT NULL DEFAULT 0,
                has_lyrics INTEGER NOT NULL DEFAULT 0,
                has_cue INTEGER NOT NULL DEFAULT 0,
                has_log INTEGER NOT NULL DEFAULT 0,
                has_accurip INTEGER NOT NULL DEFAULT 0,
                grade_pct REAL,
                grade_pass INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_library_album ON library(album);
            CREATE INDEX IF NOT EXISTS idx_library_artist ON library(artist);

            CREATE TABLE IF NOT EXISTS tags(
                path TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tags_path ON tags(path);
            CREATE INDEX IF NOT EXISTS idx_tags_key ON tags(key);

            CREATE TABLE IF NOT EXISTS albums(
                id TEXT PRIMARY KEY,
                artist TEXT,
                title TEXT,
                mb_release_id TEXT,
                mb_release_group_id TEXT,
                path TEXT,
                grade_pct REAL,
                pass INTEGER
            );
            CREATE TABLE IF NOT EXISTS artists(
                name TEXT PRIMARY KEY,
                path TEXT,
                mb_artist_id TEXT,
                grade_pass INTEGER,
                has_image INTEGER NOT NULL DEFAULT 0,
                has_description INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS queue(
                pos INTEGER PRIMARY KEY,
                path TEXT NOT NULL,
                added_at TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS player_state(
                id INTEGER PRIMARY KEY CHECK (id = 1),
                path TEXT,
                position_ms INTEGER NOT NULL DEFAULT 0,
                volume REAL NOT NULL DEFAULT 0.8,
                shuffle INTEGER NOT NULL DEFAULT 0,
                repeat TEXT NOT NULL DEFAULT 'off',
                updated_at TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS cache(
                service TEXT NOT NULL,
                key TEXT NOT NULL,
                body TEXT NOT NULL,
                fetched_at INTEGER NOT NULL,
                ttl_s INTEGER NOT NULL,
                PRIMARY KEY(service, key)
            );
            CREATE TABLE IF NOT EXISTS jobs(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                scope TEXT NOT NULL,
                state TEXT NOT NULL,
                started_at TEXT NOT NULL DEFAULT '',
                finished_at TEXT,
                journal TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS trash(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                original_path TEXT NOT NULL,
                trash_path TEXT NOT NULL,
                user TEXT NOT NULL,
                stamp TEXT NOT NULL,
                reason TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    // --- file rows ---------------------------------------------------------

    pub fn upsert_file(&self, row: &FileRow) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO library(path,kind,artist,album,track,disc,container,duration_ms,mtime,size,
                quick_hash,indexed_at,has_cover,has_lyrics,has_cue,has_log,has_accurip,grade_pct,grade_pass)
               VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
               ON CONFLICT(path) DO UPDATE SET
                 kind=excluded.kind, artist=excluded.artist, album=excluded.album, track=excluded.track,
                 disc=excluded.disc, container=excluded.container, duration_ms=excluded.duration_ms,
                 mtime=excluded.mtime, size=excluded.size, quick_hash=excluded.quick_hash,
                 indexed_at=excluded.indexed_at, has_cover=excluded.has_cover, has_lyrics=excluded.has_lyrics,
                 has_cue=excluded.has_cue, has_log=excluded.has_log, has_accurip=excluded.has_accurip,
                 grade_pct=excluded.grade_pct, grade_pass=excluded.grade_pass"#,
            params![
                row.path.to_string_lossy(),
                row.kind,
                row.artist,
                row.album,
                row.track,
                row.disc,
                row.container.map(|c| c.as_str().to_string()),
                row.duration_ms.map(|d| d as i64),
                row.mtime,
                row.size,
                row.quick_hash,
                chrono::Local::now().to_rfc3339(),
                row.has_cover as i32,
                row.has_lyrics as i32,
                row.has_cue as i32,
                row.has_log as i32,
                row.has_accurip as i32,
                row.grade_pct,
                row.grade_pass.map(|b| b as i32),
            ],
        )?;
        Ok(())
    }

    /// True when a path is cached with the same mtime+size ([§4.1] reuse).
    pub fn cached_unchanged(&self, path: &Path, mtime: i64, size: i64) -> Result<bool> {
        let found: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT mtime, size FROM library WHERE path=?1",
                params![path.to_string_lossy()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(matches!(found, Some((m, s)) if m == mtime && s == size))
    }

    pub fn remove_file(&self, path: &Path) -> Result<()> {
        let p = path.to_string_lossy().to_string();
        self.conn.execute("DELETE FROM library WHERE path=?1", params![p])?;
        self.conn.execute("DELETE FROM tags WHERE path=?1", params![p])?;
        Ok(())
    }

    pub fn all_files(&self) -> Result<Vec<FileRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT path,kind,artist,album,track,disc,container,duration_ms,mtime,size,quick_hash,
                    has_cover,has_lyrics,has_cue,has_log,has_accurip,grade_pct,grade_pass
             FROM library ORDER BY artist, album, disc, track, path",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(FileRow {
                path: PathBuf::from(r.get::<_, String>(0)?),
                kind: r.get(1)?,
                artist: r.get(2)?,
                album: r.get(3)?,
                track: r.get(4)?,
                disc: r.get(5)?,
                container: r.get::<_, Option<String>>(6)?.and_then(|s| container_from(&s)),
                duration_ms: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
                mtime: r.get(8)?,
                size: r.get(9)?,
                quick_hash: r.get(10)?,
                has_cover: r.get::<_, i32>(11)? != 0,
                has_lyrics: r.get::<_, i32>(12)? != 0,
                has_cue: r.get::<_, i32>(13)? != 0,
                has_log: r.get::<_, i32>(14)? != 0,
                has_accurip: r.get::<_, i32>(15)? != 0,
                grade_pct: r.get(16)?,
                grade_pass: r.get::<_, Option<i32>>(17)?.map(|v| v != 0),
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn file_count(&self) -> Result<u64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM library", [], |r| r.get::<_, i64>(0))? as u64)
    }

    // --- tags --------------------------------------------------------------

    pub fn set_tags(&self, path: &Path, tags: &TagMap) -> Result<()> {
        let p = path.to_string_lossy().to_string();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM tags WHERE path=?1", params![p])?;
        {
            let mut stmt = tx.prepare("INSERT INTO tags(path,key,value) VALUES(?1,?2,?3)")?;
            for (k, vals) in tags {
                for v in vals {
                    stmt.execute(params![p, k, v])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_tags(&self, path: &Path) -> Result<TagMap> {
        let p = path.to_string_lossy().to_string();
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM tags WHERE path=?1 ORDER BY key, value")?;
        let rows = stmt.query_map(params![p], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut map = TagMap::new();
        for row in rows {
            let (k, v) = row?;
            map.entry(k).or_default().push(v);
        }
        Ok(map)
    }

    // --- grade results -----------------------------------------------------

    pub fn set_grade(&self, path: &Path, pct: f64, pass: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE library SET grade_pct=?2, grade_pass=?3 WHERE path=?1",
            params![path.to_string_lossy(), pct, pass as i32],
        )?;
        Ok(())
    }

    pub fn upsert_album(
        &self,
        id: &str,
        artist: &str,
        title: &str,
        path: &Path,
        grade_pct: Option<f64>,
        pass: Option<bool>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO albums(id,artist,title,path,grade_pct,pass) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET artist=excluded.artist,title=excluded.title,
               path=excluded.path,grade_pct=excluded.grade_pct,pass=excluded.pass",
            params![id, artist, title, path.to_string_lossy(), grade_pct, pass.map(|b| b as i32)],
        )?;
        Ok(())
    }

    pub fn upsert_artist(
        &self,
        name: &str,
        path: &Path,
        grade_pass: Option<bool>,
        has_image: bool,
        has_description: bool,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO artists(name,path,grade_pass,has_image,has_description) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(name) DO UPDATE SET path=excluded.path,grade_pass=excluded.grade_pass,
               has_image=excluded.has_image,has_description=excluded.has_description",
            params![name, path.to_string_lossy(), grade_pass.map(|b| b as i32), has_image as i32, has_description as i32],
        )?;
        Ok(())
    }

    pub fn clear_index(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM library; DELETE FROM tags; DELETE FROM albums; DELETE FROM artists;")?;
        Ok(())
    }

    // --- queue / player ([§11.2]) -----------------------------------------

    pub fn queue_list(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM queue ORDER BY pos")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(PathBuf::from(r?));
        }
        Ok(out)
    }

    pub fn queue_replace(&self, paths: &[PathBuf]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM queue", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO queue(pos,path,added_at) VALUES(?1,?2,?3)")?;
            let now = chrono::Local::now().to_rfc3339();
            for (i, p) in paths.iter().enumerate() {
                stmt.execute(params![i as i64, p.to_string_lossy(), now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn player_state(&self) -> Result<PlayerState> {
        let found = self
            .conn
            .query_row(
                "SELECT path,position_ms,volume,shuffle,repeat FROM player_state WHERE id=1",
                [],
                |r| {
                    Ok(PlayerState {
                        path: r.get::<_, Option<String>>(0)?.map(PathBuf::from),
                        position_ms: r.get::<_, i64>(1)? as u64,
                        volume: r.get::<_, f64>(2)? as f32,
                        shuffle: r.get::<_, i32>(3)? != 0,
                        repeat: match r.get::<_, String>(4)?.as_str() {
                            "all" => Repeat::All,
                            "one" => Repeat::One,
                            _ => Repeat::Off,
                        },
                    })
                },
            )
            .optional()?;
        Ok(found.unwrap_or_default())
    }

    pub fn set_player_state(&self, st: &PlayerState) -> Result<()> {
        let repeat = match st.repeat {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        };
        self.conn.execute(
            "INSERT INTO player_state(id,path,position_ms,volume,shuffle,repeat,updated_at)
             VALUES(1,?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET path=excluded.path,position_ms=excluded.position_ms,
               volume=excluded.volume,shuffle=excluded.shuffle,repeat=excluded.repeat,updated_at=excluded.updated_at",
            params![
                st.path.as_ref().map(|p| p.to_string_lossy().to_string()),
                st.position_ms as i64,
                st.volume as f64,
                st.shuffle as i32,
                repeat,
                chrono::Local::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    // --- cache ([§10.1]) ---------------------------------------------------

    pub fn cache_get(&self, service: &str, key: &str, now: i64) -> Result<Option<String>> {
        let found: Option<(String, i64, i64)> = self
            .conn
            .query_row(
                "SELECT body, fetched_at, ttl_s FROM cache WHERE service=?1 AND key=?2",
                params![service, key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match found {
            Some((body, fetched, ttl)) if now - fetched <= ttl => Some(body),
            _ => None,
        })
    }

    pub fn cache_put(&self, service: &str, key: &str, body: &str, now: i64, ttl_s: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO cache(service,key,body,fetched_at,ttl_s) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(service,key) DO UPDATE SET body=excluded.body,fetched_at=excluded.fetched_at,ttl_s=excluded.ttl_s",
            params![service, key, body, now, ttl_s],
        )?;
        Ok(())
    }

    // --- jobs --------------------------------------------------------------

    pub fn job_start(&self, kind: &str, scope: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO jobs(kind,scope,state,started_at,journal) VALUES(?1,?2,'running',?3,'')",
            params![kind, scope, chrono::Local::now().to_rfc3339()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn job_finish(&self, id: i64, state: JobState, journal: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state=?2, finished_at=?3, journal=?4 WHERE id=?1",
            params![id, state.as_str(), chrono::Local::now().to_rfc3339(), journal],
        )?;
        Ok(())
    }

    pub fn jobs_recent(&self, limit: u32) -> Result<Vec<JobRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,kind,scope,state,started_at,finished_at,journal FROM jobs ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(JobRecord {
                id: r.get(0)?,
                kind: r.get(1)?,
                scope: r.get(2)?,
                state: match r.get::<_, String>(3)?.as_str() {
                    "queued" => JobState::Queued,
                    "running" => JobState::Running,
                    "done" => JobState::Done,
                    "failed" => JobState::Failed,
                    _ => JobState::Cancelled,
                },
                started_at: r.get(4)?,
                finished_at: r.get(5)?,
                journal: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Jobs left `running` by a crash ([§3.5]).
    pub fn abandoned_jobs(&self) -> Result<Vec<JobRecord>> {
        let mut out = Vec::new();
        for j in self.jobs_recent(200)? {
            if matches!(j.state, JobState::Running | JobState::Queued) {
                out.push(j);
            }
        }
        Ok(out)
    }

    // --- trash records -----------------------------------------------------

    pub fn record_trash(&self, original: &Path, trash_path: &Path, user: &str, stamp: &str, reason: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO trash(original_path,trash_path,user,stamp,reason) VALUES(?1,?2,?3,?4,?5)",
            params![
                original.to_string_lossy(),
                trash_path.to_string_lossy(),
                user,
                stamp,
                reason
            ],
        )?;
        Ok(())
    }

    pub fn grade_summary(&self) -> Result<(u32, u32, u32)> {
        let total: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM library WHERE kind='audio'", [], |r| r.get(0))?;
        let passed: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM library WHERE kind='audio' AND grade_pass=1",
            [],
            |r| r.get(0),
        )?;
        let failed: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM library WHERE kind='audio' AND grade_pass=0",
            [],
            |r| r.get(0),
        )?;
        Ok((total as u32, passed as u32, failed as u32))
    }
}

fn container_from(s: &str) -> Option<ContainerKind> {
    Some(match s {
        "flac" => ContainerKind::Flac,
        "ogg" => ContainerKind::OggVorbis,
        "opus" => ContainerKind::Opus,
        "mp3" => ContainerKind::Mp3,
        "m4a" => ContainerKind::Mp4,
        "wav" => ContainerKind::Wav,
        "aiff" => ContainerKind::Aiff,
        "mkv" => ContainerKind::Mkv,
        "mp4" => ContainerKind::Mp4Video,
        _ => return None,
    })
}

/// Convenience: map a `CheckStatus` to the DB's pass flag.
pub fn status_to_pass(s: &CheckStatus) -> bool {
    s.passed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_tags_and_queue() {
        let db = Db::in_memory().unwrap();
        let mut row = FileRow::default();
        row.path = PathBuf::from("/m/a/track.flac");
        row.kind = "audio".into();
        db.upsert_file(&row).unwrap();
        let mut tags = TagMap::new();
        tags.insert("TITLE".into(), vec!["X".into()]);
        db.set_tags(Path::new("/m/a/track.flac"), &tags).unwrap();
        assert_eq!(db.get_tags(Path::new("/m/a/track.flac")).unwrap()["TITLE"], vec!["X"]);
        db.queue_replace(&[PathBuf::from("/m/a/track.flac")]).unwrap();
        assert_eq!(db.queue_list().unwrap().len(), 1);
    }
}