//! Blocking network clients ([§10.2]) — plain `ureq` (rustls) on worker
//! threads, no async runtime. Every response body is cached in SQLite so a
//! re-run costs no network round-trip.
//!
//! Design notes:
//! * `HttpClient` owns an [`ureq::Agent`] (connection-pooled, `Send + Sync`)
//!   plus an optional owned [`crate::db::Db`] for the on-disk response cache.
//!   The cache is opt-in via [`HttpClient::with_cache`]; without it calls go
//!   straight to the network.
//! * `MusicBrainz` wraps an `HttpClient` with a 1 req/s limiter (configured by
//!   `Services::musicbrainz_rate_per_s`) and sends the required User-Agent.
//! * Every failure is mapped to [`MloError::Network`] or
//!   [`MloError::Service`] with a concrete reason (status code, timeout, DNS,
//!   …). The clients never panic and never wait forever — the configured
//!   timeout bounds each request.

use crate::config::Services;
use crate::db::Db;
use crate::error::{MloError, Result};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// MusicBrainz web service base.
const MB_BASE: &str = "https://musicbrainz.org/ws/2";
/// Cover Art Archive base.
const CAA_BASE: &str = "https://coverartarchive.org/release";
/// AcoustID lookup endpoint.
const ACOUSTID_URL: &str = "https://api.acoustid.org/v2/lookup";
/// LRCLIB lookup endpoint.
const LRCLIB_URL: &str = "https://lrclib.net/api/get";
/// User-Agent used by config-less free helpers.
fn generic_user_agent() -> String {
    format!("mlo-tui/{} ( https://github.com/dillydalli3r/mlo )", env!("CARGO_PKG_VERSION"))
}

/// Blocking HTTP client with optional SQLite response cache.
///
/// Not `Sync` when a cache is attached (`rusqlite::Connection`); give each
/// worker thread its own instance, or guard a shared one with a mutex.
pub struct HttpClient {
    agent: ureq::Agent,
    timeout: Duration,
    cache_ttl_s: i64,
    db: Option<Db>,
}

impl HttpClient {
    /// Build a client from the service configuration (User-Agent + timeouts).
    pub fn new(cfg: &Services) -> Self {
        let timeout = duration_from_s(cfg.request_timeout_s);
        Self {
            agent: build_agent(timeout, &cfg.musicbrainz_user_agent, true),
            timeout,
            cache_ttl_s: cfg.cache_ttl_s as i64,
            db: None,
        }
    }

    /// Attach an on-disk response cache. Consumes the client (builder style).
    pub fn with_cache(mut self, db: Db) -> Self {
        self.db = Some(db);
        self
    }

    /// GET `url`, returning a parsed JSON value. Cached by full URL.
    pub fn get_json(&self, service: &str, url: &str) -> Result<Value> {
        let body = self.fetch_text(service, url, true)?;
        serde_json::from_str(&body).map_err(|e| {
            MloError::service(service, format!("invalid json from {url}: {e}"))
        })
    }

    /// GET `url`, returning the raw bytes. Not cached (binary payloads).
    pub fn get_bytes(&self, service: &str, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .agent
            .get(url)
            .call()
            .map_err(|e| map_ureq_error(service, url, e))?;
        let mut body = resp;
        body.body_mut()
            .read_to_vec()
            .map_err(|e| MloError::service(service, format!("read body from {url}: {e}")))
    }

    /// GET `url`, returning UTF-8 text. Cached by full URL.
    pub fn get_text(&self, service: &str, url: &str) -> Result<String> {
        self.fetch_text(service, url, true)
    }

    fn fetch_text(&self, service: &str, url: &str, use_cache: bool) -> Result<String> {
        let now = chrono::Utc::now().timestamp();
        if use_cache {
            if let Some(db) = &self.db {
                if let Some(hit) = db.cache_get(service, url, now)? {
                    return Ok(hit);
                }
            }
        }
        let resp = self
            .agent
            .get(url)
            .call()
            .map_err(|e| map_ureq_error(service, url, e))?;
        let mut resp = resp;
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| MloError::service(service, format!("read body from {url}: {e}")))?;
        if use_cache {
            if let Some(db) = &self.db {
                db.cache_put(service, url, &text, now, self.cache_ttl_s)?;
            }
        }
        Ok(text)
    }

    /// Configured per-request timeout.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// Resolve `request_timeout_s` to a `Duration`, guarding against `0` (which
/// ureq treats as “no timeout” and could hang forever).
fn duration_from_s(s: u64) -> Duration {
    Duration::from_secs(s.max(1))
}

fn build_agent(timeout: Duration, user_agent: &str, status_as_error: bool) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout))
        .user_agent(user_agent)
        .http_status_as_error(status_as_error)
        .build();
    ureq::Agent::new_with_config(config)
}

/// Map a transport error to a named [`MloError`], preserving the concrete
/// reason (HTTP status, timeout, DNS, I/O, …).
fn map_ureq_error(service: &str, url: &str, e: ureq::Error) -> MloError {
    match e {
        ureq::Error::StatusCode(code) => {
            MloError::service(service, format!("http status {code} from {url}"))
        }
        ureq::Error::HostNotFound => {
            MloError::Network { reason: format!("dns lookup failed for {url}") }
        }
        ureq::Error::Timeout(t) => {
            MloError::Network { reason: format!("timeout ({t}) for {url}") }
        }
        ureq::Error::Io(err) => {
            MloError::Network { reason: format!("io error for {url}: {err}") }
        }
        ureq::Error::ConnectionFailed => {
            MloError::Network { reason: format!("connection failed for {url}") }
        }
        other => MloError::Network { reason: format!("{other} for {url}") },
    }
}

// --- MusicBrainz -----------------------------------------------------------

/// One release returned by a MusicBrainz search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCandidate {
    pub mbid: String,
    pub title: String,
    pub artist: String,
    pub date: Option<String>,
    pub score: i32,
    pub track_count: Option<usize>,
}

/// MusicBrainz client with a policy-required 1 req/s rate limiter.
pub struct MusicBrainz {
    http: HttpClient,
    limiter: RateLimiter,
}

impl MusicBrainz {
    pub fn new(cfg: &Services) -> Self {
        Self {
            http: HttpClient::new(cfg),
            limiter: RateLimiter::new(cfg.musicbrainz_rate_per_s),
        }
    }

    /// Attach an on-disk response cache (builder style).
    pub fn with_cache(mut self, db: Db) -> Self {
        self.http = self.http.with_cache(db);
        self
    }

    /// Search releases by artist + album, ranked best-first. `tracks` breaks
    /// ties in favour of releases whose track count matches the local album.
    pub fn search_release(&self, artist: &str, album: &str, tracks: usize) -> Result<Vec<ReleaseCandidate>> {
        let query = format!(
            "artist:\"{}\" AND release:\"{}\"",
            escape_lucene(artist),
            escape_lucene(album)
        );
        let url = format!(
            "{MB_BASE}/release?query={}&fmt=json&limit=25",
            encode_component(&query)
        );
        self.limiter.acquire();
        let value = self.http.get_json("musicbrainz", &url)?;
        let mut candidates = parse_release_candidates(&value);
        rank_candidates(&mut candidates, tracks);
        Ok(candidates)
    }

    /// Full release document, including recordings.
    pub fn release(&self, mbid: &str) -> Result<Value> {
        let url = format!(
            "{MB_BASE}/release/{}?fmt=json&inc=artists+recordings+release-groups+labels",
            encode_component(mbid)
        );
        self.limiter.acquire();
        self.http.get_json("musicbrainz", &url)
    }

    /// Artist document (aliases, tags, URL relationships).
    pub fn artist(&self, mbid: &str) -> Result<Value> {
        let url = format!(
            "{MB_BASE}/artist/{}?fmt=json&inc=aliases+tags+url-rels",
            encode_component(mbid)
        );
        self.limiter.acquire();
        self.http.get_json("musicbrainz", &url)
    }

    /// ISRC lookup (recordings that carry the code).
    pub fn lookup_isrc(&self, isrc: &str) -> Result<Value> {
        let url = format!(
            "{MB_BASE}/isrc/{}?fmt=json&inc=recordings",
            encode_component(isrc)
        );
        self.limiter.acquire();
        self.http.get_json("musicbrainz", &url)
    }
}

/// Simple spacing limiter: no more than one acquisition per `interval`.
struct RateLimiter {
    interval: Duration,
    next: Mutex<Instant>,
}

impl RateLimiter {
    fn new(per_s: f32) -> Self {
        let interval = if per_s.is_finite() && per_s > 0.0 {
            Duration::from_secs_f64(1.0 / per_s as f64)
        } else {
            Duration::ZERO
        };
        Self { interval, next: Mutex::new(Instant::now()) }
    }

    /// Block until the next slot is available.
    fn acquire(&self) {
        // A poisoned mutex only means another thread panicked mid-update; the
        // value is still a valid `Instant`, so recover rather than panic.
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if *next > now {
            std::thread::sleep(*next - now);
        }
        *next = Instant::now() + self.interval;
    }
}

// --- Pure parsing / URL helpers -------------------------------------------

/// Parse a MusicBrainz release-search response. Missing/malformed fields are
/// skipped, never panicking; a non-object or missing `releases` yields `[]`.
pub fn parse_release_candidates(value: &Value) -> Vec<ReleaseCandidate> {
    let releases = match value.get("releases").and_then(Value::as_array) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(releases.len());
    for r in releases {
        let mbid = match r.get("id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };
        let title = r
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let artist = first_artist_name(r).unwrap_or_default();
        let date = r.get("date").and_then(Value::as_str).map(str::to_string);
        let score = r
            .get("score")
            .and_then(Value::as_i64)
            .or_else(|| r.get("score").and_then(Value::as_f64).map(|f| f as i64))
            .unwrap_or(0) as i32;
        let track_count = media_track_count(r);
        out.push(ReleaseCandidate { mbid, title, artist, date, score, track_count });
    }
    out
}

fn first_artist_name(release: &Value) -> Option<String> {
    release
        .get("artist-credit")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|c| c.get("name").and_then(Value::as_str))
        .map(str::to_string)
}

fn media_track_count(release: &Value) -> Option<usize> {
    let media = release.get("media").and_then(Value::as_array)?;
    let mut total: usize = 0;
    let mut any = false;
    for m in media {
        if let Some(n) = m
            .get("track-count")
            .and_then(Value::as_u64)
            .or_else(|| m.get("track_count").and_then(Value::as_u64))
        {
            total += n as usize;
            any = true;
        }
    }
    if any { Some(total) } else { None }
}

/// Rank by MusicBrainz score (desc), then by track-count proximity to the
/// local album. Stable, so equal scores keep MB's order.
fn rank_candidates(candidates: &mut [ReleaseCandidate], tracks: usize) {
    candidates.sort_by(|a, b| {
        b.score.cmp(&a.score).then_with(|| {
            let da = track_distance(a.track_count, tracks);
            let db = track_distance(b.track_count, tracks);
            da.cmp(&db)
        })
    });
}

fn track_distance(count: Option<usize>, tracks: usize) -> usize {
    match count {
        Some(c) => c.abs_diff(tracks),
        None => usize::MAX,
    }
}

/// Cover Art Archive URL for a release.
///
/// `front == true` yields the direct front-image URL
/// (`…/release/<mbid>/front`, which redirects to the actual image).
/// `front == false` yields the JSON listing of every image with its MIME type
/// (`…/release/<mbid>`), from which callers pick e.g. the back cover.
pub fn cover_art_url(release_mbid: &str, front: bool) -> String {
    if front {
        format!("{CAA_BASE}/{}/front", release_mbid)
    } else {
        format!("{CAA_BASE}/{}", release_mbid)
    }
}

/// AcoustID lookup for a Chromaprint fingerprint. Returns the raw JSON body.
pub fn acoustid_lookup(api_key: &str, fingerprint: &str, duration_s: u32) -> Result<Value> {
    let url = format!(
        "{ACOUSTID_URL}?client={}&meta=recordings+releasegroups+compress&duration={}&fingerprint={}&format=json",
        encode_component(api_key),
        duration_s,
        encode_component(fingerprint),
    );
    let agent = build_agent(Duration::from_secs(20), &generic_user_agent(), true);
    let mut resp = agent
        .get(&url)
        .call()
        .map_err(|e| map_ureq_error("acoustid", &url, e))?;
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| MloError::service("acoustid", format!("read body from {url}: {e}")))?;
    serde_json::from_str(&text)
        .map_err(|e| MloError::service("acoustid", format!("invalid json from {url}: {e}")))
}

/// LRCLIB lyrics lookup. Returns `Ok(None)` when the track is not found.
pub fn lrclib_lookup(artist: &str, title: &str, album: &str) -> Result<Option<Value>> {
    let url = format!(
        "{LRCLIB_URL}?artist_name={}&track_name={}&album_name={}",
        encode_component(artist),
        encode_component(title),
        encode_component(album),
    );
    let agent = build_agent(Duration::from_secs(20), &generic_user_agent(), true);
    match agent.get(&url).call() {
        Ok(mut resp) => {
            let text = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| MloError::service("lrclib", format!("read body from {url}: {e}")))?;
            let value = serde_json::from_str(&text).map_err(|e| {
                MloError::service("lrclib", format!("invalid json from {url}: {e}"))
            })?;
            Ok(Some(value))
        }
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(map_ureq_error("lrclib", &url, e)),
    }
}

/// Best-effort connectivity probe: HEAD MusicBrainz with a short timeout.
/// Reachability means “no transport error”; HTTP status is irrelevant.
pub fn check_connectivity() -> bool {
    let agent = build_agent(Duration::from_secs(5), &generic_user_agent(), false);
    agent.head("https://musicbrainz.org").call().is_ok()
}

/// Percent-encode a URL component (RFC 3986 unreserved set kept verbatim).
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Escape Lucene query specials inside a quoted term.
fn escape_lucene(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_art_urls() {
        assert_eq!(
            cover_art_url("abc-123", true),
            "https://coverartarchive.org/release/abc-123/front"
        );
        assert_eq!(
            cover_art_url("abc-123", false),
            "https://coverartarchive.org/release/abc-123"
        );
    }

    #[test]
    fn encode_component_handles_reserved_and_utf8() {
        assert_eq!(encode_component("a b/c?"), "a%20b%2Fc%3F");
        assert_eq!(encode_component("A-Z.a_z~0"), "A-Z.a_z~0");
        assert_eq!(encode_component("é"), "%C3%A9");
    }

    #[test]
    fn lucene_escape_quotes() {
        assert_eq!(escape_lucene(r#"Say "Hi"\ "#), r#"Say \"Hi\"\\ "#);
    }

    #[test]
    fn parse_release_candidates_full_fixture() {
        let fixture = r#"{
            "count": 2,
            "releases": [
                {
                    "id": "11111111-1111-1111-1111-111111111111",
                    "title": "Album One",
                    "score": 100,
                    "date": "1997-03-01",
                    "artist-credit": [{ "name": "Artist X" }],
                    "media": [{ "track-count": 10 }, { "track-count": 2 }]
                },
                {
                    "id": "22222222-2222-2222-2222-222222222222",
                    "title": "Album One (reissue)",
                    "score": 78,
                    "artist-credit": [{ "name": "Artist X" }]
                }
            ]
        }"#;
        let v: Value = serde_json::from_str(fixture).unwrap();
        let got = parse_release_candidates(&v);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].mbid, "11111111-1111-1111-1111-111111111111");
        assert_eq!(got[0].title, "Album One");
        assert_eq!(got[0].artist, "Artist X");
        assert_eq!(got[0].date.as_deref(), Some("1997-03-01"));
        assert_eq!(got[0].score, 100);
        assert_eq!(got[0].track_count, Some(12));
        assert_eq!(got[1].score, 78);
        assert_eq!(got[1].date, None);
        assert_eq!(got[1].track_count, None);
    }

    #[test]
    fn parse_release_candidates_defensive() {
        // Non-object, missing key, missing id, wrong types: never panic.
        assert!(parse_release_candidates(&Value::Null).is_empty());
        assert!(parse_release_candidates(&serde_json::json!({})).is_empty());
        assert!(parse_release_candidates(&serde_json::json!({"releases": 3})).is_empty());
        let v = serde_json::json!({"releases": [
            {"title": "no id"},
            {"id": "", "title": "empty id"},
            {"id": "k", "score": "high", "media": "nope", "artist-credit": []}
        ]});
        let got = parse_release_candidates(&v);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].mbid, "k");
        assert_eq!(got[0].score, 0);
        assert_eq!(got[0].artist, "");
        assert_eq!(got[0].track_count, None);
    }

    #[test]
    fn ranking_prefers_score_then_track_count() {
        let mut c = vec![
            ReleaseCandidate { mbid: "a".into(), title: "".into(), artist: "".into(), date: None, score: 90, track_count: Some(20) },
            ReleaseCandidate { mbid: "b".into(), title: "".into(), artist: "".into(), date: None, score: 90, track_count: Some(10) },
            ReleaseCandidate { mbid: "c".into(), title: "".into(), artist: "".into(), date: None, score: 100, track_count: Some(99) },
        ];
        rank_candidates(&mut c, 10);
        assert_eq!(c.iter().map(|x| x.mbid.as_str()).collect::<Vec<_>>(), vec!["c", "b", "a"]);
    }

    #[test]
    fn rate_limiter_interval_from_config() {
        let rl = RateLimiter::new(1.0);
        assert!(rl.interval >= Duration::from_millis(999));
        let off = RateLimiter::new(0.0);
        assert_eq!(off.interval, Duration::ZERO);
    }

    #[test]
    fn duration_never_zero() {
        assert_eq!(duration_from_s(0), Duration::from_secs(1));
        assert_eq!(duration_from_s(7), Duration::from_secs(7));
    }

    /// A cache hit must be served without touching the network: the URL is an
    /// unroutable closed port, yet the value comes back from SQLite.
    #[test]
    fn cache_hit_avoids_network() {
        let db = Db::in_memory().unwrap();
        let url = "http://127.0.0.1:1/cached";
        let now = chrono::Utc::now().timestamp();
        db.cache_put("musicbrainz", url, r#"{"cached":true}"#, now, 3600).unwrap();
        let cfg = Services { request_timeout_s: 1, ..Services::default() };
        let http = HttpClient::new(&cfg).with_cache(db);
        let value = http.get_json("musicbrainz", url).expect("cache hit");
        assert_eq!(value["cached"], serde_json::json!(true));
    }

    /// Offline error mapping: connecting to a closed local port must fail
    /// fast with a named network error carrying a non-empty reason.
    #[test]
    fn connection_failure_maps_to_named_error() {
        let cfg = Services { request_timeout_s: 1, ..Services::default() };
        let http = HttpClient::new(&cfg);
        let err = http
            .get_text("musicbrainz", "http://127.0.0.1:1/")
            .expect_err("connection to closed port must fail");
        match err {
            MloError::Network { reason } => assert!(!reason.is_empty(), "reason empty"),
            MloError::Service { reason, .. } => assert!(!reason.is_empty(), "reason empty"),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}