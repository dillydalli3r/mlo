//! MP3 / ID3v2.4 tags via the `id3` crate. Unmapped keys round-trip through
//! `TXXX` frames (the Picard convention), so `MUSICBRAINZ_*`,
//! `RATEYOURMUSIC_*`, `DYNAMIC RANGE`, etc. survive intact.
//!
//! Writes are atomic: the file is copied to a sibling temp, re-tagged there,
//! then renamed over the original ([§1.3]).

use crate::error::{MloError, Result};
use crate::model::TagMap;
use id3::frame::{Comment as Id3Comment, ExtendedText, Lyrics as Id3Lyrics};
use id3::{Content, Frame, Tag, TagLike, Version};
use std::path::Path;

/// Canonical key -> ID3v2.4 frame id for fields with a dedicated frame.
const KEY_TO_ID3: &[(&str, &str)] = &[
    ("TITLE", "TIT2"),
    ("ARTIST", "TPE1"),
    ("ALBUM", "TALB"),
    ("ALBUMARTIST", "TPE2"),
    ("TRACKNUMBER", "TRCK"),
    ("DISCNUMBER", "TPOS"),
    ("DATE", "TDRC"),
    ("GENRE", "TCON"),
    ("LANGUAGE", "TLAN"),
    ("ISRC", "TSRC"),
    ("LABEL", "TPUB"),
    ("BPM", "TBPM"),
    ("INITIALKEY", "TKEY"),
    ("MOOD", "TMOO"),
    ("MOVEMENT", "MVNM"),
    ("COMPOSERSORT", "TSOC"),
    ("LYRICIST", "TEXT"),
    ("ENCODEDBY", "TENC"),
    ("ENCODER_PROGRAM", "TSSE"),
    ("MEDIA", "TMED"),
];

const ID3_TO_KEY: &[(&str, &str)] = &[
    ("TIT2", "TITLE"),
    ("TPE1", "ARTIST"),
    ("TALB", "ALBUM"),
    ("TPE2", "ALBUMARTIST"),
    ("TRCK", "TRACKNUMBER"),
    ("TPOS", "DISCNUMBER"),
    ("TDRC", "DATE"),
    ("TYER", "DATE"),
    ("TCON", "GENRE"),
    ("TLAN", "LANGUAGE"),
    ("TSRC", "ISRC"),
    ("TPUB", "LABEL"),
    ("TBPM", "BPM"),
    ("TKEY", "INITIALKEY"),
    ("TMOO", "MOOD"),
    ("MVNM", "MOVEMENT"),
    ("TSOC", "COMPOSERSORT"),
    ("TEXT", "LYRICIST"),
    ("TENC", "ENCODEDBY"),
    ("TSSE", "ENCODER_PROGRAM"),
    ("TMED", "MEDIA"),
];

fn id3_id_for(key: &str) -> Option<&'static str> {
    KEY_TO_ID3.iter().find(|(k, _)| *k == key).map(|(_, id)| *id)
}

fn key_for_id3_id(id: &str) -> String {
    ID3_TO_KEY
        .iter()
        .find(|(i, _)| *i == id)
        .map(|(_, k)| (*k).to_string())
        .unwrap_or_else(|| id.to_string())
}

pub fn read_tags(path: &Path) -> Result<TagMap> {
    let tag = match Tag::read_from_path(path) {
        Ok(t) => t,
        Err(id3::Error { kind: id3::ErrorKind::NoTag, .. }) => return Ok(TagMap::new()),
        Err(e) => {
            return Err(MloError::Tag {
                path: path.to_path_buf(),
                reason: format!("id3 read: {e}"),
            })
        }
    };
    let mut map = TagMap::new();
    for frame in tag.frames() {
        let id = frame.id().to_string();
        match frame.content() {
            Content::Text(t) => {
                // v2.4 allows null-separated multiple values
                for part in t.split('\u{0}') {
                    if !part.is_empty() {
                        map.entry(key_for_id3_id(&id)).or_default().push(part.to_string());
                    }
                }
            }
            Content::ExtendedText(ExtendedText { description, value }) => {
                let key = description.to_ascii_uppercase();
                if !key.is_empty() {
                    map.entry(key).or_default().push(value.clone());
                }
            }
            Content::Comment(Id3Comment { text, .. }) => {
                map.entry("COMMENT".into()).or_default().push(text.clone());
            }
            Content::Lyrics(Id3Lyrics { text, .. }) => {
                map.entry("UNSYNCEDLYRICS".into()).or_default().push(text.clone());
            }
            Content::Link(l) if !l.is_empty() => {
                map.entry(key_for_id3_id(&id)).or_default().push(l.clone());
            }
            Content::Picture(_) => {} // artwork handled as a cover sidecar, not a text tag
            Content::Unknown(u) => {
                map.entry(id.clone()).or_default().push(String::from_utf8_lossy(&u.data).into_owned());
            }
            _ => {}
        }
    }
    Ok(map)
}

pub fn write_tags(path: &Path, tags: &TagMap) -> Result<()> {
    let mut tag = Tag::new();
    for (key, values) in tags {
        let values: Vec<&String> = values.iter().filter(|v| !v.is_empty()).collect();
        if values.is_empty() {
            continue;
        }
        match key.as_str() {
            "COMMENT" => {
                tag.add_frame(Frame::with_content(
                    "COMM",
                    Content::Comment(Id3Comment {
                        lang: "eng".into(),
                        description: String::new(),
                        text: values[0].clone(),
                    }),
                ));
            }
            "LYRICS" | "UNSYNCEDLYRICS" => {
                tag.add_frame(Frame::with_content(
                    "USLT",
                    Content::Lyrics(Id3Lyrics {
                        lang: "eng".into(),
                        description: String::new(),
                        text: values.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"),
                    }),
                ));
            }
            _ => {
                if let Some(id) = id3_id_for(key) {
                    tag.set_text_values(id, values.iter().map(|s| s.as_str()));
                } else {
                    // arbitrary key -> TXXX(description=key)
                    for v in values {
                        tag.add_frame(Frame::with_content(
                            "TXXX",
                            Content::ExtendedText(ExtendedText {
                                description: key.clone(),
                                value: v.clone(),
                            }),
                        ));
                    }
                }
            }
        }
    }

    super::atomic_edit_copy(path, |tmp| {
        tag.write_to_path(tmp, Version::Id3v24)
            .map_err(|e| MloError::Tag { path: tmp.to_path_buf(), reason: format!("id3 write: {e}") })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_arbitrary_keys() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.mp3");
        // minimal fake mp3 body (not decoded in this test)
        std::fs::write(&f, vec![0xFFu8; 512]).unwrap();

        let mut tags = TagMap::new();
        tags.insert("TITLE".into(), vec!["T".into()]);
        tags.insert("ARTIST".into(), vec!["A".into(), "B".into()]);
        tags.insert("MUSICBRAINZ_TRACKID".into(), vec!["abc".into()]);
        tags.insert("DYNAMIC RANGE".into(), vec!["11".into()]);
        write_tags(&f, &tags).unwrap();

        let back = read_tags(&f).unwrap();
        assert_eq!(back["TITLE"], vec!["T"]);
        assert_eq!(back["ARTIST"], vec!["A", "B"]);
        assert_eq!(back["MUSICBRAINZ_TRACKID"], vec!["abc"]);
        assert_eq!(back["DYNAMIC RANGE"], vec!["11"]);
    }
}