//! Ogg Vorbis / Opus tag read/write using the pure-Rust `ogg` crate for paging.
//!
//! Header packets are preserved and the comment packet is rewritten; audio
//! packets are re-emitted through the packet writer with their granule
//! positions intact. Atomic temp+rename ([§1.3]).

use super::vorbis::VorbisComment;
use crate::atomic;
use crate::error::{IoResultExt, MloError, Result};
use crate::model::TagMap;
use ogg::{Packet, PacketReader, PacketWriteEndInfo, PacketWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    Vorbis,
    Opus,
}

const VORBIS_COMMENT_HEADER: &[u8] = &[0x03, b'v', b'o', b'r', b'b', b'i', b's'];
const OPUS_TAGS_HEADER: &[u8] = b"OpusTags";

fn detect(packets: &[Packet]) -> Result<Codec> {
    let first = packets
        .first()
        .ok_or_else(|| MloError::UnsupportedContainer {
            path: "<ogg>".into(),
            container: "ogg".into(),
            reason: "no packets".into(),
        })?;
    let d = &first.data;
    if d.len() >= 7 && d[0] == 0x01 && &d[1..7] == b"vorbis" {
        Ok(Codec::Vorbis)
    } else if d.len() >= 8 && &d[..8] == b"OpusHead" {
        Ok(Codec::Opus)
    } else {
        Err(MloError::UnsupportedContainer {
            path: "<ogg>".into(),
            container: "ogg".into(),
            reason: "unknown codec (not Vorbis or Opus)".into(),
        })
    }
}

fn read_packets(path: &Path) -> Result<Vec<Packet>> {
    let file = File::open(path).at(path)?;
    let mut reader = PacketReader::new(file);
    let mut packets = Vec::new();
    loop {
        match reader.read_packet() {
            Ok(Some(p)) => packets.push(p),
            Ok(None) => break,
            Err(e) => {
                return Err(MloError::Decode {
                    path: path.to_path_buf(),
                    reason: format!("ogg read: {e}"),
                })
            }
        }
        if packets.len() > 5_000_000 {
            return Err(MloError::Other("ogg: implausibly many packets".into()));
        }
    }
    Ok(packets)
}

/// Comment payload for a codec: Vorbis payload is prefixed by the packet type
/// and codec string and terminated by a framing bit; Opus by "OpusTags".
fn parse_comment(codec: Codec, data: &[u8]) -> Result<VorbisComment> {
    let payload: &[u8] = match codec {
        Codec::Vorbis => {
            if data.len() < 7 || &data[..7] != VORBIS_COMMENT_HEADER {
                return Err(MloError::Other("ogg: comment packet header mismatch".into()));
            }
            let body = &data[7..];
            // strip trailing framing bit
            &body[..body.len().saturating_sub(1)]
        }
        Codec::Opus => {
            if data.len() < 8 || &data[..8] != OPUS_TAGS_HEADER {
                return Err(MloError::Other("ogg: OpusTags header mismatch".into()));
            }
            &data[8..]
        }
    };
    VorbisComment::parse(payload)
}

fn build_comment(codec: Codec, tags: &TagMap, vendor: &str) -> Vec<u8> {
    let vc = VorbisComment::from_tagmap(tags, vendor);
    let body = vc.serialize();
    let mut out = Vec::with_capacity(body.len() + 9);
    match codec {
        Codec::Vorbis => {
            out.extend_from_slice(VORBIS_COMMENT_HEADER);
            out.extend_from_slice(&body);
            out.push(0x01); // framing bit
        }
        Codec::Opus => {
            out.extend_from_slice(OPUS_TAGS_HEADER);
            out.extend_from_slice(&body);
        }
    }
    out
}

/// Index of the comment packet and how many header packets precede audio.
fn header_count(codec: Codec) -> usize {
    match codec {
        Codec::Vorbis => 3,
        Codec::Opus => 2,
    }
}

pub fn read_tags(path: &Path) -> Result<TagMap> {
    let packets = read_packets(path)?;
    let codec = detect(&packets)?;
    let idx = 1usize; // comment is the second packet in both codecs
    let p = packets.get(idx).ok_or_else(|| MloError::Decode {
        path: path.to_path_buf(),
        reason: "missing comment packet".into(),
    })?;
    Ok(parse_comment(codec, &p.data)?.to_tagmap())
}

pub fn write_tags(path: &Path, tags: &TagMap) -> Result<()> {
    let packets = read_packets(path)?;
    if packets.is_empty() {
        return Err(MloError::Decode { path: path.to_path_buf(), reason: "no packets".into() });
    }
    let codec = detect(&packets)?;
    let hdr = header_count(codec);
    if packets.len() < hdr {
        return Err(MloError::Decode {
            path: path.to_path_buf(),
            reason: format!("truncated header ({}/{hdr} packets)", packets.len()),
        });
    }
    let vendor = format!("mlo {}", crate::version());
    let new_comment = build_comment(codec, tags, &vendor);

    atomic::write_atomic_with(path, |out| -> std::io::Result<()> {
        let bw = BufWriter::new(out);
        let mut writer = PacketWriter::new(bw);
        let serial = packets[0].stream_serial();
        let total = packets.len();
        for (i, p) in packets.iter().enumerate() {
            let data: &[u8] = if i == 1 { &new_comment } else { &p.data };
            let last = i + 1 == total;
            let inf = if last {
                PacketWriteEndInfo::EndStream
            } else if i < hdr {
                // header pages are terminated explicitly per codec spec
                let ends_page = match codec {
                    Codec::Vorbis => i == 0 || i == 2,
                    Codec::Opus => i == 0 || i == 1,
                };
                if ends_page {
                    PacketWriteEndInfo::EndPage
                } else {
                    PacketWriteEndInfo::NormalPacket
                }
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            writer.write_packet(data, serial, inf, p.absgp_page()).map_err(|e| {
                std::io::Error::other(format!("ogg write: {e}"))
            })?;
        }
        writer.into_inner().into_inner().map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogg::PacketWriter;

    /// Build a minimal Opus-like ogg stream with one comment packet and one
    /// dummy audio packet (not a real decoder stream, but valid paging).
    fn tiny_ogg(tags: Option<&TagMap>) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut w = PacketWriter::new(&mut out);
            let head = b"OpusHead\x01\x02\x00\x00\x80\xbb\x00\x00\x00\x00\x00";
            let comment = tags
                .map(|t| build_comment(Codec::Opus, t, "test"))
                .unwrap_or_else(|| build_comment(Codec::Opus, &TagMap::new(), "test"));
            w.write_packet(&head[..], 1, PacketWriteEndInfo::EndPage, 0).unwrap();
            w.write_packet(&comment[..], 1, PacketWriteEndInfo::EndPage, 0).unwrap();
            w.write_packet(&[0u8; 16][..], 1, PacketWriteEndInfo::EndStream, 960).unwrap();
        }
        out
    }

    #[test]
    fn opus_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.opus");
        std::fs::write(&f, tiny_ogg(None)).unwrap();

        let mut tags = TagMap::new();
        tags.insert("TITLE".into(), vec!["Opus Title".into()]);
        tags.insert("DYNAMIC RANGE".into(), vec!["7".into()]);
        write_tags(&f, &tags).unwrap();

        let back = read_tags(&f).unwrap();
        assert_eq!(back["TITLE"], vec!["Opus Title"]);
        assert_eq!(back["DYNAMIC RANGE"], vec!["7"]);
    }
}