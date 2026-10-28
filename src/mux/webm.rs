use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use super::bytes::{truncated, write_failed, Put, Res, Window};

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMECODE_SCALE: u32 = 0x2A_D7B1;
const DURATION: u32 = 0x4489;
const MUXING_APP: u32 = 0x4D80;
const WRITING_APP: u32 = 0x5741;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_UID: u32 = 0x73C5;
const TRACK_TYPE: u32 = 0x83;
const CLUSTER: u32 = 0x1F43_B675;
const TIMECODE: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const REFERENCE_BLOCK: u32 = 0xFB;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CUE_CLUSTER_POSITION: u32 = 0xF1;
const CHAPTERS: u32 = 0x1043_A770;
const TAGS: u32 = 0x1254_C367;
const ATTACHMENTS: u32 = 0x1941_A469;
const DOC_TYPE: u32 = 0x4282;

const MAX_CLUSTER_MS: i64 = 10_000;
const MAX_ELEMENT: u64 = 1 << 28;
const VIDEO: u8 = 1;
const AUDIO: u8 = 2;

struct Header {
    id: u32,
    size: Option<u64>,
    start: u64,
    body: u64,
}

fn read_header(reader: &mut Window, pos: u64) -> Res<Header> {
    let len = reader.len();
    if pos >= len {
        return truncated();
    }
    let take = (len - pos).min(12) as usize;
    let head = reader.vec_at(pos, take as u64)?;
    let first = head[0];
    let id_len = first.leading_zeros() as usize + 1;
    if id_len > 4 || head.len() < id_len + 1 {
        return Err("webm element is corrupt".to_string());
    }
    let mut id = 0u32;
    for byte in &head[..id_len] {
        id = (id << 8) | *byte as u32;
    }
    let size_first = head[id_len];
    let size_len = size_first.leading_zeros() as usize + 1;
    if size_len > 8 || head.len() < id_len + size_len {
        return Err("webm element is corrupt".to_string());
    }
    let mut value = (size_first as u64) & (0xffu64 >> size_len);
    let mut all_ones = value == (0xffu64 >> size_len);
    for byte in &head[id_len + 1..id_len + size_len] {
        value = (value << 8) | *byte as u64;
        all_ones &= *byte == 0xff;
    }
    Ok(Header {
        id,
        size: if all_ones { None } else { Some(value) },
        start: pos,
        body: pos + (id_len + size_len) as u64,
    })
}

fn read_uint(reader: &mut Window, header: &Header) -> Res<u64> {
    let size = header.size.unwrap_or(0);
    if size > 8 {
        return Err("webm element is corrupt".to_string());
    }
    let bytes = reader.vec_at(header.body, size)?;
    Ok(bytes
        .iter()
        .fold(0u64, |acc, byte| (acc << 8) | *byte as u64))
}

fn read_float(reader: &mut Window, header: &Header) -> Res<f64> {
    let bytes = reader.vec_at(header.body, header.size.unwrap_or(0))?;
    match bytes.len() {
        4 => Ok(f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64),
        8 => {
            let mut array = [0u8; 8];
            array.copy_from_slice(&bytes);
            Ok(f64::from_be_bytes(array))
        }
        _ => Ok(0.0),
    }
}

fn is_top_level(id: u32) -> bool {
    matches!(
        id,
        CLUSTER | CUES | TAGS | INFO | TRACKS | SEEK_HEAD | CHAPTERS | ATTACHMENTS | SEGMENT | EBML
    )
}

struct Entry {
    number: u64,
    kind: u8,
    fields: Vec<Vec<u8>>,
}

struct Block {
    track: u64,
    time: i64,
    flags: u8,
    keyframe: bool,
    data: u64,
    len: u32,
    extra: Vec<u8>,
    grouped: bool,
}

struct Part {
    reader: Window,
    entry: Entry,
    blocks: Vec<Block>,
    scale: u64,
    duration_ns: f64,
}

fn put_id(out: &mut Vec<u8>, id: u32) {
    let bytes = id.to_be_bytes();
    let skip = bytes.iter().position(|byte| *byte != 0).unwrap_or(3);
    out.extend_from_slice(&bytes[skip..]);
}

fn put_size(out: &mut Vec<u8>, size: u64) {
    let mut length = 1usize;
    while length < 8 && size >= (1u64 << (7 * length)) - 1 {
        length += 1;
    }
    let marked = size | (1u64 << (7 * length));
    out.extend_from_slice(&marked.to_be_bytes()[8 - length..]);
}

pub(super) fn element(id: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 12);
    put_id(&mut out, id);
    put_size(&mut out, body.len() as u64);
    out.extend_from_slice(body);
    out
}

pub(super) fn uint_element(id: u32, value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().position(|byte| *byte != 0).unwrap_or(7);
    element(id, &bytes[skip..])
}

fn float_element(id: u32, value: f64) -> Vec<u8> {
    element(id, &value.to_be_bytes())
}

fn eight_byte_size(size: u64) -> [u8; 8] {
    (size | (1u64 << 56)).to_be_bytes()
}

fn parse_entry(reader: &mut Window, header: &Header, end: u64) -> Res<Entry> {
    let mut number = 0u64;
    let mut kind = 0u8;
    let mut fields = Vec::new();
    let mut pos = header.body;
    while pos < end {
        let child = read_header(reader, pos)?;
        let size = child
            .size
            .ok_or_else(|| "webm element is corrupt".to_string())?;
        let child_end = child.body + size;
        if child_end > end {
            return truncated();
        }
        match child.id {
            TRACK_NUMBER => number = read_uint(reader, &child)?,
            TRACK_TYPE => kind = read_uint(reader, &child)? as u8,
            TRACK_UID => {}
            _ => {
                if size > MAX_ELEMENT {
                    return Err("webm track header is too large".to_string());
                }
                fields.push(reader.vec_at(child.start, child_end - child.start)?);
            }
        }
        pos = child_end;
    }
    Ok(Entry {
        number,
        kind,
        fields,
    })
}

struct Scan {
    scale: u64,
    duration: f64,
    entries: Vec<Entry>,
    blocks: Vec<Block>,
}

fn read_block_head(reader: &mut Window, body: u64, size: u64) -> Res<(u64, i16, u8, u64)> {
    if size < 4 {
        return Err("webm block is corrupt".to_string());
    }
    let take = size.min(12) as usize;
    let head = reader.vec_at(body, take as u64)?;
    let track_len = head[0].leading_zeros() as usize + 1;
    if track_len > 8 || head.len() < track_len + 3 {
        return Err("webm block is corrupt".to_string());
    }
    let mut track = (head[0] as u64) & (0xffu64 >> track_len);
    for byte in &head[1..track_len] {
        track = (track << 8) | *byte as u64;
    }
    let relative = i16::from_be_bytes([head[track_len], head[track_len + 1]]);
    let flags = head[track_len + 2];
    Ok((track, relative, flags, body + track_len as u64 + 3))
}

fn scan(reader: &mut Window) -> Res<Scan> {
    let len = reader.len();
    let first = read_header(reader, 0)?;
    if first.id != EBML {
        return Err("not a webm file".to_string());
    }
    let ebml_end = first.body + first.size.unwrap_or(0);
    let mut cursor = first.body;
    while cursor < ebml_end {
        let child = read_header(reader, cursor)?;
        if child.id == DOC_TYPE {
            let text = reader.vec_at(child.body, child.size.unwrap_or(0))?;
            if text != b"webm" && text != b"matroska" {
                return Err("not a webm file".to_string());
            }
        }
        cursor = child.body + child.size.unwrap_or(0);
    }
    let mut pos = ebml_end;
    let mut segment = None;
    while pos < len {
        let header = read_header(reader, pos)?;
        if header.id == SEGMENT {
            segment = Some(header);
            break;
        }
        pos = header.body + header.size.unwrap_or(0);
    }
    let segment = segment.ok_or_else(|| "webm has no segment".to_string())?;
    let segment_end = match segment.size {
        Some(size) => (segment.body + size).min(len),
        None => len,
    };
    let mut result = Scan {
        scale: 1_000_000,
        duration: 0.0,
        entries: Vec::new(),
        blocks: Vec::new(),
    };
    let mut pos = segment.body;
    while pos < segment_end {
        let header = match read_header(reader, pos) {
            Ok(header) => header,
            Err(_) => break,
        };
        let end = match header.size {
            Some(size) => header.body.saturating_add(size),
            None => segment_end,
        };
        match header.id {
            INFO => {
                let mut at = header.body;
                while at < end.min(segment_end) {
                    let child = read_header(reader, at)?;
                    match child.id {
                        TIMECODE_SCALE => result.scale = read_uint(reader, &child)?.max(1),
                        DURATION => result.duration = read_float(reader, &child)?,
                        _ => {}
                    }
                    at = child.body + child.size.unwrap_or(0);
                }
                pos = end;
            }
            TRACKS => {
                if end > segment_end {
                    return truncated();
                }
                let mut at = header.body;
                while at < end {
                    let child = read_header(reader, at)?;
                    let child_end = child.body + child.size.unwrap_or(0);
                    if child.id == TRACK_ENTRY {
                        result.entries.push(parse_entry(reader, &child, child_end)?);
                    }
                    at = child_end;
                }
                pos = end;
            }
            CLUSTER => {
                pos = scan_cluster(reader, &header, segment_end, &mut result.blocks)?;
            }
            _ => pos = end,
        }
    }
    Ok(result)
}

fn scan_cluster(
    reader: &mut Window,
    header: &Header,
    segment_end: u64,
    blocks: &mut Vec<Block>,
) -> Res<u64> {
    let end = match header.size {
        Some(size) => header.body.saturating_add(size),
        None => segment_end,
    };
    if header.size.is_some() && end > segment_end {
        return truncated();
    }
    let mut stamp = 0i64;
    let mut pos = header.body;
    while pos < end {
        let child = match read_header(reader, pos) {
            Ok(child) => child,
            Err(_) if header.size.is_none() => return Ok(end),
            Err(error) => return Err(error),
        };
        if header.size.is_none() && is_top_level(child.id) {
            return Ok(pos);
        }
        let size = child
            .size
            .ok_or_else(|| "webm element is corrupt".to_string())?;
        let child_end = child.body + size;
        if child_end > end {
            return truncated();
        }
        match child.id {
            TIMECODE => stamp = read_uint(reader, &child)? as i64,
            SIMPLE_BLOCK => {
                let (track, relative, flags, data) = read_block_head(reader, child.body, size)?;
                blocks.push(Block {
                    track,
                    time: stamp + relative as i64,
                    flags,
                    keyframe: flags & 0x80 != 0,
                    data,
                    len: (child_end - data) as u32,
                    extra: Vec::new(),
                    grouped: false,
                });
            }
            BLOCK_GROUP => {
                let mut found: Option<(u64, i16, u8, u64, u64)> = None;
                let mut extra = Vec::new();
                let mut keyframe = true;
                let mut at = child.body;
                while at < child_end {
                    let inner = read_header(reader, at)?;
                    let inner_size = inner
                        .size
                        .ok_or_else(|| "webm element is corrupt".to_string())?;
                    let inner_end = inner.body + inner_size;
                    if inner_end > child_end {
                        return truncated();
                    }
                    if inner.id == BLOCK {
                        let (track, relative, flags, data) =
                            read_block_head(reader, inner.body, inner_size)?;
                        found = Some((track, relative, flags, data, inner_end));
                    } else {
                        if inner.id == REFERENCE_BLOCK {
                            keyframe = false;
                        }
                        if inner_size > MAX_ELEMENT {
                            return Err("webm block is too large".to_string());
                        }
                        extra.extend_from_slice(
                            &reader.vec_at(inner.start, inner_end - inner.start)?,
                        );
                    }
                    at = inner_end;
                }
                if let Some((track, relative, flags, data, block_end)) = found {
                    blocks.push(Block {
                        track,
                        time: stamp + relative as i64,
                        flags,
                        keyframe,
                        data,
                        len: (block_end - data) as u32,
                        extra,
                        grouped: true,
                    });
                }
            }
            _ => {}
        }
        pos = child_end;
    }
    Ok(end)
}

fn demux(path: &Path, kind: u8) -> Res<Part> {
    let mut reader = Window::open(path)?;
    let scanned = scan(&mut reader)?;
    let wanted = if kind == VIDEO { "video" } else { "audio" };
    let position = scanned
        .entries
        .iter()
        .position(|entry| entry.kind == kind)
        .ok_or_else(|| format!("no {wanted} track found"))?;
    let entry = scanned.entries.into_iter().nth(position).unwrap();
    let blocks: Vec<Block> = scanned
        .blocks
        .into_iter()
        .filter(|block| block.track == entry.number)
        .collect();
    if blocks.is_empty() {
        return Err(format!("no {wanted} track found"));
    }
    Ok(Part {
        reader,
        entry,
        blocks,
        scale: scanned.scale,
        duration_ns: scanned.duration * scanned.scale as f64,
    })
}

fn to_ms(time: i64, scale: u64) -> i64 {
    let nanos = time as i128 * scale as i128;
    let half = 500_000i128;
    (if nanos >= 0 {
        (nanos + half) / 1_000_000
    } else {
        (nanos - half) / 1_000_000
    }) as i64
}

struct Planned {
    part: usize,
    index: usize,
    ms: i64,
}

fn flat(parts: &[Part]) -> Vec<Planned> {
    let mut all: Vec<Planned> = Vec::new();
    for (part_index, part) in parts.iter().enumerate() {
        for (index, block) in part.blocks.iter().enumerate() {
            all.push(Planned {
                part: part_index,
                index,
                ms: to_ms(block.time, part.scale),
            });
        }
    }
    all.sort_by_key(|planned| planned.ms);
    all
}

fn track_element(part: &Part, number: u64) -> Vec<u8> {
    let mut body = uint_element(TRACK_NUMBER, number);
    body.extend_from_slice(&uint_element(TRACK_UID, number));
    body.extend_from_slice(&uint_element(TRACK_TYPE, part.entry.kind as u64));
    for field in &part.entry.fields {
        body.extend_from_slice(field);
    }
    element(TRACK_ENTRY, &body)
}

fn block_bytes(part: &mut Part, index: usize, number: u64, relative: i16) -> Res<Vec<u8>> {
    let (len, flags, data, grouped, extra) = {
        let block = &part.blocks[index];
        (
            block.len as usize,
            block.flags,
            block.data,
            block.grouped,
            block.extra.clone(),
        )
    };
    let mut body = Vec::with_capacity(len + 4);
    body.put_u8(0x80 | number as u8);
    body.put_u16(relative as u16);
    body.put_u8(flags);
    body.extend_from_slice(&part.reader.vec_at(data, len as u64)?);
    if !grouped {
        return Ok(element(SIMPLE_BLOCK, &body));
    }
    let mut group = element(BLOCK, &body);
    group.extend_from_slice(&extra);
    Ok(element(BLOCK_GROUP, &group))
}

pub fn merge(video: &Path, audio: &Path, out: &Path) -> Res<()> {
    let mut parts = vec![demux(video, VIDEO)?, demux(audio, AUDIO)?];
    let order = flat(&parts);
    let file = File::create(out).map_err(|error| format!("cannot write output: {error}"))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    let mut head = Vec::new();
    let mut ebml = uint_element(0x4286, 1);
    ebml.extend_from_slice(&uint_element(0x42F7, 1));
    ebml.extend_from_slice(&uint_element(0x42F2, 4));
    ebml.extend_from_slice(&uint_element(0x42F3, 8));
    ebml.extend_from_slice(&element(DOC_TYPE, b"webm"));
    ebml.extend_from_slice(&uint_element(0x4287, 4));
    ebml.extend_from_slice(&uint_element(0x4285, 2));
    head.extend_from_slice(&element(EBML, &ebml));
    put_id(&mut head, SEGMENT);
    let size_at = head.len() as u64;
    head.extend_from_slice(&eight_byte_size(0));
    let segment_start = head.len() as u64;
    writer.write_all(&head).map_err(write_failed)?;

    let info_pos = seek_head(0, 0, 0).len() as u64;
    let last = order.last().map(|planned| planned.ms).unwrap_or(0);
    let source_ms = parts
        .iter()
        .map(|part| part.duration_ns / 1_000_000.0)
        .fold(0.0, f64::max);
    let mut info = uint_element(TIMECODE_SCALE, 1_000_000);
    info.extend_from_slice(&float_element(DURATION, source_ms.max(last as f64)));
    info.extend_from_slice(&element(MUXING_APP, b"snag"));
    info.extend_from_slice(&element(WRITING_APP, b"snag"));
    let info = element(INFO, &info);
    let mut tracks = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        tracks.extend_from_slice(&track_element(part, index as u64 + 1));
    }
    let tracks = element(TRACKS, &tracks);
    let tracks_pos = info_pos + info.len() as u64;
    let after_tracks = tracks_pos + tracks.len() as u64;
    writer
        .write_all(&seek_head(info_pos, tracks_pos, 0))
        .map_err(write_failed)?;
    writer.write_all(&info).map_err(write_failed)?;
    writer.write_all(&tracks).map_err(write_failed)?;

    let mut position = after_tracks;
    let mut cues: Vec<(i64, u64)> = Vec::new();
    let mut at = 0usize;
    while at < order.len() {
        let origin = order[at].ms;
        let mut body = uint_element(TIMECODE, origin.max(0) as u64);
        let base = origin.max(0);
        let mut keyed = false;
        let mut key_time = origin;
        let mut first = true;
        while at < order.len() {
            let planned = &order[at];
            let part = &parts[planned.part];
            let block = &part.blocks[planned.index];
            let video_key = planned.part == 0 && block.keyframe;
            let relative = planned.ms - base;
            if !first
                && (video_key
                    || relative > MAX_CLUSTER_MS
                    || relative > i16::MAX as i64
                    || relative < i16::MIN as i64)
            {
                break;
            }
            if video_key && !keyed {
                keyed = true;
                key_time = planned.ms;
            }
            let bytes = block_bytes(
                &mut parts[planned.part],
                planned.index,
                planned.part as u64 + 1,
                relative as i16,
            )?;
            body.extend_from_slice(&bytes);
            first = false;
            at += 1;
        }
        if keyed {
            cues.push((key_time, position));
        }
        let cluster = element(CLUSTER, &body);
        writer.write_all(&cluster).map_err(write_failed)?;
        position += cluster.len() as u64;
    }
    let cues_pos = position;
    let mut points = Vec::new();
    for (time, cluster) in &cues {
        let mut positions = uint_element(CUE_TRACK, 1);
        positions.extend_from_slice(&uint_element(CUE_CLUSTER_POSITION, *cluster));
        let mut point = uint_element(CUE_TIME, (*time).max(0) as u64);
        point.extend_from_slice(&element(CUE_TRACK_POSITIONS, &positions));
        points.extend_from_slice(&element(CUE_POINT, &point));
    }
    let cues_bytes = element(CUES, &points);
    writer.write_all(&cues_bytes).map_err(write_failed)?;
    let segment_size = cues_pos + cues_bytes.len() as u64;
    writer.flush().map_err(write_failed)?;
    let mut file = writer
        .into_inner()
        .map_err(|error| format!("cannot write output: {error}"))?;
    file.seek(SeekFrom::Start(size_at)).map_err(write_failed)?;
    file.write_all(&eight_byte_size(segment_size))
        .map_err(write_failed)?;
    file.seek(SeekFrom::Start(segment_start))
        .map_err(write_failed)?;
    file.write_all(&seek_head(info_pos, tracks_pos, cues_pos))
        .map_err(write_failed)?;
    file.flush().map_err(write_failed)
}

fn seek_head(info: u64, tracks: u64, cues: u64) -> Vec<u8> {
    let mut body = Vec::new();
    for (id, position) in [(INFO, info), (TRACKS, tracks), (CUES, cues)] {
        let mut name = Vec::new();
        put_id(&mut name, id);
        let mut entry = element(SEEK_ID, &name);
        entry.extend_from_slice(&element(SEEK_POSITION, &position.to_be_bytes()));
        body.extend_from_slice(&element(SEEK, &entry));
    }
    element(SEEK_HEAD, &body)
}

#[cfg(test)]
pub struct Probe {
    part: Part,
    cues: bool,
}

#[cfg(test)]
pub fn demux_for_test(path: &Path, number: u64) -> Res<Probe> {
    let mut part = demux(path, if number == 1 { VIDEO } else { AUDIO })?;
    let mut reader = Window::open(path)?;
    let cues = {
        let bytes = reader.vec_at(0, reader.len())?;
        bytes
            .windows(4)
            .any(|window| window == [0x1C, 0x53, 0xBB, 0x6B])
    };
    part.scale = part.scale.max(1);
    Ok(Probe { part, cues })
}

#[cfg(test)]
impl Probe {
    pub fn count(&self) -> usize {
        self.part.blocks.len()
    }

    pub fn has_cues(&self) -> bool {
        self.cues
    }

    pub fn block(&mut self, index: usize) -> (i64, bool, Vec<u8>) {
        let block = &self.part.blocks[index];
        let (time, key, data, len) = (block.time, block.keyframe, block.data, block.len);
        let bytes = self.part.reader.vec_at(data, len as u64).unwrap();
        (to_ms(time, self.part.scale), key, bytes)
    }
}
