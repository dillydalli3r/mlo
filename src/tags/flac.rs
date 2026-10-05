//! Native FLAC metadata (`fLaC`) read/write — Vorbis comments plus all other
//! metadata blocks preserved verbatim. Writes are atomic ([§1.3]): temp file
//! beside the destination, fsync, rename; audio is stream-copied (no full-file
//! buffer).

use super::vorbis::VorbisComment;
use crate::atomic;
use crate::error::{IoResultExt, MloError, Result};
use crate::model::TagMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAGIC: &[u8; 4] = b"fLaC";
const BLOCK_VORBIS_COMMENT: u8 = 4;
const BLOCK_STREAMINFO: u8 = 0;

#[derive(Debug)]
struct FlacLayout {
    /// Optional ID3v2 prefix preserved verbatim (some taggers prepend one).
    prefix: Vec<u8>,
    /// All metadata blocks in order (type, payload).
    blocks: Vec<(u8, Vec<u8>)>,
    /// Byte offset where audio frames begin.
    audio_offset: u64,
}

fn parse_layout(path: &Path) -> Result<FlacLayout> {
    let file = File::open(path).at(path)?;
    let mut r = BufReader::new(file);

    let mut prefix = Vec::new();
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).at(path)?;
    if &magic[0..3] == b"ID3" {
        // full 10-byte ID3v2 header: magic(3) ver(2) flags(1) syncsafe size(4)
        let mut tail = [0u8; 6];
        r.read_exact(&mut tail).at(path)?;
        let size = ((tail[2] as u32 & 0x7f) << 21)
            | ((tail[3] as u32 & 0x7f) << 14)
            | ((tail[4] as u32 & 0x7f) << 7)
            | (tail[5] as u32 & 0x7f);
        prefix.extend_from_slice(&magic);
        prefix.extend_from_slice(&tail);
        let mut id3 = vec![0u8; size as usize];
        r.read_exact(&mut id3).at(path)?;
        prefix.extend_from_slice(&id3);
        r.read_exact(&mut magic).at(path)?;
    }
    if &magic != MAGIC {
        return Err(MloError::UnsupportedContainer {
            path: path.to_path_buf(),
            container: "flac".into(),
            reason: "missing fLaC magic".into(),
        });
    }

    let mut blocks = Vec::new();
    loop {
        let mut hdr = [0u8; 4];
        r.read_exact(&mut hdr).at(path)?;
        let last = hdr[0] & 0x80 != 0;
        let btype = hdr[0] & 0x7f;
        let len = ((hdr[1] as usize) << 16) | ((hdr[2] as usize) << 8) | hdr[3] as usize;
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload).at(path)?;
        blocks.push((btype, payload));
        if last {
            break;
        }
        if blocks.len() > 1024 {
            return Err(MloError::Other("flac: too many metadata blocks".into()));
        }
    }
    let audio_offset = r.stream_position().at(path)?;
    Ok(FlacLayout { prefix, blocks, audio_offset })
}

fn comment_index(layout: &FlacLayout) -> Option<usize> {
    layout.blocks.iter().position(|(t, _)| *t == BLOCK_VORBIS_COMMENT)
}

pub fn read_tags(path: &Path) -> Result<TagMap> {
    let layout = parse_layout(path)?;
    match comment_index(&layout) {
        Some(i) => Ok(VorbisComment::parse(&layout.blocks[i].1)?.to_tagmap()),
        None => Ok(TagMap::new()),
    }
}

pub fn read_comment(path: &Path) -> Result<VorbisComment> {
    let layout = parse_layout(path)?;
    match comment_index(&layout) {
        Some(i) => VorbisComment::parse(&layout.blocks[i].1),
        None => Ok(VorbisComment::default()),
    }
}

/// Replace the VORBIS_COMMENT block (inserting it after STREAMINFO when absent).
pub fn write_tags(path: &Path, tags: &TagMap) -> Result<()> {
    let layout = parse_layout(path)?;
    let mut vc = match comment_index(&layout) {
        Some(i) => VorbisComment::parse(&layout.blocks[i].1)?,
        None => VorbisComment::default(),
    };
    vc.vendor = format!("mlo {}", crate::version());
    vc.entries = VorbisComment::from_tagmap(tags, &vc.vendor).entries;

    let payload = vc.serialize();
    if payload.len() > 0x00FF_FFFF {
        return Err(MloError::Invalid("vorbis comment exceeds 16 MiB FLAC block limit".into()));
    }

    let mut blocks = layout.blocks.clone();
    match comment_index(&layout) {
        Some(i) => blocks[i] = (BLOCK_VORBIS_COMMENT, payload),
        None => {
            let insert_at = blocks
                .iter()
                .position(|(t, _)| *t != BLOCK_STREAMINFO)
                .unwrap_or(blocks.len());
            blocks.insert(insert_at, (BLOCK_VORBIS_COMMENT, payload));
        }
    }

    // Serialise metadata headers + payloads.
    let mut meta = Vec::new();
    for (i, (t, p)) in blocks.iter().enumerate() {
        let last = i + 1 == blocks.len();
        meta.push(if last { t | 0x80 } else { *t });
        meta.push((p.len() >> 16) as u8);
        meta.push((p.len() >> 8) as u8);
        meta.push(p.len() as u8);
        meta.extend_from_slice(p);
    }

    atomic::write_atomic_with(path, |out| -> std::io::Result<()> {
        out.write_all(&layout.prefix)?;
        out.write_all(MAGIC)?;
        out.write_all(&meta)?;
        // stream-copy audio frames
        let mut src = File::open(path)?;
        src.seek(SeekFrom::Start(layout.audio_offset))?;
        std::io::copy(&mut src, out)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid FLAC: fLaC + STREAMINFO(34) + VORBIS_COMMENT + no audio.
    fn tiny_flac(comment: Option<&TagMap>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        // STREAMINFO block (type 0, len 34), not last when a comment follows
        let mut si = vec![0u8; 34];
        // sample rate 44100 in top 20 bits of bytes 10..14 for realism (unused here)
        si[10] = 0x0a;
        out.push(if comment.is_some() { 0 } else { 0x80 });
        out.push(0);
        out.push(0);
        out.push(34);
        out.extend_from_slice(&si);
        if let Some(tags) = comment {
            let vc = VorbisComment::from_tagmap(tags, "test");
            let p = vc.serialize();
            out.push(0x80 | BLOCK_VORBIS_COMMENT);
            out.push((p.len() >> 16) as u8);
            out.push((p.len() >> 8) as u8);
            out.push(p.len() as u8);
            out.extend_from_slice(&p);
        }
        out
    }

    #[test]
    fn read_write_roundtrip_no_audio() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.flac");
        std::fs::write(&f, tiny_flac(None)).unwrap();

        let mut tags = TagMap::new();
        tags.insert("TITLE".into(), vec!["Hello".into()]);
        tags.insert("DYNAMIC RANGE".into(), vec!["8".into()]);
        write_tags(&f, &tags).unwrap();

        let back = read_tags(&f).unwrap();
        assert_eq!(back["TITLE"], vec!["Hello"]);
        assert_eq!(back["DYNAMIC RANGE"], vec!["8"]);

        // STREAMINFO preserved, still 34 bytes and first
        let layout = parse_layout(&f).unwrap();
        assert_eq!(layout.blocks[0].0, BLOCK_STREAMINFO);
        assert_eq!(layout.blocks[0].1.len(), 34);
    }
}