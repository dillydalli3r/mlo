//! Vorbis comment payload ([§6.1] FLAC / Ogg Vorbis / Opus).
//!
//! Layout (no container prefix): `vendor_len u32le | vendor | count u32le |
//! (len u32le | "KEY=VALUE")*`. Unknown keys are preserved verbatim — this is
//! why the app does not route Vorbis through a library that drops them.

use crate::error::{MloError, Result};
use crate::model::TagMap;

#[derive(Debug, Clone, Default)]
pub struct VorbisComment {
    pub vendor: String,
    pub entries: Vec<(String, String)>,
}

impl VorbisComment {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut pos = 0usize;
        let vendor_len = read_u32(payload, &mut pos)? as usize;
        let vendor = read_bytes(payload, &mut pos, vendor_len)?;
        let vendor = String::from_utf8_lossy(vendor).into_owned();
        let count = read_u32(payload, &mut pos)? as usize;
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let len = read_u32(payload, &mut pos)? as usize;
            let raw = read_bytes(payload, &mut pos, len)?;
            let s = String::from_utf8_lossy(raw);
            if let Some((k, v)) = s.split_once('=') {
                entries.push((k.trim().to_string(), v.to_string()));
            } else if !s.trim().is_empty() {
                entries.push((s.trim().to_string(), String::new()));
            }
        }
        Ok(Self { vendor, entries })
    }

    /// Serialise the payload (without container prefix/framing).
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.entries.iter().map(|(k, v)| k.len() + v.len() + 5).sum::<usize>());
        let vendor = self.vendor.as_bytes();
        out.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        out.extend_from_slice(vendor);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (k, v) in &self.entries {
            let entry = format!("{k}={v}");
            let b = entry.as_bytes();
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(b);
        }
        out
    }

    pub fn to_tagmap(&self) -> TagMap {
        let mut map = TagMap::new();
        for (k, v) in &self.entries {
            let key = k.to_ascii_uppercase();
            map.entry(key).or_default().push(v.clone());
        }
        map
    }

    pub fn from_tagmap(tags: &TagMap, vendor: &str) -> Self {
        let mut entries = Vec::new();
        for (k, vals) in tags {
            for v in vals {
                entries.push((k.clone(), v.clone()));
            }
        }
        Self { vendor: vendor.to_string(), entries }
    }

    /// Merge: complete existing values, never truncate ([§6.3]).
    pub fn merge(&mut self, tags: &TagMap) {
        for (k, vals) in tags {
            for v in vals {
                let exists = self
                    .entries
                    .iter()
                    .any(|(ek, ev)| ek.eq_ignore_ascii_case(k) && ev == v);
                if !exists {
                    self.entries.push((k.clone(), v.clone()));
                }
            }
        }
    }
}

pub fn read_u32(buf: &[u8], pos: &mut usize) -> Result<u32> {
    if *pos + 4 > buf.len() {
        return Err(MloError::Other("truncated vorbis comment".into()));
    }
    let v = u32::from_le_bytes([buf[*pos], buf[*pos + 1], buf[*pos + 2], buf[*pos + 3]]);
    *pos += 4;
    Ok(v)
}

pub fn read_bytes<'a>(buf: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8]> {
    if *pos + len > buf.len() {
        return Err(MloError::Other("truncated vorbis comment".into()));
    }
    let s = &buf[*pos..*pos + len];
    *pos += len;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_unknown_keys() {
        let mut tags = TagMap::new();
        tags.insert("TITLE".into(), vec!["X".into()]);
        tags.insert("DYNAMIC RANGE".into(), vec!["9".into()]);
        tags.insert("RATEYOURMUSIC_ALBUM".into(), vec!["3.9".into()]);
        let vc = VorbisComment::from_tagmap(&tags, "mlo");
        let bytes = vc.serialize();
        let back = VorbisComment::parse(&bytes).unwrap();
        assert_eq!(back.vendor, "mlo");
        let map = back.to_tagmap();
        assert_eq!(map["DYNAMIC RANGE"], vec!["9"]);
        assert_eq!(map["RATEYOURMUSIC_ALBUM"], vec!["3.9"]);
    }

    #[test]
    fn duplicate_values_accumulate() {
        let mut vc = VorbisComment::default();
        let mut t = TagMap::new();
        t.insert("GENRE".into(), vec!["Rock".into(), "Jazz".into()]);
        vc.merge(&t);
        vc.merge(&t); // idempotent
        assert_eq!(vc.entries.len(), 2);
    }
}