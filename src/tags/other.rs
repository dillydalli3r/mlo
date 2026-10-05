//! MP4 / WAV / AIFF tags via `lofty`.
//!
//! These containers have no arbitrary-key convention the way Vorbis has, so the
//! app maps the shared field set and reports anything else as *unplaced*
//! ([§5.4]) rather than inventing a key. FLAC/Vorbis/Opus/MP3 get full fidelity
//! through the native paths in this module's siblings.

use crate::error::{IoResultExt, MloError, Result};
use crate::model::TagMap;
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::ItemKey;
use lofty::tag::{ItemValue, Tag, TagItem};
use std::fs::OpenOptions;
use std::path::Path;

/// Shared field set that survives the round trip on MP4/WAV/AIFF.
pub const MAPPED: &[(&str, ItemKey)] = &[
    ("TITLE", ItemKey::TrackTitle),
    ("ARTIST", ItemKey::TrackArtist),
    ("ALBUM", ItemKey::AlbumTitle),
    ("ALBUMARTIST", ItemKey::AlbumArtist),
    ("TRACKNUMBER", ItemKey::TrackNumber),
    ("DISCNUMBER", ItemKey::DiscNumber),
    ("DATE", ItemKey::Year),
    ("GENRE", ItemKey::Genre),
    ("COMMENT", ItemKey::Comment),
    ("BPM", ItemKey::Bpm),
    ("INITIALKEY", ItemKey::InitialKey),
    ("MOOD", ItemKey::Mood),
    ("ENCODER_PROGRAM", ItemKey::EncoderSoftware),
];

fn itemkey_for(key: &str) -> Option<ItemKey> {
    MAPPED.iter().find(|(k, _)| *k == key).map(|(_, i)| *i)
}

fn key_for_itemkey(ik: ItemKey) -> Option<&'static str> {
    MAPPED.iter().find(|(_, i)| *i == ik).map(|(k, _)| *k)
}

fn open_tagged(path: &Path) -> Result<lofty::file::TaggedFile> {
    lofty::read_from_path(path).map_err(|e| MloError::Tag {
        path: path.to_path_buf(),
        reason: format!("lofty read: {e}"),
    })
}

pub fn read_tags(path: &Path) -> Result<TagMap> {
    let tagged = open_tagged(path)?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let Some(tag) = tag else { return Ok(TagMap::new()) };
    let mut map = TagMap::new();
    for item in tag.items() {
        if let Some(key) = key_for_itemkey(item.key()) {
            if let ItemValue::Text(t) = item.value() {
                map.entry(key.to_string()).or_default().push(t.clone());
            }
        } else if let Some(fmt_key) = item.key().map_key(tag.tag_type()) {
            // Only accept clean ASCII keys (skip MP4 atoms like "©nam").
            if fmt_key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' ') {
                if let ItemValue::Text(t) = item.value() {
                    map.entry(fmt_key.to_ascii_uppercase()).or_default().push(t.clone());
                }
            }
        }
    }
    Ok(map)
}

/// Returns the keys that could not be represented in this container.
pub fn write_tags(path: &Path, tags: &TagMap) -> Result<Vec<String>> {
    let tmp = super::temp_sibling(path)?;
    std::fs::copy(path, &tmp).at(&tmp)?;
    let result = (|| -> Result<Vec<String>> {
        let mut tagged = open_tagged(&tmp)?;
        if tagged.first_tag().is_none() {
            let tt = tagged.primary_tag_type();
            tagged.insert_tag(Tag::new(tt));
        }
        let tag = tagged
            .first_tag_mut()
            .ok_or_else(|| MloError::Tag { path: tmp.clone(), reason: "could not create a tag".into() })?;

        let mut unplaced = Vec::new();
        for (key, values) in tags {
            match itemkey_for(key) {
                Some(ik) => {
                    tag.remove_key(ik);
                    for v in values.iter().filter(|v| !v.is_empty()) {
                        tag.push(TagItem::new(ik, ItemValue::Text(v.clone())));
                    }
                }
                None => unplaced.push(key.clone()),
            }
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&tmp)
            .at(&tmp)?;
        tagged
            .save_to(&mut file, WriteOptions::default())
            .map_err(|e| MloError::Tag { path: tmp.clone(), reason: format!("lofty save: {e}") })?;
        Ok(unplaced)
    })();
    match result {
        Ok(unplaced) => {
            std::fs::rename(&tmp, path).at(path)?;
            Ok(unplaced)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Raw bytes of the embedded front cover, if any (used as a cover fallback).
pub fn embedded_cover(path: &Path) -> Result<Option<Vec<u8>>> {
    let tagged = open_tagged(path)?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    if let Some(tag) = tag {
        if let Some(pic) = tag.pictures().first() {
            return Ok(Some(pic.data().to_vec()));
        }
    }
    Ok(None)
}