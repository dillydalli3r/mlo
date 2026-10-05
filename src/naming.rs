//! Naming template ([§3.2]) — a Picard-style naming script engine matching
//! `mlo/naming.py`. Literal text, `%field%` substitutions, `$func(...)` calls,
//! and `/` as the folder separator. `[`/`]` and `{`/`{` are ordinary literal
//! characters; the empty groups a skipped conditional leaves behind are
//! removed by [`sanitize_path`].
//!
//! Deterministic: one album yields exactly one path, always ([§1.5]).

use std::sync::LazyLock;

use regex::Regex;

use crate::error::Result;
use crate::model::TagMap;

/// The shipped naming script, verbatim from `mlo/naming.py::DEFAULT_NAMING_SCRIPT`
/// (concatenated in the same fragments, so the bytes are identical).
pub const DEFAULT_NAMING_SCRIPT: &str = concat!(
    "%albumartist% [%musicbrainz_albumartistid%]/",
    "$if(%releasetype%,[%releasetype%] ,)",
    "$if(%originaldate%,%originaldate% - ,)",
    "$if(%date%,%date% - ,)",
    "%album% {$if(%releasecountry%,%releasecountry%)",
    "$if(%media%,$if(%releasecountry%, - ,)%media%)",
    "$if(%catalognumber%,$if(%media%, - ,$if(%releasecountry%, - ,))%catalognumber%)}",
    "$if(%label%, [%label%])",
    "$if(%musicbrainz_albumid%, [%musicbrainz_albumid%])",
    "$if(%musicbrainz_releasegroupid%, [%musicbrainz_releasegroupid%])/",
    "%discnumber%-$num(%tracknumber%,2) %title%",
    "$if(%musicbrainz_trackid%, [%musicbrainz_trackid%])",
    "$if(%musicbrainz_releasegroupid%, [%musicbrainz_releasegroupid%])",
);

/// Backwards-compatible alias for callers that name the shipped template
/// `DEFAULT_TEMPLATE` (e.g. `config.rs`).
pub const DEFAULT_TEMPLATE: &str = DEFAULT_NAMING_SCRIPT;

/// Hard cap for the artist-image size ceiling ([§4.4]).
pub const ARTIST_IMAGE_CEILING: u32 = 2000;

#[derive(Debug, Clone, Copy, Default)]
pub struct NamingOptions {
    pub short_folder_names: bool,
}

#[derive(Debug, Clone, Default)]
pub struct EvalResult {
    /// DIRECTORY relative path (components WITHOUT the file-name segment).
    pub path: String,
    /// Folder + file name, `/`-joined.
    pub full_path: String,
    /// The final component, when the script names one.
    pub file_name: Option<String>,
    /// Pre-sanitisation evaluation (diagnostics only).
    pub raw: String,
    /// Fields referenced but absent/blank, in order of appearance.
    pub missing: Vec<String>,
    /// Every non-empty segment, including the file name.
    pub components: Vec<String>,
}

impl EvalResult {
    pub fn has_missing(&self) -> bool {
        !self.missing.is_empty()
    }
}

/// Map a template field to its tag key.
pub fn field_tag(name: &str) -> String {
    name.to_ascii_uppercase()
}

#[derive(Debug, Clone)]
enum Node {
    Lit(String),
    Var(String),
    Func(String, Vec<Vec<Node>>),
}

/// Token list for a script: literal text, `%field%` substitutions and
/// `$func(...)` calls with recursively compiled arguments. Mirrors
/// `mlo/naming.py::_compile` (`[`/`]`/`{`/`}` are literal characters).
fn compile(script: &str) -> Vec<Node> {
    let chars: Vec<char> = script.chars().collect();
    let n = chars.len();
    let mut nodes = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        if ch == '%' {
            match chars[i + 1..].iter().position(|&c| c == '%') {
                Some(rel) => {
                    let j = i + 1 + rel;
                    flush(&mut buf, &mut nodes);
                    nodes.push(Node::Var(chars[i + 1..j].iter().collect()));
                    i = j + 1;
                }
                None => {
                    buf.push(ch);
                    i += 1;
                }
            }
        } else if ch == '$' {
            let mut j = i + 1;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            if j > i + 1 && j < n && chars[j] == '(' {
                let name: String = chars[i + 1..j].iter().collect();
                let (body, end) = find_balanced(&chars, j);
                flush(&mut buf, &mut nodes);
                let args = split_args(&body);
                let compiled = args.iter().map(|a| compile(a)).collect();
                nodes.push(Node::Func(name, compiled));
                i = end;
            } else {
                buf.push(ch);
                i += 1;
            }
        } else {
            buf.push(ch);
            i += 1;
        }
    }
    flush(&mut buf, &mut nodes);
    nodes
}

fn flush(buf: &mut String, nodes: &mut Vec<Node>) {
    if !buf.is_empty() {
        nodes.push(Node::Lit(std::mem::take(buf)));
    }
}

/// Content and index-after-close for the parens starting at `start` (which
/// must point at `(`). Handles nesting; mirrors `naming.py::_find_balanced`.
fn find_balanced(chars: &[char], start: usize) -> (String, usize) {
    let mut depth = 0usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                if depth == 1 {
                    return (chars[start + 1..i].iter().collect(), i + 1);
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }
    (chars[start + 1..].iter().collect(), chars.len())
}

/// Split on top-level commas; whitespace is significant, so arguments are not
/// stripped. Mirrors `naming.py::_split_args`.
fn split_args(argtext: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in argtext.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => args.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    args.push(cur);
    args
}

/// `record` = whether absent fields at this level are reported as missing tags.
/// Only plain substitutions (and the taken branch of an `$if`) are reported.
fn eval_seq(nodes: &[Node], tags: &TagMap, record: bool, missing: &mut Vec<String>) -> String {
    let mut out = String::new();
    for n in nodes {
        match n {
            Node::Lit(t) => out.push_str(t),
            Node::Var(name) => {
                let key = field_tag(name);
                match value_of(tags, &key) {
                    // Every substituted TAG VALUE is a name fragment and goes
                    // through the one rule — mirroring `naming.py::_run`.
                    Some(v) => out.push_str(&sanitize_segment(&v)),
                    None => {
                        if record && !missing.iter().any(|m| m == &key) {
                            missing.push(key);
                        }
                    }
                }
            }
            Node::Func(name, args) => {
                out.push_str(&eval_func(name, args, tags, record, missing));
            }
        }
    }
    out
}

fn arg(
    args: &[Vec<Node>],
    i: usize,
    tags: &TagMap,
    record: bool,
    missing: &mut Vec<String>,
) -> String {
    match args.get(i) {
        Some(a) => eval_seq(a, tags, record, missing),
        None => String::new(),
    }
}

fn is_truthy(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t != "0"
}

/// `int(float(s))`, clamped at zero — mirrors the `naming.py` argument parsing.
fn int_of(s: &str) -> usize {
    match s.trim().parse::<f64>() {
        Ok(f) if f.is_finite() && f > 0.0 => f as usize,
        _ => 0,
    }
}

/// The first run of decimal digits anywhere in `s` (naming.py's `$num` scan).
fn first_digit_run(s: &str) -> String {
    let mut out = String::new();
    let mut started = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            out.push(ch);
            started = true;
        } else if started {
            break;
        }
    }
    out
}

/// The `$functions`, in the order `naming.py::FUNCTIONS` lists them.
fn eval_func(
    name: &str,
    args: &[Vec<Node>],
    tags: &TagMap,
    record: bool,
    missing: &mut Vec<String>,
) -> String {
    match name {
        "if" => {
            let cond = arg(args, 0, tags, false, missing);
            if is_truthy(&cond) {
                arg(args, 1, tags, record, missing)
            } else {
                arg(args, 2, tags, record, missing)
            }
        }
        "eq" => {
            if args.len() < 2 {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            let b = arg(args, 1, tags, false, missing);
            if a == b { "1".into() } else { String::new() }
        }
        "ne" => {
            if args.len() < 2 {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            let b = arg(args, 1, tags, false, missing);
            if a != b { "1".into() } else { String::new() }
        }
        "not" => {
            if args.is_empty() {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            if is_truthy(&a) { String::new() } else { "1".into() }
        }
        "and" => {
            if args.len() < 2 {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            if !is_truthy(&a) { a } else { arg(args, 1, tags, false, missing) }
        }
        "or" => {
            if args.len() < 2 {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            if is_truthy(&a) { a } else { arg(args, 1, tags, false, missing) }
        }
        "left" => {
            if args.len() < 2 {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            let n = int_of(&arg(args, 1, tags, false, missing));
            a.chars().take(n).collect()
        }
        "right" => {
            if args.len() < 2 {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            let n = int_of(&arg(args, 1, tags, false, missing));
            if n == 0 {
                String::new()
            } else {
                let mut tail: Vec<char> = a.chars().rev().take(n).collect();
                tail.reverse();
                tail.into_iter().collect()
            }
        }
        "num" => {
            if args.len() < 2 {
                return String::new();
            }
            let v = arg(args, 0, tags, false, missing);
            let width = int_of(&arg(args, 1, tags, false, missing));
            let digits = first_digit_run(&v);
            // la-musica: `str(int(digits or 0)).zfill(n)` — a missing field is "0".
            let n: u128 = if digits.is_empty() { 0 } else { digits.parse().unwrap_or(0) };
            format!("{n:0>width$}")
        }
        "lower" => {
            if args.is_empty() {
                return String::new();
            }
            arg(args, 0, tags, false, missing).to_lowercase()
        }
        "upper" => {
            if args.is_empty() {
                return String::new();
            }
            arg(args, 0, tags, false, missing).to_uppercase()
        }
        "replace" => {
            if args.len() < 3 {
                return String::new();
            }
            let a = arg(args, 0, tags, false, missing);
            let from = arg(args, 1, tags, false, missing);
            let to = arg(args, 2, tags, false, missing);
            a.replace(&from, &to)
        }
        _ => String::new(),
    }
}

/// The tag value for `key`, or `None` when absent/blank.
///
/// Multi-value tags join with `"; "`. `RELEASECOUNTRY` (falling back to
/// `COUNTRY`) and `LABEL` reduce to their FIRST value (`_first_multi`, R33).
fn value_of(tags: &TagMap, key: &str) -> Option<String> {
    if key == "RELEASECOUNTRY" {
        let v = joined(tags, "RELEASECOUNTRY").or_else(|| joined(tags, "COUNTRY"))?;
        let first = first_multi(&v);
        return if first.is_empty() { None } else { Some(first) };
    }
    if key == "LABEL" {
        let v = joined(tags, "LABEL")?;
        let first = first_multi(&v);
        return if first.is_empty() { None } else { Some(first) };
    }
    joined(tags, key)
}

fn joined(tags: &TagMap, key: &str) -> Option<String> {
    let vals = tags.get(key)?;
    let parts: Vec<&str> = vals
        .iter()
        .map(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

static FIRST_MULTI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*[;+]\s*|\s+/\s+")
        .expect("static RELEASECOUNTRY/LABEL separator regex is valid")
});

/// First non-empty entry of a multi-value tag (`naming.py::_first_multi`).
fn first_multi(value: &str) -> String {
    FIRST_MULTI_RE
        .split(value)
        .map(|p| p.trim())
        .find(|p| !p.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Evaluate a template against an album's tags.
pub fn evaluate(template: &str, tags: &TagMap, opts: NamingOptions) -> Result<EvalResult> {
    let nodes = compile(template);
    let mut missing = Vec::new();
    let raw = eval_seq(&nodes, tags, true, &mut missing);

    // `eval_script`: shorten ids, drop empty `[]`/`{}` groups, then sanitise
    // each `/`-separated segment and drop the empties.
    let shortened = if opts.short_folder_names {
        shorten_uuids(&raw)
    } else {
        raw.clone()
    };
    let cleaned = EMPTY_GROUP_RE.replace_all(&shortened, "");

    let mut components: Vec<String> = Vec::new();
    for seg in cleaned.split('/') {
        let s = sanitize_segment(seg);
        if !s.is_empty() {
            components.push(s);
        }
    }

    let full_path = components.join("/");
    let file_name = components.last().cloned();
    let path = {
        let mut dirs = components.clone();
        dirs.pop();
        dirs.join("/")
    };

    Ok(EvalResult {
        path,
        full_path,
        file_name,
        raw,
        missing,
        components,
    })
}

static EMPTY_GROUP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*\[\s*\]|\s*\{\s*\}").expect("static empty-group regex is valid")
});

/// One file or folder name — `naming.py::sanitize_segment`.
///
/// Illegal characters become `_`, runs of whitespace collapse to a single
/// space, a trailing `[ .]+` run becomes the same number of `_`, and a Windows
/// reserved device name gets `_` appended to its stem.
pub fn sanitize_segment(name: &str) -> String {
    let text = ILLEGAL_RE.replace_all(name, "_");
    let text = WS_RE.replace_all(&text, " ");
    if text.trim_matches(' ').is_empty() {
        return String::new();
    }
    let text = text.trim_start_matches(' ');
    // Trailing dot/space, one for one ("." -> "_", ".." -> "__").
    let count = text.chars().rev().take_while(|&c| c == ' ' || c == '.').count();
    let text: String = if count == 0 {
        text.to_string()
    } else {
        let keep = text.chars().count() - count;
        let mut t: String = text.chars().take(keep).collect();
        for _ in 0..count {
            t.push('_');
        }
        t
    };
    // A reserved device name gets a trailing "_" on its stem: "AUX.mp3" ->
    // "AUX_.mp3", still readable and no longer reserved.
    if let Some(c) = RESERVED_RE.captures(&text) {
        let stem = c.get(1).map(|m| m.as_str()).unwrap_or("");
        let rest = c.get(2).map(|m| m.as_str()).unwrap_or("");
        return format!("{stem}_{rest}");
    }
    text
}

/// A whole RELATIVE path — `naming.py::sanitize_path`: empty `[]`/`{}` groups
/// vanish together with the space before them, then each `/`-separated segment
/// is sanitised and the empties dropped.
pub fn sanitize_path(text: &str) -> String {
    let cleaned = EMPTY_GROUP_RE.replace_all(text, "");
    cleaned
        .split('/')
        .map(sanitize_segment)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Kept as a thin alias: older callers named the same rule `sanitize_component`.
pub fn sanitize_component(s: &str) -> String {
    sanitize_segment(s)
}

static ILLEGAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[<>:"/\\|?*\x01-\x1f]"#).expect("static illegal-character regex is valid")
});

static WS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("static whitespace-run regex is valid"));

static RESERVED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\..*)?$")
        .expect("static reserved-device-name regex is valid")
});

/// `short_folder_names`: truncate each UUID group to its first 8 chars.
pub fn shorten_uuids(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_hexdigit() && is_uuid_at(b, i) {
            out.push_str(&s[i..i + 8]);
            i += 36;
        } else {
            let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// True when 36 bytes starting at `i` form `hex{8}-hex{4}-hex{4}-hex{4}-hex{12}`.
fn is_uuid_at(b: &[u8], i: usize) -> bool {
    const DASHES: [usize; 4] = [8, 13, 18, 23];
    if i + 36 > b.len() {
        return false;
    }
    (0..36).all(|k| {
        if DASHES.contains(&k) {
            b[i + k] == b'-'
        } else {
            b[i + k].is_ascii_hexdigit()
        }
    })
}

/// Lowercase file extension per [§3.2] rule 3.
pub fn lowercase_ext(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e == e.to_lowercase())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(entries: &[(&str, &[&str])]) -> TagMap {
        let mut m = TagMap::new();
        for (k, vs) in entries {
            m.insert((*k).to_string(), vs.iter().map(|s| s.to_string()).collect());
        }
        m
    }

    /// A full tag set (all fields the shipped script reads are present).
    fn full_tags() -> TagMap {
        t(&[
            ("ALBUMARTIST", &["Radiohead"]),
            ("ALBUM", &["Kid A"]),
            ("DATE", &["2000-10-02"]),
            ("ORIGINALDATE", &["2000"]),
            ("RELEASETYPE", &["Album"]),
            ("MEDIA", &["CD"]),
            ("CATALOGNUMBER", &["CDP 7243 5 27753 2 5"]),
            ("LABEL", &["Parlophone"]),
            ("RELEASECOUNTRY", &["GB"]),
            ("DISCNUMBER", &["1"]),
            ("TRACKNUMBER", &["7"]),
            ("TITLE", &["Idioteque"]),
            ("MUSICBRAINZ_ALBUMARTISTID", &["a74b1b7f-71a5-4011-9441-d0b5e4122711"]),
            ("MUSICBRAINZ_ALBUMID", &["b1392450-e666-3926-a536-22c65f834433"]),
            ("MUSICBRAINZ_RELEASEGROUPID", &["cd12a2b3-0000-0000-0000-000000000000"]),
            ("MUSICBRAINZ_TRACKID", &["0f9b1d4e-1111-2222-3333-444455556666"]),
        ])
    }

    const ARTIST_FOLDER: &str = "Radiohead [a74b1b7f-71a5-4011-9441-d0b5e4122711]";
    const ALBUM_FOLDER: &str = "[Album] 2000 - 2000-10-02 - Kid A {GB - CD - CDP 7243 5 27753 2 5} [Parlophone] [b1392450-e666-3926-a536-22c65f834433] [cd12a2b3-0000-0000-0000-000000000000]";
    const FILE_NAME: &str = "1-07 Idioteque [0f9b1d4e-1111-2222-3333-444455556666] [cd12a2b3-0000-0000-0000-000000000000]";

    #[test]
    fn shipped_script_full_tag_path_is_exact() {
        let r = evaluate(DEFAULT_NAMING_SCRIPT, &full_tags(), NamingOptions::default()).unwrap();
        let expected_full = format!("{ARTIST_FOLDER}/{ALBUM_FOLDER}/{FILE_NAME}");
        assert_eq!(r.full_path, expected_full);
        assert_eq!(r.path, format!("{ARTIST_FOLDER}/{ALBUM_FOLDER}"));
        assert_eq!(r.file_name.as_deref(), Some(FILE_NAME));
        assert_eq!(r.components.len(), 3);
        assert!(!r.has_missing());
    }

    #[test]
    fn default_template_alias_matches() {
        assert_eq!(DEFAULT_TEMPLATE, DEFAULT_NAMING_SCRIPT);
    }

    #[test]
    fn short_folder_names_truncates_uuids_to_eight() {
        let r = evaluate(
            DEFAULT_NAMING_SCRIPT,
            &full_tags(),
            NamingOptions { short_folder_names: true },
        )
        .unwrap();
        assert_eq!(
            r.full_path,
            "Radiohead [a74b1b7f]/[Album] 2000 - 2000-10-02 - Kid A {GB - CD - CDP 7243 5 27753 2 5} [Parlophone] [b1392450] [cd12a2b3]/1-07 Idioteque [0f9b1d4e] [cd12a2b3]"
        );
    }

    #[test]
    fn sanitize_trailing_dot_and_reserved_device_name() {
        assert_eq!(sanitize_segment("trailing."), "trailing_");
        assert_eq!(sanitize_segment("trailing. "), "trailing__");
        assert_eq!(sanitize_segment("AUX.mp3"), "AUX_.mp3");
        assert_eq!(sanitize_segment("con"), "con_");
        assert_eq!(sanitize_segment("LPT9.flac"), "LPT9_.flac");
        assert_eq!(sanitize_segment("COM0.mp3"), "COM0.mp3");
    }

    #[test]
    fn empty_bracket_groups_vanish_with_preceding_space() {
        assert_eq!(sanitize_path("Artist []/X {}"), "Artist/X");
        let r = evaluate(
            "%albumartist% [%musicbrainz_albumartistid%]/%album%",
            &t(&[("ALBUMARTIST", &["Radiohead"]), ("ALBUM", &["Kid A"])]),
            NamingOptions::default(),
        )
        .unwrap();
        assert_eq!(r.full_path, "Radiohead/Kid A");
    }

    #[test]
    fn num_pads_and_missing_is_zero_padded() {
        let r = evaluate(
            "$num(%tracknumber%,2) %title%",
            &t(&[("TRACKNUMBER", &["7"]), ("TITLE", &["Idioteque"])]),
            NamingOptions::default(),
        )
        .unwrap();
        assert_eq!(r.file_name.as_deref(), Some("07 Idioteque"));

        // la-musica: `str(int(digits or 0)).zfill(n)` — missing is "0" padded
        let r = evaluate(
            "$num(%tracknumber%,2) %title%",
            &t(&[("TITLE", &["Idioteque"])]),
            NamingOptions::default(),
        )
        .unwrap();
        assert_eq!(r.file_name.as_deref(), Some("00 Idioteque"));
    }

    #[test]
    fn first_multi_reduces_country_and_label() {
        let r = evaluate(
            "%releasecountry%/%label%",
            &t(&[
                ("RELEASECOUNTRY", &["US", "GB"]),
                ("LABEL", &["Label A", "Label B"]),
            ]),
            NamingOptions::default(),
        )
        .unwrap();
        assert_eq!(r.full_path, "US/Label A");

        // a single value spelling several entries still reduces to the first
        let r = evaluate(
            "%releasecountry%",
            &t(&[("RELEASECOUNTRY", &["US / UK"])]),
            NamingOptions::default(),
        )
        .unwrap();
        assert_eq!(r.full_path, "US");
    }

    #[test]
    fn missing_plain_field_reported_not_invented() {
        let mut tags = full_tags();
        tags.remove("ALBUM");
        let r = evaluate(DEFAULT_NAMING_SCRIPT, &tags, NamingOptions::default()).unwrap();
        assert!(!r.full_path.contains("Kid A"));
        assert!(r.missing.contains(&"ALBUM".to_string()));

        // absence inside `$if`/`$num` is handled, not reported as missing
        let mut tags2 = full_tags();
        tags2.remove("ORIGINALDATE");
        tags2.remove("TRACKNUMBER");
        let r2 = evaluate(DEFAULT_NAMING_SCRIPT, &tags2, NamingOptions::default()).unwrap();
        assert!(!r2.missing.contains(&"ORIGINALDATE".to_string()));
        assert!(!r2.missing.contains(&"TRACKNUMBER".to_string()));
    }

    #[test]
    fn illegal_chars_and_whitespace_collapse() {
        assert_eq!(sanitize_segment("a:b?c"), "a_b_c");
        assert_eq!(sanitize_segment("a  b"), "a b");
        assert_eq!(sanitize_segment("AC/DC"), "AC_DC");
        assert_eq!(sanitize_segment("   "), "");
        assert_eq!(sanitize_segment("<<>>"), "____");
    }
}