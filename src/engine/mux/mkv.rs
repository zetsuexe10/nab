use std::path::Path;

use super::bytes::{Cur, Res};
use super::mp4;
use super::webm::{
    element, float_element, uint_element, write, Block, Doc, Entry, Part, AUDIO, VIDEO,
};
use crate::model::Meta;

const CODEC_ID: u32 = 0x86;
const CODEC_PRIVATE: u32 = 0x63A2;
const FLAG_LACING: u32 = 0x9C;
const LANGUAGE: u32 = 0x22_B59C;
const VIDEO_SETTINGS: u32 = 0xE0;
const PIXEL_WIDTH: u32 = 0xB0;
const PIXEL_HEIGHT: u32 = 0xBA;
const AUDIO_SETTINGS: u32 = 0xE1;
const SAMPLING_FREQUENCY: u32 = 0xB5;
const CHANNELS: u32 = 0x9F;
const VIDEO_ENTRY: usize = 78;
const AUDIO_ENTRY: usize = 28;
const AAC: u8 = 0x40;

pub fn merge(video: &Path, audio: &Path, out: &Path, meta: Option<&Meta>) -> Res<()> {
    let video = matroska_part(mp4::demux(video)?.into_part(b"vide")?, VIDEO)?;
    let audio = matroska_part(mp4::demux(audio)?.into_part(b"soun")?, AUDIO)?;
    write(vec![video, audio], out, Doc::Matroska, meta)
}

pub fn extract_audio(source: &Path, out: &Path, meta: Option<&Meta>) -> Res<()> {
    let audio = matroska_part(mp4::demux(source)?.into_part(b"soun")?, AUDIO)?;
    write(vec![audio], out, Doc::Matroska, meta)
}

type Children = Vec<(Vec<u8>, Vec<u8>)>;

fn sample_entry(stsd: &[u8], fixed: usize) -> Res<([u8; 4], Children, Vec<u8>)> {
    let mut cur = Cur::new(stsd);
    cur.skip(12)?;
    let count = cur.u32()?;
    if count == 0 {
        return Err("track has no sample description".to_string());
    }
    let size = cur.u32()? as usize;
    let kind: [u8; 4] = cur
        .take(4)?
        .try_into()
        .map_err(|_| "sample description is corrupt".to_string())?;
    if size < 8 + fixed {
        return Err("sample description is corrupt".to_string());
    }
    let body = cur.take(size - 8)?.to_vec();
    let mut children = Vec::new();
    let mut inner = Cur::new(&body[fixed..]);
    while inner.left() >= 8 {
        let length = inner.u32()? as usize;
        let name = inner.take(4)?.to_vec();
        if length < 8 || length - 8 > inner.left() {
            break;
        }
        children.push((name, inner.take(length - 8)?.to_vec()));
    }
    let head = body[..fixed].to_vec();
    Ok((kind, children, head))
}

fn child<'a>(children: &'a [(Vec<u8>, Vec<u8>)], name: &[u8; 4]) -> Option<&'a Vec<u8>> {
    children
        .iter()
        .find(|(kind, _)| kind.as_slice() == name.as_slice())
        .map(|(_, body)| body)
}

fn video_fields(stsd: &[u8]) -> Res<Vec<Vec<u8>>> {
    let (kind, children, head) = sample_entry(stsd, VIDEO_ENTRY)?;
    let (id, private): (&[u8], Vec<u8>) = match &kind {
        b"avc1" | b"avc3" => (
            b"V_MPEG4/ISO/AVC",
            child(&children, b"avcC")
                .ok_or_else(|| "h264 configuration is missing".to_string())?
                .clone(),
        ),
        b"hvc1" | b"hev1" => (
            b"V_MPEGH/ISO/HEVC",
            child(&children, b"hvcC")
                .ok_or_else(|| "hevc configuration is missing".to_string())?
                .clone(),
        ),
        b"av01" => (
            b"V_AV1",
            child(&children, b"av1C")
                .ok_or_else(|| "av1 configuration is missing".to_string())?
                .clone(),
        ),
        other => {
            return Err(format!(
                "{} video cannot be stored in mkv",
                String::from_utf8_lossy(other)
            ))
        }
    };
    let width = u16::from_be_bytes([head[24], head[25]]) as u64;
    let height = u16::from_be_bytes([head[26], head[27]]) as u64;
    let mut size = uint_element(PIXEL_WIDTH, width);
    size.extend_from_slice(&uint_element(PIXEL_HEIGHT, height));
    Ok(vec![
        element(CODEC_ID, id),
        element(CODEC_PRIVATE, &private),
        element(VIDEO_SETTINGS, &size),
    ])
}

fn descriptor(cur: &mut Cur) -> Res<(u8, usize)> {
    let tag = cur.u8()?;
    let mut length = 0usize;
    for _ in 0..4 {
        let byte = cur.u8()?;
        length = (length << 7) | (byte & 0x7f) as usize;
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok((tag, length))
}

fn aac_config(esds: &[u8]) -> Res<Vec<u8>> {
    let broken = || "aac configuration is corrupt".to_string();
    let mut cur = Cur::new(esds);
    cur.skip(4)?;
    let (tag, _) = descriptor(&mut cur)?;
    if tag != 0x03 {
        return Err(broken());
    }
    cur.skip(2)?;
    let flags = cur.u8()?;
    if flags & 0x80 != 0 {
        cur.skip(2)?;
    }
    if flags & 0x40 != 0 {
        let length = cur.u8()? as usize;
        cur.skip(length)?;
    }
    if flags & 0x20 != 0 {
        cur.skip(2)?;
    }
    let (tag, _) = descriptor(&mut cur)?;
    if tag != 0x04 {
        return Err(broken());
    }
    if cur.u8()? != AAC {
        return Err("only aac audio can be stored in mkv".to_string());
    }
    cur.skip(12)?;
    let (tag, length) = descriptor(&mut cur)?;
    if tag != 0x05 {
        return Err(broken());
    }
    Ok(cur.take(length)?.to_vec())
}

fn audio_fields(stsd: &[u8]) -> Res<Vec<Vec<u8>>> {
    let (kind, children, head) = sample_entry(stsd, AUDIO_ENTRY)?;
    if &kind != b"mp4a" {
        return Err(format!(
            "{} audio cannot be stored in mkv",
            String::from_utf8_lossy(&kind)
        ));
    }
    let esds =
        child(&children, b"esds").ok_or_else(|| "aac configuration is missing".to_string())?;
    let private = aac_config(esds)?;
    let channels = u16::from_be_bytes([head[16], head[17]]) as u64;
    let rate = u16::from_be_bytes([head[24], head[25]]) as f64;
    let mut settings = float_element(SAMPLING_FREQUENCY, rate);
    settings.extend_from_slice(&uint_element(CHANNELS, channels.max(1)));
    Ok(vec![
        element(CODEC_ID, b"A_AAC"),
        element(CODEC_PRIVATE, &private),
        element(AUDIO_SETTINGS, &settings),
    ])
}

fn ms(ticks: i64, timescale: u32) -> i64 {
    let scale = timescale.max(1) as i128;
    let value = ticks as i128 * 1000;
    let half = scale / 2;
    (if value >= 0 {
        (value + half) / scale
    } else {
        (value - half) / scale
    }) as i64
}

fn matroska_part(part: mp4::Part, kind: u8) -> Res<Part> {
    let track = &part.track;
    let mut fields = if kind == VIDEO {
        video_fields(&track.stsd)?
    } else {
        audio_fields(&track.stsd)?
    };
    fields.push(uint_element(FLAG_LACING, 0));
    fields.push(element(LANGUAGE, b"und"));
    let mut blocks = Vec::with_capacity(track.samples.len());
    let mut clock = 0i64;
    for sample in &track.samples {
        let decode = ms(clock, track.timescale);
        let present = ms(
            clock + sample.cto as i64 - track.shift as i64,
            track.timescale,
        );
        blocks.push(Block {
            track: 1,
            time: present,
            decode,
            flags: if sample.sync { 0x80 } else { 0 },
            keyframe: sample.sync,
            data: sample.offset,
            len: sample.size,
            extra: Vec::new(),
            grouped: false,
        });
        clock += sample.dur as i64;
    }
    let duration_ns = clock as f64 / track.timescale.max(1) as f64 * 1e9;
    Ok(Part {
        reader: part.reader,
        entry: Entry {
            number: 1,
            kind,
            fields,
        },
        blocks,
        scale: 1_000_000,
        duration_ns,
    })
}
